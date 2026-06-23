use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// Per-server trust posture. Drives the policy decision via `McpTool::intent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trust {
    /// Require approval on every call (third-party code is untrusted).
    #[default]
    Ask,
    /// Auto-allow this server's tools (operator vouches for it).
    Allow,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerSpec {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub trust: Trust,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpServersConfig {
    #[serde(rename = "mcpServers", default)]
    pub servers: BTreeMap<String, McpServerSpec>,
}

impl McpServersConfig {
    /// Load the config file. A missing file yields an empty config with no warning
    /// (MCP is simply not enabled); an unreadable or malformed file yields an empty
    /// config plus an operator warning, never an abort.
    pub fn load_or_empty(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(cfg) => (cfg, None),
                Err(e) => (Self::default(), Some(format!("malformed mcp config ({e})"))),
            },
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(e) => (Self::default(), Some(format!("mcp config unreadable ({e})"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_server_block_and_defaults_trust_to_ask() {
        let json = r#"{ "mcpServers": {
            "filesystem": { "command": "npx", "args": ["-y", "srv", "/w"], "env": {"K":"V"} },
            "trusted":    { "command": "x", "trust": "allow" }
        }}"#;
        let cfg: McpServersConfig = serde_json::from_str(json).unwrap();
        let fs = &cfg.servers["filesystem"];
        assert_eq!(fs.command, "npx");
        assert_eq!(fs.args, vec!["-y", "srv", "/w"]);
        assert_eq!(fs.env["K"], "V");
        assert_eq!(fs.trust, Trust::Ask, "absent trust defaults to ask");
        assert_eq!(cfg.servers["trusted"].trust, Trust::Allow);
    }

    #[test]
    fn missing_file_is_empty_and_silent() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, warn) = McpServersConfig::load_or_empty(&dir.path().join("nope.json"));
        assert!(cfg.servers.is_empty());
        assert!(warn.is_none());
    }

    #[test]
    fn malformed_file_is_empty_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        let (cfg, warn) = McpServersConfig::load_or_empty(&path);
        assert!(cfg.servers.is_empty());
        assert!(warn.unwrap().contains("malformed"));
    }
}
