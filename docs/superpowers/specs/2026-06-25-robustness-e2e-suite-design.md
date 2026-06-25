# Deterministic Robustness E2E Suite — Design

**Date:** 2026-06-25
**Status:** Approved (brainstorming) → ready for plan

## Goal

Validate this session's five merged robustness/security fixes **end-to-end through
the real assembled loop** (`agent_runtime_config::assemble_loop` — the one builder
both front-ends use), **deterministically** (scripted model / wiremock, no live
server), so the suite runs in normal CI rather than being `#[ignore]`'d like the
existing live-model e2e. This complements — does not replace — the per-crate unit
tests already merged.

The fixes under validation:
- **C** — HTTP redirect host re-decision (`agent-http`) + read-path approval
  normalization (`agent-policy`). Merged `4858cb6`.
- **B1** — collision-proof tool-call id contract + `message_tokens`
  (`agent-core`). Merged `6383a7b`.
- **B2** — OpenAI stream robustness: truncation / skip-malformed-line /
  in-band-200-error (`agent-model`). Merged `ee74971`.
- **B3** — live cancellation via `run_with_cancel` + CLI Ctrl-C (`agent-core` +
  `agent-cli`). Merged `9e5681f`.

## Key enabling facts (verified)

- `agent_core::testkit` is `pub mod testkit` (lib.rs:7) → `ScriptedModel`,
  `Scripted`, `CollectingSink`, `AlwaysApprove` are usable from an external
  integration-test crate.
- `assemble_loop` takes `LoopParts { model: Arc<dyn ModelClient>, … }`, and
  `ScriptedModel` implements `ModelClient` — so the **real** registry, policy,
  tools, protocol, and sandbox are wired by `assemble_loop` while the model is
  deterministic.
- `assemble_loop` registers `FetchUrl::new(NetworkPolicy::new(http_allow_hosts))`
  (lib.rs:81), and `FetchUrl::new` hardcodes `SsrfGuard::strict()`. A loopback
  wiremock redirect target is therefore SSRF-blocked **before** the redirect
  policy is reached → a deterministic local redirect e2e is impossible through
  the assembled loop. The redirect fix stays unit-tested (where it constructs the
  tool with an `allow_all` guard).

## Architecture

One new file: `agent/crates/agent-runtime-config/tests/e2e_robustness.rs`.
- **Not `#[ignore]`'d** — deterministic, runs in `cargo test`.
- Reuses the harness shape from `e2e_auto_retrieval.rs`: a `Capture` `EventSink`
  (collects `Token` text, `ToolResult`/`ToolStart` names, `Approval` requests,
  and the terminal `Done`/`Error`), and per-test `ApprovalChannel` impls.
- Each test: build `cfg = RuntimeConfig::from_launch("openai", url, model,
  "native", 262_144)` with `cfg.memory = false` and `cfg.sandbox_mode = "off"`;
  call `assemble_loop(&cfg, LoopParts { model, sink, approval, workspace,
  mcp_tools: vec![], memory_tools: vec![], memory_retriever: None,
  stream_idle_timeout, base_system_prompt })`; drive `built.loop_.run(...)` (or
  `run_with_cancel`); assert on `Capture` + the `WindowContext` transcript
  (`ctx.build(limit)`).

## Tests

### T1 — B1: duplicate tool-call ids don't crash, yield distinct transcript ids
Model: `ScriptedModel` scripting one turn with **two `read_file` calls sharing id
`"c1"`** (against a real `a.txt` in the workspace), then a final text turn.
Approval: `AutoApprove`. Assert: `run` returns `Ok` (no panic); the transcript
(`ctx.build`) contains **two `Role::Tool` messages with distinct `tool_call_id`s**;
two `ToolResult` events for `read_file`. Validates B1 through the real loop +
real `read_file` tool + the post-parse normalization chokepoint.

### T2 — B3: pre-cancelled token stops the assembled loop cleanly
Model: any `ScriptedModel` (a single text turn). Drive
`built.loop_.run_with_cancel(&mut ctx, "go", token)` with a token cancelled
**before** the call. Assert: returns `Ok`; the `Capture`'s only/last event is
`Done(StopReason::Cancelled)`; no `Token`/`Usage` events (model never consulted).
Validates B3's caller-owned token through the assembled `run_with_cancel`.

### T3 — C: an escaping read routes through approval (normalized gate)
Model: `ScriptedModel` scripting a `read_file` call with path `"../../escape.txt"`,
then a text turn. Approval: a **recording, denying** channel that captures the
`ApprovalRequest` and returns `Deny`. Assert: an approval **was requested** for the
read (proving the normalized engine now returns `Ask` for a `..`-escaping read
instead of silently `Allow`); a denial error is fed back as the tool result (the
transcript carries the `ERROR:`/denied content, and the run still completes `Ok`).
Validates the C path-normalization gate-bypass fix through the real
`RulePolicy`/engine.

### T4 — B2: a truncated model stream is retried then surfaced as an error
Model: the **real** `OpenAiCompatClient` pointed at a **wiremock** server whose
`/v1/chat/completions` returns a 200 SSE body with a content delta but **no**
`finish_reason`/`[DONE]` (truncation), on every call. `cfg.max_retries` small
(e.g. 1) and a short `stream_idle_timeout`. Assert: `run` returns
`Err(AgentError::Model(_))` (the truncation became a retryable
`ModelError::Stream`, was retried, exhausted, then surfaced) **and** the `Capture`
recorded an `Error` event before the failure. Validates B2's truncation detection
propagating through `completion_with_retry` in the assembled loop. (Contrast: on
current code the truncated stream would look like a *clean* completion — `run`
would return `Ok` with no `Error` event — so this test fails if B2 is reverted.)

## Out of scope (documented in the suite header, not silently omitted)
- **C redirect** — strict SSRF in `assemble_loop`'s `FetchUrl` blocks any local
  wiremock redirect target; covered by the `agent-http` unit test (allow_all guard).
- **B2 skip-malformed / in-band-200-error** — pure client-layer SSE parsing,
  already wiremock-tested in `agent-model`.
- **B1 `message_tokens`** — pure function, already unit-tested in `agent-core`.

## Dependencies

`agent-runtime-config` dev-dependencies currently: `tempfile`, `tokio`,
`async-trait`. T1–T3 need no additions (`agent_core::testkit` via the existing
`agent-core` dep, `agent-model` already a dep for `OpenAiCompatClient`). **T4
adds `wiremock` to `[dev-dependencies]`** (a workspace dep already used by
`agent-http` / `agent-model` tests; the test asserts on `Capture` events, so no
stream-combinator imports are needed).

## Testing / success criteria

`cargo test -p agent-runtime-config` runs all four new tests (plus the existing
`#[ignore]`'d live ones, still skipped) and passes. Each test fails if its fix is
reverted: T1 panics, T2 hangs/over-runs, T3 sees no approval request, T4 sees no
`Error` event.

## Alternatives considered
- **Spread tests across each crate** — rejected: the value is validating the
  *assembled* loop (the integration the unit tests can't), which lives in
  `agent-runtime-config`.
- **Drive the CLI/server binary** — rejected: heavier, needs process/socket
  orchestration; `assemble_loop` is the same loop both front-ends run, so testing
  it directly is the right altitude for deterministic coverage.
- **Make them `#[ignore]`'d like the live e2e** — rejected: they're deterministic
  by construction, so they should gate CI.
