//! Integration tests for DispatchAgentTool (spec D1-D13 core behaviors).
use agent_core::testkit::{AlwaysApprove, PassthroughProtocol, Scripted, ScriptedModel};
use agent_core::{
    AgentEvent, AgentLoop, CuratedContext, DispatchAgentTool, DispatchDeps, EventSink, LoopConfig,
    SessionArtifacts, SubAgentRegistry, SUBAGENT_PREAMBLE,
};
use agent_model::Message;
use agent_policy::{Decision, PolicyEngine, RulePolicy};
use agent_tools::backend::{Backend, CompositeBackend, HostBackend, ReadOnlyToTools};
use agent_tools::{
    Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolRegistry, ToolSchema,
};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Full-fidelity parent sink (testkit CollectingSink drops ids).
#[derive(Default)]
struct FullSink {
    events: Mutex<Vec<(String, String, String, String)>>, // (kind, id, name, parent)
}
impl EventSink for FullSink {
    fn emit(&self, event: AgentEvent) {
        let t = match event {
            AgentEvent::ToolStart {
                id,
                name,
                parent_id,
                ..
            } => (
                "tool_start".to_string(),
                id,
                name,
                parent_id.unwrap_or_default(),
            ),
            AgentEvent::ToolResult {
                id,
                name,
                status,
                parent_id,
                ..
            } => (
                format!("tool_result:{}", status.as_str()),
                id,
                name,
                parent_id.unwrap_or_default(),
            ),
            AgentEvent::Token(t) => ("token".to_string(), String::new(), t, String::new()),
            AgentEvent::Done(_) => (
                "done".to_string(),
                String::new(),
                String::new(),
                String::new(),
            ),
            AgentEvent::ServerUsage {
                prompt_tokens,
                parent_id,
                ..
            } => (
                "server_usage".to_string(),
                prompt_tokens.to_string(),
                String::new(),
                parent_id.unwrap_or_default(),
            ),
            AgentEvent::Subagent(agent_core::SubagentEvent::Start { id, .. }) => {
                ("subagent_start".into(), id, String::new(), String::new())
            }
            _ => return,
        };
        self.events.lock().unwrap().push(t);
    }
}

/// A trivial child-visible tool.
struct Echo;
#[async_trait::async_trait]
impl Tool for Echo {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echo"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "echo".into(),
            description: "echo".into(),
            parameters: serde_json::json!({"type":"object"}),
        }
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "echo".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "echo".into(),
        })
    }
    async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            content: "echoed".into(),
            display: None,
        })
    }
}

/// A second child-visible tool, used to pin transitive allowlist scoping.
struct Echo2;
#[async_trait::async_trait]
impl Tool for Echo2 {
    fn name(&self) -> &str {
        "echo2"
    }
    fn description(&self) -> &str {
        "echo2"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "echo2".into(),
            description: "echo2".into(),
            parameters: serde_json::json!({"type":"object"}),
        }
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "echo2".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "echo2".into(),
        })
    }
    async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            content: "echoed2".into(),
            display: None,
        })
    }
}

fn workspace() -> PathBuf {
    std::env::temp_dir()
}

fn child_config(ws: PathBuf) -> LoopConfig {
    LoopConfig {
        model_limit: 16384,
        max_turns: 5,
        max_retries: 1,
        tool_timeout: Duration::from_secs(5),
        stream_idle_timeout: Duration::from_secs(3600),
        workspace: ws,
        ..LoopConfig::default()
    }
}

fn deps(model: ScriptedModel, sink: Arc<dyn EventSink>, base: Vec<Arc<dyn Tool>>) -> DispatchDeps {
    let ws = workspace();
    DispatchDeps {
        model: Arc::new(model),
        protocol: Arc::new(PassthroughProtocol),
        policy: Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec![],
            command_denylist: vec![],
        }),
        approval: Arc::new(AlwaysApprove),
        sink,
        child_trace: None,
        base_tools: base,
        child_system_prompt: format!("SYS\n\n{SUBAGENT_PREAMBLE}"),
        loop_config: child_config(ws),
        max_result_bytes: 16 * 1024,
        subagent_timeout: Duration::from_secs(600),
        compaction_model: None,
        depth: 1,
        max_depth: 1,
        id_prefix: String::new(),
        description_overrides: Default::default(),
        subagents: Arc::new(SubAgentRegistry::default()),
        memories: None,
        checkpoint: None,
    }
}

fn tool_ctx() -> ToolCtx {
    ToolCtx {
        workspace: workspace(),
        timeout: Duration::from_secs(600),
        cancel: CancellationToken::new(),
        sandbox: Arc::new(agent_tools::HostExecutor),
        backend: Arc::new(agent_tools::backend::HostBackend::new(workspace())),
        call_id: "d1".into(),
    }
}

#[tokio::test]
async fn returns_child_final_text_with_footer() {
    let sink = Arc::new(FullSink::default());
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::Text("hello from child".into())]),
        sink,
        vec![],
    ));
    let out = tool
        .execute(serde_json::json!({"prompt": "do the thing"}), &tool_ctx())
        .await
        .unwrap();
    assert!(
        out.content.starts_with("hello from child"),
        "{}",
        out.content
    );
    assert!(out.content.contains("[sub-agent: "), "{}", out.content);
    assert!(out.content.contains("stop: Stop"), "{}", out.content);
}

#[tokio::test]
async fn child_tool_calls_are_forwarded_rewritten_and_tokens_suppressed() {
    let sink = Arc::new(FullSink::default());
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "echo".into(), "{}".into()),
            Scripted::Text("final".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Echo)],
    ));
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    assert!(out.content.starts_with("final"));
    let events = sink.events.lock().unwrap().clone();
    // Child echo call forwarded with rewritten id/name; NO child token/done leaked.
    assert!(
        events.iter().any(|(k, i, n, _)| k == "tool_start"
            && i.contains(":c1")
            && i.starts_with("sub")
            && n == "sub:echo"),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|(k, _, n, _)| k == "tool_result:ok" && n == "sub:echo"),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|(k, _, _, _)| k == "token" || k == "done"),
        "{events:?}"
    );
}

#[tokio::test]
async fn forwarded_child_events_carry_the_dispatch_call_id() {
    let sink = Arc::new(FullSink::default());
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "echo".into(), "{}".into()),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Echo)],
    ));
    tool.execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    let events = sink.events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .filter(|e| e.0.starts_with("tool_"))
            .all(|e| e.3 == "d1"),
        "{events:?}"
    );
}

#[tokio::test]
async fn allowlist_accepts_always_available_context_tools() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::Text("x".into())]),
        Arc::new(FullSink::default()),
        vec![Arc::new(Echo)],
    ));
    // context_compact is not in base_tools but IS always registered for the child.
    let out = tool
        .execute(
            serde_json::json!({"prompt": "p", "tools": ["context_compact"]}),
            &tool_ctx(),
        )
        .await;
    assert!(out.is_ok(), "{out:?}");
    // Genuinely unknown names still error, and the message names the implicit tools.
    let err = tool
        .execute(
            serde_json::json!({"prompt": "p", "tools": ["nope"]}),
            &tool_ctx(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, ToolError::InvalidArgs(ref m)
        if m.contains("nope") && m.contains("context_compact")),
        "{err:?}"
    );
}

#[tokio::test]
async fn budget_exhausted_child_reports_it() {
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "echo".into(), "{}".into()),
            Scripted::Call("c2".into(), "echo".into(), "{}".into()),
        ]),
        sink,
        vec![Arc::new(Echo)],
    );
    d.loop_config.max_turns = 1;
    let tool = DispatchAgentTool::new(d);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    assert!(out.content.contains("turn budget"), "{}", out.content);
    assert!(
        out.content.contains("stop: BudgetExhausted"),
        "{}",
        out.content
    );
}

/// The headline dispatch benefit: a child that exhausts its turn budget still
/// hands the parent a real wrap-up SUMMARY. The child's last (tools-disabled)
/// wrap-up turn is a `Scripted::Text`, so its text must surface in the parent's
/// dispatch tool result — alongside the `stop: BudgetExhausted` footer.
#[tokio::test]
async fn budget_exhausted_child_wrap_up_summary_reaches_parent() {
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            // The one real turn (budget = 1): a tool call.
            Scripted::Call("c1".into(), "echo".into(), "{}".into()),
            // The tools-disabled wrap-up completion the child gets after budget
            // exhaustion — plain text, which should reach the parent verbatim.
            Scripted::Text("child wrap-up summary".into()),
        ]),
        sink,
        vec![Arc::new(Echo)],
    );
    d.loop_config.max_turns = 1;
    let tool = DispatchAgentTool::new(d);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    assert!(
        out.content.contains("child wrap-up summary"),
        "{}",
        out.content
    );
    assert!(
        out.content.contains("stop: BudgetExhausted"),
        "{}",
        out.content
    );
}

#[tokio::test]
async fn tools_allowlist_filters_and_rejects_unknown_names() {
    let sink = Arc::new(FullSink::default());
    // Unknown name -> InvalidArgs listing available.
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::Text("x".into())]),
        sink.clone(),
        vec![Arc::new(Echo)],
    ));
    let err = tool
        .execute(
            serde_json::json!({"prompt": "p", "tools": ["nope"]}),
            &tool_ctx(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, ToolError::InvalidArgs(ref m) if m.contains("nope") && m.contains("echo")),
        "{err:?}"
    );

    // Filtered-out tool is unknown to the child (gate rejects it as Denied).
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "echo".into(), "{}".into()),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Echo)],
    ));
    let out = tool
        .execute(serde_json::json!({"prompt": "p", "tools": []}), &tool_ctx())
        .await
        .unwrap();
    assert!(out.content.starts_with("done"));
    let events = sink.events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .any(|(k, _, n, _)| k == "tool_result:denied" && n == "sub:echo"),
        "{events:?}"
    );
}

#[tokio::test]
async fn missing_prompt_is_invalid_args() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let err = tool
        .execute(serde_json::json!({}), &tool_ctx())
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)));
}

#[test]
fn intent_is_readonly_and_auto_allowed() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let intent = tool
        .intent(&serde_json::json!({"prompt": "summarize the repo"}))
        .unwrap();
    assert!(matches!(intent.access, Access::Read));
    assert!(intent.paths.is_empty());
    assert!(intent.command.is_none());
    assert!(intent.summary.contains("summarize"));
    let policy = RulePolicy {
        workspace: workspace(),
        command_allowlist: vec![],
        command_denylist: vec![],
    };
    assert!(matches!(policy.check(&intent), Decision::Allow));
}

#[test]
fn timeout_override_is_the_configured_subagent_timeout() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    assert_eq!(tool.timeout_override(), Some(Duration::from_secs(600)));
}

#[test]
fn schema_describes_required_prompt() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let s = tool.schema();
    assert_eq!(s.name, "dispatch_agent");
    assert!(agent_tools::required_params_missing_description(&s).is_empty());
    assert!(tool.when_not_to_call().is_some());
}

#[test]
fn intent_summary_is_single_line_for_a_multiline_prompt() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let intent = tool
        .intent(&serde_json::json!({"prompt": "line one\nline two\r\nline three"}))
        .unwrap();
    assert!(!intent.summary.contains('\n'), "{}", intent.summary);
    assert!(!intent.summary.contains('\r'), "{}", intent.summary);
    assert!(
        intent.summary.contains("line one line two"),
        "{}",
        intent.summary
    );
}

/// Records approval requests; replies with a fixed response.
struct RecordingApproval {
    seen: Mutex<Vec<String>>,
    reply: agent_policy::ApprovalResponse,
}
#[async_trait::async_trait]
impl agent_policy::ApprovalChannel for RecordingApproval {
    async fn request(&self, req: agent_policy::ApprovalRequest) -> agent_policy::ApprovalResponse {
        self.seen.lock().unwrap().push(req.intent.summary.clone());
        self.reply.clone()
    }
}

/// A Write-access tool: policy says Ask, so the shared approval channel decides.
struct Writey;
#[async_trait::async_trait]
impl Tool for Writey {
    fn name(&self) -> &str {
        "writey"
    }
    fn description(&self) -> &str {
        "writes"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "writey".into(),
            description: "writes".into(),
            parameters: serde_json::json!({"type":"object"}),
        }
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "writey".into(),
            access: Access::Write,
            paths: vec![],
            command: None,
            summary: "write something".into(),
        })
    }
    async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            content: "wrote".into(),
            display: None,
        })
    }
}

#[tokio::test]
async fn child_ask_routes_to_the_shared_approval_channel_and_deny_sticks() {
    let sink = Arc::new(FullSink::default());
    let approval = Arc::new(RecordingApproval {
        seen: Mutex::new(vec![]),
        reply: agent_policy::ApprovalResponse::Deny { feedback: None },
    });
    let mut d = deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "writey".into(), "{}".into()),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Writey)],
    );
    d.approval = approval.clone();
    let tool = DispatchAgentTool::new(d);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    // The Ask reached the PARENT's channel (spec Invariant / D2)...
    assert_eq!(
        approval.seen.lock().unwrap().as_slice(),
        &["write something".to_string()]
    );
    // ...and the denial reached the child (forwarded as a denied tool_result).
    let events = sink.events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .any(|(k, _, n, _)| k == "tool_result:denied" && n == "sub:writey"),
        "{events:?}"
    );
    assert!(out.content.starts_with("done"));
}

#[tokio::test]
async fn child_ask_approve_executes() {
    let sink = Arc::new(FullSink::default());
    let approval = Arc::new(RecordingApproval {
        seen: Mutex::new(vec![]),
        reply: agent_policy::ApprovalResponse::Approve,
    });
    let mut d = deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "writey".into(), "{}".into()),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Writey)],
    );
    d.approval = approval;
    let tool = DispatchAgentTool::new(d);
    tool.execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    let events = sink.events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .any(|(k, _, n, _)| k == "tool_result:ok" && n == "sub:writey"),
        "{events:?}"
    );
}

#[tokio::test]
async fn child_cannot_recurse_into_dispatch_agent() {
    let sink = Arc::new(FullSink::default());
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "dispatch_agent".into(),
                r#"{"prompt":"nested"}"#.into(),
            ),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Echo)],
    ));
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    assert!(out.content.starts_with("done"));
    // The child's gate rejected the unknown tool (Denied, "not found").
    let events = sink.events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .any(|(k, _, n, _)| k == "tool_result:denied" && n == "sub:dispatch_agent"),
        "{events:?}"
    );
}

/// A stub deliberately NAMED `dispatch_agent`, to prove the in-tool skip
/// (dispatch.rs:279-281) drops it from the child registry even when a caller
/// leaks it into the base snapshot — defense in depth for D4.
struct StubDispatch;
#[async_trait::async_trait]
impl Tool for StubDispatch {
    fn name(&self) -> &str {
        "dispatch_agent"
    }
    fn description(&self) -> &str {
        "stub"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "dispatch_agent".into(),
            description: "stub".into(),
            parameters: serde_json::json!({"type":"object"}),
        }
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "dispatch_agent".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "stub".into(),
        })
    }
    async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            content: "STUB DISPATCH RAN".into(),
            display: None,
        })
    }
}

#[tokio::test]
async fn dispatch_agent_in_base_tools_is_still_excluded_from_child() {
    let sink = Arc::new(FullSink::default());
    // base_tools DELIBERATELY contains a tool named "dispatch_agent" — the child
    // registry must still exclude it (dispatch.rs:279-281), so the child's call
    // is denied as unknown rather than executing the stub.
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "dispatch_agent".into(),
                r#"{"prompt":"nested"}"#.into(),
            ),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(StubDispatch)],
    ));
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    assert!(out.content.starts_with("done"));
    // The stub never executed; the child gate denied the excluded name.
    assert!(
        !out.content.contains("STUB DISPATCH RAN"),
        "{}",
        out.content
    );
    let events = sink.events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .any(|(k, _, n, _)| k == "tool_result:denied" && n == "sub:dispatch_agent"),
        "{events:?}"
    );
}

#[tokio::test]
async fn pre_cancelled_parent_token_cancels_the_child() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::Text("never returned".into())]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let ctx = tool_ctx();
    ctx.cancel.cancel();
    let err = tool
        .execute(serde_json::json!({"prompt": "p"}), &ctx)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ToolError::Failed { ref message, .. } if message.contains("cancelled")),
        "{err:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn wall_clock_timeout_cancels_the_child_and_reports_timeout() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::HangOpen]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let mut ctx = tool_ctx();
    ctx.timeout = Duration::from_secs(1);
    let started = tokio::time::Instant::now();
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &ctx)
        .await
        .unwrap();
    assert!(
        out.content.starts_with("[sub-agent timed out after 1s"),
        "{}",
        out.content
    );
    assert_eq!(started.elapsed(), Duration::from_secs(1)); // virtual time: exactly the budget
}

#[tokio::test(start_paused = true)]
async fn parent_cancel_mid_run_resolves_to_cancelled_promptly() {
    // Child hangs on an open stream (HangOpen) with a huge idle timeout; the
    // parent cancel must tear it down mid-run and surface a "cancelled" error.
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::HangOpen]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let ctx = tool_ctx(); // timeout 600s; child stream_idle 3600s (child_config)
    let cancel = ctx.cancel.clone();
    // Cancel once the child is in flight (parked on the open stream). The child
    // token derives from ctx.cancel, so this propagates down. Cancellation fires
    // via task scheduling (yield), not a timer, so the paused clock never
    // advances and the 600s wall-clock timeout never fires — no real sleeps.
    let canceller = tokio::spawn(async move {
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        cancel.cancel();
    });
    let err = tool
        .execute(serde_json::json!({"prompt": "p"}), &ctx)
        .await
        .unwrap_err();
    canceller.await.unwrap();
    assert!(
        matches!(err, ToolError::Failed { ref message, .. } if message.contains("cancelled")),
        "{err:?}"
    );
}

/// Records every CompletionRequest's system message, then delegates to a script.
struct CapturingModel {
    inner: ScriptedModel,
    systems: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl agent_model::ModelClient for CapturingModel {
    async fn stream(
        &self,
        req: agent_model::CompletionRequest,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<agent_model::Chunk, agent_model::ModelError>>,
        agent_model::ModelError,
    > {
        if let Some(m) = req.messages.first() {
            self.systems.lock().unwrap().push(m.content.clone());
        }
        self.inner.stream(req).await
    }
}

#[tokio::test]
async fn role_arg_lands_in_the_child_system_prompt() {
    let model = Arc::new(CapturingModel {
        inner: ScriptedModel::new(vec![Scripted::Text("ok".into())]),
        systems: Mutex::new(vec![]),
    });
    let mut d = deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    );
    d.model = model.clone();
    let tool = DispatchAgentTool::new(d);
    tool.execute(
        serde_json::json!({"prompt": "p", "role": "You are a meticulous code reviewer."}),
        &tool_ctx(),
    )
    .await
    .unwrap();
    let systems = model.systems.lock().unwrap().clone();
    assert!(
        systems[0].contains("Role: You are a meticulous code reviewer."),
        "{systems:?}"
    );
}

#[tokio::test]
async fn role_is_optional_and_bounded() {
    let mk = || {
        DispatchAgentTool::new(deps(
            ScriptedModel::new(vec![Scripted::Text("ok".into())]),
            Arc::new(FullSink::default()),
            vec![],
        ))
    };
    // Absent role: fine (no Role block asserted via the capturing test above).
    mk().execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    // Whitespace-only role: treated as absent (no error).
    mk().execute(
        serde_json::json!({"prompt": "p", "role": "   "}),
        &tool_ctx(),
    )
    .await
    .unwrap();
    // Over 2000 chars: InvalidArgs.
    let long = "r".repeat(2001);
    let err = mk()
        .execute(
            serde_json::json!({"prompt": "p", "role": long}),
            &tool_ctx(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, ToolError::InvalidArgs(ref m) if m.contains("role")),
        "{err:?}"
    );
    // Non-string role: InvalidArgs.
    let err = mk()
        .execute(serde_json::json!({"prompt": "p", "role": 7}), &tool_ctx())
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)), "{err:?}");
}

#[tokio::test]
async fn parallel_dispatches_get_distinct_ordinals_and_both_complete() {
    let sink = Arc::new(FullSink::default());
    let mk = |sink: Arc<FullSink>| {
        DispatchAgentTool::new(deps(
            ScriptedModel::new(vec![
                Scripted::Call("c1".into(), "echo".into(), "{}".into()),
                Scripted::Text("done".into()),
            ]),
            sink,
            vec![Arc::new(Echo)],
        ))
    };
    let (a, b) = (mk(sink.clone()), mk(sink.clone()));
    let (ca, cb) = (tool_ctx(), tool_ctx());
    let (ra, rb) = tokio::join!(
        a.execute(serde_json::json!({"prompt": "a"}), &ca),
        b.execute(serde_json::json!({"prompt": "b"}), &cb),
    );
    ra.unwrap();
    rb.unwrap();
    // Two children each made one echo call; the two forwarded start ids carry
    // distinct sub{n} prefixes even though both children used child-id c1.
    let ids: Vec<String> = sink
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|(k, _, _, _)| k == "tool_start")
        .map(|(_, id, _, _)| id.clone())
        .collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0], ids[1], "{ids:?}");
}

#[tokio::test]
async fn depth_two_child_can_dispatch_and_grandchild_attribution_chains() {
    let sink = Arc::new(FullSink::default());
    // Child model: dispatches a grandchild, then answers.
    // Grandchild model comes from the SAME deps.model (scripted queue): its turn
    // is the "gc done" text. Order: child turn1 -> grandchild turn -> child turn2.
    let tap = Arc::new(TapSpy::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "dispatch_agent".into(),
                r#"{"prompt":"nested task"}"#.into(),
            ),
            Scripted::Text("gc done".into()), // consumed by the GRANDCHILD loop
            Scripted::Text("child done".into()), // child's final answer
        ]),
        sink.clone(),
        vec![],
    );
    d.max_depth = 2;
    d.child_trace = Some(tap.clone());
    let tool = DispatchAgentTool::new(d);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    assert!(out.content.starts_with("child done"), "{}", out.content);

    let events = sink.events.lock().unwrap().clone();
    // The child's dispatch call is visible as sub{n}:c1 with parent "d1";
    // the grandchild's? there are no grandchild TOOL calls here, so the pin is:
    // the child-level dispatch_agent tool_start row itself chains to d1...
    let child_dispatch_start = events
        .iter()
        .find(|(k, _, name, _)| k == "tool_start" && name == "sub:dispatch_agent")
        .expect("child dispatch row forwarded");
    assert_eq!(child_dispatch_start.3, "d1");
    // ...and the grandchild's ServerUsage carries parent_id == the child's VISIBLE id.
    let child_visible_id = child_dispatch_start.1.clone(); // "sub{n}:c1"
    assert!(child_visible_id.ends_with(":c1"), "{child_visible_id}");
    // Concrete grandchild pin: the grandchild's ServerUsage specifically carries
    // the child's visible id as its parent_id (kind-constrained so a stray
    // forward can't satisfy the pin).
    assert!(
        events
            .iter()
            .any(|(kind, _, _, parent)| kind == "server_usage" && parent == &child_visible_id),
        "no server_usage chained to the child's visible id {child_visible_id}: {events:?}"
    );
    // Tap pin (spec Testing): grandchild suppressed events reach the tap with
    // the prefixed parent id.
    let tap_parents = tap.seen.lock().unwrap().clone();
    assert!(
        tap_parents.iter().any(|(_, p, _)| p == &child_visible_id),
        "no tap record chained to {child_visible_id}: {tap_parents:?}"
    );
}

/// Local tap spy (SubagentTrace is pub): records (ordinal, parent_id, kind).
#[derive(Default)]
struct TapSpy {
    seen: Mutex<Vec<(u64, String, &'static str)>>,
}
impl agent_core::SubagentTrace for TapSpy {
    fn record(&self, n: u64, parent_id: &str, event: &agent_core::AgentEvent) {
        let kind = match event {
            agent_core::AgentEvent::Token(_) => "token",
            agent_core::AgentEvent::Done(_) => "done",
            _ => "other",
        };
        self.seen
            .lock()
            .unwrap()
            .push((n, parent_id.to_string(), kind));
    }
}

#[tokio::test]
async fn depth_two_is_the_floor_for_the_grandchild() {
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            // Child dispatches grandchild; grandchild TRIES to dispatch (rejected: unknown tool).
            Scripted::Call(
                "c1".into(),
                "dispatch_agent".into(),
                r#"{"prompt":"nested"}"#.into(),
            ),
            Scripted::Call(
                "g1".into(),
                "dispatch_agent".into(),
                r#"{"prompt":"三"}"#.into(),
            ),
            Scripted::Text("gc done".into()),
            Scripted::Text("child done".into()),
        ]),
        sink.clone(),
        vec![],
    );
    d.max_depth = 2;
    let tool = DispatchAgentTool::new(d);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    assert!(out.content.starts_with("child done"), "{}", out.content);
    let events = sink.events.lock().unwrap().clone();
    // The grandchild's dispatch attempt is a denied tool_result at depth 3, and
    // the denied row is specifically the grandchild's own g1 call (its visible id
    // ends with the grandchild's child-side call id ":g1").
    assert!(
        events.iter().any(|(k, id, n, _)| k == "tool_result:denied"
            && n == "sub:dispatch_agent"
            && id.ends_with(":g1")),
        "{events:?}"
    );
}

#[tokio::test]
async fn default_depth_one_matches_v1_no_recursion() {
    // deps() defaults max_depth = 1: the existing child_cannot_recurse_into_dispatch_agent
    // test already pins this; this test pins the DEFAULT deps value explicitly.
    let d = deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    );
    assert_eq!((d.depth, d.max_depth), (1, 1));
}

#[tokio::test]
async fn allowlist_without_dispatch_agent_denies_child_dispatch_at_depth_two() {
    // I-1 (i): depth allows nesting (max_depth 2), but the allowlist does NOT
    // name dispatch_agent → the nested tool is not registered, so the child's
    // dispatch attempt is denied as an unknown tool.
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "dispatch_agent".into(),
                r#"{"prompt":"nested"}"#.into(),
            ),
            Scripted::Text("child done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Echo)],
    );
    d.max_depth = 2;
    let tool = DispatchAgentTool::new(d);
    let out = tool
        .execute(
            serde_json::json!({"prompt": "p", "tools": ["echo"]}),
            &tool_ctx(),
        )
        .await
        .unwrap();
    assert!(out.content.starts_with("child done"), "{}", out.content);
    let events = sink.events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .any(|(k, _, n, _)| k == "tool_result:denied" && n == "sub:dispatch_agent"),
        "{events:?}"
    );
}

#[tokio::test]
async fn allowlist_with_dispatch_agent_nests_and_scopes_grandchild_transitively() {
    // I-1 (ii): allowlist ["echo","dispatch_agent"] at max_depth 2 → the child
    // dispatches a grandchild successfully, and the grandchild inherits the
    // FILTERED base (echo only): calling echo works, calling the non-allowlisted
    // echo2 is denied (transitive scope pin).
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            // child turn1: dispatch a grandchild
            Scripted::Call(
                "c1".into(),
                "dispatch_agent".into(),
                r#"{"prompt":"nested"}"#.into(),
            ),
            // grandchild turn1: echo (allowlisted → ok)
            Scripted::Call("g1".into(), "echo".into(), "{}".into()),
            // grandchild turn2: echo2 (NOT in the filtered base → denied)
            Scripted::Call("g2".into(), "echo2".into(), "{}".into()),
            // grandchild turn3: answer
            Scripted::Text("gc done".into()),
            // child turn2: answer
            Scripted::Text("child done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Echo), Arc::new(Echo2)],
    );
    d.max_depth = 2;
    let tool = DispatchAgentTool::new(d);
    let out = tool
        .execute(
            serde_json::json!({"prompt": "p", "tools": ["echo", "dispatch_agent"]}),
            &tool_ctx(),
        )
        .await
        .unwrap();
    assert!(out.content.starts_with("child done"), "{}", out.content);
    let events = sink.events.lock().unwrap().clone();
    // Child dispatched the grandchild successfully.
    assert!(
        events
            .iter()
            .any(|(k, _, n, _)| k == "tool_result:ok" && n == "sub:dispatch_agent"),
        "{events:?}"
    );
    // Grandchild's echo (allowlisted) worked.
    assert!(
        events
            .iter()
            .any(|(k, _, n, _)| k == "tool_result:ok" && n == "sub:echo"),
        "{events:?}"
    );
    // Grandchild's echo2 (NOT in the transitively-filtered base) was denied.
    assert!(
        events
            .iter()
            .any(|(k, _, n, _)| k == "tool_result:denied" && n == "sub:echo2"),
        "{events:?}"
    );
}

#[tokio::test]
async fn allowlist_naming_dispatch_agent_at_depth_floor_is_invalid_args() {
    // I-1 (iii): at the depth floor (deps() default max_depth 1 == depth 1)
    // dispatch_agent is unknown, so naming it in the allowlist is InvalidArgs.
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::Text("x".into())]),
        Arc::new(FullSink::default()),
        vec![Arc::new(Echo)],
    ));
    let err = tool
        .execute(
            serde_json::json!({"prompt": "p", "tools": ["dispatch_agent"]}),
            &tool_ctx(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, ToolError::InvalidArgs(ref m) if m.contains("dispatch_agent")),
        "{err:?}"
    );
}

#[test]
fn description_mentions_concurrent_fanout() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    assert!(
        tool.description().contains("concurrently"),
        "{}",
        tool.description()
    );
}

/// Findings 2.3/4.5: the "minus dispatch_agent itself" claim is only true at
/// the depth floor; with nesting allowed the description must say the child
/// can dispatch its own sub-agents (it gets a nested dispatch_agent by default).
#[test]
fn description_is_depth_aware() {
    let base = deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    );
    // Depth floor (depth 1, max_depth 1 — the default): child cannot dispatch.
    let floor = DispatchAgentTool::new(base.clone());
    assert!(
        floor.description().contains("minus dispatch_agent itself"),
        "{}",
        floor.description()
    );
    assert!(
        floor
            .schema()
            .description
            .contains("minus dispatch_agent itself"),
        "schema must flow through the stored description"
    );
    // Nesting allowed (depth 1 < max_depth 2): the child CAN dispatch.
    let mut d = base;
    d.max_depth = 2;
    let nested = DispatchAgentTool::new(d);
    assert!(
        !nested.description().contains("minus dispatch_agent"),
        "{}",
        nested.description()
    );
    assert!(
        nested.description().contains("dispatch its own sub-agents"),
        "{}",
        nested.description()
    );
}

/// Finding 4.4: a child killed by the wall-clock timeout hands its parent the
/// captured partial transcript instead of a bare ToolError::Timeout.
#[tokio::test(start_paused = true)]
async fn timed_out_child_returns_partial_transcript() {
    use agent_model::{Chunk, RawToolCall, StopReason as MStop};
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            // Turn 1: streams text (captured as a segment) then a tool call, so
            // the run continues into turn 2.
            Scripted::Chunks(vec![
                Chunk::Text("partial progress note".into()),
                Chunk::ToolCallDelta(RawToolCall {
                    index: None,
                    id: Some("c1".into()),
                    name: Some("echo".into()),
                    args_fragment: "{}".into(),
                }),
                Chunk::Done(MStop::ToolCalls),
            ]),
            // Turn 2: hangs; the wall-clock timeout fires (virtual time).
            Scripted::Hang,
        ]),
        sink,
        vec![Arc::new(Echo)],
    );
    d.subagent_timeout = Duration::from_secs(1);
    let tool = DispatchAgentTool::new(d);
    let mut ctx = tool_ctx();
    ctx.timeout = Duration::from_secs(1);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &ctx)
        .await
        .unwrap();
    assert!(
        out.content
            .starts_with("[sub-agent timed out after 1s — partial transcript follows]"),
        "{}",
        out.content
    );
    assert!(
        out.content.contains("partial progress note"),
        "{}",
        out.content
    );
    assert!(out.content.contains("stop: timeout"), "{}", out.content);
}

/// Finding 4.4 (empty capture): a child that produced nothing still reports the
/// note + footer, with the no-transcript wording and no misleading "stop: Stop".
#[tokio::test(start_paused = true)]
async fn timed_out_child_with_no_capture_reports_note_and_footer() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![Scripted::Hang]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let mut ctx = tool_ctx();
    ctx.timeout = Duration::from_secs(1);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &ctx)
        .await
        .unwrap();
    assert!(
        out.content
            .starts_with("[sub-agent timed out after 1s — no partial transcript captured]"),
        "{}",
        out.content
    );
    assert!(
        out.content.contains("stop: timeout"),
        "no Done was recorded — the footer must not claim a clean Stop: {}",
        out.content
    );
    assert!(
        !out.content.contains("\n\n"),
        "no blank-line runs: {}",
        out.content
    );
}

/// Finding 4.4: a child whose model fails fatally still hands the parent the
/// captured partial transcript, with the failure note.
#[tokio::test]
async fn failed_child_returns_partial_transcript() {
    use agent_model::{Chunk, ModelError, RawToolCall, StopReason as MStop};
    let sink = Arc::new(FullSink::default());
    let d = deps(
        ScriptedModel::new(vec![
            Scripted::Chunks(vec![
                Chunk::Text("progress so far".into()),
                Chunk::ToolCallDelta(RawToolCall {
                    index: None,
                    id: Some("c1".into()),
                    name: Some("echo".into()),
                    args_fragment: "{}".into(),
                }),
                Chunk::Done(MStop::ToolCalls),
            ]),
            // Status 401 is Fatal on first sight (types.rs class()); a second
            // Fail keeps the test robust if classification ever loosens.
            Scripted::Fail(ModelError::Status {
                code: 401,
                body: "no auth".into(),
                retry_after: None,
            }),
            Scripted::Fail(ModelError::Status {
                code: 401,
                body: "no auth".into(),
                retry_after: None,
            }),
        ]),
        sink,
        vec![Arc::new(Echo)],
    );
    let tool = DispatchAgentTool::new(d);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();
    assert!(
        out.content.starts_with("[sub-agent failed: "),
        "{}",
        out.content
    );
    assert!(
        out.content.contains("— partial transcript follows]"),
        "{}",
        out.content
    );
    assert!(out.content.contains("progress so far"), "{}", out.content);
    // The child loop emits Done(StopReason::Error) on a fatal model error before
    // returning Err (loop_.rs), so the sink records that real stop reason — the
    // footer honestly reports "stop: Error" rather than the "failed" fallback
    // (which only fires if the child never recorded any Done at all).
    assert!(out.content.contains("stop: Error"), "{}", out.content);
}

// --- Task 10 Step 4: child-isolation pins (spec §5.7) ----------------------

/// The two artifact mounts (read-only) over a HostBackend at the workspace root
/// — exactly the composite assemble/dispatch build for a tenant.
fn composite_over(artifacts: &Arc<SessionArtifacts>, ws: &std::path::Path) -> Arc<dyn Backend> {
    Arc::new(CompositeBackend::new(
        vec![
            (
                "large_tool_results/".into(),
                Arc::new(ReadOnlyToTools(artifacts.results.clone())) as Arc<dyn Backend>,
            ),
            (
                "conversation_history/".into(),
                Arc::new(ReadOnlyToTools(artifacts.history.clone())) as Arc<dyn Backend>,
            ),
        ],
        Arc::new(HostBackend::new(ws.to_path_buf())),
    ))
}

/// A tenant boundary pin: the child prefixes its artifact keys with `sub{n}-`
/// (spec §5.7), so a parent read of a child-shaped path resolves against the
/// PARENT store and is NotFound — even when the parent's own store has entries.
/// No cross-tenant read is possible because the two tenants back different
/// `SessionArtifacts.results` handles, keyed disjointly.
#[tokio::test]
async fn parent_read_of_child_artifact_path_is_not_found_never_cross_tenant() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();

    // The parent tenant's store, seeded with its OWN artifact ("1-p").
    let artifacts = Arc::new(SessionArtifacts::new());
    artifacts
        .results
        .write("1-p", "the parent's own offloaded bytes")
        .await
        .unwrap();
    let backend = composite_over(&artifacts, &ws);

    let ctx = ToolCtx {
        workspace: ws.clone(),
        timeout: Duration::from_secs(30),
        cancel: CancellationToken::new(),
        sandbox: Arc::new(agent_tools::HostExecutor),
        backend: backend.clone(),
        call_id: "p1".into(),
    };

    // A child-shaped key ("sub1-1-c") is absent from the PARENT store, so the
    // read is NotFound even though the parent store is non-empty.
    let err = (agent_tools::fs::ReadFile {
        max_bytes: 16 * 1024,
    })
    .execute(
        serde_json::json!({"path": "large_tool_results/sub1-1-c"}),
        &ctx,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, ToolError::NotFound(_)),
        "a parent read of a child-shaped artifact path must be NotFound: {err:?}"
    );

    // Control: the parent CAN read its own key, proving the store is live and the
    // NotFound above is a genuine tenant miss, not a broken mount.
    let ok = (agent_tools::fs::ReadFile {
        max_bytes: 16 * 1024,
    })
    .execute(serde_json::json!({"path": "large_tool_results/1-p"}), &ctx)
    .await
    .unwrap();
    assert!(ok.content.contains("the parent's own offloaded bytes"));
}

/// Records forwarded child tool-result content by name (`sub:{name}`), which the
/// SubagentSink carries in `output.content`.
#[derive(Default)]
struct ChildResultSink {
    results: Mutex<Vec<(String, String)>>,
}
impl EventSink for ChildResultSink {
    fn emit(&self, event: AgentEvent) {
        if let AgentEvent::ToolResult { name, output, .. } = event {
            self.results.lock().unwrap().push((name, output.content));
        }
    }
}
impl ChildResultSink {
    fn content_for(&self, name: &str) -> Option<String> {
        self.results
            .lock()
            .unwrap()
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| c.clone())
    }
}

/// A tool that returns a fixed oversized body carrying a unique marker line —
/// the child's curation offloads it into the CHILD store on the ingestion cap.
struct BigMarker;
#[async_trait::async_trait]
impl Tool for BigMarker {
    fn name(&self) -> &str {
        "big_marker"
    }
    fn description(&self) -> &str {
        "returns a large body with a unique marker line"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "big_marker".into(),
            description: "big".into(),
            parameters: serde_json::json!({"type":"object"}),
        }
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "big_marker".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "big".into(),
        })
    }
    async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
        // > 16 KiB so the child's ingestion cap offloads it whole.
        let body = format!(
            "CHILD_MARKER_FACT=neon-42\n{}",
            "padding line to exceed the ingestion cap ".repeat(600)
        );
        Ok(ToolOutput {
            content: body,
            display: None,
        })
    }
}

/// Child self-isolation + workspace sharing (spec §5.7): a dispatched child
/// offloads its own oversized result into its OWN store and recovers it via grep
/// over `large_tool_results/`, and it can read a workspace file the parent wrote.
/// Both are asserted from the child's forwarded tool-result content.
#[tokio::test]
async fn child_reads_its_own_artifacts_and_shares_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    std::fs::write(ws.join("shared.txt"), "WORKSPACE_SHARED=yes").unwrap();

    let sink = Arc::new(ChildResultSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            // Turn 1: produce an oversized result → child curation offloads it.
            Scripted::Call("c1".into(), "big_marker".into(), "{}".into()),
            // Turn 2: recover it from the child's OWN store via grep (the child
            // can't predict its sub{n}- prefix, so it discovers by marker).
            Scripted::Call(
                "c2".into(),
                "grep".into(),
                r#"{"pattern":"CHILD_MARKER_FACT","path":"large_tool_results/"}"#.into(),
            ),
            // Turn 3: read the workspace file the parent wrote (shared workspace).
            Scripted::Call(
                "c3".into(),
                "read_file".into(),
                r#"{"path":"shared.txt"}"#.into(),
            ),
            Scripted::Text("recovered and read".into()),
        ]),
        sink.clone(),
        vec![
            Arc::new(BigMarker),
            Arc::new(agent_tools::fs::GrepTool),
            Arc::new(agent_tools::fs::ReadFile {
                max_bytes: 16 * 1024,
            }),
        ],
    );
    d.loop_config.workspace = ws.clone();
    let mut ctx = tool_ctx();
    ctx.workspace = ws.clone();
    ctx.backend = Arc::new(HostBackend::new(ws.clone()));
    let tool = DispatchAgentTool::new(d);
    tool.execute(serde_json::json!({"prompt": "p"}), &ctx)
        .await
        .unwrap();

    // The child grepped its OWN offloaded artifact and got the marker back —
    // proving it reads its own store (a fresh SessionArtifacts per child).
    let grep = sink
        .content_for("sub:grep")
        .expect("child grep result forwarded");
    assert!(
        grep.contains("CHILD_MARKER_FACT=neon-42"),
        "child must recover its own offloaded marker from large_tool_results/: {grep}"
    );
    assert!(
        grep.contains("large_tool_results/sub"),
        "the recovered hit must carry the child's sub-prefixed key: {grep}"
    );
    // And the child read the parent-written workspace file (shared workspace).
    let read = sink
        .content_for("sub:read_file")
        .expect("child read_file result forwarded");
    assert!(
        read.contains("WORKSPACE_SHARED=yes"),
        "child must read the shared workspace file: {read}"
    );
}

/// Task A2 / spec §5 ordering pin: the PARENT loop's own `ToolStart` for the
/// `dispatch_agent` call (emitted by the loop itself, before the tool runs)
/// must precede the typed `Subagent(Start)` (emitted inside `execute()`,
/// after dispatch validation) — and both carry the identical on-wire id, so a
/// frontend can join the typed stream to the already-rendered tool row.
#[tokio::test]
async fn subagent_start_id_matches_an_already_emitted_tool_start() {
    let sink = Arc::new(FullSink::default());
    let ws = workspace();
    let child = ScriptedModel::new(vec![Scripted::Text("child done".into())]);
    let d = deps(child, sink.clone(), vec![]);

    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(DispatchAgentTool::new(d)));

    let parent_model = ScriptedModel::new(vec![
        Scripted::Call(
            "call1".into(),
            "dispatch_agent".into(),
            r#"{"prompt":"go"}"#.into(),
        ),
        Scripted::Text("parent done".into()),
    ]);
    let config = LoopConfig {
        model_limit: 16384,
        max_turns: 3,
        max_retries: 1,
        tool_timeout: Duration::from_secs(5),
        stream_idle_timeout: Duration::from_secs(3600),
        workspace: ws.clone(),
        ..LoopConfig::default()
    };
    let agent = AgentLoop::new(
        Arc::new(parent_model),
        Arc::new(PassthroughProtocol),
        Arc::new(reg),
        Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec![],
            command_denylist: vec![],
        }),
        Arc::new(AlwaysApprove),
        sink.clone(),
        config,
    );
    let mut ctx = CuratedContext::new(
        Message::system("s"),
        Arc::new(SessionArtifacts::new()),
        Arc::new(AtomicBool::new(false)),
    );
    agent.run(&mut ctx, "go".into()).await.unwrap();

    let events = sink.events.lock().unwrap().clone();
    let tool_start_index = events
        .iter()
        .position(|(k, _, n, _)| k == "tool_start" && n == "dispatch_agent")
        .expect("parent loop must emit ToolStart for the dispatch_agent call");
    let start_index = events
        .iter()
        .position(|(k, _, _, _)| k == "subagent_start")
        .expect("execute() must emit Subagent(Start)");
    assert!(
        tool_start_index < start_index,
        "ToolStart must be observed before Subagent(Start): {events:?}"
    );
    assert_eq!(
        events[tool_start_index].1, events[start_index].1,
        "ToolStart and Subagent(Start) must carry the identical on-wire id: {events:?}"
    );
}

// ---------------------------------------------------------------------------
// Task 9: child checkpointers, wrap-at-dispatch attribution, resume rebinding.
// These use a real Checkpointer wired into DispatchDeps.checkpoint so a child
// parks on its Ask under children/<call_id> and its origin is stamped.
// ---------------------------------------------------------------------------
use agent_core::checkpoint::{write_answer, write_checkpoint};
use agent_core::{
    Checkpoint, Checkpointer, CuratedContextState, GateRecord, Guardrails, ParkedTurn,
    PendingSnapshot, CHECKPOINT_VERSION,
};

const CKKEY: [u8; 32] = [9u8; 32];

/// Records each request's `origin`; while blocked it snapshots whether the
/// child + parent park dirs exist, then replies with a fixed response after an
/// optional delay (P2 delayed-approve).
struct CapturingApproval {
    origins: Mutex<Vec<Option<agent_policy::ApprovalOrigin>>>,
    /// (child_park_present, parent_park_present) observed AT ask time.
    park_seen: Mutex<Vec<(bool, bool)>>,
    child_dir: std::path::PathBuf,
    parent_dir: std::path::PathBuf,
    reply: agent_policy::ApprovalResponse,
    delay: Option<Duration>,
}
#[async_trait::async_trait]
impl agent_policy::ApprovalChannel for CapturingApproval {
    async fn request(&self, req: agent_policy::ApprovalRequest) -> agent_policy::ApprovalResponse {
        self.origins.lock().unwrap().push(req.origin.clone());
        self.park_seen.lock().unwrap().push((
            agent_core::checkpoint::has_park(&self.child_dir),
            agent_core::checkpoint::has_park(&self.parent_dir),
        ));
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        self.reply.clone()
    }
}

/// A minimal parked-turn Checkpoint over a single gate-kind Ask on `writey`.
/// The last history entry is the parked assistant turn carrying the batch, so
/// the resume replay produces a well-formed (non-orphaned) context.
fn parked_writey_checkpoint(session_id: &str, subpath: Vec<String>) -> Checkpoint {
    let batch = vec![agent_tools::ToolCall {
        id: "c1".into(),
        name: "writey".into(),
        args: serde_json::json!({}),
    }];
    Checkpoint {
        version: CHECKPOINT_VERSION,
        session_id: session_id.into(),
        subagent_path: subpath,
        turn: 0,
        context: CuratedContextState {
            goal: Some(Message::user("go")),
            history: vec![
                Message::user("go"),
                Message::assistant("calling writey", Some(batch.clone())),
            ],
            compaction_summary: None,
            folded_facts: vec![],
            folded_sections: vec![],
            seq: 0,
            history_has_spans: false,
            history_incomplete: false,
            artifact_prefix: "sub1-".into(),
            todos: vec![],
        },
        guardrails: Guardrails {
            tool_calls: 0,
            model_calls: 1,
        },
        parked: ParkedTurn {
            assistant_text: "calling writey".into(),
            tool_calls: batch,
            invalid: vec![],
            gate_records: vec![],
            parked_index: Some(0),
            origin: None,
        },
    }
}

#[tokio::test]
async fn child_ask_carries_origin_and_parks_under_children_dir() {
    let dir = tempfile::tempdir().unwrap();
    let ckdir = dir.path().join("checkpoint");
    let root = Checkpointer::new(ckdir.clone(), CKKEY, "s1".into());
    // Mimic the parent loop's dispatch-bearing turn: a pending dispatch-kind
    // snapshot that must flush to disk when the child parks (ancestor flush).
    root.set_turn_snapshot(PendingSnapshot {
        context: parked_writey_checkpoint("s1", vec![]).context,
        guardrails: Guardrails::default(),
        turn: 0,
        assistant_text: "dispatching".into(),
        tool_calls: vec![],
        invalid: vec![],
        gate_records: vec![GateRecord::Ready],
        artifacts: Arc::new(SessionArtifacts::new()),
    });
    let child_dir = ckdir.join("children").join("d1");
    let approval = Arc::new(CapturingApproval {
        origins: Mutex::new(vec![]),
        park_seen: Mutex::new(vec![]),
        child_dir: child_dir.clone(),
        parent_dir: ckdir.clone(),
        reply: agent_policy::ApprovalResponse::Deny { feedback: None },
        delay: None,
    });
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "writey".into(), "{}".into()),
            Scripted::Text("done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Writey)],
    );
    d.approval = approval.clone();
    d.checkpoint = Some(root.clone());
    let tool = DispatchAgentTool::new(d);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();

    // Attribution: the child's Ask carries the dispatch origin.
    assert_eq!(
        approval.origins.lock().unwrap().as_slice(),
        &[Some(agent_policy::ApprovalOrigin {
            delegation_id: "d1".into(),
            subagent_name: "general-purpose".into(),
            depth: 1,
        })]
    );
    // At ask time BOTH the child park and the ancestor dispatch-kind park were
    // on disk.
    assert_eq!(
        approval.park_seen.lock().unwrap().as_slice(),
        &[(true, true)],
        "child + parent park present at ask time"
    );
    // Deny lets the run finish.
    assert!(out.content.starts_with("done"));
    // Completed child → its whole checkpoint tree reaped.
    assert!(!child_dir.exists(), "child dir cleared after Completed");
    // The parent's dispatch-kind park is removed when its turn ends.
    root.end_turn();
    assert!(
        !agent_core::checkpoint::has_park(&ckdir),
        "parent park gone"
    );
}

#[tokio::test]
async fn parent_ask_has_no_origin() {
    // A top-level loop (no dispatch) issues asks with NO origin (spec §2.6):
    // only the wrap-at-dispatch decorator stamps one.
    let ws = workspace();
    let approval = Arc::new(CapturingApproval {
        origins: Mutex::new(vec![]),
        park_seen: Mutex::new(vec![]),
        child_dir: ws.clone(),
        parent_dir: ws.clone(),
        reply: agent_policy::ApprovalResponse::Deny { feedback: None },
        delay: None,
    });
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(Writey));
    let config = LoopConfig {
        model_limit: 16384,
        max_turns: 3,
        max_retries: 1,
        tool_timeout: Duration::from_secs(5),
        stream_idle_timeout: Duration::from_secs(3600),
        workspace: ws.clone(),
        ..LoopConfig::default()
    };
    let agent = AgentLoop::new(
        Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "writey".into(), "{}".into()),
            Scripted::Text("done".into()),
        ])),
        Arc::new(PassthroughProtocol),
        Arc::new(reg),
        Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec![],
            command_denylist: vec![],
        }),
        approval.clone(),
        Arc::new(FullSink::default()),
        config,
    );
    let mut ctx = CuratedContext::new(
        Message::system("s"),
        Arc::new(SessionArtifacts::new()),
        Arc::new(AtomicBool::new(false)),
    );
    agent.run(&mut ctx, "go".into()).await.unwrap();
    assert_eq!(
        approval.origins.lock().unwrap().as_slice(),
        &[None],
        "top-level asks carry no origin"
    );
}

#[tokio::test]
async fn dispatch_rebinds_a_parked_child_and_resumes_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let ckdir = dir.path().join("checkpoint");
    let root = Checkpointer::new(ckdir.clone(), CKKEY, "s1".into());
    // Pre-existing parked child under children/d1 (the call id the rig uses),
    // with an approve=true answer committed against it.
    let child_dir = ckdir.join("children").join("d1");
    write_checkpoint(
        &child_dir,
        &CKKEY,
        &parked_writey_checkpoint("s1", vec!["d1".into()]),
        &Default::default(),
    )
    .unwrap();
    write_answer(&child_dir, &CKKEY, true, None).unwrap();

    // A recording approval proves NO prompt fires on resume.
    let approval = Arc::new(RecordingApproval {
        seen: Mutex::new(vec![]),
        reply: agent_policy::ApprovalResponse::Deny { feedback: None },
    });
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        // The resumed child consumes NO completion for the parked turn: it
        // executes the batch, then serves the post-batch text turn.
        ScriptedModel::new(vec![Scripted::Text("resumed done".into())]),
        sink.clone(),
        vec![Arc::new(Writey)],
    );
    d.approval = approval.clone();
    d.checkpoint = Some(root.clone());
    let tool = DispatchAgentTool::new(d);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .unwrap();

    // The parked writey call executed (approve consumed from answer.json).
    let events = sink.events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .any(|(k, _, n, _)| k == "tool_result:ok" && n == "sub:writey"),
        "parked writey must execute: {events:?}"
    );
    assert!(
        approval.seen.lock().unwrap().is_empty(),
        "no approval prompt on resume"
    );
    assert!(out.content.starts_with("resumed done"));
    // Completed → child checkpoint dir reaped.
    assert!(
        !child_dir.exists(),
        "child dir deleted after resume completes"
    );
}

#[tokio::test]
async fn dispatch_deadline_disarms_while_child_ask_is_parked() {
    // P2 pin: with a checkpointer wired, a tiny subagent_timeout does NOT kill
    // a child parked at a durable Ask; the late approve lets it finish.
    let dir = tempfile::tempdir().unwrap();
    let ckdir = dir.path().join("checkpoint");
    let root = Checkpointer::new(ckdir.clone(), CKKEY, "s1".into());
    let child_dir = ckdir.join("children").join("d1");
    let approval = Arc::new(CapturingApproval {
        origins: Mutex::new(vec![]),
        park_seen: Mutex::new(vec![]),
        child_dir: child_dir.clone(),
        parent_dir: ckdir.clone(),
        reply: agent_policy::ApprovalResponse::Approve,
        // Resolves at 5x the deadline.
        delay: Some(Duration::from_millis(250)),
    });
    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "writey".into(), "{}".into()),
            Scripted::Text("late done".into()),
        ]),
        sink.clone(),
        vec![Arc::new(Writey)],
    );
    d.approval = approval.clone();
    d.checkpoint = Some(root.clone());
    d.subagent_timeout = Duration::from_millis(50);
    let tool = DispatchAgentTool::new(d);
    // Use a ctx whose timeout is the tiny deadline (dispatch reads ctx.timeout).
    let mut ctx = tool_ctx();
    ctx.timeout = Duration::from_millis(50);
    let out = tool
        .execute(serde_json::json!({"prompt": "p"}), &ctx)
        .await
        .unwrap();
    assert!(
        out.content.starts_with("late done"),
        "durable ask disarms the deadline; child finished: {}",
        out.content
    );

    // Control: WITHOUT a checkpointer, the same tiny deadline still fires
    // (live-only ask keeps today's hard deadline).
    let approval2 = Arc::new(CapturingApproval {
        origins: Mutex::new(vec![]),
        park_seen: Mutex::new(vec![]),
        child_dir: child_dir.clone(),
        parent_dir: ckdir.clone(),
        reply: agent_policy::ApprovalResponse::Approve,
        delay: Some(Duration::from_millis(250)),
    });
    let sink2 = Arc::new(FullSink::default());
    let mut d2 = deps(
        ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "writey".into(), "{}".into()),
            Scripted::Text("late done".into()),
        ]),
        sink2.clone(),
        vec![Arc::new(Writey)],
    );
    d2.approval = approval2;
    // d2.checkpoint stays None.
    d2.subagent_timeout = Duration::from_millis(50);
    let tool2 = DispatchAgentTool::new(d2);
    let mut ctx2 = tool_ctx();
    ctx2.timeout = Duration::from_millis(50);
    let out2 = tool2
        .execute(serde_json::json!({"prompt": "p"}), &ctx2)
        .await
        .unwrap();
    assert!(
        out2.content.contains("timed out"),
        "live-only ask must still time out: {}",
        out2.content
    );
}

#[tokio::test]
async fn corrupt_child_checkpoint_refuses_honestly() {
    let dir = tempfile::tempdir().unwrap();
    let ckdir = dir.path().join("checkpoint");
    let root = Checkpointer::new(ckdir.clone(), CKKEY, "s1".into());
    let child_dir = ckdir.join("children").join("d1");
    write_checkpoint(
        &child_dir,
        &CKKEY,
        &parked_writey_checkpoint("s1", vec!["d1".into()]),
        &Default::default(),
    )
    .unwrap();
    // Tamper parked.json (the see-benign/run-hostile forgery) so the manifest
    // hash no longer matches.
    let p = child_dir.join("parked.json");
    let body = std::fs::read_to_string(&p)
        .unwrap()
        .replace("calling writey", "calling something else entirely");
    std::fs::write(&p, body).unwrap();

    let sink = Arc::new(FullSink::default());
    let mut d = deps(
        ScriptedModel::new(vec![Scripted::Text("should not run".into())]),
        sink.clone(),
        vec![Arc::new(Writey)],
    );
    d.checkpoint = Some(root.clone());
    let tool = DispatchAgentTool::new(d);
    let err = tool
        .execute(serde_json::json!({"prompt": "p"}), &tool_ctx())
        .await
        .expect_err("corrupt checkpoint must refuse, never start fresh");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("checkpoint"),
        "failure must mention the checkpoint: {msg}"
    );
    // The tampered dir is retained (offline inspection); no Subagent Start
    // fired (refused pre-Start).
    assert!(child_dir.exists(), "tampered dir retained");
    let events = sink.events.lock().unwrap().clone();
    assert!(
        !events.iter().any(|(k, _, _, _)| k == "subagent_start"),
        "no Subagent Start on a refused resume: {events:?}"
    );
}
