//! Build a `DaemonParams` for the fully-local desktop bridge (no Worker, no MCP).
//! Llama defaults are seeded here; the Settings UI can still edit them live via the
//! persisted `config_path`. Memory is assembled per connection from a `MemoryParts`
//! that the bridge loads once at startup (or `None` when memory is off/unavailable).
use crate::daemon::{DaemonParams, SYSTEM_PROMPT};
use agent_memory::{assemble_memory, MemoryParts};
use agent_runtime_config::RuntimeConfig;
use std::path::PathBuf;
use std::sync::Arc;

pub fn local_params(
    workspace: PathBuf,
    config_path: PathBuf,
    base_url: String,
    model: String,
    memory_parts: Option<&MemoryParts>,
) -> DaemonParams {
    let mut config =
        RuntimeConfig::from_launch("openai".into(), base_url, model, "native".into(), 262_144);
    config.preserve_thinking = true;
    config.enable_thinking = true;

    let (memory_tools, memory_retriever, recall_token_budget) = match memory_parts {
        Some(parts) => {
            let (tools, retriever) = assemble_memory(parts, &workspace);
            (
                Arc::from(tools),
                Some(retriever),
                parts.cfg.recall_token_budget,
            )
        }
        None => (
            Arc::from(Vec::<Arc<dyn agent_tools::Tool>>::new()),
            None,
            512, // matches MemoryConfig::default().recall_token_budget
        ),
    };

    DaemonParams {
        config,
        api_key: std::env::var("AGENT_API_KEY").ok(),
        claude_binary: "claude".into(),
        config_path,
        workspace,
        system_prompt: SYSTEM_PROMPT.to_string(),
        mcp_tools: Arc::from(Vec::<Arc<dyn agent_tools::Tool>>::new()),
        memory_tools,
        memory_retriever,
        recall_token_budget,
        memory_parts: memory_parts.cloned(),
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
            None,
        );
        assert_eq!(p.config.backend, "openai");
        assert_eq!(p.config.base_url, "http://localhost:8080");
        assert_eq!(p.config.model, "qwen3.6-35b-a3b");
        assert_eq!(p.config.protocol, "native");
        assert!(p.config.preserve_thinking);
        assert_eq!(p.workspace, PathBuf::from("/tmp/ws"));
        assert!(p.mcp_tools.is_empty());
        assert!(p.memory_tools.is_empty());
        assert!(p.memory_retriever.is_none());
        assert!(p.memory_parts.is_none());
    }

    #[test]
    fn local_params_with_parts_populates_memory() {
        use agent_memory::{Embedder, InMemoryStore, MemoryConfig, MemoryStore, StubEmbedder};
        let parts = MemoryParts {
            embedder: Arc::new(StubEmbedder::d384()) as Arc<dyn Embedder>,
            store: Arc::new(InMemoryStore::new()) as Arc<dyn MemoryStore>,
            cfg: Arc::new(MemoryConfig::default()),
        };
        let p = local_params(
            PathBuf::from("/tmp/ws"),
            PathBuf::from("/tmp/rt.json"),
            "http://localhost:8080".into(),
            "m".into(),
            Some(&parts),
        );
        assert_eq!(p.memory_tools.len(), 3);
        assert!(p.memory_retriever.is_some());
        assert_eq!(p.recall_token_budget, 512);
        assert!(p.memory_parts.is_some());
    }
}
