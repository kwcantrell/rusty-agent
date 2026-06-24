use crate::{Access, Display, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

fn str_arg(args: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    args.get(key).and_then(|v| v.as_str()).map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing string field `{key}`")))
}
fn opt_str(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

/// Builtin tool: render an arbitrary artifact into the browser Inspector.
/// Side-effect-free; produces a `Display` payload on the existing tool_result path.
pub struct RenderArtifact;

#[async_trait]
impl Tool for RenderArtifact {
    fn name(&self) -> &str { "render" }
    fn description(&self) -> &str {
        "Render an artifact (markdown, code, html, mermaid diagram, table, or image) into the user's Inspector panel."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": {"type": "string",
                        "enum": ["markdown","code","html","mermaid","table","image"]},
                    "title": {"type": "string"},
                    "id": {"type": "string", "description": "stable id; re-rendering the same id replaces the artifact"},
                    "content": {"type": "string",
                        "description": "primary payload: markdown/html/mermaid source, code text, or base64 image data"},
                    "lang": {"type": "string", "description": "code language (kind=code)"},
                    "filename": {"type": "string", "description": "code filename (kind=code)"},
                    "mime": {"type": "string", "description": "image mime type (kind=image)"},
                    "columns": {"type": "array", "items": {"type": "string"}},
                    "rows": {"type": "array", "items": {"type": "array", "items": {"type": "string"}}}
                },
                "required": ["kind"]
            }),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let kind = str_arg(args, "kind")?;
        Ok(ToolIntent { tool: "render".into(), access: Access::Read, paths: vec![],
            command: None, summary: format!("render {kind}") })
    }
    async fn execute(&self, args: serde_json::Value, _ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError> {
        let kind = str_arg(&args, "kind")?;
        let title = opt_str(&args, "title");
        let id = opt_str(&args, "id");
        let display = match kind.as_str() {
            "markdown" => Display::Markdown { text: str_arg(&args, "content")?, title: title.clone(), id },
            "html" => Display::Html { html: str_arg(&args, "content")?, title: title.clone(), id },
            "mermaid" => Display::Mermaid { source: str_arg(&args, "content")?, title: title.clone(), id },
            "code" => Display::Code {
                lang: opt_str(&args, "lang").unwrap_or_else(|| "text".into()),
                filename: opt_str(&args, "filename"),
                text: str_arg(&args, "content")?, title: title.clone(), id },
            "image" => Display::Image {
                mime: opt_str(&args, "mime").unwrap_or_else(|| "image/png".into()),
                data: str_arg(&args, "content")?, title: title.clone(), id },
            "table" => {
                let columns: Vec<String> = serde_json::from_value(
                    args.get("columns").cloned().unwrap_or(json!([])))
                    .map_err(|e| ToolError::InvalidArgs(format!("columns: {e}")))?;
                let rows: Vec<Vec<String>> = serde_json::from_value(
                    args.get("rows").cloned().unwrap_or(json!([])))
                    .map_err(|e| ToolError::InvalidArgs(format!("rows: {e}")))?;
                Display::Table { columns, rows, title: title.clone(), id }
            }
            other => return Err(ToolError::InvalidArgs(format!("unknown kind `{other}`"))),
        };
        let ack = match &title { Some(t) => format!("rendered {kind}: {t}"), None => format!("rendered {kind}") };
        Ok(ToolOutput { content: ack, display: Some(display) })
    }
}

#[cfg(test)]
mod tests {
    use super::RenderArtifact;
    use crate::*;
    use serde_json::json;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn ctx() -> ToolCtx {
        use std::sync::Arc;
        ToolCtx { workspace: std::env::temp_dir(), timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(), sandbox: Arc::new(crate::HostExecutor) }
    }

    #[tokio::test]
    async fn render_markdown_emits_markdown_display() {
        let out = RenderArtifact.execute(
            json!({"kind":"markdown","title":"Plan","content":"# Hello"}), &ctx())
            .await.unwrap();
        match out.display {
            Some(Display::Markdown { text, title, .. }) => {
                assert_eq!(text, "# Hello");
                assert_eq!(title.as_deref(), Some("Plan"));
            }
            other => panic!("expected Markdown, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn render_code_carries_lang_and_filename() {
        let out = RenderArtifact.execute(
            json!({"kind":"code","lang":"rust","filename":"a.rs","content":"fn x(){}"}), &ctx())
            .await.unwrap();
        assert!(matches!(out.display, Some(Display::Code { .. })));
    }

    #[tokio::test]
    async fn render_table_uses_columns_and_rows() {
        let out = RenderArtifact.execute(
            json!({"kind":"table","columns":["a","b"],"rows":[["1","2"]]}), &ctx())
            .await.unwrap();
        match out.display {
            Some(Display::Table { columns, rows, .. }) => {
                assert_eq!(columns, vec!["a", "b"]);
                assert_eq!(rows, vec![vec!["1", "2"]]);
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn render_rejects_unknown_kind() {
        let err = RenderArtifact.execute(json!({"kind":"wat","content":"x"}), &ctx())
            .await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn render_intent_is_read() {
        let i = RenderArtifact.intent(&json!({"kind":"markdown","content":"x"})).unwrap();
        assert_eq!(i.access, Access::Read);
    }
}
