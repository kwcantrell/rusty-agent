//! Integration tests for DispatchAgentTool (spec D1-D13 core behaviors).
use agent_core::testkit::{AlwaysApprove, PassthroughProtocol, Scripted, ScriptedModel};
use agent_core::{
    AgentEvent, DispatchAgentTool, DispatchDeps, EventSink, LoopConfig, SUBAGENT_PREAMBLE,
};
use agent_policy::{Decision, PolicyEngine, RulePolicy};
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Full-fidelity parent sink (testkit CollectingSink drops ids).
#[derive(Default)]
struct FullSink {
    events: Mutex<Vec<(String, String, String)>>, // (kind, id, name)
}
impl EventSink for FullSink {
    fn emit(&self, event: AgentEvent) {
        let t = match event {
            AgentEvent::ToolStart { id, name, .. } => ("tool_start".to_string(), id, name),
            AgentEvent::ToolResult {
                id, name, status, ..
            } => (format!("tool_result:{}", status.as_str()), id, name),
            AgentEvent::Token(t) => ("token".to_string(), String::new(), t),
            AgentEvent::Done(_) => ("done".to_string(), String::new(), String::new()),
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
        base_tools: base,
        child_system_prompt: format!("SYS\n\n{SUBAGENT_PREAMBLE}"),
        loop_config: child_config(ws),
        max_result_bytes: 16 * 1024,
        subagent_timeout: Duration::from_secs(600),
    }
}

fn tool_ctx() -> ToolCtx {
    ToolCtx {
        workspace: workspace(),
        timeout: Duration::from_secs(600),
        cancel: CancellationToken::new(),
        sandbox: Arc::new(agent_tools::HostExecutor),
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
        events.iter().any(|(k, i, n)| k == "tool_start"
            && i.contains(":c1")
            && i.starts_with("sub")
            && n == "sub:echo"),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|(k, _, n)| k == "tool_result:ok" && n == "sub:echo"),
        "{events:?}"
    );
    assert!(
        !events.iter().any(|(k, _, _)| k == "token" || k == "done"),
        "{events:?}"
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
            .any(|(k, _, n)| k == "tool_result:denied" && n == "sub:echo"),
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
        self.reply
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
        reply: agent_policy::ApprovalResponse::Deny,
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
            .any(|(k, _, n)| k == "tool_result:denied" && n == "sub:writey"),
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
            .any(|(k, _, n)| k == "tool_result:ok" && n == "sub:writey"),
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
            .any(|(k, _, n)| k == "tool_result:denied" && n == "sub:dispatch_agent"),
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
            .any(|(k, _, n)| k == "tool_result:denied" && n == "sub:dispatch_agent"),
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
    let err = tool
        .execute(serde_json::json!({"prompt": "p"}), &ctx)
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::Timeout), "{err:?}");
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
        .filter(|(k, _, _)| k == "tool_start")
        .map(|(_, id, _)| id.clone())
        .collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0], ids[1], "{ids:?}");
}
