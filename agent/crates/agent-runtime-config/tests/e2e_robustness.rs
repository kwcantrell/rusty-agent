//! Deterministic, CI-runnable e2e for the merged robustness/security fixes,
//! driven through the real `assemble_loop` with a scripted model (and wiremock
//! for the stream-layer fix). Complements — does not replace — the per-crate
//! unit tests. See docs/superpowers/specs/2026-06-25-robustness-e2e-suite-design.md.
//!
//! Validated here: B1 (tool-call id collision), B3 (cancellation), C (read-path
//! approval gate), B2 (truncated stream surfaced). NOT here (unit-tested, with
//! reasons): C redirect (strict SSRF blocks a local wiremock target), B2
//! skip-malformed / in-band-error (pure client-layer SSE parsing).

use agent_core::testkit::{AlwaysApprove, Scripted, ScriptedModel};
use agent_core::{AgentError, AgentEvent, ContextManager, EventSink, WindowContext};
use agent_model::{Message, ModelClient, OpenAiCompatClient, Role};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use agent_runtime_config::{assemble_loop, LoopParts, RuntimeConfig};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Records structural events (Token text is accumulated separately so `events`
/// stays a clean list of structural markers for exact-match assertions).
#[derive(Default)]
struct Capture {
    events: Mutex<Vec<String>>,
    text: Mutex<String>,
}
impl EventSink for Capture {
    fn emit(&self, e: AgentEvent) {
        match e {
            AgentEvent::Token(t) => self.text.lock().unwrap().push_str(&t),
            AgentEvent::ToolStart { name, .. } => {
                self.events.lock().unwrap().push(format!("tool_start:{name}"))
            }
            AgentEvent::ToolResult { name, .. } => {
                self.events.lock().unwrap().push(format!("tool_result:{name}"))
            }
            AgentEvent::Approval(req) => self
                .events
                .lock()
                .unwrap()
                .push(format!("approval:{}:{:?}", req.intent.tool, req.intent.paths)),
            AgentEvent::Error(m) => self.events.lock().unwrap().push(format!("error:{m}")),
            AgentEvent::Done(r) => self.events.lock().unwrap().push(format!("done:{r:?}")),
            _ => {}
        }
    }
}

/// Build the real loop with `memory=false`, `sandbox=off`, native/openai. The
/// model's URL is irrelevant for a ScriptedModel; the OpenAiCompatClient carries
/// its own URL. cfg.base_url is unused by the loop (the model is injected).
fn assemble_test(
    workspace: PathBuf,
    model: Arc<dyn ModelClient>,
    approval: Arc<dyn ApprovalChannel>,
) -> (agent_runtime_config::BuiltLoop, Arc<Capture>) {
    let mut cfg = RuntimeConfig::from_launch(
        "openai".into(),
        "http://unused".into(),
        "test-model".into(),
        "native".into(),
        262_144,
    );
    cfg.memory = false;
    cfg.sandbox_mode = "off".into();
    let sink = Arc::new(Capture::default());
    let built = assemble_loop(
        &cfg,
        LoopParts {
            model,
            sink: sink.clone(),
            approval,
            workspace,
            mcp_tools: vec![],
            memory_tools: vec![],
            memory_retriever: None,
            stream_idle_timeout: Duration::from_secs(10),
            base_system_prompt: "You are a test agent.".into(),
        },
    );
    (built, sink)
}

// T1 — B1: two tool calls sharing an id don't crash, and produce two distinct
// tool messages in the transcript. (Would panic on pre-B1 code.)
#[tokio::test]
async fn b1_duplicate_tool_call_ids_through_assembled_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();
    std::fs::write(ws.join("a.txt"), "BODY").unwrap();
    let model = Arc::new(ScriptedModel::new(vec![
        Scripted::Calls(vec![
            ("c1".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()),
            ("c1".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()),
        ]),
        Scripted::Text("done".into()),
    ]));
    let (built, sink) = assemble_test(ws, model, Arc::new(AlwaysApprove));
    let mut ctx = WindowContext::new(Message::system(built.system_prompt.clone()));

    built.loop_.run(&mut ctx, "read twice".into()).await.unwrap(); // must not panic

    let events = sink.events.lock().unwrap().clone();
    assert_eq!(
        events.iter().filter(|e| *e == "tool_result:read_file").count(),
        2,
        "both reads should produce a result; events: {events:?}"
    );
    let transcript = ctx.build(262_144);
    let tool_ids: Vec<String> = transcript
        .iter()
        .filter(|m| matches!(m.role, Role::Tool))
        .map(|m| m.tool_call_id.clone().unwrap_or_default())
        .collect();
    assert_eq!(tool_ids.len(), 2, "two tool messages expected: {tool_ids:?}");
    assert_ne!(tool_ids[0], tool_ids[1], "duplicate ids must normalize to distinct");
}

// T2 — B3: a pre-cancelled token stops the assembled loop cleanly, before the
// model is ever consulted. (run_with_cancel is the B3 entry point.)
#[tokio::test]
async fn b3_precancelled_token_stops_assembled_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();
    let model = Arc::new(ScriptedModel::new(vec![Scripted::Text("should not run".into())]));
    let (built, sink) = assemble_test(ws, model, Arc::new(AlwaysApprove));
    let mut ctx = WindowContext::new(Message::system(built.system_prompt.clone()));

    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel(); // cancelled before the run starts

    built.loop_.run_with_cancel(&mut ctx, "go".into(), cancel).await.unwrap();

    // Only the terminal Done(Cancelled); no Usage/Token (model never consulted).
    let events = sink.events.lock().unwrap().clone();
    assert_eq!(events, vec!["done:Cancelled".to_string()], "events: {events:?}");
    assert!(sink.text.lock().unwrap().is_empty(), "no model text should stream");
}
