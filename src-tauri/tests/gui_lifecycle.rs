//! Tier-2 GUI lifecycle checks (spec `2026-07-10-e2e-lifecycle-stress-design.md`
//! §3): real WebDriver DOM drives against a pre-seeded parked session (T2.1,
//! T2.2) and a live GUI→CLI cross-surface switch (T2.3).
//!
//! State relocation: the app reads `$HOME/.rusty-agent` (secret + sessions)
//! and its config via `app_config_dir` (XDG on Linux). `Gui::launch_with_envs`
//! applies `HOME`/`XDG_CONFIG_HOME`/etc to the tauri-driver spawn, which the
//! launched app inherits — characterized live on this machine (see
//! task-21-report.md): a seeded parked session under a tempdir HOME reliably
//! surfaces both the `ParkedBanner` AND a re-derived `ApprovalPrompt` (the
//! session's own `wire_parked_session` re-emits an `ApprovalRequest` for every
//! prior parked tree on attach — the banner itself carries no buttons; the
//! approve/deny/feedback UI is the same `ApprovalPrompt` the live session
//! uses).
//!
//! Run: `cd src-tauri && cargo test --test gui_lifecycle -- --ignored --test-threads=1 --nocapture`
//! Needs: display, tauri-driver + WebKitWebDriver on PATH, npm (vite auto-spawned),
//! and llama-server on :8080 (qwen3.6-35b-a3b) for all three tests (T2.1/T2.3 resume
//! a live turn against the app's configured model URL; T2.2's deny does not call the
//! model but shares the same launched-app precondition).

mod e2e_harness;

use e2e_harness::Gui;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use thirtyfour::prelude::*;

/// Tempdir-rooted fake HOME + the env vars to hand `Gui::launch_with_envs`.
struct FakeHome {
    dir: tempfile::TempDir,
}
impl FakeHome {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("mk fake home");
        std::fs::create_dir_all(dir.path().join(".config")).expect("mk .config");
        std::fs::create_dir_all(dir.path().join(".rusty-agent/sessions"))
            .expect("mk .rusty-agent/sessions");
        FakeHome { dir }
    }
    fn home(&self) -> &Path {
        self.dir.path()
    }
    fn meta(&self) -> PathBuf {
        self.dir.path().join(".rusty-agent")
    }
    fn sessions(&self) -> PathBuf {
        self.meta().join("sessions")
    }
    fn workspace(&self) -> PathBuf {
        let ws = self.dir.path().join("ws");
        std::fs::create_dir_all(&ws).expect("mk ws");
        ws
    }
    /// `(key, value)` pairs for `Gui::launch_with_envs` — owns the strings so
    /// the caller can build the `&[(&str, &str)]` slice against them.
    fn envs(&self) -> [(String, String); 4] {
        let home = self.home().to_str().unwrap().to_string();
        let xdg = self.home().join(".config").to_str().unwrap().to_string();
        [
            ("HOME".to_string(), home),
            ("XDG_CONFIG_HOME".to_string(), xdg.clone()),
            ("XDG_DATA_HOME".to_string(), xdg.clone()),
            ("XDG_CACHE_HOME".to_string(), xdg),
        ]
    }
}

fn envs_slice(pairs: &[(String, String)]) -> Vec<(&str, &str)> {
    pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

async fn llama_health_ok() -> bool {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get("http://localhost:8080/health")
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn body_text(d: &WebDriver) -> String {
    d.execute("return document.body.innerText;", vec![])
        .await
        .map(|r| r.json().as_str().unwrap_or_default().to_string())
        .unwrap_or_default()
}

async fn wait_body<F: Fn(&str) -> bool>(d: &WebDriver, timeout: Duration, what: &str, pred: F) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let t = body_text(d).await;
        if pred(&t) {
            return t;
        }
        assert!(Instant::now() < deadline, "timeout waiting for {what}; last body:\n{t}");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

/// Poll for `answer.json` under a checkpoint dir — a real, brief window: the
/// approval-resolved handler writes it in one spawned task, and the resume
/// path consumes it (reads + deletes) in a second spawned task shortly after
/// (session.rs `start_resume`). Polls tightly since the window can be small.
fn wait_answer_json(checkpoint_dir: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let path = checkpoint_dir.join("answer.json");
    // Tight spin, no sleep: on this machine the commit (write_answer) and
    // consume (take_answer) run back-to-back in already-spawned tokio tasks
    // with no network I/O between them, so the on-disk window can be well
    // under a millisecond — characterized live (task-21-report.md): a 2ms
    // poll interval reliably missed it. A bare spin loop (~microsecond
    // resolution) is what actually observes the write.
    loop {
        if path.exists() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::hint::spin_loop();
    }
}

// ---------------------------------------------------------------------------
// T2.1 — parked-run banner + DOM approve
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "GUI: needs tauri-driver + display (+ :8080 for turns)"]
async fn t21_banner_and_dom_approve() {
    assert!(
        llama_health_ok().await,
        "llama-server not healthy on :8080 — start it before this test"
    );

    let home = FakeHome::new();
    let (sess, _cap, _key) =
        agent_server::testkit::plant_parked_session(&home.workspace(), &home.sessions(), &home.meta(), "t21-prior")
            .await;
    drop(sess); // the planted Session was only a vehicle to write the checkpoint

    let checkpoint_dir = home.sessions().join("t21-prior").join("checkpoint");
    assert!(checkpoint_dir.join("parked.json").exists(), "positive-artifact check: park must exist before launch");

    let pairs = home.envs();
    let gui = Gui::launch_with_envs(&envs_slice(&pairs)).await;
    let d = &gui.driver;

    d.query(By::Css("textarea[aria-label='prompt']"))
        .wait(Duration::from_secs(30), Duration::from_millis(500))
        .first()
        .await
        .expect("composer");

    // Banner: informational, lists the prior parked session.
    let banner = d
        .query(By::Css("[data-testid='parked-banner']"))
        .wait(Duration::from_secs(20), Duration::from_millis(500))
        .first()
        .await
        .expect("parked banner should render for the seeded prior session");
    let banner_text = banner.text().await.unwrap_or_default();
    assert!(
        banner_text.contains("t21-prior"),
        "banner should name the seeded session id; got: {banner_text}"
    );

    // The banner carries no per-run controls; the re-derived ApprovalPrompt
    // (same component the live session uses) is what approve/deny/feedback
    // acts on. Click "1. Yes" (never "don't ask again" — exact text match).
    let approve_btn = d
        .query(By::XPath(
            "//button[contains(normalize-space(), 'Yes') and not(contains(., \"don't\"))]",
        ))
        .wait(Duration::from_secs(15), Duration::from_millis(300))
        .first()
        .await
        .expect("approve (Yes) button for the re-derived parked ask");
    approve_btn.click().await.expect("click approve");

    // Banner clears once `resumed` fires and filters it out of parkedRuns.
    wait_body(d, Duration::from_secs(15), "banner to clear after approve", |t| {
        !t.contains("t21-prior")
    })
    .await;
    let cleared = d.query(By::Css("[data-testid='parked-banner']")).first().await;
    assert!(cleared.is_err(), "banner element should be gone from the DOM after approve+resume");

    // No crash: composer still present/responsive.
    d.query(By::Css("textarea[aria-label='prompt']"))
        .first()
        .await
        .expect("composer still present after resume — no crash");

    let _ = d.screenshot(Path::new("target/gui_lifecycle_t21.png")).await;
    gui.shutdown().await;
}

// ---------------------------------------------------------------------------
// T2.2 — deny with feedback typed in the real DOM
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "GUI: needs tauri-driver + display (+ :8080 for turns)"]
async fn t22_deny_with_feedback_commits_answer() {
    assert!(
        llama_health_ok().await,
        "llama-server not healthy on :8080 — start it before this test"
    );

    let home = FakeHome::new();
    let (sess, _cap, _key) =
        agent_server::testkit::plant_parked_session(&home.workspace(), &home.sessions(), &home.meta(), "t22-prior")
            .await;
    drop(sess);

    let checkpoint_dir = home.sessions().join("t22-prior").join("checkpoint");
    assert!(checkpoint_dir.join("parked.json").exists(), "positive-artifact check: park must exist before launch");

    let pairs = home.envs();
    let gui = Gui::launch_with_envs(&envs_slice(&pairs)).await;
    let d = &gui.driver;

    d.query(By::Css("textarea[aria-label='prompt']"))
        .wait(Duration::from_secs(30), Duration::from_millis(500))
        .first()
        .await
        .expect("composer");

    d.query(By::Css("[data-testid='parked-banner']"))
        .wait(Duration::from_secs(20), Duration::from_millis(500))
        .first()
        .await
        .expect("parked banner should render for the seeded prior session");

    // Type feedback into the approval prompt's feedback input (placeholder
    // "optional feedback if denying" — no aria-label on this input; select by
    // the stable placeholder text since it is the only such input rendered).
    let feedback_input = d
        .query(By::Css("input[placeholder='optional feedback if denying']"))
        .wait(Duration::from_secs(15), Duration::from_millis(300))
        .first()
        .await
        .expect("feedback input on the re-derived ApprovalPrompt");
    feedback_input.click().await.expect("focus feedback input");
    feedback_input.send_keys("SQRL-T2-FB").await.expect("type feedback");

    // Spawn the filesystem watcher for `answer.json` BEFORE clicking Deny: the
    // commit-then-consume round trip (session.rs `write_answer` → `start_resume`
    // → `take_answer`) runs entirely in already-spawned tokio tasks with no
    // network I/O gating it, so on this machine it completes in well under a
    // millisecond — faster than a second WebDriver round trip could observe
    // post-hoc. Racing the watcher against the click (rather than polling
    // after `deny_btn.click().await` returns) is what makes the write
    // observable at all.
    let watcher_dir = checkpoint_dir.clone();
    let watcher = std::thread::spawn(move || wait_answer_json(&watcher_dir, Duration::from_secs(15)));

    // Deny ("3. No") — the component reads the feedback field's current value.
    let deny_btn = d
        .query(By::XPath("//button[contains(normalize-space(), 'No')]"))
        .wait(Duration::from_secs(10), Duration::from_millis(300))
        .first()
        .await
        .expect("deny (No) button");
    deny_btn.click().await.expect("click deny");

    // MAC-bound deny committed from the real DOM: answer.json appeared in the
    // seeded session's checkpoint dir (observed by the racing watcher above,
    // not a post-hoc poll — see comment).
    assert!(
        watcher.join().expect("watcher thread"),
        "answer.json never appeared under {} after DOM deny+feedback",
        checkpoint_dir.display()
    );

    let _ = d.screenshot(Path::new("target/gui_lifecycle_t22.png")).await;
    gui.shutdown().await;
}

// ---------------------------------------------------------------------------
// T2.3 — real GUI-driven park, CLI sees it, CLI reopen resumes it
// ---------------------------------------------------------------------------

/// Resolve+build the `agent` CLI binary fresh (mirrors the spec's
/// binary-freshness rule, §2.2 item 5 — a bare target-path lookup can
/// silently run a stale binary). `agent-cli`'s crate is in the SEPARATE
/// `agent/` workspace, so this cannot come from `CARGO_BIN_EXE_agent` here.
fn build_agent_cli() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let agent_dir = manifest.parent().unwrap().join("agent");
    let status = Command::new("cargo")
        .args(["build", "-p", "agent-cli", "--quiet"])
        .current_dir(&agent_dir)
        .status()
        .expect("spawn cargo build -p agent-cli");
    assert!(status.success(), "cargo build -p agent-cli failed");
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| agent_dir.join("target"));
    target_dir.join("debug/agent")
}

#[tokio::test]
#[ignore = "GUI: needs tauri-driver + display (+ :8080 for turns)"]
async fn t23_gui_park_then_cli_lists_and_reopens() {
    assert!(
        llama_health_ok().await,
        "llama-server not healthy on :8080 — start it before this test"
    );

    let agent_bin = build_agent_cli();

    let home = FakeHome::new();
    let pairs = home.envs();
    let gui = Gui::launch_with_envs(&envs_slice(&pairs)).await;
    let d = gui.driver.clone();

    let ta = d
        .query(By::Css("textarea[aria-label='prompt']"))
        .wait(Duration::from_secs(30), Duration::from_millis(500))
        .first()
        .await
        .expect("composer");
    ta.click().await.expect("focus composer");
    ta.send_keys(
        "Use the write_file tool to create t23.txt in the workspace root with the \
         content \"hello\". Call write_file now.",
    )
    .await
    .expect("type prompt");
    ta.send_keys("\u{E007}").await.expect("send Enter"); // W3C Enter, submits

    // In-app approval prompt: write_file gates deterministically (Access::Write ⇒ Ask).
    wait_body(&d, Duration::from_secs(60), "write_file approval prompt", |t| {
        t.contains("write") && t.contains("t23.txt") && t.contains("Yes")
    })
    .await;
    let _ = d.screenshot(Path::new("target/gui_lifecycle_t23_prompt.png")).await;

    // End the WebDriver session WITHOUT answering — the park is on disk
    // (session.rs writes parked.json before ever emitting ApprovalRequest).
    gui.shutdown().await;

    // `agent sessions list` against the relocated roots must show the parked session.
    let sessions_dir = home.sessions();
    let meta_dir = home.meta();
    let list_out = Command::new(&agent_bin)
        .args([
            "--trace-dir",
            sessions_dir.to_str().unwrap(),
            "--metadata-dir",
            meta_dir.to_str().unwrap(),
            "sessions",
            "list",
        ])
        .env("HOME", home.home())
        .output()
        .expect("run agent sessions list");
    let list_stdout = String::from_utf8_lossy(&list_out.stdout);
    assert!(
        list_out.status.success(),
        "agent sessions list failed: stdout={list_stdout} stderr={}",
        String::from_utf8_lossy(&list_out.stderr)
    );
    assert!(
        list_stdout.contains("[PARKED]"),
        "expected a [PARKED] session in `sessions list` output:\n{list_stdout}"
    );
    let session_id = list_stdout
        .lines()
        .find(|l| l.contains("[PARKED]"))
        .and_then(|l| l.split_whitespace().next())
        .expect("parse session id from the PARKED line")
        .to_string();
    eprintln!("CLI sees parked session: {session_id}");

    // `agent sessions reopen <id>` against the same roots, approving 'y'.
    let mut child = Command::new(&agent_bin)
        .args([
            "--base-url",
            "http://localhost:8080",
            "--model",
            "qwen3.6-35b-a3b",
            "--stream-timeout-secs",
            "120",
            "--trace-dir",
            sessions_dir.to_str().unwrap(),
            "--metadata-dir",
            meta_dir.to_str().unwrap(),
            "sessions",
            "reopen",
            &session_id,
        ])
        .env("HOME", home.home())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent sessions reopen");

    // Drain stdout/stderr on background threads concurrently with the write
    // + wait below — a reopen that streams tokens could otherwise fill the
    // pipe buffer and deadlock the child against an unread pipe.
    use std::io::Read;
    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let mut stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout_pipe.read_to_string(&mut buf);
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stderr_pipe.read_to_string(&mut buf);
        buf
    });

    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .expect("piped stdin")
            .write_all(b"y\n")
            .expect("write approve line");
        // Drop stdin so the child's blocking read sees EOF after the answer
        // line rather than waiting on a pipe we'll never write to again.
        drop(child.stdin.take());
    }

    // Deadline-bounded wait (§2.4 watchdog policy): SIGKILL on expiry rather
    // than hang a pre-push-adjacent run forever.
    let deadline = Instant::now() + Duration::from_secs(150);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("agent sessions reopen {session_id} did not exit within 150s — killed");
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let stdout_buf = stdout_thread.join().unwrap_or_default();
    let stderr_buf = stderr_thread.join().unwrap_or_default();
    assert!(
        status.success(),
        "agent sessions reopen {session_id} exited non-zero: stdout={stdout_buf} stderr={stderr_buf}"
    );
}
