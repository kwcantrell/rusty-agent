# Sandbox dev image (`agent-sandbox-dev`) — design

**Date:** 2026-07-05
**Status:** Approved design, pre-implementation

## Problem

The runtime's Docker sandbox defaults to `debian:stable-slim`, which has almost no
tooling. Sandboxed agent commands can't build or test real projects: no Node, no
Python tooling, no compilers, no browser. The agent needs a purpose-built image with
a modern polyglot dev environment that works under the sandbox's hardening flags.

## Goal

A locally built Docker image, `agent-sandbox-dev:latest`, that:

1. Ships Node, Python + uv, Playwright + Chromium, Rust, Go, C/C++ build tools, and
   modern CLI utilities.
2. Works under the exact flags `agent-sandbox` emits (`docker.rs`): `--read-only`
   rootfs, arbitrary `--user uid:gid` with no passwd entry, `--cap-drop ALL`,
   `--security-opt no-new-privileges`, `HOME=/tmp` on a tmpfs, network optionally
   `none`.
3. Becomes the runtime's default `sandbox_image`, with a startup fallback to
   `debian:stable-slim` when the image hasn't been built locally.

Non-goals: registry publishing/CI image builds (local-first, build it yourself);
non-Docker sandbox mechanisms; changing the sandbox flag set itself.

## Deliverables

### 1. `sandbox-image/Dockerfile`

Multi-stage, base `ubuntu:24.04` (Playwright's officially supported distro), final
size ~4–5 GB. Version pins live as `ARG`s at the top of the file (Node major, Go
version, Playwright version, Rust channel). Initial values: Node 22, Go 1.23, Rust
stable, Playwright latest stable at implementation time — each recorded in its ARG
so bumps are one-line diffs.

Contents:

| stack | how | where |
|-------|-----|-------|
| Node 22 LTS | NodeSource apt repo; `corepack enable` for pnpm/yarn | `/usr/bin` |
| Python 3.12 | Ubuntu system python + `python3-venv`, `python3-pip`, `python3-dev` | system |
| uv | static binary download | `/usr/local/bin/uv` |
| Playwright | `npx playwright@<pin> install --with-deps chromium` (Chromium **only**) | browsers at `/opt/ms-playwright` |
| Rust stable | rustup with `RUSTUP_HOME=/opt/rustup`, `CARGO_HOME=/opt/cargo` at build time | toolchain binaries `/opt/cargo/bin` (on PATH) |
| Go | `COPY --from=golang:<pin> /usr/local/go /usr/local/go` | `/usr/local/go/bin` on PATH |
| C/C++ | `build-essential`, `cmake`, `pkg-config` | system |
| CLI kit | git, ripgrep, fd-find (+`fd` symlink), jq, tree, curl, wget, unzip, zip, xz-utils, less, procps, file, sqlite3, ca-certificates | system |

Sandbox-constraint rules baked into the image:

- **Arbitrary UID / read-only rootfs:** every toolchain lives at a world-readable
  fixed path (`/opt/rustup`, `/opt/cargo`, `/opt/ms-playwright`, `/usr/local/go`).
  Build steps `chmod -R a+rX` the `/opt` trees. Nothing assumes a passwd entry or a
  writable install location at runtime.
- **Writable state → tmpfs:** baked `ENV CARGO_HOME=/tmp/.cargo` so cargo's registry
  cache lands on the tmpfs (the build-time `/opt/cargo` stays read-only, only its
  `bin/` is on PATH). npm, uv, and Go caches already follow `$HOME`, which the
  sandbox sets to `/tmp`.
- **Baked ENV** so any user in the container finds things without shell profiles
  (there is no login shell): `PATH` including `/opt/cargo/bin` and
  `/usr/local/go/bin`, `RUSTUP_HOME=/opt/rustup`, `CARGO_HOME=/tmp/.cargo`,
  `PLAYWRIGHT_BROWSERS_PATH=/opt/ms-playwright`.
- **Chromium under `--cap-drop ALL`:** Playwright disables Chromium's own sandbox by
  default (`chromiumSandbox: false`), so no extra capabilities are needed — the
  Docker container is the security boundary here. Documented, not worked around.

### 2. `sandbox-image/build.sh`

Builds and tags `agent-sandbox-dev:latest` from the repo Dockerfile. Thin wrapper
(`docker build -t agent-sandbox-dev:latest sandbox-image/`), exists so docs and the
fallback warning have one canonical command to name.

### 3. Runtime default + fallback (`agent-runtime-config`)

- `default_sandbox_image()` returns `"agent-sandbox-dev:latest"`.
- In `build_sandbox()` (`lib.rs`), **only when `cfg.sandbox_image` equals the
  built-in default**, probe `docker image inspect agent-sandbox-dev:latest` with a
  bounded wait (reuse the 2s-deadline pattern from `DockerSandbox::wait_bounded`).
  If the probe fails (image missing, or Docker itself down), log a warning that
  names `sandbox-image/build.sh` and substitute `debian:stable-slim` into the
  policy.
- An **explicitly configured** image never falls back: if the user set
  `sandbox_image` themselves and it's missing, `docker run` fails at launch time
  exactly as today. Silent substitution of a user-chosen image is wrong.
- The image-exists check is injectable (a closure, mirroring the existing
  `with_prober` test seam) so fallback selection is unit-testable without Docker.

### 4. Default `tmp_size` bump: `256m` → `1g`

With `HOME=/tmp`, npm/uv/cargo caches and Chromium's runtime scratch all land on the
`/tmp` tmpfs; 256m wedges real builds and browser runs. Memory (`2g`), cpus (`2`),
and pids (`512`) defaults are unchanged. Existing explicit configs are unaffected
(it's only the serde default).

### 5. `sandbox-image/smoke.sh`

Manual verification script (needs Docker + the built image; **not** part of
`cargo test`). Runs the image under the same hardening flags the sandbox emits —
`--read-only`, `--cap-drop ALL`, `--security-opt no-new-privileges`,
`--user $(id -u):$(id -g)`, `--tmpfs /tmp:rw,size=1g`, `-e HOME=/tmp`, workspace
mount at `/workspace` — and asserts:

- each toolchain reports a version (node, npm, python3, uv, cargo, rustc, go, gcc,
  cmake, rg, fd, jq, git);
- a trivial build succeeds per language in the mounted workspace (node script,
  python venv via uv, `cargo new` + build, `go build`, `gcc` hello);
- a Playwright script launches Chromium and loads a `data:` URL (works with
  `--network none`).

Exit non-zero on any failure, printing which check failed.

### 6. Docs (`agent/docs/RUNNING.md`)

New section: how to build the image, what's inside, the default-with-fallback
behavior, and which knobs to raise for heavy work (`sandbox_memory`,
`sandbox_tmp_size`, `sandbox_pids`).

## Testing

- **Unit (in `cargo test`):** fallback selection in `build_sandbox()` — default
  image present → used; default missing → `debian:stable-slim` + warning; explicit
  image missing → no fallback. Via the injectable image-exists closure.
- **Existing tests:** update the two assertions pinning `default_sandbox_image()` to
  `"debian:stable-slim"` (`runtime_config.rs`, `lib.rs`).
- **Manual:** `sandbox-image/smoke.sh` after building the image.

## Error handling

- Image missing at startup (default config): warn + fall back, session still works.
- Docker daemon down at startup: inspect probe fails → fallback string chosen, but
  launches are already refused by the existing fail-closed availability logic; no
  new behavior.
- Image missing for an explicit config: unchanged — launch-time `docker run` error
  surfaces to the tool result.

## Risks / notes

- **Staleness:** `agent-sandbox-dev:latest` is a local mutable tag; users rebuild to
  pick up new tool versions. Acceptable for local-first; registry publishing is a
  future step if ever needed.
- **Size:** ~4–5 GB accepted deliberately (full polyglot + browser).
- **Playwright version coupling:** projects inside the sandbox that pin a different
  Playwright version may want browsers the image lacks; `PLAYWRIGHT_BROWSERS_PATH`
  is baked, so `npx playwright install` inside a session would try to write to
  `/opt/ms-playwright` (read-only) and fail loudly rather than silently
  re-downloading per-session. Documented in RUNNING.md.
