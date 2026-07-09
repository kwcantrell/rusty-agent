use crate::fs::paths::resolve_in_workspace;
use crate::{Access, Display, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

fn str_arg(args: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing string field `{key}`")))
}

fn diff(path: &str, before: &str, after: &str) -> Display {
    Display::Diff {
        path: path.into(),
        before: before.into(),
        after: after.into(),
    }
}

pub struct WriteFile;

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Create or overwrite a file within the workspace."
    }
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Not for a small change to an existing file — use edit_file to replace \
              a specific substring. Use write_file to create a new file or fully \
              overwrite one.",
        )
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"Workspace-relative path of the file to create or overwrite."},
                "content":{"type":"string","description":"The full contents to write to the file."}},
                "required":["path","content"]}),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = str_arg(args, "path")?;
        Ok(ToolIntent {
            tool: "write_file".into(),
            access: Access::Write,
            paths: vec![path.clone().into()],
            command: None,
            summary: format!("write {path}"),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let path = str_arg(&args, "path")?;
        let content = str_arg(&args, "content")?;
        let full = resolve_in_workspace(&ctx.workspace, &path)?;
        let before = tokio::fs::read_to_string(&full).await.unwrap_or_default();
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ToolError::Failed {
                    message: e.to_string(),
                    stderr: None,
                })?;
        }
        tokio::fs::write(&full, &content)
            .await
            .map_err(|e| ToolError::Failed {
                message: e.to_string(),
                stderr: None,
            })?;
        Ok(ToolOutput {
            content: format!("wrote {} bytes to {path}", content.len()),
            display: Some(diff(&path, &before, &content)),
        })
    }
}

pub struct EditFile;

#[async_trait]
impl Tool for EditFile {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "Replace a unique substring in a workspace file."
    }
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Not for creating a new file or rewriting a whole file — use write_file. \
              Use edit_file to replace one unique existing substring.",
        )
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"Workspace-relative path of the file to edit."},
                "old":{"type":"string","description":"The exact existing substring to replace; must occur exactly once in the file."},
                "new":{"type":"string","description":"The replacement text."}},
                "required":["path","old","new"]}),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = str_arg(args, "path")?;
        Ok(ToolIntent {
            tool: "edit_file".into(),
            access: Access::Write,
            paths: vec![path.clone().into()],
            command: None,
            summary: format!("edit {path}"),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let path = str_arg(&args, "path")?;
        let old = str_arg(&args, "old")?;
        let new = str_arg(&args, "new")?;
        let full = resolve_in_workspace(&ctx.workspace, &path)?;
        let before = tokio::fs::read_to_string(&full)
            .await
            .map_err(|e| ToolError::NotFound(format!("{path}: {e}")))?;
        let count = before.matches(&old).count();
        if count != 1 {
            return Err(ToolError::Failed {
                message: format!("`old` matched {count} times; must match exactly once"),
                stderr: None,
            });
        }
        let after = before.replacen(&old, &new, 1);
        tokio::fs::write(&full, &after)
            .await
            .map_err(|e| ToolError::Failed {
                message: e.to_string(),
                stderr: None,
            })?;
        Ok(ToolOutput {
            content: format!("edited {path}"),
            display: Some(diff(&path, &before, &after)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{EditFile, WriteFile};
    use crate::*;
    use serde_json::json;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn ctx(ws: std::path::PathBuf) -> ToolCtx {
        use std::sync::Arc;
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
    async fn write_file_creates_and_returns_diff() {
        let dir = tempdir().unwrap();
        let out = WriteFile
            .execute(
                json!({"path":"new.txt","content":"hi\n"}),
                &ctx(dir.path().into()),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("new.txt")).unwrap(),
            "hi\n"
        );
        assert!(matches!(out.display, Some(Display::Diff { .. })));
    }

    #[test]
    fn write_file_intent_is_write() {
        let i = WriteFile
            .intent(&json!({"path":"a","content":"b"}))
            .unwrap();
        assert_eq!(i.access, Access::Write);
    }

    #[tokio::test]
    async fn edit_file_replaces_unique_substring() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "foo bar baz").unwrap();
        EditFile
            .execute(
                json!({"path":"a.txt","old":"bar","new":"QUX"}),
                &ctx(dir.path().into()),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "foo QUX baz"
        );
    }

    #[tokio::test]
    async fn edit_file_errors_when_old_not_unique() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "x x").unwrap();
        let err = EditFile
            .execute(
                json!({"path":"a.txt","old":"x","new":"y"}),
                &ctx(dir.path().into()),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed { .. }));
    }
}
