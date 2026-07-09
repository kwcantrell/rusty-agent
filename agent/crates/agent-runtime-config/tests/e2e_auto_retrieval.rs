//! Live, opt-in e2e for the shared loop assembly + memory auto-retrieval. These
//! drive `agent_runtime_config::assemble_loop` (the one builder both front-ends use)
//! against a real model, so they cover what unit tests and the raw-loop e2e cannot:
//! the consolidated builder producing a working tool-calling loop, and the live
//! memory gate (on injects recall, off suppresses it).
//! Run with: AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
//!           cargo test -p agent-runtime-config --test e2e_auto_retrieval -- --ignored --nocapture
//! (agent-memory is pulled with its default `onnx` feature, so the real embedder is used.)

use agent_core::{AgentEvent, EventSink, WindowContext};
use agent_memory::{
    assemble_memory, now_secs, open_memory_parts, project_scope, MemoryConfig, MemoryRecord,
};
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

/// Build a `MemoryParts` over an isolated temp DB (never touches ~/.agent/memory.db),
/// seed one distinctive fact scoped to `workspace`, and return the assembled retriever + tools.
async fn seed_memory(
    workspace: &std::path::Path,
    db_dir: &std::path::Path,
    fact: &str,
) -> (
    Vec<Arc<dyn agent_tools::Tool>>,
    Arc<dyn agent_core::Retriever>,
) {
    let cfg = MemoryConfig {
        db_path: db_dir.join("memory.db"),
        ..MemoryConfig::default()
    };
    let parts = open_memory_parts(cfg).expect("load embedding model + open store");
    let vector = parts
        .embedder
        .embed(&[fact.to_string()])
        .await
        .unwrap()
        .remove(0);
    parts
        .store
        .upsert(MemoryRecord {
            id: "seed-1".into(),
            text: fact.into(),
            scope: project_scope(workspace),
            tags: vec![],
            vector,
            created_at: now_secs(),
            updated_at: now_secs(),
            source: "e2e".into(),
        })
        .await
        .unwrap();
    assemble_memory(&parts, workspace)
}

/// Positive: with memory ON, the seeded fact is auto-retrieved and the model uses it.
#[tokio::test]
#[ignore = "requires AGENT_E2E_URL / AGENT_E2E_MODEL, a live server, and the embedding model"]
async fn auto_retrieval_feeds_a_seeded_fact_to_the_real_model() {
    let (url, model_name) = env();
    let tmp = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap(); // DB lives OUTSIDE the workspace, unreadable by file tools
    let workspace = tmp.path().to_path_buf();
    let (mem_tools, retriever) = seed_memory(
        &workspace,
        db.path(),
        "The project's secret deploy codeword is BANANA-7.",
    )
    .await;

    let mut cfg = RuntimeConfig::from_launch(
        "openai".into(),
        url.clone(),
        model_name.clone(),
        "native".into(),
        262_144,
    );
    cfg.memory = true;
    let sink = Arc::new(Capture::default());
    let built = assemble_loop(
        &cfg,
        LoopParts {
            model: Arc::new(OpenAiCompatClient::new(url, model_name, None)),
            sink: sink.clone(),
            approval: Arc::new(AutoApprove),
            workspace: workspace.clone(),
            mcp_tools: vec![],
            memory_tools: mem_tools,
            memory_retriever: Some(retriever),
            stream_idle_timeout: Duration::from_secs(120),
            base_system_prompt: "You are a helpful assistant. Use any relevant memories provided."
                .into(),
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
    let mut ctx =
        WindowContext::new(Message::system(built.system_prompt.clone())).with_recall_budget(512);
    built
        .loop_
        .run(
            &mut ctx,
            "What is the secret deploy codeword for this project?".into(),
        )
        .await
        .unwrap();

    let answer = sink.text.lock().unwrap().clone();
    eprintln!("[memory ON] {answer}");
    assert!(
        answer.to_lowercase().contains("banana"),
        "memory ON: answer should reflect the auto-retrieved fact; got: {answer:?}"
    );
}

/// Negative: with memory OFF, the same seeded fact is NOT injected, so the model
/// cannot produce the codeword. Proves the `cfg.memory` gate end-to-end.
#[tokio::test]
#[ignore = "requires AGENT_E2E_URL / AGENT_E2E_MODEL, a live server, and the embedding model"]
async fn memory_off_suppresses_recall() {
    let (url, model_name) = env();
    let tmp = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap(); // DB outside the workspace: only auto-retrieval can surface it
    let workspace = tmp.path().to_path_buf();
    let (mem_tools, retriever) = seed_memory(
        &workspace,
        db.path(),
        "The project's secret deploy codeword is BANANA-7.",
    )
    .await;

    let mut cfg = RuntimeConfig::from_launch(
        "openai".into(),
        url.clone(),
        model_name.clone(),
        "native".into(),
        262_144,
    );
    cfg.memory = false; // gate OFF — tools/retriever are passed but must be ignored
    let sink = Arc::new(Capture::default());
    let built = assemble_loop(
        &cfg,
        LoopParts {
            model: Arc::new(OpenAiCompatClient::new(url, model_name, None)),
            sink: sink.clone(),
            approval: Arc::new(AutoApprove),
            workspace: workspace.clone(),
            mcp_tools: vec![],
            memory_tools: mem_tools,
            memory_retriever: Some(retriever),
            stream_idle_timeout: Duration::from_secs(120),
            base_system_prompt: "You are a helpful assistant. Use any relevant memories provided."
                .into(),
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
    let mut ctx =
        WindowContext::new(Message::system(built.system_prompt.clone())).with_recall_budget(512);
    built
        .loop_
        .run(
            &mut ctx,
            "What is the secret deploy codeword for this project?".into(),
        )
        .await
        .unwrap();

    let answer = sink.text.lock().unwrap().clone();
    eprintln!("[memory OFF] {answer}");
    assert!(
        !answer.to_lowercase().contains("banana-7"),
        "memory OFF: the gated fact must not leak; got: {answer:?}"
    );
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
            memory_tools: vec![],
            memory_retriever: None,
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
