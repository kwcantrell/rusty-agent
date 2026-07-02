//! Deterministic, CI-runnable e2e for the advanced context-management subsystem,
//! driven through the real `AgentLoop` with a scripted model. Complements the
//! per-crate unit tests (offload policy, store, curated build) and the
//! `assemble.rs` registration test by exercising the full live round-trip:
//! a tool result is offloaded out of the window by `maintain`, then the model
//! pulls it back with `context_recall` and the loop re-injects the exact bytes.
//!
//! See docs/superpowers/specs/2026-06-25-context-management-design.md.

use agent_core::testkit::{AlwaysApprove, Scripted, ScriptedModel};
use agent_core::{
    AgentEvent, AgentLoop, ContextManager, ContextRecallTool, CuratedContext, EventSink,
    InMemoryOffloadStore, LoopConfig, OffloadConfig, OffloadStore,
};
use agent_model::{Message, NativeProtocol, Role};
use agent_policy::RulePolicy;
use agent_tools::{
    Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolRegistry, ToolSchema,
};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Records each successful `ToolResult` as (name, content) so a test can assert
/// the exact bytes a tool returned — `context_recall` returns the rehydrated
/// content as its result, captured here before any later re-offload.
#[derive(Default)]
struct Capture {
    tool_results: Mutex<Vec<(String, String)>>,
    done: Mutex<bool>,
}
impl EventSink for Capture {
    fn emit(&self, e: AgentEvent) {
        match e {
            // Failures now also emit ToolResult (Task 1); filter on Ok so this sink counts
            // only successful blob results, mirroring pre-change semantics.
            AgentEvent::ToolResult {
                name,
                output,
                status: agent_core::ToolStatus::Ok,
                ..
            } => self
                .tool_results
                .lock()
                .unwrap()
                .push((name, output.content)),
            AgentEvent::Done(_) => *self.done.lock().unwrap() = true,
            _ => {}
        }
    }
}
impl Capture {
    fn recall_content(&self) -> Option<String> {
        self.tool_results
            .lock()
            .unwrap()
            .iter()
            .find(|(n, _)| n == "context_recall")
            .map(|(_, c)| c.clone())
    }
}

/// A tool that always fails with a large error body — the canonical thing the
/// offload policy lifts out of the live window.
struct BoomTool {
    message: String,
}
#[async_trait::async_trait]
impl Tool for BoomTool {
    fn name(&self) -> &str {
        "boom"
    }
    fn description(&self) -> &str {
        "always fails with a large error"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "boom".into(),
            description: "always fails".into(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        }
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "boom".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "boom".into(),
        })
    }
    async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Failed {
            message: self.message.clone(),
            stderr: None,
        })
    }
}

fn loop_config(workspace: std::path::PathBuf) -> LoopConfig {
    LoopConfig {
        model_limit: 100_000,
        max_turns: 6,
        max_retries: 1,
        temperature: 0.0,
        max_tokens: Some(256),
        workspace,
        tool_timeout: Duration::from_secs(30),
        stream_idle_timeout: Duration::from_secs(120),
        ..Default::default()
    }
}

fn build_loop(
    reg: ToolRegistry,
    model: ScriptedModel,
    sink: Arc<Capture>,
    ws: std::path::PathBuf,
) -> AgentLoop {
    AgentLoop::new(
        Arc::new(model),
        Arc::new(NativeProtocol),
        Arc::new(reg),
        Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec![],
            command_denylist: vec![],
        }),
        Arc::new(AlwaysApprove),
        sink,
        loop_config(ws),
    )
}

/// The headline round-trip: a large tool error is auto-offloaded by `maintain`
/// after turn 1; on turn 2 the model calls `context_recall(1)` and the loop
/// returns the original error verbatim.
#[tokio::test]
async fn offload_then_recall_round_trips_through_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();

    // Shared store: the same handle the recall tool reads and the context offloads into.
    let store: Arc<dyn OffloadStore> = Arc::new(InMemoryOffloadStore::new());
    let flag = Arc::new(AtomicBool::new(false));

    let big_message = format!("disk exploded at sector {}", "9".repeat(300));
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(BoomTool {
        message: big_message.clone(),
    }));
    reg.register(Arc::new(ContextRecallTool::new(
        store.clone(),
        agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES,
    )));

    let model = ScriptedModel::new(vec![
        Scripted::Call("c1".into(), "boom".into(), "{}".into()),
        Scripted::Call("c2".into(), "context_recall".into(), r#"{"id":1}"#.into()),
        Scripted::Text("Recovered the error; all done.".into()),
    ]);
    let sink = Arc::new(Capture::default());

    // keep_recent: 0 makes the single turn-1 error immediately eligible to offload.
    let mut ctx = CuratedContext::new(Message::system("SYS"), store.clone(), flag)
        .with_offload_config(OffloadConfig {
            keep_recent: 0,
            error_min_bytes: 50,
            ..Default::default()
        });

    build_loop(reg, model, sink.clone(), ws)
        .run(&mut ctx, "Trigger the failure, then recover it.".into())
        .await
        .unwrap();

    // The boom error was offloaded (entry #1) and recall returned it verbatim.
    let expected = format!("ERROR: failed: {big_message}");
    assert!(
        store.get(1).is_some(),
        "the large error must have been offloaded as entry #1"
    );
    assert_eq!(
        store.get(1).unwrap().content,
        expected,
        "stored content must be the raw error"
    );
    assert_eq!(
        sink.recall_content().as_deref(),
        Some(expected.as_str()),
        "context_recall must return the exact offloaded bytes"
    );
    assert!(
        *sink.done.lock().unwrap(),
        "the run reached a normal completion"
    );
}

/// The error path: recalling an id that was never offloaded feeds a normal tool
/// error back to the model, which continues rather than crashing.
#[tokio::test]
async fn recall_unknown_id_feeds_error_back_and_continues() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();

    let store: Arc<dyn OffloadStore> = Arc::new(InMemoryOffloadStore::new());
    let flag = Arc::new(AtomicBool::new(false));

    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(ContextRecallTool::new(
        store.clone(),
        agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES,
    )));

    let model = ScriptedModel::new(vec![
        Scripted::Call("c1".into(), "context_recall".into(), r#"{"id":999}"#.into()),
        Scripted::Text("Nothing there; carrying on.".into()),
    ]);
    let sink = Arc::new(Capture::default());

    // Default offload config: the short not-found error stays in the window so we
    // can assert it was fed back to the model.
    let mut ctx = CuratedContext::new(Message::system("SYS"), store, flag);

    build_loop(reg, model, sink.clone(), ws)
        .run(&mut ctx, "Recall entry 999.".into())
        .await
        .unwrap();

    // The loop continued to a clean finish despite the recall error.
    assert!(
        *sink.done.lock().unwrap(),
        "an unknown-id recall must not abort the run"
    );
    // The not-found error was appended to the transcript as a tool message.
    let built = ctx.build(100_000);
    let fed_back = built.iter().any(|m| {
        matches!(m.role, Role::Tool)
            && m.name.as_deref() == Some("context_recall")
            && m.content.contains("no offloaded entry #999")
    });
    assert!(
        fed_back,
        "the unknown-id error must be fed back as a context_recall tool result"
    );
}
