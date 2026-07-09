# Per-Sub-Agent Permissions (3B-1c) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Activate `SubAgentSpec.permissions` as a floor-only (monotone-narrowing) per-child
policy: `deny`/`ask` lists of flat tool-name patterns, enforced by a `SubAgentPolicy`
wrapper wrapped at dispatch over the caller's effective policy and threaded into nested
dispatch deps.

**Architecture:** New `agent-policy/src/subagent.rs` module (`ToolPattern`,
`PermissionLists`, `ToolPermissions`, `SubAgentPolicy`, `narrow`); config surface
`SubAgentPermissions` + `validate()` dialect gate + advisory `warnings()` lints; assembly
carries **raw** lists on `ResolvedSubAgent` (infallible); dispatch parses once per named
child and fails closed. Spec: `docs/superpowers/specs/2026-07-09-subagent-permissions-design.md`
(PLAN-READY at 666d210; panel + gate closed — Suffix patterns CUT, rule 6 is an advisory lint).

**Tech Stack:** Rust (Cargo workspace `agent/`), serde, tokio tests. No new dependencies.

## Global Constraints (spec §3 — do-not-regress)

- **Byte-identical empty path:** no `named_subagents`, rule-less named specs, empty-block
  specs, and `general-purpose` construct children exactly as 68e846b (same policy Arc).
- **Monotonicity:** `rank(SubAgentPolicy(base,R).check(i)) ≥ rank(base.check(i))` under
  `Allow < Ask < Deny` — policy-level AND wiring-level (grandchild) guards both required.
- **No approval-channel change; no `loop_.rs` change; no wire/event change; no `LoopConfig` field.**
- **Identity-keyed only:** floor consults `intent.tool` — never `Access`/paths/command.
- **Widening unrepresentable:** only `deny`/`ask` keys exist; `deny_unknown_fields` on the block.
- **Assembly stays infallible** — no parse, no `.expect()`; dispatch is the fail-closed gate.
- **Dialect (E1: Suffix CUT):** exact / trailing-`*` prefix / bare `*` only. Leading or
  interior `*`, `""`, `"**"` are parse errors.
- `file:line` anchors below are orientation only — **locate quoted code by content**.
- Conventional commits `type(scope): summary`. Each task ends with the named tests green.

---

### Task 0: Branch

**Files:** none (git only)

- [ ] **Step 1:** From clean `main`, create the feature branch:

```bash
cd /home/kalen/rust-agent-runtime && git checkout -b feature/subagent-permissions
```

---

### Task A1: `ToolPattern` (agent-policy)

**Files:**
- Create: `agent/crates/agent-policy/src/subagent.rs`
- Modify: `agent/crates/agent-policy/src/lib.rs` (add `mod subagent; pub use subagent::*;`)
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: nothing new.
- Produces: `pub enum ToolPattern { Exact(String), Prefix(String), Any }` with
  `pub fn parse(s: &str) -> Result<Self, String>` and
  `pub fn matches(&self, tool: &str) -> bool`. Task A2 builds on these exact signatures.

- [ ] **Step 1: Write the failing tests** (in the new `subagent.rs`):

```rust
//! Per-sub-agent permission floors (spec 3B-1c): a flat tool-name pattern
//! dialect and a monotone-narrowing wrapper over a base `PolicyEngine`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPattern {
    Exact(String),
    Prefix(String), // "github__*" → names starting with "github__"
    Any,            // "*"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_exact_prefix_and_any() {
        assert_eq!(ToolPattern::parse("write_file").unwrap(), ToolPattern::Exact("write_file".into()));
        assert_eq!(ToolPattern::parse("github__*").unwrap(), ToolPattern::Prefix("github__".into()));
        assert_eq!(ToolPattern::parse("*").unwrap(), ToolPattern::Any);
    }

    #[test]
    fn parse_rejects_empty_interior_leading_and_double_star() {
        for bad in ["", "a*b", "*a*", "*_file", "**"] {
            assert!(ToolPattern::parse(bad).is_err(), "{bad:?} must be a dialect error");
        }
    }

    #[test]
    fn prefix_affix_is_nonempty_by_construction() {
        // "*" alone is Any, so Prefix("") can never arise.
        assert!(!matches!(ToolPattern::parse("*").unwrap(), ToolPattern::Prefix(_)));
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
}
```

- [ ] **Step 2: Run to verify failure** —
  `cargo test -p agent-policy subagent` from `agent/`. Expected: compile error
  (`parse`/`matches` not defined).

- [ ] **Step 3: Implement**:

```rust
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
```

And in `lib.rs` (content today: `mod command; mod engine; pub use ...`):

```rust
mod subagent;
pub use subagent::*;
```

- [ ] **Step 4: Run to verify pass** — `cargo test -p agent-policy subagent`. Expected: 4 passed.

- [ ] **Step 5: Commit** —
  `git add agent/crates/agent-policy && git commit -m "feat(policy): ToolPattern flat dialect for sub-agent permission floors (3B-1c A1)"`

---

### Task A2: `PermissionLists` + `ToolPermissions` + `narrow` + `SubAgentPolicy`

**Files:**
- Modify: `agent/crates/agent-policy/src/subagent.rs`
- Test: same file

**Interfaces:**
- Consumes: `ToolPattern` (A1); `Decision`, `PolicyEngine` from `crate::engine`
  (`Decision::{Allow, Deny(String), Ask}`; `PolicyEngine::check(&self, &ToolIntent) -> Decision`).
- Produces (later tasks rely on these exact signatures):
  - `pub struct PermissionLists { pub deny: Vec<String>, pub ask: Vec<String> }`
    (derives `Debug, Clone, Default, PartialEq`) — the RAW carrier `ResolvedSubAgent` holds (C1).
  - `impl ToolPermissions { pub fn parse(agent_name: &str, deny: &[String], ask: &[String]) -> Result<ToolPermissions, String> }`
  - `pub struct SubAgentPolicy` with `pub fn new(base: Arc<dyn PolicyEngine>, rules: ToolPermissions) -> Self`,
    implementing `PolicyEngine`.

- [ ] **Step 1: Write the failing tests** (append to `subagent.rs` tests; the intent
  helper mirrors `engine.rs`'s test helper):

```rust
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
        let p = SubAgentPolicy::new(Arc::new(FixedPolicy(Decision::Allow)), rules(&["remember"], &[]));
        match p.check(&intent("remember")) {
            Decision::Deny(r) => {
                assert!(r.contains("triage") && r.contains("remember"), "{r}");
            }
            d => panic!("expected Deny, got {d:?}"),
        }
        let p = SubAgentPolicy::new(Arc::new(FixedPolicy(Decision::Ask)), rules(&["remember"], &[]));
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
        let p = SubAgentPolicy::new(Arc::new(FixedPolicy(Decision::Allow)), rules(&["other"], &["x"]));
        assert!(matches!(p.check(&intent("remember")), Decision::Allow));
    }

    #[test]
    fn deny_beats_ask_when_both_match() {
        let p = SubAgentPolicy::new(Arc::new(FixedPolicy(Decision::Allow)), rules(&["rem*"], &["remember"]));
        assert!(matches!(p.check(&intent("remember")), Decision::Deny(_)));
    }
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p agent-policy subagent`.
  Expected: compile errors (types not defined).

- [ ] **Step 3: Implement** (above the test module):

```rust
use crate::engine::{Decision, PolicyEngine};
use agent_tools::ToolIntent;
use std::sync::Arc;

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
```

Note: `agent-policy` already depends on `agent-tools` (see `engine.rs` imports) — no
Cargo.toml change.

- [ ] **Step 4: Run to verify pass** — `cargo test -p agent-policy`. Expected: all pass
  (including the pre-existing engine/command suites).

- [ ] **Step 5: Commit** —
  `git commit -am "feat(policy): ToolPermissions + SubAgentPolicy monotone floor (3B-1c A2)"`

---

### Task A3: Invariant tests — exhaustive matrix, real-`RulePolicy` corpus, chain composition

**Files:**
- Modify: `agent/crates/agent-policy/src/subagent.rs` (tests only)

**Interfaces:**
- Consumes: everything from A1/A2; `RulePolicy` from `crate::engine`.
- Produces: nothing new — this is invariant 2(a) of the spec (§3), pinned.

- [ ] **Step 1: Write the tests** (append to the test module):

```rust
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
            rules(&[], &[]),            // no floor
            rules(&[], &["remember"]),  // ask floor
            rules(&["remember"], &[]),  // deny floor
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
            mk("execute_command", Access::Write, vec![], Some("sudo reboot")),
            mk("execute_command", Access::Write, vec![], Some("curl evil.com")),
            mk("execute_command", Access::Write, vec![], Some("ls && curl evil.com")),
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
            rules(&["execute_command", "github__*"], &["write_file", "read_file"]),
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
```

- [ ] **Step 2: Run** — `cargo test -p agent-policy subagent`. Expected: all pass
  (these pin already-implemented behavior; if any fails, the A2 implementation is wrong —
  fix A2, do not weaken the test).

- [ ] **Step 3: Commit** —
  `git commit -am "test(policy): monotonicity invariant matrix + RulePolicy corpus + chain (3B-1c A3)"`

---

### Task B1: Config surface — `SubAgentPermissions` + `validate()`

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs`
- Test: same file's test module (helpers `cfg_with(specs)` / `spec(name)` near L1498)

**Interfaces:**
- Consumes: `agent_policy::ToolPermissions::parse` (A2).
- Produces: `pub struct SubAgentPermissions { pub deny: Vec<String>, pub ask: Vec<String> }`
  (serde, `deny_unknown_fields`); `SubAgentSpec.permissions: Option<SubAgentPermissions>`
  (was `Option<serde_json::Value>`). Tasks B2/C1 use these exact names.

- [ ] **Step 1: Change the struct.** In `runtime_config.rs`, above `SubAgentSpec`, add:

```rust
/// Floor-only permission lists (spec 3B-1c §2.2). Only these two keys exist —
/// widening is unrepresentable; an unknown key (e.g. `allow`) trips
/// `deny_unknown_fields`, which under the lenient overlay discards the WHOLE
/// file (flag-derived base, empty registry — no named children at all).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubAgentPermissions {
    /// Patterns whose matching tools floor at Deny.
    #[serde(default)]
    pub deny: Vec<String>,
    /// Patterns whose matching tools floor at Ask.
    #[serde(default)]
    pub ask: Vec<String>,
}
```

Then change the `SubAgentSpec` field (currently `/// → 3B-1c (per-sub-agent permissions).`
over `pub permissions: Option<serde_json::Value>`) to:

```rust
    /// Floor-only per-child permissions (3B-1c): can only TIGHTEN the base
    /// policy for this child and its descendants, never loosen it.
    #[serde(default)]
    pub permissions: Option<SubAgentPermissions>,
```

- [ ] **Step 2: Narrow the reserved-field rejection in `validate()`.** Locate by content
  the block `// permissions/middleware/skills remain inert (3B-1c / dropped, spec §0).`
  (near L557) and replace it with:

```rust
            // middleware/skills remain inert (dropped, 3B-1 spec §9).
            if s.middleware.is_some() || s.skills.is_some() {
                return Err(format!(
                    "named_subagents['{}']: middleware/skills are not supported",
                    s.name
                ));
            }
            // 3B-1c: permissions accepted; dialect-validated (floor-only lists).
            if let Some(p) = &s.permissions {
                agent_policy::ToolPermissions::parse(&s.name, &p.deny, &p.ask)
                    .map_err(|e| format!("named_subagents['{}']: permissions {e}", s.name))?;
            }
```

(`agent-runtime-config` already depends on `agent-policy` — see `assemble.rs`'s
`use agent_policy::{ApprovalChannel, RulePolicy};`. Add `use` or fully qualify as shown.)

- [ ] **Step 3: Fix the two existing tests this breaks — BOTH sites.**
  `validate_rejects_reserved_fields_and_model_endpoint_override` (near L1555) and
  `accepts_valid_response_format_still_rejects_others` (near L1608) each set
  `s.permissions = Some(serde_json::json!({}))` and assert `is_err()` — neither compiles
  after the type change. In BOTH test bodies, switch to the new type and flip the
  empty-permissions assertion `is_err()` → `is_ok()`:

```rust
        // permissions is now accepted (3B-1c); middleware/skills still rejected.
        let mut sp = spec("p");
        sp.permissions = Some(SubAgentPermissions::default());
        assert!(cfg_with(vec![sp]).validate().is_ok());
        let mut sm = spec("m");
        sm.middleware = Some(vec!["x".into()]);
        assert!(cfg_with(vec![sm]).validate().is_err());
        let mut sk = spec("k");
        sk.skills = Some(vec!["x".into()]);
        assert!(cfg_with(vec![sk]).validate().is_err());
```

Also update the `spec(name)` helper's `permissions: None` (compiles unchanged — `None`
infers the new type) and any other `permissions: None` literal (grep the crate; all stay
`None`).

- [ ] **Step 4: Write the new validation tests** (same test module):

```rust
    #[test]
    fn permissions_dialect_validated_per_spec() {
        // good: exact + prefix + bare *
        let mut s = spec("triage");
        s.permissions = Some(SubAgentPermissions {
            deny: vec!["execute_command".into(), "github__*".into()],
            ask: vec!["*".into()],
        });
        assert!(cfg_with(vec![s]).validate().is_ok());

        // bad: leading * (suffix form — cut at gate E1); error names spec + pattern
        let mut s = spec("triage");
        s.permissions = Some(SubAgentPermissions {
            deny: vec!["*_file".into()],
            ask: vec![],
        });
        let e = cfg_with(vec![s]).validate().unwrap_err();
        assert!(e.contains("triage") && e.contains("*_file"), "{e}");

        // bad: interior *
        let mut s = spec("triage");
        s.permissions = Some(SubAgentPermissions {
            deny: vec![],
            ask: vec!["a*b".into()],
        });
        assert!(cfg_with(vec![s]).validate().is_err());

        // duplicates accepted (spec §2.2 rule 4 — harmless, 3B-1b residual posture)
        let mut s = spec("triage");
        s.permissions = Some(SubAgentPermissions {
            deny: vec!["probe".into(), "probe".into()],
            ask: vec![],
        });
        assert!(cfg_with(vec![s]).validate().is_ok());
    }

    #[test]
    fn permissions_empty_block_is_valid() {
        let mut s = spec("triage");
        s.permissions = Some(SubAgentPermissions::default());
        assert!(cfg_with(vec![s]).validate().is_ok());
    }

    /// Widening is unrepresentable: an `allow` key fails the block's serde parse
    /// (`deny_unknown_fields`). NOTE the real-world consequence (spec §2.2 rule 3):
    /// in the lenient overlay this discards the WHOLE file — flag-derived base,
    /// EMPTY registry (no named children at all) — never an unfloored named child.
    #[test]
    fn permissions_unknown_key_fails_block_parse() {
        let r: Result<SubAgentPermissions, _> =
            serde_json::from_str(r#"{"deny": [], "allow": ["execute_command"]}"#);
        assert!(r.is_err(), "an `allow` key must be unrepresentable");
    }
```

- [ ] **Step 5: Run** — `cargo test -p agent-runtime-config`. Expected: config suite
  green. (`agent-core` won't compile against this yet only if it names the old type — it
  doesn't; `ResolvedSubAgent` gains its field in C1. `cargo build` at workspace root
  should still succeed; verify with `cargo build` from `agent/`.)

- [ ] **Step 6: Commit** —
  `git commit -am "feat(config): SubAgentPermissions accepted + dialect-validated (3B-1c B1)"`

---

### Task B2: Advisory `warnings()` lints

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (`pub fn warnings`, near L585)
- Test: same file

**Interfaces:**
- Consumes: `SubAgentPermissions` (B1), `agent_policy::ToolPattern` (A1).
- Produces: three non-fatal lint strings (spec §2.2 rule 8). Nothing downstream consumes
  them programmatically — frontends surface them (CLI stderr, server `tracing::warn`),
  same as the existing interpreter-allowlist lint.

- [ ] **Step 1: Write the failing tests:**

```rust
    fn warn_of(specs: Vec<SubAgentSpec>) -> Vec<String> {
        cfg_with(specs).warnings()
    }

    #[test]
    fn lint_inert_exact_rule_vs_tools_allowlist() {
        let mut s = spec("triage");
        s.tools = Some(vec!["read_file".into()]);
        s.permissions = Some(SubAgentPermissions {
            deny: vec!["execute_command".into()],
            ask: vec![],
        });
        let w = warn_of(vec![s]);
        assert!(
            w.iter().any(|m| m.contains("triage") && m.contains("execute_command") && m.contains("inert")),
            "{w:?}"
        );
        // NOT linted: exact rule naming an allowlisted tool, prefix rules, or
        // the always-available implicit child tools.
        let mut s = spec("triage");
        s.tools = Some(vec!["read_file".into()]);
        s.permissions = Some(SubAgentPermissions {
            deny: vec!["read_file".into(), "github__*".into(), "context_compact".into()],
            ask: vec![],
        });
        assert!(warn_of(vec![s]).is_empty());
    }

    #[test]
    fn lint_unsanitizable_mcp_affix() {
        let mut s = spec("triage");
        s.permissions = Some(SubAgentPermissions {
            deny: vec!["git.hub__*".into()],
            ask: vec![],
        });
        let w = warn_of(vec![s]);
        assert!(
            w.iter().any(|m| m.contains("git.hub__*") && m.contains("never match")),
            "{w:?}"
        );
    }

    #[test]
    fn lint_respond_coverage_on_response_format_spec() {
        for pat in ["respond", "re*", "*"] {
            let mut s = spec("triage");
            s.response_format = Some(serde_json::json!({
                "type": "object", "additionalProperties": false,
                "required": ["summary"],
                "properties": {"summary": {"type": "string"}}
            }));
            s.permissions = Some(SubAgentPermissions {
                deny: vec![pat.into()],
                ask: vec![],
            });
            let w = warn_of(vec![s]);
            assert!(
                w.iter().any(|m| m.contains("respond") && m.contains("fallback")),
                "pattern {pat}: {w:?}"
            );
        }
        // no response_format → no lint even with deny=["*"]
        let mut s = spec("triage");
        s.permissions = Some(SubAgentPermissions { deny: vec!["*".into()], ask: vec![] });
        assert!(warn_of(vec![s]).is_empty());
    }
```

- [ ] **Step 2: Run to verify failure** —
  `cargo test -p agent-runtime-config lint_`. Expected: FAIL (no lints emitted).

- [ ] **Step 3: Implement.** Append to `warnings()` (after the existing
  `command_allowlist` loop, before `out` is returned):

```rust
        // 3B-1c advisory lints (spec §2.2 rule 8) — non-fatal by design.
        // DELIBERATELY broader than dispatch.rs's 1-element IMPLICIT_CHILD_TOOLS
        // (which gates allowlist-name VALIDITY): this set suppresses false-positive
        // inert-rule warnings for every tool that can be injected into a child
        // OUTSIDE its `tools` allowlist (`respond` is allowlist-exempt; write_todos
        // rebinds when present; dispatch_agent is depth-gated). Do not unify them.
        const IMPLICIT_CHILD_TOOLS: [&str; 4] =
            ["context_compact", "respond", "write_todos", "dispatch_agent"];
        for s in &self.named_subagents {
            let Some(p) = &s.permissions else { continue };
            let all_patterns = p.deny.iter().chain(p.ask.iter());
            for pat in all_patterns.clone() {
                // (a) inert exact rule: names a tool the spec's own allowlist excludes.
                let is_exact = !pat.contains('*');
                if is_exact {
                    if let Some(tools) = &s.tools {
                        if !tools.iter().any(|t| t == pat)
                            && !IMPLICIT_CHILD_TOOLS.contains(&pat.as_str())
                        {
                            out.push(format!(
                                "named_subagents['{}']: permissions rule '{pat}' names a tool \
                                 the spec's tools allowlist excludes — the floor is inert",
                                s.name
                            ));
                        }
                    }
                }
                // (b) affix the MCP sanitizer can never produce (`clean()` maps
                // non-[a-zA-Z0-9-] to '_'; '_' itself survives via the separator).
                let affix = pat.strip_suffix('*').unwrap_or(pat);
                if !affix.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    out.push(format!(
                        "named_subagents['{}']: permissions pattern '{pat}' contains characters \
                         no registered tool name can carry — it can never match",
                        s.name
                    ));
                }
            }
            // (c) respond coverage on a response_format spec (exact or wildcard).
            if s.response_format.is_some() {
                let covers_respond = all_patterns
                    .filter_map(|pat| agent_policy::ToolPattern::parse(pat).ok())
                    .any(|tp| tp.matches("respond"));
                if covers_respond {
                    out.push(format!(
                        "named_subagents['{}']: permissions cover the `respond` tool — this \
                         structured-response child will always hit the marked free-text fallback",
                        s.name
                    ));
                }
            }
        }
```

(`all_patterns.clone()`: `Chain` of slice iters is `Clone`; the second use consumes the
original.)

- [ ] **Step 4: Run** — `cargo test -p agent-runtime-config`. Expected: PASS, including
  the pre-existing `warnings()` interpreter-lint tests.

- [ ] **Step 5: Commit** —
  `git commit -am "feat(config): advisory permissions lints — inert rule, unsanitizable affix, respond coverage (3B-1c B2)"`

---

### Task C1: `ResolvedSubAgent.permissions` + assembly threading

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs` (`ResolvedSubAgent` struct + `resolved_with` test helper)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (spec-resolution loop, near L380)
- Test: `assemble.rs` test module

**Interfaces:**
- Consumes: `agent_policy::PermissionLists` (A2), `SubAgentSpec.permissions` (B1).
- Produces: `ResolvedSubAgent.permissions: Option<agent_policy::PermissionLists>` — raw,
  unparsed (assembly stays infallible). Empty blocks normalize to `None` at assembly.
  Task C2 consumes this exact field.

- [ ] **Step 1: Add the field.** In `dispatch.rs`, `ResolvedSubAgent` (after
  `response_format`):

```rust
    /// RAW floor lists (3B-1c §2.5) — parsed at dispatch, not at assembly; `None`
    /// ⇒ the child gets the caller's policy Arc untouched. Assembly normalizes
    /// empty blocks to `None`, so `Some` is non-empty by construction.
    pub permissions: Option<agent_policy::PermissionLists>,
```

Update the `ResolvedSubAgent` literals in `dispatch.rs` tests **per-site** (plan-review
MAJOR — do NOT blanket-`None`):
- The three FRESH fixtures — `resolved_with` (near L1170) and the two standalone
  literals (near L1415 and L1587) — get `permissions: None,`.
- The **capturing-child wrapper** literal inside `deps_with_capturing_child` (near
  L1520) clones every field from an existing `r: &ResolvedSubAgent` — there it MUST be
  `permissions: r.permissions.clone(),`. Writing `None` would make the wrapper silently
  strip floors from any future test that wraps a floored registry entry.

- [ ] **Step 2: Thread at assembly.** In `assemble.rs`'s resolution loop (the
  `resolved.insert(spec.name.clone(), agent_core::ResolvedSubAgent { ... })` literal near
  L398), add after `response_format: spec.response_format.clone(),`:

```rust
                    permissions: spec.permissions.as_ref().and_then(|p| {
                        // Empty block ≡ omitted (spec §2.2 rule 5): normalize to None
                        // so the dispatch same-Arc fast path stays honest (§2.5).
                        if p.deny.is_empty() && p.ask.is_empty() {
                            None
                        } else {
                            Some(agent_policy::PermissionLists {
                                deny: p.deny.clone(),
                                ask: p.ask.clone(),
                            })
                        }
                    }),
```

- [ ] **Step 3: Build** — `cargo build` from `agent/`. Fix any remaining
  `ResolvedSubAgent` literal missing the field (compiler lists them; also check
  `assemble.rs` tests near L1068/L1095).

- [ ] **Step 4: Write the assembly tests** (in `assemble.rs`'s test module, following the
  existing `named_subagents` test pattern near L1068 — reuse its `SubAgentSpec` fixture
  style):

```rust
    #[test]
    fn assembly_threads_raw_permissions_and_normalizes_empty() {
        use crate::runtime_config::SubAgentPermissions;
        let mut c = base_test_config(); // reuse the fixture fn the neighboring named_subagents tests use
        c.named_subagents = vec![
            {
                let mut s = test_spec("floored");
                s.permissions = Some(SubAgentPermissions {
                    deny: vec!["execute_command".into()],
                    ask: vec!["write_file".into()],
                });
                s
            },
            {
                let mut s = test_spec("emptyblock");
                s.permissions = Some(SubAgentPermissions::default());
                s
            },
            test_spec("ruleless"),
        ];
        let reg = assemble_registry_for(&c); // however the neighboring tests obtain the SubAgentRegistry
        let floored = reg.get("floored").unwrap();
        assert_eq!(
            floored.permissions,
            Some(agent_policy::PermissionLists {
                deny: vec!["execute_command".into()],
                ask: vec!["write_file".into()],
            })
        );
        assert_eq!(reg.get("emptyblock").unwrap().permissions, None);
        assert_eq!(reg.get("ruleless").unwrap().permissions, None);
    }
```

**Implementer note (plan-review corrected):** the role-names above map to the REAL
fixtures in the `named_subagents` assembly tests near L1064-1116: `base_test_config()`
= the existing `cfg()` helper; `test_spec("x")` = a **full inline `SubAgentSpec { ... }`
struct literal** (all fields — there is NO one-line spec helper in assemble.rs tests);
`assemble_registry_for(&c)` =
`assemble_loop(&c, parts(ws.path().into(), vec![])).subagent_registry.expect(...)` then
`.get(name)`. Reuse those exact fixtures; do not invent new plumbing.

- [ ] **Step 5: Run** — `cargo test -p agent-runtime-config assembly_threads` then the
  full `cargo test -p agent-runtime-config`. Expected: PASS.

- [ ] **Step 6: Commit** —
  `git commit -am "feat(core+config): ResolvedSubAgent carries raw permission lists; assembly normalizes empty (3B-1c C1)"`

---

### Task C2: Dispatch wiring — parse-at-dispatch, nested threading, integration tests

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs` (`DispatchAgentTool::execute` + a new
  `child_policy` helper; tests)

**Interfaces:**
- Consumes: `ResolvedSubAgent.permissions` (C1), `SubAgentPolicy`/`ToolPermissions` (A2).
- Produces: `fn child_policy(&self, subagent_type: &str, resolved: Option<&ResolvedSubAgent>) -> Result<Arc<dyn PolicyEngine>, ToolError>`
  on `DispatchAgentTool` (private; unit-tested directly for the `Arc::ptr_eq` pins).

- [ ] **Step 1: Implement the helper** (private method on `DispatchAgentTool`, near
  `execute`):

```rust
    /// 3B-1c §2.5/§2.6: the child's effective policy. Named child with floors →
    /// SubAgentPolicy over the CALLER'S effective policy (monotone down chains);
    /// everything else → the same Arc (byte-identical). Parse failure = the
    /// named child is undispatchable (fail-closed; the only dialect gate on the
    /// lenient-boot path, where validate() never ran).
    fn child_policy(
        &self,
        subagent_type: &str,
        resolved: Option<&ResolvedSubAgent>,
    ) -> Result<Arc<dyn PolicyEngine>, ToolError> {
        match resolved.and_then(|r| r.permissions.as_ref()) {
            Some(raw) => {
                let rules = agent_policy::ToolPermissions::parse(subagent_type, &raw.deny, &raw.ask)
                    .map_err(|e| {
                        ToolError::InvalidArgs(format!(
                            "named sub-agent '{subagent_type}': invalid permissions: {e}"
                        ))
                    })?;
                Ok(Arc::new(agent_policy::SubAgentPolicy::new(
                    self.deps.policy.clone(),
                    rules,
                )))
            }
            None => Ok(self.deps.policy.clone()),
        }
    }
```

Add `SubAgentPolicy`, `ToolPermissions` (and `PermissionLists` for C1 if not already) to
the existing `use agent_policy::{...}` line at the top of `dispatch.rs`.

- [ ] **Step 2: Wire it in `execute()`.** Immediately after the `resolved` binding (the
  `let resolved: Option<&ResolvedSubAgent> = ...` match near L461) add:

```rust
        // Child policy BEFORE nested-deps cloning (spec §2.5 ordering requirement:
        // grandchildren must gate against THIS child's effective policy, or a
        // denied child could delegate around its floor).
        let child_policy = self.child_policy(&subagent_type, resolved)?;
```

Then, inside the nested-dispatch block (locate by content:
`let mut nested = self.deps.clone();` … `nested.id_prefix = format!("sub{n}:");`), add:

```rust
            nested.policy = child_policy.clone();
```

Then in the child construction (locate `AgentLoop::new(` with
`self.deps.policy.clone(),` as its 4th argument, near L674) replace
`self.deps.policy.clone(),` with `child_policy.clone(),`.

- [ ] **Step 3: Build + existing tests** — `cargo test -p agent-core dispatch`.
  Expected: all pre-existing dispatch tests still pass (rule-less path is the same Arc).

- [ ] **Step 4: Write the integration tests.** In `dispatch.rs`'s test module. Add a
  probe tool + counting approval (RememberStub near L1713 is the shape template):

```rust
    /// Executable probe: flips a flag when it actually runs. Access::Read with no
    /// paths ⇒ base RulePolicy says Allow — so ONLY a floor can stop it.
    struct ProbeTool {
        name: &'static str,
        executed: Arc<std::sync::atomic::AtomicBool>,
    }
    #[async_trait::async_trait]
    impl Tool for ProbeTool {
        fn name(&self) -> &str { self.name }
        fn description(&self) -> &str { "probe" }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: self.name.into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent {
                tool: self.name.into(),
                access: Access::Read,
                paths: vec![],
                command: None,
                summary: "probe".into(),
            })
        }
        async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
            self.executed.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(ToolOutput { content: "ok".into(), display: None })
        }
    }

    struct CountingApproval {
        count: Arc<std::sync::atomic::AtomicUsize>,
        resp: ApprovalResponse,
    }
    #[async_trait::async_trait]
    impl agent_policy::ApprovalChannel for CountingApproval {
        async fn request(&self, _r: agent_policy::ApprovalRequest) -> ApprovalResponse {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.resp
        }
    }

    /// deps + registry entry "triage" with the given floors, probe in base_tools,
    /// child scripted to call the probe then finish.
    fn floored_deps(
        probe: Arc<ProbeTool>,
        perms: Option<agent_policy::PermissionLists>,
    ) -> DispatchDeps {
        let child = ScriptedModel::new(vec![
            Scripted::Call("c1".into(), probe.name.to_string(), "{}".into()),
            Scripted::Text("child done".into()),
        ]);
        let mut m = resolved_with(None, child, None);
        m.get_mut("triage").unwrap().permissions = perms;
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 4);
        deps.base_tools = vec![probe];
        deps.subagents = Arc::new(SubAgentRegistry::from_map(m));
        deps
    }

    #[tokio::test]
    async fn deny_floor_blocks_child_tool_with_reason() {
        let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = Arc::new(ProbeTool { name: "probe", executed: executed.clone() });
        let deps = floored_deps(
            probe,
            Some(agent_policy::PermissionLists { deny: vec!["probe".into()], ask: vec![] }),
        );
        let tool = DispatchAgentTool::new(deps);
        let out = tool
            .execute(serde_json::json!({"prompt":"go","subagent_type":"triage"}), &exec_ctx())
            .await
            .unwrap();
        assert!(!executed.load(std::sync::atomic::Ordering::SeqCst), "floored tool must not run");
        assert!(out.content.contains("child done"), "child run continues after the denial");
    }
```

**Implementer note (deny reason observation — plan-review corrected):** additionally
assert the denial REASON surfaced to the child (spec §6 "returns the reasoned error
string"). The ONLY viable mechanism in this harness: the denied tool result becomes a
message in the child's NEXT completion request, so capture child request contents with
the `SchemaCapturingModel`-style pattern already in this module (see
`description_overrides_reach_child_registry` near L1772 — it records
`req.messages[].content` into a shared `request_texts` vec). Install the capturing
model AS THE TRIAGE CHILD'S model (`resolved_with(...)` then
`m.get_mut("triage").unwrap().model = Arc::new(capturing)` — the named child runs
`resolved.model`, NOT `deps.model`), script `[Call(probe), Text("done")]`, and assert
some captured request text contains `denied by sub-agent 'triage' permissions`.
Do NOT try (a) reading the reason from `FullSink` — it captures only
`("tool_result:{status}", id, name, parent_id)` and drops content; or (b) scripting the
child to "echo" the result — `ScriptedModel` ignores incoming requests entirely.

```rust

    #[tokio::test]
    async fn mcp_shaped_prefix_floor_blocks_child_tool() {
        let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = Arc::new(ProbeTool { name: "github__create_issue", executed: executed.clone() });
        let deps = floored_deps(
            probe,
            Some(agent_policy::PermissionLists { deny: vec!["github__*".into()], ask: vec![] }),
        );
        let tool = DispatchAgentTool::new(deps);
        tool.execute(serde_json::json!({"prompt":"go","subagent_type":"triage"}), &exec_ctx())
            .await
            .unwrap();
        assert!(!executed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn ask_floor_routes_through_approval_channel() {
        for (resp, should_run) in [(ApprovalResponse::Approve, true), (ApprovalResponse::Deny, false)] {
            let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let probe = Arc::new(ProbeTool { name: "probe", executed: executed.clone() });
            let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mut deps = floored_deps(
                probe,
                Some(agent_policy::PermissionLists { deny: vec![], ask: vec!["probe".into()] }),
            );
            deps.approval = Arc::new(CountingApproval { count: count.clone(), resp });
            let tool = DispatchAgentTool::new(deps);
            tool.execute(serde_json::json!({"prompt":"go","subagent_type":"triage"}), &exec_ctx())
                .await
                .unwrap();
            assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1, "exactly one prompt");
            assert_eq!(executed.load(std::sync::atomic::Ordering::SeqCst), should_run);
        }
    }

    /// §3 invariant 1 pins: rule-less named, and general-purpose, get the SAME
    /// policy Arc — no wrapper. (Empty-block → None is pinned at assembly, C1.)
    #[tokio::test]
    async fn ruleless_and_general_purpose_share_the_policy_arc() {
        let deps = exec_deps(ScriptedModel::new(vec![]), 2);
        let base = deps.policy.clone();
        let m = resolved_with(None, ScriptedModel::new(vec![]), None); // permissions: None
        let mut deps = deps;
        deps.subagents = Arc::new(SubAgentRegistry::from_map(m));
        let tool = DispatchAgentTool::new(deps);
        let gp = tool.child_policy("general-purpose", None).unwrap();
        assert!(Arc::ptr_eq(&gp, &base));
        let named = tool
            .child_policy("triage", tool.deps.subagents.get("triage"))
            .unwrap();
        assert!(Arc::ptr_eq(&named, &base));
    }

    /// §2.6 lenient-boot fail-closed: a dialect-invalid block (validate() never
    /// ran) makes the named child undispatchable — NEVER unfloored.
    #[tokio::test]
    async fn invalid_permissions_fail_dispatch_closed() {
        let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = Arc::new(ProbeTool { name: "probe", executed: executed.clone() });
        let deps = floored_deps(
            probe,
            Some(agent_policy::PermissionLists { deny: vec!["a*b".into()], ask: vec![] }),
        );
        let tool = DispatchAgentTool::new(deps);
        let err = tool
            .execute(serde_json::json!({"prompt":"go","subagent_type":"triage"}), &exec_ctx())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid permissions"), "{err}");
        assert!(!executed.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// §3 invariant 2(b) — the delegation-escape guard: a floored child's
    /// general-purpose GRANDCHILD is still floored (nested.policy threading).
    #[tokio::test]
    async fn transitivity_floor_reaches_grandchild() {
        let executed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let probe = Arc::new(ProbeTool { name: "probe", executed: executed.clone() });
        // Grandchild (general-purpose, from nested deps' default child model):
        // tries the floored probe, then finishes.
        let grandchild = ScriptedModel::new(vec![
            Scripted::Call("g1".into(), "probe".into(), "{}".into()),
            Scripted::Text("grandchild done".into()),
        ]);
        // Named child: dispatches the grandchild, then finishes.
        let child = ScriptedModel::new(vec![
            Scripted::Call(
                "c1".into(),
                "dispatch_agent".into(),
                r#"{"prompt":"delegate"}"#.into(),
            ),
            Scripted::Text("child done".into()),
        ]);
        let mut m = resolved_with(None, child, None);
        m.get_mut("triage").unwrap().permissions =
            Some(agent_policy::PermissionLists { deny: vec!["probe".into()], ask: vec![] });
        let mut deps = exec_deps(grandchild, 4);
        deps.base_tools = vec![probe];
        deps.max_depth = 2; // allow one nested dispatch level
        deps.subagents = Arc::new(SubAgentRegistry::from_map(m));
        let tool = DispatchAgentTool::new(deps);
        let out = tool
            .execute(serde_json::json!({"prompt":"go","subagent_type":"triage"}), &exec_ctx())
            .await
            .unwrap();
        assert!(
            !executed.load(std::sync::atomic::Ordering::SeqCst),
            "grandchild must inherit the child's floor — delegation must not escape it"
        );
        assert!(out.content.contains("child done"));
    }
```

**Implementer notes:** (i) `exec_deps(grandchild, 4)` — the first argument is the deps'
default child model, which is exactly what a nested general-purpose grandchild runs;
(ii) if `tool.deps` is private to the test module's visibility, keep the registry handle
in a local before constructing the tool; (iii) `ApprovalResponse` derives `Copy` — the
loop over `(resp, should_run)` is fine; (iv) `Scripted`/`ScriptedModel`/
`PassthroughProtocol`/`AlwaysApprove` come from `crate::testkit` (already imported in
this test module).

- [ ] **Step 5: Run** — `cargo test -p agent-core dispatch`. Expected: all pass. The
  transitivity test MUST fail if you revert the `nested.policy = child_policy.clone();`
  line — verify that once by commenting it out (mutation check, per the 3B-1b sever-test
  lesson), then restore.

- [ ] **Step 6: Commit** —
  `git commit -am "feat(core): parse-at-dispatch SubAgentPolicy wrap + nested threading; floor/ask/transitivity/fail-closed tests (3B-1c C2)"`

---

### Task C3: Tool-name conformance test

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (test module)

**Interfaces:**
- Consumes: `build_registry(http_allow_hosts, max_read_bytes) -> ToolRegistry` (lib.rs
  ~L127) and `ToolRegistry::all()`.
- Produces: the spec §2.4 convention pin — identity-keyed floors require
  `intent.tool == registry name`.

- [ ] **Step 1: Write the test:**

```rust
    /// Spec 3B-1c §2.4: permission floors key on `intent.tool`; every registered
    /// tool must author it equal to its registry name or a configured floor
    /// silently misses. (MCP tools pin this in agent-mcp: `tool: self.namespaced`.)
    #[test]
    fn tool_intent_names_match_registry_names() {
        let reg = build_registry(&[], 65_536);
        let fixtures: std::collections::HashMap<&str, serde_json::Value> = [
            ("read_file", serde_json::json!({"path": "a.txt"})),
            ("write_file", serde_json::json!({"path": "a.txt", "content": ""})),
            ("edit_file", serde_json::json!({"path": "a.txt", "old_string": "a", "new_string": "b"})),
            ("list_directory", serde_json::json!({"path": "."})),
            ("grep", serde_json::json!({"pattern": "x"})),
            ("execute_command", serde_json::json!({"command": "ls"})),
            ("git_status", serde_json::json!({})),
            ("git_diff", serde_json::json!({})),
            ("git_commit", serde_json::json!({"message": "m"})),
            // MANDATORY (plan review): these two REJECT empty args — the test
            // panics without them. render's intent() requires "kind"; fetch_url's
            // requires a parseable "url". Adjust values to each intent() impl.
            ("render", serde_json::json!({"kind": "markdown", "content": "x"})),
            ("fetch_url", serde_json::json!({"url": "http://localhost/"})),
        ]
        .into_iter()
        .collect();
        // MCP-shaped stub (spec §2.4/§6 "+ a fake MCP tool"): pins the namespaced
        // convention (`tool: self.namespaced.clone()` in agent-mcp) in THIS suite.
        struct McpShapedStub;
        #[async_trait::async_trait]
        impl agent_tools::Tool for McpShapedStub {
            fn name(&self) -> &str { "github__create_issue" }
            fn description(&self) -> &str { "mcp-shaped stub" }
            fn schema(&self) -> agent_tools::ToolSchema {
                agent_tools::ToolSchema {
                    name: "github__create_issue".into(),
                    description: "".into(),
                    parameters: serde_json::json!({"type": "object"}),
                }
            }
            fn intent(&self, _a: &serde_json::Value) -> Result<agent_tools::ToolIntent, agent_tools::ToolError> {
                Ok(agent_tools::ToolIntent {
                    tool: "github__create_issue".into(),
                    access: agent_tools::Access::Write,
                    paths: vec![],
                    command: None,
                    summary: "stub".into(),
                })
            }
            async fn execute(&self, _a: serde_json::Value, _c: &agent_tools::ToolCtx)
                -> Result<agent_tools::ToolOutput, agent_tools::ToolError> {
                Ok(agent_tools::ToolOutput { content: "ok".into(), display: None })
            }
        }
        let mut reg = reg;
        reg.register(std::sync::Arc::new(McpShapedStub));
        for t in reg.all() {
            let args = fixtures
                .get(t.name())
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let intent = t
                .intent(&args)
                .unwrap_or_else(|e| panic!("intent({}) rejected fixture args: {e}", t.name()));
            assert_eq!(
                intent.tool,
                t.name(),
                "identity-keyed floors require intent.tool == registry name"
            );
        }
    }
```

**Implementer note:** if a tool's `intent()` rejects its fixture, read that tool's
`intent()` impl and adjust the fixture to the minimal args it accepts — the assertion
under test is the name equality, and the test must cover EVERY tool `build_registry`
returns plus the MCP-shaped stub (no skips).

- [ ] **Step 2: Run** — `cargo test -p agent-runtime-config tool_intent_names`. Expected:
  PASS once the `render`/`fetch_url` fixtures match their `intent()` impls (first run may
  panic with "rejected fixture args" — fix the fixture values, not the assertion; the
  convention itself was verified at baseline by the panel).

- [ ] **Step 3: Commit** —
  `git commit -am "test(config): pin intent.tool == registry name across the assembled set (3B-1c C3)"`

---

### Task D: `config.example.toml` docs + full CI

**Files:**
- Modify: `agent/config.example.toml` (the commented `[[named_subagents]]` block, near L11)

- [ ] **Step 1: Extend the example block.** After the `# # response_format = ...` line add:

```toml
# # Optional floor-only permissions (3B-1c): these can only TIGHTEN the runtime's
# # base policy for this sub-agent and everything it dispatches — never loosen it.
# # Patterns: exact tool name, trailing-* prefix ("github__*" covers one MCP
# # server's tools — names are the SANITIZED "{server}__{tool}", case-sensitive),
# # or bare "*" (every tool, INCLUDING built-ins like respond/context_compact —
# # fail-closed for this child, so use with care). There is no allow list, by design.
# # NOTE: a misspelled `permissions` key is silently ignored (the spec loads
# # unfloored); an unknown key INSIDE the block (e.g. `allow`) fails the whole
# # config file parse and the daemon falls back to flag defaults.
# # [named_subagents.permissions]
# # deny = ["execute_command", "github__*"]   # floored at Deny (with reason)
# # ask  = ["write_file", "edit_file"]        # floored at Ask (approval prompt)
```

- [ ] **Step 2: Full CI** — from the repo root:

```bash
bash scripts/ci.sh
```

Expected: all legs green (okf check, skills lint, fmt, clippy, `cargo test` in `agent/`,
conditional src-tauri, web typecheck/vitest). Fix any fmt/clippy fallout (e.g. clippy may
want `&str` over `&String` in lint loops) — behavior-preserving fixes only.

- [ ] **Step 3: Commit** —
  `git commit -am "docs(config): permissions example + footgun notes; ci green (3B-1c D)"`

---

## Plan review log

**2026-07-09 — 2 reviewers (opus): coverage/buildability APPROVE-WITH-FIXES;
adversarial architecture SOUND-WITH-NOTES. All findings FOLDED IN:** (MAJOR-1,
coverage) deny-reason observation rewritten — `SchemaCapturingModel`/`request_texts`
on the TRIAGE child's model is the only viable mechanism (`FullSink` drops result
content; `ScriptedModel` ignores requests). (MAJOR-2, architecture) C1 per-site
`ResolvedSubAgent` literal guidance — `r.permissions.clone()` in the capturing-child
wrapper (~L1520), `None` only in the three fresh fixtures. (MINORS) C3 `render`/
`fetch_url` fixtures mandatory + MCP-shaped stub added per spec §2.4/§6 + honest
Step-2 wording; B1 Step 3 both-sites `is_err→is_ok`; B2 const-divergence comment
(deliberately ≠ dispatch.rs's allowlist-validity const); C1 Step 4 real fixture names
(`cfg()`, inline `SubAgentSpec` literals, `assemble_loop(...).subagent_registry`).
Reviewers verified: full spec-coverage table (every §2.2-§7 requirement mapped to a
task), ~20 buildability claims at source (helper signatures, derives, coercions,
visibility), task ordering compiles at every boundary, and the C2 transitivity
mutation check genuinely flips.

## Post-plan process (not plan tasks — SDLC per AGENTS.md)

Whole-branch review, then merge `--no-ff` to main after Ready-to-merge — handled by the
executing session, matching 3B-1/3B-1b.
