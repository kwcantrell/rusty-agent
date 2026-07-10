mod approval;
mod render;

use agent_core::CuratedContext;
use agent_model::Message;
use agent_runtime_config::{
    assemble_loop, backend_name_is_valid, build_model, build_sandbox, claude_cli_opts,
    default_allowlist, default_denylist, LoopParts, RuntimeConfig, BASE_SYSTEM_PROMPT,
    DEFAULT_SANDBOX_IMAGE,
};
use approval::{ParkExit, TerminalApproval};
use clap::Parser;
use render::TerminalSink;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(clap::Subcommand)]
enum Command {
    /// Inspect and reopen past sessions.
    ///
    /// Top-level flags (--base-url, --workspace, etc.) go BEFORE the
    /// subcommand: `agent --base-url <url> sessions reopen <id>`.
    Sessions {
        #[command(subcommand)]
        cmd: SessionsCmd,
    },
}

#[derive(clap::Subcommand)]
enum SessionsCmd {
    /// List sessions, newest first; parked runs are marked.
    List,
    /// Reopen a parked session: re-prompt its pending approval and resume.
    Reopen { session_id: String },
}

struct SessionRow {
    session_id: String,
    workspace: PathBuf,
    created_ms: u64,
    parked: bool,
}

fn list_sessions(root: &Path) -> Vec<SessionRow> {
    agent_runtime_config::scan_descriptors(root)
        .into_iter()
        .map(|d| {
            let parked = agent_core::checkpoint::has_park(
                &agent_runtime_config::session_dir(root, &d.session_id).join("checkpoint"),
            );
            SessionRow {
                session_id: d.session_id,
                workspace: d.workspace,
                created_ms: d.created_ms,
                parked,
            }
        })
        .collect()
}

async fn run_sessions_cmd(cmd: &SessionsCmd, cli: &Cli) {
    match cmd {
        SessionsCmd::List => {
            let rt = runtime_config_from_cli(cli, "prompted");
            let Some(root) = agent_runtime_config::sessions_root(&rt) else {
                eprintln!("error: cannot determine sessions root (is $HOME set?)");
                std::process::exit(2);
            };
            for row in list_sessions(&root) {
                let marker = if row.parked { "  [PARKED]" } else { "" };
                println!(
                    "{}  {}  {}{}",
                    row.session_id,
                    row.created_ms,
                    row.workspace.display(),
                    marker
                );
            }
            println!("\nreopen a parked session: agent sessions reopen <id>");
        }
        SessionsCmd::Reopen { session_id } => {
            run_sessions_reopen(session_id, cli).await;
        }
    }
}

/// Delete a completed run's root checkpoint tree (`<session>/checkpoint/`,
/// including `resume.lock`). Mirrors the server's delete-on-completion reap
/// in `session.rs::start_resume` on the `Ok(())` arm; best-effort, same as
/// the server's `let _ = remove_dir_all(..)`.
fn reap_root(root_dir: &Path) {
    let _ = std::fs::remove_dir_all(root_dir);
}

/// What a resumed run's exit should do to its root checkpoint tree.
/// `resume_with_cancel`/`turn_loop` return `Ok(())` on BOTH true completion
/// AND cancellation (background fact, branch review C-1) — `Ok` alone is
/// never enough to decide reap vs. retain.
#[derive(Debug, PartialEq, Eq)]
enum ResumeCleanup {
    /// The tree truly finished: delete-on-completion (spec §2.3).
    Reap,
    /// Cancelled (or failed) mid-resume: retain the park, release the
    /// resume lock + guard so a later reopen can retry (mirrors the
    /// server's Err arm and dispatch.rs's is_cancelled()-before-Ok check).
    Retain,
}

/// Decide cleanup from a resumed run's `Result` and whether its cancel token
/// tripped. Pure/unit-testable seam for the C-1 fix (`run_sessions_reopen`'s
/// Ok(()) arm previously reaped unconditionally, destroying a park that was
/// deliberately retained because the run was cancelled while re-parked).
fn resume_cleanup_decision<E>(result: &Result<(), E>, cancelled: bool) -> ResumeCleanup {
    match result {
        Ok(()) if cancelled => ResumeCleanup::Retain,
        Ok(()) => ResumeCleanup::Reap,
        Err(_) => ResumeCleanup::Retain,
    }
}

/// claude-cli is a pure text generator; tool calls must come via the
/// prompted protocol — forced regardless of `--protocol`. Shared by the
/// normal run path and `sessions reopen` (I-2, 4B-2 branch review) so both
/// derive the same protocol from the same conditions.
fn select_protocol(cli: &Cli) -> &str {
    if cli.backend == "claude-cli" {
        if cli.protocol != "prompted" {
            eprintln!("note: forcing --protocol prompted for claude-cli backend");
        }
        "prompted"
    } else {
        cli.protocol.as_str()
    }
}

/// `agent sessions reopen <id>`: re-prompt one parked run's first unanswered
/// ask (feedback-capable) and resume its tree to completion (spec §2.4, CLI
/// surface — 4B-2 Task 10).
async fn run_sessions_reopen(session_id: &str, cli: &Cli) {
    let rt = runtime_config_from_cli(cli, select_protocol(cli));
    let Some(root) = agent_runtime_config::sessions_root(&rt) else {
        eprintln!("error: cannot determine sessions root (is $HOME set?)");
        std::process::exit(2);
    };
    let Some(descriptor) = agent_runtime_config::scan_descriptors(&root)
        .into_iter()
        .find(|d| d.session_id == session_id)
    else {
        eprintln!("session {session_id}: not found");
        std::process::exit(2);
    };
    if !descriptor.workspace.is_dir() {
        eprintln!(
            "session {session_id}: workspace no longer exists; refusing to reopen (parks retained)"
        );
        std::process::exit(2);
    }

    let Some(meta_root) = agent_runtime_config::metadata_root_for(&rt) else {
        eprintln!("error: cannot determine metadata root (is $HOME set?)");
        std::process::exit(2);
    };
    let key = match agent_runtime_config::load_or_create_secret(&meta_root) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("error: cannot load session secret: {e}");
            std::process::exit(2);
        }
    };
    let Some(parked) = agent_server::resume::scan_parked_session(&root, &key, descriptor.clone())
    else {
        eprintln!("session {session_id}: nothing parked");
        std::process::exit(2);
    };
    if !parked.errors.is_empty() {
        for e in &parked.errors {
            eprintln!("checkpoint unreadable; run cannot be resumed ({e})");
        }
        std::process::exit(2);
    }
    let Some(root_chk) = parked.root.clone() else {
        eprintln!("session {session_id}: parked tree has no root checkpoint; cannot resume");
        std::process::exit(2);
    };

    // Claim the resume lock BEFORE prompting the human (refinement 11 — no
    // wasted answer). Every error exit from here on releases it.
    match agent_core::checkpoint::claim_resume(&parked.root_dir) {
        Ok(true) => {}
        _ => {
            eprintln!(
                "session {session_id} is being resumed elsewhere (another daemon may hold it); \
                 if that process crashed, remove {}/resume.lock",
                parked.root_dir.display()
            );
            std::process::exit(2);
        }
    }

    // Assemble the loop FIRST — display re-derivation needs it. Mirrors
    // session.rs wire_parked_session/start_resume: the loop (and its
    // system_prompt) is built before the CuratedContext restore.
    let artifacts = Arc::new(agent_core::SessionArtifacts::new());
    let todos: agent_core::TodoHandle = Arc::new(std::sync::Mutex::new(Vec::new()));
    let compact_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let checkpoint =
        agent_core::Checkpointer::new(parked.root_dir.clone(), key, descriptor.session_id.clone());
    // M-4 (4B-2 branch review): build the trace writer BEFORE the ParkExit
    // that carries it, so a park-exit during THIS resumed run flushes the
    // per-descriptor trace instead of hardcoding `trace: None` (process::exit
    // skips Drop — a leaked trace tail-loss is a real audit gap, same
    // rationale as ParkExit::trace's doc comment).
    let trace = agent_runtime_config::build_trace(&rt, &descriptor.session_id);
    let approval = TerminalApproval::with_park_exit(Some(ParkExit {
        session_id: descriptor.session_id.clone(),
        trace: trace.clone(),
        release_lock: Some(parked.root_dir.clone()),
        exit: Box::new(|code| std::process::exit(code)),
    }));
    let model = build_model(
        &cli.backend,
        &cli.base_url,
        &cli.model,
        &cli.claude_binary,
        std::env::var("AGENT_API_KEY").ok(),
        claude_cli_opts(&rt),
    );
    let sandbox = build_sandbox(&rt);
    let stats = Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default()));
    let built = assemble_loop(
        &rt,
        LoopParts {
            model,
            sink: Arc::new(TerminalSink::default()),
            approval: Arc::new(approval),
            workspace: descriptor.workspace.clone(),
            mcp_tools: Vec::new(),
            stream_idle_timeout: Duration::from_secs(cli.stream_timeout_secs),
            base_system_prompt: BASE_SYSTEM_PROMPT.to_string(),
            artifacts: artifacts.clone(),
            compact_flag: compact_flag.clone(),
            todos: todos.clone(),
            sandbox,
            stats: stats.clone(),
            trace,
            api_key: std::env::var("AGENT_API_KEY").ok(),
            claude_binary: cli.claude_binary.clone(),
            checkpoint: Some(checkpoint),
        },
    );
    if !built.unknown_presets.is_empty() {
        eprintln!(
            "skills: unknown active skill(s): {}",
            built.unknown_presets.join(", ")
        );
        agent_core::checkpoint::release_resume(&parked.root_dir);
        std::process::exit(2);
    }

    if let Ok(dump) = agent_core::checkpoint::load_artifact_dump(&parked.root_dir, &key) {
        agent_core::checkpoint::restore_artifacts(&artifacts, &dump).await;
    }
    let mut ctx = CuratedContext::restore(
        Message::system(built.system_prompt.clone()),
        artifacts.clone(),
        compact_flag.clone(),
        todos.clone(),
        root_chk.context.clone(),
    );

    // Find the first unanswered ask; already-answered asks (crash-after-commit
    // window) resume directly with no re-prompt.
    let answer_opt = match parked.asks.iter().find(|a| !a.answered) {
        Some(ask) => {
            let idx = ask.checkpoint.parked.parked_index.expect("gate-kind");
            let call = &ask.checkpoint.parked.tool_calls[idx];
            let Some(intent) = built.loop_.derive_intent(&call.name, &call.args) else {
                eprintln!(
                    "session {session_id}: parked tool {} unavailable under current config; \
                     answer it after restoring the tool or start a new run",
                    call.name
                );
                agent_core::checkpoint::release_resume(&parked.root_dir);
                std::process::exit(2);
            };
            let who = match &ask.origin {
                Some(o) => format!("[sub-agent {} (depth {})] ", o.subagent_name, o.depth),
                None => String::new(),
            };
            let answer = approval::prompt_for_answer_with_reader(
                &intent.summary,
                &who,
                std::io::stdin().lock(),
            );
            if let Err(e) = agent_server::resume::commit_answer(ask, &answer, &key) {
                eprintln!("cannot commit answer: {e}");
                agent_core::checkpoint::release_resume(&parked.root_dir);
                std::process::exit(2);
            }
            if ask.subagent_path.is_empty() {
                Some(answer)
            } else {
                None
            }
        }
        None => None, // every ask already answered; crash-after-commit window
    };
    // I-3 (4B-2 branch review): mirror session.rs's `start_resume`, which
    // calls `take_answer(&root_dir, &key)` before building the resume turn.
    // When the ROOT park's answer was already committed (crash-after-commit
    // window, or a prior partial reopen), consume it here too — otherwise
    // `answer_opt` stays None and the tool re-asks a question that already
    // has a durable answer sitting on disk.
    let answer_opt =
        answer_opt.or_else(|| agent_core::checkpoint::take_answer(&parked.root_dir, &key));

    let resume = root_chk.resume_turn(answer_opt);
    let cancel = tokio_util::sync::CancellationToken::new();
    let run = built
        .loop_
        .resume_with_cancel(&mut ctx, resume, cancel.clone());
    tokio::pin!(run);
    let result = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                cancel.cancel();
                eprintln!("\n^C cancelling…");
            }
            r = &mut run => break r,
        }
    };
    // C-1 (4B-2 branch review): `resume_with_cancel`/`turn_loop` return
    // Ok(()) on cancellation too (background fact) — the Ok(()) arm
    // previously reaped unconditionally, destroying a park that the run
    // deliberately retained because it was cancelled while re-parked at a
    // NEW ask. Route through the same reap-vs-retain decision the fixed
    // server `start_resume` makes; skip the completed-run epilogue on the
    // cancelled-while-parked path and print an honest line instead.
    match resume_cleanup_decision(&result, cancel.is_cancelled()) {
        ResumeCleanup::Reap => {
            // Completed tree: mirror the server's delete-on-completion reap
            // (session.rs start_resume — `remove_dir_all(&root_dir)` on
            // success). Without this the root checkpoint dir (incl.
            // resume.lock) is orphaned on disk after a clean CLI resume.
            reap_root(&parked.root_dir);
            if let Ok(s) = stats.read() {
                eprintln!("\x1b[2m{}\x1b[0m", render::format_stats_line(&s));
            }
            std::process::exit(0);
        }
        ResumeCleanup::Retain => {
            match &result {
                Ok(()) => {
                    // Cancelled mid-resume: gate the "left parked" claim on
                    // an actual park existing on disk, mirroring the REPL's
                    // Ctrl-C handler (`has_park` check before printing it).
                    if agent_core::checkpoint::has_park(&parked.root_dir) {
                        eprintln!(
                            "run left parked; answer later with:\n  agent sessions reopen {session_id}"
                        );
                    } else {
                        eprintln!("run cancelled");
                    }
                }
                Err(e) => eprintln!("resumed run failed: {e}"),
            }
            agent_core::checkpoint::release_resume(&parked.root_dir);
            std::process::exit(if result.is_ok() { 0 } else { 1 });
        }
    }
}

/// Map CLI flags to a complete `RuntimeConfig` so the loop is assembled the same
/// way as the server (via `agent_runtime_config::assemble_loop`).
fn runtime_config_from_cli(cli: &Cli, protocol_name: &str) -> RuntimeConfig {
    let mut c = RuntimeConfig::from_launch(
        cli.backend.clone(),
        cli.base_url.clone(),
        cli.model.clone(),
        protocol_name.to_string(),
        cli.context_limit,
    );
    // Sandbox
    c.sandbox_mode = cli.sandbox_mode.clone();
    c.sandbox_image = cli.sandbox_image.clone();
    c.sandbox_network = cli.sandbox_network;
    c.sandbox_memory = cli.sandbox_memory.clone();
    c.sandbox_cpus = cli.sandbox_cpus.clone();
    c.sandbox_pids = cli.sandbox_pids;
    c.sandbox_fsize = cli.sandbox_fsize.clone();
    c.sandbox_tmp_size = cli.sandbox_tmp_size.clone();
    c.sandbox_extra_rw = cli.sandbox_extra_rw.clone();
    c.sandbox_extra_ro = cli.sandbox_extra_ro.clone();
    // Sampling + thinking (the values the CLI used to hardcode into LoopConfig)
    c.temperature = 0.2;
    c.max_turns = 25;
    c.max_tokens = 2048;
    c.top_p = cli.top_p;
    c.top_k = cli.top_k;
    c.min_p = cli.min_p;
    c.presence_penalty = cli.presence_penalty;
    c.repeat_penalty = cli.repeat_penalty;
    c.enable_thinking = !cli.no_thinking;
    c.preserve_thinking = cli.preserve_thinking;
    // claude-cli knobs: None = leave the runtime-config default untouched; Some overrides.
    if let Some(v) = cli.claude_session_reuse {
        c.claude_session_reuse = v;
    }
    if let Some(v) = cli.claude_effort.clone() {
        c.claude_effort = Some(v);
    }
    if let Some(v) = cli.claude_fallback_model.clone() {
        c.claude_fallback_model = Some(v);
    }
    // Tools / skills / memory / network
    c.http_allow_hosts = cli.allow_host.clone();
    c.skills_dirs = cli.skills_dir.clone();
    c.active_skills = cli.skill.clone();
    c.memory = cli.memory;
    c.command_allowlist = default_allowlist();
    c.command_denylist = default_denylist();
    c.trace_dir = cli.trace_dir.clone();
    c.metadata_dir = cli.metadata_dir.clone();
    c
}

#[derive(Parser)]
#[command(name = "agent", about = "Local Rust agent core (CLI)")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// OpenAI-compatible base URL (e.g. http://localhost:30000 for SGLang)
    #[arg(long, default_value = "http://localhost:30000")]
    base_url: String,
    /// Model name to request
    #[arg(long, default_value = "default")]
    model: String,
    /// Tool-call protocol: native | prompted
    #[arg(long, default_value = "native")]
    protocol: String,
    /// Inference backend: openai | claude-cli
    #[arg(long, default_value = "openai")]
    backend: String,
    /// Path/name of the Claude Code CLI binary (claude-cli backend only)
    #[arg(long, default_value = "claude")]
    claude_binary: String,
    /// Workspace directory the agent may operate in
    #[arg(long, default_value = ".")]
    workspace: String,
    /// Session artifacts root override (default: ~/.rusty-agent/sessions)
    #[arg(long)]
    trace_dir: Option<String>,
    /// Metadata root override — secret etc. (default: ~/.rusty-agent). E1 seam.
    #[arg(long)]
    metadata_dir: Option<String>,
    /// Approx context token limit
    #[arg(long, default_value_t = 8192)]
    context_limit: usize,
    /// Idle timeout (seconds) for model-stream consumption before a stalled turn fails
    #[arg(long, default_value_t = 120)]
    stream_timeout_secs: u64,
    /// Optional MCP server config (mcp.json shape). If absent, MCP is disabled.
    #[arg(long)]
    mcp_config: Option<std::path::PathBuf>,
    /// Host fetch_url may contact without approval (repeatable). Exact host or
    /// a leading-dot suffix, e.g. --allow-host docs.rs --allow-host .rust-lang.org
    #[arg(long = "allow-host")]
    allow_host: Vec<String>,
    /// Skill search directory (repeatable). Default: <workspace>/.rusty-agent/skills + ~/.rusty-agent/skills.
    #[arg(long = "skills-dir")]
    skills_dir: Vec<String>,
    /// Preload a skill as a preset by name (repeatable): its body is injected into the system prompt.
    #[arg(long = "skill")]
    skill: Vec<String>,
    /// Nucleus sampling (0.0–1.0); unset = server default
    #[arg(long)]
    top_p: Option<f32>,
    /// Top-k sampling; unset = server default
    #[arg(long)]
    top_k: Option<u32>,
    /// Min-p sampling (0.0–1.0); unset = server default
    #[arg(long)]
    min_p: Option<f32>,
    /// Presence penalty (-2.0–2.0); unset = server default
    #[arg(long)]
    presence_penalty: Option<f32>,
    /// Repetition penalty (>0.0); unset = server default
    #[arg(long)]
    repeat_penalty: Option<f32>,
    /// Disable model reasoning (chat_template_kwargs.enable_thinking=false)
    #[arg(long = "no-thinking", default_value_t = false)]
    no_thinking: bool,
    /// Keep prior-turn reasoning in conversation history. Auto-enabled whenever
    /// tools are registered (agentic runs need within-turn reasoning continuity),
    /// so this flag only matters for tool-less chat. OpenAI-compat backends get it
    /// as `reasoning_content` + chat_template_kwargs.preserve_thinking (Qwen3.6);
    /// claude_cli renders it inline as <think>. Note: backends that reject prior
    /// chain-of-thought in input (e.g. DeepSeek cloud) may error during tool use.
    #[arg(long, default_value_t = false)]
    preserve_thinking: bool,
    // ── Sandbox flags ──────────────────────────────────────────────────────
    /// Sandbox execution mode: off | auto | enforce
    #[arg(long, default_value = "auto")]
    sandbox_mode: String,
    /// Docker image used for sandboxed execution
    #[arg(long, default_value = DEFAULT_SANDBOX_IMAGE)]
    sandbox_image: String,
    /// Allow network access inside the sandbox
    #[arg(long, default_value_t = false)]
    sandbox_network: bool,
    /// Memory limit for the sandbox container (e.g. "2g")
    #[arg(long, default_value = "2g")]
    sandbox_memory: String,
    /// CPU quota for the sandbox container (e.g. "2")
    #[arg(long, default_value = "2")]
    sandbox_cpus: String,
    /// Max PIDs inside the sandbox container
    #[arg(long, default_value_t = 512u32)]
    sandbox_pids: u32,
    /// Max file size for writes inside the sandbox (e.g. "512m"); unset = no limit
    #[arg(long)]
    sandbox_fsize: Option<String>,
    /// Size of the tmpfs mounted at /tmp inside the sandbox (e.g. "1g")
    #[arg(long, default_value = "1g")]
    sandbox_tmp_size: String,
    /// Extra read-write bind-mount path inside the sandbox (repeatable)
    #[arg(long = "sandbox-extra-rw")]
    sandbox_extra_rw: Vec<String>,
    /// Extra read-only bind-mount path inside the sandbox (repeatable)
    #[arg(long = "sandbox-extra-ro")]
    sandbox_extra_ro: Vec<String>,
    // ── Memory flags ───────────────────────────────────────────────────────
    /// Enable project memory (the memories/project/ mount + pinned index block).
    #[arg(long, default_value_t = false)]
    memory: bool,
    // ── claude-cli backend knobs ───────────────────────────────────────────
    /// Enable or disable session reuse across claude-cli calls (delta resume).
    /// Omit to use the runtime-config default (on). Pass `false` to force
    /// stateless mode (useful for evals and CI where reproducibility matters
    /// more than per-round token savings). Ignored by the openai backend.
    #[arg(long)]
    claude_session_reuse: Option<bool>,
    /// Effort level passed to claude-cli via `--effort`: low | medium | high | xhigh | max.
    /// Omit to use the CLI default. Ignored by the openai backend.
    #[arg(long)]
    claude_effort: Option<String>,
    /// Fallback model name passed to claude-cli via `--fallback-model` when the
    /// primary model is unavailable. Omit to use no fallback. Ignored by the
    /// openai backend.
    #[arg(long)]
    claude_fallback_model: Option<String>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();

    if let Some(Command::Sessions { cmd }) = &cli.command {
        return run_sessions_cmd(cmd, &cli).await;
    }

    let workspace = std::fs::canonicalize(&cli.workspace)
        .unwrap_or_else(|_| std::path::PathBuf::from(&cli.workspace));

    if !backend_name_is_valid(&cli.backend) {
        eprintln!(
            "unknown --backend '{}': use openai | claude-cli",
            cli.backend
        );
        std::process::exit(2);
    }
    let api_key = std::env::var("AGENT_API_KEY").ok();
    // claude-cli is a pure text generator; tool calls must come via the prompted protocol.
    let protocol_name = select_protocol(&cli);
    // Map every loop-relevant flag into one RuntimeConfig, then assemble the loop
    // through the same shared builder the server uses (no duplicated orchestration).
    let rt = runtime_config_from_cli(&cli, protocol_name);
    if let Err(e) = rt.validate() {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
    for w in rt.warnings() {
        eprintln!("warning: {w}");
    }
    let model = build_model(
        &cli.backend,
        &cli.base_url,
        &cli.model,
        &cli.claude_binary,
        api_key.clone(),
        claude_cli_opts(&rt),
    );
    let sandbox = build_sandbox(&rt);

    // MCP servers (if configured): collect tools, keep the manager alive for the session.
    let mut mcp_tools: Vec<Arc<dyn agent_tools::Tool>> = Vec::new();
    let mcp_manager = match &cli.mcp_config {
        Some(path) => {
            let mgr = agent_runtime_config::connect_mcp(path, &workspace, sandbox.clone()).await;
            println!("{}", mgr.summary_line());
            mcp_tools = mgr.tools();
            Some(mgr)
        }
        None => None,
    };

    let artifacts = Arc::new(agent_core::SessionArtifacts::new());
    let compact_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let todos: agent_core::TodoHandle = Arc::new(std::sync::Mutex::new(Vec::new()));
    // Session-lifetime observability handles: created ONCE (a per-assemble
    // TraceWriter would interleave two writers into one file).
    let stats = Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default()));
    let session_id = agent_runtime_config::mint_session_id();
    if let Some(root) = agent_runtime_config::sessions_root(&rt) {
        let d = agent_runtime_config::SessionDescriptor {
            schema: agent_runtime_config::DESCRIPTOR_SCHEMA,
            session_id: session_id.clone(),
            workspace: workspace.clone(),
            created_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            config_path: None, // CLI config is flag-derived — no file provenance
        };
        if let Err(e) = agent_runtime_config::write_descriptor(&root, &d) {
            eprintln!("warning: cannot write session descriptor: {e}");
        }
        agent_runtime_config::prune_session_dirs(&root, 50);
    }
    let trace = agent_runtime_config::build_trace(&rt, &session_id);
    if let Some(t) = &trace {
        let dir = rt.trace_dir.as_deref().unwrap_or("~/.rusty-agent/sessions");
        eprintln!("\x1b[2mtrace: {}/{}.jsonl\x1b[0m", dir, t.session_id());
    }
    // Durable parks (4B-2): CLI runs park at Ask exactly like server runs.
    // Degrades to live-only (None) without HOME/secret — never fails boot.
    let checkpoint = agent_runtime_config::metadata_root_for(&rt)
        .and_then(|meta| agent_runtime_config::load_or_create_secret(&meta).ok())
        .and_then(|key| {
            let root = agent_runtime_config::sessions_root(&rt)?;
            Some(agent_core::Checkpointer::new(
                agent_runtime_config::session_dir(&root, &session_id).join("checkpoint"),
                key,
                session_id.clone(),
            ))
        });
    let approval = TerminalApproval::with_park_exit(checkpoint.as_ref().map(|_| ParkExit {
        session_id: session_id.clone(),
        trace: trace.clone(),
        release_lock: None, // ordinary runs hold no resume lock
        exit: Box::new(|code| std::process::exit(code)),
    }));
    let built = assemble_loop(
        &rt,
        LoopParts {
            model,
            sink: Arc::new(TerminalSink::default()),
            approval: Arc::new(approval),
            workspace: workspace.clone(),
            mcp_tools,
            stream_idle_timeout: Duration::from_secs(cli.stream_timeout_secs),
            base_system_prompt: BASE_SYSTEM_PROMPT.to_string(),
            artifacts: artifacts.clone(),
            compact_flag: compact_flag.clone(),
            todos: todos.clone(),
            sandbox: sandbox.clone(),
            stats: stats.clone(),
            trace,
            api_key: api_key.clone(),
            claude_binary: cli.claude_binary.clone(),
            checkpoint: checkpoint.clone(),
        },
    );
    if !built.unknown_presets.is_empty() {
        eprintln!(
            "skills: unknown active skill(s): {}",
            built.unknown_presets.join(", ")
        );
        std::process::exit(2);
    }
    let agent = built.loop_;

    let mut ctx = CuratedContext::new(
        Message::system(built.system_prompt),
        artifacts,
        compact_flag,
    )
    .with_offload_config(agent_core::OffloadConfig {
        max_result_bytes: rt.max_tool_result_bytes,
        ..Default::default()
    })
    .with_todos(todos);

    println!("agent ready. Type a task, or 'exit'.");
    let stdin = std::io::stdin();
    loop {
        print!("\n\x1b[1m›\x1b[0m ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if input == "exit" || input == "quit" {
            break;
        }
        let cancel = tokio_util::sync::CancellationToken::new();
        let run = agent.run_with_cancel(&mut ctx, input.to_string(), cancel.clone());
        tokio::pin!(run);
        let result = loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    cancel.cancel();
                    if checkpoint.as_ref().is_some_and(|c| agent_core::checkpoint::has_park(c.dir())) {
                        eprintln!("\n^C — a pending approval was left parked; answer later with:\n  agent sessions reopen {session_id}");
                    } else {
                        eprintln!("\n^C cancelling…");
                    }
                }
                r = &mut run => break r,
            }
        };
        if let Err(e) = result {
            eprintln!("\x1b[31mfatal: {e}\x1b[0m");
        }
        if let Ok(s) = stats.read() {
            eprintln!("\x1b[2m{}\x1b[0m", render::format_stats_line(&s));
        }
    }
    // Keep MCP manager alive for the entire REPL session (dropping it would kill server processes).
    let _ = &mcp_manager;
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn sessions_list_parses() {
        let cli = Cli::parse_from(["agent", "sessions", "list"]);
        assert!(matches!(
            cli.command,
            Some(Command::Sessions {
                cmd: SessionsCmd::List
            })
        ));
    }

    #[test]
    fn sessions_reopen_parses_with_id() {
        let cli = Cli::parse_from(["agent", "sessions", "reopen", "100-aaaaaaaa"]);
        assert!(matches!(
            cli.command,
            Some(Command::Sessions { cmd: SessionsCmd::Reopen { ref session_id } })
                if session_id == "100-aaaaaaaa"
        ));
    }

    #[test]
    fn bare_invocation_still_parses_with_no_subcommand() {
        let cli = Cli::parse_from(["agent"]);
        assert!(cli.command.is_none()); // REPL default unchanged
    }

    #[test]
    fn list_sessions_marks_parked() {
        let root = tempfile::tempdir().unwrap();
        for (id, ms) in [("100-aaaaaaaa", 1u64), ("200-bbbbbbbb", 2u64)] {
            agent_runtime_config::write_descriptor(
                root.path(),
                &agent_runtime_config::SessionDescriptor {
                    schema: agent_runtime_config::DESCRIPTOR_SCHEMA,
                    session_id: id.into(),
                    workspace: std::path::PathBuf::from("/w"),
                    created_ms: ms,
                    config_path: None,
                },
            )
            .unwrap();
        }
        let parked_dir =
            agent_runtime_config::session_dir(root.path(), "200-bbbbbbbb").join("checkpoint");
        std::fs::create_dir_all(&parked_dir).unwrap();
        std::fs::write(parked_dir.join("parked.json"), b"{}").unwrap();

        let rows = list_sessions(root.path());
        assert_eq!(rows[0].session_id, "200-bbbbbbbb");
        assert!(rows[0].parked);
        assert!(!rows[1].parked);
    }

    #[test]
    fn sandbox_mode_flag_parsed() {
        let cli = Cli::parse_from(["agent-cli", "--sandbox-mode", "enforce"]);
        assert_eq!(cli.sandbox_mode, "enforce");
    }

    #[test]
    fn sandbox_network_flag_parsed() {
        let cli = Cli::parse_from(["agent-cli", "--sandbox-network"]);
        assert!(cli.sandbox_network);
    }

    #[test]
    fn cli_sandbox_defaults_match_runtime_config_defaults() {
        // Audit 3.4: clap default_value attrs are hand-written mirrors of
        // runtime-config's default_sandbox_* fns (the documented clap-shadowing
        // gotcha class). from_launch is the canonical source; drift here means
        // a bumped server-side default silently leaves the CLI behind.
        let cli = Cli::parse_from(["agent-cli"]);
        let base = RuntimeConfig::from_launch(
            "openai".into(),
            "http://x".into(),
            "m".into(),
            "native".into(),
            8192,
        );
        assert_eq!(cli.sandbox_mode, base.sandbox_mode);
        assert_eq!(cli.sandbox_image, base.sandbox_image);
        assert_eq!(cli.sandbox_network, base.sandbox_network);
        assert_eq!(cli.sandbox_memory, base.sandbox_memory);
        assert_eq!(cli.sandbox_cpus, base.sandbox_cpus);
        assert_eq!(cli.sandbox_pids, base.sandbox_pids);
        assert_eq!(cli.sandbox_fsize, base.sandbox_fsize);
        assert_eq!(cli.sandbox_tmp_size, base.sandbox_tmp_size);
        assert_eq!(cli.sandbox_extra_rw, base.sandbox_extra_rw);
        assert_eq!(cli.sandbox_extra_ro, base.sandbox_extra_ro);
    }

    #[test]
    fn sandbox_repeatable_flags_parsed() {
        let cli = Cli::parse_from([
            "agent-cli",
            "--sandbox-extra-rw",
            "/data",
            "--sandbox-extra-rw",
            "/mnt",
            "--sandbox-extra-ro",
            "/etc/config",
        ]);
        assert_eq!(cli.sandbox_extra_rw, vec!["/data", "/mnt"]);
        assert_eq!(cli.sandbox_extra_ro, vec!["/etc/config"]);
    }

    #[test]
    fn cli_assembled_config_passes_validate() {
        // Guards the startup gate: default flags must never trip validate(),
        // or every plain `agent-cli` run would exit 2.
        let cli = Cli::parse_from(["agent-cli"]);
        let rc = runtime_config_from_cli(&cli, "prompted");
        assert!(rc.validate().is_ok(), "default CLI config must validate");
    }

    #[test]
    fn cli_bad_claude_effort_fails_validate() {
        let cli = Cli::parse_from(["agent-cli", "--claude-effort", "banana"]);
        let rc = runtime_config_from_cli(&cli, "prompted");
        let err = rc.validate().unwrap_err();
        assert!(err.contains("claude_effort"), "got: {err}");
    }

    #[test]
    fn cli_bad_sandbox_mode_fails_validate() {
        // Audit 3.3 pin: the gate itself shipped in claude-cli-followups
        // (rt.validate() + exit 2); this pins that a typo'd mode trips it.
        let cli = Cli::parse_from(["agent-cli", "--sandbox-mode", "enfore"]);
        let rc = runtime_config_from_cli(&cli, "prompted");
        let err = rc.validate().unwrap_err();
        assert!(err.contains("sandbox_mode"), "got: {err}");
    }

    #[test]
    fn claude_cli_knob_flags_absent_leave_defaults() {
        // Finding 2: absent flags must not override runtime-config defaults.
        let cli = Cli::parse_from(["agent-cli"]);
        let rc = runtime_config_from_cli(&cli, "prompted");
        // runtime-config default: session reuse on, no effort, no fallback
        assert!(rc.claude_session_reuse);
        assert!(rc.claude_effort.is_none());
        assert!(rc.claude_fallback_model.is_none());
    }

    #[test]
    fn claude_cli_knob_flags_present_override() {
        // Finding 2: present flags must override runtime-config values.
        let cli = Cli::parse_from([
            "agent-cli",
            "--claude-session-reuse",
            "false",
            "--claude-effort",
            "high",
            "--claude-fallback-model",
            "claude-haiku-4-5",
        ]);
        let rc = runtime_config_from_cli(&cli, "prompted");
        assert!(!rc.claude_session_reuse);
        assert_eq!(rc.claude_effort.as_deref(), Some("high"));
        assert_eq!(
            rc.claude_fallback_model.as_deref(),
            Some("claude-haiku-4-5")
        );
    }

    #[test]
    fn runtime_config_from_cli_carries_loop_fields() {
        let cli = Cli::parse_from([
            "agent",
            "--model",
            "m",
            "--base-url",
            "http://x",
            "--top-p",
            "0.9",
            "--allow-host",
            "example.com",
            "--skills-dir",
            "/sk",
            "--skill",
            "greeter",
            "--memory",
        ]);
        let rc = runtime_config_from_cli(&cli, "native");
        assert_eq!(rc.model, "m");
        assert_eq!(rc.base_url, "http://x");
        assert_eq!(rc.protocol, "native");
        assert_eq!(rc.top_p, Some(0.9));
        assert!(rc.memory);
        assert_eq!(rc.http_allow_hosts, vec!["example.com".to_string()]);
        assert_eq!(rc.skills_dirs, vec!["/sk".to_string()]);
        assert_eq!(rc.active_skills, vec!["greeter".to_string()]);
        assert!(!rc.command_allowlist.is_empty());
        assert!(!rc.command_denylist.is_empty());
    }

    #[test]
    fn reap_root_removes_populated_checkpoint_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root_dir = tmp.path().join("checkpoint");
        std::fs::create_dir_all(&root_dir).unwrap();
        std::fs::write(root_dir.join("root.json"), b"{}").unwrap();
        std::fs::write(root_dir.join("resume.lock"), b"").unwrap();
        assert!(root_dir.join("resume.lock").exists());

        reap_root(&root_dir);

        assert!(!root_dir.exists(), "checkpoint dir should be gone");
    }

    #[test]
    fn reap_root_is_best_effort_on_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        // Must not panic even though there's nothing to remove.
        reap_root(&missing);
    }

    // ---- C-1 (4B-2 branch review): cancelled resumed runs retain their park ----

    #[test]
    fn resume_cleanup_decision_reaps_on_true_completion() {
        let ok: Result<(), String> = Ok(());
        assert_eq!(resume_cleanup_decision(&ok, false), ResumeCleanup::Reap);
    }

    #[test]
    fn resume_cleanup_decision_retains_on_cancelled_ok() {
        // The background fact this fix guards: `turn_loop` returns Ok(()) on
        // cancellation too, so Ok(()) alone must never be read as "reap".
        let ok: Result<(), String> = Ok(());
        assert_eq!(resume_cleanup_decision(&ok, true), ResumeCleanup::Retain);
    }

    #[test]
    fn resume_cleanup_decision_retains_on_err_regardless_of_cancel_flag() {
        let err: Result<(), String> = Err("boom".into());
        assert_eq!(resume_cleanup_decision(&err, false), ResumeCleanup::Retain);
        assert_eq!(resume_cleanup_decision(&err, true), ResumeCleanup::Retain);
    }

    // ---- I-2 (4B-2 branch review): reopen derives the same protocol as run ----

    #[test]
    fn select_protocol_matches_run_path_for_native_backend() {
        let cli = Cli::parse_from(["agent-cli", "--backend", "openai", "--protocol", "native"]);
        assert_eq!(select_protocol(&cli), "native");
    }

    #[test]
    fn select_protocol_defaults_to_native_for_openai_backend() {
        let cli = Cli::parse_from(["agent-cli", "--backend", "openai"]);
        assert_eq!(select_protocol(&cli), "native");
    }

    #[test]
    fn select_protocol_forces_prompted_for_claude_cli_backend() {
        let cli = Cli::parse_from(["agent-cli", "--backend", "claude-cli"]);
        assert_eq!(select_protocol(&cli), "prompted");

        // Even an explicit --protocol native is overridden (mirrors the run
        // path's forcing block exactly).
        let cli2 = Cli::parse_from([
            "agent-cli",
            "--backend",
            "claude-cli",
            "--protocol",
            "native",
        ]);
        assert_eq!(select_protocol(&cli2), "prompted");
    }

    // ---- E1 (e2e lifecycle seam): --trace-dir/--metadata-dir map into RuntimeConfig ----

    #[test]
    fn cli_dirs_flags_map_into_runtime_config() {
        let cli = Cli::parse_from(["agent", "--trace-dir", "/tmp/s", "--metadata-dir", "/tmp/m"]);
        let rt = runtime_config_from_cli(&cli, "native");
        assert_eq!(rt.trace_dir.as_deref(), Some("/tmp/s"));
        assert_eq!(rt.metadata_dir.as_deref(), Some("/tmp/m"));
    }

    // ---- I-3 (4B-2 branch review): reopen consumes a pre-committed root answer ----

    #[test]
    fn take_answer_consumes_a_precommitted_root_answer_and_removes_it() {
        // Mirrors the shape of the reopen arm's I-3 fix: a root park exists,
        // an answer was already durably committed (crash-after-commit /
        // prior partial reopen window), and `answer_opt` falls back to
        // `take_answer` instead of staying None and re-asking live.
        let tmp = tempfile::tempdir().unwrap();
        let root_dir = tmp.path().join("checkpoint");
        let key = [9u8; 32];
        let ck = agent_core::Checkpointer::new(root_dir.clone(), key, "s1".into());

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let chk = agent_core::checkpoint::Checkpoint {
                version: agent_core::checkpoint::CHECKPOINT_VERSION,
                session_id: "s1".into(),
                subagent_path: vec![],
                turn: 0,
                context: agent_core::CuratedContextState {
                    goal: None,
                    history: vec![],
                    compaction_summary: None,
                    folded_facts: vec![],
                    folded_sections: vec![],
                    seq: 0,
                    history_has_spans: false,
                    history_incomplete: false,
                    artifact_prefix: String::new(),
                    todos: vec![],
                },
                guardrails: agent_core::checkpoint::Guardrails {
                    tool_calls: 0,
                    model_calls: 0,
                },
                parked: agent_core::checkpoint::ParkedTurn {
                    assistant_text: "running".into(),
                    tool_calls: vec![agent_tools::ToolCall {
                        id: "c1".into(),
                        name: "execute_command".into(),
                        args: serde_json::json!({"command": "echo hi"}),
                    }],
                    invalid: vec![],
                    gate_records: vec![],
                    parked_index: Some(0),
                    origin: None,
                },
            };
            ck.write_park(chk, &agent_core::SessionArtifacts::new())
                .await
                .unwrap();
        });

        // Pre-commit the root answer, same call session.rs's live-answer path
        // makes (`agent_server::resume::commit_answer` bottoms out here too).
        agent_core::checkpoint::write_answer(&root_dir, &key, true, Some("go ahead")).unwrap();
        assert!(root_dir.join("answer.json").exists());

        // The reopen arm's fallback: answer_opt.or_else(|| take_answer(..)).
        let answer_opt: Option<agent_core::checkpoint::ParkedAnswer> = None;
        let answer_opt =
            answer_opt.or_else(|| agent_core::checkpoint::take_answer(&root_dir, &key));

        assert_eq!(
            answer_opt,
            Some(agent_core::checkpoint::ParkedAnswer {
                approve: true,
                feedback: Some("go ahead".into()),
            }),
            "pre-committed root answer must be threaded through as Some(..)"
        );
        assert!(
            !root_dir.join("answer.json").exists(),
            "take_answer must consume (delete) the answer file"
        );
    }
}
