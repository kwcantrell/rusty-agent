//! Shared agent loop wiring (tool registry, protocol picker, command lists)
//! used by both the CLI (`agent-cli`) and the daemon (`agent-server`).

mod runtime_config;
pub use runtime_config::{RuntimeConfig, HARD_FLOOR_DENYLIST};

use agent_mcp::McpServersConfig;
use agent_model::{ClaudeCliClient, ModelClient, NativeProtocol, OpenAiCompatClient,
                  PromptedJsonProtocol, ToolCallProtocol};
use agent_tools::fs::{EditFile, ListDirectory, ReadFile, WriteFile};
use agent_tools::{git::{GitCommit, GitDiff, GitStatus}, shell::ExecuteCommand, ToolRegistry};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub use agent_mcp::{McpManager, ServerStatus};

/// Load `mcp.json` at `path` and connect its servers. A missing file yields an
/// empty manager (MCP disabled); a malformed file warns and yields empty. The
/// returned `McpManager` owns the server processes — keep it alive for the session.
pub async fn connect_mcp(path: &Path) -> McpManager {
    let (cfg, warning) = McpServersConfig::load_or_empty(path);
    if let Some(w) = warning {
        eprintln!("warning: {} ({}); MCP disabled", w, path.display());
    }
    McpManager::connect(&cfg, Duration::from_secs(15)).await
}

pub fn protocol_name_is_valid(name: &str) -> bool {
    matches!(name, "native" | "prompted")
}

pub fn pick_protocol(name: &str) -> Arc<dyn ToolCallProtocol> {
    match name {
        "prompted" => Arc::new(PromptedJsonProtocol),
        _ => Arc::new(NativeProtocol),
    }
}

pub fn backend_name_is_valid(name: &str) -> bool {
    matches!(name, "openai" | "claude-cli")
}

/// Build the model client for the selected backend.
/// `claude-cli` ignores `base_url`/`api_key`; `openai` ignores `claude_binary`.
pub fn build_model(
    backend: &str,
    base_url: &str,
    model: &str,
    claude_binary: &str,
    api_key: Option<String>,
) -> Arc<dyn ModelClient> {
    match backend {
        "claude-cli" => Arc::new(ClaudeCliClient::new(claude_binary, model)),
        _ => Arc::new(OpenAiCompatClient::new(base_url.to_string(), model.to_string(), api_key)),
    }
}

pub fn build_registry() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(ReadFile));
    r.register(Arc::new(WriteFile));
    r.register(Arc::new(EditFile));
    r.register(Arc::new(ListDirectory));
    r.register(Arc::new(ExecuteCommand));
    r.register(Arc::new(GitStatus));
    r.register(Arc::new(GitDiff));
    r.register(Arc::new(GitCommit));
    r
}

pub fn default_allowlist() -> Vec<String> {
    ["ls","cat","pwd","echo","git","grep","find","rg","cargo","head","tail","wc"]
        .into_iter().map(String::from).collect()
}
pub fn default_denylist() -> Vec<String> {
    ["rm -rf /","sudo",":(){","mkfs","dd if="].into_iter().map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backend_validation() {
        assert!(backend_name_is_valid("openai"));
        assert!(backend_name_is_valid("claude-cli"));
        assert!(!backend_name_is_valid("bogus"));
    }
    #[test]
    fn pick_protocol_selects_by_name() {
        assert!(protocol_name_is_valid("native"));
        assert!(protocol_name_is_valid("prompted"));
        assert!(!protocol_name_is_valid("bogus"));
    }
    #[test]
    fn registry_has_all_core_tools() {
        let r = build_registry();
        for name in ["read_file","write_file","edit_file","list_directory",
                     "execute_command","git_status","git_diff","git_commit"] {
            assert!(r.get(name).is_some(), "missing {name}");
        }
    }
}
