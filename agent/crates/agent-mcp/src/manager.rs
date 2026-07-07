use crate::client::McpClient;
use crate::config::{McpServerSpec, McpServersConfig};
use crate::tool::McpTool;
use crate::transport::StdioTransport;
use agent_tools::Tool;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ServerStatus {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
    /// Contract-lint violations across this server's tools (empty descriptions,
    /// undescribed required params). Warn-only — the tools still register.
    pub schema_warnings: usize,
    pub error: Option<String>,
}

/// Owns every connected server's client (and thus child process) for the agent's
/// lifetime, plus the wrapped tools and per-server status.
pub struct McpManager {
    clients: Vec<Arc<McpClient>>,
    tools: Vec<Arc<dyn Tool>>,
    statuses: Vec<ServerStatus>,
}

impl McpManager {
    /// Connect all configured servers concurrently, each under `connect_timeout`.
    /// A server that fails to spawn or handshake is recorded and skipped.
    pub async fn connect(
        cfg: &McpServersConfig,
        connect_timeout: Duration,
        workspace: std::path::PathBuf,
        sandbox: std::sync::Arc<dyn agent_tools::SandboxStrategy>,
    ) -> Self {
        let futs = cfg.servers.iter().map(|(name, spec)| {
            let name = name.clone();
            let spec = spec.clone();
            let workspace = workspace.clone();
            let sandbox = sandbox.clone();
            async move { connect_one(&name, &spec, connect_timeout, &workspace, &sandbox).await }
        });
        let results = futures_join_all(futs).await;

        let mut clients = Vec::new();
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        let mut statuses = Vec::new();
        for r in results {
            match r {
                Ok((name, client, server_tools)) => {
                    let warnings = schema_lint(&server_tools);
                    for w in &warnings {
                        tracing::warn!(target: "mcp", server = %name, violation = %w,
                            "MCP tool schema fails contract lint (tool still registered)");
                    }
                    statuses.push(ServerStatus {
                        name,
                        connected: true,
                        tool_count: server_tools.len(),
                        schema_warnings: warnings.len(),
                        error: None,
                    });
                    tools.extend(server_tools);
                    clients.push(client);
                }
                Err((name, e)) => {
                    if e.contains("sandbox unavailable") {
                        tracing::warn!(target: "mcp", server = %name, error = %e,
                            "server skipped: sandbox refused to launch it (exec degraded)");
                    } else {
                        tracing::warn!(target: "mcp", server = %name, error = %e, "server failed to connect");
                    }
                    statuses.push(ServerStatus {
                        name,
                        connected: false,
                        tool_count: 0,
                        schema_warnings: 0,
                        error: Some(e),
                    });
                }
            }
        }
        statuses.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            clients,
            tools,
            statuses,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_parts(tools: Vec<Arc<dyn Tool>>, statuses: Vec<ServerStatus>) -> Self {
        Self {
            clients: vec![],
            tools,
            statuses,
        }
    }

    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.clone()
    }

    pub fn summary_line(&self) -> String {
        if self.statuses.is_empty() {
            return "mcp: no servers configured".to_string();
        }
        let parts: Vec<String> = self
            .statuses
            .iter()
            .map(|s| {
                if s.connected {
                    if s.schema_warnings > 0 {
                        format!(
                            "{} \u{2713} ({} tools, {} schema warnings)",
                            s.name, s.tool_count, s.schema_warnings
                        )
                    } else {
                        format!("{} \u{2713} ({} tools)", s.name, s.tool_count)
                    }
                } else {
                    format!(
                        "{} \u{2717} ({})",
                        s.name,
                        s.error.as_deref().unwrap_or("error")
                    )
                }
            })
            .collect();
        format!("mcp: {}", parts.join(", "))
    }

    pub async fn shutdown(&self) {
        for c in &self.clients {
            c.close().await;
        }
    }
}

/// One warning string per contract-lint violation across a server's wrapped
/// tools: empty description, or a required param with no description
/// (`agent_tools::required_params_missing_description`). Warn-don't-reject.
fn schema_lint(tools: &[Arc<dyn Tool>]) -> Vec<String> {
    let mut warnings = Vec::new();
    for t in tools {
        let s = t.schema();
        if s.description.trim().is_empty() {
            warnings.push(format!("{}: empty description", s.name));
        }
        for p in agent_tools::required_params_missing_description(&s) {
            warnings.push(format!(
                "{}: required param `{p}` has no description",
                s.name
            ));
        }
    }
    warnings
}

/// Connect one server: spawn, handshake, discover, wrap tools.
async fn connect_one(
    name: &str,
    spec: &McpServerSpec,
    timeout: Duration,
    workspace: &std::path::Path,
    sandbox: &std::sync::Arc<dyn agent_tools::SandboxStrategy>,
) -> Result<(String, Arc<McpClient>, Vec<Arc<dyn Tool>>), (String, String)> {
    let name_owned = name.to_string();
    let spec_owned = spec.clone();
    let workspace_owned = workspace.to_path_buf();
    let sandbox = sandbox.clone();
    let attempt = async move {
        let transport = StdioTransport::spawn(&spec_owned, &workspace_owned, &sandbox)
            .map_err(|e| e.to_string())?;
        let client = McpClient::new(Arc::new(transport));
        client
            .initialize(timeout)
            .await
            .map_err(|e| e.to_string())?;
        let raw = client
            .list_tools(timeout)
            .await
            .map_err(|e| e.to_string())?;
        let tools: Vec<Arc<dyn Tool>> = raw
            .into_iter()
            .map(|r| {
                Arc::new(McpTool::new(
                    &name_owned,
                    client.clone(),
                    r,
                    spec_owned.trust,
                )) as Arc<dyn Tool>
            })
            .collect();
        Ok::<_, String>((client, tools))
    };
    match tokio::time::timeout(timeout, attempt).await {
        Ok(Ok((client, tools))) => Ok((name.to_string(), client, tools)),
        Ok(Err(e)) => Err((name.to_string(), e)),
        Err(_) => Err((name.to_string(), "connect timed out".to_string())),
    }
}

/// Minimal concurrent join without pulling extra deps: spawn each future and await.
async fn futures_join_all<F, T>(futs: impl IntoIterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = futs.into_iter().map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        match h.await {
            Ok(v) => out.push(v),
            Err(e) => tracing::error!(target: "mcp", error = %e, "connect task panicked"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{McpClient, RawTool};
    use crate::config::{McpServersConfig, Trust};
    use crate::tool::McpTool;
    use crate::transport::MockTransport;
    use std::time::Duration;

    fn host_sandbox() -> std::sync::Arc<dyn agent_tools::SandboxStrategy> {
        std::sync::Arc::new(agent_tools::HostExecutor)
    }

    fn mock_client() -> Arc<McpClient> {
        McpClient::new(Arc::new(MockTransport::scripted(|_| vec![])))
    }

    #[tokio::test]
    async fn schema_lint_flags_empty_description_and_undescribed_required_param() {
        let bad = RawTool {
            name: "create".into(),
            description: "   ".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"x": {"type": "string"}},
                "required": ["x"]
            }),
        };
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(McpTool::new(
            "srv",
            mock_client(),
            bad,
            Trust::Ask,
        ))];
        let w = schema_lint(&tools);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w.iter().any(|m| m.contains("empty description")), "{w:?}");
        assert!(w.iter().any(|m| m.contains("`x`")), "{w:?}");
    }

    #[tokio::test]
    async fn schema_lint_clean_schema_yields_no_warnings() {
        let clean = RawTool {
            name: "create".into(),
            description: "Create an issue".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"x": {"type": "string", "description": "the thing"}},
                "required": ["x"]
            }),
        };
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(McpTool::new(
            "srv",
            mock_client(),
            clean,
            Trust::Ask,
        ))];
        assert!(schema_lint(&tools).is_empty());
    }

    #[tokio::test]
    async fn summary_line_shows_schema_warnings_when_nonzero() {
        let mgr = McpManager::from_parts(
            vec![],
            vec![ServerStatus {
                name: "github".into(),
                connected: true,
                tool_count: 3,
                schema_warnings: 2,
                error: None,
            }],
        );
        assert_eq!(
            mgr.summary_line(),
            "mcp: github \u{2713} (3 tools, 2 schema warnings)"
        );
    }

    #[tokio::test]
    async fn empty_config_connects_nothing() {
        let mgr = McpManager::connect(
            &McpServersConfig::default(),
            Duration::from_secs(1),
            std::env::temp_dir(),
            host_sandbox(),
        )
        .await;
        assert!(mgr.tools().is_empty());
        assert_eq!(mgr.summary_line(), "mcp: no servers configured");
    }

    #[tokio::test]
    async fn degraded_sandbox_skips_server_not_fatal() {
        struct RefusingSandbox;
        impl agent_tools::SandboxStrategy for RefusingSandbox {
            fn launch(
                &self,
                _spec: agent_tools::CommandSpec,
            ) -> Result<agent_tools::SandboxedChild, agent_tools::SandboxError> {
                Err(agent_tools::SandboxError::Unavailable(
                    "docker unreachable (no daemon); command refused".into(),
                ))
            }
            fn describe(&self) -> agent_tools::SandboxDescriptor {
                agent_tools::SandboxDescriptor {
                    mode: agent_tools::Mode::Auto,
                    mechanism: "docker",
                    image: None,
                    network: false,
                    degraded: Some("no daemon".into()),
                }
            }
        }

        let mut cfg = McpServersConfig::default();
        cfg.servers.insert(
            "fs".into(),
            crate::config::McpServerSpec {
                command: "cat".into(),
                args: vec![],
                env: Default::default(),
                trust: crate::config::Trust::Ask,
            },
        );
        let mgr = McpManager::connect(
            &cfg,
            Duration::from_secs(1),
            std::env::temp_dir(),
            std::sync::Arc::new(RefusingSandbox),
        )
        .await;
        assert!(
            mgr.tools().is_empty(),
            "refused server must contribute no tools"
        );
        let line = mgr.summary_line();
        assert!(
            line.contains("fs \u{2717}") && line.contains("unavailable"),
            "skip must be recorded and name the sandbox refusal: {line}"
        );
    }

    #[test]
    fn summary_line_formats_mixed_statuses() {
        let mgr = McpManager::from_parts(
            vec![],
            vec![
                ServerStatus {
                    name: "filesystem".into(),
                    connected: true,
                    tool_count: 3,
                    schema_warnings: 0,
                    error: None,
                },
                ServerStatus {
                    name: "github".into(),
                    connected: false,
                    tool_count: 0,
                    schema_warnings: 0,
                    error: Some("timeout".into()),
                },
            ],
        );
        assert_eq!(
            mgr.summary_line(),
            "mcp: filesystem \u{2713} (3 tools), github \u{2717} (timeout)"
        );
    }

    #[tokio::test]
    async fn failed_spawn_is_reported_not_fatal() {
        let mut cfg = McpServersConfig::default();
        cfg.servers.insert(
            "broken".into(),
            crate::config::McpServerSpec {
                command: "definitely-not-a-real-binary-xyz".into(),
                args: vec![],
                env: Default::default(),
                trust: crate::config::Trust::Ask,
            },
        );
        let mgr = McpManager::connect(
            &cfg,
            Duration::from_secs(1),
            std::env::temp_dir(),
            host_sandbox(),
        )
        .await;
        assert!(mgr.tools().is_empty());
        assert!(mgr.summary_line().contains("broken \u{2717}"));
    }
}
