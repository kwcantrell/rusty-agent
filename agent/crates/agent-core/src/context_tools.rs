use crate::offload::OffloadStore;
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Continuation marker for a paged recall. `start`/`end` are byte offsets.
fn recall_marker(id: u64, start: usize, end: usize, total: usize) -> String {
    format!(
        "\n[bytes {start}–{end} of {total} — continue with context_recall(id: {id}, offset: {end})]"
    )
}

/// Rehydrate an offloaded entry by id, returning its content to the model in
/// pages bounded by a byte budget.
pub struct ContextRecallTool {
    store: Arc<dyn OffloadStore>,
    /// Max bytes returned per call (slice + continuation marker together).
    page_bytes: usize,
}

impl ContextRecallTool {
    pub fn new(store: Arc<dyn OffloadStore>, page_bytes: usize) -> Self {
        Self { store, page_bytes }
    }
}

#[async_trait]
impl Tool for ContextRecallTool {
    fn name(&self) -> &str {
        "context_recall"
    }
    fn description(&self) -> &str {
        "Recall the content of a previously offloaded tool result by its id (the \
         number in a [tool_result#N ...] placeholder or truncation marker). Large \
         entries return in pages; follow the continuation marker's offset to read more."
    }
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Not for semantic search of saved memories — use recall. Use \
              context_recall only to rehydrate a specific offloaded entry by its id.",
        )
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "context_recall".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "offload id" },
                    "offset": { "type": "integer", "description":
                        "Byte offset to continue from (default 0). Use the offset value given \
                         in a previous page's continuation marker." }
                },
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
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let id = args
            .get("id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ToolError::InvalidArgs("missing integer 'id'".into()))?;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let entry = self.store.get(id).ok_or_else(|| {
            ToolError::NotFound(format!("no offloaded entry #{id} (may have been cleared)"))
        })?;
        let total = entry.content.len();
        if offset > 0 && offset >= total {
            return Err(ToolError::InvalidArgs(format!(
                "offset {offset} is past the end of entry #{id} ({total} bytes)"
            )));
        }
        let mut start = offset;
        while !entry.content.is_char_boundary(start) {
            start -= 1;
        }
        let rest = &entry.content[start..];
        if rest.len() <= self.page_bytes {
            return Ok(ToolOutput {
                content: rest.to_string(),
                display: None,
            });
        }
        // Budget the slice against the widest the marker can render (end = total).
        let worst = recall_marker(id, start, total, total);
        let budget = self.page_bytes.saturating_sub(worst.len()).max(1);
        let mut cut = start + budget;
        while !entry.content.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut <= start {
            // Pathological page size smaller than one scalar + marker: still
            // make forward progress by taking exactly one char.
            cut = start + rest.chars().next().map_or(1, |c| c.len_utf8());
        }
        let content = format!(
            "{}{}",
            &entry.content[start..cut],
            recall_marker(id, start, cut, total)
        );
        Ok(ToolOutput {
            content,
            display: None,
        })
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

/// The context-management tool pair, sharing handles with a `CuratedContext`.
/// `recall_page_bytes` bounds each `context_recall` page (callers pass the
/// ingestion cap so recall pages can never re-trip it).
pub fn context_tools(
    store: Arc<dyn OffloadStore>,
    flag: Arc<AtomicBool>,
    recall_page_bytes: usize,
) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ContextRecallTool::new(store, recall_page_bytes)),
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

    fn put_entry(store: &InMemoryOffloadStore, content: &str) -> u64 {
        store.put(OffloadEntry {
            id: 0,
            tool_call_id: "c1".into(),
            tool_name: "shell".into(),
            kind: OffloadKind::Output,
            content: content.into(),
            bytes: content.len(),
            turn: 0,
        })
    }

    /// Extract the continuation offset from a page's trailing marker.
    fn continuation_offset(page: &str) -> Option<usize> {
        let tail = page.rsplit("offset: ").next()?;
        tail.split(')').next()?.trim().parse().ok()
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
        let tool = ContextRecallTool::new(store, 8 * 1024);
        let out = tool
            .execute(json!({ "id": id }), &tool_ctx())
            .await
            .unwrap();
        assert_eq!(out.content, "the full stack trace");
    }

    #[tokio::test]
    async fn recall_unknown_id_is_not_found() {
        let tool = ContextRecallTool::new(Arc::new(InMemoryOffloadStore::new()), 8 * 1024);
        let err = tool
            .execute(json!({ "id": 999 }), &tool_ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }

    #[tokio::test]
    async fn recall_pages_a_large_entry_to_completion() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let content: String = (0..10_000)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        let id = put_entry(&store, &content);
        let tool = ContextRecallTool::new(store, 4096);

        let mut reassembled = String::new();
        let mut offset = 0usize;
        loop {
            let out = tool
                .execute(json!({ "id": id, "offset": offset }), &tool_ctx())
                .await
                .unwrap();
            assert!(out.content.len() <= 4096, "page exceeds budget");
            match continuation_offset(&out.content) {
                Some(next) if out.content.contains("continue with context_recall") => {
                    let body = out.content.rsplit_once("\n[bytes ").unwrap().0;
                    assert!(next > offset, "no forward progress");
                    reassembled.push_str(body);
                    offset = next;
                }
                _ => {
                    reassembled.push_str(&out.content);
                    break;
                }
            }
        }
        assert_eq!(reassembled, content, "pages must reassemble the original");
    }

    #[tokio::test]
    async fn recall_small_entry_has_no_marker() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let id = put_entry(&store, "short");
        let tool = ContextRecallTool::new(store, 4096);
        let out = tool
            .execute(json!({ "id": id }), &tool_ctx())
            .await
            .unwrap();
        assert_eq!(out.content, "short");
    }

    #[tokio::test]
    async fn recall_offset_past_end_is_invalid_args() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let id = put_entry(&store, "short");
        let tool = ContextRecallTool::new(store, 4096);
        let err = tool
            .execute(json!({ "id": id, "offset": 999 }), &tool_ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn recall_slices_on_char_boundaries() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let content = "🦀".repeat(3000); // 12 000 bytes of 4-byte scalars
        let id = put_entry(&store, &content);
        let tool = ContextRecallTool::new(store, 4096);
        let out = tool
            .execute(json!({ "id": id }), &tool_ctx())
            .await
            .unwrap();
        assert!(out.content.starts_with('🦀')); // no panic, clean boundary
    }

    #[tokio::test]
    async fn compact_sets_the_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let tool = ContextCompactTool::new(flag.clone());
        tool.execute(json!({}), &tool_ctx()).await.unwrap();
        assert!(flag.load(Ordering::SeqCst));
    }
}
