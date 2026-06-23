mod approval;
mod render;

use agent_core::{AgentLoop, LoopConfig, WindowContext};
use agent_model::Message;
use agent_policy::RulePolicy;
use agent_runtime_config::{backend_name_is_valid, build_registry, build_model,
    default_allowlist, default_denylist, pick_protocol};
use approval::TerminalApproval;
use clap::Parser;
use render::TerminalSink;
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::time::Duration;

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
    let tools = Arc::new(build_registry());
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
        });

    let mut ctx = WindowContext::new(Message::system(
        "You are a local coding agent. Use the provided tools to inspect and modify the \
         workspace. Think step by step. When the task is complete, reply with a summary and \
         no tool call."));

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
}
