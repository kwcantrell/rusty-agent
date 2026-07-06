use crate::{Access, Display, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

/// `kind=url` targets must be the user's own dev server: http(s) with host exactly
/// `localhost`, `127.0.0.1`, or `[::1]` (any port). Exact-matching the authority up
/// to the port fails closed on userinfo tricks (`http://localhost@evil.com`).
fn validate_local_url(url: &str) -> Result<(), ToolError> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| ToolError::InvalidArgs(format!("url must be http(s): `{url}`")))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err(ToolError::InvalidArgs(format!(
            "url must not contain userinfo: `{url}`"
        )));
    }
    let host = if authority.starts_with('[') {
        match authority.split_once(']') {
            Some((h, tail)) if tail.is_empty() || tail.starts_with(':') => format!("{h}]"),
            _ => return Err(ToolError::InvalidArgs(format!("malformed url: `{url}`"))),
        }
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    match host.to_ascii_lowercase().as_str() {
        "localhost" | "127.0.0.1" | "[::1]" => Ok(()),
        other => Err(ToolError::InvalidArgs(format!(
            "url host must be localhost, 127.0.0.1, or [::1] — got `{other}`"
        ))),
    }
}

fn str_arg(args: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
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
    fn name(&self) -> &str {
        "render"
    }
    fn description(&self) -> &str {
        "Render an artifact (markdown, code, html, mermaid diagram, table, image, or a live \
         localhost url) into the user's Inspector panel. When a dev server is already running \
         (e.g. Vite), prefer `kind=url` with its address (content=\"http://localhost:5173\") so \
         the user sees the real app and their feedback maps to the actual code; use `kind=html` \
         only for one-off static mockups when no dev server exists. For iterative visual design, \
         use an id starting with `design:` (e.g. `design:landing-page`): each re-render of that \
         id adds a new version to the user's Design canvas, where they can step through versions, \
         compare them, and pin feedback that comes back to you as a `design-feedback` message."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": {"type": "string",
                        "enum": ["markdown","code","html","mermaid","table","image","url"],
                        "description": "Which artifact kind to render; one of the allowed enum values."},
                    "title": {"type": "string"},
                    "id": {"type": "string", "description": "stable id; re-rendering the same id replaces the artifact. Ids starting with `design:` version on the Design canvas instead of replacing."},
                    "content": {"type": "string",
                        "description": "primary payload: markdown/html/mermaid source, code text, base64 image data, or the localhost dev-server address (kind=url)"},
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
        Ok(ToolIntent {
            tool: "render".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: format!("render {kind}"),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let kind = str_arg(&args, "kind")?;
        let title = opt_str(&args, "title");
        let id = opt_str(&args, "id");
        let display = match kind.as_str() {
            "markdown" => Display::Markdown {
                text: str_arg(&args, "content")?,
                title: title.clone(),
                id,
            },
            "html" => Display::Html {
                html: str_arg(&args, "content")?,
                title: title.clone(),
                id,
            },
            "mermaid" => Display::Mermaid {
                source: str_arg(&args, "content")?,
                title: title.clone(),
                id,
            },
            "code" => Display::Code {
                lang: opt_str(&args, "lang").unwrap_or_else(|| "text".into()),
                filename: opt_str(&args, "filename"),
                text: str_arg(&args, "content")?,
                title: title.clone(),
                id,
            },
            "image" => Display::Image {
                mime: opt_str(&args, "mime").unwrap_or_else(|| "image/png".into()),
                data: str_arg(&args, "content")?,
                title: title.clone(),
                id,
            },
            "table" => {
                let columns: Vec<String> =
                    serde_json::from_value(args.get("columns").cloned().unwrap_or(json!([])))
                        .map_err(|e| ToolError::InvalidArgs(format!("columns: {e}")))?;
                let rows: Vec<Vec<String>> =
                    serde_json::from_value(args.get("rows").cloned().unwrap_or(json!([])))
                        .map_err(|e| ToolError::InvalidArgs(format!("rows: {e}")))?;
                Display::Table {
                    columns,
                    rows,
                    title: title.clone(),
                    id,
                }
            }
            "url" => {
                let url = str_arg(&args, "content")?;
                validate_local_url(&url)?;
                Display::Url {
                    url,
                    title: title.clone(),
                    id,
                }
            }
            other => return Err(ToolError::InvalidArgs(format!("unknown kind `{other}`"))),
        };
        let ack = match &title {
            Some(t) => format!("rendered {kind}: {t}"),
            None => format!("rendered {kind}"),
        };
        Ok(ToolOutput {
            content: ack,
            display: Some(display),
        })
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
        ToolCtx {
            workspace: std::env::temp_dir(),
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(crate::HostExecutor),
            call_id: "test".into(),
        }
    }

    #[tokio::test]
    async fn render_markdown_emits_markdown_display() {
        let out = RenderArtifact
            .execute(
                json!({"kind":"markdown","title":"Plan","content":"# Hello"}),
                &ctx(),
            )
            .await
            .unwrap();
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
        let out = RenderArtifact
            .execute(
                json!({"kind":"code","lang":"rust","filename":"a.rs","content":"fn x(){}"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(matches!(out.display, Some(Display::Code { .. })));
    }

    #[tokio::test]
    async fn render_table_uses_columns_and_rows() {
        let out = RenderArtifact
            .execute(
                json!({"kind":"table","columns":["a","b"],"rows":[["1","2"]]}),
                &ctx(),
            )
            .await
            .unwrap();
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
        let err = RenderArtifact
            .execute(json!({"kind":"wat","content":"x"}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn render_intent_is_read() {
        let i = RenderArtifact
            .intent(&json!({"kind":"markdown","content":"x"}))
            .unwrap();
        assert_eq!(i.access, Access::Read);
    }

    #[test]
    fn description_documents_the_design_canvas_convention() {
        let t = RenderArtifact;
        assert!(
            t.description().contains("design:"),
            "agents must learn the design canvas from the schema"
        );
        assert!(t.schema().parameters["properties"]["id"]["description"]
            .as_str()
            .unwrap()
            .contains("design:"));
    }

    #[tokio::test]
    async fn render_url_localhost_emits_url_display() {
        let out = RenderArtifact
            .execute(
                json!({"kind":"url","title":"App","content":"http://localhost:5173/app"}),
                &ctx(),
            )
            .await
            .unwrap();
        match out.display {
            Some(Display::Url { url, title, .. }) => {
                assert_eq!(url, "http://localhost:5173/app");
                assert_eq!(title.as_deref(), Some("App"));
            }
            other => panic!("expected Url, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn render_url_accepts_all_loopback_hosts() {
        for u in [
            "http://localhost:5173",
            "https://localhost/",
            "http://127.0.0.1:3000/x?y=1",
            "http://[::1]:8080/x",
            "http://LOCALHOST:80",
        ] {
            RenderArtifact
                .execute(json!({"kind":"url","content":u}), &ctx())
                .await
                .unwrap_or_else(|e| panic!("{u} should be accepted: {e:?}"));
        }
    }

    #[tokio::test]
    async fn render_url_rejects_non_local_targets() {
        for u in [
            "http://evil.com",
            "http://localhost.evil.com:5173",
            "http://localhost@evil.com/",
            "http://user@localhost:5173/",
            "ftp://localhost/",
            "localhost:5173",
            "http://[::1/x",
            // colon-in-userinfo bypass: authority splits on `:` giving "localhost", but
            // the real host is evil.com — must be caught by an `@` presence check
            "http://localhost:5173@evil.com",
            "https://127.0.0.1:8080@evil.com/",
        ] {
            let err = RenderArtifact
                .execute(json!({"kind":"url","content":u}), &ctx())
                .await
                .expect_err(&format!("{u} should be rejected"));
            assert!(matches!(err, ToolError::InvalidArgs(_)));
        }
    }

    #[test]
    fn description_steers_url_over_standalone_html() {
        let t = RenderArtifact;
        assert!(
            t.description().contains("kind=url") && t.description().contains("dev server"),
            "agents must learn to prefer the live dev server over standalone html"
        );
        let kinds = t.schema().parameters["properties"]["kind"]["enum"]
            .as_array()
            .unwrap()
            .clone();
        assert!(kinds.iter().any(|k| k == "url"));
    }
}
