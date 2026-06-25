# B3 — Live Cancellation Wiring — Design

**Date:** 2026-06-25
**Status:** Approved (brainstorming) → ready for plan
**Source:** Cluster B (loop robustness) of the security/robustness audit backlog
(`2026-06-24-security-audit-backlog.md`). Third and final Cluster B sub-spec; B1
(tool-call id contract, `6383a7b`) and B2 (stream robustness, `ee74971`) are
merged. Finding re-verified against current `main` on 2026-06-25.

## Principle

The loop already routes a `CancellationToken` into every `ToolCtx`, and tools
already honor it (`shell.rs:64`, `git.rs:37`, `agent-http/src/tool.rs:70,85,184`).
But the token is a throwaway `CancellationToken::new()` (`loop_.rs:308`) with no
source — the comment at `loop_.rs:302-304` documents it as an inert stub. Give the
loop a *real* token owned by the caller, check it where work blocks, and let each
front-end supply the source. A hung tool or a runaway multi-turn run then becomes
abortable.

## Scope decision (settled in brainstorming)

**In:** loop plumbing + CLI Ctrl-C source. **Deferred to Cluster A:** interactive
server cancel (a wire `Cancel` message + per-session token storage) — the daemon
spawns a detached task per `UserInput` over one global `session` id (`daemon.rs:104,108`),
and per-session token storage is owned by Cluster A's session/cross-talk rework.
The server call site here only passes a token to satisfy the new signature.

## Finding — cancellation token is inert (MED)

`AgentLoop::run` has no cancel source. `gate_tool` builds each `ToolCtx` with a
fresh `CancellationToken::new()` (`loop_.rs:307-309`) that nothing ever cancels,
and the CLI (`agent-cli/src/main.rs:240`) just `await`s `agent.run(...)`. So a
tool that hangs, or a model loop that runs away across turns, cannot be aborted
mid-turn — there is no Ctrl-C / SIGINT path and no programmatic handle.

## Component 1 — `run` becomes cancellable (`agent-core/src/loop_.rs`)

Signature gains the caller's token:

```rust
pub async fn run(&self, ctx: &mut dyn ContextManager, user_input: String,
                 cancel: CancellationToken) -> Result<(), AgentError>
```

The loop threads `cancel` into the work that can block, checking it at three
points:

1. **Top of each turn** (`for turn in 0..max_turns`): first statement
   ```rust
   if cancel.is_cancelled() {
       self.sink.emit(AgentEvent::Done(StopReason::Cancelled));
       return Ok(());
   }
   ```
   Stops a runaway multi-turn loop, and catches a cancel that fired during the
   tool phase of the previous iteration.

2. **Active model streaming** (`one_completion`, threaded `&cancel`): add
   `cancel.cancelled()` to the existing `tokio::select!`s (both the stream-open
   and the per-chunk `timeout`), returning early on cancel. `completion_with_retry`
   (threaded `&cancel`) checks `cancel.is_cancelled()` in its error branch and
   returns `AgentError::Cancelled` **without retrying**, so a cancelled stream is
   not re-attempted. This interrupts a long generation promptly rather than waiting
   for the idle timeout.

3. **In-flight tools** (`gate_tool`, threaded `&cancel`): replace
   `cancel: CancellationToken::new()` (`loop_.rs:308`) with `cancel: cancel.clone()`.
   The real token now reaches `ctx.cancel`, so the tools' existing `select`s abort
   and return (typically `ToolError::Timeout`); those error results append in
   Phase 3 as usual, keeping the transcript well-formed before the top-of-loop
   check (point 1) exits on the next iteration.

`run` maps `Err(AgentError::Cancelled)` from `completion_with_retry` to the same
clean outcome as point 1 (emit `Done(Cancelled)`, return `Ok(())`).

## Component 2 — Cancellation outcome

On cancel, `run` emits `AgentEvent::Done(StopReason::Cancelled)` and returns
`Ok(())`. Cancellation is a *user action*, not a fatal error: the REPL resumes at
the prompt, and any UI gets a proper terminal `Done` event rather than a stream
that simply stops. In-flight tools bail and append `ERROR: …` results before the
loop exits, so the context stays valid for the next turn.

### New types (both additive)
- `agent-model/src/types.rs`: `StopReason::Cancelled` added to the enum. The model
  parsers map *to* `StopReason` via a wildcard (`openai.rs:200` `_ => Stop`), so
  they are unaffected; the only exhaustive match that must gain an arm is
  `stop_reason_str` in `agent-server/src/wire.rs:91-94` →
  `StopReason::Cancelled => "cancelled"`.
- `agent-core/src/loop_.rs`: `AgentError::Cancelled` added to the enum (currently
  only `Model(String)`), e.g. `#[error("cancelled")] Cancelled`.

## Component 3 — CLI Ctrl-C source (`agent-cli/src/main.rs`)

Wrap the `run` call so the first Ctrl-C cancels and we keep awaiting `run` until
it finishes its cleanup (so in-flight tools tear down cleanly):

```rust
let cancel = tokio_util::sync::CancellationToken::new();
let run = agent.run(&mut ctx, input.to_string(), cancel.clone());
tokio::pin!(run);
let result = loop {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => { cancel.cancel(); eprintln!("\n^C cancelling…"); }
        r = &mut run => break r,
    }
};
if let Err(e) = result { eprintln!("\x1b[31mfatal: {e}\x1b[0m"); }
```

A second Ctrl-C while the loop is still draining just re-cancels (idempotent).
`tokio::signal::ctrl_c` is already available — workspace `tokio` is
`features = ["full"]` (includes `signal`). `agent-cli` does **not** currently
depend on `tokio-util`, so add `tokio-util.workspace = true` to
`crates/agent-cli/Cargo.toml` for `CancellationToken` (it is already a workspace
dependency, `tokio-util = "0.7"`).

## Component 4 — Server call site (deferred wiring)

`agent-server/src/daemon.rs:111` passes a fresh `CancellationToken::new()` to
satisfy the new signature, with a `// TODO(cluster-A): fire on a Cancel wire
message` marker. No server behavior change — interactive server cancel is part of
Cluster A.

## Error handling

`AgentError::Cancelled` and `StopReason::Cancelled` are the only new types, both
additive; no existing variant changes. A cancelled run is `Ok(())` to the caller.
The CLI prints the `^C` notice from its signal handler; the loop emits the
terminal `Done(Cancelled)` event.

## Testing (TDD — write the failing test first)

In `agent-core` (`loop_.rs` tests, using the existing `ScriptedModel` /
`PassthroughProtocol` / `CollectingSink` / `AlwaysApprove` harness):

- **Boundary check:** call `run` with an already-cancelled token → returns
  `Ok(())` immediately, the sink's last event is `Done(Cancelled)`, and the model
  is never consulted (no `token:` events; the scripted turn is untouched).
- **Tool-hang abort (deterministic, no sleeps):** a test-only tool whose `execute`
  notifies a `tokio::sync::Notify` that it has started, then `await`s
  `ctx.cancel.cancelled()` and returns `Err(ToolError::Timeout)`. The test scripts
  the model to call that tool, drives `run` concurrently, `await`s the
  start-notify, cancels the token, and asserts `run` returns `Ok(())` with a final
  `Done(Cancelled)` event — proving a hung tool is abortable.
- **Regression — un-cancelled token is a no-op:** every existing `run(...)` test
  passes a fresh, never-cancelled `CancellationToken`, and all existing behavior
  (tool runs, denials, retrieval, the B1 duplicate-id test) stays green.

`StopReason::Cancelled` wire mapping is covered by `agent-server`'s existing
`stop_reason_str` tests if present; otherwise add a one-line assertion that it
maps to `"cancelled"`.

## Scope

**In scope:**
- `agent-core/src/loop_.rs`: `run` signature, the three cancellation check-points,
  `one_completion`/`completion_with_retry` threading, `gate_tool` token wiring,
  `AgentError::Cancelled`.
- `agent-model/src/types.rs`: `StopReason::Cancelled`.
- `agent-server/src/wire.rs`: `stop_reason_str` arm.
- `agent-cli/src/main.rs`: Ctrl-C source; `agent-cli/Cargo.toml`: add
  `tokio-util.workspace = true`.
- `agent-server/src/daemon.rs`: pass a token at the call site (deferred wiring).
- Every other `run(...)` call site (tests across crates) updated to pass a token.

**Out of scope (explicit non-goals):**
- Interactive server cancel (wire `Cancel` message + per-session token storage) —
  Cluster A.
- Cancelling already-appended context / rolling back the user message — the
  transcript is left well-formed as-is.
- Cluster A findings (server concurrency & leaks).

## Alternatives considered

1. **Token param on `run` + check-points + CLI Ctrl-C — CHOSEN.** Explicit,
   per-run, unit-testable by passing and cancelling a token; the front-end owns the
   source.
2. **Cancel handle stored on `AgentLoop`.** The server shares `Arc<AgentLoop>`
   across runs (`current_loop()`), so a loop-held token would be shared across
   concurrent runs and need per-run reset — messier than a per-call parameter.
3. **Turn-boundary + tool-only cancellation (drop point 2).** Smaller (no
   `one_completion`/`completion_with_retry` changes, no streaming `select`), but
   Ctrl-C during an active long generation wouldn't take effect until the stream
   ended or idle-timed-out — the main reason users hit Ctrl-C. Rejected for worse
   responsiveness.
