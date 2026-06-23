use agent_server::daemon;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::from_default_env()).init();
    // Full CLI wiring is added in Task 5; this stub keeps the bin compiling.
    let _ = &daemon::run;
    eprintln!("use the `enroll` / `run` subcommands (added in Task 5)");
}
