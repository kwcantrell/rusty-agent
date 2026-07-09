//! Pins spec G4: a routed compaction model serves maintain()/overflow
//! compaction; the session model is untouched by the summary call.
//!
//! Harness note: `maintain()` runs after a TOOL turn (a tool-less final answer
//! ends the run at `Done` before the post-tools `maintain()`), so the session
//! model is scripted with one `Echo` tool call followed by the final answer.
//! That drives exactly one flag-forced `maintain()` whose compaction call must
//! route to the dedicated compaction model — leaving the session model's turns
//! for the user-facing completions only.
use agent_core::testkit::{
    AlwaysApprove, CollectingSink, PassthroughProtocol, Scripted, ScriptedModel,
};
use agent_core::{
    AgentLoop, ContextCurationMiddleware, ContextManager, CuratedContext, LoopConfig,
    SessionArtifacts,
};
use agent_model::Message;
use agent_policy::RulePolicy;
use agent_tools::{
    Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolRegistry, ToolSchema,
};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

/// A trivial tool so the session model can take a tool turn (whose post-tools
/// `maintain()` is where the flag-forced compaction fires).
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

#[tokio::test]
async fn routed_compaction_model_serves_the_summary_call() {
    let dir = tempfile::tempdir().unwrap();
    // Session model: one tool call + one plain final answer. Compaction model:
    // one summary. The tool call gives the loop a post-tools `maintain()` where
    // the flag-forced compaction runs; the final answer ends the run.
    let session = Arc::new(ScriptedModel::new(vec![
        Scripted::Call("c1".into(), "echo".into(), "{}".into()),
        Scripted::Text("done".into()),
    ]));
    let compactor = Arc::new(ScriptedModel::new(vec![Scripted::Text("SUMMARY".into())]));
    let sink = Arc::new(CollectingSink::default());

    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(Echo));

    let artifacts = Arc::new(SessionArtifacts::new());
    let flag = Arc::new(AtomicBool::new(false));

    let agent = AgentLoop::new(
        session.clone(),
        Arc::new(PassthroughProtocol),
        Arc::new(tools),
        Arc::new(RulePolicy {
            workspace: dir.path().to_path_buf(),
            command_allowlist: vec![],
            command_denylist: vec![],
        }),
        Arc::new(AlwaysApprove),
        sink.clone(),
        LoopConfig {
            model_limit: 16384,
            max_turns: 3,
            max_retries: 1,
            tool_timeout: Duration::from_secs(5),
            stream_idle_timeout: Duration::from_secs(5),
            workspace: dir.path().to_path_buf(),
            ..LoopConfig::default()
        },
    )
    .with_compaction_model(compactor.clone())
    .with_middleware(vec![Arc::new(ContextCurationMiddleware::new(flag.clone()))]);

    let mut ctx = CuratedContext::new(Message::system("s"), artifacts, flag);
    // Seed enough closed history that a forced compaction has a span to replace,
    // then request compaction so the post-turn maintain() runs the compactor.
    for i in 0..6 {
        ctx.append(Message::user(format!("old question {i}")));
        ctx.append(Message::assistant(format!("old answer {i}"), None));
    }
    ctx.request_compaction();

    agent.run(&mut ctx, "final question".into()).await.unwrap();

    // The compaction model was consumed; the summary call did NOT come from the
    // session model (its scripted turns answered the tool call + user question).
    assert_eq!(compactor.remaining(), 0, "routed compaction model unused");
    assert_eq!(session.remaining(), 0);
    let events = sink.events.lock().unwrap().clone();
    assert!(
        events.iter().any(|e| e.starts_with("compacted:")),
        "{events:?}"
    );
}
