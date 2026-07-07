//! Shared WebDriver e2e harness: owns Vite (only if it spawned it),
//! tauri-driver, and the WebDriver session. Teardown kills by held child
//! PID / process group only — never pattern-match (pkill self-match footgun).
//!
//! API deviation from brief (thirtyfour 0.37.2): `Capabilities` exposes
//! `.set(key, value)` (serialises via serde) rather than `.insert(key, value)`.
//! The public surface — `Gui::launch()`, `gui.driver`, `gui.shutdown()` — is
//! unchanged.

use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use thirtyfour::prelude::*;
use thirtyfour::Capabilities;

const VITE_URL: &str = "http://localhost:5173";

pub struct Gui {
    pub driver: WebDriver,
    tauri_driver: Child,
    vite: Option<Child>, // Some only if WE spawned it
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

async fn http_ok(url: &str) -> bool {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn wait_http(url: &str, timeout: Duration, what: &str) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if http_ok(url).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!("{what} never became ready at {url}");
}

impl Gui {
    pub async fn launch() -> Gui {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let web_dir = manifest.parent().unwrap().join("web");
        let app_bin = manifest.join("target/debug/rust-agent-runtime-desktop");
        assert!(app_bin.exists(), "debug binary missing — run `cargo build` first");

        // 1. Vite: the debug binary renders from the devUrl, not bundled assets.
        let vite = if http_ok(VITE_URL).await {
            None
        } else {
            let child = Command::new("npm")
                .args(["--prefix", web_dir.to_str().unwrap(), "run", "dev"])
                .process_group(0)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn vite (npm --prefix web run dev)");
            wait_http(VITE_URL, Duration::from_secs(60), "vite").await;
            Some(child)
        };

        // 2. tauri-driver on free ports; WEBKIT_WEBDRIVER env overrides the
        //    native driver path if WebKitWebDriver isn't on PATH.
        let port = free_port();
        let native_port = free_port();
        let mut cmd = Command::new("tauri-driver");
        cmd.args(["--port", &port.to_string(), "--native-port", &native_port.to_string()])
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Ok(native) = std::env::var("WEBKIT_WEBDRIVER") {
            cmd.args(["--native-driver", &native]);
        }
        let tauri_driver = cmd.spawn().expect("spawn tauri-driver (is it on PATH? ~/.cargo/bin)");
        let base = format!("http://127.0.0.1:{port}");
        wait_http(&format!("{base}/status"), Duration::from_secs(15), "tauri-driver").await;

        // 3. WebDriver session — this launches the app in automation mode.
        //    thirtyfour 0.37.2: use .set(key, value) (serde-based) instead of
        //    .insert(key, value) (not present on Capabilities in this version).
        let mut caps = Capabilities::new();
        caps.set(
            "tauri:options",
            serde_json::json!({ "application": app_bin.to_str().unwrap() }),
        )
        .expect("set tauri:options cap");
        caps.set("browserName", serde_json::json!("wry")).expect("set browserName cap");
        let driver = WebDriver::new(&base, caps).await.expect("webdriver session");

        Gui { driver, tauri_driver, vite }
    }

    /// Graceful end: deletes the session (closes the app), then Drop reaps
    /// the child processes.
    pub async fn shutdown(self) {
        // WebDriver derives Clone in thirtyfour 0.37.2 — clone to satisfy quit(self).
        let _ = self.driver.clone().quit().await;
    }
}

impl Drop for Gui {
    fn drop(&mut self) {
        // SIGTERM the whole process group (tauri-driver → WebKitWebDriver → app).
        unsafe { libc::kill(-(self.tauri_driver.id() as i32), libc::SIGTERM) };
        let _ = self.tauri_driver.wait();
        if let Some(v) = &mut self.vite {
            unsafe { libc::kill(-(v.id() as i32), libc::SIGTERM) };
            let _ = v.wait();
        }
    }
}
