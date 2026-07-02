//! Integration tests for DispatchAgentTool (spec D1-D13 core behaviors).
use agent_core::testkit::{AlwaysApprove, PassthroughProtocol, Scripted, ScriptedModel};
use agent_core::{AgentEvent, DispatchAgentTool, DispatchDeps, EventSink, LoopConfig, SUBAGENT_PREAMBLE};
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
            AgentEvent::ToolResult { id, name, status, .. } => {
                (format!("tool_result:{}", status.as_str()), id, name)
            }
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
    fn name(&self) -> &str { "echo" }
    fn description(&self) -> &str { "echo" }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: "echo".into(), description: "echo".into(), parameters: serde_json::json!({"type":"object"}) }
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent { tool: "echo".into(), access: Access::Read, paths: vec![], command: None, summary: "echo".into() })
    }
    async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput { content: "echoed".into(), display: None })
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
        policy: Arc::new(RulePolicy { workspace: ws.clone(), command_allowlist: vec![], command_denylist: vec![] }),
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
    assert!(out.content.starts_with("hello from child"), "{}", out.content);
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
    assert!(events.iter().any(|(k, i, n)| k == "tool_start" && i.contains(":c1") && i.starts_with("sub") && n == "sub:echo"), "{events:?}");
    assert!(events.iter().any(|(k, _, n)| k == "tool_result:ok" && n == "sub:echo"), "{events:?}");
    assert!(!events.iter().any(|(k, _, _)| k == "token" || k == "done"), "{events:?}");
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
    let out = tool.execute(serde_json::json!({"prompt": "p"}), &tool_ctx()).await.unwrap();
    assert!(out.content.contains("turn budget"), "{}", out.content);
    assert!(out.content.contains("stop: BudgetExhausted"), "{}", out.content);
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
        .execute(serde_json::json!({"prompt": "p", "tools": ["nope"]}), &tool_ctx())
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(ref m) if m.contains("nope") && m.contains("echo")), "{err:?}");

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
    assert!(events.iter().any(|(k, _, n)| k == "tool_result:denied" && n == "sub:echo"), "{events:?}");
}

#[tokio::test]
async fn missing_prompt_is_invalid_args() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let err = tool.execute(serde_json::json!({}), &tool_ctx()).await.unwrap_err();
    assert!(matches!(err, ToolError::InvalidArgs(_)));
}

#[test]
fn intent_is_readonly_and_auto_allowed() {
    let tool = DispatchAgentTool::new(deps(
        ScriptedModel::new(vec![]),
        Arc::new(FullSink::default()),
        vec![],
    ));
    let intent = tool.intent(&serde_json::json!({"prompt": "summarize the repo"})).unwrap();
    assert!(matches!(intent.access, Access::Read));
    assert!(intent.paths.is_empty());
    assert!(intent.command.is_none());
    assert!(intent.summary.contains("summarize"));
    let policy = RulePolicy { workspace: workspace(), command_allowlist: vec![], command_denylist: vec![] };
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
