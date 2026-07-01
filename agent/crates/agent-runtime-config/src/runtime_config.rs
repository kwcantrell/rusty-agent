use crate::{default_allowlist, backend_name_is_valid, protocol_name_is_valid};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Commands ALWAYS denied regardless of user settings — defense-in-depth against
/// the model (or an injected settings frame), not against the operator. Intersected
/// into the effective denylist by `RuntimeConfig::effective_denylist`. Bare-program-name
/// catastrophes (sudo/su/doas, mkfs) are handled structurally & position-aware in
/// agent-policy (so `man mkfs` is not over-denied); only specific multi-token strings and
/// the forkbomb signature live here as substring backstop literals.
pub const HARD_FLOOR_DENYLIST: &[&str] = &["rm -rf /", ":(){", "dd if="];

/// The editable runtime surface, persisted to disk and mirrored on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub backend: String,        // "openai" | "claude-cli"
    pub base_url: String,
    pub model: String,
    pub protocol: String,       // "native" | "prompted"
    pub command_allowlist: Vec<String>,
    pub command_denylist: Vec<String>, // user-editable portion ONLY (floor added separately)
    pub http_allow_hosts: Vec<String>, // hosts fetch_url may contact without approval
    pub temperature: f32,
    pub max_tokens: u32,
    pub max_turns: usize,
    pub context_limit: usize,
    #[serde(default = "default_true")]
    pub memory: bool,
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
    pub sandbox_mode: String,        // "off" | "auto" | "enforce"
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
}

fn default_true() -> bool { true }
fn default_sandbox_mode() -> String { "auto".into() }
fn default_sandbox_image() -> String { "debian:stable-slim".into() }
fn default_sandbox_memory() -> String { "2g".into() }
fn default_sandbox_cpus() -> String { "2".into() }
fn default_sandbox_pids() -> u32 { 512 }
fn default_sandbox_tmp_size() -> String { "256m".into() }

impl RuntimeConfig {
    /// Seed a config from launch flags + sensible defaults for the rest.
    pub fn from_launch(
        backend: String, base_url: String, model: String, protocol: String, context_limit: usize,
    ) -> Self {
        Self {
            backend, base_url, model, protocol,
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
            memory: true,
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
            return Err(format!("unknown backend '{}': use openai | claude-cli", self.backend));
        }
        if !protocol_name_is_valid(&self.protocol) {
            return Err(format!("unknown protocol '{}': use native | prompted", self.protocol));
        }
        if self.backend == "claude-cli" && self.protocol != "prompted" {
            return Err("claude-cli backend is prompted-only: protocol must be 'prompted' \
                        (normalized() applies this automatically)".into());
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
        if self.context_limit < 1024 {
            return Err("context_limit must be >= 1024".into());
        }
        if let Some(v) = self.top_p { if !(0.0..=1.0).contains(&v) {
            return Err("top_p must be between 0.0 and 1.0".into()); } }
        if let Some(v) = self.min_p { if !(0.0..=1.0).contains(&v) {
            return Err("min_p must be between 0.0 and 1.0".into()); } }
        if let Some(v) = self.presence_penalty { if !(-2.0..=2.0).contains(&v) {
            return Err("presence_penalty must be between -2.0 and 2.0".into()); } }
        if let Some(v) = self.repeat_penalty { if v <= 0.0 {
            return Err("repeat_penalty must be > 0.0".into()); } }
        if !matches!(self.sandbox_mode.as_str(), "off" | "auto" | "enforce") {
            return Err(format!(
                "unknown sandbox_mode '{}': use off | auto | enforce", self.sandbox_mode
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
        if let Some(v) = p.backend { self.backend = v; }
        if let Some(v) = p.base_url { self.base_url = v; }
        if let Some(v) = p.model { self.model = v; }
        if let Some(v) = p.protocol { self.protocol = v; }
        if let Some(v) = p.command_allowlist { self.command_allowlist = v; }
        if let Some(v) = p.command_denylist { self.command_denylist = v; }
        if let Some(v) = p.http_allow_hosts { self.http_allow_hosts = v; }
        if let Some(v) = p.temperature { self.temperature = v; }
        if let Some(v) = p.max_tokens { self.max_tokens = v; }
        if let Some(v) = p.max_turns { self.max_turns = v; }
        if let Some(v) = p.context_limit { self.context_limit = v; }
        if let Some(v) = p.top_p { self.top_p = Some(v); }
        if let Some(v) = p.top_k { self.top_k = Some(v); }
        if let Some(v) = p.min_p { self.min_p = Some(v); }
        if let Some(v) = p.presence_penalty { self.presence_penalty = Some(v); }
        if let Some(v) = p.repeat_penalty { self.repeat_penalty = Some(v); }
        if let Some(v) = p.enable_thinking { self.enable_thinking = v; }
        if let Some(v) = p.preserve_thinking { self.preserve_thinking = v; }
        if let Some(v) = p.skills_dirs { self.skills_dirs = v; }
        if let Some(v) = p.active_skills { self.active_skills = v; }
        if let Some(v) = p.sandbox_mode { self.sandbox_mode = v; }
        if let Some(v) = p.sandbox_image { self.sandbox_image = v; }
        if let Some(v) = p.sandbox_network { self.sandbox_network = v; }
        if let Some(v) = p.sandbox_memory { self.sandbox_memory = v; }
        if let Some(v) = p.sandbox_cpus { self.sandbox_cpus = v; }
        if let Some(v) = p.sandbox_pids { self.sandbox_pids = v; }
        if let Some(v) = p.sandbox_fsize { self.sandbox_fsize = Some(v); }
        if let Some(v) = p.sandbox_tmp_size { self.sandbox_tmp_size = v; }
        if let Some(v) = p.sandbox_extra_rw { self.sandbox_extra_rw = v; }
        if let Some(v) = p.sandbox_extra_ro { self.sandbox_extra_ro = v; }
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
            eprintln!("warning: runtime config at {} {w}; using launch defaults", path.display());
        }
        cfg
    }
}

/// Pure decision half of [`RuntimeConfig::load_over`]: given the result of reading the
/// config file, return the config to use and an optional operator warning. A missing
/// file (`NotFound`) is normal and yields no warning; any other read error or a
/// malformed file falls back to `base` *with* a warning.
fn resolve_load(
    base: RuntimeConfig, read: std::io::Result<String>,
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
            "openai".into(), "http://localhost:8080".into(), "m1".into(), "native".into(), 8192)
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
            "openai".into(), "http://x".into(), "m".into(), "native".into(), 8192);
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
        assert!(err.contains("claude-cli"), "expected claude-cli protocol error, got: {err}");
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
            "openai".into(), "http://x".into(), "flag-model".into(), "native".into(), 4096);
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
        assert_eq!(loaded.backend, b.backend);   // absent field falls back to base
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
        assert!(warn.unwrap().contains("read"), "operator should be warned the file was unreadable");
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
        assert!(loaded.skills_dirs.is_empty());   // absent field falls back to base
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
}
