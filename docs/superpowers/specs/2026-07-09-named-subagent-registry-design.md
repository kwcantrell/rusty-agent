# Named sub-agent registry (deepagents refactor, Phase 3B-1) — design

**Status:** PLAN-READY 2026-07-09. Brainstorm design approved; adversarial
spec panel run (4 reviewers); owner gate resolved the scope decision by
**trimming to the minimal-viable core** (see §0 and the Panel & review log);
post-gate light-tier consistency read passed CLEAN (2 trivial cross-ref fixes
applied). Next stage: implementation plan via `writing-plans`.
**Governing goal (owner, stated at the Phase-2 gate and reaffirmed for
Phase 3):** the refactor exists to provide deepagents-style **modularity** —
custom sub-agents change runtime behavior via configuration, not code. Every
scope judgment below is argued against that criterion.
**Knowledge base:** `docs/okf/deepagents-refactor/` (commit d997eec). Design
judgments in `comparisons/refactor-priorities.md` are *unvalidated input*;
primary practice source: `practices/subagent-context-quarantine.md`.
**Live-source baseline:** commit cb6ddf0 (3A merged), re-read 2026-07-09 by
the panel. All `file:line` anchors are orientation only — **locate quoted code
by content before editing.**
**Builds on:** Phase-1 middleware seam (707d7fd), Phase-2 backend seam
(71e23d1), Phase-3A loop middleware wave (cb6ddf0). Preserves all prior-phase
invariants (§3).

## 0. Decomposition & scope — minimal-viable registry, features re-sliced

3B (sub-agent maturity) was split by the owner (2026-07-09) into **3B-1**
(named registry, this spec) and **3B-2** (product-level typed subagent stream,
later). 3B-1 is sequenced first: the stream is most useful once named
sub-agents exist to name in it, and 3B-1 establishes the `SubAgentSpec` data
model 3B-2 references. The two are otherwise independent (registry is dispatch
config; stream is wire — no wire/frontend change in 3B-1).

**Scope gate (2026-07-09).** The brainstorm mandated the *full* deepagents
`SubAgent` shape (all four optional features in one spec). The adversarial
panel converged (3 of 4 reviewers, the 4th on threat-surface grounds) on
trimming, and found two of the four features **broken as specified** (not
merely over-scoped) plus two **BLOCKER**s. The owner **accepted the trim** at
the gate. 3B-1 is therefore the **minimal-viable registry**:

- **IN (built this cycle):** `SubAgentSpec` core (`name` / `description` /
  `system_prompt`) + `tools` allowlist + `model` override + `tool_call_limit`
  (3A residual E4) + the three panel-found core fixes (§2.3 M1, §2.2 M2,
  §2.4/§2.5 M3), + config + validation, + the standalone eval/soak
  `.with_todos` fix (§5).
- **RESERVED (struct fields, inert):** `permissions`, `response_format`,
  `middleware`, `skills` remain optional fields on `SubAgentSpec` so the config
  schema is stable, but validation **hard-rejects any non-null value** in 3B-1
  (§2.9). This keeps the data model whole while deferring the machinery
  (panel UNDER-2: defer the behavior, not the data model).
- **RE-SLICED follow-ons (own later specs, own gates):**
  **3B-1b** = `response_format` structured-response — greenfield and its
  BLOCKER-bearing mechanism must be *redesigned*, not amended (§9).
  **3B-1c** = `permissions` / per-sub-agent policy — its escalation BLOCKER
  requires a floor-only design decided before build (§9).
- **DROPPED (no committed follow-on):** `middleware` and `skills` named-refs.
  Both were broken as specified (§9 records the corrected mechanisms so a
  future slice does not rebuild them wrong). Revisit only when a concrete
  sub-agent needs them.

Rationale: the registry core rides almost entirely on machinery that already
exists in `dispatch.rs`; the four optional features add greenfield surface and
(for two of them) security/robustness BLOCKERs. Trimming lands the 80%
modularity win — a named sub-agent with its own prompt, tools, model, and
runaway bound — at low risk, and sequences the rest behind their own gates.

## 1. Problem

The runtime is *closest* to deepagents on sub-agents: `DispatchAgentTool`
already spawns an isolated child loop with fresh context, snapshot-filtered
tools, depth/turn/timeout caps, a routed child model, and `sub{n}:`-tagged
event forwarding (`agent-core/src/dispatch.rs`). The gap is **shape, not
existence** (`practices/subagent-context-quarantine.md`): every dispatch is a
one-off `dispatch_agent(prompt, tools?, role?)` where `role` is appended to the
parent's child system prompt (`dispatch.rs:498-501`). There is **no persistent
named sub-agent** with its own reusable `system_prompt` / `tools` / `model` /
runaway bound, and no way for the model to *choose* among purpose-built
children. deepagents' answer is a named registry with an always-registered
`general-purpose` child inheriting the parent
(`practices/subagent-context-quarantine.md` §"deepagents mechanics").

## 2. Design (minimal-viable core)

### 2.1 `SubAgentSpec` data model

A serde-derived struct in `agent-runtime-config`, re-exported to `agent-core`
where dispatch consumes it:

```rust
pub struct SubAgentSpec {
    pub name: String,               // required, unique registry key; RESERVED: "general-purpose"
    pub description: String,        // required — routing hint, surfaced as the
                                    //   subagent_type enum's per-value doc (§2.3)
    pub system_prompt: String,      // required; replaces the PARENT-DERIVED prompt
                                    //   but the SUBAGENT preamble is always composed in (§2.3)
    pub tools: Option<Vec<String>>, // allowlist over the parent snapshot; None ⇒ full snapshot
    pub model: Option<ModelRef>,    // None ⇒ subagent_model/parent; constrained to the
                                    //   config's declared model set (§2.4)
    pub tool_call_limit: Option<usize>, // 3A residual E4; bounded 1..=MAX (§2.5)

    // RESERVED — inert in 3B-1; validation hard-rejects any non-null value (§2.9).
    pub permissions: Option<serde_json::Value>,     // → 3B-1c (§9)
    pub response_format: Option<serde_json::Value>, // → 3B-1b (§9)
    pub middleware: Option<Vec<String>>,            // DROPPED (§9)
    pub skills: Option<Vec<String>>,                // DROPPED (§9)
}
```

`ModelRef` is the existing serde type used by `subagent_model`.
`RuntimeConfig` gains `named_subagents: Vec<SubAgentSpec>` (default empty).

### 2.2 `SubAgentRegistry` + `general-purpose` lockdown (fix M2)

Mirrors `ToolRegistry` (`agent-tools/src/registry.rs`): a
`HashMap<String, SubAgentSpec>` with **last-wins** registration (dup name →
`warn!`). Built once at assembly from `RuntimeConfig.named_subagents`, wrapped
in `Arc`, threaded into `DispatchDeps`. Methods: `get(name)`, `all()`, and a
`(name, description)` accessor for the dispatch-tool schema enum (§2.3).

**`general-purpose` is a reserved name (fix M2).** The registry always
provides the built-in `general-purpose` entry = today's ad-hoc path (inherit
parent snapshot, parent model, parent policy; per-call `role`/`tools` honored).
A config spec named `general-purpose` is a **hard config error** in 3B-1 — it
is *not* overridable. This closes the panel's MAJOR-4: without the lock, a
single config line silently re-shapes tools/model/policy for **every existing
ad-hoc caller** (CLI, server, eval, soak) and voids the §3 byte-identical
invariant. Re-shaping the default child is a deferred capability (revisit with
3B-1c, where policy semantics are designed), not a 3B-1 side-effect.

### 2.3 Dispatch flow, model discovery (fix M1), preamble (fix MINOR-5)

`dispatch_agent` gains one optional arg: `subagent_type`. **Fix M1 — it is a
JSON-schema `enum`** over the registered names (`["general-purpose", ...]`),
each value's `description` surfaced in the schema so the model can *route* to
the right child; default `"general-purpose"`. Without the typed enum the model
defaults to general-purpose and the registry is dead config (panel MAJOR-3).
A test asserts a named spec's description reaches the model request (reuse the
existing `SchemaCapturingModel` pattern, `dispatch.rs:1024`).

Behavior forks on the resolved spec:

- **`general-purpose`** → **byte-identical to today**: `prompt` + optional
  `role` (appended) + optional `tools` allowlist, inheriting the parent
  snapshot/model/policy. All existing callers and tests pass unchanged.
- **Named type** → uses the spec's `system_prompt`, `tools`, `model`,
  `tool_call_limit`. Per deepagents **"replace, not merge"**, per-call
  `role`/`tools` are **ignored** for a named type (documented soft no-op, not
  an error). **Fix MINOR-5:** the named `system_prompt` replaces the
  *parent-derived* portion, but the `SUBAGENT_PREAMBLE` (child-autonomy /
  "your final message is returned verbatim" framing, `dispatch.rs:16-19`) is
  **always composed in** — a named child must still know its handoff contract.

Unknown `subagent_type` → `ToolError::InvalidArgs` listing registered names
(posture of the existing unknown-tool error, `dispatch.rs:418`). Depth /
ordinal / transitive-scope machinery (`sub{n}:`, `nested_allowed`,
`filtered_base`) is **unchanged**.

**Handoff / merge-back (fix MINOR-6):** the handoff remains the single
tool-result string the parent sees (`dispatch.rs:595-621`). There is **no
child-state merge-back** — the runtime has no shared graph state to merge, and
the transcript/todos quarantine is a kept invariant (§3). This is an explicit
scope decision, not a dropped source requirement.

### 2.4 Tools / model resolution + endpoint constraint (fix M3)

- **tools**: `spec.tools` reuses the existing `filtered_base` allowlist path
  (`dispatch.rs:439-452`), including the transitive-scope guarantee for nested
  dispatch. `None` ⇒ full parent snapshot (minus `dispatch_agent`, per D4).
- **model**: `spec.model` overrides `deps.model` for this child only, reusing
  the routed-child-model plumbing (`build_routed_model`, `lib.rs:107`).
  **Fix M3:** `model` is **constrained at validation (§2.9) to the config's
  already-declared model/routing set** — a sub-agent spec may not name an
  arbitrary endpoint/URL (cost / SSRF-adjacent surface, panel MAJOR-5).

### 2.5 E4 child `ToolCallLimit` + ceiling (fix M3)

When `spec.tool_call_limit` is set, the child middleware stack becomes
`[curation, StuckDetection, Repair, ToolCallLimit(n)]` — reusing 3A's
`ToolCallLimit` (`middleware.rs:573`), which aborts via
`EndRun(StopReason::Error)` (established sibling pattern, verified
`middleware.rs:602`; NOT `BudgetExhausted`). Discharges 3A residual **E4**
(varying-args runaway threat, shared parent/child). When unset, the child stack
is `[curation, StuckDetection, Repair]` — **byte-identical to 3A**.

**Fix M3 (ceiling + zero-semantics):** validation bounds `tool_call_limit` to
`1..=MAX_SUBAGENT_TOOL_CALLS` (a defined constant); `0` and values above the
ceiling are config errors. Note the interaction with the loop's always-on
pre-turn backstop (`loop_.rs:728-731`): a per-spec cap is a second, tighter
`after_tools` gate — the plan pins which bounds first (harmless overlap).

### 2.6 `system_prompt` ceiling (fix M3)

The ad-hoc `role` path caps injected prompt size at `MAX_ROLE_CHARS = 2000`
(`dispatch.rs:22`). A named `system_prompt` is prepended to **every** child
turn, so validation (§2.9) bounds its length to a defined
`MAX_SUBAGENT_SYSTEM_PROMPT` ceiling — the named path must not drop the
size bound the ad-hoc path enforces (panel MAJOR-5).

### 2.7 Config + validation (§2.9 consolidated here)

- `RuntimeConfig.named_subagents: Vec<SubAgentSpec>` (default empty).
- **Assembly-time validation** (fail fast, naming the offending spec):
  1. `name` unique, non-empty, and **not** `general-purpose` (reserved, §2.2).
  2. `description` and `system_prompt` non-empty; `system_prompt` within the
     ceiling (§2.6).
  3. every `tools` ref resolves against the parent snapshot.
  4. `model` resolves **within the config's declared model set** (§2.4).
  5. `tool_call_limit` within `1..=MAX_SUBAGENT_TOOL_CALLS` (§2.5).
  6. **RESERVED fields hard-rejected**: any non-null `permissions`,
     `response_format`, `middleware`, or `skills` → config error
     ("not supported in 3B-1; see 3B-1b/3B-1c").
- `config.example.toml` gains a documented `[[named_subagents]]` example
  (core fields only).

## 3. Do-not-regress / invariants

1. **`general-purpose` path byte-identical to 3A**, now **unconditional**
   (fix M2 makes the name un-overridable, so the invariant can't be voided by
   config). Headline regression test.
2. **Child stack unchanged when no per-spec extras**:
   `[curation, StuckDetection, Repair]`; `ToolCallLimit` is additive only when
   `tool_call_limit` is set.
3. **`ToolIntent` policy richness untouched.** 3B-1 adds **no** policy code —
   children inherit the parent `PolicyEngine` Arc exactly as today
   (`dispatch.rs:523`). Per-sub-agent policy is deferred to 3B-1c.
4. **Guardrail aborts via `EndRun(StopReason::Error)`**, never
   `BudgetExhausted` (3A invariant; child `ToolCallLimit` inherits it).
5. **No calibration / estimator / pinned-block touch** — 3B-1 is dispatch
   config, not curation.
6. **Wire / event layer untouched** — `parent_id` forwarding, `sub{n}:`
   lineage, `SubagentSink`, all three frontends render as today. Typed stream
   is 3B-2.
7. **Depth / transitive-scope guarantees preserved** (D4/G7/G8).
8. **Sub-agent transcript/todos quarantine preserved** — no merge-back
   (§2.3); the 3A child-stack exclusion test (no memory-recall in children,
   `dispatch.rs:1137`) still holds (3B-1 adds nothing to the child stack but an
   optional `ToolCallLimit`).

## 4. Backward compatibility

- `subagent_type` optional, `general-purpose` default → every existing caller
  (CLI, server, Tauri, eval/soak) works unchanged.
- `named_subagents` defaults empty → a config that names no sub-agent behaves
  exactly as cb6ddf0.
- Per-call `role`/`tools` fully honored on `general-purpose`; ignored only when
  a *named* type is explicitly selected (§2.3).

## 5. Residual inputs from 3A — disposition

1. **E4 child `ToolCallLimit`** → §2.5 (built this cycle, with ceiling).
2. **`Read ⇒ side-effects` policy caveat** → **carried to 3B-1c** (permissions
   deferred; 3B-1 adds no Access-keyed policy, so the caveat is not triggered
   here — §3 invariant 3). Recorded so 3B-1c's identity-keyed rule design
   honors it.
3. **eval/soak `.with_todos` wiring gap** → built here as a **standalone fix**:
   `eval_context.rs` / `soak_live.rs` mint a todos handle (passed to
   `assemble_loop` via `LoopParts.todos`) but the harness-local
   `CuratedContext::new(...)` chains never call `.with_todos` — so the pinned
   todos block never renders in those harnesses (confirmed at source by the
   panel). Wire it. Small, isolated, its own plan step.

## 6. Testing

- **Registry** (`agent-runtime-config`): last-wins overwrite; reserved-name
  rejection (`general-purpose`); every validation failure (dup/empty name,
  over-ceiling system_prompt, unresolved tool, out-of-set model,
  out-of-range/zero `tool_call_limit`, **non-null reserved field**).
- **Dispatch** (`agent-core`): `subagent_type` enum surfaces registered names +
  descriptions to the model request (M1, `SchemaCapturingModel`); named lookup
  vs default; **replace-semantics** (named type ignores per-call `role`/`tools`,
  but retains `SUBAGENT_PREAMBLE`); unknown `subagent_type` error; per-spec
  model override routes the child; `tool_call_limit` aborts via
  `EndRun(Error)`.
- **Regression**: `general-purpose` child construction byte-identical to
  cb6ddf0 (§3 invariant 1, the headline); child stack unchanged when no
  `tool_call_limit`.
- **eval/soak**: `.with_todos` wired → the pinned todos block renders in the
  harness context.

## 7. Waves (for the plan — one spec, waved implementation)

Provisional cut for `writing-plans` (final decomposition at plan time):

- **Wave A — Registry + config:** `SubAgentSpec` (full struct, reserved fields
  inert) + `SubAgentRegistry`, `named_subagents` config + validation (incl.
  reserved-field rejection, ceilings, model-set constraint, reserved
  `general-purpose`). + the standalone eval/soak `.with_todos` fix.
- **Wave B — Dispatch integration:** `subagent_type` enum arg (M1),
  general-purpose default (byte-identical), named-type resolution
  (system_prompt+preamble, tools, model), E4 `ToolCallLimit` child-stack
  extension, replace-semantics.

(No permissions / structured-response / middleware waves — those are §9
follow-ons, not 3B-1.)

## 8. For the plan reviewer

Standard single-reviewer plan review (spec coverage, decomposition,
buildability). A *lighter* adversarial architecture pass is warranted only if
real design decisions leak into the plan — the security-bearing ones
(permissions, structured-response) are out of 3B-1, so 3B-1's plan is
low-risk.

## 9. Re-sliced follow-ons (recorded, not built in 3B-1)

Captured so future slices inherit the panel's corrected understanding:

- **3B-1b — structured-response (`response_format`).** Greenfield; the
  brainstorm's "prompt-and-parse, reuse the Repair posture" is **wrong** and
  must be redesigned, not amended: (a) `RepairMiddleware` is loop-resident, not
  tool-callable — a re-ask must re-invoke the child loop with a **fresh sink**
  (the existing `SubagentSink` counters/segments are cumulative, so reusing it
  double-counts); (b) `s.final_text` is a **cumulative token-stream tail**, not
  a clean last message — parse must **extract the last `{...}` span**, not
  `from_str` the whole string, and is dirtier under the prompted protocol;
  (c) **bound everything** — re-ask charged against the child's `max_turns`/
  `ToolCallLimit`, `final_text` size-capped before parse (reuse
  `max_result_bytes`), and `response_format` schema depth/regex bounded at
  validation (ReDoS); (d) state a **success criterion** (schema-valid rate on
  the default subagent model), and make fail-soft **structurally
  distinguishable** (a status flag / wrapper), not a prose note.
- **3B-1c — per-sub-agent `permissions`.** The escalation BLOCKER must be
  designed out **first**: `Decision::Allow` skips the approval channel
  (`loop_.rs:1348`), and "delegate-on-unmatched" is a *merge*, so an `Allow`
  rule can grant a child privilege **above** its parent. Required design:
  **floor-only / monotone-narrowing** — `SubAgentPolicy::check` computes
  `base.check(intent)` first and a rule may only move the decision *more*
  restrictive (`Allow→Ask→Deny`), never less; a widening rule is a config
  error. Identity-keyed (never `Access::Read`, honoring residual 5.2);
  `PermissionDecision` must carry `Deny`'s reason `String`; define a flat
  `*`-suffix/prefix glob dialect (not `globset` path semantics) with MCP
  `mcp__*` coverage; resolve `Ask`-under-unattended-child (headless channel →
  Deny). Add an invariant test: for every tool,
  `SubAgentPolicy(spec).check(i)` is never less restrictive than
  `base.check(i)`.
- **middleware / skills (dropped).** `skills` must resolve via **prompt
  preloading** (`compose_system_prompt`, `presets.rs`), *not* "skill tools into
  the registry" (that is a no-op — the skill tools are a fixed four-tool set).
  `middleware` by name cannot reconstruct stateful middlewares
  (`MemoryRecall`/`Curation`/`Todos` need runtime handles unavailable at
  dispatch) and would re-couple children to the memory-recall the 3A quarantine
  severs — build it only over a genuinely child-constructable set, behind a
  real construction seam, if ever needed.

## Panel & review log

### 2026-07-09 — Adversarial spec panel (4 reviewers) + owner gate

Four skeptical reviewers with distinct mandates (requirements / assumptions /
failure-&-abuse / scope-&-simpler-design), opus-tier, each reading the spec +
live source at cb6ddf0. **Convergence:** 3 of 4 independently recommended
trimming; two of the four optional features were found **broken as specified**,
plus two BLOCKERs. Owner gate (2026-07-09) **accepted the trim** (§0).

**A. Blockers/majors — resolved by the trim (features left 3B-1):**
- **B2 permissions escalation** (Failure BLOCKER-1 + Assumptions MAJOR-3):
  `Allow` bypasses the approval channel; delegate-on-unmatched is a merge not a
  replace → a config rule can widen a child above its parent. → deferred to
  **3B-1c** with a mandated **floor-only** design (§9).
- **B3 structured-response** (Failure BLOCKER-2 + Assumptions MAJOR-1/2 +
  Requirements MAJOR-4): "reuse Repair posture" and "parse `final_text`" are
  mechanically wrong; re-ask/blob/schema unbounded; no success criterion. →
  deferred to **3B-1b**, redesign required (§9).
- **B1 `skills` dead field** (Requirements BLOCKER-1) + **middleware
  modularity-in-name-only** (Requirements MAJOR-2 + Scope MAJOR-1): wrong
  activation mechanism / unreconstructable state; reopens the 3A quarantine. →
  **dropped**, corrected mechanisms recorded (§9).

**B. Majors — fixed in place in the core (this spec):**
- **M1** model discovery: `subagent_type` is now a schema **enum** with
  per-name descriptions (§2.3) — else the registry is dead config.
- **M2** `general-purpose` override footgun: the name is now **reserved /
  un-overridable** (§2.2), restoring the byte-identical invariant to
  unconditional (§3 invariant 1).
- **M3** resource ceilings: `system_prompt` length (§2.6), `tool_call_limit`
  range + zero-semantics (§2.5), and `model` constrained to the declared model
  set (§2.4), all enforced at validation (§2.7).
- **MINOR-5** `SUBAGENT_PREAMBLE` retained on the named path (§2.3);
  **MINOR-6** merge-back explicitly dispositioned as out-of-scope (§2.3).

**C. Minors — accepted as residual / deferred with their features:**
- Glob-dialect, `Ask`-under-unattended, `Deny`-reason field, fail-soft
  distinguishability, fan-out×budget — all belong to the deferred permissions /
  structured-response features and are recorded in §9 for those slices.
- "Waved plan ≠ descoped spec" (Scope MINOR-1): **addressed** — the trim
  descopes at the *spec* level (features left the spec), not merely the plan.

**Honest negatives confirmed by the panel (kept as-is):** registry core +
`tool_call_limit` is clean/low-risk on existing machinery; both 3A residuals
verified true at source; `general-purpose` byte-identical path holds (now
locked by M2).
