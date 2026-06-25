# A-b — MCP Process & Task Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make MCP stdio-transport teardown explicit and deterministic — abort the reader/stderr tasks on `close()`/`Drop`, and reap the child explicitly in `SandboxedChild::kill()`.

**Architecture:** Two small independent changes: (1) `StdioTransport` (`agent-mcp`) stores the stdout-reader + stderr-drain `JoinHandle`s and aborts them on teardown; (2) `SandboxedChild::kill()` (`agent-tools`) `wait()`s after `start_kill()` to reap now rather than deferring to the `kill_on_drop` orphan reaper.

**Tech Stack:** Rust, `tokio` (`process`, `task::JoinHandle`, `time::timeout`), `async-trait`.

## Global Constraints

- TDD: failing test first, watch it fail (or pin the contract — see notes), then the minimal change.
- Run tests from the workspace root: `cd agent` first. `source ~/.cargo/env` if needed.
- Two crates: `agent-mcp`, `agent-tools`. No daemon/server changes (that's A-a/A-c).
- All teardown stays best-effort (`let _ = …`); `close()`/`kill()` must not fail or panic.
- Preserve existing behavior: every existing `transport.rs` / `sandbox.rs` test stays green.

## Reference — confirmed current code

- `agent-mcp/src/transport.rs`: `StdioTransport { stdin: AsyncMutex<ChildStdin>, inbound: AsyncMutex<mpsc::UnboundedReceiver<Value>>, child: Mutex<Option<agent_tools::SandboxedChild>> }`. `spawn()` does `tokio::spawn(...)` for the stderr drain (~45) and the stdout reader (~53), discarding both handles; the reader owns the `tx` whose `rx` is `inbound`. `close()` (~91-97) takes the child and `c.kill().await`. `Drop` (~100-105) takes the child. `std::sync::Mutex` and `tokio::sync::Mutex as AsyncMutex` are both already imported; `mpsc` is imported.
- `agent-tools/src/sandbox.rs`: `SandboxedChild { child: Child, container: Option<String> }`. `kill(&mut self)` (~67-75) does `docker kill` (container) then `self.child.start_kill()` and returns. `wait(&mut self)` (~63-65) exists. `Drop` (~78-91) uses `kill_on_drop`. `HostExecutor.launch` sets `kill_on_drop(true)`; `ProcKind::Service` keeps stdin piped.

---

### Task 1: `SandboxedChild::kill()` reaps explicitly (`agent-tools`)

**Files:**
- Modify: `agent/crates/agent-tools/src/sandbox.rs` (`SandboxedChild::kill`)
- Test: `agent/crates/agent-tools/src/sandbox.rs` (`mod tests`)

**Interfaces:**
- Produces: `kill(&mut self)` now reaps the child (`wait()` after `start_kill()`); still idempotent and best-effort.

- [ ] **Step 1: Write the tests**

Add to `mod tests` in `sandbox.rs` (the module already has `use super::*;` and `use std::time::Duration;`):

```rust
fn service_spec(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        cwd: std::env::temp_dir(),
        env: Default::default(),
        kind: ProcKind::Service,
    }
}

#[tokio::test]
async fn kill_reaps_a_long_running_child() {
    // A 30s sleeper: kill() must return almost immediately (kill + reap), not
    // block until the process would naturally exit.
    let mut sb = HostExecutor.launch(service_spec("sh", &["-c", "sleep 30"])).unwrap();
    tokio::time::timeout(Duration::from_secs(5), sb.kill())
        .await
        .expect("kill() must return promptly, not wait out the sleep");
}

#[tokio::test]
async fn kill_is_idempotent() {
    let mut sb = HostExecutor.launch(service_spec("sh", &["-c", "sleep 30"])).unwrap();
    sb.kill().await;
    // A second kill on an already-reaped child returns promptly and does not panic.
    tokio::time::timeout(Duration::from_secs(5), sb.kill())
        .await
        .expect("second kill() must return promptly");
}
```

- [ ] **Step 2: Run them (baseline — pass on current code)**

Run: `cd agent && cargo test -p agent-tools kill_reaps_a_long_running_child kill_is_idempotent` (run each name separately; `cargo test` takes one filter — use `cargo test -p agent-tools --lib kill_` to run both).
Expected: PASS on current code too (current `kill()` already returns promptly because `start_kill()` doesn't block). These tests **pin the contract** (prompt + idempotent + — after the change — reaping); the reaping itself is established by code review (an explicit `wait()` after a SIGKILL the child cannot ignore).

- [ ] **Step 3: Add the explicit reap**

In `sandbox.rs`, replace `SandboxedChild::kill`:

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

- [ ] **Step 4: Run the full crate suite**

Run: `cd agent && cargo test -p agent-tools --lib kill_` then `cd agent && cargo test -p agent-tools`
Expected: PASS — the two new tests pass, and all existing `sandbox.rs` tests (`host_executor_runs_and_captures_stdout`, `host_descriptor_is_host_mechanism`) stay green. No warnings.

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-tools/src/sandbox.rs
git commit -m "fix(sandbox): reap the child explicitly in SandboxedChild::kill

wait() after start_kill() so the process is reaped on the kill path instead of
relying on the kill_on_drop orphan reaper firing on a later SIGCHLD."
```

---

### Task 2: `StdioTransport` aborts its reader tasks (`agent-mcp`)

**Files:**
- Modify: `agent/crates/agent-mcp/src/transport.rs` (`StdioTransport` struct, `spawn`, `close`, `Drop`)
- Test: `agent/crates/agent-mcp/src/transport.rs` (`mod tests`)

**Interfaces:**
- Produces: after `close()` (or `Drop`), the stdout-reader and stderr-drain tasks are aborted; `recv()` returns `None` once the reader's `tx` is gone. Idempotent.

- [ ] **Step 1: Write the tests**

Add to `mod tests` in `transport.rs` (the module already has `cat_spec()`, `host_sandbox()`, `use super::*;`, `serde_json::json`):

```rust
#[tokio::test]
async fn close_tears_down_the_reader_task() {
    let sandbox = host_sandbox();
    let t = StdioTransport::spawn(&cat_spec(), &sandbox).expect("spawn cat");
    // Sanity: the transport works before teardown.
    t.send(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"ping"})).await.unwrap();
    let _ = t.recv().await.expect("a message");

    t.close().await;

    // After close, the reader task is gone (its tx dropped) -> recv yields None.
    let after = tokio::time::timeout(std::time::Duration::from_secs(5), t.recv())
        .await
        .expect("recv must resolve promptly after close, not hang");
    assert!(after.is_none(), "recv after close should be None, got: {after:?}");
}

#[tokio::test]
async fn close_is_idempotent() {
    let sandbox = host_sandbox();
    let t = StdioTransport::spawn(&cat_spec(), &sandbox).expect("spawn cat");
    t.close().await;
    t.close().await; // must not panic
}
```

- [ ] **Step 2: Run them (baseline)**

Run: `cd agent && cargo test -p agent-mcp --lib close_tears_down_the_reader_task` then `... close_is_idempotent`
Expected: PASS on current code (killing `cat` EOFs its stdout fast, so the reader ends and `recv()` returns `None` within the timeout even today). These pin the deterministic-teardown contract; the `abort()` added below is what guarantees it when a child ignores the kill, and the `timeout` keeps a regression from hanging the suite.

- [ ] **Step 3: Store the task handles on the struct**

In `transport.rs`, add fields to `StdioTransport`:

```rust
pub struct StdioTransport {
    stdin: AsyncMutex<ChildStdin>,
    inbound: AsyncMutex<mpsc::UnboundedReceiver<Value>>,
    child: Mutex<Option<agent_tools::SandboxedChild>>,
    reader: Mutex<Option<tokio::task::JoinHandle<()>>>,
    stderr: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
```

- [ ] **Step 4: Capture the handles in `spawn()`**

In `spawn()`, change the stderr-drain spawn to capture its handle, and the stdout-reader spawn likewise; then store both in the returned struct. Replace the stderr block:

```rust
        let stderr_handle = child.take_stderr().map(|stderr| {
            // Drain server diagnostics to tracing so they never block the pipe.
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    tracing::debug!(target: "mcp.server", "{l}");
                }
            })
        });
```

Replace the stdout reader spawn + the `Ok(Self { ... })`:

```rust
        let (tx, rx) = mpsc::unbounded_channel();
        let reader_handle = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(v) => {
                        if tx.send(v).is_err() {
                            break;
                        }
                    }
                    Err(e) => tracing::warn!(target: "mcp", error=%e, "non-JSON line from server"),
                }
            }
        });
        Ok(Self {
            stdin: AsyncMutex::new(stdin),
            inbound: AsyncMutex::new(rx),
            child: Mutex::new(Some(child)),
            reader: Mutex::new(Some(reader_handle)),
            stderr: Mutex::new(stderr_handle),
        })
```

- [ ] **Step 5: Abort the handles in `close()` and `Drop`**

Replace `close()`:

```rust
    async fn close(&self) {
        // Take child out of the Mutex first so the guard drops before the await.
        let child = self.child.lock().unwrap().take();
        if let Some(mut c) = child {
            c.kill().await;
        }
        // Deterministically tear down the reader/stderr tasks (don't wait for EOF).
        if let Some(h) = self.reader.lock().unwrap().take() { h.abort(); }
        if let Some(h) = self.stderr.lock().unwrap().take() { h.abort(); }
    }
```

Replace `Drop`:

```rust
impl Drop for StdioTransport {
    fn drop(&mut self) {
        // SandboxedChild's own Drop handles teardown — just drop it.
        let _ = self.child.lock().unwrap().take();
        // Abort the reader/stderr tasks (abort is sync — safe in Drop).
        if let Some(h) = self.reader.lock().unwrap().take() { h.abort(); }
        if let Some(h) = self.stderr.lock().unwrap().take() { h.abort(); }
    }
}
```

- [ ] **Step 6: Run the full crate suite**

Run: `cd agent && cargo test -p agent-mcp`
Expected: PASS — the two new tests pass, and the existing `stdio_roundtrips_newline_delimited_json_via_cat` (and MCP client/manager tests) stay green. No warnings (the `#[cfg(test)] MockTransport` is unaffected — it has no reader task).

- [ ] **Step 7: Commit**

```bash
cd agent && git add crates/agent-mcp/src/transport.rs
git commit -m "fix(mcp): abort stdio reader/stderr tasks on close/Drop

Store the stdout-reader and stderr-drain JoinHandles on StdioTransport and
abort them on teardown, so tasks are torn down deterministically instead of
lingering on next_line() until the child's pipes EOF."
```

---

### Task 3: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Build + both crates**

Run: `cd agent && cargo build && cargo test -p agent-tools -p agent-mcp 2>&1 | grep -E "test result|warning:"`
Expected: all `ok`, no warnings.

- [ ] **Step 2: Downstream sweep (consumers of the transport / sandbox)**

Run: `cd agent && cargo test -p agent-server -p agent-runtime-config 2>&1 | grep "test result"`
Expected: PASS — `kill()` and `close()` signatures are unchanged, so no consumer breaks.

- [ ] **Step 3: Confirm the spec's testing checklist is satisfied**

Cross-check against `docs/superpowers/specs/2026-06-25-mcp-process-lifecycle-design.md` → "Testing": the close-teardown + close-idempotent transport tests and the kill-reaps + kill-idempotent sandbox tests are all present and passing. If any is missing, add it.
