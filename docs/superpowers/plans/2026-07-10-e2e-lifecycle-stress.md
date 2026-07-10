# E2E Lifecycle & Stress Suite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the gate-approved e2e lifecycle & stress suite (spec: `docs/superpowers/specs/2026-07-10-e2e-lifecycle-stress-design.md`): two small product seams (E1 metadata-root override, E2 CLI approval-timeout knob), a `testkit` lift in agent-server, a new `agent/crates/agent-e2e` crate with three drivers, the 22-scenario Tier-1 matrix, Tier-2 WebDriver additions, and a Tier-3 live pass.

**Architecture:** Tier 1 drives the CLI binary as a subprocess and the GUI's exact surface (`agent_server::session::Session` + `EventOut`/`ServerEvent`) in-process — plus a harness binary (`e2e-daemon`) so daemon-side scenarios get real SIGKILLs. A scripted wiremock model stub makes every scenario deterministic. All state (workspace, sessions root, metadata root/secret) lives in per-test tempdirs via the E1 seam.

**Tech Stack:** Rust, tokio, wiremock 0.6, tempfile, nix (signals), clap (existing CLI), thirtyfour (Tier 2, existing harness).

## Global Constraints

- Branch: `feature/e2e-lifecycle-stress` off `main` (Task 1 creates it). Conventional commits (`type(scope): summary`). Commit at the end of every task; never push.
- Two Cargo workspaces: `agent/` and `src-tauri/`. All `-p` flags below target `agent/` unless a step says otherwise. Run cargo from the workspace dir (`cd agent && cargo ...`).
- **Zero flakes / no sleeps as sync**: every wait is deadline-bounded (≤30s cap) on an observable event (file exists, event frame, stub request, stdout marker). On expiry: SIGKILL the child's process group, fail with captured stdout/stderr.
- **Process hygiene**: children held in a KillOnDrop guard (process-group kill via the held `Child`). Never kill by name/pattern.
- Spawned CLIs always get `--stream-timeout-secs 10` and (once Task 3 lands) `--approval-timeout-secs` explicitly.
- `file:line` anchors in this plan are orientation only — **locate the quoted code by content before editing** (earlier tasks shift lines).
- Every kill/reopen/tamper step is preceded by a positive assertion that `parked.json` exists (spec §2.4 positive-artifact rule).
- Error assertions: one stable substring + "no panic" + "session dir intact" — never full message text, never bare `is_err()`.
- Gate-triggering tool calls are `write_file` (deterministic `Ask` under default policy); never execute files from the temp workspace (`/tmp` is noexec).
- **Checkpoint artifact paths (plan-review F2):** `parked.json`, `manifest.json`, `answer.json`, `resume.lock` all live under the parked run's root dir = **`<session_dir>/checkpoint/`** (the root Checkpointer is built on `session_dir.join("checkpoint")` — `agent-cli/src/main.rs` ~:640). NEVER join these names onto the session dir directly; always go through `rig::ckpt(session_dir)` (Task 5).
- **Runtime flavor (plan-review F3):** every `#[tokio::test]` in this plan is shorthand — write `#[tokio::test(flavor = "multi_thread")]` in real code. The sync waiters (`wait_for_output`, `wait_exit`, `wait_until`) block a thread; on current_thread they deadlock against the in-runtime wiremock/Session tasks until the watchdog.
- **Prompts have no trailing newline (plan-review F1):** the approval/feedback/REPL prompts are `print!`-ed without `\n` — the CLI driver must read raw byte chunks, never `BufReader::lines()`.
- The shared test-file preamble is a menu — **trim unused imports per file** (`clippy -D warnings` gates the crate). Every test that uses a `ScriptedStub` ends with `assert_consumed()`; where a test deliberately leaves spare steps (retry ambiguity), say so in a comment instead of calling it.
- `bash scripts/ci.sh` must pass before the branch is called done.

## Verified source facts this plan builds on

(All verified against live code 2026-07-10; quoted so task implementers don't re-derive them.)

- `metadata_root()` = `$HOME/.rusty-agent`, parameterless (`agent-runtime-config/src/session_meta.rs:71`). Non-test consumers: `agent-server/src/runtime.rs:103`, `agent-server/src/session.rs:146`, `agent-cli/src/main.rs:161` (reopen), `agent-cli/src/main.rs:635` (run). `sessions_root(cfg)` already honors `cfg.trace_dir`.
- The CLI's `RuntimeConfig` is **flag-derived only** (no config-file flag, no `--trace-dir` flag today); `runtime_config_from_cli` maps flags → config.
- `TerminalApproval::with_park_exit(park_exit)` hardcodes `DEFAULT_TERMINAL_APPROVAL_TIMEOUT = 300s` (`agent-cli/src/approval.rs:14,78-82`); call sites at `main.rs:217` and `main.rs:645`.
- Approval prompt dialogue (plain stdin, no tty): `"? [y]es / [n]o / [a]lways: "`; reopen deny then prompts `"Feedback for the agent (optional, Enter to skip): "` (`approval.rs:44-63,108-140`). Park-exit hint: `"run parked; answer later with"`.
- Session driving: `agent_server::setup::local_params(workspace, config_path, base_url, model) -> DaemonParams`; `Session::from_params(params) -> Arc<Session>`; `set_event_out(Arc<dyn EventOut>)`, `send_input(String) -> SendOutcome` (`Busy` when a turn is live), `approve(&str, Decision)`, `cancel()`. `Decision` (wire) = `Approve | ApproveAlways | Deny { feedback: Option<String> }`.
- `ServerEvent` variants used: `Token`, `Error{message}`, `Done{reason}`, `ApprovalRequest{id, summary, command, ...}`, `ParkedRuns{runs}`, `ApprovalResolved{id}`, `Resumed{session_id}` (serde `tag="type"`, snake_case).
- Lock contention: server `start_resume` emits `ServerEvent::Error` containing `"is being resumed elsewhere"` (`session.rs:345-355`); CLI reopen claims the lock **before** prompting and exits 2 with the same substring (`main.rs:188-200`). `claim_resume` is O_EXCL (`agent-core/src/checkpoint.rs:298-312`).
- New input on a session with parks: `send_input` checks only the in-memory `active` slot; success-reap scopes to the resumed run's `root_dir` (`session.rs:409-412`). Scenario 9 characterizes whether a *plain* completed turn in the same session dir reaps a parked tree — assert survival; a failure is a real product finding.
- Park-write failure degrades to live-only: warn + `AgentEvent::Error("checkpoint write failed (approval not durable): ...")`, ask still functions (`agent-core/src/loop_.rs` ~1390, `checkpoint.rs:686`).
- SSE stub format the runtime parses (from `agent-model/src/openai.rs` tests): `data: {"choices":[{"delta":{"content":"..."}}]}\n\n` chunks, tool calls via `delta.tool_calls`, terminated by `data: {"choices":[{"delta":{},"finish_reason":"stop"}]}\n\n` + `data: [DONE]\n\n`; header `content-type: text/event-stream`.
- Desktop app (Tier 2): `base_url`/`model` hardcoded (`:8080`, `qwen3.6-35b-a3b`); config under `app_config_dir` (XDG) — relocatable only via `HOME`/`XDG_CONFIG_HOME` env inherited through tauri-driver. T2.3's live turn uses the real model on :8080 (like existing `turn_smoke`).
- `agent sessions reopen <id>` takes model/backend/base_url/protocol from **current flags**, workspace from the descriptor.
- Existing testkit precedent: `agent-core` uses `#[cfg(any(test, feature = "testkit"))]` (`agent-core/src/lib.rs:16`). Helpers to lift from `agent-server/src/session.rs` test mod: `Captured` sink (~:598), `plant_parked_session` (~:1560), `plant_parked_session_with_command` (~:1572), `wait_for_ask_id` (~:1671).

## File Structure

| File | Responsibility |
|---|---|
| `agent/crates/agent-runtime-config/src/runtime_config.rs` | E1: `metadata_dir` field (+partial/merge) |
| `agent/crates/agent-runtime-config/src/session_meta.rs` | E1: `metadata_root_for(cfg)` |
| `agent/crates/agent-server/src/runtime.rs`, `src/session.rs` | E1: consumer threading |
| `agent/crates/agent-server/src/testkit.rs` (new), `src/lib.rs`, `Cargo.toml` | testkit lift (feature-gated helpers) |
| `agent/crates/agent-cli/src/main.rs` | E1 flags `--trace-dir`/`--metadata-dir`; E2 flag `--approval-timeout-secs`; consumer threading |
| `agent/crates/agent-cli/src/approval.rs` | E2: timeout parameter on `with_park_exit` |
| `agent/crates/agent-e2e/Cargo.toml`, `src/lib.rs` | new tests-only crate (publish = false) |
| `agent/crates/agent-e2e/src/rig.rs` | tempdir isolation rig + Session-leg constructor |
| `agent/crates/agent-e2e/src/stub.rs` | scripted wiremock stub + raw-socket drop helper |
| `agent/crates/agent-e2e/src/cli.rs` | CLI subprocess driver (spawn/stdin/waiters/signals/KillOnDrop/bin freshness) |
| `agent/crates/agent-e2e/src/bin/e2e-daemon.rs` | harness bin: real-process Session host (run / hold-lock modes) |
| `agent/crates/agent-e2e/tests/{lifecycle,crashkill,concurrency,adversarial,robustness}.rs` | Tier-1 scenarios 1–22 |
| `agent/crates/agent-e2e/tests/live_smoke.rs` | Tier 3 (`--ignored`) |
| `src-tauri/tests/gui_lifecycle.rs`, `tests/e2e_harness/mod.rs` | Tier 2 (`--ignored`); harness gains env injection |
| `.agents/skills/auto-drive-tauri/SKILL.md` | one-paragraph Tier-2 pointer (full E4 de-stale is a separate follow-up) |

---

### Task 1: E1a — `metadata_dir` config field + `metadata_root_for`

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (field + partial + merge, mirror `approval_auto_deny_secs`)
- Modify: `agent/crates/agent-runtime-config/src/session_meta.rs` (new fn + tests)

**Interfaces:**
- Produces: `RuntimeConfig.metadata_dir: Option<String>` (serde default, file-overlayable) and `pub fn metadata_root_for(cfg: &RuntimeConfig) -> Option<PathBuf>` (override else `metadata_root()`), re-exported from `agent_runtime_config` (lib.rs re-export list already exports `metadata_root`; add `metadata_root_for`).

- [ ] **Step 1: Create the branch**

```bash
cd /home/kalen/rust-agent-runtime && git checkout -b feature/e2e-lifecycle-stress
```

- [ ] **Step 2: Write the failing tests** (append to `session_meta.rs` test mod and `runtime_config.rs` test mod)

```rust
// session_meta.rs tests
#[test]
fn metadata_root_for_honors_override_else_home() {
    let mut cfg = crate::RuntimeConfig::default();
    cfg.metadata_dir = Some("/tmp/x-meta".into());
    assert_eq!(metadata_root_for(&cfg), Some(PathBuf::from("/tmp/x-meta")));
    cfg.metadata_dir = None;
    assert_eq!(metadata_root_for(&cfg), metadata_root());
}
```

```rust
// runtime_config.rs tests (mirror approval_auto_deny_secs_defaults_none_and_roundtrips)
#[test]
fn metadata_dir_defaults_none_and_roundtrips() {
    assert_eq!(RuntimeConfig::default().metadata_dir, None);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.json");
    std::fs::write(&path, r#"{"metadata_dir": "/tmp/m"}"#).unwrap();
    let loaded = RuntimeConfig::default().overlaid_from(&path);
    assert_eq!(loaded.metadata_dir, Some("/tmp/m".into()));
}
```

(Adjust the overlay call to the exact API used by the neighboring `approval_auto_deny_secs` test — copy its shape verbatim.)

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-runtime-config metadata_dir metadata_root_for`
Expected: FAIL to compile (`no field metadata_dir`, `metadata_root_for not found`).

- [ ] **Step 4: Implement**

In `runtime_config.rs`, next to `approval_auto_deny_secs` in all three places (struct, partial struct, merge fn):

```rust
/// E1 seam (e2e spec 2026-07-10): override for the metadata root
/// ($HOME/.rusty-agent) so tests/custom setups never touch the real one.
#[serde(default)]
pub metadata_dir: Option<String>,
```

```rust
// partial struct
metadata_dir: Option<String>,
// merge fn, same pattern as approval_auto_deny_secs
if let Some(v) = p.metadata_dir {
    self.metadata_dir = Some(v);
}
```

In `session_meta.rs`, directly below `metadata_root()`:

```rust
/// Metadata root honoring the `metadata_dir` override (E1); falls back to
/// the real $HOME/.rusty-agent. The secret, and anything else under the
/// metadata root, follows this — both surfaces must resolve it the same way.
pub fn metadata_root_for(cfg: &crate::RuntimeConfig) -> Option<PathBuf> {
    match &cfg.metadata_dir {
        Some(d) => Some(PathBuf::from(d)),
        None => metadata_root(),
    }
}
```

Add `metadata_root_for` to the `pub use session_meta::{...}` list in `agent-runtime-config/src/lib.rs`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-runtime-config`
Expected: PASS (all, not just the new ones).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat(config): E1 metadata_dir override + metadata_root_for (e2e spec seam)"
```

---

### Task 2: E1b — thread `metadata_root_for` through all four consumers + CLI flags

**Files:**
- Modify: `agent/crates/agent-server/src/runtime.rs` (content: `let meta = agent_runtime_config::metadata_root()?` inside the checkpoint/secret setup)
- Modify: `agent/crates/agent-server/src/session.rs` (content: `let Some(meta) = agent_runtime_config::metadata_root() else` in Session construction)
- Modify: `agent/crates/agent-cli/src/main.rs` (two sites: reopen `metadata_root()` near `load_or_create_secret`; run-path `metadata_root().and_then(...)`; plus new flags)

**Interfaces:**
- Consumes: `metadata_root_for(&RuntimeConfig)` from Task 1.
- Produces: CLI flags `--trace-dir <PATH>` and `--metadata-dir <PATH>` mapped into `RuntimeConfig` by `runtime_config_from_cli`; all secret loads resolve through the config.

- [ ] **Step 1: Write the failing CLI mapping test** (in `main.rs` test mod, near `select_protocol_*` tests)

```rust
#[test]
fn cli_dirs_flags_map_into_runtime_config() {
    let cli = Cli::parse_from([
        "agent", "--trace-dir", "/tmp/s", "--metadata-dir", "/tmp/m",
    ]);
    let rt = runtime_config_from_cli(&cli, "native");
    assert_eq!(rt.trace_dir.as_deref(), Some("/tmp/s"));
    assert_eq!(rt.metadata_dir.as_deref(), Some("/tmp/m"));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-cli cli_dirs_flags`
Expected: FAIL to compile (no `trace_dir` field on `Cli`).

- [ ] **Step 3: Implement**

Add to the `Cli` struct (near `--workspace`):

```rust
/// Session artifacts root override (default: ~/.rusty-agent/sessions)
#[arg(long)]
trace_dir: Option<String>,
/// Metadata root override — secret etc. (default: ~/.rusty-agent). E1 seam.
#[arg(long)]
metadata_dir: Option<String>,
```

In `runtime_config_from_cli`, before `c` is returned:

```rust
c.trace_dir = cli.trace_dir.clone();
c.metadata_dir = cli.metadata_dir.clone();
```

Replace each of the four consumer sites (locate by content, not line):
- `agent-cli/src/main.rs` reopen: `agent_runtime_config::metadata_root()` → `agent_runtime_config::metadata_root_for(&rt)` (an `rt` is already built in `run_sessions_reopen`; confirm and reuse it).
- `agent-cli/src/main.rs` run path: `agent_runtime_config::metadata_root().and_then(...)` → `agent_runtime_config::metadata_root_for(&rt).and_then(...)`.
- `agent-server/src/runtime.rs`: the secret-loading closure has the config in scope (it builds from `RuntimeConfig`); change `metadata_root()` → `metadata_root_for(config)` (match the actual binding name in scope).
- `agent-server/src/session.rs`: same change inside `Session::from_params` (the `DaemonParams.config` is in scope).

- [ ] **Step 4: Run the affected crates' tests**

Run: `cd agent && cargo test -p agent-cli -p agent-server -p agent-runtime-config`
Expected: PASS. (Server tests that deliberately touch real `$HOME` — see comments at `runtime.rs` ~:980 — still pass because default config leaves `metadata_dir: None`.)

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(cli,server): thread metadata_root_for through secret loads; --trace-dir/--metadata-dir flags (E1)"
```

---

### Task 3: E2 — `--approval-timeout-secs` flag

**Files:**
- Modify: `agent/crates/agent-cli/src/approval.rs` (signature + doc)
- Modify: `agent/crates/agent-cli/src/main.rs` (flag + two call sites)

**Interfaces:**
- Produces: `TerminalApproval::with_park_exit(park_exit: Option<ParkExit>, timeout: Duration)`; CLI flag `--approval-timeout-secs <u64>` default 300.

- [ ] **Step 1: Write the failing test** (in `approval.rs` test mod)

```rust
#[test]
fn with_park_exit_honors_custom_timeout() {
    // Same-module test may read the private field — no stdin involvement
    // (plan-review F6: the public request() path would park an orphan
    // blocking read_line on real process stdin).
    let ch = TerminalApproval::with_park_exit(None, Duration::from_secs(7));
    assert_eq!(ch.timeout, Duration::from_secs(7));
    let d = TerminalApproval::with_park_exit(None, DEFAULT_TERMINAL_APPROVAL_TIMEOUT);
    assert_eq!(d.timeout, Duration::from_secs(300));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-cli with_park_exit_honors`
Expected: FAIL to compile (wrong arity).

- [ ] **Step 3: Implement**

```rust
/// `park_exit: None` degrades to timeout-denies. `timeout` is the interactive
/// approval window (E2 knob; CLI default is DEFAULT_TERMINAL_APPROVAL_TIMEOUT).
pub fn with_park_exit(park_exit: Option<ParkExit>, timeout: Duration) -> Self {
    Self {
        park_exit,
        ..Self::new(timeout)
    }
}
```

In `main.rs`: add the flag

```rust
/// Interactive approval window in seconds; on expiry the run parks-and-exits
/// (durable park wired) or denies. E2 knob.
#[arg(long, default_value_t = 300)]
approval_timeout_secs: u64,
```

and update both call sites (content-match `TerminalApproval::with_park_exit(`):

```rust
let approval = TerminalApproval::with_park_exit(
    /* existing ParkExit expr unchanged */,
    std::time::Duration::from_secs(cli.approval_timeout_secs),
);
```

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(cli): --approval-timeout-secs knob for the terminal approval window (E2)"
```

---

### Task 4: testkit lift in agent-server

**Files:**
- Create: `agent/crates/agent-server/src/testkit.rs`
- Modify: `agent/crates/agent-server/src/lib.rs`, `agent/crates/agent-server/Cargo.toml`, `agent/crates/agent-server/src/session.rs` (test mod imports)

**Interfaces:**
- Produces (under `#[cfg(any(test, feature = "testkit"))]`, module `agent_server::testkit`):
  - `pub struct Captured(pub Mutex<Vec<ServerEvent>>)` implementing `EventOut`, plus `impl Captured { pub fn snapshot(&self) -> Vec<ServerEvent> }`
  - `pub async fn plant_parked_session(ws: &Path, sessions: &Path, meta: &Path, prior_id: &str) -> (Arc<Session>, Arc<Captured>, [u8; 32])` — key from `load_or_create_secret(meta)`, `metadata_dir` set on the internal Session's config (plan-review F4: the 3-arg form would key against the developer's real `~/.rusty-agent` and break rig-rooted verification). Keep a thin `#[cfg(test)]` 3-arg wrapper in `session.rs` defaulting to `metadata_root()` so existing tests are untouched.
  - `pub async fn plant_parked_session_with_command(ws: &Path, sessions: &Path, meta: &Path, prior_id: &str, command: &str) -> (Arc<Session>, Arc<Captured>, [u8; 32])`
  - `pub async fn wait_for_ask_id(cap: &Captured, timeout: Duration) -> String`

- [ ] **Step 1: Move, don't rewrite.** Cut `Captured`, `plant_parked_session`, `plant_parked_session_with_command`, `wait_for_ask_id` (and any private fns only they use) from `session.rs`'s `#[cfg(test)] mod tests` into new `src/testkit.rs`, making items `pub` and fixing imports (`use crate::session::Session; use crate::wire::{EventOut, ServerEvent};` etc.). **E1 interaction:** the planted-session helpers currently resolve the secret via real `metadata_root()` (`.expect("HOME set")` pattern); parameterize them to accept the config they build so the rig can inject `metadata_dir` — keep the old behavior when `metadata_dir` is `None`. Add a `snapshot()` accessor so external crates don't reach into the `Mutex` field layout.

- [ ] **Step 2: Wire the feature.** `lib.rs`:

```rust
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
```

`Cargo.toml` (agent-server): add `[features] testkit = []`, and move any test-only deps the helpers need (e.g. `tempfile`) from `[dev-dependencies]` to optional deps activated by the feature **only if compilation demands it** — prefer keeping the helpers dep-free.

- [ ] **Step 3: Update `session.rs` tests** to `use crate::testkit::{Captured, plant_parked_session, plant_parked_session_with_command, wait_for_ask_id};` and delete the moved definitions.

- [ ] **Step 4: Verify**

Run: `cd agent && cargo test -p agent-server && cargo clippy -p agent-server --all-targets --features testkit -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "refactor(server): lift session test helpers into feature-gated testkit module"
```

---

### Task 5: agent-e2e crate scaffold + isolation Rig

**Files:**
- Create: `agent/crates/agent-e2e/Cargo.toml`, `src/lib.rs`, `src/rig.rs`

**Interfaces:**
- Produces:
  - `pub struct Rig { pub workspace: TempDir, pub sessions: TempDir, pub meta: TempDir, pub key: [u8; 32] }`
  - `Rig::new() -> Rig` — creates tempdirs and **pre-creates** the 32-byte secret at `<meta>/secret` (mode 0o600) so no `load_or_create_secret` race exists.
  - `Rig::runtime_config(&self, base_url: &str) -> RuntimeConfig` — `from_launch("openai", base_url, "stub-model", "native", 32768)` + `trace_dir`/`metadata_dir` pointed at the rig.
  - `Rig::session(&self, base_url: &str) -> (Arc<Session>, Arc<Captured>)` — the in-process GUI leg: `local_params(...)` with `params.config` overridden by `runtime_config()` fields, config-path JSON written into the rig with the same overrides (so a file overlay can't undo them), `Session::from_params`, `set_event_out(Captured)`.
  - `pub fn wait_until(deadline: Duration, poll: impl Fn() -> bool) -> bool` — bounded poll helper (10ms interval), the ONLY generic waiter; everything else waits on events/files through it.
  - `Rig::session_dirs(&self) -> Vec<PathBuf>` and `Rig::only_session_dir(&self) -> PathBuf` — session-dir discovery under `sessions` root.
  - `pub fn ckpt(session_dir: &Path) -> PathBuf` (= `session_dir.join("checkpoint")`) — the ONLY way any test derives artifact paths (F2), and `Rig::assert_parked(&self, session_dir: &Path)` (positive-artifact rule, checks `ckpt(dir)/parked.json`).

- [ ] **Step 1: Cargo.toml**

```toml
[package]
name = "agent-e2e"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
agent-core = { path = "../agent-core" }
agent-server = { path = "../agent-server", features = ["testkit"] }
agent-runtime-config = { path = "../agent-runtime-config" }
tokio = { workspace = true, features = ["full"] }
serde = { workspace = true }
serde_json = { workspace = true }
tempfile = { workspace = true }
wiremock = { workspace = true }
nix = { version = "0.29", features = ["signal"] }

[[bin]]
name = "e2e-daemon"
path = "src/bin/e2e-daemon.rs"
```

(Use `workspace = true` only for deps actually declared in `agent/Cargo.toml`'s `[workspace.dependencies]`; otherwise pin the same version the other crates use — check before writing. `nix` may need adding; keep features minimal: `signal`, `process`.)

- [ ] **Step 2: lib.rs**

```rust
//! Test-support library for the e2e lifecycle & stress suite
//! (spec: docs/superpowers/specs/2026-07-10-e2e-lifecycle-stress-design.md).
//! Tests-only crate: never published, no product logic.
pub mod cli;
pub mod rig;
pub mod stub;
```

(`cli.rs` and `stub.rs` land in Tasks 6–7; create empty `pub mod` files here with a `//! placeholder` doc so the crate compiles, and fill them in their tasks.)

- [ ] **Step 3: rig.rs — full implementation**

```rust
use agent_runtime_config::RuntimeConfig;
use agent_server::session::Session;
use agent_server::testkit::Captured;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

pub struct Rig {
    pub workspace: TempDir,
    pub sessions: TempDir,
    pub meta: TempDir,
    pub key: [u8; 32],
}

impl Rig {
    pub fn new() -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let meta = tempfile::tempdir().unwrap();
        // Pre-create the secret: kills load_or_create_secret's first-touch race
        // and gives tests the key for (test-local) MAC forging.
        let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(3));
        let secret = meta.path().join("secret");
        std::fs::write(&secret, key).unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        Rig { workspace, sessions, meta, key }
    }

    pub fn runtime_config(&self, base_url: &str) -> RuntimeConfig {
        let mut c = RuntimeConfig::from_launch(
            "openai".into(), base_url.into(), "stub-model".into(), "native".into(), 32_768,
        );
        c.trace_dir = Some(self.sessions.path().to_string_lossy().into_owned());
        c.metadata_dir = Some(self.meta.path().to_string_lossy().into_owned());
        c
    }

    /// The GUI leg: a Session constructed exactly the way src-tauri's bridge
    /// does, pointed at this rig's roots, with a capturing sink attached.
    pub fn session(&self, base_url: &str) -> (Arc<Session>, Arc<Captured>) {
        let cfg_path = self.meta.path().join("agent-runtime.json");
        // File overlay carries the same overrides so an overlay can't undo them.
        std::fs::write(
            &cfg_path,
            serde_json::json!({
                "trace_dir": self.sessions.path().to_string_lossy(),
                "metadata_dir": self.meta.path().to_string_lossy(),
            })
            .to_string(),
        )
        .unwrap();
        let mut params = agent_server::setup::local_params(
            self.workspace.path().to_path_buf(),
            cfg_path,
            base_url.into(),
            "stub-model".into(),
        );
        params.config = self.runtime_config(base_url);
        let session = Session::from_params(params);
        let cap = Arc::new(Captured::default());
        session.set_event_out(cap.clone());
        (session, cap)
    }

    pub fn session_dirs(&self) -> Vec<PathBuf> {
        let mut v: Vec<_> = std::fs::read_dir(self.sessions.path())
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default();
        v.sort();
        v
    }

    pub fn only_session_dir(&self) -> PathBuf {
        let dirs = self.session_dirs();
        assert_eq!(dirs.len(), 1, "expected exactly one session dir, got {dirs:?}");
        dirs.into_iter().next().unwrap()
    }

    pub fn assert_parked(&self, dir: &Path) {
        assert!(
            ckpt(dir).join("parked.json").exists(),
            "positive-artifact rule: parked.json missing in {}", ckpt(dir).display()
        );
    }
}

/// Root-ask checkpoint dir: ALL park artifacts (parked.json, manifest.json,
/// answer.json, resume.lock) live here, never on the session dir itself (F2).
pub fn ckpt(session_dir: &Path) -> PathBuf {
    session_dir.join("checkpoint")
}

/// Bounded poll — the only generic waiter (spec §2.4 watchdog policy).
pub fn wait_until(cap: Duration, poll: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < cap {
        if poll() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}
```

**Async note:** in tokio tests call `wait_until` via `tokio::task::spawn_blocking`, or use the async twin below (include both):

```rust
pub async fn wait_until_async(cap: Duration, poll: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < cap {
        if poll() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}
```

- [ ] **Step 4: Smoke test** (in `rig.rs` `#[cfg(test)]`)

```rust
#[tokio::test]
async fn rig_creates_isolated_roots_and_secret() {
    let rig = Rig::new();
    assert_eq!(std::fs::read(rig.meta.path().join("secret")).unwrap().len(), 32);
    let cfg = rig.runtime_config("http://127.0.0.1:1");
    assert_eq!(
        agent_runtime_config::metadata_root_for(&cfg).unwrap(),
        rig.meta.path()
    );
    let (session, _cap) = rig.session("http://127.0.0.1:1");
    // Constructing a Session must have created its descriptor under the rig,
    // not under ~/.rusty-agent.
    assert!(!rig.session_dirs().is_empty());
    drop(session);
}
```

- [ ] **Step 5: Run + commit**

Run: `cd agent && cargo test -p agent-e2e`
Expected: PASS.

```bash
git add -A && git commit -m "feat(e2e): agent-e2e crate scaffold + isolation Rig (tempdir roots, pre-created secret)"
```

---

### Task 6: ScriptedStub (wiremock) + RawDropStub

**Files:**
- Create/replace: `agent/crates/agent-e2e/src/stub.rs`

**Interfaces:**
- Produces:
  - `pub enum StubResponse { ToolCall { name: String, args: serde_json::Value }, Text(String), MalformedJson, BogusTool, DelayedText { text: String, delay_ms: u64 } }`
  - `pub struct ScriptStep { pub expect_substring: Option<String>, pub respond: StubResponse }`
  - `pub struct ScriptedStub`; `ScriptedStub::start(steps: Vec<ScriptStep>) -> ScriptedStub` (async); `base_url(&self) -> String`; `recorded(&self) -> Vec<String>` (request bodies, arrival order); `assert_consumed(&self)` (panics if steps remain or a stray/mismatched request arrived).
  - `pub struct RawDropStub`; `RawDropStub::start() -> RawDropStub` (async): first request gets partial SSE then TCP close; **every subsequent** request gets a canned good text SSE (`"recovered"`); `base_url(&self) -> String`.

- [ ] **Step 1: Implement `stub.rs`**

```rust
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub enum StubResponse {
    ToolCall { name: String, args: serde_json::Value },
    Text(String),
    MalformedJson,
    BogusTool,
    DelayedText { text: String, delay_ms: u64 },
}

pub struct ScriptStep {
    /// Must appear in the raw request body (e.g. the task text, or deny feedback).
    pub expect_substring: Option<String>,
    pub respond: StubResponse,
}

#[derive(Default)]
struct StubState {
    cursor: usize,
    recorded: Vec<String>,
    /// First protocol violation (stray request / matcher miss); poisons the test.
    poison: Option<String>,
}

pub struct ScriptedStub {
    server: MockServer,
    steps_len: usize,
    state: Arc<Mutex<StubState>>,
}

fn sse_text(text: &str) -> String {
    let chunk = serde_json::json!({"choices":[{"delta":{"content": text}}]});
    format!(
        "data: {chunk}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
    )
}

fn sse_tool_call(name: &str, args: &serde_json::Value) -> String {
    // One-shot tool_call delta then finish_reason=tool_calls — mirror the shape
    // agent-model's own wiremock tests use for tool deltas (copy the exact JSON
    // key layout from agent-model/src/openai.rs's tool-call streaming test —
    // index/id/function.name/function.arguments — before finalizing).
    let call = serde_json::json!({"choices":[{"delta":{"tool_calls":[{
        "index":0,"id":"call_e2e_1","type":"function",
        "function":{"name":name,"arguments":args.to_string()}
    }]}}]});
    format!(
        "data: {call}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n"
    )
}

struct ScriptResponder {
    steps: Vec<(Option<String>, StubResponse)>,
    state: Arc<Mutex<StubState>>,
}

impl Respond for ScriptResponder {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&req.body).into_owned();
        let mut st = self.state.lock().unwrap();
        st.recorded.push(body.clone());
        let i = st.cursor;
        let Some((expect, resp)) = self.steps.get(i) else {
            st.poison = Some(format!("stray request past script end: {body:.200}"));
            return ResponseTemplate::new(500).set_body_string("E2E-STUB-STRAY");
        };
        if let Some(needle) = expect {
            if !body.contains(needle.as_str()) {
                st.poison = Some(format!("step {i}: body missing {needle:?}"));
                return ResponseTemplate::new(500).set_body_string("E2E-STUB-MISMATCH");
            }
        }
        st.cursor += 1;
        let sse = |b: String| {
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(b)
        };
        match resp {
            StubResponse::Text(t) => sse(sse_text(t)),
            StubResponse::ToolCall { name, args } => sse(sse_tool_call(name, args)),
            StubResponse::BogusTool => sse(sse_tool_call("no_such_tool_e2e", &serde_json::json!({}))),
            StubResponse::MalformedJson => sse("data: {not json}\n\ndata: [DONE]\n\n".into()),
            StubResponse::DelayedText { text, delay_ms } => {
                sse(sse_text(text)).set_delay(std::time::Duration::from_millis(*delay_ms))
            }
        }
    }
}

impl ScriptedStub {
    pub async fn start(steps: Vec<ScriptStep>) -> Self {
        let steps_len = steps.len();
        let state = Arc::new(Mutex::new(StubState::default()));
        let responder = ScriptResponder {
            steps: steps.into_iter().map(|s| (s.expect_substring, s.respond)).collect(),
            state: state.clone(),
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(responder)
            .mount(&server)
            .await;
        ScriptedStub { server, steps_len, state }
    }

    pub fn base_url(&self) -> String {
        self.server.uri()
    }

    pub fn recorded(&self) -> Vec<String> {
        self.state.lock().unwrap().recorded.clone()
    }

    /// Call at the end of every test that used the stub.
    pub fn assert_consumed(&self) {
        let st = self.state.lock().unwrap();
        assert!(st.poison.is_none(), "stub poisoned: {}", st.poison.as_deref().unwrap());
        assert_eq!(st.cursor, self.steps_len, "script not fully consumed");
    }
}
```

**Convenience constructors** (same file) used all over the scenario tasks:

```rust
/// The standard approval-gated step: write_file is Access::Write ⇒ Ask.
pub fn gated_write(expect: &str) -> ScriptStep {
    ScriptStep {
        expect_substring: Some(expect.into()),
        respond: StubResponse::ToolCall {
            name: "write_file".into(),
            args: serde_json::json!({"path": "out.txt", "content": "e2e"}),
        },
    }
}

pub fn text_step(expect: Option<&str>, reply: &str) -> ScriptStep {
    ScriptStep {
        expect_substring: expect.map(Into::into),
        respond: StubResponse::Text(reply.into()),
    }
}
```

- [ ] **Step 2: RawDropStub** (same file)

```rust
pub struct RawDropStub {
    addr: std::net::SocketAddr,
}

impl RawDropStub {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut first = true;
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let drop_this = std::mem::replace(&mut first, false);
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 65536];
                    let _ = sock.read(&mut buf).await; // consume request head
                    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
                    let _ = sock.write_all(head.as_bytes()).await;
                    if drop_this {
                        // one partial chunk, then hard close mid-stream
                        let _ = sock
                            .write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"par\"}}]}\n\n")
                            .await;
                        let _ = sock.shutdown().await;
                    } else {
                        let _ = sock.write_all(sse_text("recovered").as_bytes()).await;
                        let _ = sock.shutdown().await;
                    }
                });
            }
        });
        RawDropStub { addr }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }
}
```

(HTTP/1.1 without content-length + `connection: close` means EOF-terminated body; if the runtime's reqwest client rejects the happy-path response, adjust the non-drop arm to proxy to a `ScriptedStub` instead — decide at implementation, keep the drop arm raw either way.)

- [ ] **Step 3: Self-test** (`#[cfg(test)]` in stub.rs): script `[text_step(Some("ping"), "pong")]`, POST to it with `reqwest`... **No** — avoid adding reqwest; test through the real model client instead:

```rust
#[tokio::test]
async fn scripted_stub_matches_and_records() {
    let stub = ScriptedStub::start(vec![text_step(Some("ping"), "pong")]).await;
    // Drive through a Rig session so the real OpenAiCompatClient parses our SSE.
    let rig = crate::rig::Rig::new();
    let (session, cap) = rig.session(&stub.base_url());
    assert!(matches!(session.send_input("ping".into()), agent_server::session::SendOutcome::Started));
    assert!(
        crate::rig::wait_until_async(std::time::Duration::from_secs(30), || {
            cap.snapshot().iter().any(|e| matches!(e, agent_server::wire::ServerEvent::Done { .. }))
        })
        .await
    );
    assert!(stub.recorded()[0].contains("ping"));
    stub.assert_consumed();
}
```

- [ ] **Step 4: Run + commit**

Run: `cd agent && cargo test -p agent-e2e`
Expected: PASS — this is also the first proof the whole SSE format parses. If `sse_tool_call`'s JSON shape is wrong, this is where it surfaces; fix against agent-model's tool-call test shape.

```bash
git add -A && git commit -m "feat(e2e): scripted wiremock model stub (strict matching, poison-on-stray) + raw mid-stream-drop stub"
```

---

### Task 7: CLI driver

**Files:**
- Create/replace: `agent/crates/agent-e2e/src/cli.rs`

**Interfaces:**
- Produces:
  - `pub fn agent_bin() -> PathBuf` — resolves a **fresh** `agent` binary: `OnceLock`, runs `cargo build -p agent-cli --quiet` in the `agent/` workspace once per test process, then returns `<workspace>/target/debug/agent`. Never a bare path lookup without the build.
  - `pub struct CliCmd` builder: `CliCmd::new(rig: &Rig, base_url: &str) -> Self`, `.arg(s)`, `.args([..])`, `.sessions_sub(["sessions","list"])` (subcommand form), `.spawn() -> Cli`
  - `pub struct Cli` (running child): `write_line(&mut self, &str)`, `wait_for_output(&mut self, needle: &str, cap: Duration) -> String` (combined stdout+stderr transcript so far; panics on deadline with full transcript), `close_stdin(&mut self)`, `sigint(&self)`, `sigkill(&self)`, `wait_exit(&mut self, cap: Duration) -> std::process::ExitStatus` (deadline-bounded; SIGKILLs group + panics on expiry), `transcript(&self) -> String`. Drop = SIGKILL the process group.

- [ ] **Step 1: Implement `cli.rs`**

```rust
use crate::rig::Rig;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

static AGENT_BIN: OnceLock<PathBuf> = OnceLock::new();

/// Freshness rule (spec §2.2 item 5): build once per test process, then use.
pub fn agent_bin() -> PathBuf {
    AGENT_BIN
        .get_or_init(|| {
            let ws = workspace_root();
            let status = Command::new("cargo")
                .args(["build", "-p", "agent-cli", "--quiet"])
                .current_dir(&ws)
                .status()
                .expect("cargo build -p agent-cli");
            assert!(status.success(), "agent-cli build failed");
            target_dir(&ws).join("debug/agent")
        })
        .clone()
}

/// Honor CARGO_TARGET_DIR when set (plan-review F7).
fn target_dir(ws: &std::path::Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| ws.join("target"))
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = .../agent/crates/agent-e2e → workspace = two up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

pub struct CliCmd {
    cmd: Command,
}

impl CliCmd {
    pub fn new(rig: &Rig, base_url: &str) -> Self {
        let mut cmd = Command::new(agent_bin());
        cmd.args([
            "--base-url", base_url,
            "--model", "stub-model",
            "--workspace", rig.workspace.path().to_str().unwrap(),
            "--trace-dir", rig.sessions.path().to_str().unwrap(),
            "--metadata-dir", rig.meta.path().to_str().unwrap(),
            "--stream-timeout-secs", "10",
            "--approval-timeout-secs", "20",
        ]);
        cmd.env("HOME", rig.meta.path()) // belt-and-braces (spec §2.3)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0); // own group → group-kill can't touch the test
        CliCmd { cmd }
    }

    /// Subcommand form, e.g. `sessions_sub(&["sessions","reopen","<id>"])` —
    /// clap subcommands must precede/follow flags exactly as the CLI defines;
    /// verify flag-vs-subcommand ordering against `agent --help` when first used.
    pub fn sessions_sub(mut self, sub: &[&str]) -> Self {
        self.cmd.args(sub);
        self
    }

    pub fn arg(mut self, a: &str) -> Self {
        self.cmd.arg(a);
        self
    }

    pub fn spawn(mut self) -> Cli {
        let mut child = self.cmd.spawn().expect("spawn agent");
        let stdin = child.stdin.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        // F1: prompts are print!-ed with NO trailing newline — read raw byte
        // chunks, never BufReader::lines() (a line reader never yields the
        // approval/feedback/REPL prompt and every waiter deadlines).
        fn pump(mut r: impl std::io::Read + Send + 'static, tx: std::sync::mpsc::Sender<String>) {
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match r.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if tx.send(String::from_utf8_lossy(&buf[..n]).into_owned()).is_err() {
                                return;
                            }
                        }
                    }
                }
            });
        }
        pump(child.stdout.take().unwrap(), tx.clone());
        pump(child.stderr.take().unwrap(), tx);
        Cli { child, stdin: Some(stdin), rx, transcript: String::new() }
    }
}

pub struct Cli {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<String>,
    transcript: String,
}

impl Cli {
    pub fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    pub fn write_line(&mut self, s: &str) {
        let stdin = self.stdin.as_mut().expect("stdin already closed");
        writeln!(stdin, "{s}").unwrap();
        stdin.flush().unwrap();
    }

    pub fn close_stdin(&mut self) {
        self.stdin.take();
    }

    fn drain(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(chunk) => self.transcript.push_str(&chunk), // raw chunks (F1)
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return,
            }
        }
    }

    pub fn transcript(&mut self) -> String {
        self.drain();
        self.transcript.clone()
    }

    pub fn wait_for_output(&mut self, needle: &str, cap: Duration) -> String {
        let start = Instant::now();
        loop {
            self.drain();
            if self.transcript.contains(needle) {
                return self.transcript.clone();
            }
            assert!(
                start.elapsed() < cap,
                "deadline waiting for {needle:?}; transcript so far:\n{}",
                self.transcript
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn sigint(&self) {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(self.pid()),
            nix::sys::signal::Signal::SIGINT,
        )
        .unwrap();
    }

    pub fn sigkill(&self) {
        // group kill: negative pid (we set process_group(0) at spawn)
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(-self.pid()),
            nix::sys::signal::Signal::SIGKILL,
        );
    }

    pub fn wait_exit(&mut self, cap: Duration) -> std::process::ExitStatus {
        let start = Instant::now();
        loop {
            if let Some(st) = self.child.try_wait().unwrap() {
                self.drain();
                return st;
            }
            if start.elapsed() >= cap {
                self.sigkill();
                panic!("deadline waiting for exit; transcript:\n{}", self.transcript());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        self.sigkill(); // KillOnDrop: held-Child group kill, never pattern-match
        let _ = self.child.wait();
    }
}
```

- [ ] **Step 2: Smoke test** (`#[cfg(test)]` in cli.rs)

```rust
#[test]
fn sessions_list_on_empty_root_exits_clean() {
    let rig = Rig::new();
    let mut cli = CliCmd::new(&rig, "http://127.0.0.1:1")
        .sessions_sub(&["sessions", "list"])
        .spawn();
    let st = cli.wait_exit(Duration::from_secs(30));
    assert!(st.success(), "transcript:\n{}", cli.transcript());
}
```

(If clap rejects global flags before the subcommand, fix `CliCmd::new`'s arg ordering here — this smoke test exists to settle that once.)

- [ ] **Step 3: Run + commit**

Run: `cd agent && cargo test -p agent-e2e`
Expected: PASS.

```bash
git add -A && git commit -m "feat(e2e): CLI subprocess driver (fresh-binary rule, group KillOnDrop, bounded waiters)"
```

---

### Task 8: e2e-daemon harness binary

**Files:**
- Create: `agent/crates/agent-e2e/src/bin/e2e-daemon.rs`

**Interfaces:**
- Produces a binary with two modes, driven by the same `Rig`-style flags:
  - `e2e-daemon run --workspace W --sessions S --meta M --base-url U [--task TEXT]` — constructs a `Session` exactly like `Rig::session`, attaches a **stdout-JSON sink** (each `ServerEvent` printed as one `serde_json::to_string` line prefixed `EV `), optionally sends `--task`, then serves stdin commands until EOF: `input <text>`, `approve <id>`, `deny <id> [feedback...]`, `always <id>`, `cancel`. Prints `READY` once subscribed.
  - `e2e-daemon hold-lock --dir <session_dir>` — calls `agent_core::checkpoint::claim_resume(dir)`, prints `LOCKED` on success (exit 3 if contended), then sleeps forever (kill target for scenario 11).
- Tests parse `EV `-prefixed lines back into `ServerEvent` (serde) — the compiler keeps this honest.

- [ ] **Step 1: Implement**

```rust
use agent_server::wire::{Decision, EventOut, ServerEvent};
use std::io::BufRead;
use std::sync::Arc;

struct StdoutSink;
impl EventOut for StdoutSink {
    fn send(&self, ev: ServerEvent) {
        println!("EV {}", serde_json::to_string(&ev).unwrap());
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).map(|i| args[i + 1].clone())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hold-lock") => {
            let dir = std::path::PathBuf::from(flag(&args, "--dir").expect("--dir"));
            match agent_core::checkpoint::claim_resume(&dir) {
                Ok(true) => {
                    println!("LOCKED");
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                    }
                }
                _ => std::process::exit(3),
            }
        }
        Some("run") => run_mode(&args).await,
        other => {
            eprintln!("unknown mode {other:?}");
            std::process::exit(2);
        }
    }
}

async fn run_mode(args: &[String]) {
    let ws = flag(args, "--workspace").expect("--workspace");
    let sessions = flag(args, "--sessions").expect("--sessions");
    let meta = flag(args, "--meta").expect("--meta");
    let base_url = flag(args, "--base-url").expect("--base-url");

    let cfg_path = std::path::PathBuf::from(&meta).join("agent-runtime.json");
    std::fs::write(
        &cfg_path,
        serde_json::json!({"trace_dir": sessions, "metadata_dir": meta}).to_string(),
    )
    .unwrap();
    let mut params = agent_server::setup::local_params(
        ws.clone().into(), cfg_path, base_url.clone(), "stub-model".into(),
    );
    params.config.trace_dir = Some(sessions);
    params.config.metadata_dir = Some(meta);
    let session = agent_server::session::Session::from_params(params);
    session.set_event_out(Arc::new(StdoutSink));
    println!("READY");
    if let Some(task) = flag(args, "--task") {
        let _ = session.send_input(task);
    }
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let mut it = line.splitn(3, ' ');
        match (it.next(), it.next(), it.next()) {
            (Some("input"), Some(rest), tail) => {
                let text = match tail { Some(t) => format!("{rest} {t}"), None => rest.into() };
                let _ = session.send_input(text);
            }
            (Some("approve"), Some(id), _) => session.approve(id, Decision::Approve),
            (Some("always"), Some(id), _) => session.approve(id, Decision::ApproveAlways),
            (Some("deny"), Some(id), fb) => session.approve(
                id, Decision::Deny { feedback: fb.map(str::to_string) },
            ),
            (Some("cancel"), _, _) => session.cancel(),
            _ => eprintln!("bad cmd: {line}"),
        }
    }
    // stdin EOF: park-and-stay — sleep so the test controls lifetime via kill.
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}
```

(If `Session::from_params` needs the tokio reactor for its subscribe path, `#[tokio::main]` provides it. If `claim_resume` is not `pub` from `agent_core::checkpoint`, re-export it or call through the public path the CLI uses — check `agent-cli/src/main.rs`'s import and copy it.)

- [ ] **Step 2: Add a driver-side helper** to `cli.rs` (same builder/waiter reuse):

```rust
pub fn e2e_daemon_bin() -> PathBuf {
    // Own crate's bin: cargo builds it for integration tests; still build once
    // for direct `cargo test -p agent-e2e` runs.
    option_env!("CARGO_BIN_EXE_e2e-daemon")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let ws = workspace_root();
            let status = Command::new("cargo")
                .args(["build", "-p", "agent-e2e", "--bin", "e2e-daemon", "--quiet"])
                .current_dir(&ws)
                .status()
                .expect("build e2e-daemon");
            assert!(status.success());
            target_dir(&ws).join("debug/e2e-daemon")
        })
}

pub struct DaemonCmd; // same shape as CliCmd but for e2e-daemon `run`
impl DaemonCmd {
    pub fn run(rig: &Rig, base_url: &str, task: Option<&str>) -> Cli {
        let mut cmd = Command::new(e2e_daemon_bin());
        cmd.args([
            "run",
            "--workspace", rig.workspace.path().to_str().unwrap(),
            "--sessions", rig.sessions.path().to_str().unwrap(),
            "--meta", rig.meta.path().to_str().unwrap(),
            "--base-url", base_url,
        ]);
        if let Some(t) = task {
            cmd.args(["--task", t]);
        }
        cmd.env("HOME", rig.meta.path())
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .process_group(0);
        CliCmd { cmd }.spawn()
    }

    pub fn hold_lock(rig: &Rig, dir: &std::path::Path) -> Cli {
        let mut cmd = Command::new(e2e_daemon_bin());
        cmd.args(["hold-lock", "--dir", dir.to_str().unwrap()]);
        cmd.env("HOME", rig.meta.path())
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .process_group(0);
        CliCmd { cmd }.spawn()
    }
}
```

Plus an event-line helper on `Cli`:

```rust
/// Wait for a ServerEvent (from `EV `-prefixed lines) matching `pred`.
pub fn wait_for_event(
    &mut self,
    cap: Duration,
    pred: impl Fn(&ServerEvent) -> bool,
) -> ServerEvent {
    let start = Instant::now();
    loop {
        self.drain();
        for line in self.transcript.lines() {
            if let Some(json) = line.strip_prefix("EV ") {
                if let Ok(ev) = serde_json::from_str::<ServerEvent>(json) {
                    if pred(&ev) {
                        return ev;
                    }
                }
            }
        }
        assert!(
            start.elapsed() < cap,
            "deadline waiting for event; transcript:\n{}", self.transcript
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
```

(`CliCmd { cmd }.spawn()` requires `CliCmd`'s field to be `pub(crate)`; adjust.)

- [ ] **Step 3: Smoke test**: `DaemonCmd::run` with a one-step text script; wait `READY`, wait `Done` event, `assert_consumed`, drop.

```rust
#[tokio::test]
async fn daemon_runs_one_turn_and_streams_events() {
    let stub = crate::stub::ScriptedStub::start(vec![crate::stub::text_step(Some("hi"), "yo")]).await;
    let rig = Rig::new();
    let mut d = DaemonCmd::run(&rig, &stub.base_url(), Some("hi"));
    d.wait_for_output("READY", Duration::from_secs(30));
    d.wait_for_event(Duration::from_secs(30), |e| matches!(e, ServerEvent::Done { .. }));
    stub.assert_consumed();
}
```

- [ ] **Step 4: Run + commit**

Run: `cd agent && cargo test -p agent-e2e`
Expected: PASS.

```bash
git add -A && git commit -m "feat(e2e): e2e-daemon harness bin (real-process Session host, hold-lock mode) + driver"
```

---

### Task 9: Scenarios 1–2 — cross-surface park/reopen, both directions

**Files:**
- Create: `agent/crates/agent-e2e/tests/lifecycle.rs`

**Interfaces:**
- Consumes: everything from Tasks 5–8. Test file preamble used by ALL scenario files (copy verbatim into each new test file):

```rust
use agent_e2e::cli::{Cli, CliCmd, DaemonCmd};
use agent_e2e::rig::{ckpt, wait_until, wait_until_async, Rig};
// NOTE: trim unused imports per file (clippy -D warnings); every #[tokio::test]
// below is written in full as #[tokio::test(flavor = "multi_thread")] (F3).
use agent_e2e::stub::{gated_write, text_step, ScriptStep, ScriptedStub, StubResponse};
use agent_server::wire::{Decision, ServerEvent};
use std::time::Duration;

const CAP: Duration = Duration::from_secs(30);
```

- [ ] **Step 1: Scenario 1 — park via Session (GUI leg) → CLI reopen → approve → completes**

```rust
#[tokio::test]
async fn s01_park_in_gui_reopen_in_cli() {
    // Script: task → gated tool call (parks); after approve → tool result goes
    // back → final text completes the run.
    let stub = ScriptedStub::start(vec![
        gated_write("SQRL-1 write the file"),
        text_step(None, "done"),
    ])
    .await;
    let rig = Rig::new();
    let (session, cap) = rig.session(&stub.base_url());
    assert!(matches!(
        session.send_input("SQRL-1 write the file".into()),
        agent_server::session::SendOutcome::Started
    ));
    // Ask surfaces + park lands on disk.
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::ApprovalRequest { .. })))
        .await
    );
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    rig.assert_parked(&dir);
    // "GUI closes": drop the Session mid-park.
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();

    // Reopen from the CLI, approve at the prompt.
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("y");
    let st = cli.wait_exit(CAP);
    assert!(st.success(), "transcript:\n{}", cli.transcript());
    stub.assert_consumed();
    // Completed tree is reaped (delete-on-completion) — dir gone or no park.
    assert!(!ckpt(&dir).join("parked.json").exists(), "park must be consumed");
}
```

**Note for the implementer:** if dropping a parked `Session` aborts the pending ask in a way that reaps the park, that is a FINDING (parks must survive process death — here process death is simulated by drop; scenario 12 does it with a real kill). Investigate before "fixing" the test.

- [ ] **Step 2: Scenario 2 — CLI timeout park-and-exit → Session attach sees parked_runs → approve → resumed → done**

```rust
#[tokio::test]
async fn s02_cli_timeout_park_then_gui_attach_resumes() {
    let stub = ScriptedStub::start(vec![
        gated_write("SQRL-2 write it"),
        text_step(None, "done"),
    ])
    .await;
    let rig = Rig::new();
    // 1s approval window → deterministic park-and-exit (E2 knob).
    let mut cli = CliCmd::new(&rig, &stub.base_url()).arg("--approval-timeout-secs=1").spawn();
    cli.wait_for_output("agent>", Duration::from_secs(10)); // adjust to the REPL's actual prompt marker; discover via Task 7 smoke transcript
    cli.write_line("SQRL-2 write it");
    cli.wait_for_output("run parked; answer later with", CAP);
    let st = cli.wait_exit(CAP);
    let dir = rig.only_session_dir();
    rig.assert_parked(&dir);
    drop(st);
    drop(cli);

    // "GUI opens": fresh Session over the same roots re-emits the park on attach.
    let (session, cap) = rig.session(&stub.base_url());
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::ParkedRuns { runs } if !runs.is_empty())))
        .await
    );
    let ask = wait_until_ask(&cap).await; // helper below
    session.approve(&ask, Decision::Approve);
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Resumed { .. })))
        .await
    );
    assert!(
        wait_until_async(CAP, || cap
            .snapshot()
            .iter()
            .any(|e| matches!(e, ServerEvent::Done { .. })))
        .await
    );
    stub.assert_consumed();
}

async fn wait_until_ask(cap: &agent_server::testkit::Captured) -> String {
    agent_server::testkit::wait_for_ask_id(cap, CAP).await
}
```

**REPL prompt marker:** the string `"agent>"` above is a guess — Step 1 of Task 7's smoke test shows the real interactive marker in the transcript; pin it there and reuse the constant everywhere (define `pub const REPL_MARKER: &str = ...` in `cli.rs`).

- [ ] **Step 3: Run**

Run: `cd agent && cargo test -p agent-e2e --test lifecycle`
Expected: PASS, both tests.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "test(e2e): scenarios 1-2 — cross-surface park/reopen in both directions"
```

---

### Task 10: Scenario 3 — deny-with-feedback e2e (+ hostile-content variant)

**Files:**
- Modify: `agent/crates/agent-e2e/tests/lifecycle.rs`

- [ ] **Step 1: Write both tests** (one parameterized helper, two `#[tokio::test]` wrappers)

```rust
async fn deny_feedback_roundtrip(feedback: &str) {
    let stub = ScriptedStub::start(vec![
        gated_write("SQRL-3 write it"),
        // After a deny, the tool message carrying the feedback goes back to the
        // model: the matcher REQUIRES the feedback text in the request body.
        ScriptStep {
            expect_substring: Some(feedback.into()),
            respond: StubResponse::ToolCall {
                name: "write_file".into(),
                args: serde_json::json!({"path": "out2.txt", "content": "retry"}),
            },
        },
        text_step(None, "done"),
    ])
    .await;
    let rig = Rig::new();
    // Park on the Session leg.
    let (session, cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-3 write it".into());
    let dir_ready = wait_until_async(CAP, || !rig.session_dirs().is_empty()).await;
    assert!(dir_ready);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();

    // Deny with feedback from the CLI (two stdin lines, pipe stays open).
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid])
        .spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("n");
    cli.wait_for_output("Feedback for the agent", CAP);
    cli.write_line(feedback);
    // Deny → model retries the gated call → run re-parks; the CLI re-prompts
    // in the same reopen (or parks-and-exits) — accept either by waiting for
    // the second prompt OR the park message. Characterize on first run and pin.
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("y");
    let st = cli.wait_exit(CAP);
    assert!(st.success(), "transcript:\n{}", cli.transcript());
    stub.assert_consumed();
    // The strong assertion: feedback text reached the model verbatim.
    assert!(stub.recorded().iter().any(|r| r.contains(feedback)));
}

#[tokio::test]
async fn s03_deny_feedback_travels_to_model() {
    deny_feedback_roundtrip("SQRL-FEEDBACK use path out2.txt instead").await;
}

#[tokio::test]
async fn s03b_deny_feedback_hostile_content() {
    // multibyte + control chars + JSON-meta + long tail (~10KB)
    let hostile = format!(
        "SQRL-HOSTILE 日本語 \u{1F980} quote\" backslash\\ brace}} tab\tnewline-free {}",
        "x".repeat(10_000)
    );
    deny_feedback_roundtrip(&hostile).await;
}
```

**Hostile-content caveat:** stdin lines cannot contain raw `\n` (the prompt is line-based) — that's why the hostile string is newline-free; JSON-escaping happens in the checkpoint/request layers, which is exactly what's under test. If tally desync bites here (deny → re-park), that's regression `2fad367` territory — investigate, don't loosen.

- [ ] **Step 2: Run + commit**

Run: `cd agent && cargo test -p agent-e2e --test lifecycle`
Expected: PASS (4 tests now).

```bash
git add -A && git commit -m "test(e2e): scenario 3 — deny feedback travels e2e, incl. hostile content"
```

---

### Task 11: Budget checkpoint (measure before writing the rest)

- [ ] **Step 1: Measure**

Run: `cd agent && cargo test -p agent-e2e -- --format terse 2>&1 | tail -5` and time it: `time cargo test -p agent-e2e`.

- [ ] **Step 2: Extrapolate and record.** ~6 tests exist (2 smoke + 4 scenarios). Tier 1 will have ~24. If `measured_total × 4 > 90s`, decide NOW which tests move behind `#[ignore]` + a `soak` name prefix (the §5#4 soak is the pre-committed first candidate). Record the measurement and the decision as a dated note in `docs/superpowers/plans/2026-07-10-e2e-lifecycle-stress.md` itself (append under this task) and in the commit message.

- [ ] **Step 3: Commit** (even if no split): `git commit -am "chore(e2e): budget checkpoint — <N>s measured, <decision>"`

---

### Task 12: Scenario 4 — the soak (deny/approve cycles, alternating surfaces, large-artifact cycle)

**Files:**
- Modify: `agent/crates/agent-e2e/tests/lifecycle.rs`

- [ ] **Step 1: Write the test.** N=4 cycles on ONE session: cycle i parks via a gated call; odd cycles deny-with-feedback (from CLI), even cycles approve (from a fresh Session attach); final cycle's task carries a large payload (ask the stub to emit a `write_file` whose `content` arg is ~2MB) so the checkpoint dumps a multi-MB artifact store. Script layout (build in a loop):

```rust
#[tokio::test]
async fn s04_soak_alternating_deny_approve_across_surfaces() {
    let big = "B".repeat(2_000_000);
    let mut steps = vec![gated_write("SQRL-SOAK start")];
    // cycle 1 deny → retry gated; cycle 2 approve → next gated (new turn) ...
    steps.push(ScriptStep { expect_substring: Some("SOAK-FB-1".into()),
        respond: StubResponse::ToolCall { name: "write_file".into(),
            args: serde_json::json!({"path":"a.txt","content":"x"}) } });
    steps.push(text_step(None, "cycle2 done"));
    steps.push(ScriptStep { expect_substring: Some("SQRL-SOAK turn3".into()),
        respond: StubResponse::ToolCall { name: "write_file".into(),
            args: serde_json::json!({"path":"big.txt","content": big}) } });
    steps.push(ScriptStep { expect_substring: Some("SOAK-FB-3".into()),
        respond: StubResponse::ToolCall { name: "write_file".into(),
            args: serde_json::json!({"path":"big2.txt","content":"y"}) } });
    steps.push(text_step(None, "cycle4 done"));
    let stub = ScriptedStub::start(steps).await;
    let rig = Rig::new();

    // Cycle 1: park on Session, deny w/ feedback via CLI (re-park), approve via CLI.
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-SOAK start".into());
    let dir = {
        assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
        let d = rig.only_session_dir();
        assert!(wait_until_async(CAP, || ckpt(d).join("parked.json").exists()).await);
        d
    };
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid]).spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("n");
    cli.wait_for_output("Feedback for the agent", CAP);
    cli.write_line("SOAK-FB-1");
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP); // re-parked, re-prompted
    cli.write_line("y");
    assert!(cli.wait_exit(CAP).success());
    drop(cli);

    // Cycle 3+4: new turn on a fresh Session (same roots), park, deny (Session),
    // re-park, approve from CLI reopen.
    let (session, cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-SOAK turn3".into());
    let ask = agent_server::testkit::wait_for_ask_id(&cap, CAP).await;
    // parked.json must exist before we act (positive-artifact rule) — the
    // session dir may be a NEW dir for the new Session; rediscover.
    let dirs = rig.session_dirs();
    assert!(wait_until_async(CAP, || dirs.iter().any(|d| ckpt(d).join("parked.json").exists())).await);
    session.approve(&ask, Decision::Deny { feedback: Some("SOAK-FB-3".into()) });
    let ask2 = agent_server::testkit::wait_for_ask_id(&cap, CAP).await; // re-park
    session.approve(&ask2, Decision::Approve);
    assert!(wait_until_async(CAP, || cap.snapshot().iter()
        .any(|e| matches!(e, ServerEvent::Done { .. }))).await);
    stub.assert_consumed();
    // Tally monotonicity / no drift: the surviving session dirs verify clean —
    // `sessions list` exits 0 and shows no corruption complaints.
    let mut list = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "list"]).spawn();
    let st = list.wait_exit(CAP);
    assert!(st.success());
    let t = list.transcript();
    assert!(!t.contains("corrupt"), "list transcript:\n{t}");
}
```

**Implementation honesty note:** `wait_for_ask_id` returns the FIRST ask id in the capture; after a deny the second ask needs a "newest ask" variant — add `wait_for_ask_id_after(cap, &prev_id, CAP)` to `testkit` if needed (skip-past semantics), don't loosen to sleeps.

- [ ] **Step 2: Run + commit**

Run: `cd agent && cargo test -p agent-e2e --test lifecycle s04 -- --nocapture`
Expected: PASS; note the runtime (soak is the budget canary).

```bash
git add -A && git commit -m "test(e2e): scenario 4 — cross-surface deny/approve soak with large-artifact cycle (tally-floor regression guard)"
```

---

### Task 13: Scenarios 5–6 — [real-kill] cancel-mid-resume and committed-answer durability

**Files:**
- Create: `agent/crates/agent-e2e/tests/crashkill.rs` (same preamble as lifecycle.rs)

- [ ] **Step 1: Scenario 5 — SIGINT during CLI reopen resume → park retained**

```rust
#[tokio::test]
async fn s05_sigint_mid_resume_retains_park() {
    let stub = ScriptedStub::start(vec![
        gated_write("SQRL-5 go"),
        // resume's model request stalls 20s so SIGINT lands mid-resume
        ScriptStep { expect_substring: None,
            respond: StubResponse::DelayedText { text: "slow".into(), delay_ms: 20_000 } },
        // after the second reopen approves again, complete fast
        text_step(None, "done"),
    ]).await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-5 go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();

    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid]).spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("y");
    // Synchronize on the resume actually being in flight: the stub records
    // the (delayed) resume request.
    assert!(wait_until(CAP, || stub.recorded().len() >= 2));
    cli.sigint();
    let _ = cli.wait_exit(CAP);
    // "Ok ≠ completed" guard: the park must still be there and the lock released.
    rig.assert_parked(&dir);
    assert!(wait_until(CAP, || !ckpt(&dir).join("resume.lock").exists()),
        "resume.lock must be released on cancel");
    // NOTE: the answer was consumed pre-resume, so a re-park is expected to
    // re-prompt on the next reopen. Approve and finish to prove recoverability.
    let mut cli2 = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid]).spawn();
    cli2.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli2.write_line("y");
    assert!(cli2.wait_exit(CAP).success(), "{}", cli2.transcript());
}
```

**Characterize before pinning:** whether SIGINT mid-resume re-parks at the ORIGINAL ask or a new one, and whether stub step 2 re-runs (add a spare identical step if the resume retries — adjust the script then pin). If the first reopen's SIGINT kills the delayed-response wait without the CLI writing a retained park, that's regression `76d81d5` resurfacing — a finding, not a test bug.

- [ ] **Step 2: Scenario 6 — committed answer survives death, consumed exactly once**

Two halves:

```rust
#[tokio::test]
async fn s06a_reopen_killed_while_parked_leaves_park_reopenable() {
    let stub = ScriptedStub::start(vec![gated_write("SQRL-6 go"), text_step(None, "done")]).await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-6 go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();
    // Real kill: reopen reaches the prompt (lock held), SIGKILL — no answer.
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid]).spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.sigkill();
    let _ = cli.wait_exit(CAP);
    rig.assert_parked(&dir);
    // SIGKILL leaves resume.lock — documented stale-lock behavior; clear it the
    // way the product's own error message instructs, then prove reopenability.
    // PRODUCT-GAP: no auto-recovery for stale locks (spec scenario 11 asserts
    // the contention message; here we just clear and move on).
    std::fs::remove_file(ckpt(&dir).join("resume.lock")).unwrap();
    let mut cli2 = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid]).spawn();
    cli2.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli2.write_line("y");
    assert!(cli2.wait_exit(CAP).success(), "{}", cli2.transcript());
    stub.assert_consumed();
}

#[tokio::test]
async fn s06b_committed_answer_consumed_without_reprompt() {
    // The write_answer→take_answer window is sub-millisecond and CPU-bound —
    // unhittable from outside (spec §2.4 descope: the state is produced by the
    // REAL writer, agent_core::checkpoint::write_answer, standing in for
    // "killed between commit and consume"). RECORD this descope in the spec's
    // Panel & review log for owner sign-off at branch review.
    let stub = ScriptedStub::start(vec![gated_write("SQRL-6B go"), text_step(None, "done")]).await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-6B go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    // Commit an approve answer with the real writer + the rig's real key.
    // (Exact fn signature: see agent-core/src/checkpoint.rs `write_answer` /
    // `ParkedAnswer` — bind to the parked manifest the way the CLI does.)
    agent_e2e::forge::commit_answer_like_cli(&ckpt(&dir), &rig.key, /*approve=*/ true, None);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .sessions_sub(&["sessions", "reopen", &sid]).spawn();
    let st = cli.wait_exit(CAP);
    let t = cli.transcript();
    assert!(st.success(), "{t}");
    assert!(!t.contains("[y]es / [n]o"), "must NOT re-prompt — answer was committed:\n{t}");
    stub.assert_consumed();
    assert!(!ckpt(&dir).join("answer.json").exists(), "answer must be consumed");
}
```

Add `agent-e2e/src/forge.rs` (new module, `pub mod forge;` in lib.rs) with `commit_answer_like_cli(dir, key, approve, feedback)` implemented by calling the SAME public agent-core functions the CLI's answer-commit path uses (locate by content in `agent-cli/src/main.rs`: the reopen flow's `commit_answer`/`write_answer` call; re-use, do not reimplement MAC math). Forging/tampering helpers ALWAYS take `key: &[u8; 32]` explicitly (spec §2.2 item 4).

- [ ] **Step 3: Run + commit**

Run: `cd agent && cargo test -p agent-e2e --test crashkill`
Expected: PASS (3 tests).

```bash
git add -A && git commit -m "test(e2e): scenarios 5-6 — SIGINT mid-resume retains park; committed answer consumed once (real-kill + sanctioned descope)"
```

**Also:** append the s06b descope note to the spec's Panel & review log ("§2.4 descope: scenario 6 commit-window state produced by real writer; window named: write_answer→take_answer in reopen") and include it in this commit.

---

### Task 14: Scenarios 7–9 — divergence, ApproveAlways downgrade, input-while-parked

**Files:**
- Modify: `agent/crates/agent-e2e/tests/lifecycle.rs`

- [ ] **Step 1: Scenario 7 — protocol divergence on reopen.** Park under native (default), reopen with `--protocol prompted`; assert the resume completes and (characterize first!) the second stub request is prompted-encoded. Minimum stable assertion: reopen exits 0, script consumed, and the divergence is visible in `stub.recorded()[1]` (prompted protocol puts tool instructions in prose — pin ONE stable marker string after observing a real run; e.g. the prompted-protocol preamble). Mark with `// PRODUCT-GAP: reopen accepts protocol-divergent flags without validation (spec §5#7)`.

```rust
#[tokio::test]
async fn s07_reopen_with_divergent_protocol_is_defined() {
    let stub = ScriptedStub::start(vec![
        gated_write("SQRL-7 go"),
        text_step(None, "done"),
    ]).await;
    let rig = Rig::new();
    let (session, _cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-7 go".into());
    assert!(wait_until_async(CAP, || !rig.session_dirs().is_empty()).await);
    let dir = rig.only_session_dir();
    assert!(wait_until_async(CAP, || ckpt(&dir).join("parked.json").exists()).await);
    drop(session);
    let sid = dir.file_name().unwrap().to_string_lossy().into_owned();
    let mut cli = CliCmd::new(&rig, &stub.base_url())
        .arg("--protocol").arg("prompted")
        .sessions_sub(&["sessions", "reopen", &sid]).spawn();
    cli.wait_for_output("[y]es / [n]o / [a]lways:", CAP);
    cli.write_line("y");
    // PRODUCT-GAP: divergence is accepted silently; this pins "no crash, run
    // completes" as the current contract. If completion is impossible under
    // prompted (stub replies native-shaped), pin the actual clean-error
    // behavior instead — characterize, then assert.
    let st = cli.wait_exit(CAP);
    assert!(st.success() || cli.transcript().contains("error"),
        "must complete or fail CLEANLY, transcript:\n{}", cli.transcript());
}
```

(`CliCmd.arg` takes one string — the two-call form above needs `arg()` to be called twice or accept the `--protocol=prompted` form; use `.arg("--protocol=prompted")`.)

- [ ] **Step 2: Scenario 8 — ApproveAlways downgrade across restart**

```rust
#[tokio::test]
async fn s08_approve_always_does_not_survive_restart() {
    let stub = ScriptedStub::start(vec![
        gated_write("SQRL-8 first"),
        text_step(None, "done1"),
        // post-restart: the SAME command must ask again
        gated_write("SQRL-8 second"),
        text_step(None, "done2"),
    ]).await;
    let rig = Rig::new();
    let (session, cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-8 first".into());
    let ask = agent_server::testkit::wait_for_ask_id(&cap, CAP).await;
    session.approve(&ask, Decision::ApproveAlways);
    assert!(wait_until_async(CAP, || cap.snapshot().iter()
        .any(|e| matches!(e, ServerEvent::Done { .. }))).await);
    drop(session); // restart

    let (session2, cap2) = rig.session(&stub.base_url());
    session2.send_input("SQRL-8 second".into());
    // SECURITY-RELEVANT: an identical gated call post-restart must PARK again.
    let ask2 = agent_server::testkit::wait_for_ask_id(&cap2, CAP).await;
    session2.approve(&ask2, Decision::Approve);
    assert!(wait_until_async(CAP, || cap2.snapshot().iter()
        .any(|e| matches!(e, ServerEvent::Done { .. }))).await);
    stub.assert_consumed();
}
```

- [ ] **Step 3: Scenario 9 — new input on a session holding a park.** Plant a parked prior session (testkit `plant_parked_session` against the rig roots — this is why Task 4 parameterized it), then run a NEW full turn on a fresh Session over the same roots, then assert the planted park is intact and reopenable.

```rust
#[tokio::test]
async fn s09_new_turn_does_not_clobber_existing_park() {
    let rig = Rig::new();
    let (planted_sess, _cap, _key) = agent_server::testkit::plant_parked_session(
        rig.workspace.path(), rig.sessions.path(), rig.meta.path(), "0-e2eprior").await;
    drop(planted_sess);
    let parked_dir = rig.session_dirs().into_iter()
        .find(|d| ckpt(d).join("parked.json").exists()).expect("planted park");

    let stub = ScriptedStub::start(vec![text_step(Some("SQRL-9 new task"), "ok")]).await;
    let (session, cap) = rig.session(&stub.base_url());
    session.send_input("SQRL-9 new task".into());
    assert!(wait_until_async(CAP, || cap.snapshot().iter()
        .any(|e| matches!(e, ServerEvent::Done { .. }))).await);
    stub.assert_consumed();
    // The park must survive an unrelated completed turn (spec §5#9). If this
    // fails, it is a PRODUCT FINDING (banner-ignoring input clobbers parks) —
    // escalate, do not weaken.
    rig.assert_parked(&parked_dir);
}
```

(Adapt `plant_parked_session`'s exact signature to what Task 4 produced — it may need the rig's config for metadata_dir; the string `"0-e2eprior"` is the prior session id, epoch-prefixed shape.)

- [ ] **Step 4: Run + commit**

Run: `cd agent && cargo test -p agent-e2e --test lifecycle`

```bash
git add -A && git commit -m "test(e2e): scenarios 7-9 — protocol divergence, ApproveAlways downgrade, park survives new input"
```

---

### Task 15: Scenarios 10–13 — torn checkpoint, stale lock, real daemon restart, park-write failure

**Files:**
- Modify: `agent/crates/agent-e2e/tests/crashkill.rs`

- [ ] **Step 1: Scenario 10 — torn checkpoint refused, list unaffected.** Park for real (Session leg), then `std::fs::remove_file(ckpt(&dir).join("manifest.json"))` (synthesized torn state per §2.4: manifest-last means "payload without manifest" ≡ pre-manifest crash; companion pinning already exists — the write-order comment + torn-tree unit test in `agent-core/src/checkpoint.rs` (content-search `manifest.json` + "torn"); reference it in the test comment). Then: `sessions reopen` → clean corrupt error (stable substring: characterize the actual `CheckpointError::Corrupt` surface text, pin one word like `"corrupt"`), exit != 0, no panic (`!transcript.contains("panicked")`), dir intact; `sessions list` exits 0.

- [ ] **Step 2: Scenario 11 — [real-kill] stale lock.** Park (Session leg). `DaemonCmd::hold_lock(&rig, &ckpt(&dir))` → wait `"LOCKED"` (`--dir` is the **checkpoint** dir, F2). SIGKILL it (`d.sigkill()`). `resume.lock` remains. CLI reopen → transcript contains `"is being resumed elsewhere"`, exit code 2, park intact, no panic. `// PRODUCT-GAP: stale resume.lock requires manual removal (spec §5#11)`.

- [ ] **Step 3: Scenario 12 — [real-kill] daemon SIGKILL + real-process restart re-emits.** `DaemonCmd::run(rig, stub, Some("SQRL-12 go"))` with a gated script step; wait for `ApprovalRequest` event line AND `parked.json`; `d.sigkill()`. Start a SECOND `DaemonCmd::run(rig, stub, None)` (no task) — wait for a `ParkedRuns` event with a non-empty `runs` naming the parked session id, then the re-derived `ApprovalRequest`; send `approve <id>` on its stdin; wait `Resumed` then `Done`. Assert the epoch classification: the second daemon's `ParkedRuns.runs[0].session_id` equals the FIRST process's session dir name (i.e. it owns a dir it did not create).

```rust
#[tokio::test]
async fn s12_daemon_sigkill_then_restart_reemits_and_resumes() {
    let stub = ScriptedStub::start(vec![gated_write("SQRL-12 go"), text_step(None, "done")]).await;
    let rig = Rig::new();
    let mut d1 = DaemonCmd::run(&rig, &stub.base_url(), Some("SQRL-12 go"));
    d1.wait_for_output("READY", CAP);
    d1.wait_for_event(CAP, |e| matches!(e, ServerEvent::ApprovalRequest { .. }));
    let dir = rig.session_dirs().into_iter()
        .find(|d| ckpt(d).join("parked.json").exists()).expect("park on disk");
    let parked_sid = dir.file_name().unwrap().to_string_lossy().into_owned();
    d1.sigkill();
    let _ = d1.wait_exit(CAP);

    let mut d2 = DaemonCmd::run(&rig, &stub.base_url(), None);
    d2.wait_for_output("READY", CAP);
    let ev = d2.wait_for_event(CAP, |e| matches!(e, ServerEvent::ParkedRuns { runs } if !runs.is_empty()));
    let ServerEvent::ParkedRuns { runs } = ev else { unreachable!() };
    assert!(runs.iter().any(|r| r.session_id == parked_sid),
        "restarted process must classify+own the prior process's dir");
    let ask = d2.wait_for_event(CAP, |e| matches!(e, ServerEvent::ApprovalRequest { .. }));
    let ServerEvent::ApprovalRequest { id, .. } = ask else { unreachable!() };
    d2.write_line(&format!("approve {id}"));
    d2.wait_for_event(CAP, |e| matches!(e, ServerEvent::Resumed { .. }));
    d2.wait_for_event(CAP, |e| matches!(e, ServerEvent::Done { .. }));
    stub.assert_consumed();
}
```

- [ ] **Step 4: Scenario 13 — park-write failure degrades cleanly.** Session leg; before sending the gated task, `chmod 0o555` the **session dir** (it pre-exists once the Session is constructed; a read-only sessions *root* would not block writes inside it — plan-review F5). Verify which level actually blocks the checkpoint-dir create on first run, then pin that level with a comment. Expect: `ApprovalRequest` still arrives, plus an `Error` event containing `"checkpoint write failed"` (verified stable substring, `loop_.rs` content-search `"approval not durable"`), and NO `parked.json`. Then approve → `Done` (ask functions live-only). **Restore permissions in a scopeguard/finally** (0o755) so TempDir teardown works even on panic — use a small `struct RestorePerms(PathBuf)` with `Drop`.

- [ ] **Step 5: Run + commit**

Run: `cd agent && cargo test -p agent-e2e --test crashkill`

```bash
git add -A && git commit -m "test(e2e): scenarios 10-13 — torn checkpoint, stale lock, real restart re-emit, park-write failure"
```

---

### Task 16: Scenarios 14–17 — concurrency

**Files:**
- Create: `agent/crates/agent-e2e/tests/concurrency.rs`

- [ ] **Step 1: Scenario 14 — in-process holder, CLI loser.** Park (Session leg); `agent_core::checkpoint::claim_resume(&ckpt(&dir))` in the test (same fn, explicit; checkpoint dir per F2); CLI reopen → `"is being resumed elsewhere"`, exit 2, park intact.

- [ ] **Step 2: Scenario 15 — CLI holder, Session loser, CLI completes.** Park (Session leg); drop Session; CLI reopen to the prompt (lock now held, pipe open, do NOT answer yet); fresh Session attach on same roots → wait re-derived `ApprovalRequest` → `session.approve(id, Approve)` → expect `ServerEvent::Error` containing `"is being resumed elsewhere"` (the loser observes contention; double-resolution excluded structurally). THEN answer `y` on the CLI → exits 0, script consumed.

- [ ] **Step 3: Scenario 16 — barrier race, symmetric postconditions.** Park; spawn CLI reopen AND, as soon as the CLI process is spawned (not prompted), fire `session.approve` from an attached Session. Accept EITHER winner: postconditions = exactly one of {CLI reached prompt+approve+exit-0, Session emitted Resumed+Done} succeeds; the other shows the contention error; `parked.json` consumed exactly once (gone at the end); no panic anywhere. Structure: collect both outcomes, assert `success_count == 1`.

- [ ] **Step 4: Scenario 17 — multi-session addressing.** Three sessions on one rig: park A (task text `TASK-A`), complete B (text-only script), park C (`TASK-C`). `sessions reopen <C-id>` with a script whose post-approve matcher REQUIRES `TASK-C` — the stub poisons if A's resume arrives instead. Then `sessions list` exits 0 and its transcript contains both remaining ids.

- [ ] **Step 5: Run + commit**

Run: `cd agent && cargo test -p agent-e2e --test concurrency`

```bash
git add -A && git commit -m "test(e2e): scenarios 14-17 — directed lock loser paths, symmetric race, multi-session addressing"
```

---

### Task 17: Scenarios 18–19 — corruption sweep + workspace deleted

**Files:**
- Create: `agent/crates/agent-e2e/tests/adversarial.rs`
- Modify: `agent/crates/agent-e2e/src/forge.rs` (tamper helpers)

- [ ] **Step 1: forge.rs additions** (all take explicit `key`/paths; no defaults):

```rust
/// Flip one byte mid-file. Panics if the file is shorter than 8 bytes.
pub fn flip_byte(path: &std::path::Path) {
    let mut b = std::fs::read(path).unwrap();
    let i = b.len() / 2;
    b[i] ^= 0xFF;
    std::fs::write(path, b).unwrap();
}

/// Write a forged answer.json: valid JSON, wrong MAC (random bytes hex).
pub fn forged_answer(dir: &std::path::Path) { /* shape-copy a real answer.json,
    replace the mac field with 64 hex zeros — read one real answer produced in a
    scratch run during implementation to pin the JSON shape, or construct via
    the ParkedAnswer serde type from agent-core if public */ }

/// Legacy-format answer (pre-versioning: no feedback field) — must fail closed.
pub fn legacy_answer(dir: &std::path::Path) { /* same, minus the feedback key,
    mac computed by NO ONE (any value) — the versioned formula must reject */ }
```

(Where a real type is public — `ParkedAnswer` is exported for the CLI — build through it; the plan's fallback is byte-level JSON. Resolve at implementation, keep helpers <60 lines total.)

- [ ] **Step 2: Scenario 18 — the sweep.** ONE test, five sub-cases, fresh park per sub-case (loop over `enum Corruption { ParkedBytes, ManifestBytes, ForgedAnswer, LegacyAnswer, NoDescriptor }`; a helper parks a fresh session on its own Rig per iteration — full isolation beats shared-state cleverness). For each: apply corruption; then (a) CLI `sessions reopen` → non-zero exit OR re-prompt (ForgedAnswer/LegacyAnswer cases re-prompt — answer `n` then feedback empty then expect repark/exit; characterize), transcript contains no `"panicked"`; (b) fresh Session attach → either no re-emit or a clean `Error` event — pin per-case after characterizing; session dir still exists; OTHER sessions unaffected (plant a second healthy park on the same rig for the ParkedBytes case only, assert it still re-emits).

- [ ] **Step 3: Scenario 19 — workspace deleted.** Park; `std::fs::remove_dir_all(rig.workspace.path())` (TempDir: use `close()`-then-recreate pattern or a plain `PathBuf` workspace for this test); reopen → clean error mentioning the workspace (characterize, pin substring), exit != 0, session dir intact.

- [ ] **Step 4: Run + commit**

Run: `cd agent && cargo test -p agent-e2e --test adversarial`

```bash
git add -A && git commit -m "test(e2e): scenarios 18-19 — corruption sweep (5 kinds, both surfaces) + workspace-gone refusal"
```

---

### Task 18: Scenarios 20–22 — robustness

**Files:**
- Create: `agent/crates/agent-e2e/tests/robustness.rs`

- [ ] **Step 1: Scenario 20 — mid-stream drop then recovery.** `RawDropStub`; Session leg; first `send_input` → expect `Error` event (any message — the surfacing is covered by e2e_robustness T4; do NOT pin text) and no wedge; second `send_input` → `Done` with token text `"recovered"`; session dir contains no park and no corruption (list exits 0).

- [ ] **Step 2: Scenario 21 — malformed + bogus tool.** Script `[MalformedJson]` → turn 1 errors cleanly; script continues `[BogusTool, text_step(None,"ok")]` → turn 2: the loop should feed the unknown-tool failure back and finish on the text step (characterize whether unknown tool errors the turn or round-trips; pin the observed CLEAN behavior); turn 3 (`text_step`) completes. One rig, one Session, three send_inputs.

- [ ] **Step 3: Scenario 22 — sink detach/reattach mid-turn.** Session leg with a `DelayedText{delay_ms: 3000}` step: `send_input`, then IMMEDIATELY `session.set_event_out(new_cap)` (the Tauri re-subscribe path — 4B-1 `b86e21c` abort class); assert `Done` arrives on the NEW sink. Then the parked variant: gated step → wait ask on cap1 → `set_event_out(cap2)` → assert the live ask is RE-EMITTED to cap2 (`reemit_pending`); approve on the session; `Done` on cap2.

- [ ] **Step 4: Run + commit**

Run: `cd agent && cargo test -p agent-e2e --test robustness`

```bash
git add -A && git commit -m "test(e2e): scenarios 20-22 — mid-stream drop recovery, malformed output, sink reattach"
```

---

### Task 19: Tier 1 gate — full run, budget re-measure, clippy

- [ ] **Step 1:** `cd agent && time cargo test -p agent-e2e` — ALL green, record wall time. If >90s: move `s04` soak behind `#[ignore = "soak: run explicitly"]`, re-measure, record the split in the plan (dated note) and spec log.
- [ ] **Step 2:** `cd agent && cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` (match ci.sh's exact legs — read `scripts/ci.sh` and run the agent/ legs it defines).
- [ ] **Step 3:** Run the whole `bash scripts/ci.sh` once.
- [ ] **Step 4:** Commit any fixes: `git commit -am "chore(e2e): tier-1 gate — full suite green, budget <N>s"`

---

### Task 20: Tier 3 — live-model reality check

**Files:**
- Create: `agent/crates/agent-e2e/tests/live_smoke.rs`

- [ ] **Step 1:** One `#[tokio::test] #[ignore = "live: needs llama-server on :8080 (qwen3.6-35b-a3b)"]` test: Rig; Session at `http://localhost:8080`, model `qwen3.6-35b-a3b` (add a `Rig::session_with_model` variant or parameterize); prompt: `"Use the write_file tool to create pong.txt containing exactly 'pong'. Do not ask questions."`; wait `ApprovalRequest` (retry once with a firmer prompt if the model answers in prose — spec allows one retry, print that it happened); assert `parked.json`; drop Session; CLI reopen (`--base-url http://localhost:8080 --model qwen3.6-35b-a3b`) → approve → exit 0; assert `pong.txt` exists in the rig workspace.
- [ ] **Step 2:** Gate check first: `curl -s -m 2 localhost:8080/health` — if not `ok`, skip running (the test stays `--ignored`; just don't verify it live if the server is down, and say so in the commit).
- [ ] **Step 3:** Run: `cd agent && cargo test -p agent-e2e --test live_smoke -- --ignored --nocapture` (needs :8080). Expected: PASS.
- [ ] **Step 4:** `git add -A && git commit -m "test(e2e): tier-3 live-model park/reopen reality check (--ignored)"`

---

### Task 21: Tier 2 — WebDriver GUI lifecycle checks

**Files:**
- Create: `src-tauri/tests/gui_lifecycle.rs`
- Modify: `src-tauri/tests/e2e_harness/mod.rs` (env injection), `src-tauri/Cargo.toml` (dev-deps: `agent-server` testkit + `agent-runtime-config` if not present)

**This is the `src-tauri` workspace** — build/test from `src-tauri/`. Serialized: `--test-threads=1`, one WebDriver session per test, ALWAYS quit (existing harness handles it).

- [ ] **Step 1: Harness env injection.** Extend the e2e_harness's tauri-driver spawn to accept `envs: &[(&str, &str)]` applied to the tauri-driver Command (the app inherits them). New constructor variant; existing tests untouched.
- [ ] **Step 2: State seeding under a temp HOME.** Each test: tempdir fake home; set `HOME` and `XDG_CONFIG_HOME=<home>/.config` via the harness; pre-create `<home>/.rusty-agent/secret` (32 bytes) and seed a parked session under `<home>/.rusty-agent/sessions` using `agent_server::testkit::plant_parked_session` pointed at those roots (dev-dep on agent-server with `features=["testkit"]`). **Verify first** (characterize): that the app under fake HOME actually scans that sessions root — if `app_config_dir` resists relocation via XDG on this machine, STOP and descope per spec §3 (record in spec log; do not point tests at the real HOME).
- [ ] **Step 3: T2.1** — launch app; wait for the parked-run banner element (find its DOM hook by reading `web/src` — search for the component rendering `parked_runs`; select by stable text or aria attribute, never index); click the banner's Approve button in the real DOM. **T2.1 caveat:** approve triggers a resume that hits the model URL — hardcoded `:8080`. If llama-server is up, the resume proceeds (fine, live-ish); if not, expect the resume to error visibly — assert the banner cleared + no crash. Pin whichever is deterministic on this machine (llama on :8080 is the norm — treat it as required, like `turn_smoke`).
- [ ] **Step 4: T2.2** — seeded park; open the approval prompt from the banner; click Deny; type feedback text `SQRL-T2-FB` in the feedback field (find by aria/label); submit; assert (filesystem) `answer.json` appears in the seeded session dir — MAC-bound deny committed from the real DOM.
- [ ] **Step 5: T2.3** — real cross-surface switch: drive a live turn from the composer (`textarea[aria-label='prompt']`) that triggers a write approval (`Use the write_file tool to create t23.txt...`); when the in-app approval prompt appears, close the app (end the WebDriver session — the park is on disk); then run `agent sessions list` (with `HOME`/dirs pointed at the fake home via `--trace-dir`/`--metadata-dir`) → transcript shows the parked session; `agent sessions reopen <id>` → approve → exit 0. This guards src-tauri's `local_params` plumbing end to end.
- [ ] **Step 6:** All three `#[ignore = "GUI: needs tauri-driver + display (+ :8080 for turns)"]`. Run: `cd src-tauri && cargo test --test gui_lifecycle -- --ignored --test-threads=1`. Expected: PASS on this machine.
- [ ] **Step 7:** `git add -A && git commit -m "test(gui): tier-2 lifecycle checks — banner, DOM deny feedback, real GUI→CLI switch"`

---

### Task 22: Docs + final gate + branch finish

- [ ] **Step 1:** Add ONE paragraph to `.agents/skills/auto-drive-tauri/SKILL.md` under the GUI-driving section: Tier-2 lifecycle tests exist (`src-tauri/tests/gui_lifecycle.rs`, run recipe), and the Tier-1 lifecycle matrix lives in `agent/crates/agent-e2e` (bridge-equivalent Session driving; no WS — do NOT expand the WS de-stale here, that's follow-up E4). Say the skip-ahead classification out loud in the PR/summary: this paragraph is a factual pointer, not normative guidance.
- [ ] **Step 2:** Update the spec's Panel & review log: add a dated "Implementation notes" entry listing (a) the s06b descope (if not already committed in Task 13), (b) any characterize-then-pin outcomes that surprised (scenarios 7, 9, 13, 18, 21), (c) the budget measurement + any soak split.
- [ ] **Step 3:** `bash scripts/ci.sh` — full pass, both workspaces.
- [ ] **Step 4:** Commit: `git add -A && git commit -m "docs(e2e): tier-2 pointer in auto-drive-tauri + spec implementation log"`
- [ ] **Step 5:** Use superpowers:finishing-a-development-branch (present merge/PR options; suite is done when ci.sh is green and Tier 2/3 have been run live at least once on this machine).

---

## Plan review log

**2026-07-10 — single plan reviewer (coverage/decomposition/buildability): APPROVE-WITH-FIXES; all 8 findings applied in place.** F1 raw-chunk stdout reader (prompts have no trailing newline); F2 all checkpoint artifacts under `<session_dir>/checkpoint/` — `rig::ckpt()` helper + full sweep; F3 `multi_thread` tokio flavor mandated; F4 `plant_parked_session` gains a `meta` root param (3-arg form would key against real `$HOME`); F5 s13 chmods the session dir, not the root; F6 E2 test asserts the timeout field, no stdin; F7 `CARGO_TARGET_DIR` honored; F8 trim-imports + `assert_consumed` discipline noted. Coverage/decomposition/buildability otherwise verified clean (from_launch signature, claim_resume pub, clap ordering, SSE shapes, rustc 1.96 process_group).

## Self-review notes (author)

- Spec coverage: scenarios 1–22 → Tasks 9–18 (2 per §5a table row check: #1–9 ✓, #10–13 ✓, #14–17 ✓, #18–19 ✓, #20–22 ✓); E1 → Tasks 1–2; E2 → Task 3; testkit → Task 4; drivers → Tasks 5–8; [real-kill] scenarios 5, 6a, 11, 12 use real signals ✓; budget gates → Tasks 11, 19; Tier 2 → Task 21 (T2.1–T2.3); Tier 3 → Task 20; docs/log → Task 22.
- Known deliberate deviations recorded IN tasks: s06b sanctioned descope (Task 13, logged); T2.1 resume-target caveat (Task 21); characterize-then-pin steps for scenarios 7, 9, 13, 18, 21 per spec §8.
- Type consistency: `Rig`/`ScriptedStub`/`CliCmd`/`Cli`/`DaemonCmd` signatures defined once (Tasks 5–8) and consumed by name in Tasks 9–18; `wait_for_ask_id` from testkit (Task 4); `forge` module introduced Task 13, extended Task 17.
- Placeholders check: forge helper bodies in Task 17 intentionally defer JSON shape to a pinned real artifact (explicit instruction to pin from a real run — not a TBD); REPL prompt marker explicitly discovered in Task 7 and constant-ized.

