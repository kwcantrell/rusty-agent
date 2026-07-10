//! Regex search over the loop's virtual filesystem — the search half of the
//! offload-recovery surface that replaces context_recall (spec §5.4).
use crate::backend::GREP_MAX_HITS;
use crate::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search file contents by regex. Returns path:line: text hits. Searches \
         the workspace AND the read-only offload records under large_tool_results/ \
         and conversation_history/ (shell commands cannot see those two prefixes)."
    }
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Use grep to search current file contents, including offloaded tool \
             results under large_tool_results/.",
        )
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "grep".into(),
            description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "pattern":{"type":"string","description":"Rust-flavored regex matched per line."},
                "path":{"type":"string","description":"Optional file or directory prefix to scope the search (e.g. large_tool_results/)."}},
                "required":["pattern"]}),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing string field `pattern`".into()))?;
        let scope = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        Ok(ToolIntent {
            tool: "grep".into(),
            access: Access::Read,
            paths: vec![scope.into()],
            command: None,
            summary: format!("grep {pattern:?} in {scope}"),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing string field `pattern`".into()))?;
        let scope = args.get("path").and_then(|v| v.as_str());
        let hits = ctx
            .backend
            .grep(pattern, scope)
            .await
            .map_err(crate::fs::fs_err)?;
        if hits.is_empty() {
            return Ok(ToolOutput {
                content: "no matches".into(),
                display: None,
            });
        }
        let capped = hits.len() >= GREP_MAX_HITS;
        let mut lines: Vec<String> = hits
            .into_iter()
            .map(|h| format!("{}:{}: {}", h.path, h.line, h.text))
            .collect();
        if capped {
            lines.push(format!(
                "[hit cap reached: {GREP_MAX_HITS} — narrow the pattern or scope]"
            ));
        }
        Ok(ToolOutput {
            content: lines.join("\n"),
            display: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn ctx(ws: std::path::PathBuf) -> ToolCtx {
        ToolCtx {
            workspace: ws.clone(),
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(crate::HostExecutor),
            backend: Arc::new(crate::backend::HostBackend::new(ws)),
            call_id: "test".into(),
        }
    }

    #[tokio::test]
    async fn grep_reports_path_line_and_text() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "alpha\nthe needle line\n").unwrap();
        let out = GrepTool
            .execute(json!({"pattern": "needle"}), &ctx(dir.path().into()))
            .await
            .unwrap();
        assert_eq!(out.content, "f.txt:2: the needle line");
    }

    #[tokio::test]
    async fn grep_no_hits_says_so() {
        let dir = tempdir().unwrap();
        let out = GrepTool
            .execute(json!({"pattern": "absent"}), &ctx(dir.path().into()))
            .await
            .unwrap();
        assert_eq!(out.content, "no matches");
    }

    #[test]
    fn grep_intent_is_read_with_scope() {
        let i = GrepTool
            .intent(&json!({"pattern": "x", "path": "src/"}))
            .unwrap();
        assert_eq!(i.access, Access::Read);
        assert_eq!(i.paths, vec![std::path::PathBuf::from("src/")]);
    }
}
