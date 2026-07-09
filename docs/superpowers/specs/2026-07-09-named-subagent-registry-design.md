# Named sub-agent registry (deepagents refactor, Phase 3B-1) — design

**Status:** DRAFT 2026-07-09. Design approved at brainstorm (owner decisions
recorded in §0 and §2). Next stages: adversarial spec panel → spec-review gate →
post-gate consistency read → implementation plan via `writing-plans`.
**Governing goal (owner, stated at the Phase-2 gate and reaffirmed for
Phase 3):** the refactor exists to provide deepagents-style **modularity** —
custom sub-agents/permissions/handoffs change runtime behavior via
configuration, not code. Every scope judgment below is argued against that
criterion.
**Knowledge base:** `docs/okf/deepagents-refactor/` (commit d997eec). Design
judgments in `comparisons/refactor-priorities.md` are *unvalidated input* —
including the "named sub-agent registry" bullet; this spec treats them as
claims. Primary practice source:
`practices/subagent-context-quarantine.md`.
**Live-source baseline:** commit cb6ddf0 (3A merged), re-read 2026-07-09. All
`file:line` anchors are orientation only — **locate quoted code by content
before editing.**
**Builds on:** Phase-1 middleware seam (merged 707d7fd), Phase-2 backend seam
(merged 71e23d1), Phase-3A loop middleware wave (merged cb6ddf0). This spec
designs ON the 3A child stack and guardrails and preserves all prior-phase
invariants (§3).

## 0. Decomposition — 3B is two specs, this is the first

The bundle's Phase-3 sequencing lists two sub-agent items:
**named sub-agent registry** and **product-level typed subagent stream**
(`comparisons/refactor-priorities.md` L49-50, L56-58). Per the standing
constraint that this partition is unvalidated bundle input, the decomposition
was examined at the brainstorm and resolved by the owner (2026-07-09):
**3B splits into two independently-reviewable specs**, cut along blast radius:

- **3B-1 (this spec) — Named sub-agent registry:** a persistent
  `SubAgentSpec` registry over the existing dispatch machinery, selected by
  name at dispatch. Touches `agent-runtime-config` (config + registry),
  `agent-core` `dispatch`/assembly, and `agent-policy` (per-subagent policy
  wrapper). **No wire or frontend change** — the child event forwarding
  (`parent_id`-tagged frames) is untouched.
- **3B-2 — Product-level typed subagent stream:** an additive typed
  subagent-lifecycle stream over the existing `EventSink`, surfaced through
  the wire and all three frontends. Explicitly *frontend-facing, no loop
  changes* per the bundle. Later session.

3B-1 is sequenced first: the stream (3B-2) is most useful once named
sub-agents exist to name in it, and 3B-1 establishes the `SubAgentSpec` data
model 3B-2's stream can reference. The two are otherwise independent (registry
is dispatch config; stream is wire).

**Owner scope decision (2026-07-09), carried to the spec panel:** 3B-1 adopts
the **full deepagents `SubAgent` shape** — the required trio plus *all* four
optional features (`tool_call_limit`, `permissions`, `response_format`,
`middleware`/`skills`). This is a deliberately large single spec (data model +
dispatch + policy engine + a greenfield handoff mechanism + config). The owner
mandated the full shape over a leaner first cut; per the repo SDLC this
mandate is **not silently trimmed** — it is recorded here and handed to the
adversarial panel as an explicit **scope/YAGNI stress item** (see §7).

## 1. Problem

The runtime is *closest* to deepagents on sub-agents: `DispatchAgentTool`
already spawns an isolated child loop with fresh context, snapshot-filtered
tools, depth/turn/timeout caps, a routed child model, and `sub{n}:`-tagged
event forwarding (`agent-core/src/dispatch.rs`). The gap is **shape, not
existence** (`practices/subagent-context-quarantine.md` §"Why it matters"):

1. **One ad-hoc dispatch shape.** Every dispatch is a one-off:
   `dispatch_agent(prompt, tools?, role?)` where `role` is appended to the
   parent's child system prompt (`dispatch.rs:498-501`). There is **no
   persistent named sub-agent** with its own reusable
   `system_prompt`/`tools`/`model`/permissions.
2. **No structured-response handoff.** The child hands back
   `s.final_text` + a footer (`dispatch.rs:595-621`); there is no way to
   demand a typed result. Grep confirms **zero** `response_format` /
   `structured_response` machinery in `agent/crates/`.
3. **No per-sub-agent permission scoping.** Children clone the *parent's*
   `PolicyEngine` Arc verbatim (`dispatch.rs:523`); a sub-agent cannot be
   granted a narrower (or different) permission surface than its parent.

deepagents' answer is a named `SubAgent` registry with per-agent
`name`/`description`/`system_prompt` (required) and optional
`tools`/`model`/`middleware`/`skills`/`permissions`/`response_format`, plus an
always-registered `general-purpose` sub-agent inheriting the parent
(`practices/subagent-context-quarantine.md` §"deepagents mechanics";
`sources/deepagents-docs.md` L44-49).

## 2. Design

### 2.1 `SubAgentSpec` data model

A serde-derived struct, defined in `agent-runtime-config` (the config-sourced
home) and re-exported to `agent-core` where dispatch consumes it:

```rust
pub struct SubAgentSpec {
    pub name: String,                            // required, unique registry key
    pub description: String,                     // required — the "when to use me"
                                                 //   hint surfaced in the tool schema
    pub system_prompt: String,                   // required, NEVER inherited
    pub tools: Option<Vec<String>>,              // allowlist; None ⇒ inherit parent snapshot
    pub model: Option<ModelRef>,                 // None ⇒ subagent_model / parent
    pub tool_call_limit: Option<usize>,          // 3A-E4 residual: child ToolCallLimit
    pub permissions: Option<Vec<PermissionRule>>,// replace parent policy for the child
    pub response_format: Option<serde_json::Value>, // JSON schema; None ⇒ free-text handoff
    pub middleware: Option<Vec<String>>,         // declarative named refs (§2.6)
    pub skills: Option<Vec<String>>,             // skill names loaded into child registry
}

pub struct PermissionRule {
    pub tool: String,             // exact tool name or glob (e.g. "mcp__*")
    pub decision: PermissionDecision, // Allow | Ask | Deny
}
```

`ModelRef` is the existing serde type already used by `subagent_model`.

### 2.2 `SubAgentRegistry`

Mirrors `ToolRegistry` (`agent-tools/src/registry.rs`): a
`HashMap<String, SubAgentSpec>` with **last-wins** registration (duplicate
name overwrites with a `warn!`). Built once at assembly from
`RuntimeConfig.named_subagents`, wrapped in an `Arc`, and threaded into
`DispatchDeps`. Methods: `get(name)`, `all()`, and a schema-hint accessor that
lists `(name, description)` pairs for the dispatch tool's arg documentation.

**`general-purpose` is always present.** The registry auto-registers a
`general-purpose` entry representing today's ad-hoc path (inherit parent
snapshot/model, per-call `role`/`tools` honored). A config spec named
`general-purpose` overrides it (last-wins), letting an owner re-shape the
default child.

### 2.3 Dispatch flow + general-purpose selection

`dispatch_agent` gains one optional arg: `subagent_type: String` (default
`"general-purpose"`). Behavior forks on the resolved spec:

- **`general-purpose`** → **byte-identical to today**: `prompt` + optional
  `role` (appended to parent child-prompt) + optional `tools` allowlist,
  inheriting the parent snapshot and routed model. All existing callers and
  the current test suite pass unchanged.
- **Named type** → uses the spec's `system_prompt` (replacing, not appending
  to, the parent child-prompt), `tools`, `model`, `tool_call_limit`,
  `permissions`, `response_format`, `middleware`/`skills`. Per deepagents
  **"replace, not merge"** semantics, per-call `role`/`tools` are **ignored**
  when a named type is chosen; supplying them with a non-default
  `subagent_type` is a soft no-op documented in the tool prose (not an error,
  to keep the schema forgiving). `prompt` is always the task handoff.

Unknown `subagent_type` → `ToolError::InvalidArgs` listing registered names
(same posture as the existing unknown-tool allowlist error, `dispatch.rs:418`).

The `description` of each registered spec is surfaced in the `dispatch_agent`
tool schema so the model can pick the right sub-agent. Depth/ordinal/
transitive-scope machinery (`sub{n}:`, `nested_allowed`, `filtered_base`) is
**unchanged** — named selection only changes which spec fields seed the child.

### 2.4 Tools / model resolution

- **tools**: `spec.tools` reuses the existing `filtered_base` allowlist path
  (`dispatch.rs:439-452`) — a named spec's allowlist filters the parent
  snapshot exactly as a per-call `tools` arg does today, including the
  transitive-scope guarantee for nested dispatch. `None` ⇒ full parent
  snapshot (minus `dispatch_agent`, per D4).
- **model**: `spec.model` overrides `deps.model` for this child only,
  reusing the existing routed-child-model plumbing (`deps.model.clone()` at
  `dispatch.rs:519-527` is swapped for the spec's model when set). `None` ⇒
  the configured `subagent_model` / parent model, as today.

### 2.5 E4 child `ToolCallLimit` (3A residual)

When `spec.tool_call_limit` is set, the child middleware stack becomes
`[curation, StuckDetection, Repair, ToolCallLimit(n)]` — reusing 3A's
`ToolCallLimit` guardrail (`agent-core`), which aborts via
`EndRun(StopReason::Error)` (its established sibling pattern, NOT
`BudgetExhausted`). This discharges 3A residual **E4** (the varying-args
runaway threat is shared parent/child; a named sub-agent that dispatches or
loops on varying tool args can now be bounded per-spec). When unset, the child
stack is `[curation, StuckDetection, Repair]` — **byte-identical to 3A**.

### 2.6 Per-spec `middleware` / `skills` as declarative named refs

**Design tension (owner-acknowledged):** the spec source is RuntimeConfig
JSON, but `Middleware` is a trait object, not JSON-constructible. The only
reconciliation that keeps *both* the JSON source and per-spec middleware is
**declarative named references**: `middleware: ["memory_recall", ...]`
resolved at assembly against the **known middleware set** (the constructors
`assemble.rs` already wires — `memory_recall`, `curation`, `stuck`, `repair`,
`todos`, guardrails). Unknown names → hard config error (§2.9). Resolved
middleware merge into the child stack *after* the defaults (before any
`ToolCallLimit`). `skills: ["name", ...]` resolves against discovered skills
and loads those skill tools into the child registry.

This is the sole remaining brainstorm decision point (⚠️C) not separately
gated; it is the only viable shape given the JSON source and is recorded here
for the panel. Alternative rejected: expose `middleware` only via a
programmatic injection path (drops it from the declarative config the owner
selected).

### 2.7 Per-sub-agent permissions — `SubAgentPolicy` (⚠️ decision A, resolved)

**Resolved at brainstorm: tool-name rule map, identity-keyed, replace-not-merge.**
A spec's `permissions` build a `SubAgentPolicy` that **wraps and replaces** the
parent `PolicyEngine` for the child loop (deepagents permissions "replace, not
merge"). `SubAgentPolicy::check(intent)`:

1. Match `intent.tool` against the rules (exact name, then glob). First match
   wins → its `Allow | Ask | Deny` `Decision`.
2. No rule matches → **delegate to the base `RulePolicy`** (unchanged
   workspace/command semantics).

**Critical invariant (3A residual carried in):** the rule map keys on **tool
identity**, *never* on `Access::Read`. A "read-only" sub-agent expressed as
`[{tool:"write_file",Deny},{tool:"run_command",Deny},...]` must not be built as
"auto-allow everything with `Access::Read`" — because `write_todos` and
`context_compact` carry `Access::Read` yet mutate state (established 3A: the
`context_compact` precedent flips a shared flag under `Access::Read`;
`dispatch.rs:353`, `todos.rs:123`, `context_tools.rs:37`). §6 pins this with an
explicit test: a read-only sub-agent that still exposes `write_todos` must NOT
silently auto-allow it via an access-tier shortcut. This directly honors 3A
residual **(2)**: a policy engine keying on `ToolIntent.Access` must not assume
`Read ⇒ no side-effects`.

Alternative rejected at brainstorm: a coarse access-tier preset
(read-only/read-write/full) — collides head-on with the caveat above.

### 2.8 Structured-response handoff — prompt-and-parse (⚠️ decision B, resolved)

**Resolved at brainstorm: prompt-and-parse, no loop changes.** When
`spec.response_format` (a JSON schema) is set:

1. The schema + an instruction to *end the final message with a single JSON
   object matching it* is appended to the child's system prompt.
2. After the child loop returns, `dispatch` parses `s.final_text` as JSON and
   validates it against the schema.
3. **On parse/validation failure**, a **bounded re-ask** (reuse the Repair
   posture — a small fixed retry budget, e.g. the same one-shot discipline
   3A's `RepairMiddleware` generalized) re-prompts the child for a
   schema-valid object. Exhausting the budget → the child's raw `final_text`
   is returned with a `[structured-response invalid]` note (fail soft, parent
   still gets the work product).
4. The validated JSON becomes the `ToolOutput.content` the parent sees
   (replacing the free-text handoff for this child).

This is **portable across the native and prompted tool-call protocols** and
touches **no loop internals** — it lives entirely in `DispatchAgentTool`'s
pre/post handling. Alternative rejected at brainstorm: a forced synthetic
`respond(schema)` tool (more robust validation, but the loop must treat the
call as terminal — touches loop internals, contradicting the no-loop-change
goal shared with 3B-2).

### 2.9 Config + validation

- `RuntimeConfig` gains `named_subagents: Vec<SubAgentSpec>` (default empty).
- **Assembly-time validation** (fail fast, like existing config validation):
  unique `name`s; non-empty `name`/`description`/`system_prompt`; every
  `tools`/`skills`/`middleware` ref resolves against the known
  tool/skill/middleware sets; `model` resolves; `response_format` is a
  structurally valid JSON schema; `permissions` globs compile. Any failure →
  hard config error naming the offending spec (no silent skip).
- `config.example.toml` gains a documented `[[named_subagents]]` example.

## 3. Do-not-regress / invariants

Every item below is a keep-invariant; any wave touching its subsystem must
preserve it, verified at live source.

1. **`general-purpose` path is byte-identical to 3A.** Existing
   `dispatch_agent(prompt, tools?, role?)` calls (no `subagent_type`, or
   `subagent_type:"general-purpose"`) produce identical child construction,
   system prompt, tool set, middleware stack, and handoff. This is the
   headline regression test.
2. **Child stack unchanged when no per-spec extras.**
   `[curation, StuckDetection, Repair]` remains the default; `ToolCallLimit`
   and named middleware are *additive*, only when a spec requests them.
3. **`ToolIntent` policy richness preserved.** `SubAgentPolicy` *delegates*
   to `RulePolicy` for unmatched tools — the command hard-floor, workspace
   boundary, `Access::Destroy` floor, and `TrustedWrite` semantics are
   untouched (`agent-policy/src/engine.rs`).
4. **Guardrails abort via `EndRun(StopReason::Error)`**, never
   `BudgetExhausted` (3A invariant; the child `ToolCallLimit` inherits it).
5. **No calibration / estimator touch.** No change to the token estimator,
   pinned-block order, or `build_snapshot` segments — 3B-1 is dispatch config,
   not curation (context-evolve calibration untouched).
6. **Wire / event layer untouched.** `parent_id`-tagged forwarding, the
   `sub{n}:` id lineage, `SubagentSink`, and all three frontends render
   exactly as today. The typed stream is 3B-2.
7. **Depth / transitive-scope guarantees preserved** (spec D4/G7/G8): no
   `dispatch_agent` recursion into a child; nested allowlists stay transitive;
   `sub{n}:` prefixing intact.

## 4. Backward compatibility

- `subagent_type` is optional with a `general-purpose` default → every
  existing caller (CLI, server, Tauri, eval/soak harnesses) works unchanged.
- `named_subagents` defaults empty → a config that never names a sub-agent
  behaves exactly as cb6ddf0.
- Per-call `role`/`tools` remain fully honored on the `general-purpose` path;
  they are only ignored when a *named* type is explicitly selected (§2.3).

## 5. Residual inputs from 3A — disposition

The three residuals carried out of 3A (memory: `deepagents-refactor-campaign`)
land as follows:

1. **E4 child `ToolCallLimit`** → §2.5 (per-spec field, child stack extension).
2. **`Read ⇒ side-effects` policy caveat** → §2.7 (identity-keyed permission
   rules; explicit `write_todos`-under-read-only test in §6).
3. **eval/soak `.with_todos` wiring gap** → folded in here as a **standalone
   fix**: `eval_context.rs` / `soak_live.rs` mint a todos handle but never call
   `.with_todos`; wire it so any future planning-by-recitation suite is valid.
   Small, isolated, in its own plan step.

## 6. Testing

- **Registry** (`agent-runtime-config`): last-wins overwrite (incl. overriding
  `general-purpose`); every validation failure mode (dup name, empty required
  field, unresolved tool/skill/middleware ref, bad model, invalid schema).
- **Dispatch** (`agent-core`): named lookup vs default; **replace-semantics**
  (named type ignores per-call `role`/`tools`); unknown `subagent_type` error;
  `tool_call_limit` abort via `EndRun(Error)`; per-spec model override routes
  the child; named middleware/skills merge into the child stack/registry.
- **Permissions**: rule match → Allow/Ask/Deny; unmatched → delegates to
  `RulePolicy` (boundary/command floor intact); **the identity-keying pin** — a
  read-only sub-agent exposing `write_todos` does NOT auto-allow it via any
  access-tier shortcut.
- **Structured-response**: valid JSON parses and is returned; invalid triggers
  bounded re-ask; exhausted budget falls back to raw text + note.
- **Regression**: `general-purpose` child construction byte-identical to
  cb6ddf0 (§3 invariant 1, the headline).

## 7. For the adversarial spec panel

Explicit stress items (per §0 owner scope mandate — escalated, not trimmed):

- **Scope / YAGNI (primary):** is the full deepagents shape justified in one
  spec under the modularity goal, or should `response_format` and/or
  `permissions` split into a 3B-1b? (Owner mandated full shape; panel argues
  the case, gate decides.)
- **Assumptions:** does `SubAgentPolicy` replace-not-merge actually match the
  runtime's approval flow, or does delegating-on-unmatched quietly *merge*?
- **Failure & abuse:** can a named sub-agent's `permissions` *widen* the
  parent's surface (privilege escalation) rather than only narrow it? Should
  rules be floor-only (never Allow above the parent)?
- **Simpler design:** is prompt-and-parse structured-response robust enough
  across the prompted protocol, or does it fail silently often enough to
  warrant the forced-tool alternative after all?

## 8. Waves (for the plan — one spec, waved implementation)

Owner decision (2026-07-09): **one 3B-1 spec, waved plan.** Provisional cut for
`writing-plans` (final decomposition settled at plan time):

- **Wave A — Registry + dispatch core:** `SubAgentSpec`/`SubAgentRegistry`,
  `named_subagents` config + validation, `subagent_type` arg,
  `general-purpose` default (byte-identical), tools/model resolution, E4
  child `ToolCallLimit`. + the eval/soak `.with_todos` standalone fix.
- **Wave B — Permissions:** `PermissionRule`, `SubAgentPolicy`
  wrap-and-replace, identity-keying invariant + tests.
- **Wave C — Structured-response:** prompt-and-parse handoff, schema
  validation, bounded re-ask, fail-soft fallback.
- Per-spec `middleware`/`skills` named-ref resolution rides with Wave A (it is
  child-stack assembly), gated by the §7 scope outcome.

## Panel & review log

*(dated entries added as the adversarial spec panel, gate, and post-gate
consistency read run)*
