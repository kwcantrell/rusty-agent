use agent_core::OffloadConfig;
use serde::{Deserialize, Serialize};

/// A full set of context-management knobs across BOTH curation surfaces — in-window
/// curation (`agent-core`) and long-term memory (`agent-memory`) — that one eval run
/// is driven under. Tier-A optimization edits these fields; no rebuild required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateConfig {
    // in-window curation
    pub context_limit: usize,
    pub high_water_pct: f32,
    pub keep_recent: usize,
    pub error_min_bytes: usize,
    pub output_min_bytes: usize,
    pub recall_budget: usize,
    // long-term memory
    pub memory_enabled: bool,
    pub default_k: usize,
    pub relevance_threshold: f32,
    pub dedup_threshold: f32,
    pub forget_threshold: f32,
    pub max_recall_chars: usize,
    pub recall_token_budget: usize,
    pub auto_recall: bool,
    /// Override the base system prompt for this candidate. None = the harness
    /// default (inherit). A prompt-wording genome axis for the optimizer.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Override the tool-call protocol ("native" | "prompted"). None = inherit
    /// the harness default. A protocol-encoding genome axis.
    #[serde(default)]
    pub protocol: Option<String>,
}

impl CandidateConfig {
    /// The "context manager neutralized" reference: nothing offloads, nothing
    /// compacts, retrieval surfaces everything. Used as the favorable side of the
    /// two-sided admissibility test — if the model still fails here, the task is
    /// capability-bound, not context-bound.
    pub fn favorable(window: usize) -> Self {
        Self {
            context_limit: window,
            high_water_pct: 1.0,
            keep_recent: usize::MAX,
            error_min_bytes: usize::MAX,
            output_min_bytes: usize::MAX,
            recall_budget: 4096,
            memory_enabled: true,
            default_k: 20,
            relevance_threshold: 0.0,
            dedup_threshold: 0.95,
            forget_threshold: 0.85,
            max_recall_chars: 64 * 1024,
            recall_token_budget: 8192,
            auto_recall: true,
            system_prompt: None,
            protocol: None,
        }
    }

    /// The system prompt this candidate runs under: its override, else `default`.
    pub fn resolved_system_prompt<'a>(&'a self, default: &'a str) -> &'a str {
        self.system_prompt.as_deref().unwrap_or(default)
    }
    /// The protocol name this candidate runs under: its override, else `default`.
    pub fn resolved_protocol<'a>(&'a self, default: &'a str) -> &'a str {
        self.protocol.as_deref().unwrap_or(default)
    }

    /// The in-window offload thresholds for this candidate.
    pub fn offload_config(&self) -> OffloadConfig {
        OffloadConfig {
            error_min_bytes: self.error_min_bytes,
            output_min_bytes: self.output_min_bytes,
            keep_recent: self.keep_recent,
            exclude_tools: Vec::new(),
            // The eager ingestion cap (`max_result_bytes`, size-based and
            // age-agnostic via `select_oversized`) is intentionally NOT part
            // of the candidate genome — eval semantics predate it, and an
            // active context-evolve campaign validated its champion without
            // it. Neutralize it for the whole harness so `favorable` and every
            // candidate share the "no ingestion cap" baseline; `Default`'s
            // 16 KiB cap would otherwise silently apply regardless of the
            // `usize::MAX` age thresholds above.
            max_result_bytes: usize::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn favorable_disables_curation() {
        let f = CandidateConfig::favorable(196608);
        assert_eq!(f.context_limit, 196608);
        assert!(f.high_water_pct >= 1.0);
        assert_eq!(f.offload_config().output_min_bytes, usize::MAX);
        assert_eq!(f.offload_config().error_min_bytes, usize::MAX);
        // Ingestion cap is neutralized for the whole eval harness (not part of
        // the candidate genome) — otherwise a size-based cap would apply.
        assert_eq!(f.offload_config().max_result_bytes, usize::MAX);
        assert!(f.auto_recall && f.relevance_threshold <= 0.001);
    }
    #[test]
    fn round_trips_through_json() {
        let c = CandidateConfig::favorable(32000);
        let s = serde_json::to_string(&c).unwrap();
        let back: CandidateConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.context_limit, 32000);
    }
}

#[cfg(test)]
mod widening_tests {
    use super::*;

    #[test]
    fn new_fields_default_to_none_and_inherit() {
        let cc = CandidateConfig::favorable(8192);
        assert!(cc.system_prompt.is_none());
        assert!(cc.protocol.is_none());
        assert_eq!(cc.resolved_system_prompt("BASE"), "BASE");
        assert_eq!(cc.resolved_protocol("native"), "native");
    }

    #[test]
    fn overrides_win_over_default() {
        let mut cc = CandidateConfig::favorable(8192);
        cc.system_prompt = Some("CANDIDATE PROMPT".into());
        cc.protocol = Some("prompted".into());
        assert_eq!(cc.resolved_system_prompt("BASE"), "CANDIDATE PROMPT");
        assert_eq!(cc.resolved_protocol("native"), "prompted");
    }

    #[test]
    fn json_missing_new_fields_parses_to_none() {
        // A pre-widening config (existing field set only) must still deserialize.
        let json = serde_json::to_value(CandidateConfig::favorable(8192)).unwrap();
        let mut obj = json.as_object().unwrap().clone();
        obj.remove("system_prompt");
        obj.remove("protocol");
        let cc: CandidateConfig = serde_json::from_value(serde_json::Value::Object(obj)).unwrap();
        assert!(cc.system_prompt.is_none() && cc.protocol.is_none());
    }

    #[test]
    fn explicit_values_round_trip() {
        let mut cc = CandidateConfig::favorable(8192);
        cc.system_prompt = Some("P".into());
        cc.protocol = Some("prompted".into());
        let back: CandidateConfig =
            serde_json::from_str(&serde_json::to_string(&cc).unwrap()).unwrap();
        assert_eq!(back.system_prompt.as_deref(), Some("P"));
        assert_eq!(back.protocol.as_deref(), Some("prompted"));
    }
}
