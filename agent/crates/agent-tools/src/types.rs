use crate::SandboxStrategy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// JSON Schema object describing the arguments.
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Access {
    Read,
    Write,
    /// Third-party mutation pre-approved by config (MCP `Trust::Allow`): the
    /// approval gate auto-allows it (Read-like, workspace-bounded), but
    /// post-execution validation counts it as a mutation. Never Destroy-tier.
    TrustedWrite,
    /// Irreversible destruction (e.g. deleting a stored record). Never auto-allowed:
    /// the policy floor for Destroy is Ask — no allowlist or workspace-boundary rule
    /// may return Allow for it. The hard floor can still Deny it.
    Destroy,
}

#[derive(Debug, Clone)]
pub struct ToolIntent {
    pub tool: String,
    pub access: Access,
    pub paths: Vec<PathBuf>,
    pub command: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Display {
    Text(String),
    Diff {
        path: String,
        before: String,
        after: String,
    },
    Terminal {
        command: String,
        stdout: String,
        stderr: String,
        exit_code: i32,
    },
    Markdown {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    Code {
        lang: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    Html {
        html: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    Mermaid {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    Image {
        mime: String,
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Text returned to the model.
    pub content: String,
    /// Optional richer payload for UI rendering.
    pub display: Option<Display>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ToolError {
    #[error("denied: {0}")]
    Denied(String),
    #[error("timed out")]
    Timeout,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("failed: {message}")]
    Failed {
        message: String,
        stderr: Option<String>,
    },
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
}

/// Execution context handed to every tool.
pub struct ToolCtx {
    pub workspace: PathBuf,
    pub timeout: Duration,
    pub cancel: CancellationToken,
    pub sandbox: Arc<dyn SandboxStrategy>,
    /// The tool_call id this execution serves (`gate_tool` fills it from the
    /// model's call). Lineage root for sub-agent attribution (spec E2).
    pub call_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_schema_serializes_to_openai_function_shape() {
        let s = ToolSchema {
            name: "read_file".into(),
            description: "Read a file".into(),
            parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["name"], "read_file");
        assert_eq!(v["parameters"]["type"], "object");
    }

    #[test]
    fn tool_error_carries_context() {
        let e = ToolError::Failed {
            message: "boom".into(),
            stderr: Some("trace".into()),
        };
        match e {
            ToolError::Failed { message, stderr } => {
                assert_eq!(message, "boom");
                assert_eq!(stderr.as_deref(), Some("trace"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn display_markdown_round_trips_externally_tagged() {
        let d = Display::Markdown {
            text: "# Hi".into(),
            title: Some("Notes".into()),
            id: None,
        };
        let j = serde_json::to_string(&d).unwrap();
        assert!(j.starts_with("{\"Markdown\":"), "got {j}");
        assert!(j.contains("\"text\":\"# Hi\""));
        let back: Display = serde_json::from_str(&j).unwrap();
        assert!(matches!(back, Display::Markdown { .. }));
    }

    #[test]
    fn display_code_carries_lang_and_optional_filename() {
        let d = Display::Code {
            lang: "rust".into(),
            filename: Some("a.rs".into()),
            text: "fn x(){}".into(),
            title: None,
            id: Some("art-1".into()),
        };
        let j = serde_json::to_string(&d).unwrap();
        let back: Display = serde_json::from_str(&j).unwrap();
        match back {
            Display::Code {
                lang, filename, id, ..
            } => {
                assert_eq!(lang, "rust");
                assert_eq!(filename.as_deref(), Some("a.rs"));
                assert_eq!(id.as_deref(), Some("art-1"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn display_table_round_trips() {
        let d = Display::Table {
            columns: vec!["a".into(), "b".into()],
            rows: vec![vec!["1".into(), "2".into()]],
            title: None,
            id: None,
        };
        let j = serde_json::to_string(&d).unwrap();
        let back: Display = serde_json::from_str(&j).unwrap();
        assert!(matches!(back, Display::Table { .. }));
    }

    #[test]
    fn existing_diff_variant_json_is_unchanged() {
        let d = Display::Diff {
            path: "a".into(),
            before: "x".into(),
            after: "y".into(),
        };
        let j = serde_json::to_string(&d).unwrap();
        assert_eq!(j, r#"{"Diff":{"path":"a","before":"x","after":"y"}}"#);
    }
}
