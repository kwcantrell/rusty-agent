# Claude CLI as Inference Backend — Design

**Date:** 2026-06-23
**Status:** Approved (pending spec review)

## Motivation

Run the Rust agent runtime against Claude-quality reasoning without standing up
SGLang+GPU or managing an Anthropic API key, by piggybacking on an existing
Claude Code subscription. Subscription auth is reachable **only** through the
Claude Code CLI (or Agent SDK) — the raw Anthropic Messages API is API-key
billed and does not use the subscription — so the CLI-subprocess path is the
one route that satisfies both goals (cost/auth convenience *and* capability).

## Core decisions

1. **Claude is a pure text generator; the Rust loop stays fully in charge.**
   The CLI runs with its own tools disabled. Tool calls are produced and parsed
   via the existing `Prompted` protocol (schemas injected into the system
   message, ` ```tool_call ` fenced blocks parsed from text). The Rust loop
   executes tools through its own policy engine.
2. **Integration via a new `ModelClient` impl**, not an HTTP shim. No extra
   process or network hop; lives in `agent-model`.
3. **Stateless, per-turn subprocess.** The Rust loop already owns context
   (`WindowContext`, `context_limit`); CLI session continuity is deliberately
   unused to avoid double-managing history.

## Architecture & seam

A new struct in `agent-model`, `ClaudeCliClient`, implements the existing
`ModelClient` trait (`async fn stream(req) -> BoxStream<Chunk>`). Nothing above
the trait changes — `AgentLoop`, the `Prompted` protocol, the tool registry,
and the policy engine are untouched.

Backend selection happens at construction in `agent-cli` / `agent-server` via a
new `--backend {openai|claude-cli}` flag:

- Default `openai`, preserving all current behavior.
- When `claude-cli` is chosen: `--base-url` is ignored, and `--protocol` is
  forced to `prompted` (native OpenAI-style `tool_calls` are not available from
  a disabled-tools CLI).

## Per-turn subprocess invocation

Every `stream()` call spawns a fresh `claude` process. Invocation shape:

```
claude -p --output-format stream-json --verbose --allowedTools "" --model <model>
```

- Prompt delivered on **stdin** (avoids argument-length limits).
- `--model` maps from the existing `--model` flag (e.g. `sonnet`, `opus`).
- Auth is implicit: relies on the machine's already-authenticated Claude Code
  subscription. No API key handling.

> Exact flag spelling is verified during the Phase 0 spike; some flags vary by
> CLI version. The design depends only on three behaviors being achievable:
> disable the CLI's own tools, stream output as parseable JSON, and feed the
> prompt on stdin.

## Transcript → prompt rendering

After `Prompted::prepare()` runs, `req.messages` is
`[System(tool-preamble + base system), User, Assistant, Tool, …]`.
`ClaudeCliClient` linearizes this into a single role-delimited text prompt, e.g.:

```
## System
<tool preamble + base system prompt>

## User
<task>

## Assistant
<prior assistant text>

## Tool (read_file)
<tool result>
```

…piped to stdin.

**Decision: flatten-to-text rather than `--input-format stream-json`.** The
Prompted protocol has already collapsed tool calls/results into plain messages,
so there is no rich structure left for a structured input format to preserve.
Flattening is robust across CLI versions and trivially testable. This is not a
one-way door: if the spike shows the model losing track of turn boundaries,
switching to `stream-json` input is a localized change behind the same struct.

## Streaming translation

Read `claude`'s stdout line-by-line as JSON events:

- Assistant text-delta events → `Chunk::Text(delta)`.
- Terminal `result` event → `Chunk::Done(StopReason::Stop)`, or
  `StopReason::Length` if truncation is signaled.

Tool-call parsing is **not** done here. Accumulated text flows up and the
`Prompted` protocol extracts ` ```tool_call ` blocks exactly as it does for the
SGLang backend today.

## Error & lifecycle handling

New `ModelError` mappings:

- Binary not found on `PATH`.
- Not authenticated (no active subscription session).
- Rate-limited (subscription cap reached).
- Non-zero exit, with stderr captured into the error.

On stream drop / cancellation, the child process is killed via an RAII guard so
`claude` processes are not leaked. A per-turn timeout reuses the loop's existing
timeout config.

## Testing

- **Unit tests** on the stdout-JSON → `Chunk` parser with captured sample
  events (mirrors the existing `parse_sse_line` tests in `openai.rs`).
- **Integration test** using a **fake `claude` script**: a shell stub placed on
  `PATH` that emits canned stream-json, so CI needs no real subscription.
  Parallel to today's `e2e_sglang.rs` but fully hermetic.

## Top risk — Phase 0 spike (gating)

The Claude Code CLI ships its own baked-in agent system prompt, and the `-p`
binary has historically exposed only `--append-system-prompt`, **not** full
replacement. If the harness prompt cannot be suppressed, it competes with the
injected `Prompted` preamble and Claude may editorialize, refuse, or ignore the
`tool_call` fence format.

**Before any Rust is written**, run a ~30-minute spike: pipe a hand-written
transcript into
`claude -p --allowedTools "" --output-format stream-json` and confirm:

1. It emits clean, parseable text output.
2. It reliably produces the ` ```tool_call ` fenced format when instructed.
3. Its own tools are genuinely disabled.

**Decision point:**

- **Pass** → proceed with the design as written.
- **Fail** → fall back to `--append-system-prompt` (accept harness-prompt
  coexistence) or an Agent SDK sidecar (reintroduces a non-Rust process).

## Notes / out of scope

- Driving a subscription through an automated loop is subject to rate caps and
  sits outside typical interactive use; acceptable for local dev, flagged for
  awareness, not a blocker.
- The HTTP-shim approach (wrapping the CLI in an OpenAI-compatible server) was
  considered and rejected: it adds a second process/runtime and a network hop
  for no gain, since the Rust runtime owns both ends.
```
