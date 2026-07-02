# Sandbox Fail-Closed by Default — Design

**Date:** 2026-07-01
**Status:** Implemented (this plan: docs/superpowers/plans/2026-07-01-sandbox-fail-closed.md)
**Source:** Cluster 1 of the harness deep audit
(`docs/superpowers/audits/2026-07-01-harness-deep-audit.md`, Component 3 —
Sandboxes & Execution; Top-10 fixes #1 and #2). All five findings re-verified
against live `main` on 2026-07-01 (the audit's anchors predate the merged
observability branch; drifted lines are corrected below).

## Principle

The sandbox stops being a conditional boundary. `auto` remains the local-first
default, but a degraded sandbox **refuses** exec-capable work instead of
silently running it unconfined on the host. Host execution happens only by
explicit, configured choice (`sandbox_mode: "off"`), and even then children get
a scrubbed environment. This turns the already-merged `SandboxDegraded`
*signal* into *enforcement*, and restores the sandbox as the documented
mitigation for the accepted exec-vehicle catastrophe residual
(`agent-policy/src/command.rs:205-221`).

## Findings addressed (verified live)

| # | Sev | Where (live main) | Finding |
|---|-----|-------------------|---------|
| 1 | HIGH | `runtime_config.rs:117-119`, `lib.rs:216-220`, `strategy.rs:79-87` | default `sandbox_mode` is `"auto"`, which silently degrades to unconfined `HostExecutor` when Docker is unavailable — the common case for a local-first runtime |
| 2 | HIGH | `agent-tools/src/sandbox.rs:128-134` | `HostExecutor::launch` does `.envs(&spec.env)` with no `env_clear()`: children inherit the full parent env, so a degraded `execute_command` can read `AGENT_API_KEY` (live at `agent-cli/src/main.rs:188`, `agent-server/src/setup.rs:41`) |
| 3 | MED | `docker.rs:39-47` (limits are Docker-only) | host fallback has zero resource limits — closed here by the refusal gate (the audit's own alternative fix); no host rlimits added, see Scope decisions |
| 4 | MED | `loop_.rs:592-596` | `LoopConfig.sandbox: None` silently falls back to `Arc::new(HostExecutor)` — fail-open |
| 5 | LOW | `lib.rs:255-274`, `agent-mcp/src/transport.rs:38` | `current_uid_gid()` falls back to `"0"` (root-in-container) when `id` fails; MCP transports spawn with `cwd = current_dir()` instead of the configured workspace |

Useful context discovered during verification:

- The `SandboxDegraded` event/banner (spec
  `2026-06-30-sandbox-degraded-signal-design.md`) is merged: emitted once at
  run start (`loop_.rs:214-220`), rendered on CLI/web/desktop.
- Only two consumers reach the sandbox strategy: `execute_command`
  (`shell.rs:55`) and MCP transports (`transport.rs:43`).
- `shell.rs` already maps `SandboxError::Unavailable` → `ToolError::Denied`,
  so a refusal message flows to the model with no new plumbing.
- `manager.rs::connect` already catches per-server connect failures, warns,
  and records `connected: false` — skip-and-continue exists.

## Decisions (resolved with the user)

1. **Default posture:** keep `"auto"` as the default; a degraded sandbox
   refuses exec-capable launches. (Rejected: `enforce`-by-default — harsher
   first-run UX with no security gain over refusal; per-command Ask — consent
   fatigue, stalls headless runs; startup consent — weakest, one keystroke
   disables the boundary for a session.)
2. **MCP under degradation:** skip affected servers with a loud warning;
   session starts. (Rejected: hard-fail assembly — one dead Docker daemon
   kills every session for MCP users; sandbox-exempt MCP — weakens the
   boundary and complicates the env fix.)
3. **Host resource limits:** none. `env_clear()` + allow-list only. The
   degraded path is closed by refusal; `HostExecutor` is reachable only via
   explicit, consented `sandbox_mode: "off"`, and rlimits there are YAGNI
   (would cost a `libc` dependency and `unsafe pre_exec` code).

## Section 1 — Refusal at the strategy (finding 1)

**`agent-sandbox/src/strategy.rs`** — delete the degrade-to-host arm in
`DockerSandbox::launch`. `Mode::Auto` + `Availability::Unavailable` returns
`Err(SandboxError::Unavailable(..))` with an actionable message:

> `sandbox degraded (docker unavailable: {reason}); command refused. Start
> Docker, or set sandbox_mode="off" to accept unsandboxed execution.`

`shell.rs` maps this to `ToolError::Denied`, so the turn continues and the
model can relay the fix to the user.

**Self-healing re-probe.** `DockerSandbox.available` becomes
`RwLock<Availability>`. When a launch arrives while the cached state is
`Unavailable` **and** mode is `Auto`, re-run `probe()` once and update the
cache; if Docker has come up, the launch proceeds sandboxed. "Start Docker and
retry" works mid-session with no restart.

- `describe()` stays a cached read (never re-probes), preserving the
  degraded-signal spec's invariant that connect-time/settings reads cannot
  block on `docker`.
- The re-probe cost lands only on exec attempts made while degraded — the
  same blocking-probe class as the existing startup probe.
- `Mode::Enforce` never re-probes: probe once at startup, refuse thereafter
  (strict). Both modes fail closed; `auto` recovers automatically, `enforce`
  does not.

**Messaging update.** The merged degraded banner/CLI line says "tools run
UNSANDBOXED on the host" — false under refusal. Reword to
"exec-capable tools disabled until Docker is available" in
`agent-cli/src/render.rs` and the web `SandboxBanner` copy. The
`AgentEvent::SandboxDegraded` / `ServerEvent::SandboxDegraded` shapes are
unchanged.

## Section 2 — Env hygiene in HostExecutor (finding 2)

**`agent-tools/src/sandbox.rs`** — `HostExecutor::launch` gains
`cmd.env_clear()` followed by an explicit allow-list copied from the parent
env when present:

```
PATH, HOME, LANG, LC_ALL, TERM, TMPDIR
```

then `.envs(&spec.env)` as today, so explicitly-passed vars (e.g. configured
MCP server env) override the allow-list. `PATH` must be in the list — child
program resolution on Unix uses the child env's `PATH`. This closes the
`AGENT_API_KEY` leak on every remaining host path, including explicit `off`
mode. The Docker path is already clean and is unchanged.

## Section 3 — MCP under degradation (decision 2)

Refusal makes degraded MCP spawns fail inside `connect_one`. The existing
`manager.rs` behavior (catch, `tracing::warn!`, record `connected: false`,
continue) is kept. Changes:

- Sharpen the warn to name the sandbox refusal explicitly (the
  `SandboxError::Unavailable` text already carries the reason).
- Verify the existing `connected: false` state is visible wherever server
  status is already surfaced (settings/status panel); no new UI.

## Section 4 — Fail-closed wiring (finding 4)

**`agent-core/src/loop_.rs`** — `LoopConfig.sandbox` changes from
`Option<Arc<dyn SandboxStrategy>>` to a required
`Arc<dyn SandboxStrategy>`. The silent `unwrap_or_else(|| Arc::new(HostExecutor))`
disappears at the type level; a caller that wants host execution must say so
by constructing `HostExecutor` explicitly. Only ~4 construction sites exist
(`assemble.rs:78` + `loop_.rs` tests). `sandbox_descriptor()` simplifies from
`Option<SandboxDescriptor>` to `SandboxDescriptor` and the run-start
degraded-emit drops one level of `Option`. (The degraded-signal spec's "no
sandbox wired → no signal" case can no longer occur; explicit `off` still
yields `degraded: None` and stays silent.)

## Section 5 — Least-privilege fixes (finding 5)

- **`agent-runtime-config/src/lib.rs` `current_uid_gid()`:** on `id` failure,
  fall back to `"65534:65534"` (nobody) with a `tracing::warn!` — never
  `"0:0"`. Container commands may then hit workspace-permission errors; that
  is the correct fail direction, and the trigger (`id` itself failing on
  Unix) is vanishingly rare. The non-Unix arm also moves to
  `"65534:65534"`.
- **`agent-mcp/src/transport.rs`:** spawn with `cwd = <configured workspace>`
  instead of `std::env::current_dir()`. The workspace threads through
  `manager.connect` → `connect_one` → transport spawn; callers pass the same
  workspace the fs tools are confined to.

## Error handling & edge cases

- **Refused exec:** `ToolError::Denied` (existing mapping); the turn
  continues, the model sees the actionable message.
- **Docker starts mid-session:** next exec attempt re-probes, cache flips to
  `Available`, command runs sandboxed. The run-start `SandboxDegraded` emit
  reads the cache, so the banner keeps warning until the first exec attempt
  re-probes and refreshes it — safe over-warning that self-corrects on use.
- **Docker dies mid-session:** `docker run` fails at spawn →
  `SandboxError::LaunchFailed` → `ToolError::Failed`. Unchanged from today;
  no silent host fallback exists anymore.
- **Explicit `off` mode:** deliberate host execution; no degraded signal
  (descriptor `degraded: None`), scrubbed env per Section 2.
- **`enforce` + Docker down:** per-launch `Unavailable` errors, as today.
  Deliberately NOT failing session assembly (would require plumbing `Result`
  through `build_sandbox`/`assemble_loop`; the audit does not require it).

## Testing

- **`agent-sandbox` (strategy):** `auto` + unavailable now **errors** (replaces
  `auto_degrades_to_host_when_unavailable`); re-probe recovers when a fake
  prober flips to available; `enforce` never re-probes; `describe()` never
  re-probes. Probing is injected (e.g. a probe closure/trait on
  `DockerSandbox`) so tests don't need a real Docker daemon.
- **`agent-tools` (HostExecutor):** a planted parent env var (stand-in for
  `AGENT_API_KEY`) is NOT visible to the child; `PATH` is; `spec.env` entries
  are and override the allow-list.
- **`agent-mcp`:** a sandbox that refuses `Service` launches yields
  `connected: false` for that server and the manager still returns.
- **`agent-core` (loop):** existing degraded-fake tests updated to the
  required-`sandbox` field; `SandboxDegraded` still emitted at run start on a
  degraded descriptor.
- **`agent-runtime-config`:** uid/gid fallback never returns `"0:0"`.
- **Web (vitest):** banner copy change only; existing reducer tests stand.
- Full gate: `bash scripts/ci.sh` (fmt + clippy + cargo test + web
  typecheck/vitest) stays green.

## Config & migration

None. `off` / `auto` / `enforce` keep their names and their configured
meanings; only the degraded behavior of `auto` changes, which is the point.
Existing configs need no edits. Users who relied on silent host fallback get
an actionable error naming `sandbox_mode: "off"` as the explicit opt-out.

## Files touched

- `agent/crates/agent-sandbox/src/strategy.rs` — refusal + re-probe
  (`RwLock<Availability>`, injectable probe); tests.
- `agent/crates/agent-tools/src/sandbox.rs` — `env_clear()` + allow-list in
  `HostExecutor::launch`; tests.
- `agent/crates/agent-core/src/loop_.rs` — `LoopConfig.sandbox` required;
  `sandbox_descriptor()` non-optional; test updates.
- `agent/crates/agent-runtime-config/src/lib.rs` — `current_uid_gid()`
  nobody-fallback; `assemble.rs` construction site.
- `agent/crates/agent-mcp/src/transport.rs`, `manager.rs` — workspace cwd,
  sharpened skip warning; tests.
- `agent/crates/agent-cli/src/render.rs` — degraded line copy.
- `web/src/components/SandboxBanner.tsx` — banner copy.
