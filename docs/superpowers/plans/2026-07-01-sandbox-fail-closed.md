# Sandbox Fail-Closed by Default — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A degraded sandbox refuses exec-capable work instead of silently running it unconfined; host children get a scrubbed env; the fail-open `LoopConfig.sandbox: None` fallback disappears.

**Architecture:** Delete the degrade-to-host arm in `DockerSandbox::launch` (auto now refuses with an actionable error, re-probing once per attempt so Docker coming up mid-session recovers). `HostExecutor` gains `env_clear()` + a six-var allow-list. `LoopConfig.sandbox` becomes a required `Arc<dyn SandboxStrategy>`. MCP transports get `cwd = workspace`; `current_uid_gid()` falls back to nobody, never root. Spec: `docs/superpowers/specs/2026-07-01-sandbox-fail-closed-design.md`.

**Tech Stack:** Rust (Cargo workspace under `agent/`, edition 2021), tokio, React/vitest under `web/`.

## Global Constraints

- Two Cargo workspaces: all `cargo` commands run from `agent/` (`source ~/.cargo/env` first if `cargo` is missing).
- Conventional commits: `type(scope): summary`.
- Refusal message copy (verbatim, used in strategy + asserted in tests): `docker unreachable ({reason}); command refused — start Docker, or set sandbox_mode="off" to accept unsandboxed execution`
- Degraded banner/CLI copy: degraded no longer means "runs unsandboxed on host"; it means "exec-capable tools are refused". CLI line and web banner must say so (exact strings in Task 4).
- Host env allow-list (verbatim): `PATH, HOME, LANG, LC_ALL, TERM, TMPDIR`.
- Final gate: `bash scripts/ci.sh` (from repo root) must pass — fmt + clippy `-D warnings` + all agent tests + web typecheck/vitest.
- Do not commit unless the task's tests pass.

---

### Task 1: HostExecutor env hygiene (`env_clear` + allow-list)

**Files:**
- Modify: `agent/crates/agent-tools/src/sandbox.rs:126-148` (`HostExecutor::launch`)
- Test: same file, `mod tests`

**Interfaces:**
- Consumes: nothing new.
- Produces: `HostExecutor::launch` behavior change only — signature unchanged (`fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError>`). Children see ONLY the allow-list vars + `spec.env`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `agent/crates/agent-tools/src/sandbox.rs` (the existing `spec()` helper takes `(program, args)`; a new helper adds env):

```rust
    fn spec_with_env(program: &str, args: &[&str], env: &[(&str, &str)]) -> CommandSpec {
        CommandSpec {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            kind: ProcKind::OneShot,
        }
    }

    async fn run_and_capture(spec: CommandSpec) -> String {
        let mut sb = HostExecutor.launch(spec).unwrap();
        let mut out = sb.take_stdout().unwrap();
        let mut buf = String::new();
        use tokio::io::AsyncReadExt;
        out.read_to_string(&mut buf).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), sb.wait())
            .await
            .unwrap()
            .unwrap();
        buf
    }

    #[tokio::test]
    async fn host_executor_does_not_leak_parent_env() {
        // Plant a secret in the parent env (edition 2021: set_var is safe).
        // Unique name so no other test can collide with it.
        std::env::set_var("AGENT_TEST_SECRET_XYZQ", "leaked");
        let out = run_and_capture(spec(
            "sh",
            &["-c", "printenv AGENT_TEST_SECRET_XYZQ || echo ABSENT"],
        ))
        .await;
        assert!(
            out.contains("ABSENT"),
            "parent env must not leak into host children, got: {out:?}"
        );
    }

    #[tokio::test]
    async fn host_executor_passes_allowlisted_path() {
        let out = run_and_capture(spec("sh", &["-c", "printenv PATH"])).await;
        assert!(!out.trim().is_empty(), "PATH must be forwarded to children");
    }

    #[tokio::test]
    async fn host_executor_spec_env_wins() {
        // spec.env entries survive the scrub and override allow-listed values.
        let out = run_and_capture(spec_with_env(
            "sh",
            &["-c", "printenv AGENT_TEST_EXPLICIT"],
            &[("AGENT_TEST_EXPLICIT", "explicit-value")],
        ))
        .await;
        assert!(out.contains("explicit-value"), "spec.env must be applied, got: {out:?}");
    }
```

- [ ] **Step 2: Run tests to verify the leak test fails**

Run: `cd agent && cargo test -p agent-tools host_executor -- --nocapture`
Expected: `host_executor_does_not_leak_parent_env` FAILS (prints `leaked`, not `ABSENT`); the other two new tests pass (current behavior already forwards PATH and spec.env).

- [ ] **Step 3: Implement the scrub**

In `HostExecutor::launch`, replace the builder chain:

```rust
/// Env vars forwarded from the parent to host-executed children. Everything
/// else is scrubbed (`env_clear`) so secrets like AGENT_API_KEY never reach
/// tool subprocesses; `spec.env` is applied afterwards and always wins.
const HOST_ENV_ALLOWLIST: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "TERM", "TMPDIR"];

impl SandboxStrategy for HostExecutor {
    fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError> {
        let mut cmd = tokio::process::Command::new(&spec.program);
        cmd.args(&spec.args)
            .current_dir(&spec.cwd)
            .env_clear()
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in HOST_ENV_ALLOWLIST {
            if let Ok(v) = std::env::var(key) {
                cmd.env(key, v);
            }
        }
        cmd.envs(&spec.env);
        // ... rest unchanged (stdin match, spawn, Ok(SandboxedChild::new_host(child)))
```

(`HOST_ENV_ALLOWLIST` goes at module scope next to `HostExecutor`. `PATH` must stay in the list — child program resolution on Unix uses the child env's `PATH`.)

- [ ] **Step 4: Run the crate's tests**

Run: `cd agent && cargo test -p agent-tools`
Expected: all PASS (including the pre-existing `host_executor_runs_and_captures_stdout` — `sh` is found via forwarded PATH).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/src/sandbox.rs
git commit -m "fix(tools): env_clear + allow-list in HostExecutor — stop leaking parent env (AGENT_API_KEY) to children"
```

---

### Task 2: DockerSandbox refuses when degraded (+ self-healing re-probe)

**Files:**
- Modify: `agent/crates/agent-sandbox/src/strategy.rs`
- Test: same file, `mod tests`

**Interfaces:**
- Consumes: `SandboxError::Unavailable` (existing; `shell.rs`/`git.rs` already map it to `ToolError::Denied`, MCP maps to `McpError::Io` — no downstream change needed).
- Produces: `DockerSandbox::new(policy, uid_gid, available)` signature UNCHANGED. New private `resolve_availability()`; `available` field becomes `RwLock<Availability>`; test-only `with_prober`. `Availability` gains `PartialEq` derive for test assertions.

- [ ] **Step 1: Write the failing tests**

Replace `auto_degrades_to_host_when_unavailable` and add re-probe tests in `mod tests`:

```rust
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    #[test]
    fn auto_refuses_when_unavailable() {
        let sb = DockerSandbox::new(
            policy(Mode::Auto),
            "1000:1000".into(),
            Availability::Unavailable("no daemon".into()),
        );
        let result = sb.launch(spec());
        let Err(SandboxError::Unavailable(msg)) = result else {
            panic!("auto + degraded must refuse, got a launch");
        };
        assert!(msg.contains("sandbox_mode"), "refusal must name the opt-out: {msg}");
        assert!(msg.contains("no daemon"), "refusal must carry the probe reason: {msg}");
    }

    #[test]
    fn auto_reprobe_updates_cache_and_message() {
        // Prober says "still down" — launch must re-probe (auto), refuse with the
        // FRESH reason, and update the cached availability describe() reads.
        let sb = DockerSandbox::new(
            policy(Mode::Auto),
            "1000:1000".into(),
            Availability::Unavailable("boot reason".into()),
        )
        .with_prober(|| Availability::Unavailable("still down".into()));
        let Err(SandboxError::Unavailable(msg)) = sb.launch(spec()) else {
            panic!("must refuse");
        };
        assert!(msg.contains("still down"), "must carry the re-probed reason: {msg}");
        assert_eq!(sb.describe().degraded.as_deref(), Some("still down"));
    }

    #[test]
    fn auto_reprobe_recovers_when_docker_comes_up() {
        let sb = DockerSandbox::new(
            policy(Mode::Auto),
            "1000:1000".into(),
            Availability::Unavailable("no daemon".into()),
        )
        .with_prober(|| Availability::Available);
        // Don't spawn a real container: assert on the resolved availability + posture.
        assert_eq!(sb.resolve_availability(), Availability::Available);
        assert!(sb.describe().degraded.is_none(), "recovery must clear the degraded posture");
    }

    #[test]
    fn enforce_never_reprobes() {
        let count = std::sync::Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let sb = DockerSandbox::new(
            policy(Mode::Enforce),
            "1000:1000".into(),
            Availability::Unavailable("no daemon".into()),
        )
        .with_prober(move || {
            c.fetch_add(1, AtomicOrdering::SeqCst);
            Availability::Available
        });
        assert!(matches!(sb.launch(spec()), Err(SandboxError::Unavailable(_))));
        assert_eq!(count.load(AtomicOrdering::SeqCst), 0, "enforce must not re-probe");
    }

    #[test]
    fn describe_never_probes() {
        let count = std::sync::Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let sb = DockerSandbox::new(
            policy(Mode::Auto),
            "1000:1000".into(),
            Availability::Unavailable("no daemon".into()),
        )
        .with_prober(move || {
            c.fetch_add(1, AtomicOrdering::SeqCst);
            Availability::Available
        });
        let _ = sb.describe();
        let _ = sb.describe();
        assert_eq!(count.load(AtomicOrdering::SeqCst), 0, "describe() must stay a cached read");
    }
```

Keep `enforce_denies_when_unavailable` as is. Delete `auto_degrades_to_host_when_unavailable` (its behavior is now forbidden). The tests reference `with_prober` and `resolve_availability` — defined in Step 3.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-sandbox`
Expected: COMPILE FAILURE (`with_prober`/`resolve_availability` don't exist). That is the failing state for this step.

- [ ] **Step 3: Implement refusal + re-probe**

In `agent/crates/agent-sandbox/src/strategy.rs`:

```rust
use std::sync::RwLock;

#[derive(Debug, Clone, PartialEq)]
pub enum Availability {
    Available,
    Unavailable(String),
}

pub struct DockerSandbox {
    policy: SandboxPolicy,
    uid_gid: String,
    /// Cached probe result. Written only by `resolve_availability` (auto-mode
    /// re-probe); read by `describe()` and `launch()`.
    available: RwLock<Availability>,
    /// Injectable so tests never need a Docker daemon. Defaults to `Self::probe`.
    prober: Box<dyn Fn() -> Availability + Send + Sync>,
}

impl DockerSandbox {
    pub fn new(policy: SandboxPolicy, uid_gid: String, available: Availability) -> Self {
        Self {
            policy,
            uid_gid,
            available: RwLock::new(available),
            prober: Box::new(Self::probe),
        }
    }

    #[cfg(test)]
    fn with_prober(mut self, p: impl Fn() -> Availability + Send + Sync + 'static) -> Self {
        self.prober = Box::new(p);
        self
    }

    /// Current availability, re-probing once in `auto` mode when the cache says
    /// unavailable — Docker may have come up since startup, and "start Docker
    /// and retry" should work without restarting the session. `Enforce` never
    /// re-probes: probe once at startup, refuse thereafter.
    fn resolve_availability(&self) -> Availability {
        let cached = self.available.read().unwrap().clone();
        match (&cached, self.policy.mode) {
            (Availability::Unavailable(_), Mode::Auto) => {
                let fresh = (self.prober)();
                *self.available.write().unwrap() = fresh.clone();
                fresh
            }
            _ => cached,
        }
    }
}
```

`probe()` and `spawn_docker()` are unchanged. `launch` and `describe` become:

```rust
impl SandboxStrategy for DockerSandbox {
    fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError> {
        let name = format!(
            "agent-sbx-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        match self.resolve_availability() {
            Availability::Available => self.spawn_docker(&spec, &name),
            Availability::Unavailable(reason) => match self.policy.mode {
                Mode::Enforce => Err(SandboxError::Unavailable(reason)),
                _ => {
                    // auto: fail closed. The old degrade-to-host arm is gone —
                    // unsandboxed execution is an explicit config choice only.
                    tracing::warn!(target: "sandbox", reason=%reason,
                        "docker unavailable; refusing exec (fail-closed)");
                    Err(SandboxError::Unavailable(format!(
                        "docker unreachable ({reason}); command refused — start Docker, \
                         or set sandbox_mode=\"off\" to accept unsandboxed execution"
                    )))
                }
            },
        }
    }

    fn describe(&self) -> SandboxDescriptor {
        let degraded = match &*self.available.read().unwrap() {
            Availability::Unavailable(r) => Some(r.clone()),
            Availability::Available => None,
        };
        SandboxDescriptor {
            mode: self.policy.mode,
            mechanism: "docker",
            image: Some(self.policy.image.clone()),
            network: self.policy.network,
            degraded,
        }
    }
}
```

(The `HostExecutor` import in the `use agent_tools::{...}` line at the top becomes unused — remove it from that import list.)

- [ ] **Step 4: Run the crate's tests**

Run: `cd agent && cargo test -p agent-sandbox`
Expected: all PASS.

- [ ] **Step 5: Check nothing else in the workspace regressed**

Run: `cd agent && cargo test -p agent-tools -p agent-sandbox && cargo clippy -p agent-sandbox -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-sandbox/src/strategy.rs
git commit -m "feat(sandbox): refuse exec when degraded — auto fails closed with self-healing re-probe"
```

---

### Task 3: `LoopConfig.sandbox` required — fail-closed at the type level

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (field, `Default`, `sandbox_descriptor`, run-start emit, `Decision::Ask` posture, gate `ToolCtx`, 4 test sites)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs:78`
- Modify: `agent/crates/agent-server/src/wire.rs:109-120` + its test
- Test: existing suites in all three crates (behavior is type-level; existing tests are the net)

**Interfaces:**
- Consumes: `agent_tools::HostExecutor`, `SandboxStrategy`, `SandboxDescriptor`.
- Produces: `LoopConfig.sandbox: Arc<dyn agent_tools::SandboxStrategy>` (no longer `Option`); `AgentLoop::sandbox_descriptor(&self) -> agent_tools::SandboxDescriptor` (no longer `Option`); `wire::sandbox_degraded_from(d: agent_tools::SandboxDescriptor) -> Option<SandboxDegraded>` (parameter no longer `Option`). Task 5's MCP work and Task 4's copy work do NOT depend on these.

- [ ] **Step 1: Change the field and `Default`**

In `agent/crates/agent-core/src/loop_.rs`, remove `#[derive(Default)]` from `LoopConfig` (line 31), change line 50 to:

```rust
    pub sandbox: std::sync::Arc<dyn agent_tools::SandboxStrategy>,
```

and add below the struct (full field list — the struct has exactly these 17 fields):

```rust
impl Default for LoopConfig {
    /// Test convenience only — production wiring (`assemble_loop` →
    /// `loop_config_from`) sets every field explicitly, `sandbox` included.
    /// The default sandbox is an explicit `HostExecutor`: the same posture
    /// `sandbox_mode: "off"` selects, never a silent fallback at gate time.
    fn default() -> Self {
        Self {
            model_limit: 0,
            max_turns: 0,
            max_retries: 0,
            temperature: 0.0,
            max_tokens: None,
            workspace: PathBuf::new(),
            tool_timeout: Duration::default(),
            stream_idle_timeout: Duration::default(),
            top_p: None,
            top_k: None,
            min_p: None,
            presence_penalty: None,
            repeat_penalty: None,
            enable_thinking: false,
            preserve_thinking: false,
            sandbox: std::sync::Arc::new(agent_tools::HostExecutor),
            max_parallel_tools: 0,
        }
    }
}
```

- [ ] **Step 2: Simplify the three consumers in `loop_.rs`**

`sandbox_descriptor` (lines 90-94) becomes:

```rust
    /// The live sandbox posture (cached; never re-probes Docker).
    pub fn sandbox_descriptor(&self) -> agent_tools::SandboxDescriptor {
        self.config.sandbox.describe()
    }
```

Run-start emit (lines 214-227) — the comment claims degraded tools "run with ambient host access", which is now false; replace block with:

```rust
        // Surface a degraded sandbox loudly at run start. While degraded,
        // exec-capable tools (execute_command, git, MCP spawns) are REFUSED
        // rather than run unconfined on the host. The per-approval posture
        // string carries this too, but a run may never hit an approval
        // prompt — emit it unconditionally, once, here.
        let d = self.sandbox_descriptor();
        if let Some(reason) = d.degraded {
            self.sink.emit(AgentEvent::SandboxDegraded {
                mechanism: d.mechanism,
                reason,
            });
        }
```

`Decision::Ask` posture (lines 546-566) — the `unwrap_or(...)` descriptor and the `unavailable->host` copy both go:

```rust
            Decision::Ask => {
                let d = self.config.sandbox.describe();
                let posture = if d.degraded.is_some() {
                    format!(" (sandbox: {} degraded; exec refused)", d.mechanism)
                } else {
                    format!(
                        " (sandbox: {}, network {})",
                        d.mechanism,
                        if d.network { "on" } else { "off" }
                    )
                };
```

Gate `ToolCtx` (lines 592-602) — the fail-open fallback goes:

```rust
        let ctx = ToolCtx {
            workspace: self.config.workspace.clone(),
            timeout: self.config.tool_timeout,
            cancel: cancel.clone(),
            sandbox: self.config.sandbox.clone(),
        };
```

- [ ] **Step 3: Fix the construction sites**

- `agent/crates/agent-runtime-config/src/assemble.rs:78`: `sandbox: Some(build_sandbox(cfg)),` → `sandbox: build_sandbox(cfg),`
- `loop_.rs` tests at lines ~1791, ~1864, ~1963, ~2446: drop the `Some(...)` wrapper, e.g. `sandbox: Some(Arc::new(DegradedFake))` → `sandbox: Arc::new(DegradedFake)`.
- Update the degraded-posture test assertions (test `degraded_posture_shows_unavailable_and_network_on`, lines ~1884, ~1976-1982): rename to `degraded_posture_shows_exec_refused`, assert the new copy:

```rust
        assert!(
            summary.contains("degraded; exec refused"),
            "summary should signal degraded fail-closed state: {summary:?}"
        );
```

(delete the old `contains("unavailable")` / `contains("network on")` assertions for this test; the non-degraded test `approval_summary_includes_sandbox_posture` keeps `"(sandbox: host, network on)"` unchanged). NOTE: that test's `DegradedFake::launch` delegates to `HostExecutor.launch` — leave it; the fake models posture, not refusal, and the command is denied by the approval anyway.

- [ ] **Step 4: Update `wire.rs`**

`agent/crates/agent-server/src/wire.rs:111-120`:

```rust
pub fn sandbox_degraded_from(d: agent_tools::SandboxDescriptor) -> Option<SandboxDegraded> {
    d.degraded.map(|reason| SandboxDegraded {
        mechanism: d.mechanism.to_string(),
        reason,
    })
}
```

Update its unit test (`sandbox_degraded_from_maps_only_when_degraded`, line ~456): drop the `Some(...)` wrappers around descriptors; if it has a `None`-input case, replace it with a non-degraded descriptor (`degraded: None`) asserting the function returns `None`. The caller at `agent-server/src/runtime.rs:175` already passes `self.current_loop().sandbox_descriptor()` verbatim and needs no edit.

- [ ] **Step 5: Run the affected crates' tests**

Run: `cd agent && cargo test -p agent-core -p agent-runtime-config -p agent-server`
Expected: all PASS. If other `LoopConfig { ..Default::default() }` sites fail to compile, they shouldn't — `Default` still exists; only explicit `sandbox: Some(...)` sites needed edits.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs agent/crates/agent-runtime-config/src/assemble.rs agent/crates/agent-server/src/wire.rs
git commit -m "refactor(core): make LoopConfig.sandbox required — remove fail-open HostExecutor fallback"
```

---

### Task 4: Degraded messaging copy (CLI line + web banner)

**Files:**
- Modify: `agent/crates/agent-cli/src/render.rs:129-135`
- Modify: `agent/crates/agent-core/src/event.rs:~85` (doc comment)
- Modify: `web/src/components/SandboxBanner.tsx`
- Test: `web/src/components/SandboxBanner.test.tsx`, existing render tests

**Interfaces:**
- Consumes: `AgentEvent::SandboxDegraded { mechanism, reason }` (shape unchanged).
- Produces: copy only. No API change.

- [ ] **Step 1: Update the web banner test first**

In `web/src/components/SandboxBanner.test.tsx`, change the copy assertion (line ~8):

```tsx
    expect(screen.getByRole("alert").textContent).toMatch(/exec-capable tools disabled/i);
```

Run: `cd web && npx vitest run src/components/SandboxBanner.test.tsx`
Expected: FAIL (component still says "unsandboxed").

- [ ] **Step 2: Update the banner copy**

In `web/src/components/SandboxBanner.tsx`, replace the `<span>` copy:

```tsx
      <span>
        ⚠ <strong>Sandbox degraded</strong> — exec-capable tools disabled until the
        sandbox is available ({info.mechanism}: {info.reason}).
      </span>
```

Run: `cd web && npx vitest run src/components/SandboxBanner.test.tsx`
Expected: PASS.

- [ ] **Step 3: Update the CLI line**

`agent/crates/agent-cli/src/render.rs:129-135`:

```rust
            AgentEvent::SandboxDegraded { mechanism, reason } => {
                let _ = writeln!(
                    out,
                    "\n\x1b[33m⚠ sandbox degraded: {mechanism} unavailable ({reason}); \
                     exec-capable tools are DISABLED until it is available\x1b[0m"
                );
            }
```

If a render test asserts the old "UNSANDBOXED" text (grep `render.rs` tests for `UNSANDBOXED`/`unsandboxed`), update it to assert `DISABLED`.

- [ ] **Step 4: Update the stale event doc comment**

`agent/crates/agent-core/src/event.rs` (~line 83-86), the `SandboxDegraded` variant doc says the run is unsandboxed; replace the doc comment with:

```rust
    /// Emitted once at run start when the configured sandbox is degraded
    /// (e.g. Docker unavailable in `auto` mode). Exec-capable tools are
    /// refused while degraded; `auto` recovers automatically once the
    /// mechanism becomes available again.
```

- [ ] **Step 5: Run both suites**

Run: `cd agent && cargo test -p agent-cli -p agent-core && cd ../web && npm test`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-cli/src/render.rs agent/crates/agent-core/src/event.rs web/src/components/SandboxBanner.tsx web/src/components/SandboxBanner.test.tsx
git commit -m "fix(ui): degraded-sandbox copy — exec disabled, not 'runs unsandboxed'"
```

---

### Task 5: MCP transports — workspace cwd + degraded-skip test

**Files:**
- Modify: `agent/crates/agent-mcp/src/transport.rs:31-41` (`StdioTransport::spawn`)
- Modify: `agent/crates/agent-mcp/src/manager.rs` (`connect`, `connect_one` — thread `workspace`)
- Modify: `agent/crates/agent-runtime-config/src/lib.rs:40-46` (`connect_mcp`)
- Modify: `agent/crates/agent-cli/src/main.rs:214` (caller)
- Modify: `agent/crates/agent-mcp/tests/live_filesystem.rs:33` (caller)
- Test: `agent/crates/agent-mcp/src/manager.rs` + `transport.rs` `mod tests`

**Interfaces:**
- Consumes: `SandboxError::Unavailable` from Task 2 (already merged by then; but this task compiles standalone regardless).
- Produces:
  - `StdioTransport::spawn(spec: &McpServerSpec, workspace: &std::path::Path, sandbox: &Arc<dyn SandboxStrategy>) -> Result<Self, McpError>`
  - `McpManager::connect(cfg: &McpServersConfig, connect_timeout: Duration, workspace: std::path::PathBuf, sandbox: Arc<dyn SandboxStrategy>) -> Self`
  - `connect_mcp(path: &Path, workspace: &Path, sandbox: Arc<dyn SandboxStrategy>) -> McpManager`

- [ ] **Step 1: Write the failing tests**

In `agent/crates/agent-mcp/src/manager.rs` `mod tests`, add (uses the new `workspace` param — compile failure is the first "fail"):

```rust
    #[tokio::test]
    async fn degraded_sandbox_skips_server_not_fatal() {
        struct RefusingSandbox;
        impl agent_tools::SandboxStrategy for RefusingSandbox {
            fn launch(
                &self,
                _spec: agent_tools::CommandSpec,
            ) -> Result<agent_tools::SandboxedChild, agent_tools::SandboxError> {
                Err(agent_tools::SandboxError::Unavailable(
                    "docker unreachable (no daemon); command refused".into(),
                ))
            }
            fn describe(&self) -> agent_tools::SandboxDescriptor {
                agent_tools::SandboxDescriptor {
                    mode: agent_tools::Mode::Auto,
                    mechanism: "docker",
                    image: None,
                    network: false,
                    degraded: Some("no daemon".into()),
                }
            }
        }

        let mut cfg = McpServersConfig::default();
        cfg.servers.insert(
            "fs".into(),
            crate::config::McpServerSpec {
                command: "cat".into(),
                args: vec![],
                env: Default::default(),
                trust: crate::config::Trust::Ask,
            },
        );
        let mgr = McpManager::connect(
            &cfg,
            Duration::from_secs(1),
            std::env::temp_dir(),
            std::sync::Arc::new(RefusingSandbox),
        )
        .await;
        assert!(mgr.tools().is_empty(), "refused server must contribute no tools");
        let line = mgr.summary_line();
        assert!(
            line.contains("fs \u{2717}") && line.contains("unavailable"),
            "skip must be recorded and name the sandbox refusal: {line}"
        );
    }
```

In `agent/crates/agent-mcp/src/transport.rs` `mod tests`, add (the probe emits JSON because `StdioTransport.recv` only surfaces JSON lines):

```rust
    #[tokio::test]
    async fn spawn_uses_workspace_as_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        let sandbox = host_sandbox();
        let spec = McpServerSpec {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                r#"printf '{"cwd":"%s"}\n' "$(pwd)""#.into(),
            ],
            env: BTreeMap::new(),
            trust: crate::config::Trust::Ask,
        };
        let t = StdioTransport::spawn(&spec, dir.path(), &sandbox).expect("spawn");
        let got = t.recv().await.expect("one JSON line");
        let cwd = std::path::PathBuf::from(got["cwd"].as_str().unwrap());
        assert_eq!(
            cwd.canonicalize().unwrap(),
            canonical,
            "MCP child must run in the configured workspace"
        );
        t.close().await;
    }
```

Also update the existing tests to the new signatures:

- Transport tests (`stdio_roundtrips…`, `close_tears_down…`, `close_is_idempotent`): `StdioTransport::spawn(&cat_spec(), std::env::temp_dir().as_path(), &sandbox)`.
- Manager tests (`empty_config_connects_nothing`, `failed_spawn_is_reported_not_fatal`): add `std::env::temp_dir()` as the new 3rd argument to `McpManager::connect`.
- Add `tempfile` to `agent-mcp` `[dev-dependencies]` if not already present (check `agent/crates/agent-mcp/Cargo.toml`; other crates in the workspace already use it — match their version spec).

- [ ] **Step 2: Verify compile failure**

Run: `cd agent && cargo test -p agent-mcp`
Expected: COMPILE FAILURE (new signatures don't exist yet).

- [ ] **Step 3: Implement the threading**

`transport.rs:31-41`:

```rust
    pub fn spawn(
        spec: &McpServerSpec,
        workspace: &std::path::Path,
        sandbox: &std::sync::Arc<dyn agent_tools::SandboxStrategy>,
    ) -> Result<Self, McpError> {
        let cspec = agent_tools::CommandSpec {
            program: spec.command.clone(),
            args: spec.args.clone(),
            // Confine the server to the same root the fs tools are confined to —
            // never the daemon's incidental current_dir.
            cwd: workspace.to_path_buf(),
            env: spec.env.clone().into_iter().collect(),
            kind: agent_tools::ProcKind::Service,
        };
```

`manager.rs` — `connect` gains `workspace: std::path::PathBuf` (3rd param, before `sandbox`), clones it into each `connect_one(&name, &spec, connect_timeout, &workspace, &sandbox)`; `connect_one` gains `workspace: &std::path::Path` and passes it to `StdioTransport::spawn(&spec_owned, &workspace_owned, &sandbox)` (clone a `PathBuf` into the async block like `spec_owned`).

`agent-runtime-config/src/lib.rs:40-46`:

```rust
pub async fn connect_mcp(
    path: &Path,
    workspace: &Path,
    sandbox: Arc<dyn SandboxStrategy>,
) -> McpManager {
    let (cfg, warning) = McpServersConfig::load_or_empty(path);
    if let Some(w) = warning {
        eprintln!("warning: {} ({}); MCP disabled", w, path.display());
    }
    McpManager::connect(&cfg, Duration::from_secs(15), workspace.to_path_buf(), sandbox).await
}
```

Callers: `agent-cli/src/main.rs:214` passes the CLI's workspace path (the same value used for `LoopParts.workspace` — find it in scope, it's the `--workspace` arg resolved earlier in `main`); `agent-mcp/tests/live_filesystem.rs:33` passes its existing temp/workspace dir.

- [ ] **Step 4: Run the tests**

Run: `cd agent && cargo test -p agent-mcp -p agent-runtime-config -p agent-cli`
Expected: all PASS (live_filesystem is `#[ignore]`d — `cargo test -p agent-mcp -- --ignored` only if a live server is configured; skip it).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-mcp/src/transport.rs agent/crates/agent-mcp/src/manager.rs agent/crates/agent-mcp/Cargo.toml agent/crates/agent-runtime-config/src/lib.rs agent/crates/agent-cli/src/main.rs agent/crates/agent-mcp/tests/live_filesystem.rs
git commit -m "fix(mcp): spawn servers in the configured workspace; degraded sandbox skips servers loudly"
```

---

### Task 6: `current_uid_gid()` — nobody fallback, never root

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/lib.rs:253-274`
- Test: same file's `mod tests`

**Interfaces:**
- Consumes: nothing new.
- Produces: `current_uid_gid()` (private, same signature); new private pure helper `uid_gid_or_nobody(uid: Option<String>, gid: Option<String>) -> String`.

- [ ] **Step 1: Write the failing test**

In the `mod tests` of `agent/crates/agent-runtime-config/src/lib.rs`:

```rust
    #[test]
    fn uid_gid_fallback_is_nobody_never_root() {
        assert_eq!(uid_gid_or_nobody(None, None), "65534:65534");
        assert_eq!(uid_gid_or_nobody(Some("1000".into()), None), "65534:65534");
        assert_eq!(uid_gid_or_nobody(None, Some("1000".into())), "65534:65534");
        assert_eq!(
            uid_gid_or_nobody(Some("1000".into()), Some("1000".into())),
            "1000:1000"
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd agent && cargo test -p agent-runtime-config uid_gid`
Expected: COMPILE FAILURE (`uid_gid_or_nobody` undefined).

- [ ] **Step 3: Implement**

Replace `current_uid_gid` (lines 253-274):

```rust
/// Return `"uid:gid"` of the current process on Unix. On any failure (or on
/// non-Unix), fall back to nobody (`65534:65534`) — NEVER `0:0`, which would
/// run container workloads as root.
fn current_uid_gid() -> String {
    #[cfg(unix)]
    {
        fn id_part(flag: &str) -> Option<String> {
            let out = std::process::Command::new("id").arg(flag).output().ok()?;
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (!s.is_empty() && out.status.success()).then_some(s)
        }
        uid_gid_or_nobody(id_part("-u"), id_part("-g"))
    }
    #[cfg(not(unix))]
    {
        "65534:65534".into()
    }
}

/// Pure fallback logic, unit-tested: any missing part degrades BOTH to nobody.
fn uid_gid_or_nobody(uid: Option<String>, gid: Option<String>) -> String {
    match (uid, gid) {
        (Some(u), Some(g)) => format!("{u}:{g}"),
        _ => {
            tracing::warn!(target: "sandbox",
                "could not determine uid/gid via `id`; container will run as nobody (65534:65534)");
            "65534:65534".into()
        }
    }
}
```

On non-Unix, `uid_gid_or_nobody` would be dead code — if clippy complains under `-D warnings`, put `uid_gid_or_nobody` under `#[cfg(any(unix, test))]`.

- [ ] **Step 4: Run tests**

Run: `cd agent && cargo test -p agent-runtime-config`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/lib.rs
git commit -m "fix(runtime-config): uid/gid probe falls back to nobody, never root-in-container"
```

---

### Task 7: Full gate + spec status

**Files:**
- Modify: `docs/superpowers/specs/2026-07-01-sandbox-fail-closed-design.md` (status line)

- [ ] **Step 1: Run the whole CI gate**

Run from repo root: `bash scripts/ci.sh`
Expected: fmt clean, clippy clean (`-D warnings`), all agent tests pass, web typecheck + vitest pass. Fix anything it flags before proceeding (formatting drift from new code is the usual suspect: `cd agent && cargo fmt --all`).

- [ ] **Step 2: Mark the spec implemented**

Change the spec's `**Status:**` line to: `Implemented (this plan: docs/superpowers/plans/2026-07-01-sandbox-fail-closed.md)`.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-07-01-sandbox-fail-closed-design.md
git commit -m "docs(spec): mark sandbox fail-closed spec implemented"
```
