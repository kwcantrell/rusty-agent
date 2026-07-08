# Audit Drain Cluster 4 — Sub-agent Composition (Design)

**Date:** 2026-07-07
**Input:** findings 4.1, 4.2, 4.3, 4.4, and 2.3≡4.5 of
`docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md`, as triaged by
`docs/superpowers/specs/2026-07-07-audit-drain-action-plan-design.md` (Cluster 4).
**Branch:** `feature/audit-subagent-composition` (worktree off `main`).
**Owner decisions (2026-07-07, batched round):** 4.3 → transient injection;
4.2 compaction half → `min()` into the maintenance limit; 4.4 → Ok-with-loud-note.

All five findings are composition seams between individually-reviewed July clusters —
none is a regression. Line numbers below are from live source at design time and
drift; the plan re-opens live source before acting.

## 1. Finding 4.1 — description overrides reach child registries

The tool-description override seam (`a0bbf0d`) applies `cfg.tool_description_overrides`
only to the parent registry (`assemble.rs:307`); the child registry built in
`DispatchAgentTool::execute` (`dispatch.rs:385`) is bare, contradicting the seam spec's
uniformity claim and splitting the tool vocabulary for description-variant eval
candidates.

**Fix.**

- `DispatchDeps` gains `pub description_overrides: std::collections::HashMap<String, String>`.
- assemble.rs fills it from `cfg.tool_description_overrides.clone()` where it constructs
  `DispatchDeps` (assemble.rs:282).
- In `DispatchAgentTool::execute`, after the child registry is populated (base tools +
  nested dispatch + context tools): `reg.set_description_overrides(self.deps.description_overrides.clone())`.
- Nested deps are `self.deps.clone()` (dispatch.rs:415), so the map reaches grandchildren
  with no further plumbing.

**Interaction with §5:** an explicit `dispatch_agent` override replaces the depth-variant
base description uniformly — explicit-wins is the registry's existing override contract,
and an operator overriding a tool's prose is asserting they know what they want.

**Pins.**

- Child registry `schemas()` shows the override for an overridden base tool
  (execute-level test through a mock child run, or a unit test on the registry the
  execute path builds).
- A nested dispatch tool's deps carry the same map (clone-propagation assert).

## 2. Finding 4.2 — ModelRef window/output limits

`ModelRef` carries no window field, so a routed `subagent_model` with a smaller context
window inherits the primary's `model_limit` (`child_config` overrides only `max_turns`,
assemble.rs:279-280) and systematically over-builds; the shrink-only calibration and
once-per-turn overflow recovery cannot compensate (second overflow in a turn is fatal
by design). Same class of gap for a routed `compaction_model`: compaction requests are
span-sized (a span can approach the parent window, `compactor.rs::render_span`), so a
smaller compaction window can overflow on the summarize call itself.

**Fix — config surface.**

- `ModelRef` gains two additive, serde-defaulted fields, inherit-on-None like its
  siblings: `pub context_limit: Option<usize>`, `pub max_tokens: Option<u32>`.
  Old JSON parses unchanged; on-wire serialization is additive (CandidateConfig
  precedent).
- `RuntimeConfig::validate()` applies the existing `context_limit >= 1024` floor to
  both `subagent_model` and `compaction_model` when their `context_limit` is `Some`.

**Fix — child-loop half.**

- In assemble.rs where `child_config.max_turns` is already overridden: when
  `cfg.subagent_model` is set, apply `context_limit → child_config.model_limit` and
  `max_tokens → child_config.max_tokens` (each only when `Some`).

**Fix — compaction half (owner call: min() into the maintenance limit).**

- `LoopConfig` gains `pub compaction_model_limit: Option<usize>` (in-process only, not
  serialized). assemble.rs sets it from `cfg.compaction_model.as_ref().and_then(|m| m.context_limit)`
  when the compaction model is routed.
- A `maint_model_limit()` helper on the loop returns
  `min(effective_model_limit(), compaction_model_limit.unwrap_or(usize::MAX))` and feeds
  `MaintCtx.model_limit` at the three construction sites (loop_.rs:526, 715, 995).
  Rationale: a span the compactor cannot read cannot be evicted, so the effective
  curation window is the min of the two declared windows.
- Placing the field on `LoopConfig` (not `DispatchDeps`) means the child's cloned config
  inherits it with zero extra plumbing, and a child with both a routed subagent model
  and a routed compaction model composes correctly
  (`min(child model_limit, compaction limit)`).

**Behavioral scope:** inert unless the new optional fields are set — no existing config
sets them, so the context-evolve champion spine is untouched and no guard sweep is
triggered. Build-request sizing (`ctx.build`) continues to use `effective_model_limit()`
unchanged; only maintenance targeting uses the min.

**Pins.**

- ModelRef inherit-on-None resolution for the two new fields (unit).
- assemble test: routed subagent model with `context_limit`/`max_tokens` set →
  `child_config.model_limit`/`max_tokens` reflect them; unset → inherit primary.
- Maintenance-limit test: `compaction_model_limit = Some(smaller)` →
  `MaintCtx.model_limit` receives the min (observable via a maintenance-triggering test
  or a unit on the helper).
- validate() rejects `context_limit < 1024` on either ModelRef.

## 3. Finding 4.3 — budget wrap-up prompt goes transient

`ctx.append(Message::user(BUDGET_WRAP_UP_PROMPT))` (loop_.rs:1015) puts "tools are
disabled for the remainder of this run" into session-persistent history, where it
survives into subsequent runs (CLI REPL and server both reuse the context) as a stale,
false capability statement — and this codebase has measured models imitating visible
history patterns (loop_.rs:704-711). When the wrap-up completion errors, the
instruction persists with no reply at all.

**Fix (owner call: transient injection).**

- The budget arm no longer appends the prompt to the context. Instead:
  `let mut messages = ctx.build(self.effective_model_limit());
  messages.push(Message::user(BUDGET_WRAP_UP_PROMPT));` — the request sees the
  instruction; durable history never does.
- The assistant summary append on success is unchanged (text-only, stray tool calls
  discarded). On the error path, history is left exactly as it was — strictly better
  than today's stranded instruction.
- History shape after a budget-exhausted run: ...tool results → assistant summary.
  An unprompted-looking summary is benign (compaction summaries already appear
  unprompted as system messages); no neutral marker is added (owner declined that
  variant).

**Pins.**

- Two-run test: run 1 exhausts the turn budget and completes the wrap-up; run 2's
  `ctx.build()` contains no tools-disabled instruction text anywhere.
- Error-path test: wrap-up completion fails → durable history contains neither the
  prompt nor a summary.
- Existing wrap-up tests that assert the appended user message are updated to assert
  the request-message injection instead (the request still ends with the wrap-up
  prompt and `tools = []`).

## 4. Finding 4.4 — partial transcript on timeout/failure

The timeout and `Ok(Err(e))` dispatch arms (dispatch.rs:468-478) return bare
`ToolError::Timeout`/`Failed` before `sink.summary()`, discarding up to
`subagent_timeout` (default 600s) of captured child work; the turn-budget path already
hands the parent a real summary (July runtime-knobs cluster), so the two budget kinds
are inconsistent.

**Fix (owner call: Ok-with-loud-note, mirroring the budget posture).**

- `Err(_elapsed)` arm: cancel the child (unchanged), then build the tool result from
  `sink.summary()`: first line
  `[sub-agent timed out after {N}s — partial transcript follows]`, then the captured
  text, blank line, then the existing turns/tool-calls footer. Empty capture → note +
  footer only (mirroring the existing empty-text composition).
- `Ok(Err(e))` arm: same shape with
  `[sub-agent failed: {e} — partial transcript follows]`.
- The parent-cancelled arm (dispatch.rs:481) stays `Err` — the parent is being torn
  down and the result is unread.
- The footer's stop field must not claim a clean `Stop` via the existing
  `unwrap_or(StopReason::Stop)` fallback when the child never emitted `Done`: on these
  paths render the recorded stop when present, else the failure kind
  (`stop: timeout` / `stop: failed`).

**Accepted semantics change (owner-adjudicated):** these paths now return `Ok`, so they
count as successful tool calls in stats rather than `tools_error`; the loud note carries
the failure signal to the parent model. Recorded here so the whole-branch review does
not re-litigate it.

**Pins.**

- Timeout: a child that streams text then hangs past a short `subagent_timeout` →
  parent's tool result contains the note line AND the captured text.
- `Ok(Err)`: a child whose model errors after emitting text → same shape with the
  failure note.
- Empty capture: note + footer only, no stray blank lines.
- Parent-cancel still returns `Err` (existing test posture preserved).

## 5. Findings 2.3 ≡ 4.5 — depth-aware dispatch description

`description()` statically says the child gets your tools "minus dispatch_agent
itself" (dispatch.rs:246-253), which is false whenever `subagent_max_depth > 1`
(the child registry registers a nested `DispatchAgentTool` by default at
dispatch.rs:414) and contradicts the `tools`-param prose in the same schema.

**Fix.**

- `DispatchAgentTool::new` computes and stores a `description: String` chosen by
  `deps.depth < deps.max_depth`:
  - **Depth floor** (`depth >= max_depth`): today's text, "minus dispatch_agent
    itself" — accurate, unchanged.
  - **Nesting allowed** (`depth < max_depth`): a variant stating the sub-agent has the
    same permissions and tools and may by default dispatch its own sub-agents while
    nesting depth allows (the `tools` allowlist can restrict this) — consistent with
    the existing `tools`-param prose at dispatch.rs:277.
- `description()` returns `&self.description`; `schema()` already flows through
  `self.description()`. Nested tools recompute in `new()` at `depth + 1`, so a
  floor-level nested tool self-corrects to the "minus dispatch_agent" text.

**Pins.**

- `max_depth = 1` (default): description contains "minus dispatch_agent itself".
- `max_depth = 2`, depth 1: description omits it and states nested dispatch; the
  nested tool built at depth 2 flips back to the floor text.

## Testing & process

- TDD per fix; dispatch pins live in `agent-core` (in-file tests +
  `tests/dispatch_tool.rs` with the existing mock-model/sink kit); ModelRef/validate
  pins in `runtime_config.rs`; child-config propagation pins in `assemble.rs` tests.
- Process per the triage spec: worktree branch off `main`, per-task subagent review,
  whole-branch review, `bash scripts/ci.sh` green before merge (`--no-ff`, delete
  branch), ledger section in `.superpowers/sdd/progress.md`, dated re-stamps in
  `.agents/skills/harness-engineering/audit.md`.
- Subagent-dispatch gotchas from memory go into every implementer brief: verify
  `git rev-parse --show-toplevel` == worktree path before every commit; no
  `git reset`/`rebase`; run `cargo fmt --all --check` in verification; worktrees
  branch from the local `main` tip, not stale `origin/main`.

## Out of scope

- Any change to override semantics beyond threading the existing map (no per-child
  override layers).
- Tokenizer conflation and child cost attribution (recorded residuals of sub-spec #3).
- Retract/replace surface on `ContextManager` (rejected alternative for 4.3).
- A separate compactor span cap (rejected alternative for 4.2's compaction half).
- Err-with-embedded-transcript (rejected alternative for 4.4).
