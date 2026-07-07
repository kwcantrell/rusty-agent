# Claude CLI Backend Optimization — Design

**Date:** 2026-07-07
**Status:** Approved (brainstorm complete)
**Target:** `agent/crates/agent-model/src/claude_cli.rs`, `agent/crates/agent-runtime-config/`, new OKF bundle `docs/okf/claude-cli-headless/`

## Problem

`ClaudeCliClient` drives the Claude Code CLI as a stateless bare text generator:
every model call spawns a fresh `claude -p` process and re-pipes the **entire
transcript** on stdin; tool calls ride the prompted text protocol; only whole
`assistant` text blocks and the final `result` usage are parsed. This leaves
four optimization axes unused:

1. **Cost/caching** — no session reuse; the transcript is re-sent (and
   re-prefill-processed) on every call, growing with each tool round.
2. **Streaming latency** — text arrives per whole assistant message, not as
   token deltas.
3. **Reasoning** — `thinking` output from the CLI is dropped instead of
   surfacing as `Chunk::Reasoning`.
4. **Model knobs** — `--effort`, `--fallback-model`, richer stop reasons are
   unexposed.

## Decisions taken during brainstorm

- **All four axes in scope.** Research covers everything; code targets all four.
- **Research mode: docs + local CLI probing.** Targeted WebFetch of official
  Anthropic docs plus empirical probing of the installed binary (2.1.195).
  Every claim in the bundle is verified against the binary we actually drive;
  probe captures double as test fixtures.
- **Loop ownership: Option B — the agent loop stays in agent-core.** The CLI
  remains a per-turn generator on the prompted protocol. Native tool calling
  via an MCP bridge (Option A "hybrid") and full CLI delegation (Option C) were
  considered and declined: A idles the context manager and splits loop dynamics
  per backend; C bypasses the agent-sandbox/policy boundary and breaks backend
  parity outright. Consequence accepted: tool calls stay text-parsed, and the
  caching win is partial — valid only between context-manager history rewrites.
- Flag support confirmed on claude 2.1.195: `--resume [id]`, `--session-id
  <uuid>`, `--fork-session`, `--include-partial-messages`, `--effort <level>`,
  `--fallback-model`, `--betas`.

## Section 1 — Research phase → OKF bundle

**Sources:**
- WebFetch: official docs for headless/print mode, `stream-json` output format,
  CLI reference, session management/resume.
- Local probes: live `stream-json` captures with `--include-partial-messages`
  (delta shapes), thinking blocks, the `system/init` event carrying
  `session_id`, a resume round-trip (suffix-only stdin), `--effort` and
  `--fallback-model` behavior, session persistence file locations/cleanup.

**Bundle:** `docs/okf/claude-cli-headless/`, mirroring `docs/okf/agent-sdlc/`
layout and passing `scripts/okf_check.py`. Known checker gotchas to honor:
inline-list YAML frontmatter only; citations use the literal `/sources/`
prefix.

Structure (adapted to this domain):

| dir | contents |
|---|---|
| `sources/` | one file per fetched doc / probe transcript |
| `capabilities/` | session-resume, partial-message streaming, thinking output, model knobs (effort/fallback/betas), caching economics of resume |
| `practices/` | delta-resume pattern, prefix-invalidation handling, auth preservation (the `--bare` footgun), stderr-drain, flag-pinning tests |
| `comparisons/` | stateless-full-send vs session-resume; prompted vs native tool protocol (documents *why* this repo stays prompted, citing the loop-ownership decision) |
| `index.md`, `log.md` | bundle entry point and build log |

## Section 2 — Session reuse (delta resume)

`ClaudeCliClient` becomes minimally stateful. **No `ModelClient` trait change.**

- Internal `Mutex<Option<CliSession>>` holding `session_id` plus fingerprints
  (content hashes) of messages already sent to that session.
- On `stream()`:
  - If `req.messages` is a **strict append-only extension** of the fingerprints
    → spawn with `--resume <session_id>`, pipe only the rendered **suffix**
    (new tool results / user message).
  - Otherwise (first call, or context manager rewrote history via
    curation/compaction) → fresh `--session-id <uuid>`, full transcript.
- Prefix-matching detects history rewrites **automatically** — no coupling to
  or signal from the context manager.
- `--no-session-persistence` is dropped only in reuse mode. Resume failure
  (evicted/expired session) falls back to one transparent fresh-session retry.
- `session_id` is captured from the `system/init` event; state updates only on
  a successful `Done`; any stream error resets state to `None`.
- Config gate: `claude_session_reuse: bool`, **default on**; `false` restores
  today's stateless behavior exactly.

Payoff concentrates inside tool loops: each post-tool-round model call resumes
with just the tool-result suffix instead of an ever-growing full transcript.

## Section 3 — Streaming, thinking, model knobs

- **Partial streaming:** add `--include-partial-messages`; parse `stream_event`
  lines (`content_block_delta` → `text_delta` / `thinking_delta`) into
  incremental `Chunk::Text` / `Chunk::Reasoning`. Whole-message `assistant`
  events are then skipped for text to avoid duplication — exact dedup rule
  pinned by research captures before implementation.
- **Thinking:** `thinking` blocks → `Chunk::Reasoning` in both delta and
  whole-message forms. Round-trip already works: `render_transcript` re-injects
  preserved reasoning as `<think>` blocks.
- **Model knobs:** pass `--effort <level>` and `--fallback-model <model>` when
  configured; map additional `result` subtypes to richer `StopReason`s where
  the captures show they exist.

## Section 4 — Config, tests, rollout

**Config** (`agent-runtime-config`): new optional fields —
`claude_session_reuse` (default `true`), `claude_effort` (default unset),
`claude_fallback_model` (default unset). Existing configs unchanged; claude-cli
remains prompted-only (validation untouched). Mind the known gotcha: CLI clap
defaults can shadow runtime-config defaults — new knobs get `Option` clap args.

**Tests** (existing style in `claude_cli.rs`):
- Fixture-line unit tests using **real captured** stream-json, replacing the
  hand-written literals flagged at the `NOTE` near the fixture constants.
- Fake-CLI proc tests: second call gets `--resume` + suffix-only stdin;
  history rewrite forces fresh session; resume failure falls back cleanly;
  partial-delta parsing; updated flag-pinning test covering the new flags.

**Gate:** `cargo test -p agent-model -p agent-runtime-config`, then
`bash scripts/ci.sh`.

## Error handling summary

| failure | behavior |
|---|---|
| resume rejected / session evicted | one transparent fresh-session retry |
| any stream error | session state reset to `None` (next call starts fresh) |
| unknown `stream_event` subtype | ignored (forward-compatible) |
| CLI exit non-zero | unchanged: stderr drained concurrently, surfaced as `ModelError::Process` |

## Out of scope

- Native tool calling / MCP bridge (declined Option A) — documented in the
  bundle's `comparisons/` instead.
- Full CLI delegation (declined Option C).
- Any change to the prompted protocol, `agent-core` loop, or context manager.
