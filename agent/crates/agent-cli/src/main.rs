mod approval;
mod render;

use agent_core::{AgentLoop, LoopConfig, WindowContext};
use agent_model::{Message, OpenAiCompatClient};
use agent_policy::RulePolicy;
use agent_runtime_config::{build_registry, default_allowlist, default_denylist, pick_protocol};
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
    /// Workspace directory the agent may operate in
    #[arg(long, default_value = ".")]
    workspace: String,
    /// Approx context token limit
    #[arg(long, default_value_t = 8192)]
    context_limit: usize,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    let workspace = std::fs::canonicalize(&cli.workspace)
        .unwrap_or_else(|_| std::path::PathBuf::from(&cli.workspace));

    let api_key = std::env::var("AGENT_API_KEY").ok();
    let model = Arc::new(OpenAiCompatClient::new(cli.base_url.clone(), cli.model.clone(), api_key));
    let protocol = pick_protocol(&cli.protocol);
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
