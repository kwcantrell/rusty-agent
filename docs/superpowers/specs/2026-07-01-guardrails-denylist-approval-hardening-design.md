# Guardrails hardening: denylist gaps + CLI approval hang

**Date:** 2026-07-01
**Component:** Harness Anatomy #5 — Guardrails / Hooks
**Source finding:** `.agents/skills/harness-engineering/audit.md` Finding 2 (also ranked #1 in Top 3)
**Status:** design approved; ready for implementation plan

## Problem

The `harness-engineering` audit surfaced two gaps in the runtime's catastrophic-command
guardrail and its interactive approval channel:

1. **Structural denylist gaps.** `simple_command_is_catastrophic`
   (`agent/crates/agent-policy/src/command.rs:59-76`) has structural (Layer-A)
   handlers for `sudo`/`doas`/`su`, recursive-`rm`-of-root, and `dd` writing to a
   block device — but **not** for `mkfs` or the `:(){` forkbomb. Both are present in
   `HARD_FLOOR_DENYLIST` (`agent/crates/agent-runtime-config/src/runtime_config.rs:8`)
   yet are caught **only** by the Layer-B substring backstop, which is bypassable by
   spacing/quoting variants and depends on the denylist literal staying configured. No
   test exercises `mkfs` through `hard_floor_violation`.

2. **CLI approval can hang the agent.** `TerminalApproval`
   (`agent/crates/agent-cli/src/approval.rs:13-27`) runs a blocking `stdin` read via
   `spawn_blocking` with **no timeout**, unlike the server's `IpcApprovalChannel`
   (`agent/crates/agent-server/src/approval.rs:53`), which wraps its wait in
   `tokio::time::timeout(self.timeout, …)` and denies on elapse. A non-interactive
   caller that holds stdin open (a pipe that never sends a newline) hangs the agent
   indefinitely.

**Violated principle:** "hooks are fast, side-effect-free validators; block bad
actions, not delay good ones" — SKILL.md Spine A, component 5.

The audit note confirms the surrounding design is sound and out of scope here: gate
ordering (policy → approval → execute) and default-Deny-when-no-approver are both
correct. This spec only closes the two named gaps.

## Design

### Part A — Structural `mkfs` handler + hardened forkbomb backstop

**File:** `agent/crates/agent-policy/src/command.rs`

1. **`mkfs` structural handler.** In `simple_command_is_catastrophic`, after the
   existing `dd` check, add:

   ```rust
   if name == "mkfs" || name.starts_with("mkfs.") {
       return Some(format!("filesystem creation via `{name}` is denied"));
   }
   ```

   `name` is already the basename (`basename(prog)`), so this fires for `mkfs`,
   `mkfs.ext4`, `mkfs.xfs`, and path-qualified forms like `/sbin/mkfs.vfat`. Unlike
   the substring backstop it fires regardless of whether `"mkfs"` is in the configured
   denylist. The precise `mkfs.` prefix (rather than a bare `starts_with("mkfs")`)
   avoids matching an unrelated program that merely begins with `mkfs`.

2. **Hardened forkbomb backstop.** The forkbomb `:(){ :|:& };:` is a shell
   *function-definition* construct, not a "program + args" simple command, so the
   tokenizer-based Layer-A is the wrong layer for it — a true structural parser would
   be brittle against function-name variants (`b(){ b|b& };b`). It stays on Layer B.

   Harden Layer B so spacing variants cannot dodge the `:(){ ` literal: in
   `hard_floor_violation`, in addition to the existing `normalize_ws` (collapse-runs)
   comparison, also compare an **all-whitespace-removed** form of both the command and
   each denylist pattern. Add a helper (e.g. `strip_ws(s: &str) -> String`) alongside
   `normalize_ws` and check `norm.contains(pnorm) || stripped.contains(pstripped)`.

   This is strictly a **superset** of current matching (fail-safe direction). At the
   catastrophic hard floor, over-denial of bizarre-looking commands is acceptable by
   design — the hard floor is an absolute deny, and no benign, normal-looking command
   collapses to contain `:(){`, `sudo`, `rm-rf/`, or `ddif=` once whitespace is
   removed.

3. **Tests** (in the existing `#[cfg(test)] mod tests`, via the `floor` helper).
   Extend the test denylist to include `"mkfs"` so backstop parity is covered, and add:
   - `floor("mkfs /dev/sda").is_some()` — structural, bare `mkfs`.
   - `floor("mkfs.ext4 /dev/sdb1").is_some()` — structural, `mkfs.` variant.
   - `floor("/sbin/mkfs.xfs /dev/sdb1").is_some()` — structural, path-qualified.
   - A spaced forkbomb variant, e.g. `floor(": ( ) { :|:& } ; :").is_some()` — exercises
     the hardened all-whitespace-removed backstop (the existing no-space
     `:(){ :|:& };:` case already passes and is retained).
   - Negative: confirm benign commands still pass (`floor("ls -la").is_none()`,
     `floor("git status").is_none()`), guarding against false positives from the
     stricter backstop.

### Part B — Configurable timeout on `TerminalApproval`

**Files:** `agent/crates/agent-cli/src/approval.rs`, `agent/crates/agent-cli/src/main.rs`

1. Give `TerminalApproval` a `timeout: Duration` field, a `new(timeout: Duration)`
   constructor, and a `Default` impl of `Duration::from_secs(300)` — matching the
   server's existing `APPROVAL_TIMEOUT` constant
   (`agent/crates/agent-server/src/session.rs:26`), so both front-ends share the same
   generous, human-friendly interactive window. Drop the now-unnecessary
   `#[allow(dead_code)]` if the type is fully used.

2. Wrap the existing `spawn_blocking` stdin read in `tokio::time::timeout`:

   ```rust
   let handle = tokio::task::spawn_blocking(move || { /* existing read + match */ });
   match tokio::time::timeout(self.timeout, handle).await {
       Ok(Ok(resp)) => resp,          // user responded
       Ok(Err(_join_err)) => ApprovalResponse::Deny,   // task panicked/cancelled
       Err(_elapsed) => {             // timed out
           eprintln!("\nApproval timed out; denying.");
           ApprovalResponse::Deny
       }
   }
   ```

   Semantics mirror `IpcApprovalChannel`: elapse → `Deny`, join error → `Deny`.

3. **Known, accepted limitation** (documented in a code comment): std's blocking
   `read_line` cannot be cancelled, so on timeout the `spawn_blocking` thread is
   orphaned — it stays parked on stdin until the next line or EOF arrives, at which
   point its result is discarded. This is harmless: a single idle thread, and the
   agent is no longer blocked. Cancelling a blocking stdin read cleanly would require
   raw-fd polling, which is out of scope and not worth the complexity for a CLI
   approval prompt.

4. Update the single construction site `agent/crates/agent-cli/src/main.rs:216` from
   `Arc::new(TerminalApproval)` to `Arc::new(TerminalApproval::default())`.

## Testing

- `cargo test -p agent-policy` — new `command.rs` cases (Part A).
- `cargo test -p agent-cli` — add a unit test in `approval.rs` constructing
  `TerminalApproval::new(Duration::from_millis(1))` and asserting `request(...)`
  resolves to `ApprovalResponse::Deny` promptly, driving the timeout branch without
  depending on real stdin input. (No stdin is written, so the blocking read parks and
  the timeout fires.)
- `cargo build` — whole workspace compiles.

## Scope guards (YAGNI)

- **No `RuntimeConfig` plumbing** for the terminal timeout — a field plus `Default` is
  sufficient; the single caller can override via `new(...)` if a need ever arises.
- **No true structural forkbomb parser** — brittle against function-name variants; the
  `:(){` signature is handled at the substring layer that fits it, now hardened against
  whitespace evasion.
- Gate ordering and default-Deny-when-no-approver are **out of scope** — the audit
  confirmed both are already sound.

## Files touched

| File | Change |
|---|---|
| `agent/crates/agent-policy/src/command.rs` | `mkfs` structural handler; `strip_ws` helper + all-whitespace-removed backstop pass; new tests |
| `agent/crates/agent-cli/src/approval.rs` | `timeout` field + `new` + `Default`; `tokio::time::timeout` wrapper; limitation comment; new timeout test |
| `agent/crates/agent-cli/src/main.rs` | construct `TerminalApproval::default()` |

## Follow-ups (not in this spec)

Finding 1 (Observability) and Finding 3 (Instructions) from the same audit remain
open and are tracked in `audit.md`.
