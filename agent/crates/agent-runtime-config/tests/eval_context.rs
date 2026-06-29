//! LIVE EVAL HARNESS — one run of one task under one CandidateConfig. Drives the
//! real `assemble_loop` (single- or cross-session), sums SERVER-reported token usage,
//! runs the hidden tests in a sealed step, and prints one `RunResult` JSON line.
//! Opt-in (needs a live server). Driven by the context-evolve skill. Run with:
//!   AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
//!   TASK_JSON=task.json CONFIG_JSON=cfg.json HIDDEN_TESTS_DIR=hidden_tests \
//!   cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture
use agent_core::{AgentEvent, CuratedContext, EventSink, InMemoryOffloadStore, OffloadStore, Retriever};
use agent_memory::{
    build_tools_with, project_scope, Embedder, FastEmbedEmbedder, MemoryConfig, MemoryRetriever,
    MemoryScope, MemoryStore, SqliteStore, StubEmbedder,
};
use agent_model::{Message, OpenAiCompatClient};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use agent_runtime_config::eval::{CandidateConfig, RunResult, TaskSpec};
use agent_runtime_config::{assemble_loop, LoopParts, RuntimeConfig};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Bounds the blast radius regardless of model behaviour: workspace-bounded fs +
/// context/memory tools are allowed; `execute_command` only for read-only commands.
struct SafeApproval {
    denied: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl ApprovalChannel for SafeApproval {
    async fn request(&self, r: ApprovalRequest) -> ApprovalResponse {
        let allow = match r.intent.tool.as_str() {
            "read_file" | "list_directory" | "write_file" | "edit_file" | "render" | "git_status"
            | "git_diff" | "context_recall" | "context_compact" | "remember" | "recall" | "forget" => true,
            "execute_command" => r
                .intent
                .command
                .as_deref()
                .map(|c| {
                    let first = c.split_whitespace().next().unwrap_or("");
                    let base = first.rsplit('/').next().unwrap_or(first);
                    matches!(
                        base,
                        "ls" | "cat" | "wc" | "head" | "tail" | "echo" | "grep" | "find" | "pwd"
                            | "sort" | "uniq" | "true" | "date" | "nl"
                            // `cargo` for code tasks (e.g. locked-hostpolicy): lets the agent
                            // build/check its work. Bounded: eval crates are std-only, no deps,
                            // no build.rs — so cargo only invokes rustc on trusted local source.
                            | "cargo"
                    )
                })
                .unwrap_or(false),
            _ => false,
        };
        if allow {
            ApprovalResponse::Approve
        } else {
            self.denied.lock().unwrap().push(format!("{}:{:?}", r.intent.tool, r.intent.command));
            ApprovalResponse::Deny
        }
    }
}

/// Sums the faithful, server-reported token metric across every turn of every session.
#[derive(Default)]
struct TokenMeter {
    total: AtomicU64,
    turns: AtomicU64,
}
impl EventSink for TokenMeter {
    fn emit(&self, e: AgentEvent) {
        if let AgentEvent::ServerUsage { prompt_tokens, completion_tokens, .. } = e {
            self.total.fetch_add(prompt_tokens as u64 + completion_tokens as u64, Ordering::Relaxed);
            self.turns.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live eval: requires AGENT_E2E_URL/MODEL, TASK_JSON, CONFIG_JSON, HIDDEN_TESTS_DIR"]
async fn eval_context_run() {
    let url = std::env::var("AGENT_E2E_URL").expect("set AGENT_E2E_URL");
    let model = std::env::var("AGENT_E2E_MODEL").expect("set AGENT_E2E_MODEL");
    let task: TaskSpec = TaskSpec::from_json(
        &std::fs::read_to_string(std::env::var("TASK_JSON").expect("set TASK_JSON")).unwrap(),
    )
    .unwrap();
    let cc: CandidateConfig = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("CONFIG_JSON").expect("set CONFIG_JSON")).unwrap(),
    )
    .unwrap();
    let hidden = std::env::var("HIDDEN_TESTS_DIR").expect("set HIDDEN_TESTS_DIR");

    // Throwaway workspace + seed files. Memory store SHARED across sessions; each
    // session gets a FRESH window (new CuratedContext + new offload store).
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    for sf in &task.seed_files {
        let dest = ws.join(&sf.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).unwrap(); // support nested seed paths (e.g. src/lib.rs)
        }
        std::fs::write(dest, &sf.contents).unwrap();
    }

    let mem_db = ws.join("memory.db");
    let meter = Arc::new(TokenMeter::default());

    for session in &task.sessions {
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(),
            url.clone(),
            model.clone(),
            "native".into(),
            cc.context_limit,
        );
        cfg.context_limit = cc.context_limit; // realistic (or favorable) window
        cfg.memory = task.memory_enabled && cc.memory_enabled;
        cfg.sandbox_mode = "off".into();
        cfg.max_turns = 12;

        // Memory: shared SqliteStore so facts persist across sessions. Default embedder
        // is the deterministic StubEmbedder (exact-match only, no network). Set
        // EVAL_REAL_EMBEDDINGS=1 (+ optional FASTEMBED_CACHE=<dir>) to use the real
        // BGE-Small model — required for any task that tests *semantic* recall, since the
        // stub scores distinct text near-orthogonal regardless of meaning.
        let (mem_tools, retriever) = if cfg.memory {
            let store: Arc<dyn MemoryStore> = Arc::new(SqliteStore::open(&mem_db).unwrap());
            let embedder: Arc<dyn Embedder> = if std::env::var("EVAL_REAL_EMBEDDINGS").is_ok() {
                let mut ecfg = MemoryConfig::default();
                ecfg.model_cache_dir = std::env::var("FASTEMBED_CACHE").ok().map(std::path::PathBuf::from);
                Arc::new(FastEmbedEmbedder::new(&ecfg).expect("load BGE-Small embedder"))
            } else {
                Arc::new(StubEmbedder::d384())
            };
            let mut mcfg = MemoryConfig::default();
            mcfg.default_k = cc.default_k;
            mcfg.relevance_threshold = cc.relevance_threshold;
            mcfg.dedup_threshold = cc.dedup_threshold;
            mcfg.forget_threshold = cc.forget_threshold;
            mcfg.max_recall_chars = cc.max_recall_chars;
            mcfg.recall_token_budget = cc.recall_token_budget;
            mcfg.auto_recall = cc.auto_recall;
            let mcfg = Arc::new(mcfg);
            let scope = project_scope(&ws);
            let tools = build_tools_with(embedder.clone(), store.clone(), mcfg.clone(), scope.clone());
            let key = match &scope {
                MemoryScope::Project(k) => k.clone(),
                MemoryScope::Global => String::new(),
            };
            let r: Arc<dyn Retriever> =
                Arc::new(MemoryRetriever { embedder, store, cfg: mcfg, project_key: key });
            (tools, Some(r))
        } else {
            (vec![], None)
        };

        let offload: Arc<dyn OffloadStore> = Arc::new(InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        let built = assemble_loop(
            &cfg,
            LoopParts {
                model: Arc::new(OpenAiCompatClient::new(
                    url.clone(),
                    model.clone(),
                    std::env::var("AGENT_API_KEY").ok(),
                )),
                sink: meter.clone(),
                approval: Arc::new(SafeApproval { denied: Mutex::new(Vec::new()) }),
                workspace: ws.clone(),
                mcp_tools: vec![],
                memory_tools: mem_tools,
                memory_retriever: retriever,
                stream_idle_timeout: Duration::from_secs(120),
                base_system_prompt: "You are a coding agent operating in a sandboxed workspace. Use \
                    the provided tools to complete each task, then give a short final reply."
                    .into(),
                offload_store: offload.clone(),
                compact_flag: flag.clone(),
            },
        );
        let agent = built.loop_;
        let mut ctx = CuratedContext::new(Message::system(built.system_prompt), offload.clone(), flag)
            .with_recall_budget(cc.recall_budget)
            .with_offload_config(cc.offload_config())
            .with_high_water_pct(cc.high_water_pct);

        for prompt in &session.prompts {
            let cancel = tokio_util::sync::CancellationToken::new();
            let run = agent.run_with_cancel(&mut ctx, prompt.clone(), cancel.clone());
            let _ = tokio::time::timeout(Duration::from_secs(120), run).await;
        }
    }

    // Sealed grading step: copy hidden tests in, run test_cmd, capture exit code.
    let dest = ws.join("hidden_tests");
    std::fs::create_dir_all(&dest).unwrap();
    for entry in std::fs::read_dir(&hidden).unwrap() {
        let e = entry.unwrap();
        if e.path().is_file() {
            std::fs::copy(e.path(), dest.join(e.file_name())).unwrap();
        }
    }
    let status = std::process::Command::new("bash")
        .arg("-c")
        .arg(&task.test_cmd)
        .current_dir(&ws)
        .status()
        .unwrap();

    let result = RunResult {
        passed: status.success(),
        tokens: meter.total.load(Ordering::Relaxed),
        turns: meter.turns.load(Ordering::Relaxed) as usize,
    };
    println!("{}", serde_json::to_string(&result).unwrap());
}
