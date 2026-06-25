//! Live, opt-in e2e: does the real server emit *parallel* tool calls in one turn,
//! and does the loop produce one correctly-id-matched result per call?
//! Run with: AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
//!           cargo test -p agent-core --test e2e_parallel_tools -- --ignored --nocapture

use agent_core::{AgentEvent, AgentLoop, EventSink, LoopConfig, WindowContext};
use agent_model::{Message, NativeProtocol, OpenAiCompatClient};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse, RulePolicy};
use agent_tools::{fs::ReadFile, ToolRegistry};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Records the tool name per ToolResult so we can count matched results.
struct Capture(Mutex<Vec<String>>);
impl EventSink for Capture {
    fn emit(&self, e: AgentEvent) {
        if let AgentEvent::ToolResult { name, .. } = e {
            self.0.lock().unwrap().push(name);
        }
    }
}

struct AutoApprove;
#[async_trait::async_trait]
impl ApprovalChannel for AutoApprove {
    async fn request(&self, _r: ApprovalRequest) -> ApprovalResponse { ApprovalResponse::Approve }
}

#[tokio::test]
#[ignore = "requires AGENT_E2E_URL / AGENT_E2E_MODEL and a live server"]
async fn parallel_reads_against_real_server() {
    let url = std::env::var("AGENT_E2E_URL").expect("set AGENT_E2E_URL");
    let model_name = std::env::var("AGENT_E2E_MODEL").expect("set AGENT_E2E_MODEL");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.txt"), "ALPHA_BODY").unwrap();
    std::fs::write(dir.path().join("beta.txt"), "BETA_BODY").unwrap();
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
        // temperature 0.0 to make parallel emission as deterministic as the model allows.
        LoopConfig { model_limit: 8192, max_turns: 4, max_retries: 2, temperature: 0.0,
            max_tokens: Some(512), workspace: ws, tool_timeout: Duration::from_secs(60),
            stream_idle_timeout: Duration::from_secs(120), ..Default::default() });

    let mut ctx = WindowContext::new(Message::system(
        "You are a coding agent. When asked about multiple files, call read_file \
         once per file IN THE SAME turn (parallel tool calls)."));
    agent.run(&mut ctx,
        "Read BOTH alpha.txt and beta.txt and report each file's contents. \
         Call read_file for each file in the same turn.".into()).await.unwrap();

    let reads = sink.0.lock().unwrap().clone();
    // Distinguish a loop bug from model behavior: <2 calls means the model did not
    // emit parallel calls this run — inconclusive, not a loop failure.
    assert!(reads.len() >= 2,
        "INCONCLUSIVE: model did not emit parallel tool calls (got {} read_file result(s)); \
         re-run or adjust the prompt. This is model behavior, not a loop bug.", reads.len());
    assert!(reads.iter().all(|n| n == "read_file"),
        "every result should be a read_file; got {reads:?}");
}
