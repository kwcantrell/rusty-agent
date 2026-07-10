//! Build a `DaemonParams` for the fully-local desktop bridge (no Worker, no MCP).
//! Llama defaults are seeded here; the Settings UI can still edit them live via the
//! persisted `config_path`.
use crate::daemon::{DaemonParams, SYSTEM_PROMPT};
use agent_runtime_config::RuntimeConfig;
use std::path::PathBuf;
use std::sync::Arc;

pub fn local_params(
    workspace: PathBuf,
    config_path: PathBuf,
    base_url: String,
    model: String,
) -> DaemonParams {
    let mut config =
        RuntimeConfig::from_launch("openai".into(), base_url, model, "native".into(), 262_144);
    config.preserve_thinking = true;
    config.enable_thinking = true;

    DaemonParams {
        config,
        api_key: std::env::var("AGENT_API_KEY").ok(),
        claude_binary: "claude".into(),
        config_path,
        workspace,
        system_prompt: SYSTEM_PROMPT.to_string(),
        mcp_tools: Arc::from(Vec::<Arc<dyn agent_tools::Tool>>::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_params_seeds_llama_defaults() {
        let p = local_params(
            PathBuf::from("/tmp/ws"),
            PathBuf::from("/tmp/agent-runtime.json"),
            "http://localhost:8080".into(),
            "qwen3.6-35b-a3b".into(),
        );
        assert_eq!(p.config.backend, "openai");
        assert_eq!(p.config.base_url, "http://localhost:8080");
        assert_eq!(p.config.model, "qwen3.6-35b-a3b");
        assert_eq!(p.config.protocol, "native");
        assert!(p.config.preserve_thinking);
        assert_eq!(p.workspace, PathBuf::from("/tmp/ws"));
        assert!(p.mcp_tools.is_empty());
    }
}
