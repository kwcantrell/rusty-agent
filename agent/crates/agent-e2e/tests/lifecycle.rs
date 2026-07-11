//! Tier-1 scenario matrix: cross-surface park/reopen, both directions.
//! Spec: docs/superpowers/specs/2026-07-10-e2e-lifecycle-stress-design.md.
use agent_e2e::cli::{CliCmd, REPL_MARKER};
use agent_e2e::rig::{ckpt, wait_until_async, Rig};
use agent_e2e::stub::{gated_write, text_step, ScriptedStub};
use agent_server::wire::{Decision, ServerEvent};
use std::time::Duration;

const CAP: Duration = Duration::from_secs(30);

/// Scenario 1: park via Session (GUI leg) -> CLI `sessions reopen` -> approve
/// -> completes.
#[tokio::test(flavor = "multi_thread")]
async fn s01_park_in_gui_reopen_in_cli() {
    // Script: task -> gated tool call (parks); after approve -> tool result goes
    // back -> final text completes the run.
    let stub = ScriptedStub::start(vec![
        gated_write("SQRL-1 write the file"),
        text_step(None, "done"),
    ])
    .await;
    let rig = Rig::new();
    let (session, cap) = rig.session(&stub.base_url());
    assert!(matches!(
        session.send_input("SQRL-1 write the file".into()),
        agent_server::session::SendOutcome::Started
    ));
    // Ask surfaces + park lands on disk.
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::ApprovalRequest { .. })))
        .await
    );
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    rig.assert_parked(&dir);
    // "GUI closes": drop the Session mid-park. Parks must survive process
    // death (simulated here by drop; scenario 12 does it with a real kill) —
    // if this destroys the park, that's a product finding, not a test bug.
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();

    // Reopen from the CLI, approve at the prompt.
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("y");
    let st = cli.wait_exit(CAP);
    assert!(st.success(), "transcript:\n{}", cli.transcript());
    stub.assert_consumed();
    // Completed tree is reaped (delete-on-completion) — dir gone or no park.
    assert!(
        !ckpt(&dir).join("parked.json").exists(),
        "park must be consumed"
    );
}

/// Scenario 2: CLI timeout park-and-exit -> Session attach sees parked_runs
/// -> approve -> resumed -> done.
#[tokio::test(flavor = "multi_thread")]
async fn s02_cli_timeout_park_then_gui_attach_resumes() {
    let stub = ScriptedStub::start(vec![
        gated_write("SQRL-2 write it"),
        text_step(None, "done"),
    ])
    .await;
    let rig = Rig::new();
    // 1s approval window -> deterministic park-and-exit (E2 knob).
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .approval_timeout_secs(1)
        .spawn();
    cli.wait_for_output(REPL_MARKER, Duration::from_secs(10));
    cli.write_line("SQRL-2 write it");
    cli.wait_for_output("run parked; answer later with", CAP);
    let st = cli.wait_exit(CAP);
    assert!(st.success(), "transcript:\n{}", cli.transcript());
    let dir = rig.only_session_dir();
    rig.assert_parked(&dir);
    drop(cli);

    // "GUI opens": fresh Session over the same roots re-emits the park on attach.
    let (session, cap) = rig.session(&stub.base_url());
    assert!(
        wait_until_async(CAP, || cap.snapshot().iter().any(
            |e| matches!(e, ServerEvent::ParkedRuns { runs } if !runs.is_empty())
        ))
        .await
    );
    let ask = agent_server::testkit::wait_for_ask_id(&cap, CAP).await;
    session.approve(&ask, Decision::Approve);
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Resumed { .. })))
        .await
    );
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Done { .. })))
        .await
    );
    stub.assert_consumed();
}
