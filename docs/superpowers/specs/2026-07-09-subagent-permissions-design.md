# Per-sub-agent permissions — floor-only narrowing (deepagents refactor, Phase 3B-1c) — design

**Status:** PANEL-REVIEWED 2026-07-09. Brainstorm decisions (dialect = flat two
lists; Ask-under-unattended = rely on channel; placement = wrap-at-dispatch) held.
Adversarial panel (4 reviewers, distinct mandates): all **APPROVE-WITH-FIXES, no
BLOCKER**; the Failure reviewer confirmed **no path for a child to exceed its
parent's effective policy**. All majors folded in (see Panel & review log —
notably §2.6 reworked to parse-at-dispatch, and the `respond` conflict rule
demoted to an advisory lint). Owner gate pending: **one escalation (E1: cut or
keep the `Suffix` pattern variant)** + sign-off on the two folded reworks.
**Governing goal (owner, carried from Phase 2/3):** deepagents-style **modularity** —
a custom sub-agent changes runtime behavior via configuration, not code. This slice
adds one configured capability: a named sub-agent may declare a **permission floor**
that tightens — never loosens — the runtime's base policy for that child and its
descendants.
**Knowledge base:** `docs/okf/deepagents-refactor/` (commit d997eec).
**Live-source baseline:** commit 68e846b (3B-1b merged), re-read 2026-07-09 during
brainstorm. All `file:line` anchors are orientation only — **locate quoted code by
content before editing.**
**Builds on:** Phase-1 middleware seam (707d7fd), Phase-2 backend seam (71e23d1),
Phase-3A loop middleware wave (cb6ddf0), Phase-3B-1 named registry (4cf682d),
Phase-3B-1b structured response (68e846b). Preserves all prior-phase invariants (§3).
**Supersedes / implements:** the `permissions` bullet of the 3B-1 spec §9. That
bullet recorded the panel's escalation BLOCKER and mandated the floor-only /
monotone-narrowing design; this spec is that design. It also discharges the
`Read ⇒ side-effects` residual carried from 3A (§2.4: identity-keyed, never
Access-keyed).

## 0. Scope

3B-1 reserved `permissions` as an inert field on `SubAgentSpec` (validation
hard-rejects any non-null value). This slice **activates that one field** and
nothing else. `middleware` and `skills` stay validated-inert (dropped, 3B-1 §9).

**IN (built this cycle):**
- Un-reserve `permissions` on `SubAgentSpec` as a typed two-list block (§2.2).
- A flat tool-name pattern dialect — exact / `prefix*` / `*suffix` / bare `*`
  (§2.3). Not `globset`, no path semantics.
- `ToolPattern` + `ToolPermissions` + `SubAgentPolicy` in `agent-policy`
  (new module `subagent.rs`), where `SubAgentPolicy: PolicyEngine` computes
  `narrow(base.check(intent), rules.floor(&intent.tool))` (§2.4).
- Dispatch wiring: a named child with rules gets its loop policy wrapped over the
  **caller's effective** policy, and the **nested dispatch deps carry the wrapped
  Arc** so narrowing composes monotonically down every chain (§2.5).
- Config validation (dialect rules) at `RuntimeConfig::validate()` + advisory
  `warnings()` lints; assembly stays infallible by carrying the **raw** lists;
  the dialect parse runs once at dispatch and **fails closed** with a dispatch
  error on the lenient-boot path (§2.6).
- The §9-mandated monotonicity invariant test + unit/validation/dispatch tests (§6).
- `config.example.toml` gains a commented permissions example.

**OUT / deferred (no committed follow-on unless stated):**
- **No parent/global permissions surface.** Rules exist only on named sub-agent
  specs; the parent loop's policy is untouched. `general-purpose` and ad-hoc
  dispatch are untouched.
- **No `allow` rules.** Widening is *unrepresentable*: the block has only
  `deny`/`ask` keys and unknown keys are a config error (§2.2). This is stronger
  than "widening rejected at validation".
- **No Access-/path-/command-keyed rules.** Identity (tool name) only (§2.4).
  Access-keying is explicitly forbidden by the 3A residual (`write_todos` /
  `context_compact` / `respond` are `Access::Read` yet mutate).
- **No new approval-channel mechanism.** Ask-under-unattended resolves through the
  existing channels (owner decision, §2.7): terminal times out to Deny (300 s); IPC
  denies immediately with no UI attached, else times out to Deny. No `attended()`
  trait method, no `ask_becomes_deny` knob.
- **No wire/event change.** Deny surfaces as the existing tool-result error string;
  Ask rides the existing `Approval` event. 3B-2 typed stream is separate.
- **No cross-validation of patterns against the live tool set** (§4): a pattern
  naming a nonexistent tool is a silent no-op, same posture as 3B-1's unknown
  `tools` ref. Partially mitigated by non-fatal `warnings()` lints (§2.2);
  remainder is a documented residual.

## 1. Problem

Children share the parent's policy engine and approval channel verbatim
(`dispatch.rs` child construction, `self.deps.policy.clone()` /
`self.deps.approval.clone()`). A named sub-agent built for a narrow purpose — a
read-only triager, a summarizer with no business running commands — runs at the
full privilege of its parent. deepagents answers this with per-sub-agent tool
permission overrides.

The 3B-1 panel found the naive port is an **escalation vector** (Failure
BLOCKER-1): `Decision::Allow` executes with **no approval prompt** (`loop_.rs`
gate, ~L1348), and a "delegate-on-unmatched" rule table is a *merge* — one
config-authored `Allow` rule would grant a child privilege its parent's policy
gates behind Ask. The owner-accepted mandate (3B-1 §9): **floor-only / monotone
narrowing** — a rule may move a decision only toward more restrictive
(`Allow→Ask→Deny`), never less, and the design must make widening impossible
rather than merely discouraged.

## 2. Design

### 2.1 Approaches considered

- **(A) Wrap-at-dispatch — CHOSEN.** Typed rules parsed at config load, carried on
  `ResolvedSubAgent`; dispatch wraps the caller's effective policy in a
  `SubAgentPolicy` and threads the wrapped Arc into nested deps. Monotone by
  construction across chains; policy semantics stay in `agent-policy`; the
  invariant is a pure unit test.
- **(B) Pre-wrap at assembly** (`ResolvedSubAgent.policy: Arc<dyn PolicyEngine>`).
  REJECTED — broken for chains: assembly can only wrap over the *base* policy, so
  when narrowed child X dispatches named grandchild Y, Y's pre-built policy ignores
  X's narrowing. Non-monotone across named→named chains — exactly the hole this
  slice exists to close.
- **(C) Rules threaded into the loop gate** (`LoopConfig` field, gate applies the
  floor). REJECTED — leaks rule semantics into `agent-core`'s loop; the gate's
  contract stays cleaner consulting one `PolicyEngine`; the invariant would become
  a loop-harness test instead of a unit test.

### 2.2 Config surface

`SubAgentSpec.permissions` changes from reserved `Option<serde_json::Value>` to:

```rust
// agent-runtime-config/src/runtime_config.rs
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubAgentPermissions {
    #[serde(default)]
    pub deny: Vec<String>, // patterns; matching tool floors at Deny
    #[serde(default)]
    pub ask: Vec<String>,  // patterns; matching tool floors at Ask
}
```

```toml
[[named_subagents]]
name = "triage"
# ...
[named_subagents.permissions]
deny = ["execute_command", "github__*"]  # never runs commands / touches GitHub MCP
ask  = ["write_file", "edit_file"]       # mutations always prompt, even in-workspace
```

Semantics and validation (all violations are config errors naming the spec, in
`RuntimeConfig::validate()` alongside the existing per-spec checks):

1. **Both lists match a tool → Deny wins** (most restrictive).
2. Every pattern must parse under the §2.3 dialect (empty string, interior `*`,
   multiple `*` → error).
3. Unknown keys in the block (e.g. `allow`) are rejected by
   `deny_unknown_fields` — widening is unrepresentable: no `allow` term can
   ever reach the floor. **Precise consequence (panel A-MAJOR):** the persisted
   runtime config is **JSON** (`rt.json`; `config.example.toml` is
   documentation-only, never parsed), loaded via the lenient
   `PartialRuntimeConfig` overlay — a `deny_unknown_fields` trip aborts the
   *whole overlay*, and the daemon silently boots on the flag-derived base with
   only a stderr/log warning. That empties `named_subagents` entirely: **no
   named children exist at all** (dispatching one errors as an unknown
   `subagent_type`), so nothing runs *unfloored* — the runtime reverts to the
   pre-registry baseline. The CLI path and IPC `apply()` DO hard-reject via
   `validate()`. Accepted residual: this is the pre-existing lenient-load
   posture, not a new escalation surface (§4).
4. Duplicate patterns are accepted (harmless; matches 3B-1b's residual posture).
5. `permissions = {}` (both lists empty) is valid: no floors. Treated as
   rule-less at dispatch (§2.5) — the child gets the caller's policy Arc
   untouched, so empty-block behavior is *identical* to omitting the block.
6. **`respond` coverage is an advisory lint, not an error** (panel rework: the
   original exact-name-only hard error was asymmetric — a wildcard covering
   `respond` produced the identical dead-bolt silently). A spec with
   `response_format` whose `deny` or `ask` patterns cover `respond` — exact
   **or** wildcard — triggers a non-fatal `RuntimeConfig::warnings()` entry:
   the structured child is guaranteed to hit the marked free-text fallback.
   Uniform with §4's fail-closed posture; a broad wildcard may legitimately
   mean "lock this child down hard".
7. The reserved-field rejection narrows: `permissions` accepted+validated;
   `middleware`/`skills` still hard-rejected non-null.
8. **Advisory `warnings()` lints** (non-fatal, surfaced on CLI stderr / server
   `tracing::warn` — the existing audit-5.2 machinery): (a) an *exact* `deny`/
   `ask` pattern naming a tool that the spec's own `tools` allowlist excludes
   (the rule is inert — the gate never sees that tool); (b) a pattern whose
   affix contains characters the MCP name sanitizer would rewrite (it can
   never match a live MCP name — §2.3); (c) the `respond`-coverage lint of
   rule 6.

### 2.3 Pattern dialect (flat, not `globset`)

```rust
// agent-policy/src/subagent.rs
pub enum ToolPattern {
    Exact(String),   // "write_file"
    Prefix(String),  // "github__*"  → tools starting with "github__"
    Suffix(String),  // "*_file"     → tools ending with "_file"
    Any,             // "*"
}
impl ToolPattern {
    pub fn parse(s: &str) -> Result<Self, String>; // dialect errors are strings
    pub fn matches(&self, tool: &str) -> bool;     // plain str prefix/suffix/eq
}
```

- Exactly zero or one `*`, and only at the first or last byte. `"a*b"`, `"*a*"`,
  `""`, `"**"` → parse errors.
- Matching is case-sensitive byte comparison — tool names are already constrained
  to `[a-zA-Z0-9_-]` (MCP sanitizer, `agent-mcp/src/tool.rs`).
- **MCP coverage:** live MCP names are `{server}__{tool}` (sanitized), so
  `github__*` floors one server's whole tool set. (The 3B-1 §9 note said
  `mcp__*`; that prefix does not exist in this runtime — server-name prefix is
  the real convention.)
- **MCP sanitizer divergence (panel F-MAJOR-2/3, documented):** floors match the
  **sanitized** name `{clean(server)}__{clean(tool)}` — `clean()` rewrites every
  char outside `[a-zA-Z0-9-]` to `_`, and matching is case-sensitive. A floor
  `github__*` misses server spellings `GitHub`, `git.hub` (→ `git_hub__…`), or
  a renamed/reconnected variant. This is floor-*evasion vs author intent*, never
  escalation above the parent (the child still gets the base decision). Lint
  (b) of §2.2 rule 8 catches unsanitizable affixes; the residual (cosmetic
  server-name variants, and the pre-existing `__`-boundary ambiguity — server
  `a__b`+tool `c` and server `a`+tool `b__c` both namespace to `a__b__c` under
  last-wins registration) is documented in §4 and `config.example.toml`.
- **GATE DECISION E1 (escalated, not folded):** whether to **cut
  `ToolPattern::Suffix`** from this slice. Two reviewers found it net-negative:
  the only real suffix family is `*_file`, which spans `read_file` + two
  mutators — a security author writing `deny=["*_file"]` to block mutations
  also blocks reads. Prefix+exact+`Any` cover every documented use-case.
  Recommendation: cut (defer behind the same future gate as command/path
  floors, §5). If kept: add the `*_file`-spans-reads footgun note here and in
  `config.example.toml`.
- `Prefix("")`/`Suffix("")` cannot arise: `"*"` alone parses as `Any`, so every
  `Prefix`/`Suffix` affix is non-empty by construction (no accidental
  match-everything from a malformed affix).

### 2.4 Policy types & narrowing semantics

```rust
// agent-policy/src/subagent.rs
#[derive(Clone)]
pub struct ToolPermissions {
    agent_name: String,          // for Deny reasons
    deny: Vec<ToolPattern>,
    ask: Vec<ToolPattern>,
}
impl ToolPermissions {
    /// Parses both lists; Err on any dialect violation. Called by validate()
    /// (config-load gate) and by dispatch (the only gate on the lenient-boot
    /// path — §2.6). NOT called at assembly.
    pub fn parse(agent_name: &str, deny: &[String], ask: &[String]) -> Result<Self, String>;
    /// None = no floor. Some(Ask) / Some(Deny(reason)) = floor for this tool.
    fn floor(&self, tool: &str) -> Option<Decision>;
}

pub struct SubAgentPolicy {
    base: Arc<dyn PolicyEngine>,
    rules: ToolPermissions,
}
impl PolicyEngine for SubAgentPolicy {
    fn check(&self, intent: &ToolIntent) -> Decision {
        narrow(self.base.check(intent), self.rules.floor(&intent.tool))
    }
}
```

- **`narrow` is max over the total order `Allow < Ask < Deny`.** A base `Deny`
  is never overridden and keeps its base reason. A floor `Deny` over a base
  `Allow`/`Ask` produces
  `Deny("denied by sub-agent '{name}' permissions (rule: {pattern})")` — the §9
  "Deny carries reason" requirement, and it makes the child's tool-result error
  debuggable. A floor `Ask` over base `Allow` produces `Ask`; over base
  `Ask`/`Deny` it is a no-op.
- **Identity-keyed only:** the floor consults `intent.tool` and nothing else —
  never `Access`, paths, or command text. This honors the 3A residual: a policy
  keying on `Access::Read` would wrongly trust `write_todos`/`context_compact`/
  `respond`, which declare Read yet mutate.
- **`intent.tool` == registry name is a convention, verified at baseline** for
  every in-tree tool (panel-swept: all 21 tools' `intent()` literals match
  their `name()`) and for MCP (`tool: self.namespaced.clone()`). §6 adds a
  conformance test over the **assembled** tool set *including an MCP-shaped
  tool* so a future tool that breaks the convention fails loudly — a mismatch
  would make a configured floor silently miss. This test also stands guard on
  the §2.3 sanitizer residual: identity-keyed floors are only as good as name
  identity.
- The floor scans `deny` before `ask` (deny short-circuits; rule 1 of §2.2 falls
  out of evaluation order).

### 2.5 Dispatch wiring & transitivity

In `DispatchAgentTool::execute()` (named-resolution region, `dispatch.rs`):

```rust
// Parse-at-dispatch (§2.6): the raw lists ride ResolvedSubAgent; a dialect
// error here fails the WHOLE dispatch call, closed and loud.
let child_policy: Arc<dyn PolicyEngine> = match resolved.and_then(|r| r.permissions.as_ref()) {
    Some(raw) => {
        let rules = ToolPermissions::parse(&type_name, &raw.deny, &raw.ask)
            .map_err(|e| ToolError::InvalidArgs(format!(
                "named sub-agent '{type_name}': invalid permissions: {e}"
            )))?;
        Arc::new(SubAgentPolicy::new(self.deps.policy.clone(), rules))
    }
    None => self.deps.policy.clone(), // general-purpose & rule-less named: same Arc
};
// `Some` is non-empty by construction: assembly normalized empty blocks to None (§2.6).
```

- `AgentLoop::new` receives `child_policy` in place of `self.deps.policy.clone()`.
- **Ordering requirement (found live at baseline):** the nested
  `DispatchAgentTool` is registered from `self.deps.clone()` *early* in child
  registry construction — **before** the child loop is built. `child_policy`
  must therefore be computed **before** the nested deps are cloned, and
  `nested.policy = child_policy` set explicitly. Without this, a named child's
  grandchildren gate against the **base** policy — a delegation escape (the
  child dispatches a `general-purpose` grandchild to run what it was denied).
- Composition: chain `base → X(Rx) → Y(Ry)` yields
  `SubAgentPolicy(SubAgentPolicy(base, Rx), Ry)`; an ad-hoc grandchild under X
  gets `SubAgentPolicy(base, Rx)` unchanged. Every hop is `narrow`, so the
  effective decision for any intent is monotonically non-decreasing in
  restrictiveness down any chain — named or ad-hoc, any depth.
- `ResolvedSubAgent` gains `permissions: Option<agent_policy::PermissionLists>`
  — a **raw** two-list carrier struct (`pub deny: Vec<String>, pub ask:
  Vec<String>`, `Clone`) defined in `agent-policy` so `agent-core` never
  depends on the config crate. Mirrors 3B-1b, which carries the raw
  `response_format` Value on `ResolvedSubAgent` unparsed. Per the 3B-1 borrow
  rule, clone it into an owned local before any `.await`.
- The approval channel stays the shared parent Arc — an `ask`-floored call
  prompts exactly like any Ask today; children already emit `Approval` through
  `SubagentSink` (unchanged).
- **Byte-identical paths:** the parent loop, `general-purpose` children,
  rule-less named specs, and empty-block specs all take the `None`/same-Arc arm
  — no behavior change for any existing config.

### 2.6 Assembly & fallibility posture (panel-reworked)

`assemble_loop` stays **infallible** (3B-1 BLOCKER precedent — no `.expect()`
panics in the lenient-boot server path). **Verified boot-path fact (panel):
the server boots WITHOUT `validate()`** (`RuntimeState::new` is deliberately
lenient); only the CLI and IPC `apply()` validate. So validation is *not* a
guarantee at assembly time, and the original draft's "can't-happen fail-closed
`deny_all` arm at assembly" was both mis-attributed to the 3B-1b precedent
(which carries the raw value and never re-parses at assembly) and falsely
framed as unreachable. Reworked posture:

- `RuntimeConfig::validate()` calls `ToolPermissions::parse` per spec and
  rejects the config on any dialect error — the fail-fast gate on the CLI and
  `apply()` paths.
- **Assembly does not parse.** It clones the raw `deny`/`ask` lists into
  `ResolvedSubAgent.permissions` (as `PermissionLists`, §2.5) — trivially
  infallible, matching 3B-1b's raw-`response_format` handling exactly.
- **Dispatch parses once per named-child launch** (§2.5 snippet). On the
  lenient-boot path this is the *only* dialect gate, and it **fails closed**:
  a dialect-invalid block makes the named child **undispatchable** (the
  dispatch call errors with the parse reason) rather than running unfloored.
  Same surface-at-dispatch posture as 3B-1's unknown-`tools` precedent.
- `None` permissions and empty-block permissions resolve to `None` on
  `ResolvedSubAgent` (assembly normalizes empty blocks away, keeping the §2.5
  same-Arc fast path honest).

### 2.7 Ask under unattended runs (§9 item — resolved by owner decision)

A permissions `ask` floor creates prompts that did not exist before. No new
mechanism is added: both production channels already resolve unanswered prompts
to Deny — `TerminalApproval` times out (300 s default) to Deny;
`IpcApprovalChannel` denies immediately when no UI is attached and times out to
Deny otherwise. The safety property (unanswered ⇒ never executes) holds
structurally; the cost is a bounded wait in a truly unattended terminal run —
note that N concurrently-dispatched ask-floored children make the aggregate
unattended stall up to N×300 s of serialized Deny latency (prompts are
map-keyed with independent timeouts; nothing wedges). Revisit with a knob only
if a real headless deployment needs fail-fast.

## 3. Invariants (do-not-regress)

1. **Empty-config byte-identical:** a config with no `named_subagents`, or named
   specs without `permissions`, constructs children exactly as 68e846b (same
   policy Arc, no wrapper).
2. **Monotonicity (the §9 invariant) — two complementary, both-required
   guards** (panel R-MAJOR-2): (a) *policy-level* — for every intent `i` and
   every rule set `R`, `rank(SubAgentPolicy(base, R).check(i)) ≥
   rank(base.check(i))` under `Allow < Ask < Deny`, pinned exhaustively and
   over a real `RulePolicy` corpus (§6); (b) *wiring-level* — a grandchild
   (named or ad-hoc) dispatched by a floored child is itself subject to that
   child's floor: the delegation-escape test (§6) pins `nested.policy =
   child_policy`. (a) proves `narrow` is monotone; (b) proves the monotone
   policy actually reaches every descendant. Neither substitutes for the other.
3. **No approval-channel change:** trait, impls, timeouts untouched.
4. **Identity-keyed only:** no `Access`/path/command keying in the floor
   (3A residual honored).
5. **Wire untouched:** no new events; Deny/Ask ride existing strings/events.
6. **Parent loop untouched:** no `LoopConfig` field, no gate change in
   `loop_.rs`.
7. **Widening unrepresentable:** no `allow` key exists; `deny_unknown_fields`
   pins it. (Consequence of tripping it under the lenient JSON overlay is the
   whole-file fallback documented in §2.2 rule 3 — flag-derived base, empty
   registry, no named children at all; never an unfloored named child.)
8. **Prior-phase invariants preserved:** 3B-1 registry semantics, 3B-1b respond
   machinery (subject only to the §2.2 conflict rule), 3A child-stack quarantine.

## 4. Edge cases & accepted residuals

- **Typo'd pattern = silent no-op.** A `deny` naming a tool that never appears
  does nothing; no cross-validation against the live tool set (MCP names are
  runtime-dynamic; matches 3B-1's unknown-`tools` posture). Mitigated by the
  §2.2 rule-8 lints (inert-vs-allowlist, unsanitizable MCP affix); the
  remainder (plain misspelling of an existing tool) stays an accepted residual.
- **Misspelled `permissions` key = spec loads unfloored** (panel F-MINOR-6).
  `SubAgentSpec` deliberately has no `deny_unknown_fields` (the lenient-overlay
  forward-compat depends on outer tolerance; adding it would convert any typo
  into the §2.2 rule-3 whole-file fallback — strictly worse). So `permision =
  {…}` is silently dropped and the named child runs at parent privilege.
  Fail-open **relative to author intent only** — never above the parent.
  Accepted residual, documented in `config.example.toml`.
- **MCP identity residuals** (panel F-MAJOR-2/3): floors inherit the name
  sanitizer's divergence (cosmetic server-name variants miss a prefix floor)
  and the `__`-boundary ambiguity under last-wins registration (§2.3). Both are
  floor-evasion vs author intent, never escalation above the parent. Lint (b)
  + the §6 conformance-over-assembled-set test are the guards; the rest is
  documented residual on the pre-existing MCP registration surface.
- **`deny = ["dispatch_agent"]`** is legitimate: the nested tool stays registered
  but every call is gate-denied with the reasoned error.
- **`tools` allowlist vs `permissions` are orthogonal narrowings:** the allowlist
  removes tools from the child registry (call → unknown-tool error); permissions
  floor what remains. A tool in both is simply absent (allowlist wins by
  construction — the gate never sees it).
- **Wildcards cover injected internals.** `deny = ["*"]` floors `respond`,
  `context_compact`, `write_todos`, `dispatch_agent` too — fail-closed against
  the child's own functionality, not an escalation. Uniformity is deliberate:
  carve-outs would be a widening path. Documented footgun in
  `config.example.toml`; only the explicit `respond`-vs-`response_format`
  conflict is a validation error (§2.2 rule 6).
- **A denied call surfaces to the child model** as the existing
  `ERROR: Denied(...)` tool result; the child adapts or reports failure. The
  parent sees whatever the child hands off — no new parent-facing signal (3B-2's
  typed stream is the place for that, if ever).

## 5. What this slice deliberately does not solve

- Per-sub-agent **command** allow/deny lists (the base `RulePolicy` command
  scanner is global). A future slice could add command-pattern floors under the
  same monotone contract; nothing here precludes it.
- Path-scoped floors (per-sub-agent workspace subsets) — same story.
- Parent-level / global permission profiles.
- **deepagents parity, recorded divergence** (panel R-MINOR-3): deepagents'
  per-sub-agent permissions are path+operation glob keyed, offer
  `allow|deny|interrupt` with replace-on-declare semantics, and HITL
  approve/edit/reject/respond decisions. This surface is deliberately
  tool-name identity keyed and **narrow-only** — replace semantics *are* the
  escalation BLOCKER this slice closes (§1). An author expecting path-scoped
  rules or an `allow` override will find neither, by design.

## 6. Testing

**Unit — `agent-policy` (pure, no loop):**
- `ToolPattern::parse` accept/reject table (exact, prefix, suffix, bare `*`,
  empty, interior `*`, double `*`).
- Match semantics per variant; case sensitivity.
- `narrow` total order: all 9 (base × floor) combinations, including
  reason-preservation on base Deny and reason-content on floor Deny (agent name
  + rule pattern present).
- Deny-beats-ask when both lists match.
- **The §9 invariant test:** (a) exhaustive over the (base decision × floor)
  matrix via a stub base; (b) corpus form — a real `RulePolicy` over intents
  covering read-inside/outside-workspace, write, destroy, allowlisted /
  denylisted / unknown / operator-chained commands, against several rule sets
  (empty, deny-all, ask-all, mixed): for every intent,
  `rank(sub.check(i)) ≥ rank(base.check(i))`.
- Chain composition: `SubAgentPolicy(SubAgentPolicy(base, Rx), Ry)` is at least
  as restrictive as each single layer for every corpus intent.

**Config validation — `agent-runtime-config`:**
- Dialect accept/reject through `RuntimeConfig::validate()` (error names the
  spec and the bad pattern).
- Unknown key in the block fails the overlay parse (`deny_unknown_fields`
  pin) — test documents the lenient-load fallback consequence (§2.2 rule 3).
- `warnings()` lints fire: inert-vs-allowlist exact rule; unsanitizable MCP
  affix; `respond` coverage (exact AND wildcard) on a `response_format` spec.
  None are fatal.
- Empty block valid and normalized away (assembly sets `permissions: None`).
- `middleware`/`skills` still rejected; `permissions` no longer rejected
  (the 3B-1 reserved-field test updates).

**Dispatch integration — `agent-core` (capturing harness near `dispatch.rs`
tests, extended in 3B-1 B4):**
- Deny-floored tool: child's call returns the reasoned error string; run
  continues (not aborted).
- Ask-floored tool: `Approval` event emitted (RecordingApproval double);
  approve ⇒ executes, deny ⇒ rejected.
- Rule-less named spec, **empty-block spec** (panel R-MINOR-4), and
  `general-purpose`: the child loop receives the **same policy Arc**
  (`Arc::ptr_eq` pin, all three cases) — byte-identical path.
- **Transitivity (invariant 2b, required guard):** named child with a `deny`
  floor dispatches a grandchild; the grandchild's call to the denied tool is
  denied (pins `nested.policy = child_policy`).
- **Lenient-boot fail-closed (panel F-MAJOR-1):** a dialect-invalid
  permissions block driven through assembly + dispatch **without**
  `validate()` makes the named child undispatchable (dispatch errors with the
  parse reason) — pins the §2.6 parse-at-dispatch gate.
- Tool-name conformance: for the assembled default tool set (+ a fake MCP
  tool), `tool.name() == tool.intent(args).tool` (§2.4 convention pin).

**Docs/CI:** `config.example.toml` commented example (incl. the wildcard
footgun note); full `bash scripts/ci.sh` green before merge.

## 7. Success criterion

A named sub-agent with `deny`/`ask` floors observably: (1) never executes a
deny-floored tool at any dispatch depth beneath it; (2) prompts (or
channel-denies unattended) for every ask-floored call; (3) changes nothing for
any config that does not use the field. All three pinned by the §6 suites; no
live-model soak needed (pure policy mechanics, no model-behavior dependence).

## Panel & review log

### 2026-07-09 — Adversarial spec panel (4 reviewers) — all APPROVE-WITH-FIXES, no BLOCKER

Four skeptical reviewers with distinct mandates (requirements / assumptions /
failure-&-abuse / scope-&-simpler-design), opus-tier, each reading the spec at
1f5d579 + live source at HEAD (68e846b). **Headline:** the Failure reviewer
found **no path for a child to exceed its parent's effective policy** — the
floor-only construction is sound; every found weakness is floor-*evasion vs
author intent* (child ≤ parent always). The Assumptions reviewer verified 16/16
load-bearing source claims (one PARTIAL, see A1); all 21 in-tree tools +
MCP satisfy `intent.tool == name()`.

**A. Blockers/majors — FIXED IN PLACE:**
- **S-MAJOR (scope) × F-MAJOR-1 (failure) × A-MINOR-3, converged:** the draft's
  assembly re-parse + `deny_all` "can't-happen" arm was (i) mis-attributed to
  the 3B-1b precedent (which carries the raw value, never re-parses at
  assembly) and (ii) falsely framed unreachable — the **server boots without
  `validate()`** (verified `RuntimeState::new`, lenient by design). → §2.6
  REWORKED: assembly carries raw `PermissionLists`; **parse once at dispatch**,
  failing closed (named child undispatchable with the parse reason);
  `deny_all` deleted; new §6 lenient-boot fail-closed test.
- **A-MAJOR (assumptions):** "widening unrepresentable → fails file parse"
  hid the lenient-JSON-overlay consequence: an unknown key discards the WHOLE
  file → flag-derived base + empty registry (stderr/log warning only). Not an
  escalation (no named children exist at all, nothing runs unfloored) but the
  spec framed a fallback as a strength. → §2.2 rule 3 + invariant 7 rewritten
  with the precise consequence; JSON-vs-TOML framing fixed
  (`config.example.toml` is doc-only); accepted residual recorded.
- **R-MAJOR-1 (requirements):** a `deny`/`ask` rule naming a tool excluded by
  the spec's own `tools` allowlist is silently inert — author's
  defense-in-depth intent voided. → §2.2 rule 8 advisory `warnings()` lint (a).
- **R-MAJOR-2 (requirements):** the §9 invariant as drafted covered only the
  policy-level half; the delegation-escape (wiring) half was an incidental
  test bullet. → §3 invariant 2 split into (a) policy-level + (b) wiring-level,
  both REQUIRED.
- **F-MAJOR-2/3 (failure):** MCP sanitizer divergence (`GitHub`/`git.hub` dodge
  a `github__*` floor) + `__`-boundary ambiguity under last-wins registration
  can silently evade an intended floor (never exceed the parent). → §2.3
  documented; lint (b); conformance test extended over the assembled set incl.
  MCP; residual recorded in §4.

**B. Escalated to the owner gate:**
- **E1 — cut or keep `ToolPattern::Suffix`** (S-MINOR + F-footgun convergence):
  only real suffix family is `*_file`, which spans `read_file` + two mutators;
  prefix+exact+`Any` cover every documented use-case. Panel + synthesis
  recommendation: **CUT** (defer with command/path floors, §5). *(decision
  pending)*
- **Gate sign-off on the two folded reworks** (they alter brainstorm-approved
  sections): §2.6 parse-at-dispatch (was fail-closed-at-assembly) and §2.2
  rule 6 hard-error → advisory lint (S-MINOR "drop rule 6" × F-MINOR-4 "keep +
  warn" resolved as: uniform non-fatal lint covering exact AND wildcard).
  *(decision pending)*

**C. Minors — accepted as residual / folded:**
- R-MINOR-3 deepagents parity divergence → recorded in §5 (folded).
- R-MINOR-4 `Arc::ptr_eq` pin extended to the empty-block case → §6 (folded).
- F-MINOR-5 N×300 s aggregate unattended stall → §2.7 sentence (folded).
- F-MINOR-6 misspelled `permissions` key loads the spec unfloored; outer
  `deny_unknown_fields` rejected as strictly-worse (whole-file fallback) →
  §4 residual (accepted).
- A-MINOR-2 web UI does not edit `named_subagents` — no UI round-trip
  obligation; serde round-trip pin kept for persistence (accepted, no spec
  text carried it).
- S-cleared: empty-block normalization, same-Arc fast path, dual invariant
  tests, and the conformance test are load-bearing, not ceremony (explicitly
  probed and upheld; "gate matches on call.name" alternative REJECTED — it
  would widen the `PolicyEngine` seam that approach (C) was rejected to
  protect).

**Honest positives confirmed:** `narrow`-as-max is the minimal monotone
operator; wrap-at-dispatch is the only chain-monotone placement; §2.7 channel
posture holds structurally (map-keyed prompts, independent timeouts, nothing
wedges); the §9 escalation BLOCKER stays closed under every probed attack.
