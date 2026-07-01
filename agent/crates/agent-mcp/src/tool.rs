use crate::client::{McpClient, RawTool};
use crate::config::Trust;
use agent_tools::{Access, Display, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// `server__tool`, sanitized to the model-tool-name charset `[a-zA-Z0-9_-]`.
pub fn namespaced_name(server: &str, tool: &str) -> String {
    fn clean(s: &str) -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }
    format!("{}__{}", clean(server), clean(tool))
}

/// A single MCP server tool wrapped as a native `Tool`.
pub struct McpTool {
    server: String,
    client: Arc<McpClient>,
    local_name: String, // server-local name used on the wire
    namespaced: String, // exposed to the model + registry
    description: String,
    input_schema: Value,
    trust: Trust,
}

impl McpTool {
    pub fn new(server: &str, client: Arc<McpClient>, raw: RawTool, trust: Trust) -> Self {
        let namespaced = namespaced_name(server, &raw.name);
        Self {
            server: server.to_string(),
            client,
            local_name: raw.name,
            namespaced,
            description: raw.description,
            input_schema: raw.input_schema,
            trust,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.namespaced
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.namespaced.clone(),
            description: self.description.clone(),
            parameters: self.input_schema.clone(),
        }
    }

    fn intent(&self, _args: &Value) -> Result<ToolIntent, ToolError> {
        // Trust is encoded onto the policy's Read/Write axis (zero policy change):
        // Ask → Write (RulePolicy asks); Allow → Read with empty paths (vacuously true → Allow).
        let access = match self.trust {
            Trust::Allow => Access::Read,
            Trust::Ask => Access::Write,
        };
        Ok(ToolIntent {
            tool: self.namespaced.clone(),
            access,
            paths: vec![],
            command: None,
            summary: format!(
                "MCP {}::{} (third-party server)",
                self.server, self.local_name
            ),
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let params = json!({"name": self.local_name, "arguments": args});
        let timeout = ctx.timeout.max(Duration::from_secs(1));
        let result = self
            .client
            .request("tools/call", params, timeout)
            .await
            .map_err(|e| ToolError::Failed {
                message: e.to_string(),
                stderr: None,
            })?;

        let text = result
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .map(|p| match p.get("type").and_then(Value::as_str) {
                        Some("text") => p
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        Some(other) => format!("[{other} content omitted]"),
                        None => String::new(),
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

        if result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(ToolError::Failed {
                message: text,
                stderr: None,
            });
        }

        Ok(ToolOutput {
            content: text.clone(),
            display: Some(Display::Text(text)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{McpClient, RawTool};
    use crate::transport::MockTransport;
    use agent_policy::{Decision, PolicyEngine, RulePolicy};
    use agent_tools::{Access, Tool};
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn raw() -> RawTool {
        RawTool {
            name: "create_issue".into(),
            description: "Create an issue".into(),
            input_schema: json!({"type":"object","properties":{"title":{"type":"string"}}}),
        }
    }

    fn client_that<F>(f: F) -> Arc<McpClient>
    where
        F: Fn(&serde_json::Value) -> Vec<serde_json::Value> + Send + Sync + 'static,
    {
        McpClient::new(Arc::new(MockTransport::scripted(f)))
    }

    fn policy() -> RulePolicy {
        RulePolicy {
            workspace: PathBuf::from("/work"),
            command_allowlist: vec![],
            command_denylist: vec![],
        }
    }

    #[test]
    fn name_is_namespaced_and_sanitized() {
        assert_eq!(
            namespaced_name("git hub", "create.issue"),
            "git_hub__create_issue"
        );
    }

    #[tokio::test]
    async fn schema_carries_namespaced_name_and_input_schema() {
        let tool = McpTool::new("github", client_that(|_| vec![]), raw(), Trust::Ask);
        let s = tool.schema();
        assert_eq!(s.name, "github__create_issue");
        assert_eq!(s.parameters["properties"]["title"]["type"], "string");
    }

    #[tokio::test]
    async fn ask_trust_maps_to_policy_ask() {
        let tool = McpTool::new("github", client_that(|_| vec![]), raw(), Trust::Ask);
        let intent = tool.intent(&json!({})).unwrap();
        assert_eq!(intent.access, Access::Write);
        assert!(intent.command.is_none());
        assert!(intent.paths.is_empty());
        assert!(matches!(policy().check(&intent), Decision::Ask));
    }

    #[tokio::test]
    async fn allow_trust_maps_to_policy_allow() {
        let tool = McpTool::new("fs", client_that(|_| vec![]), raw(), Trust::Allow);
        let intent = tool.intent(&json!({})).unwrap();
        assert_eq!(intent.access, Access::Read);
        assert!(intent.paths.is_empty());
        assert!(matches!(policy().check(&intent), Decision::Allow));
    }

    #[tokio::test]
    async fn execute_forwards_call_and_normalizes_text_content() {
        let tool = McpTool::new(
            "github",
            client_that(|req| {
                let id = req["id"].clone();
                assert_eq!(req["method"], "tools/call");
                assert_eq!(req["params"]["name"], "create_issue"); // server-local name, not namespaced
                vec![json!({"jsonrpc":"2.0","id":id,"result":{
                    "content":[{"type":"text","text":"issue #1 created"}],"isError":false}})]
            }),
            raw(),
            Trust::Ask,
        );
        let ctx = agent_tools::ToolCtx {
            workspace: PathBuf::from("/work"),
            timeout: std::time::Duration::from_secs(2),
            cancel: tokio_util::sync::CancellationToken::new(),
            // stopgap; Task 3 replaces this with the config-driven strategy
            sandbox: std::sync::Arc::new(agent_tools::HostExecutor),
        };
        let out = tool.execute(json!({"title":"bug"}), &ctx).await.unwrap();
        assert!(out.content.contains("issue #1 created"));
    }

    #[tokio::test]
    async fn execute_maps_is_error_to_tool_error() {
        let tool = McpTool::new(
            "github",
            client_that(|req| {
                let id = req["id"].clone();
                vec![json!({"jsonrpc":"2.0","id":id,"result":{
                    "content":[{"type":"text","text":"boom"}],"isError":true}})]
            }),
            raw(),
            Trust::Ask,
        );
        let ctx = agent_tools::ToolCtx {
            workspace: PathBuf::from("/work"),
            timeout: std::time::Duration::from_secs(2),
            cancel: tokio_util::sync::CancellationToken::new(),
            // stopgap; Task 3 replaces this with the config-driven strategy
            sandbox: std::sync::Arc::new(agent_tools::HostExecutor),
        };
        let err = tool.execute(json!({}), &ctx).await.unwrap_err();
        assert!(matches!(err, agent_tools::ToolError::Failed { .. }));
    }
}
