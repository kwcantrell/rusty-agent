mod approval;
mod render;

use agent_core::CuratedContext;
use agent_model::Message;
use agent_runtime_config::{assemble_loop, backend_name_is_valid, build_memory_full, build_model,
    build_sandbox, default_allowlist, default_denylist, LoopParts, RuntimeConfig};
use approval::TerminalApproval;
use clap::Parser;
use render::TerminalSink;
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

const BASE_SYSTEM_PROMPT: &str = "You are a local coding agent. Use the provided tools to \
inspect and modify the workspace. Think step by step. When the task is complete, reply with a \
summary and no tool call.";

/// Map CLI flags to a complete `RuntimeConfig` so the loop is assembled the same
/// way as the server (via `agent_runtime_config::assemble_loop`).
fn runtime_config_from_cli(cli: &Cli, protocol_name: &str) -> RuntimeConfig {
    let mut c = RuntimeConfig::from_launch(
        cli.backend.clone(), cli.base_url.clone(), cli.model.clone(),
        protocol_name.to_string(), cli.context_limit);
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
    // Tools / skills / memory / network
    c.http_allow_hosts = cli.allow_host.clone();
    c.skills_dirs = cli.skills_dir.clone();
    c.active_skills = cli.skill.clone();
    c.memory = cli.memory;
    c.command_allowlist = default_allowlist();
    c.command_denylist = default_denylist();
    c
}

#[derive(Parser)]
#[command(name = "agent", about = "Local Rust agent core (CLI)")]
struct Cli {
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
    /// Skill search directory (repeatable). Default: <workspace>/.agent/skills + ~/.agent/skills.
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
    #[arg(long, default_value = "debian:stable-slim")]
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
    /// Size of the tmpfs mounted at /tmp inside the sandbox (e.g. "256m")
    #[arg(long, default_value = "256m")]
    sandbox_tmp_size: String,
    /// Extra read-write bind-mount path inside the sandbox (repeatable)
    #[arg(long = "sandbox-extra-rw")]
    sandbox_extra_rw: Vec<String>,
    /// Extra read-only bind-mount path inside the sandbox (repeatable)
    #[arg(long = "sandbox-extra-ro")]
    sandbox_extra_ro: Vec<String>,
    // ── Memory flags ───────────────────────────────────────────────────────
    /// Enable long-term memory (remember/recall/forget tools).
    #[arg(long, default_value_t = false)]
    memory: bool,
    /// Override the memory DB path (default ~/.agent/memory.db).
    #[arg(long)]
    memory_db: Option<std::path::PathBuf>,
    /// Override the embedding-model cache dir.
    #[arg(long)]
    memory_model_dir: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    let workspace = std::fs::canonicalize(&cli.workspace)
        .unwrap_or_else(|_| std::path::PathBuf::from(&cli.workspace));

    if !backend_name_is_valid(&cli.backend) {
        eprintln!("unknown --backend '{}': use openai | claude-cli", cli.backend);
        std::process::exit(2);
    }
    let api_key = std::env::var("AGENT_API_KEY").ok();
    let model = build_model(&cli.backend, &cli.base_url, &cli.model, &cli.claude_binary, api_key);
    // claude-cli is a pure text generator; tool calls must come via the prompted protocol.
    let protocol_name = if cli.backend == "claude-cli" {
        if cli.protocol != "prompted" {
            eprintln!("note: forcing --protocol prompted for claude-cli backend");
        }
        "prompted"
    } else {
        cli.protocol.as_str()
    };
    // Map every loop-relevant flag into one RuntimeConfig, then assemble the loop
    // through the same shared builder the server uses (no duplicated orchestration).
    let rt = runtime_config_from_cli(&cli, protocol_name);
    let sandbox = build_sandbox(&rt);

    // MCP servers (if configured): collect tools, keep the manager alive for the session.
    let mut mcp_tools: Vec<Arc<dyn agent_tools::Tool>> = Vec::new();
    let mcp_manager = match &cli.mcp_config {
        Some(path) => {
            let mgr = agent_runtime_config::connect_mcp(path, sandbox.clone()).await;
            println!("{}", mgr.summary_line());
            mcp_tools = mgr.tools();
            Some(mgr)
        }
        None => None,
    };

    // Long-term memory: construct once (loads the embedding model); pass tools + retriever in.
    let memory = build_memory_full(cli.memory, cli.memory_db.clone(),
        cli.memory_model_dir.clone(), &workspace);

    let offload_store: Arc<dyn agent_core::OffloadStore> =
        Arc::new(agent_core::InMemoryOffloadStore::new());
    let compact_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Session-lifetime observability handles: created ONCE (a per-assemble
    // TraceWriter would mint a colliding {epoch}-{pid} session id).
    let stats = Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default()));
    let trace = agent_runtime_config::build_trace(&rt);
    if let Some(t) = &trace {
        let dir = rt.trace_dir.as_deref().unwrap_or("~/.agent/sessions");
        eprintln!("\x1b[2mtrace: {}/{}.jsonl\x1b[0m", dir, t.session_id());
    }
    let built = assemble_loop(&rt, LoopParts {
        model,
        sink: Arc::new(TerminalSink::default()),
        approval: Arc::new(TerminalApproval::default()),
        workspace: workspace.clone(),
        mcp_tools,
        memory_tools: memory.tools.clone(),
        memory_retriever: memory.retriever.clone(),
        stream_idle_timeout: Duration::from_secs(cli.stream_timeout_secs),
        base_system_prompt: BASE_SYSTEM_PROMPT.to_string(),
        offload_store: offload_store.clone(),
        compact_flag: compact_flag.clone(),
        stats: stats.clone(),
        trace,
    });
    if !built.unknown_presets.is_empty() {
        eprintln!("skills: unknown active skill(s): {}", built.unknown_presets.join(", "));
        std::process::exit(2);
    }
    let agent = built.loop_;

    let mut ctx = CuratedContext::new(
        Message::system(built.system_prompt),
        offload_store,
        compact_flag,
    )
    .with_recall_budget(memory.recall_token_budget);

    println!("agent ready. Type a task, or 'exit'.");
    let stdin = std::io::stdin();
    loop {
        print!("\n\x1b[1m›\x1b[0m ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 { break; }
        let input = line.trim();
        if input.is_empty() { continue; }
        if input == "exit" || input == "quit" { break; }
        let cancel = tokio_util::sync::CancellationToken::new();
        let run = agent.run_with_cancel(&mut ctx, input.to_string(), cancel.clone());
        tokio::pin!(run);
        let result = loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => { cancel.cancel(); eprintln!("\n^C cancelling…"); }
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
    fn sandbox_defaults() {
        let cli = Cli::parse_from(["agent-cli"]);
        assert_eq!(cli.sandbox_mode, "auto");
        assert_eq!(cli.sandbox_image, "debian:stable-slim");
        assert!(!cli.sandbox_network);
        assert_eq!(cli.sandbox_memory, "2g");
        assert_eq!(cli.sandbox_cpus, "2");
        assert_eq!(cli.sandbox_pids, 512u32);
        assert!(cli.sandbox_fsize.is_none());
        assert_eq!(cli.sandbox_tmp_size, "256m");
        assert!(cli.sandbox_extra_rw.is_empty());
        assert!(cli.sandbox_extra_ro.is_empty());
    }

    #[test]
    fn sandbox_repeatable_flags_parsed() {
        let cli = Cli::parse_from([
            "agent-cli",
            "--sandbox-extra-rw", "/data",
            "--sandbox-extra-rw", "/mnt",
            "--sandbox-extra-ro", "/etc/config",
        ]);
        assert_eq!(cli.sandbox_extra_rw, vec!["/data", "/mnt"]);
        assert_eq!(cli.sandbox_extra_ro, vec!["/etc/config"]);
    }

    #[test]
    fn runtime_config_from_cli_carries_loop_fields() {
        let cli = Cli::parse_from([
            "agent", "--model", "m", "--base-url", "http://x",
            "--top-p", "0.9", "--allow-host", "example.com",
            "--skills-dir", "/sk", "--skill", "greeter", "--memory",
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
}
