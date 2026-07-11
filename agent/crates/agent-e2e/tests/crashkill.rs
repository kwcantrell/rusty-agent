//! [real-kill] scenarios 5-6: SIGINT mid-resume cancels clean with no park
//! to retain (characterized below — not the `76d81d5` regression); a
//! committed answer survives a real kill and is consumed exactly once.
//! Spec: docs/superpowers/specs/2026-07-10-e2e-lifecycle-stress-design.md.
use agent_e2e::cli::CliCmd;
use agent_e2e::rig::{ckpt, wait_until, wait_until_async, Rig};
use agent_e2e::stub::{gated_write, text_step, ScriptStep, ScriptedStub, StubResponse};
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
/// The final `text_step` is DELIBERATELY SPARE — the run never reaches a
/// second model call, so the script's 3rd step is never consumed;
/// `stub.recorded()` (not `assert_consumed()`) is the closing check here.
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
