# Context Primer — OS-Level Sandboxing

**Status:** Not started. Context primer — run `brainstorming` before implementing.
**Attaches via:** the `intent()` → `PolicyEngine`/execution boundary in `agent-tools`/`agent-policy`.
**Depends on:** agent core only.

## What it is

Hardening that goes beyond the core's logical policy engine (allowlists, path boundaries) to enforce isolation at the OS level, so a misbehaving command *cannot* escape the workspace even if policy logic has a gap. The core spec deliberately ships logical enforcement only; this primer covers the real sandbox.

## Where it fits

The core already separates **declaration** (`Tool::intent()`) from **execution** (`Tool::execute()`), and routes every call through `PolicyEngine`. Sandboxing plugs in at exactly that seam: wrap or replace the execution step (especially `execute_command` and file writes) with a confined execution mechanism. Tools shouldn't need rewriting — introduce a `SandboxedExecutor` that `execute_command` runs through.

## Key responsibilities

- Confine filesystem access to the workspace root (and explicitly granted paths).
- Restrict/disable network from sandboxed commands unless allowed.
- Limit process capabilities, env, and resource usage (CPU/mem/time).
- Clean teardown; no leaked processes.

## Proposed approach (tiered — pick per platform in brainstorming)

- **Linux, lightest:** `firejail` wrapping `execute_command` (easy, external dep).
- **Linux, native:** namespaces (mount/net/pid/user) + seccomp via a crate (e.g. tooling around `nix`/`landlock`/`seccompiler`). Landlock for filesystem confinement is a clean modern fit.
- **Containers:** run commands in a throwaway container (Docker/podman) with the workspace bind-mounted — strong isolation, heavier.
- **WASM tools:** for *pure* tools (no shell), run logic in a WASM sandbox (`wasmtime`) — relevant later for untrusted MCP tools, not shell.

Recommend designing a `SandboxStrategy` trait so the mechanism is swappable and the default can be "none" (parity with core) on unsupported platforms.

## Open questions for brainstorming

- Target platforms first (Linux-only is simplest; macOS sandboxing is different)?
- Landlock+seccomp vs containers vs firejail as the default?
- How do sandboxed commands surface stdout/stderr/timeouts back through `ToolOutput`?
- Network policy: fully off by default inside the sandbox?

## Definition of done (high level)

`execute_command` runs through a `SandboxStrategy`; a chosen mechanism demonstrably blocks out-of-workspace reads/writes and (optionally) network, with resource/time limits. Escape-attempt tests prove confinement. Graceful no-op fallback on unsupported platforms.
