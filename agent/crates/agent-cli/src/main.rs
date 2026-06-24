mod approval;
mod render;

use agent_core::{AgentLoop, LoopConfig, WindowContext};
use agent_model::Message;
use agent_policy::RulePolicy;
use agent_runtime_config::{backend_name_is_valid, build_memory, build_registry, build_model,
    build_sandbox, build_skills, default_allowlist, default_denylist, pick_protocol};
use approval::TerminalApproval;
use clap::Parser;
use render::TerminalSink;
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

const BASE_SYSTEM_PROMPT: &str = "You are a local coding agent. Use the provided tools to \
inspect and modify the workspace. Think step by step. When the task is complete, reply with a \
summary and no tool call.";

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
    /// Keep prior-turn reasoning in conversation history. OpenAI-compat backends
    /// receive it as `reasoning_content` + chat_template_kwargs.preserve_thinking
    /// (Qwen3.6); claude_cli renders it inline as <think>. Backends that reject
    /// prior chain-of-thought (e.g. DeepSeek) should leave this off.
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
    // Build the sandbox strategy early so it can be passed to both LoopConfig and (in Task 11)
    // the MCP manager. We synthesise a RuntimeConfig purely as a carrier for the sandbox fields.
    let mut sbcfg = agent_runtime_config::RuntimeConfig::from_launch(
        cli.backend.clone(), cli.base_url.clone(), cli.model.clone(),
        protocol_name.to_string(), cli.context_limit);
    sbcfg.sandbox_mode = cli.sandbox_mode.clone();
    sbcfg.sandbox_image = cli.sandbox_image.clone();
    sbcfg.sandbox_network = cli.sandbox_network;
    sbcfg.sandbox_memory = cli.sandbox_memory.clone();
    sbcfg.sandbox_cpus = cli.sandbox_cpus.clone();
    sbcfg.sandbox_pids = cli.sandbox_pids;
    sbcfg.sandbox_fsize = cli.sandbox_fsize.clone();
    sbcfg.sandbox_tmp_size = cli.sandbox_tmp_size.clone();
    sbcfg.sandbox_extra_rw = cli.sandbox_extra_rw.clone();
    sbcfg.sandbox_extra_ro = cli.sandbox_extra_ro.clone();
    let sandbox = build_sandbox(&sbcfg);
    let protocol = pick_protocol(protocol_name);
    let mut registry = build_registry(&cli.allow_host);
    // Connect MCP servers (if configured), register their tools, keep the manager alive.
    let mcp_manager = match &cli.mcp_config {
        Some(path) => {
            let mgr = agent_runtime_config::connect_mcp(path, sandbox.clone()).await;
            println!("{}", mgr.summary_line());
            for t in mgr.tools() {
                registry.register(t);
            }
            Some(mgr)
        }
        None => None,
    };
    // Skills: register the four skill tools, then compose any presets into the system prompt.
    let (skill_registry, skill_tools) = build_skills(&cli.skills_dir, &workspace);
    for t in skill_tools {
        registry.register(t);
    }
    // Long-term memory: construct once (loads the embedding model) and register.
    for t in build_memory(cli.memory, cli.memory_db.clone(), cli.memory_model_dir.clone(), &workspace) {
        registry.register(t);
    }
    let system_prompt = match agent_skills::compose_system_prompt(
        BASE_SYSTEM_PROMPT, &skill_registry, &cli.skill) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("skills: {e}");
            std::process::exit(2);
        }
    };
    let tools = Arc::new(registry);
    let policy = Arc::new(RulePolicy {
        workspace: workspace.clone(),
        command_allowlist: default_allowlist(),
        command_denylist: default_denylist(),
    });
    let sink = Arc::new(TerminalSink::default());
    let agent = AgentLoop::new(model, protocol, tools, policy, Arc::new(TerminalApproval),
        sink, LoopConfig {
            model_limit: cli.context_limit, max_turns: 25, max_retries: 3, temperature: 0.2,
            max_tokens: Some(2048), workspace, tool_timeout: Duration::from_secs(120),
            stream_idle_timeout: Duration::from_secs(cli.stream_timeout_secs),
            top_p: cli.top_p, top_k: cli.top_k, min_p: cli.min_p,
            presence_penalty: cli.presence_penalty, repeat_penalty: cli.repeat_penalty,
            enable_thinking: !cli.no_thinking, preserve_thinking: cli.preserve_thinking,
            sandbox: Some(sandbox.clone()),
        });

    let mut ctx = WindowContext::new(Message::system(system_prompt));

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
        if let Err(e) = agent.run(&mut ctx, input.to_string()).await {
            eprintln!("\x1b[31mfatal: {e}\x1b[0m");
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
}
