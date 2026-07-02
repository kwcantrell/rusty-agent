//! Pins spec D7: gate_tool builds ToolCtx with the tool's timeout_override
//! when present, else the loop's tool_timeout.
use agent_core::testkit::{
    AlwaysApprove, CollectingSink, PassthroughProtocol, Scripted, ScriptedModel,
};
use agent_core::{AgentLoop, CuratedContext, InMemoryOffloadStore, LoopConfig};
use agent_model::Message;
use agent_policy::RulePolicy;
use agent_tools::{
    Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolRegistry, ToolSchema,
};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Records the ToolCtx.timeout it was executed with.
struct TimeoutProbe {
    name: &'static str,
    override_secs: Option<u64>,
    seen: Mutex<Option<Duration>>,
}
#[async_trait::async_trait]
impl Tool for TimeoutProbe {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "probe"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name.into(),
            description: "probe".into(),
            parameters: serde_json::json!({"type":"object"}),
        }
    }
    fn timeout_override(&self) -> Option<Duration> {
        self.override_secs.map(Duration::from_secs)
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: self.name.into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "probe".into(),
        })
    }
    async fn execute(&self, _a: serde_json::Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        *self.seen.lock().unwrap() = Some(ctx.timeout);
        Ok(ToolOutput {
            content: "ok".into(),
            display: None,
        })
    }
}

fn run_probe(probe: Arc<TimeoutProbe>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let mut reg = ToolRegistry::new();
        reg.register(probe.clone());
        let dir = tempfile::tempdir().unwrap();
        let config = LoopConfig {
            model_limit: 16384,
            max_turns: 3,
            max_retries: 1,
            tool_timeout: Duration::from_secs(5),
            stream_idle_timeout: Duration::from_secs(5),
            workspace: dir.path().to_path_buf(),
            ..LoopConfig::default()
        };
        let agent = AgentLoop::new(
            Arc::new(ScriptedModel::new(vec![
                Scripted::Call("c1".into(), probe.name.into(), "{}".into()),
                Scripted::Text("done".into()),
            ])),
            Arc::new(PassthroughProtocol),
            Arc::new(reg),
            Arc::new(RulePolicy {
                workspace: dir.path().to_path_buf(),
                command_allowlist: vec![],
                command_denylist: vec![],
            }),
            Arc::new(AlwaysApprove),
            Arc::new(CollectingSink::default()),
            config,
        );
        let mut ctx = CuratedContext::new(
            Message::system("s"),
            Arc::new(InMemoryOffloadStore::new()),
            Arc::new(AtomicBool::new(false)),
        );
        agent.run(&mut ctx, "go".into()).await.unwrap();
    });
}

#[test]
fn tool_ctx_uses_timeout_override_when_present() {
    let probe = Arc::new(TimeoutProbe {
        name: "probe_a",
        override_secs: Some(555),
        seen: Mutex::new(None),
    });
    run_probe(probe.clone());
    assert_eq!(*probe.seen.lock().unwrap(), Some(Duration::from_secs(555)));
}

#[test]
fn tool_ctx_defaults_to_loop_tool_timeout() {
    let probe = Arc::new(TimeoutProbe {
        name: "probe_b",
        override_secs: None,
        seen: Mutex::new(None),
    });
    run_probe(probe.clone());
    assert_eq!(*probe.seen.lock().unwrap(), Some(Duration::from_secs(5)));
}
