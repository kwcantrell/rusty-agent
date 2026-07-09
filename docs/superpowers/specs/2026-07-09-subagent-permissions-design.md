# Per-sub-agent permissions — floor-only narrowing (deepagents refactor, Phase 3B-1c) — design

**Status:** BRAINSTORM-APPROVED 2026-07-09 (dialect = flat two lists; Ask-under-
unattended = rely on channel; placement = wrap-at-dispatch). Adversarial spec panel
pending; owner gate pending.
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
- Config validation (dialect + conflict rules) at `RuntimeConfig::validate()`;
  assembly stays infallible with a fail-closed can't-happen arm (§2.6).
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
  `tools` ref. Documented residual.

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
3. Unknown keys in the block (e.g. `allow`) fail file parse
   (`deny_unknown_fields`) — widening is unrepresentable.
4. Duplicate patterns are accepted (harmless; matches 3B-1b's residual posture).
5. `permissions = {}` (both lists empty) is valid: no floors. Treated as
   rule-less at dispatch (§2.5) — the child gets the caller's policy Arc
   untouched, so empty-block behavior is *identical* to omitting the block.
6. **Conflict rule:** a spec with `response_format` whose `deny` or `ask` list
   *explicitly names* `respond` (exact pattern) is a config error — installing a
   structured-response tool and dead-bolting it is always a mistake. A wildcard
   that happens to cover `respond` is accepted (§4, footgun documented).
7. The reserved-field rejection narrows: `permissions` accepted+validated;
   `middleware`/`skills` still hard-rejected non-null.

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
    /// Parses both lists; Err on any dialect violation (used by validate() AND assembly).
    pub fn parse(agent_name: &str, deny: &[String], ask: &[String]) -> Result<Self, String>;
    /// Fail-closed sentinel for the assembly can't-happen arm (§2.6).
    pub fn deny_all(agent_name: &str) -> Self;
    pub fn is_empty(&self) -> bool;
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
  every in-tree tool (each `intent()` hardcodes its registry name) and for MCP
  (`tool: self.namespaced.clone()`). §6 adds a conformance test over the
  assembled tool set so a future tool that breaks the convention fails loudly —
  a mismatch would make a configured floor silently miss.
- The floor scans `deny` before `ask` (deny short-circuits; rule 1 of §2.2 falls
  out of evaluation order).

### 2.5 Dispatch wiring & transitivity

In `DispatchAgentTool::execute()` (named-resolution region, `dispatch.rs`):

```rust
let child_policy: Arc<dyn PolicyEngine> = match resolved.and_then(|r| r.permissions.as_ref()) {
    Some(rules) => Arc::new(SubAgentPolicy::new(self.deps.policy.clone(), rules.clone())),
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
- `ResolvedSubAgent` gains `permissions: Option<agent_policy::ToolPermissions>`
  (`ToolPermissions: Clone`, two small vecs — no Arc needed). Per the 3B-1
  borrow rule, clone it into an owned local before any `.await`.
- The approval channel stays the shared parent Arc — an `ask`-floored call
  prompts exactly like any Ask today; children already emit `Approval` through
  `SubagentSink` (unchanged).
- **Byte-identical paths:** the parent loop, `general-purpose` children,
  rule-less named specs, and empty-block specs all take the `None`/same-Arc arm
  — no behavior change for any existing config.

### 2.6 Assembly & fallibility posture

`assemble_loop` stays **infallible** (3B-1 BLOCKER precedent — no `.expect()`
panics in the lenient-boot server path). Resolution:

- `RuntimeConfig::validate()` calls `ToolPermissions::parse` per spec and rejects
  the config on any dialect error (the *real* gate).
- Assembly (`assemble.rs` spec-resolution loop) calls the same
  `ToolPermissions::parse`; the `Err` arm is unreachable post-validate and is
  handled **fail-closed**: substitute `ToolPermissions::deny_all(name)` and emit
  a `tracing::error!`. A can't-happen bug thus degrades to an over-restrictive
  child, never an over-permissive one.
- `None` permissions and empty-block permissions resolve to `None` on
  `ResolvedSubAgent` (assembly normalizes `is_empty` blocks away, keeping the
  §2.5 same-Arc fast path honest).

### 2.7 Ask under unattended runs (§9 item — resolved by owner decision)

A permissions `ask` floor creates prompts that did not exist before. No new
mechanism is added: both production channels already resolve unanswered prompts
to Deny — `TerminalApproval` times out (300 s default) to Deny;
`IpcApprovalChannel` denies immediately when no UI is attached and times out to
Deny otherwise. The safety property (unanswered ⇒ never executes) holds
structurally; the cost is a bounded wait in a truly unattended terminal run.
Revisit with a knob only if a real headless deployment needs fail-fast.

## 3. Invariants (do-not-regress)

1. **Empty-config byte-identical:** a config with no `named_subagents`, or named
   specs without `permissions`, constructs children exactly as 68e846b (same
   policy Arc, no wrapper).
2. **Monotonicity (the §9 invariant):** for every intent `i` and every rule set
   `R`, `rank(SubAgentPolicy(base, R).check(i)) ≥ rank(base.check(i))` under
   `Allow < Ask < Deny`. Pinned by tests (§6) exhaustively and over a real
   `RulePolicy` corpus.
3. **No approval-channel change:** trait, impls, timeouts untouched.
4. **Identity-keyed only:** no `Access`/path/command keying in the floor
   (3A residual honored).
5. **Wire untouched:** no new events; Deny/Ask ride existing strings/events.
6. **Parent loop untouched:** no `LoopConfig` field, no gate change in
   `loop_.rs`.
7. **Widening unrepresentable:** no `allow` key exists; `deny_unknown_fields`
   pins it.
8. **Prior-phase invariants preserved:** 3B-1 registry semantics, 3B-1b respond
   machinery (subject only to the §2.2 conflict rule), 3A child-stack quarantine.

## 4. Edge cases & accepted residuals

- **Typo'd pattern = silent no-op.** A `deny` naming a tool that never appears
  does nothing; no cross-validation against the live tool set (MCP names are
  runtime-dynamic; matches 3B-1's unknown-`tools` posture). Accepted residual —
  the panel may propose an advisory `warnings()` lint (non-fatal) if it disagrees.
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
- Unknown key in the block fails parse (`deny_unknown_fields` pin).
- `respond` × `response_format` conflict (exact name rejected; `re*` wildcard
  accepted).
- Empty block valid and normalized away (assembly sets `permissions: None`).
- `middleware`/`skills` still rejected; `permissions` no longer rejected
  (the 3B-1 reserved-field test updates).

**Dispatch integration — `agent-core` (capturing harness near `dispatch.rs`
tests, extended in 3B-1 B4):**
- Deny-floored tool: child's call returns the reasoned error string; run
  continues (not aborted).
- Ask-floored tool: `Approval` event emitted (RecordingApproval double);
  approve ⇒ executes, deny ⇒ rejected.
- Rule-less named spec + `general-purpose`: the child loop receives the **same
  policy Arc** (`Arc::ptr_eq` pin) — byte-identical path.
- **Transitivity:** named child with a `deny` floor dispatches a grandchild;
  the grandchild's call to the denied tool is denied (pins `nested.policy =
  child_policy`).
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

_(pending — adversarial spec panel to be run before the owner gate; dispositions
will be recorded here in the three standard buckets.)_
