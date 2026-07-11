//! Tier 3: live-model reality check. Everything else in this crate drives a
//! `ScriptedStub` (a deterministic fake OpenAI-compatible server); this test
//! is the ONE place that talks to a REAL model — llama-server on :8080
//! serving `qwen3.6-35b-a3b` — end to end through the same park/reopen path
//! scenario 5/6 etc. exercise against the stub. It stays `#[ignore]`: it
//! needs a live server and takes real wall-clock time for a real generation.
//!
//! Spec: docs/superpowers/specs/2026-07-10-e2e-lifecycle-stress-design.md §6
//! (Tier 3 + the one-retry allowance for model nondeterminism).
use agent_e2e::cli::CliCmd;
use agent_e2e::rig::{ckpt, wait_until, wait_until_async, Rig};
use std::time::Duration;

/// Live generation + approval interaction can run long; CAP is generous
/// on purpose (spec §6: "a couple of minutes live is fine, it's --ignored").
const CAP: Duration = Duration::from_secs(120);

const BASE_URL: &str = "http://localhost:8080";
const MODEL: &str = "qwen3.6-35b-a3b";

const PROMPT: &str = "Use the write_file tool to create pong.txt containing exactly: pong. \
     Do not ask questions, call the tool now.";
const FIRM_PROMPT: &str = "You MUST call the write_file tool with path pong.txt and content pong. \
     Reply with the tool call only.";

/// Drive the Session leg with `prompt` and wait up to `CAP` for a live
/// `ApprovalRequest` to land as a durable `parked.json`. Returns the session
/// dir on success, `None` on a timeout (caller decides whether to retry).
async fn try_park(rig: &Rig, prompt: &str) -> Option<std::path::PathBuf> {
    let (session, _cap) = rig.session_with_model(BASE_URL, MODEL);
    session.send_input(prompt.into());
    if !wait_until_async(CAP, || !rig.session_dirs().is_empty()).await {
        return None;
    }
    let dir = rig.only_session_dir();
    if !wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await {
        return None;
    }
    Some(dir)
}

/// Tier 3 reality check: a real model, prompted to call `write_file` (an
/// `Access::Write` tool), must actually emit a native tool call that the
/// runtime turns into a live `ApprovalRequest` + durable park — then a
/// SEPARATE process (`agent sessions reopen`) must load that park, re-drive
/// the approval prompt, and complete the write once approved.
///
/// One retry is allowed for the FIRST half only (model nondeterminism: the
/// model might answer in prose instead of calling the tool) — spec §6. The
/// reopen half gets no retry: once a park exists, reopening it is a
/// deterministic code path, not a model-sampling one.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: needs llama-server on :8080 (qwen3.6-35b-a3b)"]
async fn live_park_then_reopen_completes_pong_write() {
    // First attempt, original prompt, fresh rig.
    let rig = Rig::new();
    let (rig, dir) = match try_park(&rig, PROMPT).await {
        Some(dir) => (rig, dir),
        None => {
            eprintln!(
                "live_smoke: no ApprovalRequest within {CAP:?} on the first \
                 attempt (model answered in prose or otherwise didn't call the \
                 tool); retrying once with a firmer prompt (spec §6 one-retry \
                 allowance)"
            );
            // Tear down whatever the first attempt left behind before
            // retrying — a fresh Rig gives the retry a clean workspace/
            // session root. NO retry beyond this second attempt.
            let retry_rig = Rig::new();
            let dir = try_park(&retry_rig, FIRM_PROMPT).await.unwrap_or_else(|| {
                panic!(
                    "live_smoke: no ApprovalRequest within {CAP:?} even after \
                     the firmer-prompt retry — model did not call write_file \
                     twice in a row"
                )
            });
            (retry_rig, dir)
        }
    };
    finish_live_smoke(rig, dir).await;
}

/// Second half, factored out so both the first-try and retry paths share it:
/// assert the park, drop the Session leg, reopen via a fresh CLI process,
/// approve, wait for a clean exit, and assert the write actually landed.
async fn finish_live_smoke(rig: Rig, dir: std::path::PathBuf) {
    rig.assert_parked(&dir);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();

    // The in-process Session/Captured handles from `try_park` already went
    // out of scope when that fn returned, so the Session leg is already
    // gone — the reopen below loads the park from disk in a brand-new
    // process, exactly like a real restart.
    let mut cli = CliCmd::new(&rig, BASE_URL)
        .model(MODEL)
        // Live generation is slower than the stub; give the model-stream
        // idle watchdog a live-appropriate window instead of the harness's
        // stub-oriented 10s default.
        .stream_timeout_secs(120)
        .approval_timeout_secs(120)
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("y");
    let st = cli.wait_exit(CAP);
    let t = cli.transcript();
    assert!(st.success(), "reopen must exit 0 after approval:\n{t}");

    let pong = rig.workspace.path().join("pong.txt");
    assert!(
        wait_until(CAP, || pong.exists()),
        "pong.txt must exist in the rig workspace after the approved write:\n{t}"
    );
    let contents = std::fs::read_to_string(&pong).unwrap();
    assert_eq!(
        contents.trim(),
        "pong",
        "pong.txt must contain exactly 'pong', got {contents:?}:\n{t}"
    );
}
