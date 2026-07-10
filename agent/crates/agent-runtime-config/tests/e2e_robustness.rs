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
            AgentEvent::ToolStart { name, .. } => self
                .events
                .lock()
                .unwrap()
                .push(format!("tool_start:{name}")),
            AgentEvent::ToolResult { name, status, .. } => self
                .events
                .lock()
                .unwrap()
                .push(format!("tool_result:{name}:{}", status.as_str())),
            AgentEvent::Approval(req) => self.events.lock().unwrap().push(format!(
                "approval:{}:{:?}",
                req.intent.tool, req.intent.paths
            )),
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
            stream_idle_timeout: Duration::from_secs(10),
            base_system_prompt: "You are a test agent.".into(),
            artifacts: Arc::new(agent_core::SessionArtifacts::new()),
            compact_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            todos: Arc::new(std::sync::Mutex::new(Vec::new())),
            sandbox: agent_runtime_config::build_sandbox(&cfg),
            stats: Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default())),
            trace: None,
            api_key: None,
            claude_binary: "claude".into(),
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
            (
                "c1".into(),
                "read_file".into(),
                r#"{"path":"a.txt"}"#.into(),
            ),
            (
                "c1".into(),
                "read_file".into(),
                r#"{"path":"a.txt"}"#.into(),
            ),
        ]),
        Scripted::Text("done".into()),
    ]));
    let (built, sink) = assemble_test(ws, model, Arc::new(AlwaysApprove));
    let mut ctx = WindowContext::new(Message::system(built.system_prompt.clone()));

    built
        .loop_
        .run(&mut ctx, "read twice".into())
        .await
        .unwrap(); // must not panic

    let events = sink.events.lock().unwrap().clone();
    assert_eq!(
        events
            .iter()
            .filter(|e| *e == "tool_result:read_file:ok")
            .count(),
        2,
        "both reads should produce a result; events: {events:?}"
    );
    let transcript = ctx.build(262_144);
    let tool_ids: Vec<String> = transcript
        .iter()
        .filter(|m| matches!(m.role, Role::Tool))
        .map(|m| m.tool_call_id.clone().unwrap_or_default())
        .collect();
    assert_eq!(
        tool_ids.len(),
        2,
        "two tool messages expected: {tool_ids:?}"
    );
    assert_ne!(
        tool_ids[0], tool_ids[1],
        "duplicate ids must normalize to distinct"
    );
}

// T2 — B3: a pre-cancelled token stops the assembled loop cleanly, before the
// model is ever consulted. (run_with_cancel is the B3 entry point.)
#[tokio::test]
async fn b3_precancelled_token_stops_assembled_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();
    let model = Arc::new(ScriptedModel::new(vec![Scripted::Text(
        "should not run".into(),
    )]));
    let (built, sink) = assemble_test(ws, model, Arc::new(AlwaysApprove));
    let mut ctx = WindowContext::new(Message::system(built.system_prompt.clone()));

    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel(); // cancelled before the run starts

    built
        .loop_
        .run_with_cancel(&mut ctx, "go".into(), cancel)
        .await
        .unwrap();

    // Only the terminal Done(Cancelled); no Usage/Token (model never consulted).
    let events = sink.events.lock().unwrap().clone();
    assert_eq!(
        events,
        vec!["done:Cancelled".to_string()],
        "events: {events:?}"
    );
    assert!(
        sink.text.lock().unwrap().is_empty(),
        "no model text should stream"
    );
}

struct DenyAll;
#[async_trait::async_trait]
impl ApprovalChannel for DenyAll {
    async fn request(&self, _r: ApprovalRequest) -> ApprovalResponse {
        ApprovalResponse::Deny
    }
}

// T3 — C: a `..`-escaping read now routes through approval (the normalized engine
// returns Ask instead of silently Allow). The sink witnesses the Approval event;
// the channel denies, so the read is rejected and fed back as an error result.
// (On pre-C code the gate returned Allow → no approval event.)
#[tokio::test]
async fn c_escaping_read_requests_approval_through_assembled_loop() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();
    let model = Arc::new(ScriptedModel::new(vec![
        Scripted::Call(
            "c1".into(),
            "read_file".into(),
            r#"{"path":"../../escape.txt"}"#.into(),
        ),
        Scripted::Text("ok".into()),
    ]));
    let (built, sink) = assemble_test(ws, model, Arc::new(DenyAll));
    let mut ctx = WindowContext::new(Message::system(built.system_prompt.clone()));

    built
        .loop_
        .run(&mut ctx, "read escaping".into())
        .await
        .unwrap();

    let events = sink.events.lock().unwrap().clone();
    assert!(
        events.iter().any(|e| e.starts_with("approval:read_file")),
        "escaping read must request approval (normalized gate); events: {events:?}"
    );
    // Cluster-2 pinning: a denied call must surface a terminal tool_result with its status.
    assert!(
        events
            .iter()
            .any(|e| e.starts_with("tool_result:") && e.ends_with(":denied")),
        "denied call must emit a terminal ToolResult event, got: {events:?}"
    );
}

// T4 — B2: a truncated model stream (no finish_reason / [DONE]) is detected,
// retried, and finally surfaced as an Error through the assembled loop — not
// silently accepted as a clean completion. (On pre-B2 code: clean Ok, no Error.)
#[tokio::test]
async fn b2_truncated_stream_surfaces_error_through_assembled_loop() {
    let server = MockServer::start().await;
    // A content delta, then the body just ends: no finish_reason, no [DONE].
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().to_path_buf();
    let model = Arc::new(OpenAiCompatClient::new(
        server.uri(),
        "test-model".into(),
        None,
    ));
    let (built, sink) = assemble_test(ws, model, Arc::new(AlwaysApprove));
    let mut ctx = WindowContext::new(Message::system(built.system_prompt.clone()));

    // max_retries is 3 (hardcoded in loop_config_from): 4 attempts, all truncated,
    // then the loop gives up and propagates the model error.
    let err = built.loop_.run(&mut ctx, "go".into()).await.unwrap_err();
    assert!(
        matches!(err, AgentError::Model(_)),
        "expected Model error, got {err:?}"
    );
    let events = sink.events.lock().unwrap().clone();
    assert!(
        events.iter().any(|e| e.starts_with("error:")),
        "the truncation must surface as an Error event; events: {events:?}"
    );
}
