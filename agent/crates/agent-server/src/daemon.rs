use agent_core::Retriever;
use agent_runtime_config::RuntimeConfig;
use agent_tools::Tool;
use std::path::PathBuf;
use std::sync::Arc;

/// Everything the bridge needs to construct a [`crate::session::Session`].
/// (Formerly also drove the WebSocket `serve()`; the transport is now Tauri IPC.)
pub struct DaemonParams {
    pub config: RuntimeConfig, // flag-derived base; the file at config_path overlays it
    pub api_key: Option<String>,
    pub claude_binary: String,
    pub config_path: PathBuf,
    pub workspace: PathBuf,
    pub system_prompt: String,
    pub mcp_tools: Arc<[Arc<dyn Tool>]>,
    pub memory_tools: Arc<[Arc<dyn Tool>]>,
    pub memory_retriever: Option<Arc<dyn Retriever>>,
    pub recall_token_budget: usize,
}

pub const SYSTEM_PROMPT: &str = "You are a local coding agent. Use the provided tools to inspect \
and modify the workspace. Think step by step. When the task is complete, reply with a summary \
and no tool call.";
