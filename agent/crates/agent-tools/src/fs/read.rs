use crate::fs::paths::resolve_in_workspace;
use crate::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

fn arg_path(args: &serde_json::Value) -> Result<String, ToolError> {
    args.get("path").and_then(|v| v.as_str()).map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field `path`".into()))
}

pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read the contents of a file within the workspace." }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: self.name().into(), description: self.description().into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}) }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = arg_path(args)?;
        Ok(ToolIntent { tool: "read_file".into(), access: Access::Read,
            paths: vec![path.clone().into()], command: None, summary: format!("read {path}") })
    }
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError> {
        let path = arg_path(&args)?;
        let full = resolve_in_workspace(&ctx.workspace, &path)?;
        let content = tokio::fs::read_to_string(&full).await
            .map_err(|e| ToolError::NotFound(format!("{path}: {e}")))?;
        Ok(ToolOutput { content, display: None })
    }
}

pub struct ListDirectory;

#[async_trait]
impl Tool for ListDirectory {
    fn name(&self) -> &str { "list_directory" }
    fn description(&self) -> &str { "List entries of a directory within the workspace." }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: self.name().into(), description: self.description().into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}) }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = arg_path(args)?;
        Ok(ToolIntent { tool: "list_directory".into(), access: Access::Read,
            paths: vec![path.clone().into()], command: None, summary: format!("list {path}") })
    }
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError> {
        let path = arg_path(&args)?;
        let full = resolve_in_workspace(&ctx.workspace, &path)?;
        let mut entries = tokio::fs::read_dir(&full).await
            .map_err(|e| ToolError::NotFound(format!("{path}: {e}")))?;
        let mut names = Vec::new();
        while let Some(e) = entries.next_entry().await
            .map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })? {
            names.push(e.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(ToolOutput { content: names.join("\n"), display: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use serde_json::json;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn ctx(ws: std::path::PathBuf) -> ToolCtx {
        use std::sync::Arc;
        ToolCtx { workspace: ws, timeout: Duration::from_secs(5), cancel: CancellationToken::new(),
            sandbox: Arc::new(crate::HostExecutor) }
    }

    #[tokio::test]
    async fn read_file_returns_contents() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let out = ReadFile.execute(json!({"path":"a.txt"}), &ctx(dir.path().into())).await.unwrap();
        assert_eq!(out.content, "hello");
    }

    #[test]
    fn read_file_intent_is_read_access() {
        let intent = ReadFile.intent(&json!({"path":"a.txt"})).unwrap();
        assert_eq!(intent.access, Access::Read);
        assert_eq!(intent.tool, "read_file");
    }

    #[tokio::test]
    async fn list_directory_lists_entries() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "").unwrap();
        let out = ListDirectory.execute(json!({"path":"."}), &ctx(dir.path().into())).await.unwrap();
        assert!(out.content.contains("x.txt"));
    }
}
