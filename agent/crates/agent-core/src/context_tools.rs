use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Request a compaction pass on the next maintenance cycle.
pub struct ContextCompactTool {
    flag: Arc<AtomicBool>,
}

impl ContextCompactTool {
    pub fn new(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }
}

#[async_trait]
impl Tool for ContextCompactTool {
    fn name(&self) -> &str {
        "context_compact"
    }
    fn description(&self) -> &str {
        "Request compaction of older conversation history into a summary on the \
         next turn. Use when the context is full of resolved sub-tasks."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "context_compact".into(),
            description: self.description().into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }
    fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "context_compact".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "request context compaction".into(),
        })
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        self.flag.store(true, Ordering::SeqCst);
        Ok(ToolOutput {
            content: "Compaction requested; it will run on the next turn.".into(),
            display: None,
        })
    }
}

/// The context-management toolset. Since Phase 2 (spec G5) this is compact
/// only: offload recovery goes through the ordinary file tools.
pub fn context_tools(flag: Arc<AtomicBool>) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(ContextCompactTool::new(flag))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn tool_ctx() -> ToolCtx {
        ToolCtx {
            workspace: std::env::temp_dir(),
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(agent_tools::HostExecutor),
            backend: Arc::new(agent_tools::backend::HostBackend::new(std::env::temp_dir())),
            call_id: "test".into(),
        }
    }

    #[tokio::test]
    async fn compact_sets_the_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let tool = ContextCompactTool::new(flag.clone());
        tool.execute(json!({}), &tool_ctx()).await.unwrap();
        assert!(flag.load(Ordering::SeqCst));
    }
}
