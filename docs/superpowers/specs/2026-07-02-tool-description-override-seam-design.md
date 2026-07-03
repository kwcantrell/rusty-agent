# Tool-description override seam (eval axis) — declined for now

**Date:** 2026-07-02
**Status:** DECLINED-FOR-NOW (owner decision 2026-07-02) — design investigated and
recorded so it is a ~half-day build when a consumer appears. Do not build without
a concrete experiment that varies tool descriptions.

## Problem

The 2026-07-02 CandidateConfig widening
(`2026-07-02-candidateconfig-widening-design.md`) deliberately scoped OUT
tool-description variants: tool descriptions come from each `Tool::schema()` impl
across `agent-tools`, and there is no override seam. `CandidateConfig` can vary
`system_prompt` and `protocol` per candidate, but not the wording of the tool
vocabulary the model sees — a known high-leverage surface for tool-call
reliability that the eval/optimizer cannot currently measure.

## Investigation (2026-07-02, verified against live source)

- **The seam already has a natural home.** `ToolRegistry::schemas()`
  (`agent-tools/src/registry.rs:33`) is the single choke point where model-facing
  `ToolSchema`s are produced, and it already applies one description transform:
  the `when_not_to_call` exclusion-prose fold. Every tool — base, memory, MCP,
  `dispatch_agent` — flows through it regardless of where it was registered.
- **Per-turn consumption.** `AgentLoop` calls `self.tools.schemas()` when building
  each model request (`agent-core/src/loop_.rs:405`), so a registry-level override
  applies uniformly to parent and (via its own registry) child loops.
- **The registry is built inside `assemble_loop`** (`agent-runtime-config/src/assemble.rs`),
  which takes `(&RuntimeConfig, LoopParts)`. There is no post-assembly access to
  the registry (it is moved into an `Arc` inside `AgentLoop`), so the override map
  must arrive through one of those two parameters.

## Design sketch (what to build when a consumer appears)

1. **Registry layer** (`agent-tools/src/registry.rs`): add
   `description_overrides: HashMap<String, String>` to `ToolRegistry` plus a
   setter. In `schemas()`, when the map names a tool, the override replaces the
   *base* description; the `when_not_to_call` fold still appends afterward (so
   exclusion prose survives unless the experiment deliberately includes it in the
   override). Unknown names in the map: warn and ignore (mirrors the
   `active_skills` unknown-preset handling). ~40 LOC + unit tests.
2. **Threading — RuntimeConfig route (preferred).** Additive serde-default
   `tool_description_overrides: HashMap<String, String>` on `RuntimeConfig`
   (+ `PartialRuntimeConfig` mirror + `merge()` arm + the exhaustive round-trip
   guard test now pins this automatically). One line in `assemble_loop` to copy it
   into the registry. Old on-disk configs parse unchanged (empty default).
   - Alternative considered: carry the map on `LoopParts` to keep it eval-only.
     Rejected as the default: `LoopParts` has ~9 literal construction sites
     (agent-cli, agent-server, five test harnesses, assemble.rs) that would all
     churn, for no compatibility gain — the RuntimeConfig field is additive.
3. **Eval genome** (`eval/config.rs`): additive
   `#[serde(default)] tool_descriptions: Option<HashMap<String, String>>` on
   `CandidateConfig` + a `resolved_tool_descriptions()` accessor following the
   existing inherit-on-`None` resolver pattern; `favorable()` leaves it `None`.
   Harness (`tests/eval_context.rs`): `cfg.tool_description_overrides =
   cc.tool_descriptions.clone().unwrap_or_default();` — no `LoopParts` change.
4. **Untouched:** promotion gate, admissibility, RunResult, TaskSpec, live tool
   behavior (`intent`/`execute` never consult the override).

**Rough size:** ~150–250 LOC across `agent-tools/src/registry.rs`,
`agent-runtime-config/src/{runtime_config.rs,eval/config.rs}`,
`tests/eval_context.rs`, plus tests — about half a day including adversarial
review, following the subagent-driven flow the 2026-07 round used.

## Why declined now

- **No consumer.** H6b resolved without it (example-loading does not
  under-trigger; evidence on record in
  `docs/superpowers/experiments/h6b-example-triggering/`), catalog-inlining was
  declined-by-owner, and the context-evolve campaign's queued priority is the
  portmap gap — nothing scheduled would vary tool descriptions.
- Building now creates a dead genome axis plus an operator-visible runtime knob
  that nothing exercises.

## Revisit trigger

A scheduled experiment (context-evolve campaign or a new decision round) that
wants tool-description wording as a candidate axis. When that happens, this
sketch is the spec's starting point; re-verify the three anchor points
(`registry.rs schemas()`, `loop_.rs` request build, `assemble_loop` signature)
against live source first.
