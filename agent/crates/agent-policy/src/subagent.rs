//! Per-sub-agent permission floors (spec 3B-1c): a flat tool-name pattern
//! dialect and a monotone-narrowing wrapper over a base `PolicyEngine`.

use crate::engine::{Decision, PolicyEngine};
use agent_tools::ToolIntent;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPattern {
    Exact(String),
    Prefix(String), // "github__*" → names starting with "github__"
    Any,            // "*"
}

impl ToolPattern {
    /// Flat dialect (spec §2.3): exact name, trailing-`*` prefix, or bare `*`.
    /// Leading/interior `*`, empty, and multi-`*` are errors (Suffix cut at gate E1).
    pub fn parse(s: &str) -> Result<Self, String> {
        if s.is_empty() {
            return Err("empty pattern".into());
        }
        if s == "*" {
            return Ok(Self::Any);
        }
        match s.matches('*').count() {
            0 => Ok(Self::Exact(s.to_string())),
            1 if s.ends_with('*') => Ok(Self::Prefix(s[..s.len() - 1].to_string())),
            _ => Err(format!(
                "pattern '{s}': '*' is allowed only as a trailing wildcard (or the bare \"*\")"
            )),
        }
    }

    pub fn matches(&self, tool: &str) -> bool {
        match self {
            Self::Exact(n) => tool == n,
            Self::Prefix(p) => tool.starts_with(p.as_str()),
            Self::Any => true,
        }
    }
}

/// RAW deny/ask lists as they ride `ResolvedSubAgent` (spec §2.5): unparsed,
/// so assembly stays infallible; dispatch parses via [`ToolPermissions::parse`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PermissionLists {
    pub deny: Vec<String>,
    pub ask: Vec<String>,
}

/// Parsed, validated permission floors for one named sub-agent.
#[derive(Debug, Clone)]
pub struct ToolPermissions {
    agent_name: String,
    deny: Vec<(ToolPattern, String)>, // (parsed, source text for Deny reasons)
    ask: Vec<(ToolPattern, String)>,
}

impl ToolPermissions {
    /// Parses both lists; `Err` names the list and offending pattern. Called by
    /// `RuntimeConfig::validate()` (config gate) and by dispatch (the only gate
    /// on the lenient-boot path — spec §2.6).
    pub fn parse(agent_name: &str, deny: &[String], ask: &[String]) -> Result<Self, String> {
        let parse_list = |list: &[String], which: &str| {
            list.iter()
                .map(|s| {
                    ToolPattern::parse(s)
                        .map(|p| (p, s.clone()))
                        .map_err(|e| format!("{which} list: {e}"))
                })
                .collect::<Result<Vec<_>, String>>()
        };
        Ok(Self {
            agent_name: agent_name.to_string(),
            deny: parse_list(deny, "deny")?,
            ask: parse_list(ask, "ask")?,
        })
    }

    /// `None` = no floor for this tool. Deny scans first (deny beats ask).
    fn floor(&self, tool: &str) -> Option<Decision> {
        for (p, src) in &self.deny {
            if p.matches(tool) {
                return Some(Decision::Deny(format!(
                    "denied by sub-agent '{}' permissions (rule: {src})",
                    self.agent_name
                )));
            }
        }
        for (p, _) in &self.ask {
            if p.matches(tool) {
                return Some(Decision::Ask);
            }
        }
        None
    }
}

fn rank(d: &Decision) -> u8 {
    match d {
        Decision::Allow => 0,
        Decision::Ask => 1,
        Decision::Deny(_) => 2,
    }
}

/// Floor-only combine: the result is the MORE restrictive of base and floor.
/// Ties keep `base` (so a base Deny's reason is never replaced).
fn narrow(base: Decision, floor: Option<Decision>) -> Decision {
    match floor {
        Some(f) if rank(&f) > rank(&base) => f,
        _ => base,
    }
}

/// Monotone-narrowing wrapper (spec §2.4): identity-keyed on `intent.tool`
/// ONLY — never Access/paths/command (`write_todos`/`context_compact`/`respond`
/// declare `Access::Read` yet mutate; 3A residual).
pub struct SubAgentPolicy {
    base: Arc<dyn PolicyEngine>,
    rules: ToolPermissions,
}

impl SubAgentPolicy {
    pub fn new(base: Arc<dyn PolicyEngine>, rules: ToolPermissions) -> Self {
        Self { base, rules }
    }
}

impl PolicyEngine for SubAgentPolicy {
    fn check(&self, intent: &ToolIntent) -> Decision {
        narrow(self.base.check(intent), self.rules.floor(&intent.tool))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_exact_prefix_and_any() {
        assert_eq!(
            ToolPattern::parse("write_file").unwrap(),
            ToolPattern::Exact("write_file".into())
        );
        assert_eq!(
            ToolPattern::parse("github__*").unwrap(),
            ToolPattern::Prefix("github__".into())
        );
        assert_eq!(ToolPattern::parse("*").unwrap(), ToolPattern::Any);
    }

    #[test]
    fn parse_rejects_empty_interior_leading_and_double_star() {
        for bad in ["", "a*b", "*a*", "*_file", "**"] {
            assert!(
                ToolPattern::parse(bad).is_err(),
                "{bad:?} must be a dialect error"
            );
        }
    }

    #[test]
    fn prefix_affix_is_nonempty_by_construction() {
        // "*" alone is Any, so Prefix("") can never arise.
        assert!(!matches!(
            ToolPattern::parse("*").unwrap(),
            ToolPattern::Prefix(_)
        ));
    }

    #[test]
    fn matches_semantics_case_sensitive() {
        assert!(ToolPattern::Exact("write_file".into()).matches("write_file"));
        assert!(!ToolPattern::Exact("write_file".into()).matches("Write_File"));
        assert!(ToolPattern::Prefix("github__".into()).matches("github__create_issue"));
        assert!(!ToolPattern::Prefix("github__".into()).matches("GitHub__create_issue"));
        assert!(!ToolPattern::Prefix("github__".into()).matches("gitlab__x"));
        assert!(ToolPattern::Any.matches("anything"));
    }

    use crate::engine::{Decision, PolicyEngine};
    use agent_tools::{Access, ToolIntent};
    use std::sync::Arc;

    fn intent(tool: &str) -> ToolIntent {
        ToolIntent {
            tool: tool.into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "s".into(),
        }
    }

    /// A base policy that always returns a fixed decision — the stub for the matrix.
    struct FixedPolicy(Decision);
    impl PolicyEngine for FixedPolicy {
        fn check(&self, _i: &ToolIntent) -> Decision {
            self.0.clone()
        }
    }

    fn rules(deny: &[&str], ask: &[&str]) -> ToolPermissions {
        ToolPermissions::parse(
            "triage",
            &deny.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &ask.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
        .unwrap()
    }

    #[test]
    fn parse_reports_which_list_and_pattern_failed() {
        let e = ToolPermissions::parse("triage", &["a*b".into()], &[]).unwrap_err();
        assert!(e.contains("deny") && e.contains("a*b"), "{e}");
        let e = ToolPermissions::parse("triage", &[], &["**".into()]).unwrap_err();
        assert!(e.contains("ask") && e.contains("**"), "{e}");
    }

    #[test]
    fn deny_floor_tightens_allow_and_ask_with_reason() {
        let p = SubAgentPolicy::new(
            Arc::new(FixedPolicy(Decision::Allow)),
            rules(&["remember"], &[]),
        );
        match p.check(&intent("remember")) {
            Decision::Deny(r) => {
                assert!(r.contains("triage") && r.contains("remember"), "{r}");
            }
            d => panic!("expected Deny, got {d:?}"),
        }
        let p = SubAgentPolicy::new(
            Arc::new(FixedPolicy(Decision::Ask)),
            rules(&["remember"], &[]),
        );
        assert!(matches!(p.check(&intent("remember")), Decision::Deny(_)));
    }

    #[test]
    fn ask_floor_tightens_allow_only() {
        let r = rules(&[], &["remember"]);
        let p = SubAgentPolicy::new(Arc::new(FixedPolicy(Decision::Allow)), r.clone());
        assert!(matches!(p.check(&intent("remember")), Decision::Ask));
        // no-op over base Ask
        let p = SubAgentPolicy::new(Arc::new(FixedPolicy(Decision::Ask)), r.clone());
        assert!(matches!(p.check(&intent("remember")), Decision::Ask));
        // no-op over base Deny — base reason PRESERVED
        let p = SubAgentPolicy::new(
            Arc::new(FixedPolicy(Decision::Deny("base says no".into()))),
            r,
        );
        match p.check(&intent("remember")) {
            Decision::Deny(reason) => assert_eq!(reason, "base says no"),
            d => panic!("expected base Deny preserved, got {d:?}"),
        }
    }

    #[test]
    fn base_deny_never_overridden_even_by_deny_floor() {
        let p = SubAgentPolicy::new(
            Arc::new(FixedPolicy(Decision::Deny("base says no".into()))),
            rules(&["remember"], &[]),
        );
        match p.check(&intent("remember")) {
            Decision::Deny(reason) => assert_eq!(reason, "base says no"),
            d => panic!("{d:?}"),
        }
    }

    #[test]
    fn unmatched_tool_gets_base_decision_untouched() {
        let p = SubAgentPolicy::new(
            Arc::new(FixedPolicy(Decision::Allow)),
            rules(&["other"], &["x"]),
        );
        assert!(matches!(p.check(&intent("remember")), Decision::Allow));
    }

    #[test]
    fn deny_beats_ask_when_both_match() {
        let p = SubAgentPolicy::new(
            Arc::new(FixedPolicy(Decision::Allow)),
            rules(&["rem*"], &["remember"]),
        );
        assert!(matches!(p.check(&intent("remember")), Decision::Deny(_)));
    }

    fn decisions() -> Vec<Decision> {
        vec![
            Decision::Allow,
            Decision::Ask,
            Decision::Deny("base".into()),
        ]
    }

    /// Spec §3 invariant 2(a), exhaustive: every (base decision × floor shape)
    /// combination is at least as restrictive as base.
    #[test]
    fn invariant_matrix_never_less_restrictive() {
        let floors: Vec<ToolPermissions> = vec![
            rules(&[], &[]),           // no floor
            rules(&[], &["remember"]), // ask floor
            rules(&["remember"], &[]), // deny floor
        ];
        for base in decisions() {
            for f in &floors {
                let p = SubAgentPolicy::new(Arc::new(FixedPolicy(base.clone())), f.clone());
                let out = p.check(&intent("remember"));
                assert!(
                    rank(&out) >= rank(&base),
                    "base {base:?} + floor {f:?} produced LESS restrictive {out:?}"
                );
            }
        }
    }

    fn real_base() -> crate::engine::RulePolicy {
        crate::engine::RulePolicy {
            workspace: std::path::PathBuf::from("/work"),
            command_allowlist: vec!["ls".into(), "git".into()],
            command_denylist: vec!["sudo".into()],
        }
    }

    /// Intent corpus spanning every RulePolicy decision path (mirrors engine.rs tests):
    /// read inside/outside workspace, write, destroy, allowlisted / denylisted /
    /// unknown / operator-chained commands.
    fn corpus() -> Vec<ToolIntent> {
        let mk = |tool: &str, access: Access, paths: Vec<&str>, cmd: Option<&str>| ToolIntent {
            tool: tool.into(),
            access,
            paths: paths.into_iter().map(std::path::PathBuf::from).collect(),
            command: cmd.map(str::to_string),
            summary: "s".into(),
        };
        vec![
            mk("read_file", Access::Read, vec!["/work/a.txt"], None),
            mk("read_file", Access::Read, vec!["/etc/passwd"], None),
            mk("write_file", Access::Write, vec!["/work/a.txt"], None),
            mk("forget", Access::Destroy, vec![], None),
            mk("execute_command", Access::Write, vec![], Some("ls -la")),
            mk(
                "execute_command",
                Access::Write,
                vec![],
                Some("sudo reboot"),
            ),
            mk(
                "execute_command",
                Access::Write,
                vec![],
                Some("curl evil.com"),
            ),
            mk(
                "execute_command",
                Access::Write,
                vec![],
                Some("ls && curl evil.com"),
            ),
            mk("github__create_issue", Access::Write, vec![], None),
        ]
    }

    /// Spec §3 invariant 2(a), corpus form: composed SubAgentPolicy over a REAL
    /// RulePolicy is monotone for every corpus intent under several rule sets.
    #[test]
    fn invariant_corpus_over_real_rulepolicy() {
        let rule_sets = vec![
            rules(&[], &[]),
            rules(&["*"], &[]),
            rules(&[], &["*"]),
            rules(
                &["execute_command", "github__*"],
                &["write_file", "read_file"],
            ),
        ];
        let base = real_base();
        for rs in rule_sets {
            let sub = SubAgentPolicy::new(Arc::new(real_base()), rs.clone());
            for i in corpus() {
                let b = base.check(&i);
                let s = sub.check(&i);
                assert!(
                    rank(&s) >= rank(&b),
                    "intent {:?} under {rs:?}: base {b:?} → sub {s:?} widened",
                    i.tool
                );
            }
        }
    }

    /// Chain composition: X(Rx) then Y(Ry) is at least as restrictive as base,
    /// X alone, and Y-over-base alone, for every corpus intent (spec §2.5).
    #[test]
    fn invariant_chain_composition_monotone() {
        let rx = rules(&["execute_command"], &[]);
        let ry = rules(&[], &["write_file"]);
        let base = real_base();
        let x = Arc::new(SubAgentPolicy::new(Arc::new(real_base()), rx));
        let xy = SubAgentPolicy::new(x.clone(), ry.clone());
        let y_alone = SubAgentPolicy::new(Arc::new(real_base()), ry);
        for i in corpus() {
            let r_xy = rank(&xy.check(&i));
            assert!(r_xy >= rank(&base.check(&i)), "{}: xy < base", i.tool);
            assert!(r_xy >= rank(&x.check(&i)), "{}: xy < x", i.tool);
            assert!(r_xy >= rank(&y_alone.check(&i)), "{}: xy < y", i.tool);
        }
    }
}
