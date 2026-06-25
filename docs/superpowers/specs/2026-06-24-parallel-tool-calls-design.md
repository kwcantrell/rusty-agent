# Concurrent execution of parallel tool calls in the agent loop — design

**Status:** approved, ready for implementation plan
**Date:** 2026-06-24
**Scope:** one focused change to `AgentLoop::run`'s tool-execution path + test coverage

## Context

The local llama.cpp server (`llama-agent`, Qwen3.6-35B-A3B) emits OpenAI-style
**parallel tool calls** — multiple `tool_calls` in a single assistant turn
(`chat_template_caps.supports_parallel_tool_calls: true`; verified live: a "weather
in Paris and Tokyo" prompt returns two `get_weather` calls in one turn). We want the
agent loop to handle this "properly."

Exploration found the runtime is **already correct** for the multi-call case on the
native path: parsing iterates the full `tool_calls` array (`merge_tool_call` keys on
the streaming `index` so interleaved fragments reassemble), each result is matched to
its own `ToolCall.id`, one `role:"tool"` message is appended per call in order, and a
per-call failure is isolated as `ERROR: …` without aborting the turn. The single-call
assumption exists **only** in the prompted-JSON fallback (`prompted.rs`), which is not
the llama.cpp path.

Two gaps remain:

1. **Execution is sequential** — `loop_.rs:181` runs `for call … run_tool(call).await`,
   so parallel calls run one-after-another with no concurrency benefit.
2. **No test proves the multi-call pipeline end-to-end** — merge logic is unit-tested in
   isolation, but no loop-level or e2e test drives an assistant turn emitting multiple
   calls and asserts N correctly-id-matched results. The testkit's `Scripted::Call`
   carries only one call per step.

This change closes both: parallelize execution **safely**, and lock the correctness with
tests.

## Decisions (locked during brainstorming)

1. **Scope = both:** add the missing multi-call tests (guarantee correctness) **and**
   make execution concurrent (latency win).
2. **Concurrency model = two-phase**, not naive `join_all(run_tool)`. Naive parallelism
   would fire simultaneous interactive approval prompts and interleave the event stream.
3. **Approval stays serialized** — one prompt at a time, in call order. Only the actual
   tool I/O parallelizes.
4. **Output ordering = model's call order**, never completion order — deterministic
   context and deterministic tests.
5. **Bounded concurrency** via a new `max_parallel_tools` config (default **8**) so a turn
   with many `bash` calls can't fork-bomb the sandbox.
6. **Mutation safety is the model's responsibility** (matches OpenAI/Anthropic): concurrent
   parallel calls are assumed independent. Per-tool read/write classification is an
   explicit **non-goal** here; it is the escape hatch if races ever bite.

## Mechanism — three phases replacing the `for` loop at `loop_.rs:181`

`run_tool` (`loop_.rs:198-239`) splits into a **gate** step and an **execute** step.

### Phase 1 — Gate (sequential, in call order)
For each `call` in `parsed.tool_calls`:
- emit `ToolStart`
- resolve via `self.tools.get(&call.name)` (returns `Arc<dyn Tool>` — cloneable)
- `tool.intent(&call.args)`
- `self.policy.check(&intent)`:
  - `Allow` → `Ready`
  - `Deny(reason)` → `Rejected("ERROR: …")`
  - `Ask` → emit `Approval`, await `self.approval.request(req)` **one at a time** →
    approve → `Ready`; else `Rejected("ERROR: user declined")`
- unknown tool / intent error → `Rejected("ERROR: …")`

Yields an ordered `Vec<GateOutcome>` where
`Ready { tool: Arc<dyn Tool>, args, id, name, ctx: ToolCtx }` or
`Rejected { id, name, content }`.

### Phase 2 — Execute (concurrent, bounded)
Drive all `Ready` futures (`tool.execute(args, &ctx)`) with a `FuturesUnordered`
gated by a `tokio::sync::Semaphore(max_parallel_tools)`. Each resolves to
`(id, name, Result<ToolOutput, ToolError>)`. `Rejected` outcomes already hold terminal
content and skip this phase.

### Phase 3 — Append (sequential, original call order)
Walk `parsed.tool_calls` in order; for each id, take its executed-or-rejected result,
emit `ToolResult` on success, and `ctx.append(Message::tool(id, name, content))`.
Ordering is by the model's call order regardless of which future finished first.

## Error handling (semantics unchanged)

Per-call failures remain `ERROR: …` content for that call; siblings are unaffected (they
are already running concurrently). Denied/unknown are resolved in the gate. One failure
never aborts the turn. The existing protocol-repair and budget paths are untouched.

## Files touched

- `agent/crates/agent-core/src/loop_.rs` — replace the sequential tool block; split
  `run_tool` into `gate_tool` (per-call, sequential) and `execute_ready` (concurrent),
  then ordered append.
- agent `LoopConfig` (in `agent-core`) — add `max_parallel_tools: usize` (default 8);
  thread through any constructor/CLI default.
- `agent/crates/agent-core/src/testkit.rs` — add `Scripted::Calls(Vec<(String,String,String)>)`
  emitting multiple `ToolCallDelta` chunks in one turn (keep `Call` as the single-call
  sugar).
- Dependencies: `futures::stream::FuturesUnordered` (crate already used) and
  `tokio::sync::Semaphore` (tokio already a dep).

## Testing

Correctness lock (1–2), concurrency guarantees (3–5), real-world (6):

1. **Multi-call happy path** — ScriptedModel emits 3 calls; assert the assistant message
   carries 3 `tool_calls` and 3 `tool` messages are appended, ids matched, in order.
2. **Per-call error isolation** — 3 calls, the middle one unknown/denied → 3 results,
   middle is `ERROR`, others OK, turn continues to the next completion.
3. **Concurrency proof** — two tools share a `tokio::sync::Barrier(2)`; each completes only
   once both have started. Sequential execution deadlocks (caught by a test timeout);
   concurrent passes.
4. **Order under out-of-order completion** — call-1's tool sleeps longer than call-2's →
   assert `tool` messages are still appended in order 1, 2.
5. **Approval serialization** — two `Ask` calls → an approval channel that records in-flight
   count asserts it never exceeds 1.
6. **e2e smoke (opt-in, non-blocking)** — mirror `tests/e2e_sglang.rs` exactly:
   `#[ignore = "requires AGENT_E2E_URL / AGENT_E2E_MODEL and a live server"]`, driven by
   the `AGENT_E2E_URL` / `AGENT_E2E_MODEL` (+ optional `AGENT_API_KEY`) env vars, run via
   `cargo test -p agent-core --test <file> -- --ignored --nocapture`. Invisible to normal
   `cargo test`; it is **not** a required/CI-blocking gate. Design for a clean manual
   signal:
   - **Deterministic local tools** — read two distinct workspace files (or one tool
     invoked with two different args), so result content is checkable without an external
     API.
   - **Low temperature + an unambiguous "do BOTH" prompt** to coax two parallel calls.
   - **Distinguish a loop bug from model behavior:** assert that for however many calls
     came back, the loop produced exactly that many correctly-id-matched `tool` messages
     in order. If the model emits `<2` calls, **fail with an explicit "model did not emit
     parallel calls — inconclusive" message** rather than a misleading pass.

   Rationale for opt-in (not required): emitting *parallel* calls is probabilistic, and the
   test needs a live server + host-network boundary. Tests 1–5 are the deterministic
   correctness lock that runs everywhere; #6 only adds confirmation that the real chat
   template + SSE wire format still round-trips multiple `tool_calls`.

## Verification

- `cargo test -p agent-core` — all new + existing loop tests green.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`.
- Optional e2e against `llama-agent` on `localhost:8080` (cargo path requires
  `source ~/.cargo/env`; live-server tests need the host-network boundary).

## Non-goals

- Per-tool read/write parallel-safety classification (decision 6).
- Pipelining approval with execution (approve call 1 while approving call 2). Phase
  separation is intentionally simple; revisit only if approval latency becomes a problem.
- Any change to the prompted-JSON fallback protocol (single-call by design).
