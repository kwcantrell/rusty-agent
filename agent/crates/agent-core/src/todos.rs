//! Planning-by-recitation (spec §5.4). `write_todos` rewrites a shared list the
//! curator renders as a durable PINNED block (E3 pin/recall) — the tool itself
//! performs no computation; its value is keeping the plan in the attention
//! window over long tasks. The list is never merged back from subagents.
use agent_model::Message;
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

impl TodoStatus {
    fn label(self) -> &'static str {
        match self {
            TodoStatus::Pending => "pending",
            TodoStatus::InProgress => "in_progress",
            TodoStatus::Completed => "completed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

/// The plan list shared between `WriteTodosTool` (writer) and `CuratedContext`
/// (renderer) — the `compact_flag` shape (spec §5.4/§5.6).
pub type TodoHandle = Arc<Mutex<Vec<TodoItem>>>;

/// The non-empty list as the pinned todos block, or `None` when empty (spec
/// §5.4). Rendered as the LAST pinned block by `CuratedContext::pinned()`.
pub fn render_todos_block(items: &[TodoItem]) -> Option<Message> {
    if items.is_empty() {
        return None;
    }
    let lines = items
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. [{}] {}", i + 1, t.status.label(), t.content))
        .collect::<Vec<_>>()
        .join("\n");
    Some(Message::system(format!(
        "Current task plan (from write_todos) — keep working the in_progress \
         items until the plan is complete:\n{lines}"
    )))
}

/// Rewrites the whole plan list into the shared handle. Returns a COMPACT
/// confirmation (not the list) so its own tool-result message is offload-
/// irrelevant; the authoritative recitation is the pinned block (spec §5.4).
pub struct WriteTodosTool {
    handle: TodoHandle,
}

impl WriteTodosTool {
    pub fn new(handle: TodoHandle) -> Self {
        Self { handle }
    }
}

#[derive(Deserialize)]
struct WriteTodosArgs {
    todos: Vec<TodoItem>,
}

#[async_trait]
impl Tool for WriteTodosTool {
    fn name(&self) -> &str {
        "write_todos"
    }
    fn description(&self) -> &str {
        "Record or update your task plan for a complex, multi-step objective \
         (3+ distinct steps or non-trivial planning). Do NOT use it for single, \
         straightforward, or conversational turns — for a simple objective, just \
         do the work directly. Each call REPLACES the whole list. Keep at least \
         one task in_progress while work remains; multiple tasks may be \
         in_progress at once when they are independent and can proceed in \
         parallel. Mark a task completed immediately when it is done — do not \
         batch completions. The plan stays visible in your context so you stay \
         on track over long tasks."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "write_todos".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "description": "The full task list; replaces any prior list.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": {"type": "string", "description": "The task."},
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"],
                                    "description": "Task status."
                                }
                            },
                            "required": ["content", "status"]
                        }
                    }
                },
                "required": ["todos"]
            }),
        }
    }
    fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "write_todos".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "update the task plan".into(),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let parsed: WriteTodosArgs = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(format!("write_todos: {e}")))?;
        let n = parsed.todos.len();
        *self.handle.lock().unwrap() = parsed.todos;
        Ok(ToolOutput {
            content: format!("Plan updated ({n} task(s))."),
            display: None,
        })
    }
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
    async fn write_todos_sets_the_handle_and_returns_a_compact_confirmation() {
        let handle: Arc<Mutex<Vec<TodoItem>>> = Arc::new(Mutex::new(Vec::new()));
        let tool = WriteTodosTool::new(handle.clone());
        let out = tool
            .execute(
                json!({"todos": [
                    {"content": "parse", "status": "in_progress"},
                    {"content": "wire", "status": "pending"}
                ]}),
                &tool_ctx(),
            )
            .await
            .unwrap();
        // The list is now in the handle...
        let items = handle.lock().unwrap().clone();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].status, TodoStatus::InProgress);
        // ...and the tool result is a COMPACT confirmation, not the full list, so
        // its own tool-result message is offload-irrelevant (spec §5.4).
        assert!(
            out.content.len() < 80,
            "compact confirmation: {}",
            out.content
        );
        assert!(
            !out.content.contains("parse"),
            "must not echo the list back"
        );
    }

    #[test]
    fn render_todos_block_is_none_when_empty_and_lists_statuses_when_set() {
        assert!(render_todos_block(&[]).is_none());
        let block = render_todos_block(&[
            TodoItem {
                content: "parse".into(),
                status: TodoStatus::InProgress,
            },
            TodoItem {
                content: "wire".into(),
                status: TodoStatus::Pending,
            },
        ])
        .expect("non-empty renders a block");
        assert!(matches!(block.role, agent_model::Role::System));
        assert!(block.content.contains("in_progress"));
        assert!(block.content.contains("parse"));
    }

    #[test]
    fn write_todos_description_permits_multiple_in_progress() {
        // Panel B1: the real LangChain contract allows multiple independent
        // in_progress tasks; the earlier draft's "exactly one" inverted it, and this
        // ships verbatim to the model. Snapshot-guard the wording.
        let tool = WriteTodosTool::new(Arc::new(Mutex::new(Vec::new())));
        let d = tool.description().to_lowercase();
        assert!(d.contains("multiple") && d.contains("in_progress") && d.contains("parallel"));
        assert!(d.contains("in_progress") && d.contains("completed"));
    }
}
