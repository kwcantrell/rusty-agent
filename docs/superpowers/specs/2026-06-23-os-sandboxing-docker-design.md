# OS-Level Sandboxing via Docker — Design Spec

**Date:** 2026-06-23
**Status:** Approved design (pre-implementation). Next step: `writing-plans`.
**Subsystem:** #2 OS-level sandboxing (deferred list — flagged as the top production-hardening priority).
**Attaches via:** the `intent()` → execution boundary in `agent-tools` (and the `agent-mcp` stdio spawn path).
**Depends on:** agent core only.

## 1. Problem & goal

The core ships **logical** enforcement only: `PolicyEngine` (allow/deny lists, path boundary, a shell-meta heuristic) judges a `ToolIntent` before execution, but `execute_command` then spawns `sh -c <command>` directly on the host with `current_dir(workspace)`. A misbehaving or injected command can read `$HOME/.ssh`, exfiltrate over the network, write outside the workspace, or fork-bomb — policy logic is the only thing standing in the way, and a gap in it is a full escape.

This subsystem adds **OS-level confinement under the logical policy** (defense-in-depth, not a replacement): even if policy logic has a hole, a sandboxed command *cannot* escape the workspace, reach the network, or exhaust host resources. The chosen mechanism is **Docker** (container-per-execution).

Non-goal this slice: confining the in-process file tools (`read_file`/`write_file`/`edit_file`). They run inside the trusted agent process, not subprocesses, and keep their existing logical `resolve_in_workspace` guard. OS-confining them would mean sandboxing the agent itself — a separate, larger effort.

## 2. Architecture & the seam

A new **`agent-sandbox`** crate holds the Docker implementation. The **trait and value types live in `agent-tools`** (alongside `ToolCtx`), so there is no dependency cycle — `agent-sandbox` depends on `agent-tools`, and every consumer already depends on `agent-tools` (including `agent-mcp`, which implements `Tool`).

```
SandboxStrategy (trait, agent-tools)         ← the seam
 ├─ HostExecutor   (agent-tools, default)    ← today's behavior; no-op confinement
 └─ DockerSandbox  (agent-sandbox)           ← docker run; the real confinement
```

### 2.1 Types (in `agent-tools`)

```rust
pub enum ProcKind { OneShot, Service }       // command (reaped) | mcp server (long-lived)

pub struct CommandSpec {                      // what to run, mechanism-agnostic
    pub program: String,                      // "sh"            | mcp spec.command
    pub args: Vec<String>,                    // ["-c", command] | mcp spec.args
    pub cwd: PathBuf,                          // workspace
    pub env: BTreeMap<String, String>,
    pub kind: ProcKind,
}

pub struct SandboxedChild { /* wraps tokio::process::Child + Option<container_id> + pipes */ }
impl SandboxedChild {
    // stdin/stdout/stderr accessors (for the mcp transport),
    // wait-with-output (for execute_command),
    // async fn kill(&mut self)  -> docker kill <id> (or child.start_kill() for Host)
}
impl Drop for SandboxedChild { /* backstop docker kill of the tracked id; never leak */ }

pub struct SandboxDescriptor {                // posture, for approval text + tracing
    pub mode: Mode, pub mechanism: &'static str, pub image: Option<String>,
    pub network: bool, pub limits: Limits, pub degraded: Option<String>,
}

pub enum Mode { Off, Auto, Enforce }

#[async_trait]
pub trait SandboxStrategy: Send + Sync {
    async fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError>;
    fn describe(&self) -> SandboxDescriptor;
}
```

`SandboxError` is a small enum (`Unavailable(reason)`, `LaunchFailed(reason)`, `InvalidMount(reason)`). The strategy is constructed with its resolved policy (mode/image/network/limits/mounts); `launch` takes only the per-call `CommandSpec`.

### 2.2 Wiring (no core mechanism knowledge)

- **`ToolCtx`** gains `sandbox: Arc<dyn SandboxStrategy>`. `shell.rs` stops calling `tokio::process::Command` directly: it builds a `CommandSpec { program:"sh", args:["-c", command], cwd:workspace, kind:OneShot }`, calls `ctx.sandbox.launch(...)`, and waits-with-output under the **existing** `tokio::select!`(timeout / cancel). On timeout or cancel it calls `SandboxedChild::kill()` (which is `docker kill <id>` for Docker, replacing today's `kill_on_drop`).
- **`agent-mcp`'s `StdioTransport::spawn`** takes an injected `Arc<dyn SandboxStrategy>` and launches the server through it as `ProcKind::Service`, keeping the piped stdin/stdout/stderr and stderr-drain task it already wires. The tracked container is killed on transport `close()` / `Drop` (mirrors today's `start_kill`).
- **`LoopConfig`** and **`McpManager::connect`** receive `Arc<dyn SandboxStrategy>`. The **binaries** (`agent-cli` / `agent-server`) build the concrete strategy from `RuntimeConfig` and inject it. `agent-core` only ever sees the trait — **the core stays mechanism-agnostic.**
- **Default wiring is `HostExecutor`** (today's exact behavior), so nothing changes until sandboxing is configured on. `agent-core` does not depend on `agent-sandbox`; `agent-runtime-config` (and the binaries) do, to construct `DockerSandbox`.

## 3. Docker launch mechanics

`DockerSandbox::launch` shells out to the `docker` **CLI** (not the API socket — no new daemon-socket trust beyond what `docker` already needs; podman-CLI-compatible as a drop-in):

```
docker run --rm                              # OneShot; Service drops --rm and adds a tracked --name
  --network none | bridge                    # sandbox_network (default none)
  --memory <m> --cpus <c> --pids-limit <p> --ulimit fsize=<f>
  --read-only --tmpfs /tmp:rw,size=<t>       # immutable rootfs + scratch
  --cap-drop ALL --security-opt no-new-privileges
  --user <uid>:<gid>                         # workspace files stay host-owned; non-root in container
  -v <workspace>:/workspace -w /workspace
  [ -v <extra_rw>:<extra_rw>:rw | -v <extra_ro>:<extra_ro>:ro ]   # strict grants; $HOME never mounted
  -i                                         # Service only: keep stdin open for the mcp pipe
  <image> sh -c <command>   |   <image> <program> <args...>
```

The default seccomp/AppArmor profile stays **on**. We never pass `--privileged`, `--security-opt seccomp=unconfined`, `--cap-add`, or mount the docker socket.

## 4. Modes & failure behavior

`DockerSandbox` runs a **one-time availability probe** at construction (`docker version --format '{{.Server.Version}}'`, short timeout) and caches the result.

| Mode | Docker available | Docker unavailable / `launch` fails |
|---|---|---|
| `off` | Never sandboxes — `HostExecutor` is wired instead | same |
| `auto` (**default**) | Containerize every launch | **Warn-and-degrade**: emit a degradation event/log naming the cause, run on host |
| `enforce` | Containerize every launch | **Fail-closed**: `SandboxError` → `ToolError::Denied`; the command never runs on the host |

Failure modes and handling:

- **Probe unavailable + `enforce`** — construction still succeeds; every `launch` returns `Denied` with a clear message (`"sandbox enforce: Docker daemon unreachable"`). The agent stays up; commands are refused, never silently un-sandboxed.
- **Image missing** — first `docker run` errors with an unmistakable no-such-image message. We do **not** auto-`docker pull` (network/latency/supply-chain surprise); the error tells the user to pull it. `auto` degrades-with-warning, `enforce` denies. Documented.
- **Container outlives the request** (timeout/cancel) — `SandboxedChild::kill()` issues `docker kill <id>`; `Drop` is a backstop `docker kill` for the tracked id so nothing leaks even on panic. `--rm` reaps OneShot containers; Service (MCP) containers die on transport `close()` / `Drop`.
- **`docker run` startup hang** — the existing per-call `tokio::time::timeout` wraps launch+wait; a hung `docker run` trips it and is killed.
- **Degradation is never silent** — every `auto` host-fallback emits a warning on both the tracing channel and the approval/result text (see §7).

## 5. Threat surface

- **Docker daemon = root; a container escape is host-root.** We shrink the blast radius with `--cap-drop ALL`, `--security-opt no-new-privileges`, `--read-only` rootfs, `--pids-limit`, `--memory`, and `--user <uid>:<gid>` (non-root in-container, no setuid gain); the default seccomp/AppArmor profile stays on. The spec **recommends rootless Docker or podman** for users who want escape ≠ host-root, and documents it; the CLI shells out identically.
- **Docker socket / `docker`-run privilege** is a pre-existing host trust (user in the `docker` group, or rootless). We add no new socket exposure: we never mount `/var/run/docker.sock`, and we reject any `extra_rw`/`extra_ro` that resolves to the docker socket or its directory.
- **Mount-grant validation.** `extra_rw`/`extra_ro` paths are canonicalized and rejected if they resolve to `$HOME` root, `/`, or the docker socket/dir. `$HOME` is never auto-mounted. The workspace is mounted at a fixed `/workspace` (no host path leaks into the container's view).
- **Network.** Default `--network none`: no DNS, no egress, no metadata-IP reachability — strictly stronger than the http-tool SSRF floor *for shell commands*. `sandbox_network=true` opts into `bridge`; the approval prompt and tool-start text surface the posture (`network: on`).
- **Logical policy still runs first.** The model's command still passes through `PolicyEngine` (allow/deny/shell-meta) before any container starts. The sandbox is defense-in-depth *under* policy, not a replacement.
- **Image trust.** The configured image is user-trusted code and a supply-chain surface; the spec recommends pinning by digest and documents this.

## 6. Resource limits

| Limit | Mechanism | Default | Applies to |
|---|---|---|---|
| Memory | `--memory` (+ implicit swap cap) | `2g` | both |
| CPU | `--cpus` | `2` | both |
| PIDs (fork-bomb) | `--pids-limit` | `512` | both |
| File size | `--ulimit fsize=` | configurable | both |
| `/tmp` size | `--tmpfs /tmp:size=` | `256m` | both |
| Wall-clock | existing `ctx.timeout` + `docker kill` | `LoopConfig.tool_timeout` | **OneShot only** |

MCP **Service** containers are long-lived daemons: memory/CPU/PIDs/FS confinement, but **no wall-clock timeout and no CPU-time rlimit**. This is the lifetime distinction made concrete.

## 7. Observability

`SandboxDescriptor` (`{ mode, mechanism, image, network, limits, degraded }`) is the single posture object. v1 surfaces it without touching the core event enum / wire / web:

- **`tracing` structured spans on every launch** (the always-on, load-bearing channel): `mode`, `image`, `network`, mounts, limits, `container_id`, `exit_code`, `duration_ms`, and an explicit `degraded` warn-level event whenever `auto` falls back to host.
- **The approval summary string** gains a posture suffix (e.g. `` run `npm test` (sandbox: docker, network off) ``), reusing the existing `ToolIntent.summary`. **No new `AgentEvent` variant, no wire/web change this slice.**

A structured `AgentEvent::SandboxNotice` for the SPA is a documented follow-up that pairs with the deferred Settings-panel wiring.

## 8. Configuration surface

New `RuntimeConfig` fields, each `#[serde(default)]` so an older on-disk config or a browser settings round-trip can never silently wipe them (same discipline as the skills-runtime-config cycle):

```rust
pub sandbox_mode: String,            // "off" | "auto" | "enforce"   (default "auto")
pub sandbox_image: String,           // documented default dev image; user-overridable
pub sandbox_network: bool,           // default false
pub sandbox_memory: String,          // "2g"
pub sandbox_cpus: String,            // "2"
pub sandbox_pids: u32,               // 512
pub sandbox_fsize: Option<String>,   // ulimit fsize
pub sandbox_tmp_size: String,        // "256m"
pub sandbox_extra_rw: Vec<String>,   // e.g. "~/.cargo"  (validated per §5)
pub sandbox_extra_ro: Vec<String>,
```

CLI flags mirror these (`--sandbox-mode`, `--sandbox-image`, `--sandbox-network`, `--sandbox-memory`, …) on both `agent-cli` and `agent-server`. `validate()` rejects an unknown `sandbox_mode`. Settings-panel/web round-trip is **deferred** (flags + disk only this slice), so a browser save cannot wipe an un-round-tripped field — exactly the skills-config precedent.

## 9. Testing & Definition of Done

**Hermetic unit tests (no Docker required):**
- `CommandSpec` → `docker run` argv builder asserts the exact flag vector for each `{mode × network × limits × mounts × OneShot/Service}` combination.
- Mount-grant validation rejects `$HOME` / `/` / docker-socket; accepts a normal extra dir.
- Mode matrix (`off`/`auto`/`enforce` × available/unavailable) drives the right Allow / Deny / degrade outcome via a **mock probe + mock launcher** (the `SandboxStrategy` trait lets `agent-core` and `shell.rs` tests inject a fake).
- `HostExecutor` parity: `execute_command` through the default strategy behaves exactly as today (the existing shell tests pass unchanged against it).

**`#[ignore]`-gated real-Docker tests** (run only when a daemon is present, like the existing real-CLI gated tests) — escape-attempt proofs:
- read of `/etc/shadow` / `$HOME/.ssh` is blocked;
- write outside `/workspace` fails on the read-only rootfs;
- `--network none` makes `curl` / DNS fail;
- a fork bomb hits `--pids-limit`;
- a `>memory` allocation is OOM-killed;
- workspace files written in-container are **host-uid-owned**;
- a timeout `docker kill`s the container with no leak (`docker ps` clean afterward).

**DoD:** `execute_command` and MCP servers run through `SandboxStrategy`; with Docker present in `auto`/`enforce`, out-of-workspace reads/writes and network are demonstrably blocked with resource/time limits enforced; escape-attempt tests pass; `auto` degrades-with-warning and `enforce` fails-closed when Docker is absent; `off` and non-Docker platforms are a graceful no-op (`HostExecutor`); `cargo test --workspace` + `cargo clippy --all-targets -- -D warnings` green.

## 10. Explicitly deferred (intentional)

- Settings-panel / web UI + `AgentEvent::SandboxNotice` wire round-trip (flags + disk only this slice).
- Per-approval / per-command network and path grants (global toggle this slice).
- Auto `docker pull`, devcontainer / Dockerfile detection, image-build caching.
- Landlock / seccomp / firejail strategies (the `SandboxStrategy` trait keeps them pluggable later).
- Redirect-caches-into-workspace build ergonomics (strict + config knob this slice).
- OS-confining the in-process file tools (would require sandboxing the agent process itself).
```

