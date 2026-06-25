//! Live, opt-in e2e for memory auto-retrieval: seed a memory, then ask a related
//! question with NO tools and NO mention of the fact — the retriever must inject it
//! into context and the real model must answer from it.
//! Run with: AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
//!           cargo test -p agent-runtime-config --test e2e_auto_retrieval -- --ignored --nocapture
//! (agent-memory is pulled with its default `onnx` feature, so the real embedder is used.)

use agent_core::{AgentEvent, AgentLoop, EventSink, LoopConfig, WindowContext};
use agent_memory::{
    assemble_memory, open_memory_parts, project_scope, now_secs,
    MemoryConfig, MemoryRecord, MemoryStore,
};
use agent_model::{Message, NativeProtocol, OpenAiCompatClient};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse, RulePolicy};
use agent_tools::ToolRegistry;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Collects streamed assistant text so we can assert on the final answer.
struct TextCapture(Mutex<String>);
impl EventSink for TextCapture {
    fn emit(&self, e: AgentEvent) {
        if let AgentEvent::Token(t) = e {
            self.0.lock().unwrap().push_str(&t);
        }
    }
}

struct AutoApprove;
#[async_trait::async_trait]
impl ApprovalChannel for AutoApprove {
    async fn request(&self, _r: ApprovalRequest) -> ApprovalResponse { ApprovalResponse::Approve }
}

#[tokio::test]
#[ignore = "requires AGENT_E2E_URL / AGENT_E2E_MODEL, a live server, and the embedding model"]
async fn auto_retrieval_feeds_a_seeded_fact_to_the_real_model() {
    let url = std::env::var("AGENT_E2E_URL").expect("set AGENT_E2E_URL");
    let model_name = std::env::var("AGENT_E2E_MODEL").expect("set AGENT_E2E_MODEL");

    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().to_path_buf();
    // Isolated DB so we never touch the shared ~/.agent/memory.db; default embedder cache.
    let cfg = MemoryConfig { db_path: tmp.path().join("memory.db"), ..MemoryConfig::default() };
    let parts = open_memory_parts(cfg).expect("load embedding model + open store");

    // Seed a distinctive fact scoped to this workspace (no tool call needed).
    let fact = "The project's secret deploy codeword is BANANA-7.";
    let vector = parts.embedder.embed(&[fact.to_string()]).await.unwrap().remove(0);
    parts.store.upsert(MemoryRecord {
        id: "seed-1".into(), text: fact.into(), scope: project_scope(&workspace),
        tags: vec![], vector, created_at: now_secs(), updated_at: now_secs(), source: "e2e".into(),
    }).await.unwrap();

    let (_tools, retriever) = assemble_memory(&parts, &workspace);

    // Empty tool registry: this isolates auto-retrieval (the model answers purely from
    // the injected recall block, not from calling a tool).
    let sink = Arc::new(TextCapture(Mutex::new(String::new())));
    let agent = AgentLoop::new(
        Arc::new(OpenAiCompatClient::new(url, model_name, None)),
        Arc::new(NativeProtocol),
        Arc::new(ToolRegistry::new()),
        Arc::new(RulePolicy { workspace: workspace.clone(), command_allowlist: vec![], command_denylist: vec![] }),
        Arc::new(AutoApprove),
        sink.clone(),
        LoopConfig {
            model_limit: 100_000, max_turns: 3, max_retries: 2, temperature: 0.0,
            max_tokens: Some(256), workspace: workspace.clone(),
            tool_timeout: Duration::from_secs(60),
            stream_idle_timeout: Duration::from_secs(120),
            ..Default::default()
        },
    ).with_retriever(retriever);

    let mut ctx = WindowContext::new(Message::system(
        "You are a helpful assistant. Use any relevant memories provided to answer."));
    // The query relates to the fact but never mentions the codeword.
    agent.run(&mut ctx, "What is the secret deploy codeword for this project?".into())
        .await.unwrap();

    let answer = sink.0.lock().unwrap().clone();
    eprintln!("MODEL ANSWER: {answer}");
    assert!(
        answer.to_lowercase().contains("banana"),
        "model answer should reflect the auto-retrieved memory; got: {answer:?}"
    );
}
