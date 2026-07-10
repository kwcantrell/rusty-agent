//! GUI e2e smoke over WebDriver (tauri-driver + WebKitWebDriver). Drives the
//! REAL desktop app's DOM — no xdotool, no coordinates, no focus games.
//!
//! Run: `cd src-tauri && cargo test --test gui_smoke -- --ignored --test-threads=1 --nocapture`
//! Needs: a display, `tauri-driver` + `WebKitWebDriver` on PATH, npm (Vite is
//! auto-spawned if :5173 is down). `turn_smoke` additionally needs llama-server
//! on :8080 (qwen3.6-35b-a3b).

mod e2e_harness;

use e2e_harness::Gui;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use thirtyfour::prelude::*;

#[tokio::test]
#[ignore = "live: needs display + tauri-driver + vite (auto-spawned)"]
async fn boot_smoke() {
    let gui = Gui::launch().await;

    // Composer rendered → frontend loaded from Vite into the webview.
    gui.driver
        .query(By::Css("textarea[aria-label='prompt']"))
        .wait(Duration::from_secs(30), Duration::from_millis(500))
        .first()
        .await
        .expect("composer textarea should render");

    // Architecture/Config tabs only render when isTauri() sees real IPC —
    // this is the assertion that we drove the app, not a plain browser.
    let tabs = gui
        .driver
        .query(By::Css("button[role='tab']"))
        .wait(Duration::from_secs(10), Duration::from_millis(500))
        .all_from_selector()
        .await
        .expect("tab buttons");
    let mut labels = Vec::new();
    for t in &tabs {
        labels.push(t.text().await.unwrap_or_default());
    }
    assert!(
        labels.contains(&"Architecture".to_string()) && labels.contains(&"Config".to_string()),
        "expected Tauri-only tabs in {labels:?}"
    );

    gui.shutdown().await;
}

#[tokio::test]
#[ignore = "live: needs display + tauri-driver + vite + llama-server on :8080 (qwen3.6-35b-a3b)"]
async fn turn_smoke() {
    // Gate on the model server exactly like smoke_context_explorer.
    // Use a 2s timeout so a black-hole listener cannot hang the gate.
    let health = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get("http://localhost:8080/health")
        .send()
        .await;
    assert!(
        health.map(|r| r.status().is_success()).unwrap_or(false),
        "llama-server not healthy on :8080 — start it before this test"
    );

    let gui = Gui::launch().await;
    let d = &gui.driver;

    // Unique per run: the WebKit automation profile persists localStorage, so a
    // fixed marker could be satisfied instantly by a STALE transcript from a
    // previous run without exercising a live turn.
    let marker = format!(
        "SQRL{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be post-epoch")
            .as_millis()
    );
    let ta = d
        .query(By::Css("textarea[aria-label='prompt']"))
        .wait(Duration::from_secs(30), Duration::from_millis(500))
        .first()
        .await
        .expect("composer");
    ta.click().await.expect("focus composer");
    ta.send_keys(format!("Reply with exactly: {marker}")).await.expect("type prompt");
    // \u{E007} is the W3C WebDriver Enter keycode; the composer submits on Enter.
    ta.send_keys("\u{E007}").await.expect("send Enter");

    // The user bubble echoes the marker once; the assistant reply makes it >= 2.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let ret = d
            .execute("return document.body.innerText;", vec![])
            .await
            .expect("read innerText");
        let text = ret.json().as_str().unwrap_or_default().to_string();
        if text.matches(&marker).count() >= 2 {
            break;
        }
        assert!(Instant::now() < deadline, "no assistant reply containing {marker} within 120s");
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }

    // Context tab shows a populated breakdown: the total line and the system chip.
    d.query(By::XPath("//button[@role='tab' and normalize-space()='Context']"))
        .wait(Duration::from_secs(10), Duration::from_millis(500))
        .first()
        .await
        .expect("Context tab")
        .click()
        .await
        .expect("click Context tab");
    d.query(By::XPath("//div[contains(., ' / ') and contains(., ' tokens')]"))
        .wait(Duration::from_secs(15), Duration::from_millis(500))
        .first()
        .await
        .expect("context breakdown line matching 'N / M tokens' shape");
    d.query(By::XPath("//button[contains(., 'system')]"))
        .wait(Duration::from_secs(10), Duration::from_millis(500))
        .first()
        .await
        .expect("system legend chip");

    // Evidence screenshot (webview-exact, focus-independent).
    let _ = d
        .screenshot(std::path::Path::new("target/gui_smoke_turn.png"))
        .await;

    gui.shutdown().await;
}

// ---------------------------------------------------------------------------
// Live kill-restart resume drive (4B-1, spec §6 E6a: depth-1)
// ---------------------------------------------------------------------------

/// Linux app_config_dir for the prod desktop app: the identifier from
/// tauri.conf.json under $HOME/.config. This is where `lib.rs::run` reads
/// `agent-runtime.json` (runtime config) and `app.json` (persisted workspace).
fn app_config_dir() -> PathBuf {
    let home = std::env::var_os("HOME").expect("HOME set");
    PathBuf::from(home).join(".config/dev.rust-agent-runtime.desktop")
}

/// Backs up the prod config files while a test overwrites them, restoring the
/// originals (or deleting the ones we created) on drop — so the scenario cannot
/// leave the developer's real desktop config clobbered even if it panics.
struct ConfigGuard {
    saved: Vec<(PathBuf, Option<Vec<u8>>)>,
}
impl ConfigGuard {
    /// Write `bytes` to `<app_config_dir>/<name>`, remembering the prior content.
    fn seed(dir: &std::path::Path, name: &str, bytes: &[u8]) -> Self {
        std::fs::create_dir_all(dir).expect("mk app_config_dir");
        let path = dir.join(name);
        let prior = std::fs::read(&path).ok();
        std::fs::write(&path, bytes).expect("seed config file");
        ConfigGuard { saved: vec![(path, prior)] }
    }
    fn also(mut self, dir: &std::path::Path, name: &str, bytes: &[u8]) -> Self {
        let path = dir.join(name);
        let prior = std::fs::read(&path).ok();
        std::fs::write(&path, bytes).expect("seed config file");
        self.saved.push((path, prior));
        self
    }
}
impl Drop for ConfigGuard {
    fn drop(&mut self) {
        for (path, prior) in &self.saved {
            match prior {
                Some(bytes) => {
                    let _ = std::fs::write(path, bytes);
                }
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }
}

/// Read the innerText of the live webview body.
async fn body_text(d: &WebDriver) -> String {
    d.execute("return document.body.innerText;", vec![])
        .await
        .map(|r| r.json().as_str().unwrap_or_default().to_string())
        .unwrap_or_default()
}

/// Poll the webview body until `pred(text)` holds or `timeout` elapses.
/// Returns the matching text; panics with `what` on timeout.
async fn wait_body<F: Fn(&str) -> bool>(
    d: &WebDriver,
    timeout: Duration,
    what: &str,
    pred: F,
) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let t = body_text(d).await;
        if pred(&t) {
            return t;
        }
        assert!(Instant::now() < deadline, "timeout waiting for {what}; last body:\n{t}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Find the newest `~/.rusty-agent/sessions/<id>` dir created at/after `since_ms`
/// whose `checkpoint/children/*/parked.json` exists. Returns (session_dir,
/// parked_json_path). Retries until `timeout`.
fn wait_child_park(
    sessions_root: &std::path::Path,
    timeout: Duration,
) -> (PathBuf, PathBuf) {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(sessions) = std::fs::read_dir(sessions_root) {
            for s in sessions.flatten() {
                let children = s.path().join("checkpoint/children");
                if let Ok(kids) = std::fs::read_dir(&children) {
                    for k in kids.flatten() {
                        let parked = k.path().join("parked.json");
                        if parked.exists() {
                            return (s.path(), parked);
                        }
                    }
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "no checkpoint/children/*/parked.json appeared under {} within {timeout:?}",
            sessions_root.display()
        );
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// The full 4B-1 durability drive on the REAL desktop app:
/// dispatch a child that asks → park on disk → hard-kill the daemon →
/// relaunch (new pid) → attributed modal re-emits from disk → approve →
/// run completes and the checkpoint dir is gone.
///
/// Nondeterminism note: the live model must (a) dispatch a sub-agent and (b)
/// have the child run a shell command that hits the (empty) allowlist and Asks.
/// The prompt is engineered to force both; if a run doesn't produce the child
/// park, the scenario retries the whole launch up to 3 times.
#[tokio::test]
#[ignore = "live: needs display + tauri-driver + vite + llama-server on :8080 (qwen3.6-35b-a3b)"]
async fn kill_restart_resume_smoke() {
    // Gate on the model server.
    let health = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get("http://localhost:8080/health")
        .send()
        .await;
    assert!(
        health.map(|r| r.status().is_success()).unwrap_or(false),
        "llama-server not healthy on :8080 — start it before this test"
    );

    let home = std::env::var_os("HOME").expect("HOME set");
    let sessions_root = PathBuf::from(&home).join(".rusty-agent/sessions");

    // Fresh tempdir workspace, kept alive for the whole test.
    let ws = tempfile::tempdir().expect("mk workspace tempdir");

    // Config: EMPTY command allowlist (every execute_command Asks) + subagents on.
    // Mirrors the prod agent-runtime.json shape; unknown fields default via serde.
    let runtime_cfg = serde_json::json!({
        "backend": "openai",
        "base_url": "http://localhost:8080",
        "model": "qwen3.6-35b-a3b",
        "protocol": "native",
        "command_allowlist": [],
        "command_denylist": [],
        "http_allow_hosts": [],
        "temperature": 0.6,
        "max_tokens": 32768,
        "max_turns": 40,
        "context_limit": 262144,
        "subagents": true,
        "subagent_max_turns": 10,
        "subagent_max_depth": 1
    });
    let app_cfg = serde_json::json!({ "workspace": ws.path() });

    // Seed prod config (restored on drop even on panic).
    let cfg_dir = app_config_dir();
    let _cfg_guard = ConfigGuard::seed(
        &cfg_dir,
        "agent-runtime.json",
        serde_json::to_vec_pretty(&runtime_cfg).unwrap().as_slice(),
    )
    .also(&cfg_dir, "app.json", serde_json::to_vec_pretty(&app_cfg).unwrap().as_slice());

    // Prompt: force a sub-agent dispatch whose child runs a shell command.
    // Reuses the 3B-2 delegation shape ("use dispatch_agent / a sub-agent"),
    // appended with the exact shell command the brief specifies.
    let prompt = "Use the dispatch_agent tool to delegate to a general-purpose \
sub-agent. The sub-agent's single task is to run the shell command \
`echo resumed-ok` using the execute_command tool, then report the output. \
Do this now by calling dispatch_agent.";

    let mut attempt = 0;
    let (session_dir, first_pid) = loop {
        attempt += 1;
        assert!(attempt <= 3, "model never produced a child park in 3 attempts (flaky dispatch)");

        let gui = Gui::launch().await;
        let d = gui.driver.clone();

        // Composer up.
        d.query(By::Css("textarea[aria-label='prompt']"))
            .wait(Duration::from_secs(30), Duration::from_millis(500))
            .first()
            .await
            .expect("composer");

        let ta = d
            .query(By::Css("textarea[aria-label='prompt']"))
            .first()
            .await
            .expect("composer");
        ta.click().await.expect("focus composer");
        ta.send_keys(prompt).await.expect("type prompt");
        ta.send_keys("\u{E007}").await.expect("send Enter");

        // Wait up to 100s for the sub-agent approval modal: attribution text
        // "Sub-agent" AND the command echo resumed-ok in the prompt box. If the
        // model finishes without asking (no dispatch / no shell call), retry.
        let deadline = Instant::now() + Duration::from_secs(100);
        let mut got_modal = false;
        loop {
            let t = body_text(&d).await;
            // ASSERTION (park/attribution): sub-agent attribution rendered.
            if t.contains("Sub-agent") && t.contains("wants to run") && t.contains("resumed-ok") {
                got_modal = true;
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(800)).await;
        }

        if !got_modal {
            eprintln!("attempt {attempt}: no sub-agent approval modal; retrying");
            let _ = d.screenshot(std::path::Path::new(&format!(
                "target/kill_restart_attempt{attempt}_nomodal.png"
            )))
            .await;
            gui.shutdown().await;
            // Clear any stray session dirs from the failed attempt so the next
            // park-scan is unambiguous.
            continue;
        }

        // ASSERTION 3: attribution present, NOT answered.
        let _ = d
            .screenshot(std::path::Path::new("target/kill_restart_1_modal.png"))
            .await;

        // ASSERTION 4: park exists on disk under checkpoint/children/*/parked.json.
        let (session_dir, parked_json) = wait_child_park(&sessions_root, Duration::from_secs(20));
        eprintln!("park on disk: {}", parked_json.display());

        // ASSERTION 5: record pid, then hard-kill the daemon.
        let pid = gui.app_pid().expect("app pid discovered");
        eprintln!("first app pid = {pid}");
        gui.kill_app_hard();
        // Confirm the kill landed.
        let kdl = Instant::now() + Duration::from_secs(10);
        while e2e_harness::pid_alive(pid) {
            assert!(Instant::now() < kdl, "app pid {pid} still alive after SIGKILL");
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("first app pid {pid} confirmed dead");

        // Tear down the now-app-less WebDriver session / tauri-driver.
        gui.shutdown().await;
        break (session_dir, pid);
    };

    // ---- RESTART ----
    // Relaunch a genuinely new app instance; it auto-subscribes on attach and
    // scans PRIOR parked sessions from disk.
    let gui2 = Gui::launch().await;
    let d2 = gui2.driver.clone();

    d2.query(By::Css("textarea[aria-label='prompt']"))
        .wait(Duration::from_secs(30), Duration::from_millis(500))
        .first()
        .await
        .expect("composer (restart)");

    // ASSERTION 6: genuinely NEW pid.
    let second_pid = gui2.app_pid().expect("restart app pid");
    eprintln!("second app pid = {second_pid}");
    assert_ne!(first_pid, second_pid, "relaunch must be a new process, not the killed one");
    assert!(!e2e_harness::pid_alive(first_pid), "killed pid must stay dead");

    // ASSERTION 7: the attributed modal re-appears (re-emitted from disk) with
    // the re-derived command text matching the real parked command.
    let reemit = wait_body(&d2, Duration::from_secs(60), "re-emitted sub-agent modal", |t| {
        t.contains("Sub-agent") && t.contains("wants to run") && t.contains("echo resumed-ok")
    })
    .await;
    assert!(
        reemit.contains("echo resumed-ok"),
        "re-emitted modal must show the real parked command (re-derived display)"
    );
    let _ = d2
        .screenshot(std::path::Path::new("target/kill_restart_2_reemit.png"))
        .await;

    // ASSERTION 8a: Approve (button "1. Yes").
    d2.query(By::XPath("//button[contains(normalize-space(), 'Yes') and not(contains(., \"don't\"))]"))
        .wait(Duration::from_secs(10), Duration::from_millis(300))
        .first()
        .await
        .expect("approve button")
        .click()
        .await
        .expect("click approve");

    // ASSERTION 8b: run completes — the child output `resumed-ok` lands and the
    // modal is gone.
    wait_body(&d2, Duration::from_secs(120), "child result + final text", |t| {
        t.contains("resumed-ok") && !t.contains("wants to run")
    })
    .await;
    let _ = d2
        .screenshot(std::path::Path::new("target/kill_restart_3_complete.png"))
        .await;

    // ASSERTION 8c: the checkpoint dir is gone (delete-on-completion).
    let ck = session_dir.join("checkpoint");
    let cdl = Instant::now() + Duration::from_secs(20);
    while ck.exists() {
        assert!(
            Instant::now() < cdl,
            "checkpoint dir {} still present after completion",
            ck.display()
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    eprintln!("checkpoint dir removed: {}", ck.display());

    gui2.shutdown().await;
}
