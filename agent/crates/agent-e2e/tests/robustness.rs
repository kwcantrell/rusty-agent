//! Scenarios 20-22 (Task 18, spec §5e "General robustness"): mid-stream
//! connection drop + recovery, malformed/bogus model output, and event-sink
//! detach/reattach mid-turn (the 4B-1 `b86e21c` sync-subscribe abort class).
//! Spec: docs/superpowers/specs/2026-07-10-e2e-lifecycle-stress-design.md §5e.
use agent_e2e::cli::CliCmd;
use agent_e2e::rig::{ckpt, wait_until_async, Rig};
use agent_e2e::stub::{
    gated_write, text_step, RawDropStub, ScriptStep, ScriptedStub, StubResponse,
};
use agent_server::testkit::Captured;
use agent_server::wire::{Decision, ServerEvent};
use std::sync::Arc;
use std::time::Duration;

const CAP: Duration = Duration::from_secs(30);

/// Scenario 20: a mid-stream TCP drop on the model connection, then recovery.
///
/// **Characterized (live-probed, not the brief's assumed shape):** production
/// wiring (`agent_runtime_config::assemble.rs::loop_config_from`) hardcodes
/// `max_retries: 3` for the assembled loop that `Session` uses — there is no
/// config seam to lower it from this crate. `RawDropStub`'s first connection
/// sends a partial SSE chunk (`"par"`) then a hard TCP close
/// (`connection: close` + `shutdown()`); the real `OpenAiCompatClient`'s
/// stream loop (`agent-model/src/openai.rs`) sees `byte_stream.next() ==
/// None` before any terminal marker (`finish_reason`/`[DONE]`) and yields
/// `ModelError::Stream("stream ended before a completion marker (truncated
/// response)")` **immediately** — no idle-timeout wait, because this is a
/// clean EOF, not a stall. `ModelError::Stream(_)` classifies as
/// `ErrorClass::Retryable` (`agent-model/src/types.rs`), so
/// `completion_with_retry` (`agent-core/src/loop_.rs`) retries in-process: it
/// emits `AgentEvent::StreamRetry` (retracting the "par" partial), backs off
/// ~100-125ms, and re-issues the request — which lands on `RawDropStub`'s
/// SECOND connection (the "recovered" arm) and completes cleanly.
///
/// Net effect: **the very first `send_input` already reaches `Done` with
/// "recovered" token text** — no `Error` event ever surfaces to the
/// `Session`/GUI layer for this drop, because the retry absorbs it below the
/// turn-loop boundary. This is itself the scenario's recovery signal (spec
/// §5e#20: "session not corrupted; next turn on the same session works" —
/// error-surfacing itself is `e2e_robustness` T4's job, not re-asserted here,
/// and in this configuration there is no user-visible error to pin anyway).
/// Verified live (throwaway probe): elapsed ~112ms, captured order is
/// `Token("par") -> StreamRetry{3,0} -> Token("recovered") -> Done{stop}`.
/// A SECOND `send_input` (a fresh, ordinary turn) proves the session isn't
/// wedged; `sessions list` exits 0 and no `parked.json` exists anywhere.
#[tokio::test(flavor = "multi_thread")]
async fn s20_midstream_drop_then_recovery() {
    let stub = RawDropStub::start().await;
    let rig = Rig::new();
    let (session, cap) = rig.session(&stub.base_url());

    session.send_input("SQRL-20 first".into());
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Done { .. })))
        .await,
        "first send_input must reach Done (drop absorbed by in-process retry); captured: {:#?}",
        cap.snapshot()
    );
    // Pinned shape: the partial "par" chunk is retracted via StreamRetry, and
    // the retried request's text ("recovered") is what the turn actually
    // completes with -- not the dropped partial.
    let snap = cap.snapshot();
    assert!(
        snap.iter()
            .any(|e| matches!(e, ServerEvent::StreamRetry { .. })),
        "the drop must surface as an internal StreamRetry, not silently vanish; captured: {snap:#?}"
    );
    assert!(
        snap.iter()
            .any(|e| matches!(e, ServerEvent::Token { text } if text.contains("recovered"))),
        "the recovered arm's text must reach the sink; captured: {snap:#?}"
    );
    // No Error event for this drop -- characterized above as absorbed by retry.
    assert!(
        !snap.iter().any(|e| matches!(e, ServerEvent::Error { .. })),
        "this drop is absorbed by retry, not surfaced as Error; captured: {snap:#?}"
    );
    let dir = rig.only_session_dir();
    assert!(
        !ckpt(&dir).join("parked.json").exists(),
        "a plain-text recovered turn must not leave a park behind"
    );

    // Second send_input: an ordinary turn against the (now steady-state)
    // "recovered" arm -- proves the session isn't wedged by the first drop.
    session.send_input("SQRL-20 second".into());
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .filter(|e| matches!(e, ServerEvent::Done { .. }))
            .count()
            >= 2)
        .await,
        "second send_input must also reach Done; captured: {:#?}",
        cap.snapshot()
    );
    let snap2 = cap.snapshot();
    assert!(
        snap2
            .iter()
            .any(|e| matches!(e, ServerEvent::Token { text } if text.contains("recovered"))),
        "second turn's token text must contain 'recovered'; captured: {snap2:#?}"
    );

    // No park anywhere, and the CLI's independent read path agrees.
    assert!(
        !ckpt(&dir).join("parked.json").exists(),
        "no park must exist after two clean recovered turns"
    );
    let mut list = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "list"])
        .spawn();
    let st = list.wait_exit(CAP);
    assert!(
        st.success(),
        "sessions list transcript:\n{}",
        list.transcript()
    );
}

/// Scenario 21: malformed model output (invalid JSON) and a bogus tool name,
/// three turns on one Session, proving neither wedges the run.
///
/// **Turn 1 (`MalformedJson`) characterized:** the stub's body is
/// `data: {not json}\n\ndata: [DONE]\n\n`. `openai.rs`'s stream loop treats a
/// bad `data:` JSON line as transient corruption (`Some(Err(ModelError::Decode(e)))
/// => { continue; }` — skip and keep reading), so the malformed line is
/// dropped and the immediately-following `data: [DONE]` still terminates the
/// stream cleanly. Net: turn 1 is **not an error at all** at the model-client
/// layer -- it's a clean `Done{reason: "stop"}` with empty token text (no
/// tool call, no error). This is the "clean failure" the brief anticipated,
/// just one layer earlier (SSE decode, not turn-level) than "an Error event".
///
/// **Turn 2 (`BogusTool` then `text_step(None, "ok2")`) characterized:** the
/// model calls a tool name (`no_such_tool_e2e`) the registry doesn't know.
/// The loop looks it up, fails, and round-trips a `ToolResult{status:
/// "denied", content: "ERROR: not found: unknown tool no_such_tool_e2e"}`
/// back to the model as the next request's tool message (confirmed live: 3
/// requests recorded after turn 2, vs 1 after turn 1) -- the model's next
/// reply (the script's `text_step`) completes the turn normally with "ok2".
/// No `Error` event fires; the unknown-tool failure is carried entirely
/// inside the ordinary tool-result protocol.
///
/// **Turn 3** (`text_step`) completes normally, proving the session isn't
/// wedged by either of the first two turns.
#[tokio::test(flavor = "multi_thread")]
async fn s21_malformed_and_bogus_tool() {
    let stub = ScriptedStub::start(vec![
        ScriptStep {
            expect_substring: Some("SQRL-21 t1".into()),
            respond: StubResponse::MalformedJson,
        },
        ScriptStep {
            expect_substring: Some("SQRL-21 t2".into()),
            respond: StubResponse::BogusTool,
        },
        text_step(None, "ok2"),
        ScriptStep {
            expect_substring: Some("SQRL-21 t3".into()),
            respond: StubResponse::Text("t3done".into()),
        },
    ])
    .await;
    let rig = Rig::new();
    let (session, cap) = rig.session(&stub.base_url());

    // Turn 1: malformed JSON -- clean Done, no wedge, no Error.
    session.send_input("SQRL-21 t1".into());
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .filter(|e| matches!(e, ServerEvent::Done { .. }))
            .count()
            >= 1)
        .await,
        "turn 1 (malformed JSON) must reach Done cleanly; captured: {:#?}",
        cap.snapshot()
    );
    assert!(
        !cap.snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Error { .. })),
        "malformed SSE line is skipped, not surfaced as Error; captured: {:#?}",
        cap.snapshot()
    );
    let dir = rig.only_session_dir();
    assert!(
        !ckpt(&dir).join("parked.json").exists(),
        "no park after a clean malformed-JSON turn"
    );

    // Turn 2: bogus tool name round-trips a denied ToolResult, then completes
    // on the follow-up text step.
    session.send_input("SQRL-21 t2".into());
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .filter(|e| matches!(e, ServerEvent::Done { .. }))
            .count()
            >= 2)
        .await,
        "turn 2 (bogus tool) must complete via the text-step follow-up; captured: {:#?}",
        cap.snapshot()
    );
    let snap2 = cap.snapshot();
    assert!(
        snap2
            .iter()
            .any(|e| matches!(e, ServerEvent::ToolResult { name, status, .. }
                if name == "no_such_tool_e2e" && status == "denied")),
        "unknown tool must round-trip a denied ToolResult; captured: {snap2:#?}"
    );
    assert!(
        snap2
            .iter()
            .any(|e| matches!(e, ServerEvent::Token { text } if text.contains("ok2"))),
        "turn 2 must finish on the follow-up text step; captured: {snap2:#?}"
    );
    assert!(
        !snap2.iter().any(|e| matches!(e, ServerEvent::Error { .. })),
        "bogus tool is carried via the ordinary tool-result protocol, not an Error event; captured: {snap2:#?}"
    );
    assert!(
        !ckpt(&dir).join("parked.json").exists(),
        "no park after the bogus-tool turn completes"
    );

    // Turn 3: ordinary completion, proving the session isn't wedged.
    session.send_input("SQRL-21 t3".into());
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .filter(|e| matches!(e, ServerEvent::Done { .. }))
            .count()
            >= 3)
        .await,
        "turn 3 must complete, proving the session survived turns 1-2; captured: {:#?}",
        cap.snapshot()
    );
    assert!(
        cap.snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Token { text } if text.contains("t3done"))),
        "turn 3's text must reach the sink; captured: {:#?}",
        cap.snapshot()
    );

    // All 4 scripted steps consumed (t1, t2's BogusTool, t2's ok2 follow-up, t3).
    stub.assert_consumed();
}

/// Scenario 22a: event-sink swap **mid-turn** (no pending ask) -- the plain
/// `set_event_out` re-subscribe path a Tauri webview reload/reattach takes.
/// `send_input` starts a 3s-delayed turn on `cap1`, then the sink is swapped
/// to a fresh `Captured` (`cap2`) immediately, before the delayed response
/// lands. Asserts `Done` arrives on the NEW sink -- this is the b86e21c
/// sync-subscribe abort class: if `set_event_out`'s `spawn_parked_reemit`
/// call ever regressed to `tokio::spawn`ing without a reactor guard, this
/// swap would abort the whole test process rather than merely fail an
/// assertion (verified live: the swap and subsequent Done both land cleanly,
/// with `cap1` receiving nothing at all post-swap since the swap happens
/// before the delayed response is even in flight).
#[tokio::test(flavor = "multi_thread")]
async fn s22a_sink_swap_mid_turn_delivers_done_to_new_sink() {
    let stub = ScriptedStub::start(vec![ScriptStep {
        expect_substring: Some("SQRL-22 a".into()),
        respond: StubResponse::DelayedText {
            text: "slow-done".into(),
            delay_ms: 3000,
        },
    }])
    .await;
    let rig = Rig::new();
    let (session, _cap1) = rig.session(&stub.base_url());

    session.send_input("SQRL-22 a".into());
    // Swap IMMEDIATELY -- before the 3s-delayed response lands.
    let cap2 = Arc::new(Captured::default());
    session.set_event_out(cap2.clone());

    assert!(
        wait_until_async(CAP, || cap2
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Done { .. })))
        .await,
        "Done must arrive on the NEW sink after a mid-turn swap; cap2 captured: {:#?}",
        cap2.snapshot()
    );
    assert!(
        cap2.snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Token { text } if text.contains("slow-done"))),
        "the turn's token text must also land on the new sink; cap2 captured: {:#?}",
        cap2.snapshot()
    );
    stub.assert_consumed();
}

/// Scenario 22b: event-sink swap **while a live approval ask is pending** --
/// the ask must be RE-EMITTED to the new sink (`reemit_pending`, not
/// silently dropped), and approving on the new sink drives the run to
/// completion there too. `gated_write` parks behind an `ApprovalRequest` on
/// `cap1`; the sink is swapped to `cap2` while that ask is still outstanding
/// (unanswered); `set_event_out`'s `reemit_pending` call re-emits the SAME
/// ask id (`c0`) on `cap2` -- no duplicate id is minted (verified live:
/// `wait_for_ask_id(cap1)` and the id re-emitted on `cap2` are identical).
/// Approving on `cap2` resolves the ask and the run completes with `Done` on
/// `cap2`.
#[tokio::test(flavor = "multi_thread")]
async fn s22b_parked_ask_reemitted_on_swap_then_completes_on_new_sink() {
    let stub = ScriptedStub::start(vec![gated_write("SQRL-22 b"), text_step(None, "b-done")]).await;
    let rig = Rig::new();
    let (session, cap1) = rig.session(&stub.base_url());

    session.send_input("SQRL-22 b".into());
    let ask = agent_server::testkit::wait_for_ask_id(&cap1, CAP).await;
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);

    // Swap sinks WHILE the ask is still outstanding (unanswered).
    let cap2 = Arc::new(Captured::default());
    session.set_event_out(cap2.clone());

    // The LIVE ask must be re-emitted on the new sink under the SAME id.
    let ask2 = agent_server::testkit::wait_for_ask_id(&cap2, CAP).await;
    assert_eq!(
        ask, ask2,
        "the re-emitted ask on the new sink must reuse the SAME id, not mint a duplicate"
    );

    // Approve on the session (the current, live sink is cap2) and observe
    // completion there.
    session.approve(&ask2, Decision::Approve);
    assert!(
        wait_until_async(CAP, || cap2
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Done { .. })))
        .await,
        "Done must land on the new sink after approving the re-emitted ask; cap2 captured: {:#?}",
        cap2.snapshot()
    );
    assert!(
        cap2.snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::ApprovalResolved { id } if id == &ask2)),
        "ApprovalResolved for the reused id must also reach the new sink; cap2 captured: {:#?}",
        cap2.snapshot()
    );
    assert!(
        !ckpt(&dir).join("parked.json").exists(),
        "park must be consumed once the reemitted ask is approved and the run completes"
    );
    stub.assert_consumed();
}
