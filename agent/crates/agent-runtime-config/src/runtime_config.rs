use crate::{backend_name_is_valid, default_allowlist, protocol_name_is_valid};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Commands ALWAYS denied regardless of user settings — defense-in-depth against
/// the model (or an injected settings frame), not against the operator. Intersected
/// into the effective denylist by `RuntimeConfig::effective_denylist`. Bare-program-name
/// catastrophes (sudo/su/doas, mkfs) are handled structurally & position-aware in
/// agent-policy (so `man mkfs` is not over-denied); only specific multi-token strings and
/// the forkbomb signature live here as substring backstop literals.
pub const HARD_FLOOR_DENYLIST: &[&str] = &["rm -rf /", ":(){", "dd if="];

/// Partial model override (spec 2026-07-02 sub-spec #3, G1): every `None`
/// inherits the primary config's value, so `{"model": "haiku"}` just works.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ModelRef {
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub claude_binary: Option<String>,
    /// Tool-call protocol for routed CHILD LOOPS ("native" | "prompted");
    /// compaction ignores it (plain completion).
    #[serde(default)]
    pub protocol: Option<String>,
}

impl ModelRef {
    /// Merge with the primary config: (backend, base_url, model, claude_binary).
    /// `primary_claude_binary` is a parameter because it lives on the frontends,
    /// not RuntimeConfig.
    pub fn resolve(
        &self,
        cfg: &RuntimeConfig,
        primary_claude_binary: &str,
    ) -> (String, String, String, String) {
        (
            self.backend.clone().unwrap_or_else(|| cfg.backend.clone()),
            self.base_url
                .clone()
                .unwrap_or_else(|| cfg.base_url.clone()),
            self.model.clone().unwrap_or_else(|| cfg.model.clone()),
            self.claude_binary
                .clone()
                .unwrap_or_else(|| primary_claude_binary.to_string()),
        )
    }
}

/// The editable runtime surface, persisted to disk and mirrored on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub backend: String, // "openai" | "claude-cli"
    pub base_url: String,
    pub model: String,
    pub protocol: String, // "native" | "prompted"
    pub command_allowlist: Vec<String>,
    pub command_denylist: Vec<String>, // user-editable portion ONLY (floor added separately)
    pub http_allow_hosts: Vec<String>, // hosts fetch_url may contact without approval
    pub temperature: f32,
    pub max_tokens: u32,
    pub max_turns: usize,
    pub context_limit: usize,
    #[serde(default = "default_max_tool_result_bytes")]
    pub max_tool_result_bytes: usize,
    /// Max tool calls executed concurrently within one turn.
    #[serde(default = "default_max_parallel_tools")]
    pub max_parallel_tools: usize,
    /// Shell commands run once after any turn in which a mutating (Write/Destroy)
    /// tool call succeeded; failures are fed back to the model. Empty = disabled.
    #[serde(default)]
    pub post_tool_validators: Vec<String>,
    #[serde(default = "default_true")]
    pub memory: bool,
    #[serde(default = "default_true")]
    pub subagents: bool,
    #[serde(default = "default_subagent_max_turns")]
    pub subagent_max_turns: usize,
    #[serde(default = "default_subagent_timeout_secs")]
    pub subagent_timeout_secs: u64,
    /// Model serving sub-agent (dispatch_agent) children; None = the session model.
    #[serde(default)]
    pub subagent_model: Option<ModelRef>,
    /// Model serving context compaction; None = the session model.
    #[serde(default)]
    pub compaction_model: Option<ModelRef>,
    /// Max sub-agent nesting depth (1 = children cannot dispatch). Read sites
    /// clamp to >= 1; "no sub-agents at all" is `subagents: false`.
    #[serde(default = "default_subagent_max_depth")]
    pub subagent_max_depth: usize,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub min_p: Option<f32>,
    #[serde(default)]
    pub presence_penalty: Option<f32>,
    #[serde(default)]
    pub repeat_penalty: Option<f32>,
    #[serde(default = "default_true")]
    pub enable_thinking: bool,
    #[serde(default)]
    pub preserve_thinking: bool,
    #[serde(default)]
    pub skills_dirs: Vec<String>,
    #[serde(default)]
    pub active_skills: Vec<String>,
    #[serde(default = "default_sandbox_mode")]
    pub sandbox_mode: String, // "off" | "auto" | "enforce"
    #[serde(default = "default_sandbox_image")]
    pub sandbox_image: String,
    #[serde(default)]
    pub sandbox_network: bool,
    #[serde(default = "default_sandbox_memory")]
    pub sandbox_memory: String,
    #[serde(default = "default_sandbox_cpus")]
    pub sandbox_cpus: String,
    #[serde(default = "default_sandbox_pids")]
    pub sandbox_pids: u32,
    #[serde(default)]
    pub sandbox_fsize: Option<String>,
    #[serde(default = "default_sandbox_tmp_size")]
    pub sandbox_tmp_size: String,
    #[serde(default)]
    pub sandbox_extra_rw: Vec<String>,
    #[serde(default)]
    pub sandbox_extra_ro: Vec<String>,
    #[serde(default = "default_true")]
    pub trace: bool,
    #[serde(default)]
    pub trace_dir: Option<String>,
    #[serde(default = "default_trace_max_mb")]
    pub trace_max_mb: u64,
}

/// All-optional mirror used only for on-disk merge: a file written by an older
/// build is missing newer fields, which then fall back to the flag-derived base.
#[derive(Debug, Default, Deserialize)]
struct PartialRuntimeConfig {
    backend: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    protocol: Option<String>,
    command_allowlist: Option<Vec<String>>,
    command_denylist: Option<Vec<String>>,
    http_allow_hosts: Option<Vec<String>>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    max_turns: Option<usize>,
    context_limit: Option<usize>,
    max_tool_result_bytes: Option<usize>,
    max_parallel_tools: Option<usize>,
    post_tool_validators: Option<Vec<String>>,
    memory: Option<bool>,
    subagents: Option<bool>,
    subagent_max_turns: Option<usize>,
    subagent_timeout_secs: Option<u64>,
    subagent_model: Option<ModelRef>,
    compaction_model: Option<ModelRef>,
    subagent_max_depth: Option<usize>,
    top_p: Option<f32>,
    top_k: Option<u32>,
    min_p: Option<f32>,
    presence_penalty: Option<f32>,
    repeat_penalty: Option<f32>,
    enable_thinking: Option<bool>,
    preserve_thinking: Option<bool>,
    skills_dirs: Option<Vec<String>>,
    active_skills: Option<Vec<String>>,
    sandbox_mode: Option<String>,
    sandbox_image: Option<String>,
    sandbox_network: Option<bool>,
    sandbox_memory: Option<String>,
    sandbox_cpus: Option<String>,
    sandbox_pids: Option<u32>,
    sandbox_fsize: Option<String>,
    sandbox_tmp_size: Option<String>,
    sandbox_extra_rw: Option<Vec<String>>,
    sandbox_extra_ro: Option<Vec<String>>,
    trace: Option<bool>,
    trace_dir: Option<String>,
    trace_max_mb: Option<u64>,
}

fn default_true() -> bool {
    true
}
fn default_max_tool_result_bytes() -> usize {
    agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES
}
fn default_max_parallel_tools() -> usize {
    agent_core::DEFAULT_MAX_PARALLEL_TOOLS
}
fn default_subagent_max_turns() -> usize {
    10
}
fn default_subagent_timeout_secs() -> u64 {
    600
}
fn default_subagent_max_depth() -> usize {
    1
}
fn default_sandbox_mode() -> String {
    "auto".into()
}
fn default_sandbox_image() -> String {
    "debian:stable-slim".into()
}
fn default_sandbox_memory() -> String {
    "2g".into()
}
fn default_sandbox_cpus() -> String {
    "2".into()
}
fn default_sandbox_pids() -> u32 {
    512
}
fn default_sandbox_tmp_size() -> String {
    "256m".into()
}
fn default_trace_max_mb() -> u64 {
    64
}

impl RuntimeConfig {
    /// Seed a config from launch flags + sensible defaults for the rest.
    pub fn from_launch(
        backend: String,
        base_url: String,
        model: String,
        protocol: String,
        context_limit: usize,
    ) -> Self {
        Self {
            backend,
            base_url,
            model,
            protocol,
            command_allowlist: default_allowlist(),
            command_denylist: Vec::new(),
            http_allow_hosts: Vec::new(),
            temperature: 0.2,
            // Completion budget per turn. Sized for a reasoning model that also
            // writes files in one turn (reasoning + a large tool-call args JSON);
            // 2048 truncated mid-tool-call. Context (262k) leaves ample room.
            max_tokens: 16384,
            max_turns: 25,
            context_limit,
            max_tool_result_bytes: default_max_tool_result_bytes(),
            max_parallel_tools: default_max_parallel_tools(),
            post_tool_validators: Vec::new(),
            memory: true,
            subagents: true,
            subagent_max_turns: default_subagent_max_turns(),
            subagent_timeout_secs: default_subagent_timeout_secs(),
            subagent_model: None,
            compaction_model: None,
            subagent_max_depth: default_subagent_max_depth(),
            top_p: None,
            top_k: None,
            min_p: None,
            presence_penalty: None,
            repeat_penalty: None,
            enable_thinking: true,
            preserve_thinking: false,
            skills_dirs: Vec::new(),
            active_skills: Vec::new(),
            sandbox_mode: default_sandbox_mode(),
            sandbox_image: default_sandbox_image(),
            sandbox_network: false,
            sandbox_memory: default_sandbox_memory(),
            sandbox_cpus: default_sandbox_cpus(),
            sandbox_pids: default_sandbox_pids(),
            sandbox_fsize: None,
            sandbox_tmp_size: default_sandbox_tmp_size(),
            sandbox_extra_rw: Vec::new(),
            sandbox_extra_ro: Vec::new(),
            trace: true,
            trace_dir: None,
            trace_max_mb: default_trace_max_mb(),
        }
    }

    /// Apply invariants that are corrections rather than errors (claude-cli is prompted-only).
    pub fn normalized(mut self) -> Self {
        if self.backend == "claude-cli" {
            self.protocol = "prompted".into();
        }
        self
    }

    /// Reject structurally invalid configs. Call `normalized()` first so the
    /// claude-cli protocol correction is not flagged as an error.
    pub fn validate(&self) -> Result<(), String> {
        if !backend_name_is_valid(&self.backend) {
            return Err(format!(
                "unknown backend '{}': use openai | claude-cli",
                self.backend
            ));
        }
        if !protocol_name_is_valid(&self.protocol) {
            return Err(format!(
                "unknown protocol '{}': use native | prompted",
                self.protocol
            ));
        }
        if self.backend == "claude-cli" && self.protocol != "prompted" {
            return Err(
                "claude-cli backend is prompted-only: protocol must be 'prompted' \
                        (normalized() applies this automatically)"
                    .into(),
            );
        }
        if self.backend == "openai" && self.base_url.trim().is_empty() {
            return Err("base_url is required for the openai backend".into());
        }
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err("temperature must be between 0.0 and 2.0".into());
        }
        if self.max_tokens == 0 {
            return Err("max_tokens must be > 0".into());
        }
        if self.max_turns == 0 {
            return Err("max_turns must be >= 1".into());
        }
        if self.max_parallel_tools == 0 {
            return Err("max_parallel_tools must be >= 1".into());
        }
        if self.context_limit < 1024 {
            return Err("context_limit must be >= 1024".into());
        }
        if let Some(v) = self.top_p {
            if !(0.0..=1.0).contains(&v) {
                return Err("top_p must be between 0.0 and 1.0".into());
            }
        }
        if let Some(v) = self.min_p {
            if !(0.0..=1.0).contains(&v) {
                return Err("min_p must be between 0.0 and 1.0".into());
            }
        }
        if let Some(v) = self.presence_penalty {
            if !(-2.0..=2.0).contains(&v) {
                return Err("presence_penalty must be between -2.0 and 2.0".into());
            }
        }
        if let Some(v) = self.repeat_penalty {
            if v <= 0.0 {
                return Err("repeat_penalty must be > 0.0".into());
            }
        }
        if !matches!(self.sandbox_mode.as_str(), "off" | "auto" | "enforce") {
            return Err(format!(
                "unknown sandbox_mode '{}': use off | auto | enforce",
                self.sandbox_mode
            ));
        }
        Ok(())
    }

    /// The denylist actually handed to `RulePolicy`: the immutable hard floor unioned
    /// with the user's editable entries (deduped, floor first).
    pub fn effective_denylist(&self) -> Vec<String> {
        let mut out: Vec<String> = HARD_FLOOR_DENYLIST.iter().map(|s| s.to_string()).collect();
        for d in &self.command_denylist {
            if !out.contains(d) {
                out.push(d.clone());
            }
        }
        out
    }

    fn merge(mut self, p: PartialRuntimeConfig) -> Self {
        if let Some(v) = p.backend {
            self.backend = v;
        }
        if let Some(v) = p.base_url {
            self.base_url = v;
        }
        if let Some(v) = p.model {
            self.model = v;
        }
        if let Some(v) = p.protocol {
            self.protocol = v;
        }
        if let Some(v) = p.command_allowlist {
            self.command_allowlist = v;
        }
        if let Some(v) = p.command_denylist {
            self.command_denylist = v;
        }
        if let Some(v) = p.http_allow_hosts {
            self.http_allow_hosts = v;
        }
        if let Some(v) = p.temperature {
            self.temperature = v;
        }
        if let Some(v) = p.max_tokens {
            self.max_tokens = v;
        }
        if let Some(v) = p.max_turns {
            self.max_turns = v;
        }
        if let Some(v) = p.context_limit {
            self.context_limit = v;
        }
        if let Some(v) = p.max_tool_result_bytes {
            self.max_tool_result_bytes = v;
        }
        if let Some(v) = p.max_parallel_tools {
            self.max_parallel_tools = v;
        }
        if let Some(v) = p.post_tool_validators {
            self.post_tool_validators = v;
        }
        if let Some(v) = p.memory {
            self.memory = v;
        }
        if let Some(v) = p.subagents {
            self.subagents = v;
        }
        if let Some(v) = p.subagent_max_turns {
            self.subagent_max_turns = v;
        }
        if let Some(v) = p.subagent_timeout_secs {
            self.subagent_timeout_secs = v;
        }
        if let Some(v) = p.subagent_model {
            self.subagent_model = Some(v);
        }
        if let Some(v) = p.compaction_model {
            self.compaction_model = Some(v);
        }
        if let Some(v) = p.subagent_max_depth {
            self.subagent_max_depth = v;
        }
        if let Some(v) = p.top_p {
            self.top_p = Some(v);
        }
        if let Some(v) = p.top_k {
            self.top_k = Some(v);
        }
        if let Some(v) = p.min_p {
            self.min_p = Some(v);
        }
        if let Some(v) = p.presence_penalty {
            self.presence_penalty = Some(v);
        }
        if let Some(v) = p.repeat_penalty {
            self.repeat_penalty = Some(v);
        }
        if let Some(v) = p.enable_thinking {
            self.enable_thinking = v;
        }
        if let Some(v) = p.preserve_thinking {
            self.preserve_thinking = v;
        }
        if let Some(v) = p.skills_dirs {
            self.skills_dirs = v;
        }
        if let Some(v) = p.active_skills {
            self.active_skills = v;
        }
        if let Some(v) = p.sandbox_mode {
            self.sandbox_mode = v;
        }
        if let Some(v) = p.sandbox_image {
            self.sandbox_image = v;
        }
        if let Some(v) = p.sandbox_network {
            self.sandbox_network = v;
        }
        if let Some(v) = p.sandbox_memory {
            self.sandbox_memory = v;
        }
        if let Some(v) = p.sandbox_cpus {
            self.sandbox_cpus = v;
        }
        if let Some(v) = p.sandbox_pids {
            self.sandbox_pids = v;
        }
        if let Some(v) = p.sandbox_fsize {
            self.sandbox_fsize = Some(v);
        }
        if let Some(v) = p.sandbox_tmp_size {
            self.sandbox_tmp_size = v;
        }
        if let Some(v) = p.sandbox_extra_rw {
            self.sandbox_extra_rw = v;
        }
        if let Some(v) = p.sandbox_extra_ro {
            self.sandbox_extra_ro = v;
        }
        if let Some(v) = p.trace {
            self.trace = v;
        }
        if let Some(v) = p.trace_dir {
            self.trace_dir = Some(v);
        }
        if let Some(v) = p.trace_max_mb {
            self.trace_max_mb = v;
        }
        self
    }

    /// Persist the full config (pretty JSON).
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Flag-derived `base`, overlaid per-field by the file at `path` if it parses.
    /// A missing file is silent (normal first run); an unreadable or malformed file
    /// leaves `base` unchanged but warns to stderr so the operator isn't surprised
    /// that an on-disk config was ignored.
    pub fn load_over(base: RuntimeConfig, path: &Path) -> RuntimeConfig {
        let (cfg, warning) = resolve_load(base, std::fs::read_to_string(path));
        if let Some(w) = warning {
            eprintln!(
                "warning: runtime config at {} {w}; using launch defaults",
                path.display()
            );
        }
        cfg
    }
}

/// Pure decision half of [`RuntimeConfig::load_over`]: given the result of reading the
/// config file, return the config to use and an optional operator warning. A missing
/// file (`NotFound`) is normal and yields no warning; any other read error or a
/// malformed file falls back to `base` *with* a warning.
fn resolve_load(
    base: RuntimeConfig,
    read: std::io::Result<String>,
) -> (RuntimeConfig, Option<String>) {
    match read {
        Ok(text) => match serde_json::from_str::<PartialRuntimeConfig>(&text) {
            Ok(p) => (base.merge(p), None),
            Err(e) => (base, Some(format!("is malformed JSON ({e})"))),
        },
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => (base, None),
        Err(e) => (base, Some(format!("could not be read ({e})"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> RuntimeConfig {
        RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        )
    }

    #[test]
    fn max_tool_result_bytes_defaults_and_merges() {
        // Old on-disk file without the field → default.
        let c = RuntimeConfig::from_launch(
            "openai".into(),
            "http://x".into(),
            "m".into(),
            "native".into(),
            8192,
        );
        assert_eq!(
            c.max_tool_result_bytes,
            agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES
        );

        // A serialized config missing the field deserializes to the default.
        let mut v: serde_json::Value = serde_json::to_value(&c).unwrap();
        v.as_object_mut().unwrap().remove("max_tool_result_bytes");
        let parsed: RuntimeConfig = serde_json::from_value(v).unwrap();
        assert_eq!(
            parsed.max_tool_result_bytes,
            agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES
        );
    }

    #[test]
    fn max_parallel_tools_defaults_and_merges() {
        // A JSON blob missing the field parses to the default (old files).
        let mut v = serde_json::to_value(base()).unwrap();
        v.as_object_mut().unwrap().remove("max_parallel_tools");
        let parsed: RuntimeConfig = serde_json::from_value(v).unwrap();
        assert_eq!(
            parsed.max_parallel_tools,
            agent_core::DEFAULT_MAX_PARALLEL_TOOLS,
            "serde default is DEFAULT_MAX_PARALLEL_TOOLS"
        );

        // An explicit value round-trips.
        let mut c = base();
        c.max_parallel_tools = 3;
        let round: RuntimeConfig =
            serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(round.max_parallel_tools, 3);
    }

    #[test]
    fn post_tool_validators_default_empty_and_merge() {
        let mut v = serde_json::to_value(base()).unwrap();
        v.as_object_mut().unwrap().remove("post_tool_validators");
        let parsed: RuntimeConfig = serde_json::from_value(v).unwrap();
        assert!(
            parsed.post_tool_validators.is_empty(),
            "serde default is empty"
        );

        let merged = base().merge(
            serde_json::from_str::<PartialRuntimeConfig>(
                r#"{"post_tool_validators": ["cargo check"]}"#,
            )
            .unwrap(),
        );
        assert_eq!(merged.post_tool_validators, vec!["cargo check".to_string()]);
    }

    #[test]
    fn max_parallel_tools_partial_file_overrides_only_that_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.json");
        std::fs::write(&path, r#"{"max_parallel_tools": 2}"#).unwrap();
        let b = base();
        let loaded = RuntimeConfig::load_over(b.clone(), &path);
        assert_eq!(loaded.max_parallel_tools, 2); // file wins
        assert_eq!(loaded.model, b.model); // absent fields fall back to base
    }

    #[test]
    fn validate_rejects_zero_max_parallel_tools() {
        let mut c = base();
        c.max_parallel_tools = 0;
        assert!(c.validate().unwrap_err().contains("max_parallel_tools"));
        c.max_parallel_tools = 1;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn max_tool_result_bytes_partial_file_overrides_only_that_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.json");
        std::fs::write(&path, r#"{"max_tool_result_bytes": 4096}"#).unwrap();
        let b = base();
        let loaded = RuntimeConfig::load_over(b.clone(), &path);
        assert_eq!(loaded.max_tool_result_bytes, 4096); // file wins
        assert_eq!(loaded.model, b.model); // absent fields fall back to base
        assert_eq!(loaded.max_tokens, b.max_tokens);
    }

    #[test]
    fn subagent_fields_default_and_survive_old_files() {
        // from_launch defaults.
        let c = base();
        assert!(c.subagents);
        assert_eq!(c.subagent_max_turns, 10);
        assert_eq!(c.subagent_timeout_secs, 600);

        // Old on-disk file without the fields -> defaults.
        let mut v = serde_json::to_value(&c).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("subagents");
        o.remove("subagent_max_turns");
        o.remove("subagent_timeout_secs");
        let parsed: RuntimeConfig = serde_json::from_value(v).unwrap();
        assert!(parsed.subagents);
        assert_eq!(parsed.subagent_max_turns, 10);
        assert_eq!(parsed.subagent_timeout_secs, 600);
    }

    #[test]
    fn subagent_fields_partial_file_overrides_only_those_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.json");
        std::fs::write(
            &path,
            r#"{"subagents": false, "subagent_max_turns": 4, "subagent_timeout_secs": 30}"#,
        )
        .unwrap();
        let b = base();
        let loaded = RuntimeConfig::load_over(b.clone(), &path);
        assert!(!loaded.subagents);
        assert_eq!(loaded.subagent_max_turns, 4);
        assert_eq!(loaded.subagent_timeout_secs, 30);
        assert_eq!(loaded.model, b.model); // absent fields fall back to base
    }

    #[test]
    fn model_routing_fields_default_none_and_depth_one() {
        let c = base();
        assert!(c.subagent_model.is_none());
        assert!(c.compaction_model.is_none());
        assert_eq!(c.subagent_max_depth, 1);
        // Old on-disk file without the fields -> defaults.
        let mut v = serde_json::to_value(&c).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("subagent_model");
        o.remove("compaction_model");
        o.remove("subagent_max_depth");
        let parsed: RuntimeConfig = serde_json::from_value(v).unwrap();
        assert!(parsed.subagent_model.is_none());
        assert_eq!(parsed.subagent_max_depth, 1);
    }

    #[test]
    fn model_routing_fields_partial_merge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.json");
        std::fs::write(
            &path,
            r#"{"subagent_model": {"model": "haiku"}, "subagent_max_depth": 2}"#,
        )
        .unwrap();
        let b = base();
        let loaded = RuntimeConfig::load_over(b.clone(), &path);
        let r = loaded.subagent_model.expect("merged");
        assert_eq!(r.model.as_deref(), Some("haiku"));
        assert!(r.backend.is_none()); // partial ModelRef: unset fields stay None
        assert_eq!(loaded.subagent_max_depth, 2);
        assert!(loaded.compaction_model.is_none()); // untouched
        assert_eq!(loaded.model, b.model);
    }

    #[test]
    fn from_launch_seeds_defaults() {
        let c = base();
        assert_eq!(c.temperature, 0.2);
        assert_eq!(c.max_tokens, 16384);
        assert_eq!(c.max_turns, 25);
        assert!(c.command_denylist.is_empty()); // floor is added by effective_denylist, not stored
        assert!(!c.command_allowlist.is_empty());
    }

    #[test]
    fn memory_defaults_on() {
        let c = RuntimeConfig::from_launch(
            "openai".into(),
            "http://x".into(),
            "m".into(),
            "native".into(),
            8192,
        );
        assert!(c.memory, "memory should default on");
    }

    #[test]
    fn memory_absent_in_json_defaults_true() {
        // A persisted config written before the field existed must deserialize to memory=true.
        let json = r#"{"backend":"openai","base_url":"http://x","model":"m","protocol":"native",
            "command_allowlist":[],"command_denylist":[],"http_allow_hosts":[],
            "temperature":0.2,"max_tokens":2048,"max_turns":25,"context_limit":8192}"#;
        let c: RuntimeConfig = serde_json::from_str(json).unwrap();
        assert!(c.memory);
    }

    #[test]
    fn memory_partial_file_overrides_only_that_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.json");
        std::fs::write(&path, r#"{"memory": false}"#).unwrap();
        let b = base();
        let loaded = RuntimeConfig::load_over(b.clone(), &path);
        assert!(!loaded.memory, "partial file must override memory");
        assert_eq!(loaded.model, b.model); // absent fields fall back to base
        assert!(loaded.subagents); // sibling bool untouched
    }

    #[test]
    fn full_saved_file_overrides_every_field_via_partial_merge() {
        // Structural guard: every RuntimeConfig field needs a PartialRuntimeConfig
        // mirror + merge arm. Every field here is flipped away from the launch
        // defaults, so a dropped mirror surfaces as an inequality after load_over;
        // the exhaustive struct literal makes adding a field without updating
        // this test a compile error.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("full.json");
        let c = RuntimeConfig {
            backend: "claude-cli".into(),
            base_url: "http://other:9999".into(),
            model: "m2".into(),
            protocol: "prompted".into(),
            command_allowlist: vec!["only".into()],
            command_denylist: vec!["nope".into()],
            http_allow_hosts: vec!["docs.rs".into()],
            temperature: 0.9,
            max_tokens: 4096,
            max_turns: 7,
            context_limit: 4096,
            max_tool_result_bytes: 1234,
            max_parallel_tools: 3,
            post_tool_validators: vec!["cargo check".into()],
            memory: false,
            subagents: false,
            subagent_max_turns: 4,
            subagent_timeout_secs: 30,
            subagent_model: Some(ModelRef {
                model: Some("haiku".into()),
                ..Default::default()
            }),
            compaction_model: Some(ModelRef {
                model: Some("mini".into()),
                ..Default::default()
            }),
            subagent_max_depth: 2,
            top_p: Some(0.9),
            top_k: Some(40),
            min_p: Some(0.05),
            presence_penalty: Some(0.5),
            repeat_penalty: Some(1.1),
            enable_thinking: false,
            preserve_thinking: true,
            skills_dirs: vec!["/s".into()],
            active_skills: vec!["greeter".into()],
            sandbox_mode: "enforce".into(),
            sandbox_image: "alpine:3".into(),
            sandbox_network: true,
            sandbox_memory: "1g".into(),
            sandbox_cpus: "1".into(),
            sandbox_pids: 64,
            sandbox_fsize: Some("10m".into()),
            sandbox_tmp_size: "64m".into(),
            sandbox_extra_rw: vec!["/rw".into()],
            sandbox_extra_ro: vec!["/ro".into()],
            trace: false,
            trace_dir: Some("/tmp/traces".into()),
            trace_max_mb: 8,
        };
        c.save(&path).unwrap();
        let loaded = RuntimeConfig::load_over(base(), &path);
        assert_eq!(loaded, c);
    }

    #[test]
    fn validate_accepts_a_good_config() {
        assert!(base().validate().is_ok());
    }

    #[test]
    fn validate_rejects_unknown_backend_and_protocol() {
        let mut c = base();
        c.backend = "bogus".into();
        assert!(c.validate().unwrap_err().contains("backend"));
        let mut c = base();
        c.protocol = "bogus".into();
        assert!(c.validate().unwrap_err().contains("protocol"));
    }

    #[test]
    fn validate_requires_base_url_for_openai() {
        let mut c = base();
        c.base_url = "  ".into();
        assert!(c.validate().unwrap_err().contains("base_url"));
    }

    #[test]
    fn validate_enforces_numeric_bounds() {
        let mut c = base();
        c.temperature = 3.0;
        assert!(c.validate().is_err());
        let mut c = base();
        c.max_tokens = 0;
        assert!(c.validate().is_err());
        let mut c = base();
        c.max_turns = 0;
        assert!(c.validate().is_err());
        let mut c = base();
        c.context_limit = 16;
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_claude_cli_with_native_protocol() {
        // Defense-in-depth: a caller that skips normalized() must not slip a
        // claude-cli + native config past validation.
        let mut c = base();
        c.backend = "claude-cli".into();
        c.protocol = "native".into();
        let err = c.validate().unwrap_err();
        assert!(
            err.contains("claude-cli"),
            "expected claude-cli protocol error, got: {err}"
        );
    }

    #[test]
    fn claude_cli_normalizes_protocol_to_prompted() {
        let mut c = base();
        c.backend = "claude-cli".into();
        c.protocol = "native".into();
        let n = c.normalized();
        assert_eq!(n.protocol, "prompted");
        assert!(n.validate().is_ok());
    }

    #[test]
    fn effective_denylist_always_contains_the_hard_floor() {
        let mut c = base();
        c.command_denylist = vec!["custom-bad".into()];
        let eff = c.effective_denylist();
        for floor in HARD_FLOOR_DENYLIST {
            assert!(eff.iter().any(|d| d == floor), "missing floor {floor}");
        }
        assert!(eff.iter().any(|d| d == "custom-bad"));
    }

    #[test]
    fn effective_denylist_floor_survives_empty_user_list() {
        let c = base(); // command_denylist empty
        assert!(c.effective_denylist().iter().any(|d| d == "rm -rf /"));
    }

    #[test]
    fn save_then_load_over_round_trips_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let mut c = base();
        c.model = "saved-model".into();
        c.temperature = 0.7;
        c.command_denylist = vec!["nope".into()];
        c.http_allow_hosts = vec!["docs.rs".into()];
        c.save(&path).unwrap();

        // A different base proves the file wins.
        let other = RuntimeConfig::from_launch(
            "openai".into(),
            "http://x".into(),
            "flag-model".into(),
            "native".into(),
            4096,
        );
        let loaded = RuntimeConfig::load_over(other, &path);
        assert_eq!(loaded.model, "saved-model");
        assert_eq!(loaded.temperature, 0.7);
        assert_eq!(loaded.command_denylist, vec!["nope".to_string()]);
        assert_eq!(loaded.http_allow_hosts, vec!["docs.rs".to_string()]);
    }

    #[test]
    fn load_over_returns_base_when_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");
        let b = base();
        assert_eq!(RuntimeConfig::load_over(b.clone(), &path), b);
    }

    #[test]
    fn load_over_falls_back_per_field_for_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.json");
        std::fs::write(&path, r#"{"model":"only-model"}"#).unwrap();
        let b = base();
        let loaded = RuntimeConfig::load_over(b.clone(), &path);
        assert_eq!(loaded.model, "only-model"); // file wins
        assert_eq!(loaded.backend, b.backend); // absent field falls back to base
        assert_eq!(loaded.max_tokens, b.max_tokens);
    }

    #[test]
    fn load_over_ignores_a_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        let b = base();
        assert_eq!(RuntimeConfig::load_over(b.clone(), &path), b);
    }

    #[test]
    fn resolve_load_is_silent_when_file_absent() {
        let absent = Err(std::io::Error::from(std::io::ErrorKind::NotFound));
        let (cfg, warn) = resolve_load(base(), absent);
        assert_eq!(cfg, base());
        assert!(warn.is_none(), "a missing file is normal, not a warning");
    }

    #[test]
    fn resolve_load_warns_on_other_io_errors() {
        let denied = Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        let (cfg, warn) = resolve_load(base(), denied);
        assert_eq!(cfg, base(), "still falls back to the launch base");
        assert!(
            warn.unwrap().contains("read"),
            "operator should be warned the file was unreadable"
        );
    }

    #[test]
    fn resolve_load_warns_on_malformed_json() {
        let (cfg, warn) = resolve_load(base(), Ok("not json".into()));
        assert_eq!(cfg, base());
        assert!(warn.unwrap().contains("malformed"));
    }

    #[test]
    fn resolve_load_is_silent_on_a_good_file() {
        let (_cfg, warn) = resolve_load(base(), Ok(r#"{"model":"m"}"#.into()));
        assert!(warn.is_none());
    }

    #[test]
    fn from_launch_seeds_thinking_and_sampling_defaults() {
        let c = base();
        assert!(c.enable_thinking);
        assert!(!c.preserve_thinking);
        assert!(c.top_p.is_none() && c.top_k.is_none() && c.min_p.is_none());
        assert!(c.presence_penalty.is_none() && c.repeat_penalty.is_none());
    }

    #[test]
    fn validate_enforces_sampling_bounds_only_when_set() {
        let mut c = base();
        c.top_p = Some(1.5);
        assert!(c.validate().is_err());
        let mut c = base();
        c.min_p = Some(-0.1);
        assert!(c.validate().is_err());
        let mut c = base();
        c.presence_penalty = Some(3.0);
        assert!(c.validate().is_err());
        let mut c = base();
        c.repeat_penalty = Some(0.0);
        assert!(c.validate().is_err());
        // None and in-range Some both pass; top_k has no bound.
        let mut c = base();
        c.top_p = Some(0.9);
        c.top_k = Some(40);
        c.repeat_penalty = Some(1.1);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn skills_fields_default_empty_and_round_trip() {
        let c = base();
        assert!(c.skills_dirs.is_empty());
        assert!(c.active_skills.is_empty());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let mut c = base();
        c.skills_dirs = vec!["/ws/.agent/skills".into()];
        c.active_skills = vec!["greeter".into()];
        c.save(&path).unwrap();

        // A different base proves the file wins; a partial file falls back per-field.
        let loaded = RuntimeConfig::load_over(base(), &path);
        assert_eq!(loaded.skills_dirs, vec!["/ws/.agent/skills".to_string()]);
        assert_eq!(loaded.active_skills, vec!["greeter".to_string()]);

        std::fs::write(&path, r#"{"model":"only-model"}"#).unwrap();
        let loaded = RuntimeConfig::load_over(base(), &path);
        assert_eq!(loaded.model, "only-model");
        assert!(loaded.skills_dirs.is_empty()); // absent field falls back to base
        assert!(loaded.active_skills.is_empty());
    }

    #[test]
    fn sandbox_defaults_and_round_trip() {
        let b = base();
        assert_eq!(b.sandbox_mode, "auto");
        assert_eq!(b.sandbox_image, "debian:stable-slim");
        assert!(!b.sandbox_network);
        assert_eq!(b.sandbox_memory, "2g");
        assert_eq!(b.sandbox_cpus, "2");
        assert_eq!(b.sandbox_pids, 512);
        assert!(b.sandbox_fsize.is_none());
        assert_eq!(b.sandbox_tmp_size, "256m");
        assert!(b.sandbox_extra_rw.is_empty());
        assert!(b.sandbox_extra_ro.is_empty());
    }

    #[test]
    fn trace_defaults_and_round_trip() {
        let b = base();
        assert!(b.trace, "trace should default on");
        assert!(b.trace_dir.is_none());
        assert_eq!(b.trace_max_mb, 64);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let mut c = base();
        c.trace = false;
        c.trace_dir = Some("/tmp/traces".into());
        c.trace_max_mb = 8;
        c.save(&path).unwrap();
        let loaded = RuntimeConfig::load_over(base(), &path);
        assert!(!loaded.trace);
        assert_eq!(loaded.trace_dir.as_deref(), Some("/tmp/traces"));
        assert_eq!(loaded.trace_max_mb, 8);

        // A file predating the trace fields keeps the base defaults (trace stays on).
        std::fs::write(&path, r#"{"model":"only-model"}"#).unwrap();
        let loaded = RuntimeConfig::load_over(base(), &path);
        assert!(loaded.trace);
        assert!(loaded.trace_dir.is_none());
        assert_eq!(loaded.trace_max_mb, 64);
    }

    #[test]
    fn validate_rejects_unknown_sandbox_mode() {
        let mut c = base();
        c.sandbox_mode = "bogus".into();
        assert!(c.validate().is_err());
        // valid modes should pass
        for mode in &["off", "auto", "enforce"] {
            let mut c = base();
            c.sandbox_mode = mode.to_string();
            assert!(c.validate().is_ok(), "mode '{mode}' should be valid");
        }
    }

    #[test]
    fn old_config_file_missing_sandbox_keeps_base_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.json");
        // Simulate an old config file that predates sandbox fields
        std::fs::write(&path, r#"{"backend":"openai","base_url":"http://localhost:8080","model":"m1","protocol":"native","command_allowlist":[],"command_denylist":[],"http_allow_hosts":[],"temperature":0.2,"max_tokens":2048,"max_turns":25,"context_limit":8192}"#).unwrap();
        let loaded = RuntimeConfig::load_over(base(), &path);
        // Sandbox fields must fall back to base defaults, not wipe to empty
        assert_eq!(loaded.sandbox_mode, "auto");
        assert_eq!(loaded.sandbox_image, "debian:stable-slim");
        assert!(!loaded.sandbox_network);
        assert_eq!(loaded.sandbox_pids, 512);
        assert!(loaded.sandbox_fsize.is_none());
        assert!(loaded.sandbox_extra_rw.is_empty());
        assert!(loaded.sandbox_extra_ro.is_empty());
    }

    #[test]
    fn sampling_round_trips_and_partial_file_keeps_base() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let mut c = base();
        c.top_k = Some(20);
        c.enable_thinking = false;
        c.preserve_thinking = true;
        c.save(&path).unwrap();
        let loaded = RuntimeConfig::load_over(base(), &path);
        assert_eq!(loaded.top_k, Some(20));
        assert!(!loaded.enable_thinking);
        assert!(loaded.preserve_thinking);

        // A file missing the new keys leaves the base values intact.
        std::fs::write(&path, r#"{"model":"only-model"}"#).unwrap();
        let loaded = RuntimeConfig::load_over(base(), &path);
        assert_eq!(loaded.model, "only-model");
        assert!(loaded.enable_thinking); // base default preserved
        assert!(loaded.top_k.is_none());
    }

    #[test]
    fn cli_default_config_does_not_over_deny_benign_catastrophe_names() {
        // The CLI seeds command_denylist = default_denylist(); the policy denylist is
        // effective_denylist() = HARD_FLOOR ∪ that. Regression (Finding B): benign catastrophe-name
        // arguments must NOT be hard-denied under the REAL assembled denylist — not just the floor.
        let mut c = base();
        c.command_denylist = crate::default_denylist();
        let deny = c.effective_denylist();
        assert!(agent_policy::hard_floor_violation("man mkfs", &deny).is_none());
        assert!(agent_policy::hard_floor_violation("man sudo", &deny).is_none());
        assert!(agent_policy::hard_floor_violation("cat sudoku.txt", &deny).is_none());
        // Direct catastrophe invocation is still denied (structural / boundary scan):
        assert!(agent_policy::hard_floor_violation("sudo reboot", &deny).is_some());
        assert!(agent_policy::hard_floor_violation("mkfs /dev/sda", &deny).is_some());
    }

    #[test]
    fn default_allowlist_is_subcommand_aware_for_exec_capable_programs() {
        let al = crate::default_allowlist();
        // Read-safe subcommands stay frictionless.
        assert!(agent_policy::is_auto_allowed(
            "git status --porcelain -b",
            &al
        ));
        // NOTE: `git diff HEAD~1` from the plan can't be auto-allowed — `~` is a
        // SHELL_SIGNIFICANT char (pre-existing agent-policy behavior, out of scope
        // here), so any command containing it goes to Ask. Use a shell-safe argument
        // that exercises the same "read-safe `git diff` subcommand is frictionless" intent.
        assert!(agent_policy::is_auto_allowed("git diff HEAD", &al));
        assert!(agent_policy::is_auto_allowed("git log --oneline -5", &al));
        assert!(agent_policy::is_auto_allowed(
            "cargo test -p agent-core",
            &al
        ));
        assert!(agent_policy::is_auto_allowed("ls -la", &al));
        // Destructive / unlisted forms are no longer auto-allowed (audit Top-10 #9).
        assert!(!agent_policy::is_auto_allowed("git push --force", &al));
        assert!(!agent_policy::is_auto_allowed("git reset --hard", &al));
        assert!(!agent_policy::is_auto_allowed("git clean -fdx", &al));
        assert!(!agent_policy::is_auto_allowed("git push", &al));
        assert!(!agent_policy::is_auto_allowed("git commit -m x", &al));
        assert!(!agent_policy::is_auto_allowed("cargo publish", &al));
        assert!(!agent_policy::is_auto_allowed("cargo install evil", &al));
    }

    #[test]
    fn execute_command_git_status_matches_git_status_tool_friction() {
        use agent_policy::{Decision, PolicyEngine, RulePolicy};
        let policy = RulePolicy {
            workspace: std::path::PathBuf::from("/work"),
            command_allowlist: crate::default_allowlist(),
            command_denylist: crate::default_denylist(),
        };
        // execute_command("git status …") — judged by the command branch.
        let via_shell = agent_tools::ToolIntent {
            tool: "execute_command".into(),
            access: agent_tools::Access::Write,
            paths: vec![],
            command: Some("git status --short --branch".into()),
            summary: "run".into(),
        };
        // git_status tool — judged by the access branch (Read, no paths, no command).
        let via_tool = agent_tools::ToolIntent {
            tool: "git_status".into(),
            access: agent_tools::Access::Read,
            paths: vec![],
            command: None,
            summary: "status".into(),
        };
        // Same operation, same friction on both routes (audit Tools-component asymmetry).
        assert!(matches!(policy.check(&via_shell), Decision::Allow));
        assert!(matches!(policy.check(&via_tool), Decision::Allow));
    }
}
