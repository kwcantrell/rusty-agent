# OS-Level Sandboxing via Docker — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Confine `execute_command` and MCP stdio servers inside Docker containers so a command cannot escape the workspace, reach the network, or exhaust host resources even when logical policy has a gap.

**Architecture:** A `SandboxStrategy` trait (in `agent-tools`, alongside `ToolCtx`) abstracts process launch. `HostExecutor` (default, in `agent-tools`) reproduces today's behavior; `DockerSandbox` (new `agent-sandbox` crate) shells out to `docker run` with confinement flags. `ToolCtx` and the MCP `StdioTransport` launch through the trait; the binaries build the concrete strategy from `RuntimeConfig` and inject it. `agent-core` only ever sees the trait, so the core stays mechanism-agnostic.

**Tech Stack:** Rust (workspace under `agent/`), `tokio::process`, `async_trait` (already used), `thiserror`, `tracing`, the `docker` CLI (podman-compatible). No new runtime dependencies beyond the `docker` binary at runtime.

## Global Constraints

- Rust 1.96 via rustup; cargo is NOT on PATH — run `source "$HOME/.cargo/env"` before any cargo command.
- Build/test from `agent/`: `cargo test --workspace`; lint `cargo clippy --all-targets -- -D warnings` (warnings are errors).
- `SandboxStrategy::launch` is a **synchronous** trait method (spawning a child is non-blocking); `describe` is sync too. Do not make them async — the MCP transport's `spawn` must stay callable from its current sync context.
- Default mode is `auto`: containerize when Docker is available; **warn-and-degrade to host** when it is not. `enforce` **fails closed** (`SandboxError::Unavailable` → `ToolError::Denied`). `off` wires `HostExecutor`.
- Default wiring is `HostExecutor` — behavior is unchanged until sandboxing is configured on. `agent-core` must NOT depend on `agent-sandbox`.
- Docker flags are non-negotiable hardening: `--rm` (OneShot), `--network none` unless `sandbox_network`, `--read-only --tmpfs /tmp:rw,size=<t>`, `--cap-drop ALL`, `--security-opt no-new-privileges`, `--user <uid>:<gid>`, `-v <workspace>:/workspace -w /workspace`. NEVER pass `--privileged`, `--security-opt seccomp=unconfined`, `--cap-add`, or mount the docker socket.
- New `RuntimeConfig` fields use `#[serde(default)]` so an old on-disk config / browser round-trip can never wipe them. Settings-panel/web wiring is OUT of scope this slice (flags + disk only).
- Concrete defaults: image `debian:stable-slim`, memory `2g`, cpus `2`, pids `512`, tmp `256m`, fsize `None`.
- TDD: write the failing test first, watch it fail, implement minimally, watch it pass, commit. Real-Docker tests are `#[ignore]`-gated.

---

### Task 1: Sandbox core types, trait, `HostExecutor`, `SandboxedChild`

**Files:**
- Create: `agent/crates/agent-tools/src/sandbox.rs`
- Modify: `agent/crates/agent-tools/src/lib.rs` (add `mod sandbox; pub use sandbox::*;`)
- Modify: `agent/crates/agent-tools/Cargo.toml` (ensure `thiserror`, `async-trait`, `tokio` features `process`/`io-util` present — they already are; confirm)

**Interfaces:**
- Produces: `CommandSpec`, `ProcKind`, `Mode`, `Limits`, `SandboxDescriptor`, `SandboxError`, `SandboxedChild`, `trait SandboxStrategy`, `struct HostExecutor`. These names are consumed by every later task.

Exact API to implement:

```rust
// agent-tools/src/sandbox.rs
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::{Child, ChildStdin, ChildStdout, ChildStderr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcKind { OneShot, Service }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode { Off, Auto, Enforce }

#[derive(Debug, Clone)]
pub struct Limits {
    pub memory: String,        // "2g"
    pub cpus: String,          // "2"
    pub pids: u32,             // 512
    pub fsize: Option<String>, // ulimit fsize, e.g. "1g"
    pub tmp_size: String,      // "256m"
}

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub kind: ProcKind,
}

#[derive(Debug, Clone)]
pub struct SandboxDescriptor {
    pub mode: Mode,
    pub mechanism: &'static str, // "host" | "docker"
    pub image: Option<String>,
    pub network: bool,
    pub degraded: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("sandbox unavailable: {0}")]
    Unavailable(String),
    #[error("sandbox launch failed: {0}")]
    LaunchFailed(String),
    #[error("invalid mount: {0}")]
    InvalidMount(String),
}

/// A launched process plus optional container id for teardown.
pub struct SandboxedChild {
    child: Child,
    container: Option<String>, // docker container name; None for host
}

impl SandboxedChild {
    pub fn new_host(child: Child) -> Self { Self { child, container: None } }
    pub fn new_container(child: Child, name: String) -> Self {
        Self { child, container: Some(name) }
    }
    pub fn take_stdin(&mut self) -> Option<ChildStdin> { self.child.stdin.take() }
    pub fn take_stdout(&mut self) -> Option<ChildStdout> { self.child.stdout.take() }
    pub fn take_stderr(&mut self) -> Option<ChildStderr> { self.child.stderr.take() }
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }
    /// Kill the container (docker kill) or the local child; idempotent best-effort.
    pub async fn kill(&mut self) {
        if let Some(name) = &self.container {
            let _ = tokio::process::Command::new("docker")
                .args(["kill", name]).output().await;
        }
        let _ = self.child.start_kill();
    }
}

impl Drop for SandboxedChild {
    fn drop(&mut self) {
        // Backstop: Drop cannot await. Fire-and-forget a detached docker kill,
        // and start_kill the local child so nothing leaks on panic/early-return.
        if let Some(name) = self.container.take() {
            let _ = std::process::Command::new("docker").args(["kill", &name]).spawn();
        }
        let _ = self.child.start_kill();
    }
}

pub trait SandboxStrategy: Send + Sync {
    fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError>;
    fn describe(&self) -> SandboxDescriptor;
}

/// Default strategy: run on the host exactly as the core did pre-sandbox.
pub struct HostExecutor;

impl SandboxStrategy for HostExecutor {
    fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError> {
        let mut cmd = tokio::process::Command::new(&spec.program);
        cmd.args(&spec.args).current_dir(&spec.cwd).envs(&spec.env)
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Service (mcp) needs an open stdin pipe; OneShot does not read stdin.
        match spec.kind {
            ProcKind::Service => { cmd.stdin(Stdio::piped()); }
            ProcKind::OneShot => { cmd.stdin(Stdio::null()); }
        }
        let child = cmd.spawn().map_err(|e| SandboxError::LaunchFailed(e.to_string()))?;
        Ok(SandboxedChild::new_host(child))
    }
    fn describe(&self) -> SandboxDescriptor {
        SandboxDescriptor { mode: Mode::Off, mechanism: "host", image: None,
            network: true, degraded: None }
    }
}
```

- [ ] **Step 1: Write the failing test**

```rust
// in agent-tools/src/sandbox.rs
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spec(program: &str, args: &[&str]) -> CommandSpec {
        CommandSpec { program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir(), env: Default::default(), kind: ProcKind::OneShot }
    }

    #[tokio::test]
    async fn host_executor_runs_and_captures_stdout() {
        let mut sb = HostExecutor.launch(spec("sh", &["-c", "echo hi"])).unwrap();
        let mut out = sb.take_stdout().unwrap();
        let mut buf = String::new();
        use tokio::io::AsyncReadExt;
        out.read_to_string(&mut buf).await.unwrap();
        let status = tokio::time::timeout(Duration::from_secs(5), sb.wait())
            .await.unwrap().unwrap();
        assert!(status.success());
        assert!(buf.contains("hi"));
    }

    #[test]
    fn host_descriptor_is_host_mechanism() {
        assert_eq!(HostExecutor.describe().mechanism, "host");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && source "$HOME/.cargo/env" && cargo test -p agent-tools sandbox`
Expected: FAIL — `sandbox` module / types do not exist.

- [ ] **Step 3: Write minimal implementation**

Create `agent/crates/agent-tools/src/sandbox.rs` with the full API block above. Add to `agent/crates/agent-tools/src/lib.rs`:

```rust
pub mod sandbox;
pub use sandbox::*;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-tools sandbox`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/src/sandbox.rs agent/crates/agent-tools/src/lib.rs
git commit -m "feat(sandbox): SandboxStrategy trait + HostExecutor + SandboxedChild"
```

---

### Task 2: Thread the strategy into `ToolCtx` and route `execute_command` through it

**Files:**
- Modify: `agent/crates/agent-tools/src/types.rs` (`ToolCtx` gains `sandbox` field)
- Modify: `agent/crates/agent-tools/src/shell.rs` (`ExecuteCommand::execute` uses `ctx.sandbox`)
- Modify: `agent/crates/agent-tools/src/fs/write.rs` (test `ctx()` constructor)
- Modify any other in-crate `ToolCtx { .. }` literal in tests (`shell.rs`, `fs/read.rs` if present)

**Interfaces:**
- Consumes: `SandboxStrategy`, `HostExecutor`, `CommandSpec`, `ProcKind`, `SandboxError` (Task 1).
- Produces: `ToolCtx.sandbox: std::sync::Arc<dyn SandboxStrategy>`. Later tasks (loop, tests) construct `ToolCtx` with this field.

`ToolCtx` becomes:

```rust
// agent-tools/src/types.rs
use std::sync::Arc;
use crate::SandboxStrategy;

pub struct ToolCtx {
    pub workspace: PathBuf,
    pub timeout: Duration,
    pub cancel: CancellationToken,
    pub sandbox: Arc<dyn SandboxStrategy>,
}
```

`ExecuteCommand::execute` is rewritten to launch through the strategy. A `SandboxError::Unavailable` maps to `ToolError::Denied` (so `enforce` mode refuses cleanly); other sandbox errors map to `ToolError::Failed`:

```rust
// agent-tools/src/shell.rs — replace the body of execute()
async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx)
    -> Result<ToolOutput, ToolError> {
    use tokio::io::AsyncReadExt;
    let command = cmd_arg(&args)?;
    let spec = crate::CommandSpec {
        program: "sh".into(),
        args: vec!["-c".into(), command.clone()],
        cwd: ctx.workspace.clone(),
        env: Default::default(),
        kind: crate::ProcKind::OneShot,
    };
    let mut child = ctx.sandbox.launch(spec).map_err(|e| match e {
        crate::SandboxError::Unavailable(m) => ToolError::Denied(m),
        other => ToolError::Failed { message: other.to_string(), stderr: None },
    })?;

    let mut out_pipe = child.take_stdout();
    let mut err_pipe = child.take_stderr();
    let read_out = async {
        let mut s = String::new();
        if let Some(p) = out_pipe.as_mut() { let _ = p.read_to_string(&mut s).await; }
        s
    };
    let read_err = async {
        let mut s = String::new();
        if let Some(p) = err_pipe.as_mut() { let _ = p.read_to_string(&mut s).await; }
        s
    };

    let status = tokio::select! {
        _ = ctx.cancel.cancelled() => { child.kill().await; return Err(ToolError::Denied("cancelled".into())); }
        r = tokio::time::timeout(ctx.timeout, child.wait()) => match r {
            Err(_elapsed) => { child.kill().await; return Err(ToolError::Timeout); }
            Ok(inner) => inner.map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })?,
        }
    };
    let (stdout, stderr) = tokio::join!(read_out, read_err);
    let exit_code = status.code().unwrap_or(-1);
    let content = format!("exit={exit_code}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}");
    Ok(ToolOutput { content, display: Some(Display::Terminal {
        command, stdout, stderr, exit_code }) })
}
```

- [ ] **Step 1: Update the failing test first**

In `agent/crates/agent-tools/src/shell.rs` test module, change the `ctx()` helper to supply a `HostExecutor`:

```rust
use std::sync::Arc;
fn ctx(timeout: Duration) -> ToolCtx {
    ToolCtx { workspace: std::env::temp_dir(), timeout, cancel: CancellationToken::new(),
        sandbox: Arc::new(crate::HostExecutor) }
}
```

Do the same for the `ctx()` helper in `agent/crates/agent-tools/src/fs/write.rs` (and `fs/read.rs` if it has one): add `sandbox: Arc::new(crate::HostExecutor)`.

- [ ] **Step 2: Run tests to verify they fail to compile**

Run: `cd agent && cargo test -p agent-tools`
Expected: FAIL — `ToolCtx` has no `sandbox` field / `execute` signature mismatch.

- [ ] **Step 3: Write minimal implementation**

Apply the `ToolCtx` field addition (`types.rs`) and the `ExecuteCommand::execute` rewrite (`shell.rs`) shown above.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-tools && cargo clippy -p agent-tools --all-targets -- -D warnings`
Expected: PASS — including the pre-existing `runs_command_and_captures_stdout` and `times_out_long_command` (parity through `HostExecutor`).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/src/types.rs agent/crates/agent-tools/src/shell.rs agent/crates/agent-tools/src/fs/write.rs
git commit -m "feat(sandbox): route execute_command through ToolCtx.sandbox (HostExecutor parity)"
```

---

### Task 3: Thread the strategy through `LoopConfig` → `run_tool`

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (`LoopConfig` field; `run_tool` builds `ToolCtx` with it; posture suffix deferred to Task 12)
- Modify: `agent/crates/agent-core/Cargo.toml` (already depends on `agent-tools`; no change expected — confirm)

**Interfaces:**
- Consumes: `Arc<dyn SandboxStrategy>`, `HostExecutor` (Tasks 1–2).
- Produces: `LoopConfig.sandbox: Option<Arc<dyn agent_tools::SandboxStrategy>>` (default `None` ⇒ `HostExecutor`). Binaries set it in Task 10.

`LoopConfig` keeps `#[derive(Default)]` by using `Option`:

```rust
// agent-core/src/loop_.rs — add to LoopConfig
pub sandbox: Option<std::sync::Arc<dyn agent_tools::SandboxStrategy>>,
```

`run_tool` builds the `ToolCtx` with the configured strategy (or a `HostExecutor` fallback):

```rust
// in run_tool, replace the ToolCtx construction
let sandbox = self.config.sandbox.clone()
    .unwrap_or_else(|| std::sync::Arc::new(agent_tools::HostExecutor));
let ctx = ToolCtx { workspace: self.config.workspace.clone(),
    timeout: self.config.tool_timeout, cancel: CancellationToken::new(),
    sandbox };
```

- [ ] **Step 1: Write the failing test**

Add to `agent/crates/agent-core/src/loop_.rs` test module a fake strategy that records launches, and assert the loop routes `execute_command` through `ctx.sandbox`:

```rust
#[tokio::test]
async fn loop_routes_execute_command_through_injected_sandbox() {
    use agent_tools::{CommandSpec, SandboxStrategy, SandboxedChild, SandboxError,
        SandboxDescriptor, Mode, HostExecutor};
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    struct CountingSandbox { inner: HostExecutor, hits: Arc<AtomicUsize> }
    impl SandboxStrategy for CountingSandbox {
        fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            self.inner.launch(spec)
        }
        fn describe(&self) -> SandboxDescriptor {
            SandboxDescriptor { mode: Mode::Off, mechanism: "counting", image: None,
                network: true, degraded: None }
        }
    }

    let hits = Arc::new(AtomicUsize::new(0));
    let sandbox = Arc::new(CountingSandbox { inner: HostExecutor, hits: hits.clone() });
    // Build an AgentLoop whose LoopConfig.sandbox = Some(sandbox), drive a single
    // execute_command tool call via the existing testkit (AlwaysApprove + allow policy),
    // then:
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}
```

(Use the crate's existing `testkit` helpers — `AlwaysApprove`, a stub `ModelClient` that emits one `execute_command` tool call — mirroring the established `run_tool` tests in this file. Set `LoopConfig { sandbox: Some(sandbox), ..base }`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-core loop_routes_execute_command_through_injected_sandbox`
Expected: FAIL — `LoopConfig` has no `sandbox` field.

- [ ] **Step 3: Write minimal implementation**

Add the `sandbox` field to `LoopConfig` and update `run_tool`'s `ToolCtx` construction as shown.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core && cargo clippy -p agent-core --all-targets -- -D warnings`
Expected: PASS — the new test plus all existing `loop_` tests (their `LoopConfig` literals get `sandbox: None` via `..Default::default()`, or add `sandbox: None` explicitly if they use full literals).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs
git commit -m "feat(sandbox): thread SandboxStrategy through LoopConfig into run_tool"
```

---

### Task 4: New `agent-sandbox` crate + pure `docker run` argv builder

**Files:**
- Create: `agent/crates/agent-sandbox/Cargo.toml`
- Create: `agent/crates/agent-sandbox/src/lib.rs`
- Create: `agent/crates/agent-sandbox/src/docker.rs` (argv builder + `SandboxPolicy`)
- Modify: `agent/Cargo.toml` (add `crates/agent-sandbox` to workspace members)

**Interfaces:**
- Consumes: `CommandSpec`, `ProcKind`, `Limits`, `Mode` (agent-tools).
- Produces: `SandboxPolicy { mode, image, network, limits, extra_rw, extra_ro }`; `fn docker_run_args(policy: &SandboxPolicy, spec: &CommandSpec, container_name: &str, uid_gid: &str) -> Vec<String>`. Consumed by Task 6.

`Cargo.toml` deps: `agent-tools = { path = "../agent-tools" }`, `tracing`, `thiserror`, `tokio = { workspace = true, features = ["process","io-util","rt","macros","time"] }` (match the workspace style used by `agent-mcp`/`agent-http`).

```rust
// agent-sandbox/src/docker.rs
use agent_tools::{CommandSpec, Limits, Mode, ProcKind};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    pub mode: Mode,
    pub image: String,
    pub network: bool,
    pub limits: Limits,
    pub extra_rw: Vec<PathBuf>, // already validated (Task 5)
    pub extra_ro: Vec<PathBuf>,
}

pub const WORKDIR: &str = "/workspace";

/// Build the full `docker run …` argument vector (excluding the leading "docker").
pub fn docker_run_args(policy: &SandboxPolicy, spec: &CommandSpec,
    container_name: &str, uid_gid: &str) -> Vec<String> {
    let mut a: Vec<String> = vec!["run".into()];
    match spec.kind {
        ProcKind::OneShot => a.push("--rm".into()),
        ProcKind::Service => { a.push("-i".into()); } // keep stdin open; no --rm (we kill by name)
    }
    a.push("--name".into()); a.push(container_name.into());
    a.push("--network".into());
    a.push(if policy.network { "bridge".into() } else { "none".into() });
    a.push("--memory".into());      a.push(policy.limits.memory.clone());
    a.push("--cpus".into());        a.push(policy.limits.cpus.clone());
    a.push("--pids-limit".into());  a.push(policy.limits.pids.to_string());
    if let Some(f) = &policy.limits.fsize {
        a.push("--ulimit".into());  a.push(format!("fsize={f}"));
    }
    a.push("--read-only".into());
    a.push("--tmpfs".into());       a.push(format!("/tmp:rw,size={}", policy.limits.tmp_size));
    a.push("--cap-drop".into());    a.push("ALL".into());
    a.push("--security-opt".into()); a.push("no-new-privileges".into());
    a.push("--user".into());        a.push(uid_gid.into());
    // Workspace mount (RW) at a fixed path.
    a.push("-v".into());
    a.push(format!("{}:{}", spec.cwd.display(), WORKDIR));
    a.push("-w".into());            a.push(WORKDIR.into());
    for p in &policy.extra_rw {
        a.push("-v".into()); a.push(format!("{}:{}:rw", p.display(), p.display()));
    }
    for p in &policy.extra_ro {
        a.push("-v".into()); a.push(format!("{}:{}:ro", p.display(), p.display()));
    }
    // Env (-e KEY=VAL), sorted for determinism.
    for (k, v) in &spec.env {
        a.push("-e".into()); a.push(format!("{k}={v}"));
    }
    a.push(policy.image.clone());
    a.push(spec.program.clone());
    a.extend(spec.args.iter().cloned());
    a
}
```

- [ ] **Step 1: Write the failing test**

```rust
// agent-sandbox/src/docker.rs
#[cfg(test)]
mod tests {
    use super::*;
    use agent_tools::{CommandSpec, Limits, Mode, ProcKind};
    use std::path::PathBuf;

    fn policy(network: bool) -> SandboxPolicy {
        SandboxPolicy { mode: Mode::Auto, image: "debian:stable-slim".into(), network,
            limits: Limits { memory: "2g".into(), cpus: "2".into(), pids: 512,
                fsize: None, tmp_size: "256m".into() },
            extra_rw: vec![], extra_ro: vec![] }
    }
    fn oneshot() -> CommandSpec {
        CommandSpec { program: "sh".into(), args: vec!["-c".into(), "echo hi".into()],
            cwd: PathBuf::from("/work"), env: Default::default(), kind: ProcKind::OneShot }
    }

    #[test]
    fn oneshot_has_hardening_flags_and_network_none() {
        let v = docker_run_args(&policy(false), &oneshot(), "agent-sbx-1", "1000:1000");
        let s = v.join(" ");
        assert!(s.contains("run --rm"));
        assert!(s.contains("--network none"));
        assert!(s.contains("--read-only"));
        assert!(s.contains("--cap-drop ALL"));
        assert!(s.contains("--security-opt no-new-privileges"));
        assert!(s.contains("--user 1000:1000"));
        assert!(s.contains("-v /work:/workspace"));
        assert!(s.contains("-w /workspace"));
        assert!(s.ends_with("debian:stable-slim sh -c echo hi"));
        assert!(!s.contains("--privileged"));
        assert!(!s.contains("seccomp=unconfined"));
    }

    #[test]
    fn network_true_uses_bridge() {
        let v = docker_run_args(&policy(true), &oneshot(), "n", "1000:1000");
        assert!(v.join(" ").contains("--network bridge"));
    }

    #[test]
    fn service_keeps_stdin_open_and_no_rm() {
        let mut spec = oneshot(); spec.kind = ProcKind::Service;
        let v = docker_run_args(&policy(false), &spec, "n", "1000:1000");
        assert!(v.contains(&"-i".to_string()));
        assert!(!v.contains(&"--rm".to_string()));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-sandbox`
Expected: FAIL — crate / module does not exist yet.

- [ ] **Step 3: Write minimal implementation**

Create the crate files, add `crates/agent-sandbox` to `agent/Cargo.toml` `members`, and implement `docker.rs` as above. `lib.rs`:

```rust
mod docker;
pub use docker::{docker_run_args, SandboxPolicy, WORKDIR};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-sandbox && cargo clippy -p agent-sandbox --all-targets -- -D warnings`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/Cargo.toml agent/crates/agent-sandbox
git commit -m "feat(sandbox): agent-sandbox crate with pure docker run argv builder"
```

---

### Task 5: Mount-grant validation

**Files:**
- Create: `agent/crates/agent-sandbox/src/mounts.rs`
- Modify: `agent/crates/agent-sandbox/src/lib.rs` (export)

**Interfaces:**
- Produces: `fn validate_mount(path: &str, home: Option<&Path>) -> Result<PathBuf, SandboxError>`. Returns the canonicalized path or `SandboxError::InvalidMount`. Consumed by Task 10 when building `SandboxPolicy.extra_rw/ro`.

Rules: expand a leading `~` to `home`; canonicalize; reject if the result is `/`, equals `home` exactly (the `$HOME` root), or is `/var/run/docker.sock` / its parent `/var/run` / `/run/docker.sock`. (Subdirectories of `$HOME`, e.g. `~/.cargo`, are allowed — only the root is blocked.)

```rust
// agent-sandbox/src/mounts.rs
use agent_tools::SandboxError;
use std::path::{Path, PathBuf};

pub fn validate_mount(path: &str, home: Option<&Path>) -> Result<PathBuf, SandboxError> {
    let expanded: PathBuf = if let Some(rest) = path.strip_prefix("~/") {
        match home { Some(h) => h.join(rest),
            None => return Err(SandboxError::InvalidMount(format!("~ unsupported: {path}"))) }
    } else if path == "~" {
        match home { Some(h) => h.to_path_buf(),
            None => return Err(SandboxError::InvalidMount(format!("~ unsupported: {path}"))) }
    } else { PathBuf::from(path) };

    let canon = expanded.canonicalize()
        .map_err(|e| SandboxError::InvalidMount(format!("{}: {e}", expanded.display())))?;

    if canon == Path::new("/") {
        return Err(SandboxError::InvalidMount("refusing to mount /".into()));
    }
    if let Some(h) = home {
        if canon == h {
            return Err(SandboxError::InvalidMount("refusing to mount \\$HOME root".into()));
        }
    }
    for bad in ["/var/run/docker.sock", "/run/docker.sock", "/var/run", "/run"] {
        if canon == Path::new(bad) {
            return Err(SandboxError::InvalidMount(format!("refusing to mount {bad}")));
        }
    }
    Ok(canon)
}
```

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn rejects_root_and_home_root_and_socket() {
        assert!(validate_mount("/", None).is_err());
        let home = std::env::temp_dir();
        assert!(validate_mount(home.to_str().unwrap(), Some(&home)).is_err());
        assert!(validate_mount("/var/run/docker.sock", None).is_err());
    }

    #[test]
    fn accepts_a_real_subdir() {
        let dir = std::env::temp_dir();
        let sub = dir.join("agent-sbx-mount-test");
        std::fs::create_dir_all(&sub).unwrap();
        let got = validate_mount(sub.to_str().unwrap(), Some(&dir)).unwrap();
        assert_eq!(got, sub.canonicalize().unwrap());
    }

    #[test]
    fn expands_tilde_subdir() {
        let home = std::env::temp_dir();
        let sub = home.join("agent-sbx-tilde");
        std::fs::create_dir_all(&sub).unwrap();
        let got = validate_mount("~/agent-sbx-tilde", Some(&home)).unwrap();
        assert_eq!(got, sub.canonicalize().unwrap());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-sandbox mounts`
Expected: FAIL — `mounts` module / `validate_mount` missing.

- [ ] **Step 3: Write minimal implementation**

Create `mounts.rs` as above; add `mod mounts; pub use mounts::validate_mount;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-sandbox mounts && cargo clippy -p agent-sandbox --all-targets -- -D warnings`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-sandbox/src/mounts.rs agent/crates/agent-sandbox/src/lib.rs
git commit -m "feat(sandbox): mount-grant validation (reject /, \\$HOME root, docker socket)"
```

---

### Task 6: `DockerSandbox` — probe, mode logic, launch, kill

**Files:**
- Create: `agent/crates/agent-sandbox/src/strategy.rs`
- Modify: `agent/crates/agent-sandbox/src/lib.rs` (export `DockerSandbox`, `Availability`)

**Interfaces:**
- Consumes: `docker_run_args`, `SandboxPolicy` (Task 4); `SandboxStrategy`, `CommandSpec`, `SandboxedChild`, `SandboxError`, `Mode`, `SandboxDescriptor` (agent-tools).
- Produces: `DockerSandbox::new(policy: SandboxPolicy, uid_gid: String, available: Availability) -> DockerSandbox` and `pub enum Availability { Available, Unavailable(String) }`; plus `DockerSandbox::probe() -> Availability` (runs `docker version`). `new` takes `available` injected so tests need no Docker. The binary calls `probe()` then `new`.

A monotonic container-name counter avoids collisions without a uuid dep:

```rust
// agent-sandbox/src/strategy.rs
use agent_tools::{CommandSpec, Mode, ProcKind, SandboxDescriptor, SandboxError,
    SandboxStrategy, SandboxedChild};
use crate::{docker_run_args, SandboxPolicy};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub enum Availability { Available, Unavailable(String) }

pub struct DockerSandbox {
    policy: SandboxPolicy,
    uid_gid: String,
    available: Availability,
}

impl DockerSandbox {
    pub fn new(policy: SandboxPolicy, uid_gid: String, available: Availability) -> Self {
        Self { policy, uid_gid, available }
    }

    /// Blocking availability probe; run once at startup before `new`.
    pub fn probe() -> Availability {
        match std::process::Command::new("docker")
            .args(["version", "--format", "{{.Server.Version}}"])
            .stdout(Stdio::null()).stderr(Stdio::null()).status() {
            Ok(s) if s.success() => Availability::Available,
            Ok(s) => Availability::Unavailable(format!("docker version exited {s}")),
            Err(e) => Availability::Unavailable(e.to_string()),
        }
    }

    fn spawn_docker(&self, spec: &CommandSpec, name: &str)
        -> Result<SandboxedChild, SandboxError> {
        let args = docker_run_args(&self.policy, spec, name, &self.uid_gid);
        let mut cmd = tokio::process::Command::new("docker");
        cmd.args(&args).kill_on_drop(true)
            .stdout(Stdio::piped()).stderr(Stdio::piped());
        match spec.kind {
            ProcKind::Service => { cmd.stdin(Stdio::piped()); }
            ProcKind::OneShot => { cmd.stdin(Stdio::null()); }
        }
        tracing::info!(target: "sandbox", mechanism="docker", image=%self.policy.image,
            network=self.policy.network, container=%name, "launching sandboxed process");
        let child = cmd.spawn().map_err(|e| SandboxError::LaunchFailed(e.to_string()))?;
        Ok(SandboxedChild::new_container(child, name.to_string()))
    }
}

impl SandboxStrategy for DockerSandbox {
    fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError> {
        let name = format!("agent-sbx-{}-{}", std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst));
        match &self.available {
            Availability::Available => self.spawn_docker(&spec, &name),
            Availability::Unavailable(reason) => match self.policy.mode {
                Mode::Enforce => Err(SandboxError::Unavailable(reason.clone())),
                _ => {
                    // auto (or off, though off never wires DockerSandbox): degrade to host.
                    tracing::warn!(target: "sandbox", reason=%reason,
                        "docker unavailable; degrading to host execution");
                    agent_tools::HostExecutor.launch(spec)
                }
            },
        }
    }

    fn describe(&self) -> SandboxDescriptor {
        let degraded = match &self.available {
            Availability::Unavailable(r) => Some(r.clone()),
            Availability::Available => None,
        };
        SandboxDescriptor { mode: self.policy.mode, mechanism: "docker",
            image: Some(self.policy.image.clone()), network: self.policy.network, degraded }
    }
}
```

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use agent_tools::{Limits, Mode};

    fn policy(mode: Mode) -> SandboxPolicy {
        SandboxPolicy { mode, image: "debian:stable-slim".into(), network: false,
            limits: Limits { memory: "2g".into(), cpus: "2".into(), pids: 512,
                fsize: None, tmp_size: "256m".into() },
            extra_rw: vec![], extra_ro: vec![] }
    }
    fn spec() -> CommandSpec {
        CommandSpec { program: "sh".into(), args: vec!["-c".into(), "true".into()],
            cwd: std::env::temp_dir(), env: Default::default(), kind: ProcKind::OneShot }
    }

    #[test]
    fn enforce_denies_when_unavailable() {
        let sb = DockerSandbox::new(policy(Mode::Enforce), "1000:1000".into(),
            Availability::Unavailable("no daemon".into()));
        let err = sb.launch(spec()).unwrap_err();
        assert!(matches!(err, SandboxError::Unavailable(_)));
        assert!(sb.describe().degraded.is_some());
    }

    #[tokio::test]
    async fn auto_degrades_to_host_when_unavailable() {
        let sb = DockerSandbox::new(policy(Mode::Auto), "1000:1000".into(),
            Availability::Unavailable("no daemon".into()));
        // Host fallback actually runs `sh -c true`.
        let mut child = sb.launch(spec()).expect("auto degrades, does not error");
        let status = child.wait().await.unwrap();
        assert!(status.success());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-sandbox strategy`
Expected: FAIL — `DockerSandbox` / `Availability` missing.

- [ ] **Step 3: Write minimal implementation**

Create `strategy.rs` as above; export from `lib.rs`:

```rust
mod strategy;
pub use strategy::{Availability, DockerSandbox};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-sandbox && cargo clippy -p agent-sandbox --all-targets -- -D warnings`
Expected: PASS (all agent-sandbox tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-sandbox/src/strategy.rs agent/crates/agent-sandbox/src/lib.rs
git commit -m "feat(sandbox): DockerSandbox with probe, mode logic (enforce=deny, auto=degrade), launch/kill"
```

---

### Task 7: `#[ignore]`-gated real-Docker escape-attempt tests

**Files:**
- Create: `agent/crates/agent-sandbox/tests/escape.rs`

**Interfaces:**
- Consumes: `DockerSandbox`, `Availability`, `SandboxPolicy` (public API). Drives real `docker run`.

These run only on demand (a daemon + the `debian:stable-slim` image must be present): `cargo test -p agent-sandbox -- --ignored`.

```rust
// agent-sandbox/tests/escape.rs
use agent_sandbox::{Availability, DockerSandbox, SandboxPolicy};
use agent_tools::{CommandSpec, Limits, Mode, ProcKind, SandboxStrategy};
use tokio::io::AsyncReadExt;

fn policy(network: bool) -> SandboxPolicy {
    SandboxPolicy { mode: Mode::Enforce, image: "debian:stable-slim".into(), network,
        limits: Limits { memory: "256m".into(), cpus: "1".into(), pids: 128,
            fsize: None, tmp_size: "64m".into() },
        extra_rw: vec![], extra_ro: vec![] }
}
fn cmd(c: &str, ws: std::path::PathBuf) -> CommandSpec {
    CommandSpec { program: "sh".into(), args: vec!["-c".into(), c.into()],
        cwd: ws, env: Default::default(), kind: ProcKind::OneShot }
}
async fn run(network: bool, ws: std::path::PathBuf, c: &str) -> (i32, String) {
    let sb = DockerSandbox::new(policy(network), "0:0".into(), Availability::Available);
    let mut child = sb.launch(cmd(c, ws)).unwrap();
    let mut out = child.take_stdout().unwrap();
    let mut s = String::new(); out.read_to_string(&mut s).await.unwrap();
    let code = child.wait().await.unwrap().code().unwrap_or(-1);
    (code, s)
}

#[tokio::test] #[ignore]
async fn cannot_read_etc_shadow() {
    let (code, _) = run(false, std::env::temp_dir(), "cat /etc/shadow").await;
    assert_ne!(code, 0); // unreadable as non-... here root-in-container but file empty on slim; assert blocked or empty
}

#[tokio::test] #[ignore]
async fn rootfs_is_read_only() {
    let (code, _) = run(false, std::env::temp_dir(), "echo x > /etc/escape").await;
    assert_ne!(code, 0);
}

#[tokio::test] #[ignore]
async fn network_is_off_by_default() {
    let (code, _) = run(false, std::env::temp_dir(),
        "getent hosts example.com || exit 7").await;
    assert_ne!(code, 0);
}

#[tokio::test] #[ignore]
async fn workspace_is_writable_and_host_owned() {
    let ws = tempfile::tempdir().unwrap();
    let uid = format!("{}:{}", users_uid(), users_gid());
    let sb = DockerSandbox::new(
        SandboxPolicy { mode: Mode::Enforce, image: "debian:stable-slim".into(),
            network: false, limits: policy(false).limits, extra_rw: vec![], extra_ro: vec![] },
        uid, Availability::Available);
    let mut child = sb.launch(cmd("touch /workspace/made_in_container", ws.path().into())).unwrap();
    assert!(child.wait().await.unwrap().success());
    let meta = std::fs::metadata(ws.path().join("made_in_container")).unwrap();
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(meta.uid(), users_uid());
    }
}

#[cfg(unix)] fn users_uid() -> u32 { unsafe { libc_getuid() } }
#[cfg(unix)] fn users_gid() -> u32 { unsafe { libc_getgid() } }
// Use the `nix` crate if already a dev-dep, else read from std::env or a tiny extern.
```

NOTE for the implementer: prefer the `nix` crate (`nix::unistd::{getuid,getgid}`) for uid/gid in the test if it is already in the workspace lock; otherwise gate the ownership assertion behind a helper that shells out to `id -u`. Keep the `cannot_read_etc_shadow` assertion as "blocked-or-empty" since `debian:stable-slim` runs as root-in-container by default unless `--user` is non-root; the load-bearing isolation proofs are `rootfs_is_read_only`, `network_is_off_by_default`, and `workspace_is_writable_and_host_owned`.

- [ ] **Step 1: Write the tests** (above) as `tests/escape.rs`.
- [ ] **Step 2: Confirm they are skipped by default**

Run: `cd agent && cargo test -p agent-sandbox`
Expected: the escape tests show as `ignored`; the suite passes.

- [ ] **Step 3: (Optional, environment-dependent) run them**

Run: `cd agent && cargo test -p agent-sandbox -- --ignored` (only where Docker + image exist)
Expected: PASS — confinement holds.

- [ ] **Step 4: Commit**

```bash
git add agent/crates/agent-sandbox/tests/escape.rs
git commit -m "test(sandbox): ignore-gated real-docker escape-attempt proofs"
```

---

### Task 8: `RuntimeConfig` sandbox fields + `validate` + serde defaults + round-trip

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (`RuntimeConfig`, `PartialRuntimeConfig`, defaults, `validate`, merge)

**Interfaces:**
- Produces: `RuntimeConfig.sandbox_mode/sandbox_image/sandbox_network/sandbox_memory/sandbox_cpus/sandbox_pids/sandbox_fsize/sandbox_tmp_size/sandbox_extra_rw/sandbox_extra_ro`. Consumed by Task 9.

Add fields (each `#[serde(default = ...)]`), matching the existing default-fn pattern in this file:

```rust
#[serde(default = "default_sandbox_mode")]
pub sandbox_mode: String,            // "off" | "auto" | "enforce"
#[serde(default = "default_sandbox_image")]
pub sandbox_image: String,
#[serde(default)]
pub sandbox_network: bool,
#[serde(default = "default_sandbox_memory")]
pub sandbox_memory: String,
#[serde(default = "default_sandbox_cpus")]
pub sandbox_cpus: String,
#[serde(default = "default_sandbox_pids")]
pub sandbox_pids: u32,
#[serde(default)]
pub sandbox_fsize: Option<String>,
#[serde(default = "default_sandbox_tmp_size")]
pub sandbox_tmp_size: String,
#[serde(default)]
pub sandbox_extra_rw: Vec<String>,
#[serde(default)]
pub sandbox_extra_ro: Vec<String>,
```

```rust
fn default_sandbox_mode() -> String { "auto".into() }
fn default_sandbox_image() -> String { "debian:stable-slim".into() }
fn default_sandbox_memory() -> String { "2g".into() }
fn default_sandbox_cpus() -> String { "2".into() }
fn default_sandbox_pids() -> u32 { 512 }
fn default_sandbox_tmp_size() -> String { "256m".into() }
```

`validate()` gains: reject `sandbox_mode` not in `{"off","auto","enforce"}`. Add the matching `Option` fields + merge arms to `PartialRuntimeConfig` (mirror an existing field exactly), and extend the base-construction (wherever flags build the `RuntimeConfig`) to populate them from defaults.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn sandbox_defaults_and_round_trip() {
    let base = /* construct a RuntimeConfig via the existing test helper / from_flags base */;
    assert_eq!(base.sandbox_mode, "auto");
    assert_eq!(base.sandbox_image, "debian:stable-slim");
    assert!(!base.sandbox_network);
    assert_eq!(base.sandbox_pids, 512);
}

#[test]
fn validate_rejects_unknown_sandbox_mode() {
    let mut c = /* base */;
    c.sandbox_mode = "bogus".into();
    assert!(c.validate().is_err());
}

#[test]
fn old_config_file_missing_sandbox_keeps_base_defaults() {
    // A JSON file written by an older build (no sandbox_* keys) must merge to the
    // flag-derived base, not wipe to empty — same discipline as skills config.
    // Load a partial JSON {"backend":"openai", ...} and assert sandbox_mode == base.
}
```

(Use the file's existing test scaffolding for constructing a base config and for the load/merge round-trip — mirror `sampling_round_trips_and_partial_file_keeps_base`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-runtime-config sandbox`
Expected: FAIL — fields/defaults/validation missing.

- [ ] **Step 3: Write minimal implementation**

Add the fields, default fns, `validate` arm, and `PartialRuntimeConfig` mirror + merge arms.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-runtime-config && cargo clippy -p agent-runtime-config --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/runtime_config.rs
git commit -m "feat(sandbox): RuntimeConfig sandbox_* fields, defaults, validate, partial-merge"
```

---

### Task 9: `build_sandbox()` factory in `agent-runtime-config`

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (new `build_sandbox`)
- Modify: `agent/crates/agent-runtime-config/Cargo.toml` (add `agent-sandbox = { path = "../agent-sandbox" }`)

**Interfaces:**
- Consumes: `RuntimeConfig` sandbox fields (Task 8); `validate_mount`, `SandboxPolicy`, `DockerSandbox`, `Availability` (agent-sandbox); `Mode`, `Limits`, `HostExecutor`, `SandboxStrategy` (agent-tools).
- Produces: `pub fn build_sandbox(cfg: &RuntimeConfig) -> Arc<dyn SandboxStrategy>`. Consumed by both binaries (Task 10) and the MCP plumb (Task 11).

```rust
// agent-runtime-config/src/lib.rs
use agent_tools::{HostExecutor, Limits, Mode, SandboxStrategy};
use agent_sandbox::{validate_mount, Availability, DockerSandbox, SandboxPolicy};

pub fn build_sandbox(cfg: &RuntimeConfig) -> Arc<dyn SandboxStrategy> {
    let mode = match cfg.sandbox_mode.as_str() {
        "off" => return Arc::new(HostExecutor),
        "enforce" => Mode::Enforce,
        _ => Mode::Auto,
    };
    let home = dirs_home();                  // see note below
    let resolve = |list: &[String]| list.iter().filter_map(|p| {
        match validate_mount(p, home.as_deref()) {
            Ok(c) => Some(c),
            Err(e) => { tracing::warn!(target: "sandbox", "dropping mount {p}: {e}"); None }
        }
    }).collect::<Vec<_>>();

    let policy = SandboxPolicy {
        mode,
        image: cfg.sandbox_image.clone(),
        network: cfg.sandbox_network,
        limits: Limits {
            memory: cfg.sandbox_memory.clone(),
            cpus: cfg.sandbox_cpus.clone(),
            pids: cfg.sandbox_pids,
            fsize: cfg.sandbox_fsize.clone(),
            tmp_size: cfg.sandbox_tmp_size.clone(),
        },
        extra_rw: resolve(&cfg.sandbox_extra_rw),
        extra_ro: resolve(&cfg.sandbox_extra_ro),
    };
    let uid_gid = current_uid_gid();         // "uid:gid" on unix; "0:0" fallback elsewhere
    Arc::new(DockerSandbox::new(policy, uid_gid, DockerSandbox::probe()))
}
```

Implement two small helpers in the same file:
- `current_uid_gid()` — on `#[cfg(unix)]` read `/proc/self` is overkill; use the `nix` crate if present (`nix::unistd::{getuid,getgid}`), else shell out once to `id -u`/`id -g` at startup; non-unix → `"0:0"`.
- `dirs_home()` — `std::env::var_os("HOME").map(PathBuf::from)` (no new dep).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn build_sandbox_off_is_host() {
    let mut cfg = /* base */; cfg.sandbox_mode = "off".into();
    assert_eq!(build_sandbox(&cfg).describe().mechanism, "host");
}
#[test]
fn build_sandbox_auto_is_docker_descriptor() {
    let mut cfg = /* base */; cfg.sandbox_mode = "auto".into();
    let d = build_sandbox(&cfg).describe();
    assert_eq!(d.mechanism, "docker");
    assert_eq!(d.image.as_deref(), Some("debian:stable-slim"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-runtime-config build_sandbox`
Expected: FAIL — `build_sandbox` / dep missing.

- [ ] **Step 3: Write minimal implementation**

Add the dep, `build_sandbox`, and helpers.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-runtime-config && cargo clippy -p agent-runtime-config --all-targets -- -D warnings`
Expected: PASS. (`auto` descriptor test passes regardless of whether Docker is present — `describe()` reads policy, and `degraded` may be Some on a Docker-less CI box; do not assert on `degraded`.)

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/lib.rs agent/crates/agent-runtime-config/Cargo.toml
git commit -m "feat(sandbox): build_sandbox factory from RuntimeConfig (off=host, auto/enforce=docker)"
```

---

### Task 10: CLI flags + wire the strategy into both binaries' `LoopConfig`

**Files:**
- Modify: `agent/crates/agent-cli/src/main.rs` (clap flags; set `LoopConfig.sandbox`)
- Modify: `agent/crates/agent-server/src/main.rs` and/or `agent/crates/agent-server/src/runtime.rs` (same — wherever `LoopConfig` is built; `build_loop`)

**Interfaces:**
- Consumes: `build_sandbox` (Task 9); `LoopConfig.sandbox` (Task 3).
- Produces: the running agent now confines `execute_command` per config.

Add clap flags to `agent-cli` mirroring the config (defaults sourced from `RuntimeConfig` so a flag left unset inherits the config/default): `--sandbox-mode <off|auto|enforce>`, `--sandbox-image <img>`, `--sandbox-network`, `--sandbox-memory <m>`, `--sandbox-cpus <c>`, `--sandbox-pids <n>`, `--sandbox-fsize <f>`, `--sandbox-tmp-size <s>`, `--sandbox-extra-rw <path>` (repeatable), `--sandbox-extra-ro <path>` (repeatable). Fold these into the `RuntimeConfig` the binary already builds, then:

```rust
// where LoopConfig is constructed in each binary
let sandbox = agent_runtime_config::build_sandbox(&runtime_cfg);
let loop_config = LoopConfig { /* existing fields */, sandbox: Some(sandbox.clone()), ..Default::default() };
```

Keep `sandbox` (the `Arc`) in scope to also hand to the MCP manager in Task 11.

- [ ] **Step 1: Write/extend a test or smoke check**

`agent-server` has a `daemon_roundtrip`-style integration harness — extend the model-free smoke path to assert the daemon starts with `sandbox_mode = "auto"` in its config (no behavior change needed beyond construction succeeding). For `agent-cli`, add a unit test that parsing `--sandbox-mode enforce` yields `runtime_cfg.sandbox_mode == "enforce"`.

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-cli sandbox_flag` (and the server harness)
Expected: FAIL — flag not parsed / not mapped.

- [ ] **Step 3: Write minimal implementation**

Add the clap flags, map them into `RuntimeConfig`, and set `LoopConfig.sandbox = Some(build_sandbox(&cfg))` in both binaries.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-cli -p agent-server && cargo clippy -p agent-cli -p agent-server --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-cli/src/main.rs agent/crates/agent-server/src/main.rs agent/crates/agent-server/src/runtime.rs
git commit -m "feat(sandbox): --sandbox-* flags + wire strategy into agent-cli/agent-server LoopConfig"
```

---

### Task 11: Sandbox MCP stdio servers (`ProcKind::Service`)

**Files:**
- Modify: `agent/crates/agent-mcp/src/transport.rs` (`StdioTransport::spawn` takes the strategy; stores `SandboxedChild`)
- Modify: `agent/crates/agent-mcp/src/manager.rs` (`McpManager::connect` threads the strategy)
- Modify: `agent/crates/agent-mcp/Cargo.toml` (already depends on `agent-tools` — confirm)
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (`connect_mcp` accepts + passes the strategy)
- Modify: both binaries' `connect_mcp` call sites (pass the same `Arc` from Task 10)

**Interfaces:**
- Consumes: `Arc<dyn SandboxStrategy>`, `CommandSpec`, `ProcKind::Service`, `SandboxedChild` (Tasks 1–2).
- Produces: MCP servers spawned through the sandbox; `StdioTransport` holds a `SandboxedChild` and kills it on `close()`/`Drop`.

`StdioTransport::spawn` signature changes to accept the strategy and build a `Service` `CommandSpec` (instead of `tokio::process::Command` directly):

```rust
// agent-mcp/src/transport.rs
pub fn spawn(spec: &McpServerSpec, sandbox: &std::sync::Arc<dyn agent_tools::SandboxStrategy>)
    -> Result<Self, McpError> {
    let cspec = agent_tools::CommandSpec {
        program: spec.command.clone(),
        args: spec.args.clone(),
        cwd: std::env::current_dir().unwrap_or_else(|_| ".".into()),
        env: spec.env.clone().into_iter().collect(),
        kind: agent_tools::ProcKind::Service,
    };
    let mut child = sandbox.launch(cspec).map_err(|e| McpError::Io(e.to_string()))?;
    let stdin = child.take_stdin().ok_or_else(|| McpError::Io("no stdin".into()))?;
    let stdout = child.take_stdout().ok_or_else(|| McpError::Io("no stdout".into()))?;
    if let Some(stderr) = child.take_stderr() { /* same stderr-drain task as today */ }
    /* same reader task wiring */
    Ok(Self { stdin: AsyncMutex::new(stdin), inbound: AsyncMutex::new(rx),
        child: Mutex::new(Some(child)) }) // field type becomes Option<SandboxedChild>
}
```

Change `StdioTransport.child` from `Mutex<Option<Child>>` to `Mutex<Option<agent_tools::SandboxedChild>>`. In `close()` and `Drop`, replace `child.start_kill()` with the `SandboxedChild` teardown: `close()` is `async` so `if let Some(mut c) = self.child.lock().unwrap().take() { c.kill().await; }`; `Drop` relies on `SandboxedChild`'s own `Drop` backstop (just drop it).

`McpManager::connect(cfg, connect_timeout, sandbox)` and `connect_mcp(path, sandbox)` gain the parameter and pass it to `StdioTransport::spawn`. The binaries pass the `Arc` from Task 10.

- [ ] **Step 1: Update the failing test**

The existing `transport.rs` test (`StdioTransport::spawn(&cat_spec())`) must now pass a `HostExecutor`:

```rust
let sandbox: std::sync::Arc<dyn agent_tools::SandboxStrategy> =
    std::sync::Arc::new(agent_tools::HostExecutor);
let t = StdioTransport::spawn(&cat_spec(), &sandbox).expect("spawn cat");
```

Add a parity assertion: the `cat` server still echoes a JSON line round-trip (proves `Service` stdio wiring through `HostExecutor` is intact). Update `manager.rs` tests that call `connect` to pass a `HostExecutor` arc.

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-mcp`
Expected: FAIL — `spawn`/`connect` arity, `child` field type.

- [ ] **Step 3: Write minimal implementation**

Apply the transport + manager + `connect_mcp` changes and update both binaries' call sites to pass the `Arc`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-mcp -p agent-runtime-config && cargo clippy -p agent-mcp --all-targets -- -D warnings`
Expected: PASS — including the `cat` round-trip parity test.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-mcp/src/transport.rs agent/crates/agent-mcp/src/manager.rs agent/crates/agent-runtime-config/src/lib.rs agent/crates/agent-cli/src/main.rs agent/crates/agent-server/src/main.rs
git commit -m "feat(sandbox): launch MCP stdio servers through SandboxStrategy (ProcKind::Service)"
```

---

### Task 12: Approval-summary posture suffix + full-workspace gate

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (`run_tool`: append posture to the approval summary for command intents)

**Interfaces:**
- Consumes: `SandboxStrategy::describe()` (Task 1), `ToolIntent.summary`.
- Produces: the approval prompt shows the sandbox posture, e.g. `` run `npm test` (sandbox: docker, network off) ``.

In `run_tool`, before emitting the `Approval` request, when `intent.command.is_some()`, append a posture suffix derived from the active strategy's `describe()`:

```rust
let d = self.config.sandbox.as_ref()
    .map(|s| s.describe())
    .unwrap_or(agent_tools::SandboxDescriptor { mode: agent_tools::Mode::Off,
        mechanism: "host", image: None, network: true, degraded: None });
let posture = format!(" (sandbox: {}, network {})",
    d.mechanism, if d.network { "on" } else { "off" });
let mut intent = intent;            // intent is owned here
if intent.command.is_some() { intent.summary.push_str(&posture); }
```

(Keep this purely additive — no new `AgentEvent` variant, no wire/web change, per the spec.)

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn approval_summary_includes_sandbox_posture() {
    // Build an AgentLoop with a policy that returns Ask for a command, a recording
    // ApprovalChannel that captures the ApprovalRequest, and LoopConfig.sandbox =
    // Some(Arc::new(HostExecutor)). Drive one execute_command call; assert the
    // captured request.intent.summary contains "(sandbox: host, network on)".
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-core approval_summary_includes_sandbox_posture`
Expected: FAIL — summary has no posture suffix.

- [ ] **Step 3: Write minimal implementation**

Apply the posture-suffix block in `run_tool`.

- [ ] **Step 4: Run the full workspace gate**

Run:
```bash
cd agent && source "$HOME/.cargo/env" \
  && cargo test --workspace \
  && cargo clippy --all-targets -- -D warnings
```
Expected: PASS — entire workspace green, clippy clean.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs
git commit -m "feat(sandbox): show sandbox posture in the approval summary"
```

---

## Self-Review

**Spec coverage:**
- §2 seam (trait in `agent-tools`, `agent-sandbox` crate, no core dep) → Tasks 1, 4, 6.
- §2.2 wiring (`ToolCtx`, `LoopConfig`, binaries inject) → Tasks 2, 3, 10.
- §3 docker mechanics (flag set) → Task 4.
- §4 modes/failure (off/auto/enforce, degrade vs deny, kill/no-leak, image-missing surfaced) → Tasks 6, 9; teardown in Tasks 1, 11.
- §5 threat surface (hardening flags asserted incl. NOT `--privileged`; mount validation; `$HOME`/socket blocked; network default off) → Tasks 4, 5.
- §6 resource limits (memory/cpus/pids/fsize/tmp + OneShot wall-clock; Service no wall-clock) → Tasks 4 (flags), 2 (timeout retained for OneShot), 11 (Service has no timeout).
- §7 observability (tracing spans, degraded warn, approval posture suffix; no new event) → Tasks 6, 12.
- §8 config (`sandbox_*` fields, serde defaults, validate, flags; web deferred) → Tasks 8, 10.
- §9 testing (hermetic argv/mount/mode-matrix/parity + ignore-gated escape proofs + DoD gate) → Tasks 4, 5, 6, 7, 11, 12.
- §10 deferred items are not implemented (correct).

**Placeholder scan:** No "TBD"/"add error handling"-style steps; the few `/* base */` and `/* same wiring */` markers reference concrete, already-shown patterns in the named files (the existing test scaffolding and the current `transport.rs` reader tasks) — the implementer copies those verbatim. The uid/gid helper explicitly names `nix::unistd` or an `id -u` fallback.

**Type consistency:** `SandboxStrategy::launch` (sync) / `describe` consistent across Tasks 1, 3, 6, 11, 12. `SandboxedChild` methods (`take_stdin/out/err`, `wait`, `kill`) used identically in Tasks 2 and 11. `SandboxPolicy` fields match between Tasks 4, 6, 9. `Mode`/`Limits`/`CommandSpec`/`ProcKind` names stable throughout. `build_sandbox` signature matches its call sites (Tasks 9, 10, 11).
