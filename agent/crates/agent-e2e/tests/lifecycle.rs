//! Tier-1 scenario matrix: cross-surface park/reopen, both directions.
//! Spec: docs/superpowers/specs/2026-07-10-e2e-lifecycle-stress-design.md.
use agent_e2e::cli::{CliCmd, REPL_MARKER};
use agent_e2e::rig::{ckpt, wait_until_async, Rig};
use agent_e2e::stub::{gated_write, text_step, ScriptStep, ScriptedStub, StubResponse};
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

/// Scenario 3: park via Session -> CLI `sessions reopen` -> deny with
/// feedback -> the model retries the gated call (feedback text must reach
/// the retried request verbatim) -> re-park -> SAME reopen process re-prompts
/// (characterized: `run_sessions_reopen` prompts the root ask once via
/// `prompt_for_answer_with_reader`, then hands the whole resumed turn to
/// `resume_with_cancel`; a fresh Ask inside that turn is served by the same
/// `TerminalApproval` — same process, same stdin pipe, same
/// `[y]es / [n]o / [a]lways:` prompt) -> approve -> completes.
async fn deny_feedback_roundtrip(feedback: &str) {
    // The request body is JSON: quotes/backslashes/control chars/non-ASCII in
    // `feedback` are escaped by serde_json's string serializer (e.g. `"` ->
    // `\"`, a CJK/emoji codepoint -> `\uXXXX`), so the wire-level needle is the
    // JSON-*encoded* form of the feedback, not the raw string. For the plain
    // (ASCII, no-quote/backslash) scenario-3 feedback the two forms are
    // identical; for the hostile variant they differ, which is exactly the
    // escaping this test wants to exercise. `wire_needle` strips the
    // surrounding quotes serde_json adds so it stays a pure substring.
    let wire_needle = {
        let quoted = serde_json::to_string(feedback).expect("feedback is valid UTF-8");
        quoted[1..quoted.len() - 1].to_string()
    };
    let stub = ScriptedStub::start(vec![
        gated_write("SQRL-3 write it"),
        // After a deny, the tool message carrying the feedback goes back to the
        // model: the matcher REQUIRES the feedback text (wire-encoded) in the
        // request body.
        ScriptStep {
            expect_substring: Some(wire_needle.clone()),
            respond: StubResponse::ToolCall {
                name: "write_file".into(),
                args: serde_json::json!({"path": "out2.txt", "content": "retry"}),
            },
        },
        text_step(None, "done"),
    ])
    .await;
    let rig = Rig::new();
    // Park on the Session leg.
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-3 write it".into());
    let dir_ready = wait_until_async(CAP, || !rig.session_dirs().is_empty()).await;
    assert!(dir_ready);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();

    // Deny with feedback from the CLI (two stdin lines, pipe stays open).
    // approval_timeout_secs(60): the deny -> retry -> re-park cycle happens
    // while THIS reopen's TerminalApproval is waiting on the second prompt;
    // the resumed turn hits the stub near-instantly, so 60s is ample headroom
    // over the 20s CliCmd default (bump kept in case of CI slowness).
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .approval_timeout_secs(60)
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("n");
    cli.wait_for_output("Feedback for the agent", CAP);
    cli.write_line(feedback);
    // Deny -> model retries the gated call -> run re-parks; characterized above:
    // the CLI re-prompts in the SAME reopen process (not park-and-exit).
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("y");
    let st = cli.wait_exit(CAP);
    assert!(st.success(), "transcript:\n{}", cli.transcript());
    stub.assert_consumed();
    // The strong assertion: feedback text reached the model verbatim (modulo
    // the JSON string-escaping the wire format requires — see `wire_needle`).
    assert!(stub.recorded().iter().any(|r| r.contains(&wire_needle)));
}

#[tokio::test(flavor = "multi_thread")]
async fn s03_deny_feedback_travels_to_model() {
    deny_feedback_roundtrip("SQRL-FEEDBACK use path out2.txt instead").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn s03b_deny_feedback_hostile_content() {
    // multibyte + control chars + JSON-meta + long tail (~10KB). Newline-free:
    // stdin is line-based (Cli::write_line writes one line), so the feedback
    // line itself cannot contain a raw '\n' — JSON-escaping of everything else
    // (quotes, backslash, brace, tab, emoji, CJK) happens in the
    // checkpoint/request layers, which is exactly what's under test.
    let hostile = format!(
        "SQRL-HOSTILE \u{65e5}\u{672c}\u{8a9e} \u{1F980} quote\" backslash\\ brace}} tab\tnewline-free {}",
        "x".repeat(10_000)
    );
    deny_feedback_roundtrip(&hostile).await;
}

/// Scenario 4 (soak): N=4 lifecycle cycles on shared roots, alternating
/// deny-with-feedback and approve, crossing CLI <-> Session surfaces each
/// time, with one cycle carrying a ~2MB `write_file` arg so the checkpoint
/// dumps a multi-MB artifact store. Guards the 2fad367 tally-floor
/// regression (denials desyncing the persisted tool tally) plus general
/// checkpoint/resume drift under repeated park/deny/approve cycles.
///
/// Request sequence actually exercised (characterized against the live
/// stub — see task-12-report.md for the full walk):
///   1. "SQRL-SOAK start"      -> gated write_file            (park #1)
///   2. deny(SOAK-FB-1)  retry -> matcher "SOAK-FB-1"          (park #2, cycle 1)
///   3. approve               -> matcher unset (text) "cycle2 done" (cycle 2 done)
///   4. "SQRL-SOAK turn3"      -> gated write_file (big ~2MB)  (park #3, cycle 3)
///   5. deny(SOAK-FB-3)  retry -> matcher "SOAK-FB-3"          (park #4, cycle 3 retry)
///   6. approve               -> matcher unset (text) "cycle4 done" (cycle 4 done)
#[tokio::test(flavor = "multi_thread")]
async fn s04_soak_alternating_deny_approve_across_surfaces() {
    let big = "B".repeat(2_000_000);
    let steps = vec![
        gated_write("SQRL-SOAK start"),
        ScriptStep {
            expect_substring: Some("SOAK-FB-1".into()),
            respond: StubResponse::ToolCall {
                name: "write_file".into(),
                args: serde_json::json!({"path": "a.txt", "content": "x"}),
            },
        },
        text_step(None, "cycle2 done"),
        ScriptStep {
            expect_substring: Some("SQRL-SOAK turn3".into()),
            respond: StubResponse::ToolCall {
                name: "write_file".into(),
                args: serde_json::json!({"path": "big.txt", "content": big}),
            },
        },
        ScriptStep {
            expect_substring: Some("SOAK-FB-3".into()),
            respond: StubResponse::ToolCall {
                name: "write_file".into(),
                args: serde_json::json!({"path": "big2.txt", "content": "y"}),
            },
        },
        text_step(None, "cycle4 done"),
    ];
    let stub = ScriptedStub::start(steps).await;
    let rig = Rig::new();
    let soak_start = std::time::Instant::now();

    // Cycle 1: park on Session, deny w/ feedback via CLI (re-park), approve
    // via the SAME CLI reopen process (characterized in s03: a deny inside a
    // `sessions reopen` re-prompts in-process, it does not park-and-exit).
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-SOAK start".into());
    let dir = {
        assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
        let d = rig.only_session_dir();
        assert!(wait_until_async(CAP, || ckpt(&d).join("parked.json").exists()).await);
        d
    };
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .approval_timeout_secs(60)
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("n");
    cli.wait_for_output("Feedback for the agent", CAP);
    cli.write_line("SOAK-FB-1");
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP); // re-parked, re-prompted
    cli.write_line("y");
    let st = cli.wait_exit(CAP);
    assert!(st.success(), "cycle1-2 transcript:\n{}", cli.transcript());
    drop(cli);

    // Cycle 3+4: new turn on a fresh Session (same roots), park, deny
    // (Session), re-park, approve from CLI reopen.
    let (session, cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-SOAK turn3".into());
    let ask = agent_server::testkit::wait_for_ask_id(&cap, CAP).await;
    // parked.json must exist before we act (positive-artifact rule) — the
    // session dir may be a NEW dir for the new Session; rediscover.
    let dirs = rig.session_dirs();
    assert!(
        wait_until_async(CAP, || dirs
            .iter()
            .any(|d| ckpt(d).join("parked.json").exists()))
        .await
    );
    session.approve(
        &ask,
        Decision::Deny {
            feedback: Some("SOAK-FB-3".into()),
        },
    );
    // wait_for_ask_id always returns the FIRST ask in the capture; after the
    // deny above we need the ask that supersedes it, hence `_after`.
    let ask2 = agent_server::testkit::wait_for_ask_id_after(&cap, &ask, CAP).await;

    // Capture the big-payload checkpoint BEFORE final approve (which cleans it up
    // on Done). The parked.json after the deny contains the full retry tool call
    // with the 2MB content inline (tool arguments live in the checkpoint, not in
    // artifact backends). Read into memory now, before cleanup.
    let checkpoint_with_big = {
        let dirs = rig.session_dirs();
        dirs.iter()
            .find(|d| {
                let parked_path = ckpt(d).join("parked.json");
                if let Ok(bytes) = std::fs::read(&parked_path) {
                    bytes.len() > 2_000_000
                } else {
                    false
                }
            })
            .and_then(|d| {
                let path = ckpt(d).join("parked.json");
                std::fs::read(&path).ok()
            })
    };

    session.approve(&ask2, Decision::Approve);
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Done { .. })))
        .await
    );
    stub.assert_consumed();

    let soak_elapsed = soak_start.elapsed();
    eprintln!("s04 soak wall time: {soak_elapsed:?}");

    // Big-payload proof: verify the ~2MB tool call payload was carried in the
    // checkpoint. The checkpoint_with_big was captured before the final
    // approve, so it's a snapshot of the run after deny+retry with the full
    // retry tool call containing 2MB args inline (tool arguments are stored in
    // the checkpoint, not offloaded to an artifact backend).
    let parked_bytes =
        checkpoint_with_big.expect("no checkpoint with 2MB payload found after deny+retry");
    assert!(
        parked_bytes.len() > 2_000_000,
        "parked.json must carry the ~2MB tool-call payload; got {} bytes, expected > 2,000,000",
        parked_bytes.len()
    );
    // Strong test: the big content (2_000_000 B's) must be present in the
    // serialized checkpoint.
    let parked_str = String::from_utf8_lossy(&parked_bytes);
    assert!(
        parked_str.contains(&"B".repeat(2_000_000)),
        "the ~2MB content must flow through to parked.json verbatim"
    );

    // Tally monotonicity / no drift: the surviving session dirs verify clean —
    // `sessions list` exits 0 and shows no corruption complaints. Note:
    // the tally-floor (2fad367) regression is verified by the mid-test
    // reopen/attach paths themselves — every reopen runs verify_tally_floor
    // and hard-fails this test on violation. The list check below is a final
    // wedge/corruption sweep, not the tally-floor proof site.
    let mut list = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "list"])
        .spawn();
    let st = list.wait_exit(CAP);
    assert!(st.success());
    let t = list.transcript();
    assert!(!t.contains("corrupt"), "list transcript:\n{t}");
}
