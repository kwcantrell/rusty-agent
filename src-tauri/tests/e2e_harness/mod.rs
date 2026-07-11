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

/// Kills the child's whole process group on drop — covers panics between
/// spawn and Gui construction (Child::drop alone does not kill).
struct KillOnDrop(Option<Child>);
impl KillOnDrop {
    fn into_inner(mut self) -> Child {
        self.0.take().unwrap()
    }
}
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(c) = &mut self.0 {
            unsafe { libc::kill(-(c.id() as i32), libc::SIGTERM) };
            let _ = c.wait();
        }
    }
}

pub struct Gui {
    pub driver: WebDriver,
    tauri_driver: Child,
    vite: Option<Child>, // Some only if WE spawned it
    /// PID of the launched app process (the prod desktop binary), discovered by
    /// matching the binary path — used by the kill-restart drive to hard-kill the
    /// daemon (SIGKILL) between two WebDriver sessions. `None` if never found.
    app_pid: Option<i32>,
}

/// The prod desktop binary name — the process the WebDriver session launches and
/// the ONLY process the kill-restart drive ever `SIGKILL`s (matched by exact
/// path, never a broad pattern).
const APP_BIN_NAME: &str = "rust-agent-runtime-desktop";

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Find the PID of the running app by matching its EXACT binary path against
/// each process's `/proc/<pid>/exe` symlink target (the app's argv[0] is the
/// absolute path we handed tauri:options). Exact-path match, never a substring
/// pattern — so we can only ever target the process we launched, not some other
/// `rust-agent-runtime-desktop` a developer happens to be running. Retries
/// briefly: WebKitWebDriver forks the app a beat after session-create returns.
fn find_app_pid(app_bin_path: &str) -> Option<i32> {
    let want = std::fs::canonicalize(app_bin_path).ok()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for e in entries.flatten() {
                let name = e.file_name();
                let Some(pid) = name.to_str().and_then(|s| s.parse::<i32>().ok()) else {
                    continue;
                };
                if let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) {
                    // Exact-path match against the binary we handed tauri:options.
                    // (A rebuilt binary would show a " (deleted)" exe suffix and not
                    // match — acceptable: we launched THIS on-disk binary.)
                    if exe == want && exe.ends_with(APP_BIN_NAME) {
                        return Some(pid);
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// True while a process with `pid` still exists (`kill(pid, 0)` succeeds).
pub fn pid_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
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
        Self::launch_with_envs(&[]).await
    }

    /// Like `launch()`, but applies `envs` to the tauri-driver `Command` —
    /// tauri-driver execs WebKitWebDriver, which execs the app, so the app
    /// inherits them. Used to relocate the app's `$HOME` / XDG dirs onto a
    /// tempdir for state-isolated GUI tests (gui_lifecycle.rs).
    pub async fn launch_with_envs(envs: &[(&str, &str)]) -> Gui {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let web_dir = manifest.parent().unwrap().join("web");
        let app_bin = manifest.join("target/debug/rust-agent-runtime-desktop");
        assert!(app_bin.exists(), "debug binary missing — run `cargo build` first");

        // 1. Vite: the debug binary renders from the devUrl, not bundled assets.
        //    Wrap in KillOnDrop immediately so a panic in wait_http reaps the process.
        let vite_guard = if http_ok(VITE_URL).await {
            None
        } else {
            let child = Command::new("npm")
                .args(["--prefix", web_dir.to_str().unwrap(), "run", "dev"])
                .process_group(0)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn vite (npm --prefix web run dev)");
            let guard = KillOnDrop(Some(child));
            wait_http(VITE_URL, Duration::from_secs(60), "vite").await;
            Some(guard)
        };

        // 2. tauri-driver on free ports; WEBKIT_WEBDRIVER env overrides the
        //    native driver path if WebKitWebDriver isn't on PATH.
        //    Wrap in KillOnDrop immediately so a panic in wait_http or WebDriver::new
        //    reaps the process without orphaning it.
        let port = free_port();
        let native_port = free_port();
        let mut cmd = Command::new("tauri-driver");
        cmd.args(["--port", &port.to_string(), "--native-port", &native_port.to_string()])
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        if let Ok(native) = std::env::var("WEBKIT_WEBDRIVER") {
            cmd.args(["--native-driver", &native]);
        }
        let tauri_driver_guard =
            KillOnDrop(Some(cmd.spawn().expect("spawn tauri-driver (is it on PATH? ~/.cargo/bin)")));
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

        // Session established — transfer ownership out of the guards into Gui.
        let tauri_driver = tauri_driver_guard.into_inner();
        let vite = vite_guard.map(|g| g.into_inner());
        // Discover the launched app PID by its exact binary path (never a broad
        // pattern) so the kill-restart drive can SIGKILL only this process.
        let app_pid = find_app_pid(app_bin.to_str().unwrap());
        Gui { driver, tauri_driver, vite, app_pid }
    }

    /// PID of the launched prod app process, if discovered.
    pub fn app_pid(&self) -> Option<i32> {
        self.app_pid
    }

    /// Hard-kill ONLY the launched app process (SIGKILL), simulating a daemon
    /// crash. tauri-driver / WebKitWebDriver / Vite are left alone; the caller
    /// still `shutdown()`s the (now app-less) WebDriver session afterwards.
    pub fn kill_app_hard(&self) {
        if let Some(pid) = self.app_pid {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
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
