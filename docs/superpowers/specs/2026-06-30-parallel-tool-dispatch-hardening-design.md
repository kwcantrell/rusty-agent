# Harden Parallel Tool Dispatch — Design

**Date:** 2026-06-30
**Status:** Approved (brainstorming) → ready for plan
**Source:** Finding 1 of the harness-engineering audit re-run
(`.agents/skills/harness-engineering/audit.md`, top-ranked after the two HIGHs
were fixed). Anchors re-verified against current `main` on 2026-06-30.

## Principle

A single misbehaving tool must not be able to take down or wedge a whole agent
turn. In the concurrent Phase-2 dispatch, one tool that **panics** or **hangs**
should be isolated to its own `tool_call_id`: the model receives an error
tool-result it can recover from, the operator sees a loud signal, and every other
tool in the batch still completes. Failure isolation is judged from first
principles + this runtime's own conventions (the external corpus thinly covers
parallel tool execution — see `audit.md` "Thinly-sourced components").

## Current state (verified on `main`, `loop_.rs`)

The turn's tool phase is three sequential stages:

- **Phase 1** (`loop_.rs:276-291`) gates each call sequentially; builds
  `order: Vec<String>` (one id per call) and seeds `results: HashMap<id,(name,Resolved)>`
  for rejected calls; approved calls go to `ready: Vec<ReadyCall>`.
- **Phase 2** (`loop_.rs:293-310`) runs `ready` through
  `futures::stream::iter(...).buffer_unordered(cap)` where each item is
  `async move { (id, name, tool.execute(args, &ctx).await) }`, collects, then
  inserts each result into `results`.
- **Phase 3** (`loop_.rs:312-329`) drains `results` in `order`, appending one
  `Message::tool(id, name, content)` per id.

Three gaps, per the finding — reconciled against the current code:

1. **Panic (OPEN).** `tool.execute(...).await` runs inline in the loop's own
   task. A `panic!` inside any tool unwinds through `buffer_unordered` and
   **aborts the entire `AgentLoop`** — one tool kills the session.
2. **Hang (OPEN).** Nothing wraps `tool.execute` in a timeout. `ctx.timeout`
   exists (`ToolCtx.timeout` = `config.tool_timeout`) and some tools honor it
   (shell), but a tool that ignores it (or a stalled MCP/HTTP call) occupies a
   `buffer_unordered` slot forever. Because `.collect()` awaits **all** items,
   one hung tool hangs the whole turn indefinitely.
3. **Silent drop (already closed; minor hardening).** The audit cited a silent
   `None => continue` in Phase 3. The `2026-06-25` tool-call-id contract
   (`normalize_tool_call_ids`, `loop_.rs:436`) now guarantees a unique, non-empty
   id per call, so every id in `order` has a `results` slot and the `continue`
   (`loop_.rs:318`) is unreachable. It remains a silent *drop* if that invariant
   is ever broken by a future change, which would produce a malformed transcript
   (an assistant `tool_call` with no matching tool message). We upgrade it to an
   explicit error message — defense-in-depth, not a live-bug fix.

## Goal

- A panicking tool is caught; its `tool_call_id` yields an error tool-result; the
  run continues; the panic is surfaced loudly.
- A hanging tool is bounded by `ctx.timeout` at the dispatch layer regardless of
  whether the tool honors the timeout itself; its slot frees and the turn
  proceeds; the timeout is surfaced loudly.
- Every `tool_call_id` in the assistant message always gets exactly one tool
  message, even under a future invariant break.

Non-goals: changing the concurrency cap, ordering, gating/approval flow, or the
`ctx.timeout` value. No new config knob — the dispatch backstop reuses
`ctx.timeout`.

## Surfacing

Refined after the final-review finding (below): the dispatch timeout is a **true
backstop** with a grace margin, so a tool that honors `ctx.timeout` itself hits
its own deadline first and keeps its own message and cleanup. Loudness therefore
splits:

- **Panic:** error tool-result to the model + `AgentEvent::Error` (visible in
  CLI/web) + `tracing::error!`. Always loud.
- **Backstop timeout** (tool ignored its own deadline and the grace-margin
  timer fired): error tool-result + `AgentEvent::Error` + `tracing::warn!`. Loud.
- **Tool-honored timeout** (the tool returned `ToolError::Timeout` before the
  backstop): flows through the normal `ToolErr` path — model-visible tool-result
  only, **quiet** (no `AgentEvent::Error`), preserving the tool's own message.
  A tool self-timing-out is a normal outcome, not an operator-visible fault.

(This refines the brainstorming "both loud" answer: the final review showed that
arming the backstop at exactly `ctx.timeout` would preempt every tool's own
timeout — racy at equal deadlines and discarding tool-specific messages — so the
backstop is armed with a grace margin instead, and only *it* is loud.)

The model-visible tool-result text is phrased so the model can recover
("tool '<name>' panicked" / "tool '<name>' timed out after <dur>").

## Architecture

Chosen approach: **in-place `catch_unwind` + `tokio::time::timeout`, with the
isolation logic extracted into a tested free function.** Rejected alternative:
`tokio::task::spawn` per tool — it abandons the bounded `buffer_unordered` cap
(would need a `Semaphore`) and changes cancellation (spawned tasks aren't dropped
with the loop), for no isolation benefit `catch_unwind` doesn't already provide
within the single task.

### Component 1 — `execute_isolated` (new free fn in `loop_.rs`)

```rust
/// Outcome of an isolated tool execution: the terminal result plus a tag the
/// caller uses to decide how loudly to surface it.
enum Executed {
    Ok(agent_tools::ToolOutput),
    ToolErr(String),   // tool returned Err — normal, model-visible only
    Panicked(String),  // tool panicked — loud
    TimedOut(String),  // dispatch timeout tripped — loud
}

/// Run one tool with panic + timeout isolation. Sink-free and `'static`-free so
/// it can be unit-tested without driving the loop; the caller owns event emission.
async fn execute_isolated(
    tool: std::sync::Arc<dyn Tool>,
    args: serde_json::Value,
    name: &str,
    ctx: &ToolCtx,
) -> Executed {
    use futures::FutureExt;
    let fut = std::panic::AssertUnwindSafe(tool.execute(args, ctx)).catch_unwind();
    // Grace margin: the backstop is armed at 2x the tool budget so a tool that
    // honors `ctx.timeout` itself resolves first (returning ToolError::Timeout,
    // routed quietly through ToolErr). The backstop only fires for a tool that
    // ignores its deadline entirely — the only case that is surfaced loudly.
    let backstop = ctx.timeout.saturating_mul(2);
    match tokio::time::timeout(backstop, fut).await {
        Ok(Ok(Ok(output)))      => Executed::Ok(output),
        // A tool's own ToolError::Timeout arrives here and stays quiet.
        Ok(Ok(Err(e)))          => Executed::ToolErr(format!("ERROR: {e}")),
        Ok(Err(_panic))         => Executed::Panicked(
            format!("ERROR: tool '{name}' panicked during execution")),
        Err(_elapsed)           => Executed::TimedOut(
            format!("ERROR: tool '{name}' ignored its {:?} timeout and was \
                     force-stopped by the dispatch backstop", ctx.timeout)),
    }
}
```

Notes:
- `catch_unwind` requires the future to be `UnwindSafe`; `AssertUnwindSafe`
  asserts it (standard for `async` futures). A caught panic leaves the loop's
  task intact.
- Nesting is `timeout(catch_unwind(execute))`: a panic resolves the inner
  future to `Ok(Err(_))` before the timeout; a hang trips `Err(Elapsed)`.
- `Executed` maps to the existing `Resolved` at the call site:
  `Ok→Resolved::Ok`, the three error variants → `Resolved::Err(text)`.

### Component 2 — Phase 2 wiring (`loop_.rs:293-310`)

The `buffer_unordered` closure calls `execute_isolated` and yields
`(id, name, Executed)`. The post-collect loop:

```rust
for (id, name, ex) in executed {
    let resolved = match ex {
        Executed::Ok(o)        => Resolved::Ok(o),
        Executed::ToolErr(s)   => Resolved::Err(s),
        Executed::Panicked(s)  => {
            tracing::error!(target: "loop", tool=%name, "tool panicked during parallel dispatch");
            self.sink.emit(AgentEvent::Error(s.clone()));
            Resolved::Err(s)
        }
        Executed::TimedOut(s)  => {
            tracing::warn!(target: "loop", tool=%name, timeout=?self.config.tool_timeout,
                "tool timed out during parallel dispatch");
            self.sink.emit(AgentEvent::Error(s.clone()));
            Resolved::Err(s)
        }
    };
    results.insert(id, (name, resolved));
}
```

Concurrency cap (`cap`), ordering, gating, and approval are untouched.

### Component 3 — Phase 3 transcript-validity upgrade (`loop_.rs:312-329`)

Replace:

```rust
let (name, resolved) = match results.remove(&id) {
    Some(v) => v,
    None => continue,
};
```

with an explicit error message so no `tool_call_id` is ever left without a
tool message:

```rust
let (name, resolved) = match results.remove(&id) {
    Some(v) => v,
    // Unreachable while normalize_tool_call_ids holds; if a future change ever
    // breaks the one-slot-per-id invariant, emit an error rather than drop the
    // result and desync the transcript.
    None => (String::new(), Resolved::Err(
        format!("ERROR: internal: no result for tool_call_id {id}"))),
};
```

(The `Message::tool(id, name, content)` append below is unchanged; an empty
`name` is acceptable for this should-never-happen path.)

## Error handling & edge cases

- **Panic isolation scope:** only the panicking tool's future is caught; siblings
  in the same `buffer_unordered` batch are unaffected and still collected.
- **Timeout vs. tool self-timeout:** the dispatch backstop is armed at
  `ctx.timeout * 2` (`saturating_mul`), a grace margin over the per-tool budget.
  A tool that honors `ctx.timeout` fires its own timeout first and returns
  `ToolError::Timeout` (quiet `ToolErr`, its own message preserved); the backstop
  only fires for a tool that ignores its deadline entirely, and only that case is
  loud. No double-penalty, no race at equal deadlines, no new config.
- **Cancellation:** `ctx.cancel` (Ctrl-C / SIGINT) still propagates into tools
  that honor it; the timeout is an independent bound. Both coexist.
- **A tool that ignores `ctx.timeout` and runs forever:** force-stopped by the
  backstop at `ctx.timeout * 2` and surfaced loudly. A tool that self-limits at
  `ctx.timeout` is cut by its own timer first (quietly), never reaching the
  backstop.
- **`catch_unwind` and `panic=abort`:** if the build ever sets
  `panic = "abort"`, `catch_unwind` cannot intercept — the process aborts. The
  workspace uses the default `unwind`; note the assumption, do not add config.

## Testing

**Unit (`execute_isolated`, no loop):** fake `Tool`s —
1. panics in `execute` → `Executed::Panicked`.
2. ignores `ctx.timeout` and `tokio::time::sleep`s past the `2x` backstop →
   `Executed::TimedOut` (drive with `#[tokio::test(start_paused = true)]`).
3. returns `Err(ToolError::Failed{..})` → `Executed::ToolErr`; returns `Ok` →
   `Executed::Ok`.
4. **honors** its own timeout — wraps its body in
   `tokio::time::timeout(ctx.timeout, …)` and returns `ToolError::Timeout` on
   elapse → `Executed::ToolErr` (quiet), and it resolves before the `2x`
   backstop (start_paused proves the grace margin lets the tool win the race).

**Loop-level (modeled on `parallel_tool_calls_execute_concurrently`,
`loop_.rs:1270`):**
- A batch with one panicking tool + one normal tool: the run completes
  (`Ok(())`), the normal tool's result lands, the panicker's `tool_call_id` gets
  an `ERROR: … panicked` tool message, and an `AgentEvent::Error` is emitted
  (assert via `CollectingSink` label `error:…`).
- A batch with one deadline-ignoring tool (sleeps ≫ `2 * tool_timeout`) + one
  normal tool under `start_paused` time: the turn does not hang, the backstop
  force-stops the offender which yields an `ERROR: … backstop` tool message +
  `AgentEvent::Error`, the normal tool's result lands.
- Ordering preserved: reuse the intent of
  `tool_results_keep_model_call_order_despite_completion_order` — one tool
  panicking/timing out does not disturb the `order`-based Phase-3 append.

All tests deterministic (no wall-clock sleeps in the loop tests beyond paused
`tokio::time`).

## Files touched

- `agent/crates/agent-core/src/loop_.rs` — add `Executed` enum + `execute_isolated`;
  rewire Phase 2 closure + post-collect loop; upgrade Phase 3 `None` arm; add unit
  + loop tests.

No other crates change. `AgentEvent::Error` and `ToolError` already exist.
