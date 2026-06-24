mod approval;
mod render;

use agent_core::{AgentLoop, LoopConfig, WindowContext};
use agent_model::Message;
use agent_policy::RulePolicy;
use agent_runtime_config::{backend_name_is_valid, build_registry, build_model, build_skills,
    default_allowlist, default_denylist, pick_protocol};
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
    /// Keep prior <think> reasoning in conversation history
    #[arg(long, default_value_t = false)]
    preserve_thinking: bool,
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
    let protocol = pick_protocol(protocol_name);
    let mut registry = build_registry(&cli.allow_host);
    // Connect MCP servers (if configured), register their tools, keep the manager alive.
    let mcp_manager = match &cli.mcp_config {
        Some(path) => {
            let mgr = agent_runtime_config::connect_mcp(path).await;
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
            sandbox: None,
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
