//! Live end-to-end test. Requires a running OpenAI-compatible server.
//! Run with: AGENT_E2E_URL=http://localhost:30000 AGENT_E2E_MODEL=<name> \
//!           cargo test -p agent-core --test e2e_sglang -- --ignored --nocapture

use agent_core::{AgentLoop, EventSink, AgentEvent, LoopConfig, WindowContext};
use agent_model::{Message, NativeProtocol, OpenAiCompatClient};
use agent_policy::RulePolicy;
use agent_tools::{fs::ReadFile, ToolRegistry};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Capture(Mutex<Vec<String>>);
impl EventSink for Capture {
    fn emit(&self, e: AgentEvent) {
        if let AgentEvent::ToolResult { name, .. } = e {
            self.0.lock().unwrap().push(name);
        }
    }
}

#[tokio::test]
#[ignore = "requires AGENT_E2E_URL / AGENT_E2E_MODEL and a live server"]
async fn reads_a_file_against_real_server() {
    let url = std::env::var("AGENT_E2E_URL").expect("set AGENT_E2E_URL");
    let model_name = std::env::var("AGENT_E2E_MODEL").expect("set AGENT_E2E_MODEL");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("secret.txt"), "the password is swordfish").unwrap();
    let ws = dir.path().to_path_buf();

    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(ReadFile));
    let sink = Arc::new(Capture(Mutex::new(vec![])));
    let agent = AgentLoop::new(
        Arc::new(OpenAiCompatClient::new(url, model_name, std::env::var("AGENT_API_KEY").ok())),
        Arc::new(NativeProtocol), Arc::new(reg),
        Arc::new(RulePolicy { workspace: ws.clone(), command_allowlist: vec![],
            command_denylist: vec![] }),
        Arc::new(AutoApprove), sink.clone(),
        LoopConfig { model_limit: 8192, max_turns: 8, max_retries: 2, temperature: 0.0,
            max_tokens: Some(512), workspace: ws, tool_timeout: Duration::from_secs(60),
            stream_idle_timeout: Duration::from_secs(120), ..Default::default() });

    let mut ctx = WindowContext::new(Message::system(
        "You are a coding agent. Use read_file to answer questions about files."));
    agent.run(&mut ctx, "Read secret.txt and tell me the password.".into()).await.unwrap();

    let tools_used = sink.0.lock().unwrap().clone();
    assert!(tools_used.iter().any(|n| n == "read_file"),
        "model should have called read_file; got {tools_used:?}");
}

struct AutoApprove;
#[async_trait::async_trait]
impl agent_policy::ApprovalChannel for AutoApprove {
    async fn request(&self, _r: agent_policy::ApprovalRequest) -> agent_policy::ApprovalResponse {
        agent_policy::ApprovalResponse::Approve
    }
}
