//! Scenarios 14-17 (Task 16): directed resume.lock loser-paths, one
//! barrier-synchronized symmetric race, and multi-session id addressing.
//! Spec: docs/superpowers/specs/2026-07-10-e2e-lifecycle-stress-design.md §5c.
use agent_e2e::cli::CliCmd;
use agent_e2e::rig::{ckpt, wait_until, wait_until_async, Rig};
use agent_e2e::stub::{gated_write, text_step, ScriptedStub};
use agent_server::wire::{Decision, ServerEvent};
use std::time::Duration;

const CAP: Duration = Duration::from_secs(30);

/// Shared setup: park a run via the Session (GUI) leg, drop the Session, and
/// return (rig, stub, session dir, session id). Factored out of s01-style
/// inline flow (crashkill.rs/lifecycle.rs repeat this same shape).
async fn park_via_session(task: &str, stub: &ScriptedStub) -> (Rig, std::path::PathBuf, String) {
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input(task.into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    rig.assert_parked(&dir);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();
    (rig, dir, sid)
}

/// Scenario 14: an in-process holder (the TEST itself, standing in for a
/// live daemon) claims `resume.lock` directly via
/// `agent_core::checkpoint::claim_resume`, then a CLI `sessions reopen`
/// against the SAME tree must lose: exit 2, the stable contention substring,
/// and the park left fully intact (a loser never touches the checkpoint).
#[tokio::test(flavor = "multi_thread")]
async fn s14_inprocess_holder_cli_loser() {
    // Script: [gated_write] only. The CLI reopen never gets far enough to
    // reach a resume (claim_resume fails before any model request), so the
    // run is spare by design — `stub.recorded()`, not `assert_consumed()`.
    let stub = ScriptedStub::start(vec![gated_write("SQRL-14 go")]).await;
    let (rig, dir, sid) = park_via_session("SQRL-14 go", &stub).await;

    // In-process holder: the test itself claims the lock, exactly like
    // s11's real-process holder but without a second binary.
    let claimed = agent_core::checkpoint::claim_resume(&ckpt(&dir)).unwrap();
    assert!(claimed, "test must be the one to claim resume.lock first");

    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    let st = cli.wait_exit(CAP);
    let t = cli.transcript();
    assert_eq!(st.code(), Some(2), "transcript:\n{t}");
    assert!(
        t.contains("is being resumed elsewhere"),
        "expected the contention message, got:\n{t}"
    );
    assert!(!t.contains("panicked"), "must be a clean refusal:\n{t}");
    rig.assert_parked(&dir);

    agent_core::checkpoint::release_resume(&ckpt(&dir));
    let _ = stub.recorded();
}

/// Scenario 15: the CLI reopen claims `resume.lock` BEFORE it ever prompts
/// (agent-cli's `run_sessions_reopen` — see main.rs) and holds it with the
/// pipe open at the approval prompt, unanswered. A fresh Session attached
/// over the SAME roots re-derives the parked ask on attach (re-emit does
/// not touch the lock) and its `approve()` drives `start_resume`, which
/// hits `claim_resume` contention and surfaces `ServerEvent::Error`
/// containing "is being resumed elsewhere" — the loser observes contention,
/// never a silent no-op. THEN answering the CLI's still-open prompt lets
/// the CLI (the lock holder) complete the run.
///
/// Characterized: double-resolution is NOT structurally excluded here —
/// the CLI claims the lock at reopen-scan time, before prompting, but that
/// only blocks a SECOND resume attempt (start_resume's own claim_resume
/// call); it does not stop a fresh Session from re-deriving and answering
/// the SAME ask id. The Session-side answer is durably committed
/// (write_answer) and then its own start_resume attempt loses the lock race
/// and reports the Error above — the CLI's in-flight prompt, once answered,
/// is the one that actually drives the resume to completion.
#[tokio::test(flavor = "multi_thread")]
async fn s15_cli_holder_session_loser_then_cli_completes() {
    let stub = ScriptedStub::start(vec![gated_write("SQRL-15 go"), text_step(None, "done")]).await;
    let (rig, dir, sid) = park_via_session("SQRL-15 go", &stub).await;

    // CLI reopen: claims resume.lock during its scan, THEN prompts. Hold the
    // pipe open at the prompt — do NOT answer yet.
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .approval_timeout_secs(60)
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    // Positive-artifact check: the CLI really does hold the lock now.
    assert!(ckpt(&dir).join("resume.lock").exists());

    // Fresh Session attach over the same roots: re-derives the SAME parked
    // ask (re-emit is lock-independent — it only reads checkpoint payload).
    let (session, cap) = rig.session(&stub.base_url());
    let ask = agent_server::testkit::wait_for_ask_id(&cap, CAP).await;
    session.approve(&ask, Decision::Approve);

    // The Session-side start_resume contends for resume.lock (held by the
    // CLI) and loses: it must observe an Error event, never silently drop
    // the answer or double-resolve.
    assert!(
        wait_until_async(CAP, || {
            cap.snapshot().iter().any(|e| {
                matches!(e, ServerEvent::Error { message } if message.contains("is being resumed elsewhere"))
            })
        })
        .await,
        "expected the Session-side loser to observe a contention Error; captured: {:#?}",
        cap.snapshot()
    );
    // The loser must not have driven the run to completion.
    assert!(
        !cap.snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Done { .. })),
        "the lock-losing Session must not itself complete the run"
    );

    // NOW answer the CLI's still-outstanding prompt — the lock holder
    // completes the run.
    cli.write_line("y");
    let st = cli.wait_exit(CAP);
    let t = cli.transcript();
    assert!(st.success(), "{t}");
    stub.assert_consumed();
    assert!(
        !ckpt(&dir).join("parked.json").exists(),
        "park must be fully consumed once the CLI (lock holder) completes"
    );
}

/// Scenario 16: a genuine, symmetric race — spawn the CLI reopen AND, as
/// soon as it's spawned (before waiting for its prompt), attach a fresh
/// Session and race its approve against the CLI's own claim+prompt. Accept
/// EITHER winner; assert only the symmetric postconditions: exactly one
/// success, the other observes a clean contention outcome, no panic
/// anywhere, and the park is consumed exactly once (parked.json gone at the
/// end). One gated step + one completion step suffices — the park is
/// consumed exactly once regardless of who wins.
#[tokio::test(flavor = "multi_thread")]
async fn s16_symmetric_race() {
    let stub = ScriptedStub::start(vec![gated_write("SQRL-16 go"), text_step(None, "done")]).await;
    let (rig, dir, sid) = park_via_session("SQRL-16 go", &stub).await;
    let base_url = stub.base_url();

    // Spawn the CLI reopen on a blocking task IMMEDIATELY -- before waiting
    // for its prompt -- so it races genuinely against the Session leg below.
    // `CliCmd::new` only needs `&Rig` to read three paths + the key; build
    // the Command by hand here instead so the blocking closure is `'static`
    // (owns cloned PathBufs/Strings) and never borrows `rig` across the
    // spawn_blocking boundary.
    let cli_task = {
        let workspace = rig.workspace.path().to_path_buf();
        let sessions = rig.sessions.path().to_path_buf();
        let meta = rig.meta.path().to_path_buf();
        let base_url = base_url.clone();
        let sid = sid.clone();
        tokio::task::spawn_blocking(move || {
            use std::process::{Command, Stdio};
            let mut cmd = Command::new(agent_e2e::cli::agent_bin());
            cmd.args([
                "--base-url",
                &base_url,
                "--model",
                "stub-model",
                "--workspace",
                workspace.to_str().unwrap(),
                "--trace-dir",
                sessions.to_str().unwrap(),
                "--metadata-dir",
                meta.to_str().unwrap(),
                "--stream-timeout-secs",
                "10",
                "--approval-timeout-secs",
                "60",
                "sessions",
                "reopen",
                &sid,
            ]);
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                cmd.process_group(0);
            }
            cmd.env("HOME", &meta)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut cli = agent_e2e::cli::CliCmd::from_command(cmd).spawn();
            // Tolerate either winner: poll transcript for the prompt; if the
            // CLI instead loses the race before ever prompting, it exits on
            // its own (contention refusal) -- `has_exited` breaks out of
            // this loop immediately instead of burning the full CAP waiting
            // for a prompt that will never come.
            let start = std::time::Instant::now();
            let mut saw_prompt = false;
            while start.elapsed() < CAP {
                if cli.transcript().contains("[y]es / [n]o / [a]lways:") {
                    saw_prompt = true;
                    break;
                }
                if cli.has_exited() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if saw_prompt {
                cli.write_line("y");
            }
            let st = cli.wait_exit(CAP);
            let t = cli.transcript();
            (st, t)
        })
    };

    // Immediately (no waiting on the CLI leg) attach a fresh Session and
    // race its approve against the CLI's own claim+prompt.
    let (session, cap) = rig.session(&base_url);
    let ask = agent_server::testkit::wait_for_ask_id(&cap, CAP).await;
    session.approve(&ask, Decision::Approve);

    // Bounded window to reach EITHER Done or a contention Error -- whichever
    // the race produces.
    let session_outcome = wait_until_async(CAP, || {
        cap.snapshot().iter().any(|e| matches!(e, ServerEvent::Done { .. }))
            || cap.snapshot().iter().any(|e| {
                matches!(e, ServerEvent::Error { message } if message.contains("is being resumed elsewhere"))
            })
    })
    .await;
    assert!(
        session_outcome,
        "Session leg must reach a definite outcome (Done or contention Error); captured: {:#?}",
        cap.snapshot()
    );
    let session_won = cap
        .snapshot()
        .iter()
        .any(|e| matches!(e, ServerEvent::Done { .. }));
    let session_lost = cap.snapshot().iter().any(|e| {
        matches!(e, ServerEvent::Error { message } if message.contains("is being resumed elsewhere"))
    });
    assert!(
        session_won ^ session_lost,
        "Session leg must land on exactly one outcome, not both; captured: {:#?}",
        cap.snapshot()
    );

    let (cli_status, cli_transcript) = cli_task.await.unwrap();
    assert!(
        !cli_transcript.contains("panicked"),
        "CLI leg must never panic:\n{cli_transcript}"
    );
    // CLI-side winner signature: it reached the prompt, answered it, and
    // exited 0.
    let cli_won = cli_status.success() && cli_transcript.contains("[y]es / [n]o / [a]lways:");
    // CLI-side clean-loser signatures -- accept ANY of the observed shapes
    // (characterize, don't over-narrow): (a) the stale-lock contention exit
    // (code 2 + the stable substring), or (b) the CLI found the park already
    // gone by the time it scanned ("nothing parked" -- the Session won
    // before the CLI even got that far).
    let cli_lost_contention =
        cli_status.code() == Some(2) && cli_transcript.contains("is being resumed elsewhere");
    let cli_lost_already_consumed = cli_transcript.contains("nothing parked");
    let cli_clean_loser = !cli_won && (cli_lost_contention || cli_lost_already_consumed);

    assert!(
        cli_won ^ session_won,
        "exactly one of {{CLI, Session}} must win the race; cli_won={cli_won} session_won={session_won} \
         cli transcript:\n{cli_transcript}"
    );
    if cli_won {
        assert!(
            session_lost,
            "if the CLI won, the Session leg must have observed the contention Error; captured: {:#?}",
            cap.snapshot()
        );
    } else {
        assert!(
            cli_clean_loser,
            "if the Session won, the CLI must observe a clean, characterized loser outcome; \
             transcript:\n{cli_transcript}"
        );
    }

    // Symmetric postcondition regardless of winner: the park is consumed
    // exactly once (gone at the end), never left dangling, never
    // double-consumed with an error.
    assert!(
        wait_until(CAP, || !ckpt(&dir).join("parked.json").exists()),
        "park must be consumed exactly once by the race's winner"
    );
    stub.assert_consumed();
}

/// Scenario 17: three sessions on one rig (sequential Session legs). Park A
/// (task `TASK-A`, left parked), complete B (text-only), park C (`TASK-C`).
/// `sessions reopen <C-id>` with a script whose post-approve matcher
/// REQUIRES "TASK-C" -- the resume request replays C's own history, so if A
/// were addressed instead its body would carry TASK-A and NOT TASK-C,
/// mismatching the matcher and poisoning the stub. Then `sessions list`
/// exits 0 and its transcript contains A's id (still parked).
#[tokio::test(flavor = "multi_thread")]
async fn s17_multi_session_addressing() {
    let stub = ScriptedStub::start(vec![
        gated_write("TASK-A"),
        text_step(Some("TASK-B"), "b-done"),
        gated_write("TASK-C"),
        // Post-approve resume request for C: MUST carry TASK-C (C's own
        // history). If A were resumed instead, this matcher fails -> poison.
        agent_e2e::stub::ScriptStep {
            expect_substring: Some("TASK-C".into()),
            respond: agent_e2e::stub::StubResponse::Text("c-done".into()),
        },
    ])
    .await;
    let rig = Rig::new();

    // Session A: park, task text TASK-A, leave parked (never answer).
    let (session_a, _cap_a) = rig.session(&stub.base_url());
    session_a.send_input("TASK-A".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dirs_after_a = rig.session_dirs();
    assert_eq!(dirs_after_a.len(), 1);
    let dir_a = dirs_after_a[0].clone();
    assert!(wait_until_async(CAP, || ckpt(&dir_a).join("parked.json").exists()).await);
    let sid_a = dir_a.file_name().unwrap().to_string_lossy().into_owned();
    drop(session_a);

    // Session B: completes (text-only, no park).
    let (session_b, cap_b) = rig.session(&stub.base_url());
    session_b.send_input("TASK-B".into());
    assert!(
        wait_until_async(CAP, || cap_b
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Done { .. })))
        .await
    );
    drop(session_b);

    // Session C: park, task text TASK-C, leave parked. New dir = the one not
    // already present after A (B never creates a park, but it DOES create
    // its own session dir; track incrementally by diffing against the set
    // after A+B).
    let dirs_after_ab: std::collections::BTreeSet<_> = rig.session_dirs().into_iter().collect();
    let (session_c, _cap_c) = rig.session(&stub.base_url());
    session_c.send_input("TASK-C".into());
    assert!(
        wait_until_async(CAP, || rig
            .session_dirs()
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .difference(&dirs_after_ab)
            .count()
            > 0)
        .await
    );
    let dir_c = rig
        .session_dirs()
        .into_iter()
        .find(|d| !dirs_after_ab.contains(d))
        .expect("session C must create a new session dir");
    assert!(wait_until_async(CAP, || ckpt(&dir_c).join("parked.json").exists()).await);
    let sid_c = dir_c.file_name().unwrap().to_string_lossy().into_owned();
    drop(session_c);

    // Sanity: A and C are genuinely distinct parked dirs.
    assert_ne!(sid_a, sid_c);
    rig.assert_parked(&dir_a);
    rig.assert_parked(&dir_c);

    // Reopen C explicitly by id; approve -> the resume request must carry
    // TASK-C (the matcher above is the addressing proof).
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid_c])
        .spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("y");
    let st = cli.wait_exit(CAP);
    let t = cli.transcript();
    assert!(st.success(), "{t}");
    stub.assert_consumed();
    assert!(
        !ckpt(&dir_c).join("parked.json").exists(),
        "C's park must be fully consumed"
    );

    // A stays parked -- spare/unconsumed by design (script never answers it).
    rig.assert_parked(&dir_a);

    // `sessions list` exits 0 and shows A's id (still parked).
    let mut list = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "list"])
        .spawn();
    let st2 = list.wait_exit(CAP);
    assert!(st2.success());
    let t2 = list.transcript();
    assert!(
        t2.contains(&sid_a),
        "sessions list must show A's still-parked id, transcript:\n{t2}"
    );
}
