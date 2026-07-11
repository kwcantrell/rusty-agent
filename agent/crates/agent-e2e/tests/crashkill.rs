//! [real-kill] scenarios 5-6: SIGINT mid-resume cancels clean with no park
//! to retain (characterized below — not the `76d81d5` regression); a
//! committed answer survives a real kill and is consumed exactly once.
//! Scenarios 10-13 (Task 15): torn checkpoint refusal, stale-lock contention,
//! a real daemon-process SIGKILL + restart re-emit/resume, and a degraded
//! (write-blocked) park that stays live-only.
//! Spec: docs/superpowers/specs/2026-07-10-e2e-lifecycle-stress-design.md.
use agent_e2e::cli::{CliCmd, DaemonCmd};
use agent_e2e::rig::{ckpt, wait_until, wait_until_async, Rig};
use agent_e2e::stub::{gated_write, text_step, ScriptStep, ScriptedStub, StubResponse};
use agent_server::wire::ServerEvent;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

const CAP: Duration = Duration::from_secs(30);

/// Scenario 5: park via Session -> CLI `sessions reopen` -> approve -> the
/// resume's NEXT model request stalls (DelayedText) so SIGINT lands mid-resume.
///
/// **Characterized (not the `76d81d5` regression):** at consume-time,
/// `tool_phase`'s answered-ask arm (loop_.rs) clears the root park BEFORE the
/// approved `write_file` call executes ("CONSUME-TIME COMMIT... a crash after
/// this point loses the run from here (D1); it never re-prompts an
/// already-consumed approval, so the approved call can never execute twice" —
/// an explicitly documented, accepted tradeoff, not a bug). The delayed step
/// here is a plain-text model response, not a gated tool call, so by the time
/// SIGINT lands the write_file call has already executed and there is no new
/// Ask to re-park at — `turn_loop`'s cancellation arm
/// (`Err(CompletionFailure::Cancelled) => { Done(Cancelled); return Ok(()) }`)
/// returns `Ok(())` with nothing durable to retain. `resume_cleanup_decision`
/// correctly routes `Ok(()) + cancelled` to `Retain`, but the `Retain` arm's
/// own `has_park` check (mirroring the REPL's Ctrl-C handler) finds no park
/// and prints "run cancelled" rather than "run left parked" — verified via a
/// throwaway probe run: exit 0, transcript contains "run cancelled", both
/// `parked.json` and `resume.lock` are gone. This is scenario 5's actual,
/// pinned behavior: SIGINT mid-model-call cancels cleanly with no resumable
/// state, matching `resume_cleanup_decision`'s documented Ok+cancelled path.
/// This test deliberately uses exactly 2 steps: the initial gated request and
/// the delayed text response. The run never reaches a 3rd request (cancelled
/// first), so both steps are consumed; `assert_eq!(stub.recorded().len(), 2)`
/// is the closing check.
#[tokio::test(flavor = "multi_thread")]
async fn s05_sigint_mid_resume_cancels_clean_no_park_to_retain() {
    let stub = ScriptedStub::start(vec![
        gated_write("SQRL-5 go"),
        // resume's model request stalls 20s so SIGINT lands mid-resume
        ScriptStep {
            expect_substring: None,
            respond: StubResponse::DelayedText {
                text: "slow".into(),
                delay_ms: 20_000,
            },
        },
    ])
    .await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-5 go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();

    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("y");
    // Synchronize on the resume actually being in flight: the stub records
    // the (delayed) resume request.
    assert!(wait_until(CAP, || stub.recorded().len() >= 2));
    cli.sigint();
    let st = cli.wait_exit(CAP);
    let t = cli.transcript();
    // Pinned: SIGINT mid-model-call cancels cleanly (exit 0) with no
    // resumable state — the answer was already consumed before the (now
    // executed) write_file call, and the delayed step never yields a new Ask.
    assert!(st.success(), "{t}");
    assert!(
        t.contains("run cancelled"),
        "expected the parkless-cancel message, got:\n{t}"
    );
    assert!(
        !ckpt(&dir).join("parked.json").exists(),
        "no gate-kind park is pending at the cancel point; nothing to retain"
    );
    assert!(
        wait_until(CAP, || !ckpt(&dir).join("resume.lock").exists()),
        "resume.lock must be released on cancel"
    );
    // Script's 2nd step (the delayed text) was consumed by the request that
    // stalled; the run never issues a 3rd request (cancelled first). Only
    // 2 of the 2 scripted steps for this run are ever hit.
    assert_eq!(stub.recorded().len(), 2, "transcript:\n{t}");
    stub.assert_consumed();
}

/// Scenario 5b: the actual `76d81d5`/`01179d8` C-1 regression window — cancel
/// while RE-PARKED AT A NEW ASK mid-resume. Unlike s05 (whose delayed step was
/// plain text, so the approved `write_file` had already executed and there was
/// no new Ask left to re-park at), this script's resume issues a SECOND gated
/// tool call after the first approval, so the resumed run genuinely re-parks
/// and re-prompts (Task 10 characterization: a reopen resume can loop through
/// more than one live approval prompt in the SAME process). Cancelling while
/// that second prompt is outstanding must retain the park — the opposite
/// polarity of s05's "run cancelled": `resume_cleanup_decision` routes
/// `Ok(()) + cancelled` to `Retain`, and the `Retain` arm's `has_park` check
/// must find the re-park and print "run left parked" instead.
#[tokio::test(flavor = "multi_thread")]
async fn s05b_sigint_while_reparked_at_new_ask_retains_park() {
    let stub = ScriptedStub::start(vec![
        gated_write("SQRL-5B go"),
        // resume's first live request after the approval: a SECOND gated call.
        gated_write("SQRL-5B go"),
        // eventual completion after the second reopen approves it too.
        text_step(None, "done"),
    ])
    .await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-5B go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();

    // approval_timeout_secs(60): the SECOND prompt must not time out under load
    // while we drive the first answer + wait for the re-park to land.
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .approval_timeout_secs(60)
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("y");
    // The resume executes write#1, then the stub's 2nd script step (another
    // gated write_file) lands as the resume's next model request -> re-parks
    // and re-prompts in the SAME process. Wait for the SECOND prompt
    // occurrence — the first prompt's text is still in the transcript, so
    // count occurrences rather than just `contains`. `wait_until`'s poll
    // closure is `Fn`, and `transcript()` takes `&mut self`, so drive the
    // bounded loop by hand here instead.
    {
        let start = std::time::Instant::now();
        loop {
            let hits = cli.transcript().matches("[y]es / [n]o / [a]lways:").count();
            if hits >= 2 {
                break;
            }
            assert!(
                start.elapsed() < CAP,
                "deadline waiting for the second approval prompt; transcript:\n{}",
                cli.transcript()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    // Positive-artifact check BEFORE the kill: the re-park actually landed on
    // disk, not just in the transcript.
    assert!(wait_until(CAP, || ckpt(&dir).join("parked.json").exists()));
    // Do NOT answer the second prompt — cancel while it's outstanding.
    cli.sigint();
    let st = cli.wait_exit(CAP);
    let t = cli.transcript();
    assert!(st.success(), "{t}");
    assert!(
        t.contains("run left parked; answer later with"),
        "expected the has_park-gated retain message, got:\n{t}"
    );
    assert!(
        ckpt(&dir).join("parked.json").exists(),
        "the re-park at the new ask must survive the cancel"
    );
    assert!(
        wait_until(CAP, || !ckpt(&dir).join("resume.lock").exists()),
        "resume.lock must be released on the retained-park cancel path"
    );

    // Second reopen: answer the still-outstanding (re-parked) ask, run to
    // completion, and consume the final text step.
    let mut cli2 = CliCmd::new(&rig, &stub.base_url())
        .approval_timeout_secs(60)
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli2.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli2.write_line("y");
    let st2 = cli2.wait_exit(CAP);
    assert!(st2.success(), "{}", cli2.transcript());
    stub.assert_consumed();
    assert!(
        !ckpt(&dir).join("parked.json").exists(),
        "park must be fully consumed after the second reopen completes"
    );
}

/// Scenario 6a: reopen reaches the approval prompt (claims the resume lock),
/// gets SIGKILL'd before answering -> park retained, lock leaked (documented
/// stale-lock behavior). Clearing the lock the way the product's own error
/// message instructs makes the session reopenable again.
#[tokio::test(flavor = "multi_thread")]
async fn s06a_reopen_killed_while_parked_leaves_park_reopenable() {
    let stub = ScriptedStub::start(vec![gated_write("SQRL-6 go"), text_step(None, "done")]).await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-6 go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();
    // Real kill: reopen reaches the prompt (lock held), SIGKILL — no answer.
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.sigkill();
    let _ = cli.wait_exit(CAP);
    rig.assert_parked(&dir);
    // SIGKILL leaves resume.lock — documented stale-lock behavior; clear it the
    // way the product's own error message instructs, then prove reopenability.
    // PRODUCT-GAP: no auto-recovery for stale locks (spec scenario 11 asserts
    // the contention message; here we just clear and move on).
    std::fs::remove_file(ckpt(&dir).join("resume.lock")).unwrap();
    let mut cli2 = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli2.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli2.write_line("y");
    assert!(cli2.wait_exit(CAP).success(), "{}", cli2.transcript());
    stub.assert_consumed();
}

/// Scenario 6b: an answer committed by the real writer (standing in for a
/// process killed between commit and consume — spec §2.4 sanctioned
/// descope, see the Panel & review log) is consumed by the next reopen
/// without ever re-prompting.
#[tokio::test(flavor = "multi_thread")]
async fn s06b_committed_answer_consumed_without_reprompt() {
    // The write_answer->take_answer window is sub-millisecond and CPU-bound —
    // unhittable from outside (spec §2.4 descope: the state is produced by the
    // REAL writer, agent_core::checkpoint::write_answer, standing in for
    // "killed between commit and consume"). RECORDED in the spec's Panel &
    // review log for owner sign-off at branch review.
    let stub = ScriptedStub::start(vec![gated_write("SQRL-6B go"), text_step(None, "done")]).await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-6B go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    // Commit an approve answer with the real writer + the rig's real key.
    // (Exact fn: agent_core::checkpoint::write_answer — see
    // agent-core/src/checkpoint.rs; agent_server::resume::commit_answer, the
    // call `run_sessions_reopen` makes, bottoms out into this same function.)
    agent_e2e::forge::commit_answer_like_cli(&ckpt(&dir), &rig.key, /* approve= */ true, None);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    let st = cli.wait_exit(CAP);
    let t = cli.transcript();
    assert!(st.success(), "{t}");
    assert!(
        !t.contains("[y]es / [n]o"),
        "must NOT re-prompt — answer was committed:\n{t}"
    );
    stub.assert_consumed();
    assert!(
        !ckpt(&dir).join("answer.json").exists(),
        "answer must be consumed"
    );
}

/// Scenario 10: a torn checkpoint tree (payload written, manifest missing) is
/// refused cleanly rather than half-loaded. `write_park` in
/// `agent-core/src/checkpoint.rs` writes the manifest LAST on purpose — "its
/// presence marks a complete tree (a crash mid-write leaves no manifest ⇒
/// load refuses as corrupt ⇒ spec §4 torn-tree row)" — so deleting
/// `manifest.json` after a real park synthesizes exactly the state a crash
/// between the payload writes and the manifest write would leave. The
/// companion unit test pinning this same shape from the write side is
/// `wrong_key_and_missing_manifest_refuse` (`agent-core/src/checkpoint.rs`),
/// which removes `manifest.json` and asserts `CheckpointError::Corrupt`.
/// `CheckpointError::Corrupt`'s `#[error(...)]` Display is
/// `"checkpoint corrupt: {0}"`, so "corrupt" is a stable substring of
/// whatever reaches the CLI's `eprintln!` in `run_sessions_reopen`
/// (agent-cli/src/main.rs): `"checkpoint unreadable; run cannot be resumed
/// ({e})"`.
#[tokio::test(flavor = "multi_thread")]
async fn s10_torn_checkpoint_refused_list_unaffected() {
    let stub = ScriptedStub::start(vec![gated_write("SQRL-10 go"), text_step(None, "done")]).await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-10 go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();

    // Synthesize the torn tree: payload written, manifest never lands — the
    // §2.4-sanctioned stand-in for a crash between the two write phases.
    std::fs::remove_file(ckpt(&dir).join("manifest.json")).unwrap();

    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    let st = cli.wait_exit(CAP);
    let t = cli.transcript();
    assert!(!st.success(), "torn checkpoint must be refused:\n{t}");
    // Pinned stable substring — see the Corrupt Display note above.
    assert!(
        t.contains("corrupt"),
        "expected the corrupt-checkpoint refusal text, got:\n{t}"
    );
    assert!(
        !t.contains("panicked"),
        "refusal must be a clean error path, not a panic:\n{t}"
    );
    // The session dir (and its surviving payload artifacts) must be left
    // alone — refusal is read-only.
    assert!(dir.is_dir(), "session dir must survive a refused reopen");
    assert!(
        ckpt(&dir).join("parked.json").exists(),
        "torn tree's payload files are untouched by a refused load"
    );

    // `sessions list` must not choke on the same torn tree.
    let mut cli2 = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "list"])
        .spawn();
    let st2 = cli2.wait_exit(CAP);
    assert!(st2.success(), "{}", cli2.transcript());
    // Script never runs to completion (refused before any resume) — spare by
    // design; `assert_consumed` would be wrong here.
    let _ = stub.recorded();
}

/// Scenario 11: [real-kill] a live `resume.lock` holder that gets SIGKILL'd
/// leaves the lock file behind (no `Drop`-based release survives SIGKILL),
/// so a subsequent reopen must refuse with a contention message rather than
/// double-resume the same tree. `DaemonCmd::hold_lock` claims the resume
/// lock and prints "LOCKED" without ever touching the checkpoint payload —
/// `--dir` here MUST be the checkpoint dir (F2), matching `claim_resume`'s
/// own signature.
#[tokio::test(flavor = "multi_thread")]
async fn s11_stale_lock_after_real_kill_refuses_contention() {
    let stub = ScriptedStub::start(vec![gated_write("SQRL-11 go"), text_step(None, "done")]).await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-11 go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();

    let mut holder = DaemonCmd::hold_lock(&rig, &ckpt(&dir));
    holder.wait_for_output("LOCKED", CAP);
    assert!(ckpt(&dir).join("resume.lock").exists());
    holder.sigkill();
    let _ = holder.wait_exit(CAP);
    // SIGKILL cannot run any Drop-based lock release — resume.lock is
    // orphaned on disk exactly like s06a's kill-while-parked case.
    assert!(
        ckpt(&dir).join("resume.lock").exists(),
        "stale lock must remain after the holder is killed"
    );

    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    let st = cli.wait_exit(CAP);
    let t = cli.transcript();
    assert_eq!(st.code(), Some(2), "transcript:\n{t}");
    assert!(
        t.contains("is being resumed elsewhere"),
        "expected the stale-lock contention message, got:\n{t}"
    );
    assert!(!t.contains("panicked"), "must be a clean refusal:\n{t}");
    rig.assert_parked(&dir);
    // PRODUCT-GAP: stale resume.lock requires manual removal (spec §5#11).
    let _ = stub.recorded();
}

/// Scenario 12: [real-kill] a real daemon PROCESS (not just the in-process
/// `Session` leg) gets SIGKILL'd while parked on a live approval, and a
/// SECOND, freshly-started daemon process must classify + own the first
/// process's session dir, re-derive the same `ApprovalRequest`, and resume
/// it to completion after being answered.
#[tokio::test(flavor = "multi_thread")]
async fn s12_daemon_sigkill_then_restart_reemits_and_resumes() {
    let stub = ScriptedStub::start(vec![gated_write("SQRL-12 go"), text_step(None, "done")]).await;
    let rig = Rig::new();
    let mut d1 = DaemonCmd::run(&rig, &stub.base_url(), Some("SQRL-12 go"));
    d1.wait_for_output("READY", CAP);
    d1.wait_for_event(CAP, |e| matches!(e, ServerEvent::ApprovalRequest { .. }));
    // Capture the FIRST process's session dir/id BEFORE the kill — the second
    // daemon constructs its OWN new session dir too (one per process start),
    // so the ParkedRuns assert below must be pinned to this specific id, not
    // "any parked run".
    let dir = rig
        .session_dirs()
        .into_iter()
        .find(|d| ckpt(d).join("parked.json").exists())
        .expect("park on disk");
    let parked_sid = dir.file_name().unwrap().to_string_lossy().into_owned();
    d1.sigkill();
    let _ = d1.wait_exit(CAP);

    let mut d2 = DaemonCmd::run(&rig, &stub.base_url(), None);
    d2.wait_for_output("READY", CAP);
    let ev = d2.wait_for_event(
        CAP,
        |e| matches!(e, ServerEvent::ParkedRuns { runs } if !runs.is_empty()),
    );
    let ServerEvent::ParkedRuns { runs } = ev else {
        unreachable!()
    };
    // Epoch/ownership evidence: the restarted (second) process classifies and
    // owns a session dir it did not create — it never called `session()` or
    // sent a task for `parked_sid`; it only scanned `rig.sessions` on startup.
    assert!(
        runs.iter().any(|r| r.session_id == parked_sid),
        "restarted process must classify+own the prior process's dir; runs={runs:?}"
    );
    let ask = d2.wait_for_event(CAP, |e| matches!(e, ServerEvent::ApprovalRequest { .. }));
    let ServerEvent::ApprovalRequest { id, .. } = ask else {
        unreachable!()
    };
    d2.write_line(&format!("approve {id}"));
    d2.wait_for_event(CAP, |e| matches!(e, ServerEvent::Resumed { .. }));
    d2.wait_for_event(CAP, |e| matches!(e, ServerEvent::Done { .. }));
    stub.assert_consumed();
}

/// Small RAII guard: restores a chmod'd directory's permissions on drop so
/// `TempDir`'s own teardown (which needs write+execute to unlink children)
/// still succeeds even if this test panics partway through.
struct RestorePerms(std::path::PathBuf);
impl Drop for RestorePerms {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

/// Scenario 13: a park write that can't land on disk degrades to live-only
/// rather than losing the approval prompt or wedging the run. `write_park`
/// creates the checkpoint dir with `create_dir_0700` — with the SESSION dir
/// itself (its parent) chmod'd 0o555 (no write/execute-for-create bit), that
/// `create_dir_all` fails at the "create the `checkpoint/` entry inside the
/// session dir" step (confirmed by content: the session dir pre-exists —
/// `Session::from_params`/`send_input` create it before any gate fires — so a
/// read-only *sessions root* would never be touched here; per plan-review F5
/// it's the session dir's own permissions that must block the create). The
/// write failure is caught in `loop_.rs`'s `NeedsApproval` arm and downgraded
/// to `AgentEvent::Error("checkpoint write failed (approval not durable):
/// {e}")` (content-search "approval not durable") — the approval prompt still
/// fires from the SAME arm, live-only, and the run must still complete once
/// answered.
#[tokio::test(flavor = "multi_thread")]
async fn s13_park_write_failure_degrades_to_live_only() {
    use agent_server::wire::Decision;

    let stub = ScriptedStub::start(vec![gated_write("SQRL-13 go"), text_step(None, "done")]).await;
    let rig = Rig::new();
    let (session, cap) = rig.session(&stub.base_url());
    // Constructing the Session creates its session dir (descriptor write) up
    // front, before any task is sent — lock it down now.
    assert!(!rig.session_dirs().is_empty());
    let dir = rig.only_session_dir();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let _restore = RestorePerms(dir.clone());

    session.send_input("SQRL-13 go".into());

    let ask = agent_server::testkit::wait_for_ask_id(&cap, CAP).await;
    // The Error event fires from the SAME NeedsApproval arm, degrading the
    // park to live-only rather than blocking or losing the ask.
    assert!(
        wait_until(CAP, || {
            cap.snapshot().iter().any(|e| {
                matches!(
                    e, ServerEvent::Error { message } if message.contains("checkpoint write failed")
                )
            })
        }),
        "expected a checkpoint-write-failed Error event; captured: {:#?}",
        cap.snapshot()
    );
    assert!(
        !ckpt(&dir).join("parked.json").exists(),
        "the write failure must leave no half-written park artifact"
    );

    session.approve(&ask, Decision::Approve);

    assert!(
        wait_until(CAP, || {
            cap.snapshot()
                .iter()
                .any(|e| matches!(e, ServerEvent::Done { .. }))
        }),
        "run must still complete live-only after the degraded (unparked) approval; captured: {:#?}",
        cap.snapshot()
    );
    stub.assert_consumed();
}
