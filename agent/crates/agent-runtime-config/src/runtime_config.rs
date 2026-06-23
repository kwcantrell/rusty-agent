use crate::{default_allowlist, backend_name_is_valid, protocol_name_is_valid};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Commands ALWAYS denied regardless of user settings — defense-in-depth against
/// the model (or an injected settings frame), not against the operator. Intersected
/// into the effective denylist by `RuntimeConfig::effective_denylist`.
pub const HARD_FLOOR_DENYLIST: &[&str] = &["rm -rf /", "sudo", ":(){", "mkfs", "dd if="];

/// The editable runtime surface, persisted to disk and mirrored on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub backend: String,        // "openai" | "claude-cli"
    pub base_url: String,
    pub model: String,
    pub protocol: String,       // "native" | "prompted"
    pub command_allowlist: Vec<String>,
    pub command_denylist: Vec<String>, // user-editable portion ONLY (floor added separately)
    pub temperature: f32,
    pub max_tokens: u32,
    pub max_turns: usize,
    pub context_limit: usize,
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
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    max_turns: Option<usize>,
    context_limit: Option<usize>,
}

impl RuntimeConfig {
    /// Seed a config from launch flags + sensible defaults for the rest.
    pub fn from_launch(
        backend: String, base_url: String, model: String, protocol: String, context_limit: usize,
    ) -> Self {
        Self {
            backend, base_url, model, protocol,
            command_allowlist: default_allowlist(),
            command_denylist: Vec::new(),
            temperature: 0.2,
            max_tokens: 2048,
            max_turns: 25,
            context_limit,
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
        if let Some(v) = p.temperature { self.temperature = v; }
        if let Some(v) = p.max_tokens { self.max_tokens = v; }
        if let Some(v) = p.max_turns { self.max_turns = v; }
        if let Some(v) = p.context_limit { self.context_limit = v; }
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
        assert_eq!(c.max_tokens, 2048);
        assert_eq!(c.max_turns, 25);
        assert!(c.command_denylist.is_empty()); // floor is added by effective_denylist, not stored
        assert!(!c.command_allowlist.is_empty());
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
        assert!(c.effective_denylist().iter().any(|d| d == "sudo"));
    }

    #[test]
    fn save_then_load_over_round_trips_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rt.json");
        let mut c = base();
        c.model = "saved-model".into();
        c.temperature = 0.7;
        c.command_denylist = vec!["nope".into()];
        c.save(&path).unwrap();

        // A different base proves the file wins.
        let other = RuntimeConfig::from_launch(
            "openai".into(), "http://x".into(), "flag-model".into(), "native".into(), 4096);
        let loaded = RuntimeConfig::load_over(other, &path);
        assert_eq!(loaded.model, "saved-model");
        assert_eq!(loaded.temperature, 0.7);
        assert_eq!(loaded.command_denylist, vec!["nope".to_string()]);
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
}
