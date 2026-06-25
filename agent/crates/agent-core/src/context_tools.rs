use crate::offload::OffloadStore;
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Rehydrate an offloaded entry by id, returning its full content to the model.
pub struct ContextRecallTool {
    store: Arc<dyn OffloadStore>,
}

impl ContextRecallTool {
    pub fn new(store: Arc<dyn OffloadStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ContextRecallTool {
    fn name(&self) -> &str {
        "context_recall"
    }
    fn description(&self) -> &str {
        "Recall the full content of a previously offloaded tool result by its id \
         (the number in a [tool_result#N offloaded ...] placeholder)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "context_recall".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "offload id" } },
                "required": ["id"]
            }),
        }
    }
    fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "context_recall".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "recall offloaded content".into(),
        })
    }
    async fn execute(&self, args: serde_json::Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let id = args
            .get("id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ToolError::InvalidArgs("missing integer 'id'".into()))?;
        match self.store.get(id) {
            Some(entry) => Ok(ToolOutput { content: entry.content, display: None }),
            None => Err(ToolError::NotFound(format!(
                "no offloaded entry #{id} (may have been cleared)"
            ))),
        }
    }
}

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
    async fn execute(&self, _args: serde_json::Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        self.flag.store(true, Ordering::SeqCst);
        Ok(ToolOutput {
            content: "Compaction requested; it will run on the next turn.".into(),
            display: None,
        })
    }
}

/// The context-management tool pair, sharing handles with a `CuratedContext`.
pub fn context_tools(store: Arc<dyn OffloadStore>, flag: Arc<AtomicBool>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ContextRecallTool::new(store)),
        Arc::new(ContextCompactTool::new(flag)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offload::{InMemoryOffloadStore, OffloadEntry, OffloadKind};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn tool_ctx() -> ToolCtx {
        ToolCtx {
            workspace: std::env::temp_dir(),
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(agent_tools::HostExecutor),
        }
    }

    #[tokio::test]
    async fn recall_returns_full_content() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let id = store.put(OffloadEntry {
            id: 0,
            tool_call_id: "c1".into(),
            tool_name: "shell".into(),
            kind: OffloadKind::Error,
            content: "the full stack trace".into(),
            bytes: 20,
            turn: 0,
        });
        let tool = ContextRecallTool::new(store);
        let out = tool.execute(json!({ "id": id }), &tool_ctx()).await.unwrap();
        assert_eq!(out.content, "the full stack trace");
    }

    #[tokio::test]
    async fn recall_unknown_id_is_not_found() {
        let tool = ContextRecallTool::new(Arc::new(InMemoryOffloadStore::new()));
        let err = tool.execute(json!({ "id": 999 }), &tool_ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }

    #[tokio::test]
    async fn compact_sets_the_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let tool = ContextCompactTool::new(flag.clone());
        tool.execute(json!({}), &tool_ctx()).await.unwrap();
        assert!(flag.load(Ordering::SeqCst));
    }
}
