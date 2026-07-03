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
    // ---- v2 genome axes (spec 2026-07-03 harness-evolve). All inherit-on-None. ----
    /// Runtime skills inlined into the system prompt (axis 5). None = inherit.
    #[serde(default)]
    pub active_skills: Option<Vec<String>>,
    /// Skill tree roots served to the loop (axis 5). None = inherit.
    #[serde(default)]
    pub skills_dirs: Option<Vec<String>>,
    /// Sampler overrides (axis 7a). None = inherit the harness default.
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub min_p: Option<f32>,
    /// Sub-agent policy (axis 4). None = inherit.
    #[serde(default)]
    pub subagents: Option<bool>,
    #[serde(default)]
    pub subagent_max_turns: Option<usize>,
    #[serde(default)]
    pub subagent_max_depth: Option<usize>,
    #[serde(default)]
    pub subagent_model: Option<crate::ModelRef>,
    /// Per-tool description overrides (axis 3). None = every tool's own schema text.
    #[serde(default)]
    pub tool_descriptions: Option<std::collections::HashMap<String, String>>,
    /// Ingestion cap (axis 8). None = the eval's historical "cap off" semantics.
    #[serde(default)]
    pub max_result_bytes: Option<usize>,
    /// Per-prompt turn budget. None = the driver default (12).
    #[serde(default)]
    pub max_turns: Option<usize>,
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
            active_skills: None,
            skills_dirs: None,
            temperature: None,
            top_p: None,
            top_k: None,
            min_p: None,
            subagents: None,
            subagent_max_turns: None,
            subagent_max_depth: None,
            subagent_model: None,
            tool_descriptions: None,
            max_result_bytes: None,
            max_turns: None,
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

    /// Overlay the RuntimeConfig-shaped v2 fields onto `cfg`; `None` inherits.
    /// NOTE: `max_result_bytes` applies via `offload_config()`, and
    /// `system_prompt`/`protocol` via their resolvers — apply those separately.
    pub fn apply_to(&self, cfg: &mut crate::RuntimeConfig) {
        if let Some(v) = self.temperature {
            cfg.temperature = v;
        }
        if let Some(v) = self.top_p {
            cfg.top_p = Some(v);
        }
        if let Some(v) = self.top_k {
            cfg.top_k = Some(v);
        }
        if let Some(v) = self.min_p {
            cfg.min_p = Some(v);
        }
        if let Some(v) = self.subagents {
            cfg.subagents = v;
        }
        if let Some(v) = self.subagent_max_turns {
            cfg.subagent_max_turns = v;
        }
        if let Some(v) = self.subagent_max_depth {
            cfg.subagent_max_depth = v;
        }
        if let Some(v) = &self.subagent_model {
            cfg.subagent_model = Some(v.clone());
        }
        if let Some(v) = &self.skills_dirs {
            cfg.skills_dirs = v.clone();
        }
        if let Some(v) = &self.active_skills {
            cfg.active_skills = v.clone();
        }
        if let Some(v) = self.max_turns {
            cfg.max_turns = v;
        }
        if let Some(v) = &self.tool_descriptions {
            cfg.tool_description_overrides = v.clone();
        }
    }

    /// The in-window offload thresholds for this candidate.
    pub fn offload_config(&self) -> OffloadConfig {
        OffloadConfig {
            error_min_bytes: self.error_min_bytes,
            output_min_bytes: self.output_min_bytes,
            keep_recent: self.keep_recent,
            exclude_tools: Vec::new(),
            // None = the eval's historical "ingestion cap off" semantics (the
            // context-evolve champion was validated without the cap). Some(n)
            // lets a candidate opt into a realistic cap (harness-evolve axis 8).
            max_result_bytes: self.max_result_bytes.unwrap_or(usize::MAX),
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

    #[test]
    fn v2_fields_default_to_none_and_parse_from_v1_json() {
        // A pre-v2 config (the existing field set only) must still deserialize.
        let json = serde_json::to_value(CandidateConfig::favorable(8192)).unwrap();
        let mut obj = json.as_object().unwrap().clone();
        for k in [
            "active_skills",
            "skills_dirs",
            "temperature",
            "top_p",
            "top_k",
            "min_p",
            "subagents",
            "subagent_max_turns",
            "subagent_max_depth",
            "subagent_model",
            "tool_descriptions",
            "max_result_bytes",
            "max_turns",
        ] {
            obj.remove(k);
        }
        let cc: CandidateConfig = serde_json::from_value(serde_json::Value::Object(obj)).unwrap();
        assert!(cc.temperature.is_none() && cc.subagent_model.is_none());
        assert!(cc.tool_descriptions.is_none() && cc.max_turns.is_none());
        // favorable leaves every v2 field None
        let f = CandidateConfig::favorable(8192);
        assert!(f.active_skills.is_none() && f.skills_dirs.is_none());
        assert!(f.subagents.is_none() && f.max_result_bytes.is_none());
    }

    #[test]
    fn apply_to_inherits_on_none_and_overrides_on_some() {
        let mut cfg = crate::RuntimeConfig::from_launch(
            "openai".into(),
            "http://x".into(),
            "m".into(),
            "native".into(),
            8192,
        );
        let baseline = cfg.clone();
        CandidateConfig::favorable(8192).apply_to(&mut cfg);
        assert_eq!(
            cfg, baseline,
            "all-None candidate must not touch the config"
        );

        let mut cc = CandidateConfig::favorable(8192);
        cc.temperature = Some(0.7);
        cc.top_k = Some(40);
        cc.subagents = Some(false);
        cc.subagent_max_turns = Some(4);
        cc.subagent_max_depth = Some(2);
        cc.subagent_model = Some(crate::ModelRef {
            model: Some("other".into()),
            ..Default::default()
        });
        cc.skills_dirs = Some(vec!["/skills".into()]);
        cc.active_skills = Some(vec!["sdlc".into()]);
        cc.max_turns = Some(30);
        cc.tool_descriptions = Some(
            [("read_file".to_string(), "OVERRIDE".to_string())]
                .into_iter()
                .collect(),
        );
        cc.apply_to(&mut cfg);
        assert_eq!(cfg.temperature, 0.7);
        assert_eq!(cfg.top_k, Some(40));
        assert!(!cfg.subagents);
        assert_eq!(cfg.subagent_max_turns, 4);
        assert_eq!(cfg.subagent_max_depth, 2);
        assert_eq!(
            cfg.subagent_model.as_ref().unwrap().model.as_deref(),
            Some("other")
        );
        assert_eq!(cfg.skills_dirs, vec!["/skills".to_string()]);
        assert_eq!(cfg.active_skills, vec!["sdlc".to_string()]);
        assert_eq!(cfg.max_turns, 30);
        assert_eq!(
            cfg.tool_description_overrides.get("read_file").unwrap(),
            "OVERRIDE"
        );
    }

    #[test]
    fn max_result_bytes_defaults_to_neutralized_and_overrides() {
        let f = CandidateConfig::favorable(8192);
        assert_eq!(f.offload_config().max_result_bytes, usize::MAX);
        let mut cc = CandidateConfig::favorable(8192);
        cc.max_result_bytes = Some(16 * 1024);
        assert_eq!(cc.offload_config().max_result_bytes, 16 * 1024);
    }
}
