use agent_runtime_config::{backend_name_is_valid, RuntimeConfig};
use agent_server::config::{ws_url, DaemonConfig};
use agent_server::{config, daemon};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "agent-serverd", about = "Local agent daemon (Cloudflare control plane)")]
struct Cli {
    /// Path to the persisted enrollment config.
    #[arg(long, default_value = "agent-server.json")]
    config: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Register this daemon with the Worker and store credentials.
    Enroll {
        #[arg(long, default_value = "http://localhost:8787")]
        worker_url: String,
        #[arg(long, env = "AGENT_BOOTSTRAP_SECRET")]
        bootstrap_secret: String,
        #[arg(long, default_value = "local-dev")]
        name: String,
    },
    /// Connect to the Worker and serve the agent over WebSocket.
    Run {
        #[arg(long, default_value = "http://localhost:30000")]
        base_url: String,
        #[arg(long, default_value = "default")]
        model: String,
        #[arg(long, default_value = "native")]
        protocol: String,
        #[arg(long, default_value = ".")]
        workspace: String,
        #[arg(long, default_value_t = 8192)]
        context_limit: usize,
        #[arg(long, default_value = "openai")]
        backend: String,
        #[arg(long, default_value = "claude")]
        claude_binary: String,
        /// Path to the persisted runtime config (live settings). Seeded from the flags
        /// above; overlaid by this file if present.
        #[arg(long, default_value = "agent-runtime.json")]
        runtime_config: PathBuf,
        /// Optional MCP server config (mcp.json shape). If absent, MCP is disabled.
        #[arg(long)]
        mcp_config: Option<PathBuf>,
        /// Host fetch_url may contact without approval (repeatable); overlaid by the runtime config file.
        #[arg(long = "allow-host")]
        allow_host: Vec<String>,
        /// Skill search directory (repeatable). Default: <workspace>/.agent/skills + ~/.agent/skills.
        #[arg(long = "skills-dir")]
        skills_dir: Vec<String>,
        /// Preload a skill as a preset by name (repeatable).
        #[arg(long = "skill")]
        skill: Vec<String>,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Enroll { worker_url, bootstrap_secret, name } => {
            match config::enroll(&worker_url, &bootstrap_secret, &name).await {
                Ok(cfg) => {
                    cfg.save(&cli.config).expect("write config");
                    println!("enrolled. agent_id={}", cfg.agent_id);
                    println!("pairing code (give this to the browser): {}", cfg.pairing_code);
                    println!("config written to {}", cli.config.display());
                }
                Err(e) => { eprintln!("enroll failed: {e}"); std::process::exit(1); }
            }
        }
        Cmd::Run { base_url, model, protocol, workspace, context_limit, backend, claude_binary,
                   runtime_config, mcp_config, allow_host, skills_dir, skill } => {
            let cfg = DaemonConfig::load(&cli.config)
                .expect("load config (run `enroll` first)");
            println!("pairing code: {}", cfg.pairing_code);
            let workspace = std::fs::canonicalize(&workspace)
                .unwrap_or_else(|_| PathBuf::from(&workspace));
            if !backend_name_is_valid(&backend) {
                eprintln!("unknown --backend '{backend}': use openai | claude-cli");
                std::process::exit(2);
            }
            let api_key = std::env::var("AGENT_API_KEY").ok();
            let mut base = RuntimeConfig::from_launch(backend, base_url, model, protocol, context_limit);
            base.http_allow_hosts = allow_host;
            // Surface bad flags early (the persisted file is only ever written post-validation).
            if let Err(e) = base.clone().normalized().validate() {
                eprintln!("invalid launch config: {e}");
                std::process::exit(2);
            }
            // Connect MCP once at process start; the manager owns server processes for the
            // full lifetime of the binary (across all WebSocket reconnects below).
            let mcp_manager = match &mcp_config {
                Some(path) => {
                    let mgr = agent_runtime_config::connect_mcp(path).await;
                    println!("{}", mgr.summary_line());
                    Some(mgr)
                }
                None => None,
            };
            let mcp_tools: std::sync::Arc<[std::sync::Arc<dyn agent_tools::Tool>]> =
                mcp_manager.as_ref().map(|m| std::sync::Arc::from(m.tools()))
                    .unwrap_or_else(|| std::sync::Arc::from(Vec::<std::sync::Arc<dyn agent_tools::Tool>>::new()));
            // Skills: build the shared registry + tools, fold the tools into the
            // same slice build_loop already registers, and compose presets into the prompt.
            let (skill_registry, skill_tools) =
                agent_runtime_config::build_skills(&skills_dir, &workspace);
            let mut all_tools: Vec<std::sync::Arc<dyn agent_tools::Tool>> = mcp_tools.to_vec();
            all_tools.extend(skill_tools);
            let extra_tools: std::sync::Arc<[std::sync::Arc<dyn agent_tools::Tool>]> =
                std::sync::Arc::from(all_tools);
            let system_prompt = match agent_skills::compose_system_prompt(
                daemon::SYSTEM_PROMPT, &skill_registry, &skill) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("skills: {e}");
                    std::process::exit(2);
                }
            };
            let params = daemon::DaemonParams {
                ws_url: ws_url(&cfg.worker_url),
                agent_token: cfg.agent_token,
                config: base,
                api_key,
                claude_binary,
                config_path: runtime_config,
                workspace,
                system_prompt,
                mcp_tools: extra_tools,
            };
            // Reconnect with simple backoff.
            let mut backoff = 1u64;
            loop {
                match daemon::run(params_clone(&params)).await {
                    Ok(()) => {
                        backoff = 1;
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "daemon disconnected");
                        tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                        backoff = (backoff * 2).min(30);
                    }
                }
            }
        }
    }
}

// DaemonParams holds a RuntimeConfig + plain fields; clone by hand for reconnect.
fn params_clone(p: &daemon::DaemonParams) -> daemon::DaemonParams {
    daemon::DaemonParams {
        ws_url: p.ws_url.clone(),
        agent_token: p.agent_token.clone(),
        config: p.config.clone(),
        api_key: p.api_key.clone(),
        claude_binary: p.claude_binary.clone(),
        config_path: p.config_path.clone(),
        workspace: p.workspace.clone(),
        system_prompt: p.system_prompt.clone(),
        mcp_tools: p.mcp_tools.clone(),
    }
}
