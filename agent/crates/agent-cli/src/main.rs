mod approval;
mod render;

use agent_core::CuratedContext;
use agent_model::Message;
use agent_runtime_config::{
    assemble_loop, backend_name_is_valid, build_model, build_sandbox, claude_cli_opts,
    default_allowlist, default_denylist, LoopParts, RuntimeConfig, BASE_SYSTEM_PROMPT,
    DEFAULT_SANDBOX_IMAGE,
};
use approval::TerminalApproval;
use clap::Parser;
use render::TerminalSink;
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

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
    // TraceWriter would mint a colliding {epoch}-{pid} session id).
    let stats = Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default()));
    let session_id = agent_runtime_config::mint_session_id();
    let trace = agent_runtime_config::build_trace(&rt, &session_id);
    if let Some(t) = &trace {
        let dir = rt.trace_dir.as_deref().unwrap_or("~/.rusty-agent/sessions");
        eprintln!("\x1b[2mtrace: {}/{}.jsonl\x1b[0m", dir, t.session_id());
    }
    let built = assemble_loop(
        &rt,
        LoopParts {
            model,
            sink: Arc::new(TerminalSink::default()),
            approval: Arc::new(TerminalApproval::default()),
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
}
