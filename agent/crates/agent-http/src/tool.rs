//! The `fetch_url` tool: GET a URL, gate the host, hard-block SSRF, return readable text.
use crate::policy::{HostDecision, NetworkPolicy};
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use url::Url;

pub struct FetchUrl {
    policy: NetworkPolicy,
}

impl FetchUrl {
    pub fn new(policy: NetworkPolicy) -> Self {
        Self { policy }
    }
}

// NOTE: the `guard` field, `with_guard`, and the USER_AGENT/MAX_* consts are added in
// Task 4, where they are first used — introducing them here would trip dead-code under
// `clippy -D warnings` at this task's gate.

/// Parse the `url` arg, accepting only http/https. Used by both `intent` and `execute`.
fn parse_url(args: &Value) -> Result<Url, ToolError> {
    let s = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArgs("missing 'url' string".into()))?;
    let url = Url::parse(s).map_err(|e| ToolError::InvalidArgs(format!("invalid url: {e}")))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(ToolError::InvalidArgs(format!(
            "unsupported scheme '{other}': only http/https are allowed"
        ))),
    }
}

#[async_trait]
impl Tool for FetchUrl {
    fn name(&self) -> &str {
        "fetch_url"
    }

    fn description(&self) -> &str {
        "Fetch a web page or document over HTTP(S) (GET only) and return its readable \
         text/JSON. Use for docs and reference pages. Non-allowlisted hosts require approval."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fetch_url".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Absolute http(s) URL to GET." }
                },
                "required": ["url"]
            }),
        }
    }

    fn intent(&self, args: &Value) -> Result<ToolIntent, ToolError> {
        let url = parse_url(args)?;
        let host = url
            .host_str()
            .ok_or_else(|| ToolError::InvalidArgs("url has no host".into()))?;
        let access = match self.policy.decide(host) {
            HostDecision::Allow => Access::Read,
            HostDecision::Ask => Access::Write,
        };
        Ok(ToolIntent {
            tool: "fetch_url".into(),
            access,
            paths: vec![],
            command: None,
            summary: format!("GET {url}"),
        })
    }

    async fn execute(&self, _args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        // Implemented in Task 4.
        Err(ToolError::Failed { message: "fetch_url execute not yet implemented".into(), stderr: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_policy::{Decision, PolicyEngine, RulePolicy};
    use agent_tools::{Access, Tool};
    use serde_json::json;
    use std::path::PathBuf;

    fn rule_policy() -> RulePolicy {
        RulePolicy {
            workspace: PathBuf::from("/work"),
            command_allowlist: vec![],
            command_denylist: vec![],
        }
    }

    #[test]
    fn schema_and_name_are_stable() {
        let t = FetchUrl::new(NetworkPolicy::new(&[]));
        assert_eq!(t.name(), "fetch_url");
        assert_eq!(t.schema().parameters["properties"]["url"]["type"], "string");
    }

    #[test]
    fn allowlisted_host_maps_to_read_and_rule_policy_allows() {
        let t = FetchUrl::new(NetworkPolicy::new(&["example.com".to_string()]));
        let intent = t.intent(&json!({"url": "https://example.com/page"})).unwrap();
        assert!(matches!(intent.access, Access::Read));
        assert!(intent.paths.is_empty());
        assert!(matches!(rule_policy().check(&intent), Decision::Allow));
    }

    #[test]
    fn unknown_host_maps_to_write_and_rule_policy_asks() {
        let t = FetchUrl::new(NetworkPolicy::new(&[]));
        let intent = t.intent(&json!({"url": "https://example.com/"})).unwrap();
        assert!(matches!(intent.access, Access::Write));
        assert!(matches!(rule_policy().check(&intent), Decision::Ask));
    }

    #[test]
    fn non_http_scheme_is_invalid_args() {
        let t = FetchUrl::new(NetworkPolicy::new(&[]));
        let err = t.intent(&json!({"url": "file:///etc/passwd"})).unwrap_err();
        assert!(matches!(err, agent_tools::ToolError::InvalidArgs(_)));
    }

    #[test]
    fn missing_url_is_invalid_args() {
        let t = FetchUrl::new(NetworkPolicy::new(&[]));
        assert!(matches!(t.intent(&json!({})).unwrap_err(), agent_tools::ToolError::InvalidArgs(_)));
    }
}
