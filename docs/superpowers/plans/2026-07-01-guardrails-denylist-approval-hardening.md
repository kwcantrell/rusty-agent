# Guardrails Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close guardrails audit Finding 2 — add structural `mkfs`/forkbomb coverage to the catastrophic-command floor, and give the CLI approval prompt a timeout so a non-interactive stdin can't hang the agent.

**Architecture:** Two independent changes in two crates. (A) `agent-policy`: add an `mkfs` structural handler to `simple_command_is_catastrophic` and a second, all-whitespace-removed matching pass to the Layer-B substring backstop. (B) `agent-cli`: give `TerminalApproval` a `timeout` field + `Default`, wrap its blocking stdin read in `tokio::time::timeout`, deny on elapse.

**Tech Stack:** Rust, tokio (`spawn_blocking`, `time::timeout`), `shell-words` (already in use), `async_trait`.

## Global Constraints

- Rust workspace is `agent/`; run cargo from there (`source ~/.cargo/env` if `cargo` isn't on PATH).
- Conventional commits: `type(scope): summary`.
- Changes ship with tests; run the relevant `cargo test -p <crate>` before calling a task done.
- Spec: `docs/superpowers/specs/2026-07-01-guardrails-denylist-approval-hardening-design.md`.
- Fail-safe direction only: the hard floor is an absolute deny; over-denial of bizarre commands is acceptable, under-denial is not.

---

### Task 1: Structural `mkfs` handler + hardened forkbomb backstop

**Files:**
- Modify: `agent/crates/agent-policy/src/command.rs` (`simple_command_is_catastrophic` ~lines 59-76; `hard_floor_violation` ~lines 81-100; add `strip_ws` helper near `normalize_ws` ~line 43; tests in the in-file `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: existing `basename()`, `normalize_ws()`, `split_simple_commands()`, `hard_floor_violation(cmd, denylist)` — all in the same file.
- Produces: no new public API. `hard_floor_violation` gains stricter (superset) matching. New private `fn strip_ws(s: &str) -> String`.

**Note on TDD isolation:** the in-file test helper `floor()` uses the denylist
`["sudo", "rm -rf /", "dd if=", ":(){"]` — deliberately **without** `"mkfs"`. This
means the new `mkfs` tests can only pass via the *structural* handler (not the
substring backstop), and the spaced-forkbomb test can only pass via the new
*whitespace-stripped* backstop pass. Do not add `"mkfs"` to the `floor()` helper.

- [ ] **Step 1: Write the failing tests**

Add these three tests inside `mod tests` in `agent/crates/agent-policy/src/command.rs` (after `floor_denies_dd_and_fork_bomb`):

```rust
    #[test]
    fn floor_denies_mkfs_structurally() {
        // "mkfs" is NOT in the floor() denylist, so only the structural handler catches these.
        assert!(floor("mkfs /dev/sda").is_some());
        assert!(floor("mkfs.ext4 /dev/sdb1").is_some());
        assert!(floor("/sbin/mkfs.xfs /dev/sdb1").is_some());
        assert!(floor("echo hi && mkfs /dev/sda").is_some());
    }

    #[test]
    fn floor_denies_spaced_fork_bomb_via_stripped_backstop() {
        // Spaced variant dodges normalize_ws (single spaces remain) but not the
        // all-whitespace-removed pass, which collapses it to ":(){:|:&};:".
        assert!(floor(": ( ) { :|:& } ; :").is_some());
    }

    #[test]
    fn floor_allows_benign_despite_stricter_backstop() {
        assert!(floor("ls -la").is_none());
        assert!(floor("git status").is_none());
        assert!(floor("make build").is_none());   // 'mk' prefix must not trip mkfs
        assert!(floor("cat mkfs-notes.txt").is_none()); // 'mkfs' as an arg substring is fine (not in denylist)
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd agent && cargo test -p agent-policy floor_denies_mkfs_structurally floor_denies_spaced_fork_bomb_via_stripped_backstop`
Expected: FAIL — `floor("mkfs /dev/sda")` returns `None` (no structural handler yet), and the spaced forkbomb is not matched by the current single-space backstop. (`floor_allows_benign_despite_stricter_backstop` will already pass — that's fine, it's a regression guard.)

- [ ] **Step 3: Add the `strip_ws` helper**

In `agent/crates/agent-policy/src/command.rs`, immediately after the `normalize_ws` function (~line 45), add:

```rust
/// Remove ALL ASCII whitespace. A second backstop pass uses this so spacing variants
/// (`: ( ) { :|:& } ; :`) cannot dodge a denylist literal like `:(){`.
fn strip_ws(s: &str) -> String {
    s.split_whitespace().collect::<String>()
}
```

- [ ] **Step 4: Add the `mkfs` structural handler**

In `simple_command_is_catastrophic`, after the `dd` block (after line 74, before `None`), add:

```rust
    if name == "mkfs" || name.starts_with("mkfs.") {
        return Some(format!("filesystem creation via `{name}` is denied"));
    }
```

- [ ] **Step 5: Add the all-whitespace-removed backstop pass**

In `hard_floor_violation`, replace the Layer-B loop (the block starting `let norm = normalize_ws(cmd);` through the `for pat in denylist { ... }` loop, ~lines 92-98) with:

```rust
    let norm = normalize_ws(cmd);
    let stripped = strip_ws(cmd);
    for pat in denylist {
        let pnorm = normalize_ws(pat);
        if !pnorm.is_empty() && norm.contains(&pnorm) {
            return Some(format!("command matches denylist: {pat}"));
        }
        let pstripped = strip_ws(pat);
        if !pstripped.is_empty() && stripped.contains(&pstripped) {
            return Some(format!("command matches denylist: {pat}"));
        }
    }
```

- [ ] **Step 6: Run the full crate test suite to verify pass**

Run: `cd agent && cargo test -p agent-policy`
Expected: PASS — all existing tests plus the three new ones. Confirm no existing test (e.g. `floor_allows_benign_commands`, `auto_allow_*`) regressed.

- [ ] **Step 7: Commit**

```bash
cd agent && git add crates/agent-policy/src/command.rs
git commit -m "fix(policy): structural mkfs handler + whitespace-stripped denylist backstop"
```

---

### Task 2: Configurable timeout on `TerminalApproval`

**Files:**
- Modify: `agent/crates/agent-cli/src/approval.rs` (whole file — add field, `new`, `Default`, timeout wrapper, test module)
- Modify: `agent/crates/agent-cli/src/main.rs:216` (construction site)

**Interfaces:**
- Consumes: `agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse}`; `agent_tools::{Access, ToolIntent}` (test only); `tokio::task::spawn_blocking`, `tokio::time::timeout`.
- Produces: `TerminalApproval { timeout: Duration }` with `TerminalApproval::new(timeout: Duration) -> Self` and `impl Default` (300s). `impl ApprovalChannel for TerminalApproval` unchanged in signature; behavior now denies on timeout.

- [ ] **Step 1: Write the failing test**

Replace the file `agent/crates/agent-cli/src/approval.rs` with the version below **but first** — to make the test fail against the current code — temporarily keep the current unit-struct `TerminalApproval`. Simplest path: write the test module now (it references `TerminalApproval::new`, which does not exist yet), so the crate fails to compile.

Add to the bottom of `agent/crates/agent-cli/src/approval.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use agent_policy::{ApprovalRequest, ApprovalResponse};
    use agent_tools::{Access, ToolIntent};
    use std::time::Duration;

    fn req() -> ApprovalRequest {
        ApprovalRequest {
            intent: ToolIntent {
                tool: "bash".into(),
                access: Access::Write,
                paths: vec![],
                command: Some("echo hi".into()),
                summary: "run echo".into(),
            },
            display: None,
        }
    }

    #[tokio::test]
    async fn denies_when_timeout_elapses() {
        // A ~1ms timeout with no stdin input drives the timeout branch and returns Deny
        // promptly (the blocking read parks; the timeout fires first).
        let ch = TerminalApproval::new(Duration::from_millis(1));
        let resp = ch.request(req()).await;
        assert!(matches!(resp, ApprovalResponse::Deny));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd agent && cargo test -p agent-cli denies_when_timeout_elapses`
Expected: FAIL to compile — `TerminalApproval::new` and the `timeout` field do not exist yet.

- [ ] **Step 3: Implement the timeout on `TerminalApproval`**

Replace the non-test portion of `agent/crates/agent-cli/src/approval.rs` (everything above the `#[cfg(test)] mod tests` you just added) with:

```rust
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use async_trait::async_trait;
use std::io::Write;
use std::time::Duration;

/// Default interactive approval window. Matches the server's `APPROVAL_TIMEOUT`
/// (`agent-server/src/session.rs`) so both front-ends share the same human-friendly
/// timeout before auto-denying.
const DEFAULT_TERMINAL_APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

pub struct TerminalApproval {
    timeout: Duration,
}

impl TerminalApproval {
    #[allow(dead_code)]
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl Default for TerminalApproval {
    fn default() -> Self {
        Self { timeout: DEFAULT_TERMINAL_APPROVAL_TIMEOUT }
    }
}

#[async_trait]
impl ApprovalChannel for TerminalApproval {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
        // Run the blocking stdin read off the async runtime, bounded by a timeout.
        //
        // NOTE: std's blocking `read_line` cannot be cancelled, so on timeout the
        // spawned thread is orphaned — it stays parked on stdin until the next line or
        // EOF arrives, then its result is discarded. Harmless: one idle thread, and the
        // agent is no longer blocked. A clean cancel would need raw-fd polling, not worth
        // the complexity for a CLI approval prompt.
        let summary = req.intent.summary.clone();
        let handle = tokio::task::spawn_blocking(move || {
            print!("\n\x1b[35mAllow:\x1b[0m {summary} ? [y]es / [n]o / [a]lways: ");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() {
                return ApprovalResponse::Deny;
            }
            match line.trim().to_lowercase().as_str() {
                "y" | "yes" => ApprovalResponse::Approve,
                "a" | "always" => ApprovalResponse::ApproveAlways,
                _ => ApprovalResponse::Deny,
            }
        });
        match tokio::time::timeout(self.timeout, handle).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(_join_err)) => ApprovalResponse::Deny,
            Err(_elapsed) => {
                eprintln!("\nApproval timed out; denying.");
                ApprovalResponse::Deny
            }
        }
    }
}
```

- [ ] **Step 4: Update the construction site**

In `agent/crates/agent-cli/src/main.rs:216`, change:

```rust
        approval: Arc::new(TerminalApproval),
```

to:

```rust
        approval: Arc::new(TerminalApproval::default()),
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd agent && cargo test -p agent-cli`
Expected: PASS — `denies_when_timeout_elapses` passes and the crate builds. (The test returns `Deny` in ~1ms.)

- [ ] **Step 6: Build the workspace to confirm the wiring**

Run: `cd agent && cargo build`
Expected: clean build, no `dead_code` warning about `TerminalApproval` (it's fully constructed via `default()`).

- [ ] **Step 7: Commit**

```bash
cd agent && git add crates/agent-cli/src/approval.rs crates/agent-cli/src/main.rs
git commit -m "fix(cli): time-bound TerminalApproval stdin read, deny on elapse"
```

---

## Self-Review

**Spec coverage:**
- Part A `mkfs` structural handler → Task 1 Step 4. ✔
- Part A hardened forkbomb backstop (whitespace-removed pass) → Task 1 Steps 3, 5. ✔
- Part A tests (mkfs structural, path-qualified, spaced forkbomb, benign negatives) → Task 1 Step 1. ✔
- Part B `timeout` field + `new` + `Default` (300s) → Task 2 Step 3. ✔
- Part B `tokio::time::timeout` wrapper, deny on elapse, join-error deny, timeout notice → Task 2 Step 3. ✔
- Part B orphan-thread limitation comment → Task 2 Step 3 (code comment). ✔
- Part B construction site update → Task 2 Step 4. ✔
- Part B fast timeout-branch test → Task 2 Step 1. ✔
- Testing (`cargo test -p agent-policy`, `-p agent-cli`, `cargo build`) → Task 1 Step 6, Task 2 Steps 5-6. ✔

**Placeholder scan:** none — every code step shows complete code; every run step shows command + expected result.

**Type consistency:** `TerminalApproval::new(Duration)` used in Task 2 Step 1 test matches the definition in Step 3. `strip_ws(&str) -> String` defined in Task 1 Step 3, used in Step 5. `ApprovalRequest { intent, display }` and `ToolIntent { tool, access, paths, command, summary }` in the test match the real struct definitions (`engine.rs:14`, `types.rs:27-33`). `ApprovalResponse::{Approve, ApproveAlways, Deny}` matches `engine.rs:17`.
