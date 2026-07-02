# Sub-agent dispatch — advanced dispatch (sub-spec #3)

**Date:** 2026-07-02
**Cluster:** sub-agent orchestration capability, sub-spec #3 of 3 (decomposition in
`docs/superpowers/specs/2026-07-01-subagent-dispatch-core-design.md`, merged
`af4dd14`; sub-spec #2 merged `0224383`). This closes the capability's planned
scope; the audit's remaining absent capability (Examples context type) is
separate work.
**Charter:** per-child model routing (audit Component-4 build opportunity —
"single model per session today, even compaction uses the expensive session
model; first customer is `run_compaction`"), role prompts, depth>1 with a depth
budget, fan-out ergonomics.

## YAGNI triage (what this spec deliberately is NOT)

- **No models map / named-model registry.** Two consumers are identified
  (sub-agents, compaction); two optional slots cover them.
- **No skill-defined agent types.** A free-text `role` arg gives persona
  steering; curated role registries (with per-role tools/models) wait for a
  demonstrated consumer. Recorded as future work.
- **No batch/fan-out arg.** Parallel `tool_calls` already run multiple
  dispatches concurrently (`buffer_unordered`, cap 8; pinned by sub-spec #1's
  parallel test). Fan-out ergonomics = one tool-description sentence.

## Invariant

Routing never widens privilege and never changes defaults. With all new config
absent, behavior is byte-identical to today: same single model everywhere,
depth 1, no role text. A routed child/compaction model changes **which client
serves the completion** — policy, approval, sandbox, registry subsetting,
budgets, attribution, and the no-recursion floor at max depth are untouched.

## Verified live-source facts the design rests on

- `build_model(backend, base_url, model, claude_binary, api_key) ->
  Arc<dyn ModelClient>` (`agent-runtime-config/src/lib.rs:75-90`); called once
  per frontend (`agent-cli/src/main.rs:189`, `agent-server/src/runtime.rs:208`),
  which is where the api key lives. `assemble_loop` never sees the key today.
- Compaction: `run_compaction(span, …, model: &Arc<dyn ModelClient>, cancel)`
  (`agent-core/src/compactor.rs:92-103`); the model reaches it via
  `MaintCtx.model`, constructed at exactly two sites from `&self.model`
  (`loop_.rs:352` overflow-recovery, `:594` post-tools maintain).
- Protocol is picked per loop from config (`pick_protocol(&cfg.protocol)`,
  `lib.rs:62-66`); a routed child on a different backend may need a different
  protocol — compaction does not (plain completion, no tool-call parsing).
- `DispatchDeps` (dispatch.rs) holds `model`, `protocol`, `sink`,
  `base_tools`, `child_system_prompt`, `loop_config`, `max_result_bytes`,
  `subagent_timeout`, `child_trace` — all Arc/String/scalar: `Clone` derivable.
- **Depth-2 attribution gap (the design subtlety):** `ToolCtx.call_id` is the
  RAW child-loop call id (`c1`); the visible id is the SubagentSink rewrite
  (`sub{n}:c1`), minted inside `execute()`. A grandchild stamping
  `parent_id = ctx.call_id` would point at an id no surface ever saw. The
  child-level dispatch tool must know its sink's prefix.
- `SubagentTrace::record(n, parent_id, event)` + `record_child` already carry
  per-child attribution (sub-spec #2 fast-follow) — depth-2 reuses it as-is.
- `subagent_*` config fields follow the serde-default + PartialConfig +
  `merge()` pattern (`runtime_config.rs`).

## Decisions

- **G1 — `ModelRef`: partial override, inherit-the-rest.**

  ```rust
  #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
  pub struct ModelRef {
      #[serde(default)] pub backend: Option<String>,
      #[serde(default)] pub base_url: Option<String>,
      #[serde(default)] pub model: Option<String>,
      #[serde(default)] pub claude_binary: Option<String>,
      #[serde(default)] pub protocol: Option<String>, // child loops only
  }
  ```

  Every `None` inherits the primary config's value, so `{"model": "haiku"}` on
  a claude-cli session or `{"base_url": "http://…:8081", "model": "qwen-mini"}`
  on an openai session both just work. Resolution + construction live in ONE
  helper: `build_routed_model(cfg, &ModelRef, claude_binary, api_key) ->
  Arc<dyn ModelClient>` (agent-runtime-config; `claude_binary` is a parameter
  because it lives on the frontends, not `RuntimeConfig`), delegating to
  `build_model` with merged values.
  Rejected: model-name-only override (can't reach a second local server);
  full second config block (duplicates unrelated knobs).
- **G2 — Two config slots.** `RuntimeConfig.subagent_model: Option<ModelRef>`
  and `compaction_model: Option<ModelRef>` (serde default `None`, PartialConfig
  + merge arms, tests). `None` = primary model (today).
- **G3 — routed-construction inputs reach assembly once.** `LoopParts` gains
  `api_key: Option<String>` and `claude_binary: String` (both are
  frontend-held today — the CLI flag / server state — and `RuntimeConfig`
  carries neither; doc: used only to construct routed clients; the primary
  model stays caller-built). Both frontends already hold the key for their own
  `build_model` call. `assemble_loop` builds routed clients centrally — no
  per-frontend duplication. Rejected: frontends building routed clients
  themselves (two copies of the same wiring, the drift the assemble module
  exists to prevent).
- **G4 — Compaction routing.** `AgentLoop` gains `compaction_model:
  Option<Arc<dyn ModelClient>>` + `with_compaction_model(...)` builder (the
  `with_retriever` pattern). Both `MaintCtx` sites use
  `self.compaction_model.as_ref().unwrap_or(&self.model)`. Child loops route
  compaction too: `DispatchDeps` gains `compaction_model:
  Option<Arc<dyn ModelClient>>`, applied to the child `AgentLoop` in
  `execute()`. This lands the audit's "first customer `run_compaction`".
- **G5 — Sub-agent model routing.** `assemble_loop` passes
  `subagent_model → DispatchDeps.model` (else `parts.model`) and, when the
  `ModelRef` sets `protocol`, `pick_protocol(that)` → `DispatchDeps.protocol`
  (else the session protocol). Retries/overflow-recovery inside the child ride
  the routed client automatically (it IS the child's model). Cost attribution
  needs nothing new — child `ServerUsage` already carries `parent_id`.
- **G6 — `role` arg (minimal role prompts).** `dispatch_agent(prompt, tools?,
  role?)`: optional string, max 2000 chars (`InvalidArgs` beyond), injected
  into the child's system prompt as a `Role: {role}` block appended after
  `SUBAGENT_PREAMBLE`. System-prompt placement steers harder than task-prompt
  text; no config machinery. Schema describes it; when_not_to_call untouched.
- **G7 — Depth budget, structurally safe.** `RuntimeConfig.subagent_max_depth:
  usize` (default **1** = exactly today). `DispatchDeps` gains `depth: usize`
  (top-level tool = 1) and `max_depth: usize`, and derives `Clone`. In
  `execute()`, iff `deps.depth < deps.max_depth`, register into the child
  registry a NEW `DispatchAgentTool` whose deps are `self.deps.clone()` with
  `depth: depth + 1` and the id-prefix fix below. At `depth == max_depth`
  nothing is registered — the no-recursion floor is unchanged in mechanism
  (absent tool → gate rejects), just configurable in depth. The existing
  in-tool "skip base tool named dispatch_agent" guard stays.
- **G8 — Depth-2 attribution: visible-id prefix threads down.**
  `DispatchDeps` gains `id_prefix: String` (top-level `""`). `execute()` mints
  `n = next_dispatch_n()` FIRST, then: `parent_id = format!("{}{}",
  deps.id_prefix, ctx.call_id)` (stamped on forwards and passed to the trace
  tap), and the nested dispatch tool's deps get `id_prefix:
  format!("sub{n}:")`. So a grandchild's `parent_id` is `sub{n}:c1` — exactly
  the id every surface saw for the child's dispatch row; web nesting and trace
  joins work transitively with zero renderer changes. (Flat+indent renders
  grandchildren at the same indent level as children — accepted cosmetic,
  recorded residual.)
- **G9 — Fan-out ergonomics = prose.** Append to the tool description: "You
  may dispatch several sub-agents in one message by issuing multiple
  dispatch_agent calls — they run concurrently." Nothing else.
- **G10 — Timeout at depth is nested, not additive.** A nested dispatch's
  `timeout_override` is the same `subagent_timeout`, but it runs inside the
  outer dispatch's wall clock, so the outer budget caps the whole subtree. No
  new knob; documented here and in the depth test.

## Section 1 — Config + routing helper (agent-runtime-config)

`runtime_config.rs`: `ModelRef` (G1), three fields — `subagent_model`,
`compaction_model` (G2), `subagent_max_depth` w/ `default_subagent_max_depth()
= 1` — PartialConfig + merge arms + default/merge tests.
`lib.rs`: `build_routed_model(cfg: &RuntimeConfig, r: &ModelRef, api_key:
Option<String>) -> Arc<dyn ModelClient>`.

## Section 2 — Compaction routing (agent-core + assembly)

`loop_.rs`: field + `with_compaction_model` builder + the two `MaintCtx` sites
(G4). `assemble.rs`: `LoopParts.api_key` (G3); build
`compaction_model = cfg.compaction_model.as_ref().map(|r|
build_routed_model(cfg, r, parts.api_key.clone()))`; apply to the parent loop
builder and into `DispatchDeps.compaction_model`. Frontends: pass their
existing api_key into `LoopParts`.

## Section 3 — Dispatch changes (agent-core/src/dispatch.rs)

`DispatchDeps`: `+ compaction_model`, `+ depth`, `+ max_depth`, `+ id_prefix`,
`derive(Clone)`. `execute()`: mint `n` first; parent_id/prefix per G8; nested
tool registration per G7; `role` arg per G6; child loop gets
`with_compaction_model` when set. Description sentence per G9.

## Section 4 — Assembly wiring (agent-runtime-config/src/assemble.rs)

Dispatch tool construction: `model: routed-or-parts.model` (G5), `protocol:
routed-or-session`, `depth: 1`, `max_depth: cfg.subagent_max_depth`,
`id_prefix: String::new()`, `compaction_model` (G4).

## Error handling & edge cases

- Bad `ModelRef` (unknown backend string): `build_model` already falls through
  to the openai client for unknown backends — same behavior as the primary
  path today; no new failure mode. `backend_name_is_valid` stays a
  frontend-input concern.
- Routed client unreachable: identical to a primary-model outage — the child's
  classified-retry machinery (Fatal/Retryable/Overflow) handles it; the
  dispatch call surfaces `ToolError::Failed("sub-agent failed: …")`.
- `role` over 2000 chars → `InvalidArgs`; empty/whitespace `role` → treated as
  absent.
- `subagent_max_depth: 0` → clamp to 1 at read site (0 would make the
  top-level tool self-contradictory; the flag for "no sub-agents at all" is
  `subagents: false`). Documented on the field.
- Depth chain cancellation/timeout: child token chains already compose
  (`child_token` of a `child_token`); G10 covers the wall clock.
- Old configs / partial files: all new fields serde-default to None/1 —
  byte-compatible load, no migration.

## Testing

- ModelRef: resolve-merge unit tests (each field inherits when None; protocol
  override honored); config defaults + partial-merge for the three new fields.
- `build_routed_model`: claude-cli and openai arms with merged values
  (constructible without a live server — same as existing build_model tests if
  any, else construct-and-describe only).
- Compaction routing: `AgentLoop` with `with_compaction_model(Scripted A)` and
  primary `Scripted B`; drive a compaction (`request_compaction` + maintain via
  a tool turn); assert A was consumed (`remaining()`) and B untouched by the
  summary call. Default path (no routing): existing tests unchanged.
- Assemble: routed-vs-primary Arc identity pins via `#[cfg(test)]` BuiltLoop
  fields (`subagent_model_routed: bool`, `compaction_model_routed: bool` —
  `Arc::ptr_eq` based), for all four combinations of the two slots.
- Role: capturing ModelClient wrapper records the child's CompletionRequest;
  assert the system message contains the `Role:` block; over-limit role →
  `InvalidArgs`; absent role → no block.
- Depth: default (max_depth 1) → child registry lacks dispatch (existing
  no-recursion test green, unchanged); max_depth 2 → child dispatches a
  grandchild; grandchild's forwarded events carry `parent_id == "sub{n}:c1"`
  where `sub{n}:c1` is the child dispatch row's visible id (FullSink quad
  pin); at depth 2 the grandchild registry lacks dispatch (three-level attempt
  rejected). Grandchild suppressed events reach the tap with the prefixed
  parent id.
- Fan-out prose: schema description contains the concurrency sentence.
- `bash scripts/ci.sh` green; zero wire/web changes expected (assert nothing
  under `web/` changes — attribution machinery from #2 covers depth-2 as-is).

## Files touched

- `agent/crates/agent-runtime-config/src/runtime_config.rs`, `lib.rs`,
  `assemble.rs`, tests.
- `agent/crates/agent-core/src/loop_.rs` (compaction_model), `dispatch.rs`,
  tests (`dispatch_tool.rs`).
- `agent/crates/agent-cli/src/main.rs`, `agent/crates/agent-server/src/runtime.rs`
  (LoopParts.api_key pass-through).
- No `agent-server/src/wire.rs`, no `web/` changes.

## The 20% (human-judgment points)

The id-prefix threading (G8) is the correctness crux — get it wrong and depth-2
attribution silently points nowhere; the ModelRef-inherits-primary semantics
(G1) and the depth-0 clamp (edge cases) are the other judgment points.

## Out of scope (recorded residuals)

- Skill-defined agent types / curated role registry (revisit on demand).
- Models map beyond the two slots; per-dispatch-call model override arg.
- Grandchild indent depth on the web (flat+indent renders one level).
- Live child token streaming (E9, unchanged); Examples context type (separate
  capability, the audit's last absent one).
