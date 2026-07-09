//! LIVE EVAL HARNESS — one run of one task under one CandidateConfig. Drives the
//! real `assemble_loop` (single- or cross-session), sums SERVER-reported token usage,
//! runs the hidden tests in a sealed step, and prints one `RunResult` JSON line.
//! Opt-in (needs a live server). Driven by the context-evolve skill. Run with:
//!   AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
//!   TASK_JSON=task.json CONFIG_JSON=cfg.json HIDDEN_TESTS_DIR=hidden_tests \
//!   cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture
use agent_core::{AgentEvent, CuratedContext, EventSink, Retriever, SessionArtifacts};
use agent_memory::{
    build_tools_with, project_scope, Embedder, FastEmbedEmbedder, MemoryConfig, MemoryRetriever,
    MemoryScope, MemoryStore, SqliteStore, StubEmbedder,
};
use agent_model::{Message, OpenAiCompatClient};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use agent_runtime_config::eval::{CandidateConfig, RunResult, TaskSpec, TrajectoryStep};
use agent_runtime_config::{assemble_loop, LoopParts, RuntimeConfig};
use agent_sandbox::docker_run_args;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const EVAL_DEFAULT_PROMPT: &str =
    "You are a coding agent operating in a sandboxed workspace. Use the provided \
    tools to complete each task, then give a short final reply.";

/// Bounds the blast radius regardless of model behaviour: workspace-bounded fs +
/// context/memory tools are allowed; `execute_command` only for read-only commands.
struct SafeApproval {
    denied: Mutex<Vec<String>>,
    /// True for exec_profile == "node-offline": node/npx/vite/vitest/tsc are
    /// approvable because execution is docker-contained (network none, read-only
    /// root). NOT extended to npm install/ci — offline by construction.
    node_profile: bool,
}
#[async_trait::async_trait]
impl ApprovalChannel for SafeApproval {
    async fn request(&self, r: ApprovalRequest) -> ApprovalResponse {
        let allow = match r.intent.tool.as_str() {
            "read_file" | "list_directory" | "write_file" | "edit_file" | "render"
            | "git_status" | "git_diff" | "grep" | "context_compact" | "remember"
            | "recall" | "forget" => true,
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
                    ) || (self.node_profile
                        && matches!(base, "node" | "npx" | "tsc" | "vitest" | "vite"))
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
            ApprovalResponse::Deny
        }
    }
}

/// Sums the faithful, server-reported token metric across every turn of every session,
/// and records the ordered ToolStart trajectory (diagnostic; spec 2026-07-02 eval-flywheel §3).
#[derive(Default)]
struct TokenMeter {
    total: AtomicU64,
    turns: AtomicU64,
    trajectory: Mutex<Vec<TrajectoryStep>>,
}
impl EventSink for TokenMeter {
    fn emit(&self, e: AgentEvent) {
        match e {
            AgentEvent::ServerUsage {
                prompt_tokens,
                completion_tokens,
                ..
            } => {
                self.total.fetch_add(
                    prompt_tokens as u64 + completion_tokens as u64,
                    Ordering::Relaxed,
                );
                self.turns.fetch_add(1, Ordering::Relaxed);
            }
            AgentEvent::ToolStart { name, args, .. } => {
                self.trajectory
                    .lock()
                    .unwrap()
                    .push(TrajectoryStep { tool: name, args });
            }
            _ => {}
        }
    }
}

/// Phase-0 exec-profile → sandbox mapping (spec 2026-07-03 harness-evolve).
/// Kept as a testable helper: a silent flip of this branch would run
/// agent-written node code on the HOST.
fn apply_exec_profile(cfg: &mut agent_runtime_config::RuntimeConfig, node_offline: bool) {
    if node_offline {
        cfg.sandbox_mode = "enforce".into();
        cfg.sandbox_image = "node:22-bookworm-slim".into();
        cfg.sandbox_memory = "4g".into();
        cfg.sandbox_pids = 1024;
    } else {
        cfg.sandbox_mode = "off".into();
    }
}

#[test]
fn exec_profile_mapping_is_fail_closed() {
    let base = || {
        agent_runtime_config::RuntimeConfig::from_launch(
            "openai".into(),
            "http://x".into(),
            "m".into(),
            "native".into(),
            8192,
        )
    };
    let mut off = base();
    apply_exec_profile(&mut off, false);
    assert_eq!(off.sandbox_mode, "off");

    let mut node = base();
    apply_exec_profile(&mut node, true);
    assert_eq!(node.sandbox_mode, "enforce");
    assert_eq!(node.sandbox_image, "node:22-bookworm-slim");
    assert_eq!(node.sandbox_memory, "4g");
    assert_eq!(node.sandbox_pids, 1024);
    assert!(
        !node.sandbox_network,
        "network must stay none for node-offline"
    );
}

// Legacy lint, unrelated to this branch: field-by-field config build reads clearly here.
#[allow(clippy::field_reassign_with_default)]
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
    let task_json_path = std::path::PathBuf::from(std::env::var("TASK_JSON").unwrap());
    let task_dir = task_json_path
        .parent()
        .expect("TASK_JSON has a parent dir")
        .to_path_buf();
    let node_offline = task.exec_profile.as_deref() == Some("node-offline");

    // Throwaway workspace + seed files. Memory store SHARED across sessions; each
    // session gets a FRESH window (new CuratedContext + new offload store).
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    if let Some(sd) = &task.seed_dir {
        let src = task_dir.join(sd);
        assert!(
            src.is_dir(),
            "seed_dir {} missing — run the task's seed.sh first",
            src.display()
        );
        // cp -a preserves the node_modules tree (symlinked .bin entries included).
        let st = std::process::Command::new("cp")
            .arg("-a")
            .arg(format!("{}/.", src.display()))
            .arg(&ws)
            .status()
            .unwrap();
        assert!(st.success(), "seed_dir copy failed");
    }
    for sf in &task.seed_files {
        let dest = ws.join(&sf.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).unwrap(); // support nested seed paths (e.g. src/lib.rs)
        }
        std::fs::write(dest, &sf.contents).unwrap();
    }

    let mem_db = ws.join("memory.db");
    let meter = Arc::new(TokenMeter::default());
    // Shared across sessions (like `meter`) so denials accumulate for the whole run and
    // stay in scope at RunResult construction below.
    let approval = Arc::new(SafeApproval {
        denied: Mutex::new(Vec::new()),
        node_profile: node_offline,
    });

    let started = std::time::Instant::now();
    for session in &task.sessions {
        let protocol = cc.resolved_protocol("native").to_string();
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(),
            url.clone(),
            model.clone(),
            protocol,
            cc.context_limit,
        );
        cfg.context_limit = cc.context_limit; // realistic (or favorable) window
        cfg.memory = task.memory_enabled && cc.memory_enabled;
        // Phase-0 node-offline profile (spec 2026-07-03): enforced docker
        // sandbox, pinned node image, network stays none (the default).
        apply_exec_profile(&mut cfg, node_offline);
        cfg.max_turns = 12; // historical default; candidates override via max_turns
        cc.apply_to(&mut cfg);
        // Additive eval hook: let a run opt into a skills catalog so example-bearing
        // skills (list_skills/use_skill/read_skill_file) are exercised. Unset = no skills.
        if let Ok(d) = std::env::var("SKILLS_DIR") {
            cfg.skills_dirs = vec![d];
        }

        // Memory: shared SqliteStore so facts persist across sessions. Default embedder
        // is the deterministic StubEmbedder (exact-match only, no network). Set
        // EVAL_REAL_EMBEDDINGS=1 (+ optional FASTEMBED_CACHE=<dir>) to use the real
        // BGE-Small model — required for any task that tests *semantic* recall, since the
        // stub scores distinct text near-orthogonal regardless of meaning.
        let (mem_tools, retriever) = if cfg.memory {
            let store: Arc<dyn MemoryStore> = Arc::new(SqliteStore::open(&mem_db).unwrap());
            let embedder: Arc<dyn Embedder> = if std::env::var("EVAL_REAL_EMBEDDINGS").is_ok() {
                let mut ecfg = MemoryConfig::default();
                ecfg.model_cache_dir = std::env::var("FASTEMBED_CACHE")
                    .ok()
                    .map(std::path::PathBuf::from);
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
            let tools =
                build_tools_with(embedder.clone(), store.clone(), mcfg.clone(), scope.clone());
            let key = match &scope {
                MemoryScope::Project(k) => k.clone(),
                MemoryScope::Global => String::new(),
            };
            let r: Arc<dyn Retriever> = Arc::new(MemoryRetriever {
                embedder,
                store,
                cfg: mcfg,
                project_key: key,
            });
            (tools, Some(r))
        } else {
            (vec![], None)
        };

        let artifacts = Arc::new(SessionArtifacts::new());
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
                approval: approval.clone(),
                workspace: ws.clone(),
                mcp_tools: vec![],
                memory_tools: mem_tools,
                memory_retriever: retriever,
                stream_idle_timeout: Duration::from_secs(120),
                base_system_prompt: cc.resolved_system_prompt(EVAL_DEFAULT_PROMPT).to_string(),
                artifacts: artifacts.clone(),
                compact_flag: flag.clone(),
                sandbox: agent_runtime_config::build_sandbox(&cfg),
                stats: Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default())),
                trace: None,
                api_key: None,
                claude_binary: "claude".into(),
            },
        );
        let agent = built.loop_;
        let mut ctx = CuratedContext::new(
            Message::system(built.system_prompt),
            artifacts.clone(),
            flag,
        )
        .with_recall_budget(cc.recall_budget)
        .with_offload_config(cc.offload_config())
        .with_high_water_pct(cc.high_water_pct);

        let per_prompt = Duration::from_secs(task.prompt_timeout_secs.unwrap_or(120));
        for prompt in &session.prompts {
            let cancel = tokio_util::sync::CancellationToken::new();
            let run = agent.run_with_cancel(&mut ctx, prompt.clone(), cancel.clone());
            let _ = tokio::time::timeout(per_prompt, run).await;
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
    let status = if node_offline {
        // Grade INSIDE the same container profile: vitest/vite execute agent-written
        // code (untrusted output) — it gets the same boundary as the agent.
        let uid = String::from_utf8(
            std::process::Command::new("id")
                .arg("-u")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let gid = String::from_utf8(
            std::process::Command::new("id")
                .arg("-g")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let policy = agent_sandbox::SandboxPolicy {
            mode: agent_tools::Mode::Enforce,
            image: "node:22-bookworm-slim".into(),
            network: false,
            limits: agent_tools::Limits {
                memory: "4g".into(),
                cpus: "2".into(),
                pids: 1024,
                fsize: None,
                tmp_size: "256m".into(),
            },
            extra_rw: vec![],
            extra_ro: vec![],
        };
        let spec = agent_tools::CommandSpec {
            program: "bash".into(),
            args: vec!["-c".into(), task.test_cmd.clone()],
            cwd: ws.clone(),
            env: Default::default(),
            kind: agent_tools::ProcKind::OneShot,
        };
        let name = format!("eval-grade-{}", std::process::id());
        let args = docker_run_args(
            &policy,
            &spec,
            &name,
            &format!("{}:{}", uid.trim(), gid.trim()),
        );
        std::process::Command::new("docker")
            .args(&args)
            .status()
            .unwrap()
    } else {
        std::process::Command::new("bash")
            .arg("-c")
            .arg(&task.test_cmd)
            .current_dir(&ws)
            .status()
            .unwrap()
    };

    // Denials go to stderr only (prefixed `eval-denied:`); stdout stays exactly one JSON line.
    let denied = approval.denied.lock().unwrap();
    for d in denied.iter() {
        eprintln!("eval-denied: {d}");
    }
    let trajectory = meter.trajectory.lock().unwrap().clone();
    let gold_matched = if task.gold_trajectory.is_empty() {
        None
    } else {
        Some(agent_runtime_config::eval::trajectory_matches_gold(
            &trajectory,
            &task.gold_trajectory,
        ))
    };
    let result = RunResult {
        passed: status.success(),
        tokens: meter.total.load(Ordering::Relaxed),
        turns: meter.turns.load(Ordering::Relaxed) as usize,
        trajectory,
        denials: denied.len(),
        gold_matched,
        wall_ms: started.elapsed().as_millis() as u64,
    };
    println!("{}", serde_json::to_string(&result).unwrap());
}
