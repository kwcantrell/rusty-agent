# Widen eval CandidateConfig beyond context knobs

**Date:** 2026-07-02
**Status:** Approved (2026-07-02 product-decision round, item 4)
**Branch:** `feat/candidateconfig-widening`

## Problem

`CandidateConfig` (`agent-runtime-config/src/eval/config.rs`) is the eval genome —
the fields Tier-A optimization edits per run. Today it holds only in-window
curation + long-term memory knobs. Two things the eval cannot currently vary:

- **System prompt** — hardcoded in the live harness's `LoopParts.base_system_prompt`
  (`tests/eval_context.rs:210-213`).
- **Protocol** — pinned to `"native"` via `RuntimeConfig::from_launch(…, "native", …)`
  (`eval_context.rs:142`).

This blocks measuring prompt/protocol variants as candidates — which is the enabler
the approved items 3 (H6b measuring experiment) and 5 (catalog-inlining, if ever
revisited) both need to be tested as prompt variants rather than guessed.

## Design

Two additive `Option` fields on `CandidateConfig`, each inherit-on-`None`, plus
two small resolver methods so the resolution logic is unit-testable without the
live (`#[ignore]`) harness.

```rust
pub struct CandidateConfig {
    // ... existing context + memory knobs unchanged ...
    /// Override the base system prompt for this candidate. None = the harness
    /// default (inherit). Lets the optimizer vary prompt wording as a genome axis.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Override the tool-call protocol ("native" | "prompted"). None = inherit
    /// the harness default. Lets the optimizer compare protocol encodings.
    #[serde(default)]
    pub protocol: Option<String>,
}

impl CandidateConfig {
    /// The system prompt this candidate runs under: its override, else `default`.
    pub fn resolved_system_prompt<'a>(&'a self, default: &'a str) -> &'a str {
        self.system_prompt.as_deref().unwrap_or(default)
    }
    /// The protocol name this candidate runs under: its override, else `default`.
    pub fn resolved_protocol<'a>(&'a self, default: &'a str) -> &'a str {
        self.protocol.as_deref().unwrap_or(default)
    }
}
```

`favorable(window)` sets both to `None` (inherit — the reference config varies
nothing but context knobs, unchanged behavior).

### Harness wiring (`tests/eval_context.rs`)

- Protocol: build the `RuntimeConfig` with the resolved protocol —
  `let protocol = cc.resolved_protocol("native"); … from_launch(…, protocol.into(), …)`
  (or assign `cfg.protocol` after construction). The eval backend is `openai`, so
  both `native` and `prompted` pass `validate()` (the claude-cli-is-prompted-only
  rule does not bind here).
- System prompt: pass `cc.resolved_system_prompt(EVAL_DEFAULT_PROMPT)` as
  `base_system_prompt`, where `EVAL_DEFAULT_PROMPT` is today's hardcoded string
  lifted to a `const` in the test module (single source).

### What is NOT in scope: tool-description variants

The verification named "prompt/protocol/tool-description variants." Prompt and
protocol have clean seams (above). **Tool-description variants do not** — tool
descriptions come from each `Tool::schema()` impl across `agent-tools`; there is
no override seam, and adding a per-candidate tool-description registry is a
tool-vocabulary build, not a config widening. It is deliberately deferred and
recorded as a follow-up: it needs a `ToolRegistry` description-override layer
first. Items 3 and 5 are prompt/protocol-shaped, so this scoping does not block
them.

## Compatibility

- Additive serde-default fields: existing `CandidateConfig` JSON (frozen champion
  configs, campaign genomes) parses unchanged — missing fields → `None` → inherit.
- The promotion gate (`eval/gate.rs`) and admissibility (`eval/admissibility.rs`)
  are UNTOUCHED — they compare on `passes()` / `median_tokens_passing()` only.
  Prompt/protocol are new genome axes the optimizer may vary; the gate judges the
  outcome exactly as before.
- No change to `eval_gate` binary, RunResult, or TaskSpec.

## Tests

Unit tests in `eval/config.rs`:
- Serde default: a JSON object omitting `system_prompt` and `protocol` parses with
  both `None`; a full round-trip preserves explicit values.
- `favorable(window)` leaves both `None`.
- `resolved_system_prompt` / `resolved_protocol`: `None` → returns the passed
  default; `Some(x)` → returns `x`.
- Back-compat: a serialized pre-widening `CandidateConfig` JSON literal (the existing
  field set only) still deserializes (pin one frozen-shape literal).

The live-harness wiring itself is exercised only under the `#[ignore]` eval run
(needs `AGENT_E2E_URL` etc.); the resolver unit tests are the CI-visible coverage.

## Out of scope (recorded)

- Tool-description variants (needs a registry override seam — deferred, above).
- Any promotion-gate change to *reward* prompt/protocol (gate stays outcome-only).
- Wiring these axes into the context-evolve campaign's own driver scripts (separate
  program; this only makes the axes available in the genome + harness).
