use crate::fs::paths::resolve_in_workspace;
use crate::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

fn arg_path(args: &serde_json::Value) -> Result<String, ToolError> {
    args.get("path")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field `path`".into()))
}

pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read the contents of a file within the workspace."
    }
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Not for files bundled inside a loaded skill's directory — use \
              read_skill_file for those. Use read_file for workspace paths.",
        )
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"Workspace-relative path of the file to read."},
                "offset":{"type":"integer","description":"1-based line number to start reading from (default 1)."},
                "limit":{"type":"integer","description":"Maximum number of lines to return (default: all lines)."}},
                "required":["path"]}),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = arg_path(args)?;
        Ok(ToolIntent {
            tool: "read_file".into(),
            access: Access::Read,
            paths: vec![path.clone().into()],
            command: None,
            summary: format!("read {path}"),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let path = arg_path(&args)?;
        let full = resolve_in_workspace(&ctx.workspace, &path)?;
        let content = tokio::fs::read_to_string(&full)
            .await
            .map_err(|e| ToolError::NotFound(format!("{path}: {e}")))?;
        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| (v as usize).max(1));
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let content = match (offset, limit) {
            (None, None) => content, // whole-file fast path, byte-identical
            (o, l) => {
                let first = o.unwrap_or(1);
                let lines: Vec<&str> = content.lines().collect();
                let n = lines.len();
                if first > n {
                    return Err(ToolError::InvalidArgs(format!(
                        "offset {first} is past the end of {path} ({n} lines)"
                    )));
                }
                let last = l.map_or(n, |l| (first + l - 1).min(n));
                if first == 1 && last == n {
                    content // limit covers the whole file: unchanged
                } else {
                    format!(
                        "[lines {first}–{last} of {n}]\n{}",
                        lines[first - 1..last].join("\n")
                    )
                }
            }
        };
        Ok(ToolOutput {
            content,
            display: None,
        })
    }
}

pub struct ListDirectory;

#[async_trait]
impl Tool for ListDirectory {
    fn name(&self) -> &str {
        "list_directory"
    }
    fn description(&self) -> &str {
        "List entries of a directory within the workspace."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"Workspace-relative path of the directory to list."}},
                "required":["path"]}),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let path = arg_path(args)?;
        Ok(ToolIntent {
            tool: "list_directory".into(),
            access: Access::Read,
            paths: vec![path.clone().into()],
            command: None,
            summary: format!("list {path}"),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let path = arg_path(&args)?;
        let full = resolve_in_workspace(&ctx.workspace, &path)?;
        let mut entries = tokio::fs::read_dir(&full)
            .await
            .map_err(|e| ToolError::NotFound(format!("{path}: {e}")))?;
        let mut names = Vec::new();
        while let Some(e) = entries.next_entry().await.map_err(|e| ToolError::Failed {
            message: e.to_string(),
            stderr: None,
        })? {
            names.push(e.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(ToolOutput {
            content: names.join("\n"),
            display: None,
        })
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
        ToolCtx {
            workspace: ws,
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(crate::HostExecutor),
        }
    }

    #[tokio::test]
    async fn read_file_returns_contents() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let out = ReadFile
            .execute(json!({"path":"a.txt"}), &ctx(dir.path().into()))
            .await
            .unwrap();
        assert_eq!(out.content, "hello");
    }

    #[tokio::test]
    async fn read_file_slices_with_offset_and_limit() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        let out = ReadFile
            .execute(
                json!({"path": "f.txt", "offset": 2, "limit": 2}),
                &ctx(dir.path().into()),
            )
            .await
            .unwrap();
        assert_eq!(out.content, "[lines 2–3 of 5]\nl2\nl3");
    }

    #[tokio::test]
    async fn read_file_limit_clamps_to_eof() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "l1\nl2\nl3\n").unwrap();
        let out = ReadFile
            .execute(
                json!({"path": "f.txt", "offset": 3, "limit": 99}),
                &ctx(dir.path().into()),
            )
            .await
            .unwrap();
        assert_eq!(out.content, "[lines 3–3 of 3]\nl3");
    }

    #[tokio::test]
    async fn read_file_default_is_whole_file_unchanged() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "l1\nl2\n").unwrap();
        let out = ReadFile
            .execute(json!({"path": "f.txt"}), &ctx(dir.path().into()))
            .await
            .unwrap();
        assert_eq!(out.content, "l1\nl2\n"); // byte-identical, incl. trailing newline
    }

    #[tokio::test]
    async fn read_file_offset_past_eof_is_invalid_args() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "l1\n").unwrap();
        let err = ReadFile
            .execute(
                json!({"path": "f.txt", "offset": 5}),
                &ctx(dir.path().into()),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn read_file_offset_limit_params_are_described() {
        let schema = ReadFile.schema();
        for p in ["offset", "limit"] {
            let d = schema.parameters["properties"][p]["description"]
                .as_str()
                .unwrap_or("");
            assert!(!d.is_empty(), "{p} must be described");
        }
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
        let out = ListDirectory
            .execute(json!({"path":"."}), &ctx(dir.path().into()))
            .await
            .unwrap();
        assert!(out.content.contains("x.txt"));
    }
}
