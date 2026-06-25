# A-b — MCP Process & Task Lifecycle — Design

**Date:** 2026-06-25
**Status:** Approved (brainstorming) → ready for plan
**Source:** Cluster A (server concurrency & resource leaks) of the
security/robustness audit backlog (`2026-06-24-security-audit-backlog.md`).
First of three Cluster A sub-specs (A-a bounded channel, A-b this, A-c run
lifecycle/session/server-cancel). Finding re-verified against current `main`
on 2026-06-25.

## Principle

Make MCP stdio-transport teardown **explicit and deterministic** instead of
relying on pipe-EOF timing plus tokio's best-effort orphan reaper. Two small,
independent changes in two crates.

## Finding A3 — honest current state

The backlog lists A3 as HIGH ("reader tasks never aborted; children killed but
never `wait()`ed → zombies"). Re-verification against current code shows it is
**largely mitigated** but **non-deterministic**:

- `agent-sandbox/src/strategy.rs:38` sets `kill_on_drop(true)` on the container
  `docker run` client child, and `agent-tools/src/sandbox.rs:105` sets it on the
  host child. So tokio's Drop reaper already kills + best-effort-reaps **both**
  paths.
- `StdioTransport::spawn` is called once per server (`agent-mcp/src/manager.rs:112`)
  — there is **no reconnect loop**, so the originally-described
  reconnect-cycle leak does not exist.

What remains is non-deterministic, best-effort teardown:
- The stdout-reader (`transport.rs:53`) and stderr-drain (`transport.rs:45`)
  `JoinHandle`s are discarded. The tasks only end when the child's pipes EOF
  (i.e. after the OS tears the process down) — they are not aborted on `close()`.
- `SandboxedChild::kill()` (`sandbox.rs:67-75`) issues `start_kill()` (signal
  only) and returns; the child is reaped by the `kill_on_drop` orphan reaper on a
  later SIGCHLD, not by an explicit `wait()`.

This spec hardens both to deterministic, explicit teardown. Severity in practice
is closer to MED than HIGH given `kill_on_drop`; the value is testable, FD-clean,
deterministic shutdown rather than closing an unbounded leak.

## Component 1 — `StdioTransport` aborts its reader tasks (`agent-mcp/src/transport.rs`)

Capture the two spawned tasks' handles on the struct and abort them on teardown.

- Add to `StdioTransport`:
  ```rust
  reader: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
  stderr: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
  ```
- `spawn()` stores the handles instead of discarding them:
  - the stderr-drain handle is `Some(...)` only when `child.take_stderr()` returned
    a pipe, else `None`;
  - the stdout-reader handle is always `Some(...)`.
- `close()`: after `c.kill().await`, abort and clear both handles:
  ```rust
  if let Some(h) = self.reader.lock().unwrap().take() { h.abort(); }
  if let Some(h) = self.stderr.lock().unwrap().take() { h.abort(); }
  ```
  (Order: kill the child first so the pipes close, then abort — abort is the
  deterministic backstop if the task is still blocked on `next_line()`.)
- `Drop`: abort and clear both handles (abort is synchronous — safe in `Drop`),
  alongside the existing child `take()`.

Both `Mutex<Option<…>>` make the abort idempotent: a second `close()` (or
`close()` then `Drop`) finds `None` and is a no-op.

## Component 2 — `SandboxedChild::kill()` reaps explicitly (`agent-tools/src/sandbox.rs`)

Make reaping explicit rather than deferred to the Drop reaper:

```rust
/// Kill the container (docker kill) or the local child, then reap it; idempotent best-effort.
pub async fn kill(&mut self) {
    if let Some(name) = &self.container {
        let _ = tokio::process::Command::new("docker")
            .args(["kill", name]).output().await;
    }
    // Intentional dual-kill: docker kill stops the container; start_kill reaps
    // the local foreground `docker run` client process.
    let _ = self.child.start_kill();
    // Reap now instead of relying on the kill_on_drop orphan reaper.
    let _ = self.child.wait().await;
}
```

`kill()` is already `async` and idempotent: the added `wait()` returns
immediately once SIGKILL lands, and immediately if the child has already exited
(so a second `kill()` is a no-op). `Drop` stays unchanged as the `kill_on_drop`
backstop (it cannot await).

## Error handling

All teardown stays best-effort (`let _ = …`): `close()`/`kill()` are shutdown
paths and must not fail. Aborting an already-finished task and `wait()`ing an
already-exited child are both no-ops.

## Testing (TDD — write the failing test first)

**`agent-mcp/src/transport.rs`** (reuses the existing `cat` + `HostExecutor`
test harness):
- `close_tears_down_the_reader_task`: spawn `cat`, round-trip one message
  (sanity), call `close()`, then `recv().await` returns `None` within a short
  `tokio::time::timeout` — pinning the contract that after `close()` the reader
  task is gone (its `tx` dropped, closing `inbound`). (Honest note: for a
  fast-exiting child like `cat` the pipe EOFs quickly even on current code, so
  this test pins the deterministic-teardown *contract* rather than being
  guaranteed to fail on a revert; the `abort()` is what guarantees teardown when a
  child ignores the kill. The `timeout` keeps a regression from hanging the suite.)
- `close_is_idempotent`: a second `close()` after the first does not panic.

**`agent-tools/src/sandbox.rs`** (reuses the `HostExecutor` test harness):
- `kill_reaps_a_long_running_child`: launch a `Service` child
  (`sh -c "sleep 30"`), `kill()` it inside a short `tokio::time::timeout`, and
  assert it returns well under the 30 s sleep — proving it kills+reaps rather than
  blocking on natural exit.
- `kill_is_idempotent`: call `kill()` twice; the second returns promptly and does
  not panic.

All existing `transport.rs` / `sandbox.rs` tests stay green.

## Scope

**In scope:** `agent-mcp/src/transport.rs` (`StdioTransport` fields + `spawn`/
`close`/`Drop`), `agent-tools/src/sandbox.rs` (`SandboxedChild::kill`), and tests
in both.

**Out of scope (explicit non-goals):**
- **A-a** — bounded event channel + backpressure (`daemon.rs`/`sink.rs`).
- **A-c** — run lifecycle, session identity, atomic `RuntimeState`, and the
  deferred B3 interactive server-cancel.
- Reconnect/restart logic — there is no reconnect path (`spawn` once per server).
- Changing `Drop` to do explicit async reaping — `Drop` cannot await;
  `kill_on_drop` remains its backstop.

## Alternatives considered
1. **Store + abort task handles; explicit `wait()` in `kill()` — CHOSEN.**
   Deterministic, testable, small.
2. **Rely on EOF + `kill_on_drop` (status quo).** Works in the common case but is
   non-deterministic and leaves tasks blocked on `next_line()` until the OS closes
   the pipe — not testable, and depends on the orphan reaper firing.
3. **Replace the unbounded reader channel with a bounded one.** Out of scope —
   that is the A-a backpressure concern, not process lifecycle.
