//! Live, opt-in e2e for the shared loop assembly. Drives
//! `agent_runtime_config::assemble_loop` (the one builder both front-ends use)
//! against a real model, covering what unit tests and the raw-loop e2e cannot:
//! the consolidated builder producing a working tool-calling loop.
//! Run with: AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
//!           cargo test -p agent-runtime-config --test e2e_auto_retrieval -- --ignored --nocapture
//!
//! (The vector-memory auto-retrieval tests that used to live here were deleted in
//! 4A-1 B2 along with the `agent-memory` crate — their premise no longer exists.
//! The B4 task adds the replacement cross-run soak for file-based memory.)

use agent_core::{AgentEvent, EventSink, WindowContext};
use agent_model::{Message, OpenAiCompatClient};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use agent_runtime_config::{assemble_loop, LoopParts, RuntimeConfig};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Captures streamed assistant text and the names of tools that produced results.
#[derive(Default)]
struct Capture {
    text: Mutex<String>,
    tools: Mutex<Vec<String>>,
}
impl EventSink for Capture {
    fn emit(&self, e: AgentEvent) {
        match e {
            AgentEvent::Token(t) => self.text.lock().unwrap().push_str(&t),
            AgentEvent::ToolResult { name, .. } => self.tools.lock().unwrap().push(name),
            _ => {}
        }
    }
}

struct AutoApprove;
#[async_trait::async_trait]
impl ApprovalChannel for AutoApprove {
    async fn request(&self, _r: ApprovalRequest) -> ApprovalResponse {
        ApprovalResponse::Approve
    }
}

fn env() -> (String, String) {
    (
        std::env::var("AGENT_E2E_URL").expect("set AGENT_E2E_URL"),
        std::env::var("AGENT_E2E_MODEL").expect("set AGENT_E2E_MODEL"),
    )
}

/// The consolidated builder produces a working *tool-calling* loop: with no memory,
/// `assemble_loop` wires the registry/policy/sandbox/protocol so the real model can
/// call `read_file` and report a file's contents. Covers the assemble_loop path that
/// the raw-loop e2e (e2e_sglang) does not exercise.
#[tokio::test]
#[ignore = "requires AGENT_E2E_URL / AGENT_E2E_MODEL and a live server"]
async fn assemble_loop_drives_a_real_tool_call() {
    let (url, model_name) = env();
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().to_path_buf();
    std::fs::write(workspace.join("a.txt"), "HELLO-FROM-ASSEMBLE").unwrap();

    let mut cfg = RuntimeConfig::from_launch(
        "openai".into(),
        url.clone(),
        model_name.clone(),
        "native".into(),
        262_144,
    );
    cfg.memory = false;
    cfg.sandbox_mode = "off".into(); // file read goes straight through the tool, no container
    let sink = Arc::new(Capture::default());
    let built = assemble_loop(
        &cfg,
        LoopParts {
            model: Arc::new(OpenAiCompatClient::new(url, model_name, None)),
            sink: sink.clone(),
            approval: Arc::new(AutoApprove),
            workspace: workspace.clone(),
            mcp_tools: vec![],
            stream_idle_timeout: Duration::from_secs(120),
            base_system_prompt:
                "You are a coding agent. Use the provided tools to inspect the workspace.".into(),
            artifacts: Arc::new(agent_core::SessionArtifacts::new()),
            compact_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            todos: Arc::new(std::sync::Mutex::new(Vec::new())),
            sandbox: agent_runtime_config::build_sandbox(&cfg),
            stats: Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default())),
            trace: None,
            api_key: None,
            claude_binary: "claude".into(),
            checkpoint: None,
        },
    );
    let mut ctx = WindowContext::new(Message::system(built.system_prompt.clone()));
    built
        .loop_
        .run(
            &mut ctx,
            "Read the file a.txt and tell me its exact contents.".into(),
        )
        .await
        .unwrap();

    let answer = sink.text.lock().unwrap().clone();
    let tools = sink.tools.lock().unwrap().clone();
    eprintln!("[tool path] tools={tools:?} answer={answer}");
    // The only way to surface the marker is to actually read the file via the tool.
    assert!(answer.contains("HELLO-FROM-ASSEMBLE"),
        "assemble_loop tool path: answer should contain the file contents; tools={tools:?} answer={answer:?}");
    assert!(
        tools.iter().any(|t| t == "read_file"),
        "assemble_loop should have registered + driven read_file; got tools={tools:?}"
    );
}
