//! LIVE SOAK TEST — cross-run file-based-memory persistence (spec §6 Live,
//! `2026-07-09-file-based-memory-design.md`): drive TWO real `assemble_loop`
//! sessions, back to back, over the SAME tempdir `memories_dir`. Run 1 is
//! prompted to remember a fact; run 2 is a completely FRESH loop/context that
//! must see the fact in its pinned memory block and answer from it.
//!
//! Mirrors `soak_live.rs`'s env-gated model setup verbatim (same env vars,
//! same skip semantics, same `SafeApproval` shape) — see that file for the
//! harness this one reuses.
//!
//! Opt-in (needs a live server). Run with:
//!   AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
//!     cargo test -p agent-runtime-config --test memory_files_soak \
//!     -- --ignored --nocapture

use agent_core::{AgentEvent, CuratedContext, EventSink, SessionArtifacts};
use agent_model::{Message, ModelClient, OpenAiCompatClient};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use agent_runtime_config::{assemble_loop, project_key, LoopParts, RuntimeConfig};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Approval gate that bounds the blast radius regardless of model behaviour:
/// workspace-bounded fs + context tools are allowed (incl. writes/edits under
/// `memories/`, which the policy engine floors at Ask — spec §2.6); everything
/// else is denied. Identical shape to `soak_live.rs::SafeApproval`.
struct SafeApproval {
    denied: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl ApprovalChannel for SafeApproval {
    async fn request(&self, r: ApprovalRequest) -> ApprovalResponse {
        let allow = match r.intent.tool.as_str() {
            "read_file" | "list_directory" | "write_file" | "edit_file" | "render"
            | "git_status" | "git_diff" | "grep" | "context_compact" => true,
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
                    )
                })
                .unwrap_or(false),
            _ => false,
        };
        if allow {
            ApprovalResponse::Approve
        } else {
            self.denied
                .lock()
                .unwrap()
                .push(format!("{}:{:?}", r.intent.tool, r.intent.command));
            ApprovalResponse::Deny { feedback: None }
        }
    }
}

/// Collects streamed reply text (`AgentEvent::Token`) and errors, so the
/// driver can inspect the model's final reply without reaching into
/// `CuratedContext`'s crate-private history accessor.
#[derive(Default)]
struct QuietSink {
    reply: Mutex<String>,
    errors: Mutex<Vec<String>>,
}
impl EventSink for QuietSink {
    fn emit(&self, e: AgentEvent) {
        match e {
            AgentEvent::Token(t) => self.reply.lock().unwrap().push_str(&t),
            AgentEvent::Error(m) => self.errors.lock().unwrap().push(m),
            _ => {}
        }
    }
}

/// Wraps a real `ModelClient` and records every request's full system+user
/// text, so the driver can inspect the actual rendered pinned prompt the
/// model saw — the literal text `MemoryFilesMiddleware` injects, without
/// reaching into agent-core's crate-private `MEMORY_HEADER` const. Mirrors
/// the `SchemaCapturingModel`/`DispatchCapture` capture-wrapper pattern used
/// by `dispatch.rs`'s own tests and `soak_live.rs`.
struct CapturingModel {
    inner: Arc<dyn ModelClient>,
    request_texts: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl ModelClient for CapturingModel {
    async fn stream(
        &self,
        req: agent_model::CompletionRequest,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<agent_model::Chunk, agent_model::ModelError>>,
        agent_model::ModelError,
    > {
        self.request_texts.lock().unwrap().push(
            req.messages
                .iter()
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        self.inner.stream(req).await
    }
}

/// One assemble_loop + one run_with_cancel call over a shared `memories_dir`,
/// each with its OWN fresh artifacts/todos/context — i.e. a genuinely
/// independent "run" (matches a fresh CLI invocation), not a continued
/// conversation. Returns (all request texts seen by the model, final reply,
/// sink for error diagnostics).
async fn run_once(
    url: &str,
    model_name: &str,
    ws: &std::path::Path,
    memories_dir: &std::path::Path,
    prompt: &str,
) -> (Vec<String>, String, Arc<QuietSink>) {
    let mut cfg = RuntimeConfig::from_launch(
        "openai".into(),
        url.to_string(),
        model_name.to_string(),
        "native".into(),
        8000,
    );
    cfg.memory = true;
    cfg.memories_dir = Some(memories_dir.to_string_lossy().to_string());
    cfg.sandbox_mode = "off".into(); // host executor — no docker needed
    cfg.max_turns = 8;

    let artifacts = Arc::new(SessionArtifacts::new());
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let todos = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::new(QuietSink::default());
    let approval = Arc::new(SafeApproval {
        denied: Mutex::new(Vec::new()),
    });
    let model = Arc::new(CapturingModel {
        inner: Arc::new(OpenAiCompatClient::new(
            url.to_string(),
            model_name.to_string(),
            std::env::var("AGENT_API_KEY").ok(),
        )),
        request_texts: Mutex::new(Vec::new()),
    });

    let built = assemble_loop(
        &cfg,
        LoopParts {
            model: model.clone(),
            sink: sink.clone(),
            approval: approval.clone(),
            workspace: ws.to_path_buf(),
            mcp_tools: vec![],
            stream_idle_timeout: Duration::from_secs(120),
            base_system_prompt:
                "You are a coding agent operating in a sandboxed workspace. Use the \
                provided tools (read_file, list_directory, write_file, edit_file, \
                execute_command, grep, context_compact) to complete each task, then \
                give a short final reply. Keep replies concise."
                    .into(),
            artifacts: artifacts.clone(),
            compact_flag: flag.clone(),
            todos: todos.clone(),
            sandbox: agent_runtime_config::build_sandbox(&cfg),
            stats: Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default())),
            trace: None,
            api_key: None,
            claude_binary: "claude".into(),
            checkpoint: None,
        },
    );
    let agent = built.loop_;

    let mut ctx = CuratedContext::new(
        Message::system(built.system_prompt),
        artifacts.clone(),
        flag,
    )
    .with_todos(todos.clone());

    let cancel = tokio_util::sync::CancellationToken::new();
    let outcome = tokio::time::timeout(
        Duration::from_secs(90),
        agent.run_with_cancel(&mut ctx, prompt.to_string(), cancel.clone()),
    )
    .await;
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(e)) => sink.errors.lock().unwrap().push(format!("run: {e}")),
        Err(_) => {
            cancel.cancel();
            sink.errors.lock().unwrap().push("task_timeout".into());
        }
    }

    let request_texts = model.request_texts.lock().unwrap().clone();
    let last_reply = sink.reply.lock().unwrap().clone();
    (request_texts, last_reply, sink)
}

/// Cross-run persistence (spec §6 Live): run 1 writes a memory via ordinary
/// tools; a FRESH run 2 sees its index line in the pinned block and can read
/// the node. Also asserts node-count == index-line-count (rot observability).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live soak: requires AGENT_E2E_URL / AGENT_E2E_MODEL and a live server"]
async fn memory_survives_across_runs() {
    let url = std::env::var("AGENT_E2E_URL").expect("set AGENT_E2E_URL");
    let model_name = std::env::var("AGENT_E2E_MODEL").expect("set AGENT_E2E_MODEL");

    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    let memories_dir = dir.path().join("memories");
    std::fs::create_dir_all(&memories_dir).unwrap();

    // ---- Run 1: remember a fact via ordinary tools ----
    let (_run1_requests, run1_reply, run1_sink) = run_once(
        &url,
        &model_name,
        &ws,
        &memories_dir,
        "Remember for later: the project mascot is a red panda. Write a memory \
         file under memories/project/ with this fact (OKF frontmatter: type + \
         description) and add its index line to memories/project/index.md, \
         then confirm briefly.",
    )
    .await;
    eprintln!(
        "[run1] errors={:?}\n[run1] reply={run1_reply}",
        run1_sink.errors.lock().unwrap()
    );

    // On-disk assertions: index.md exists, lists exactly 1 entry, and
    // node-count == index-line-count (rot observability, spec §4/§6).
    let key = project_key(&ws);
    let project_dir = memories_dir.join("projects").join(&key);
    let index_path = project_dir.join("index.md");
    assert!(
        index_path.exists(),
        "expected {index_path:?} to exist after run 1 wrote a memory"
    );
    let index_content = std::fs::read_to_string(&index_path).unwrap();
    let index_lines: Vec<&str> = index_content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert_eq!(
        index_lines.len(),
        1,
        "expected exactly 1 index entry after run 1, got: {index_content:?}"
    );

    // Node-count == index-line-count: every markdown file in the project dir
    // other than index.md itself is a node; each index line should name one.
    let node_files: Vec<_> = std::fs::read_dir(&project_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".md") && name != "index.md"
        })
        .collect();
    assert_eq!(
        node_files.len(),
        index_lines.len(),
        "node-count ({}) must equal index-line-count ({}) — rot check: {node_files:?} vs {index_lines:?}",
        node_files.len(),
        index_lines.len()
    );

    // ---- Run 2: FRESH loop, same memories_dir — recall the fact ----
    let (run2_requests, run2_reply, run2_sink) = run_once(
        &url,
        &model_name,
        &ws,
        &memories_dir,
        "What is the project mascot?",
    )
    .await;
    eprintln!(
        "[run2] errors={:?}\n[run2] reply={run2_reply}",
        run2_sink.errors.lock().unwrap()
    );

    // The FIRST request of the fresh run 2 loop carries the pinned block
    // (system + memory index, injected by MemoryFilesMiddleware::on_run_start
    // before any turn runs) — assert on the literal header text (MEMORY_HEADER
    // is agent-core-crate-private) and the index entry's content.
    assert!(
        !run2_requests.is_empty(),
        "run 2 must have sent at least one request to the model"
    );
    let first_request = &run2_requests[0];
    assert!(
        first_request.contains("Long-term memory")
            && first_request.to_lowercase().contains("memories/project"),
        "run 2's rendered pinned prompt must contain the memory-index header \
         naming the store: {first_request}"
    );
    assert!(
        first_request.to_lowercase().contains("red panda")
            || first_request.to_lowercase().contains("mascot"),
        "run 2's rendered pinned prompt must contain the remembered index \
         entry: {first_request}"
    );

    // The reply itself must mention the fact (the model actually recalled it,
    // not just that the block was present).
    assert!(
        run2_reply.to_lowercase().contains("red panda"),
        "run 2's reply must mention the remembered fact (red panda): {run2_reply}"
    );
}
