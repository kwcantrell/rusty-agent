# Sandbox Dev Image (`agent-sandbox-dev`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a locally built polyglot dev Docker image (`agent-sandbox-dev:latest`) that works under the agent sandbox's hardening flags, and make it the runtime's default `sandbox_image` with a startup fallback to `debian:stable-slim`.

**Architecture:** A `sandbox-image/` directory at the repo root holds the Dockerfile, build script, and smoke test. In Rust, `agent-sandbox` gains a bounded `image_exists` probe, and `agent-runtime-config` gains a pure `resolve_sandbox_image` selection function used by `build_sandbox()`. The spec is `docs/superpowers/specs/2026-07-05-sandbox-dev-image-design.md`.

**Tech Stack:** Docker (Ubuntu 24.04 base, multi-stage), Rust (two crates in the `agent/` workspace), bash.

## Global Constraints

- The image MUST work under the exact flags `agent-sandbox` emits (`agent/crates/agent-sandbox/src/docker.rs`): `--read-only` rootfs, arbitrary `--user uid:gid` with no passwd entry, `--cap-drop ALL`, `--security-opt no-new-privileges`, `HOME=/tmp` on a tmpfs, network possibly `none`.
- Version pins as Dockerfile `ARG`s: Node major `22`, Go `1.23`, Rust channel `stable`, Playwright pinned to the latest stable at implementation time (a step checks it).
- Fallback rule: only the **built-in default** image name falls back when missing; an explicitly configured image is never substituted.
- Default `sandbox_tmp_size` changes `256m` → `1g`. Memory/cpus/pids defaults unchanged.
- All `cargo` commands run from `agent/` (separate workspace from `src-tauri/`). If `cargo` is not on PATH: `source ~/.cargo/env`.
- Conventional commits (`type(scope): summary`).

---

### Task 1: `agent-sandbox`: bounded local-image existence probe

**Files:**
- Modify: `agent/crates/agent-sandbox/src/strategy.rs` (add method + test)

**Interfaces:**
- Consumes: existing private `DockerSandbox::wait_bounded(cmd, timeout) -> Availability` and `PROBE_TIMEOUT` (`strategy.rs:14`).
- Produces: `pub fn DockerSandbox::image_exists(image: &str) -> bool` — `true` iff `docker image inspect <image>` succeeds within the probe deadline. Task 2 calls this.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module at the bottom of `agent/crates/agent-sandbox/src/strategy.rs`:

```rust
    #[test]
    fn image_exists_is_false_for_a_missing_image() {
        // Hermetic in both environments: daemon up → inspect of a garbage tag
        // exits non-zero; daemon absent → spawn/exit failure. Never true.
        assert!(!DockerSandbox::image_exists(
            "agent-sbx-test-no-such-image:none"
        ));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-sandbox image_exists_is_false_for_a_missing_image`
Expected: COMPILE ERROR — `no function or associated item named `image_exists``

- [ ] **Step 3: Write minimal implementation**

Add to `impl DockerSandbox` in `strategy.rs`, right after the `probe()` method:

```rust
    /// Bounded check that `image` exists in the local Docker image store.
    /// `docker image inspect` exits non-zero when the image is missing — and
    /// the probe also fails when the daemon is unreachable; both map to
    /// `false`, which callers treat as "don't rely on this image".
    pub fn image_exists(image: &str) -> bool {
        let mut cmd = std::process::Command::new("docker");
        cmd.args(["image", "inspect", image])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Self::wait_bounded(cmd, PROBE_TIMEOUT) == Availability::Available
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-sandbox`
Expected: all PASS (including the new test)

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-sandbox/src/strategy.rs
git commit -m "feat(sandbox): bounded image_exists probe for local images"
```

---

### Task 2: `agent-runtime-config`: new default image with fallback + tmp_size bump

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (consts + two defaults + three test assertions)
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (`resolve_sandbox_image` + wire into `build_sandbox()` + tests)

**Interfaces:**
- Consumes: `DockerSandbox::image_exists(image: &str) -> bool` from Task 1.
- Produces: `pub const DEFAULT_SANDBOX_IMAGE: &str = "agent-sandbox-dev:latest"` and `pub const FALLBACK_SANDBOX_IMAGE: &str = "debian:stable-slim"` in `runtime_config.rs`; private `resolve_sandbox_image(configured: &str, image_exists: impl Fn(&str) -> bool) -> String` in `lib.rs`. Tasks 3–5 use the image name `agent-sandbox-dev:latest` and the build-script path `sandbox-image/build.sh` verbatim.

- [ ] **Step 1: Write the failing selection-logic test**

Add to the `tests` module in `agent/crates/agent-runtime-config/src/lib.rs`:

```rust
    #[test]
    fn resolve_sandbox_image_falls_back_only_for_missing_default() {
        use crate::runtime_config::{DEFAULT_SANDBOX_IMAGE, FALLBACK_SANDBOX_IMAGE};
        // default + present locally → default
        assert_eq!(
            resolve_sandbox_image(DEFAULT_SANDBOX_IMAGE, |_| true),
            DEFAULT_SANDBOX_IMAGE
        );
        // default + missing → fallback
        assert_eq!(
            resolve_sandbox_image(DEFAULT_SANDBOX_IMAGE, |_| false),
            FALLBACK_SANDBOX_IMAGE
        );
        // explicit + missing → kept verbatim (never silently substituted)
        assert_eq!(resolve_sandbox_image("my-img:1", |_| false), "my-img:1");
        // explicit + present → kept, and the probe must not even run
        assert_eq!(
            resolve_sandbox_image("my-img:1", |_| panic!("explicit image must not probe")),
            "my-img:1"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-runtime-config resolve_sandbox_image`
Expected: COMPILE ERROR — `DEFAULT_SANDBOX_IMAGE` / `resolve_sandbox_image` not found

- [ ] **Step 3: Implement consts and selection function**

In `agent/crates/agent-runtime-config/src/runtime_config.rs`, replace the existing `default_sandbox_image` and `default_sandbox_tmp_size` functions (`runtime_config.rs:216-218` and `:228-230`) with:

```rust
/// Built-in default sandbox image: the locally built dev image
/// (`sandbox-image/build.sh`). Falls back to [`FALLBACK_SANDBOX_IMAGE`] at
/// startup when it hasn't been built — see `resolve_sandbox_image` in lib.rs.
pub const DEFAULT_SANDBOX_IMAGE: &str = "agent-sandbox-dev:latest";
/// Substitute when the default image is absent locally (always pullable).
pub const FALLBACK_SANDBOX_IMAGE: &str = "debian:stable-slim";

fn default_sandbox_image() -> String {
    DEFAULT_SANDBOX_IMAGE.into()
}
```

```rust
fn default_sandbox_tmp_size() -> String {
    // HOME=/tmp inside the sandbox: npm/uv/cargo caches and Chromium scratch
    // all land on this tmpfs; 256m wedged real builds.
    "1g".into()
}
```

In `agent/crates/agent-runtime-config/src/lib.rs`, add above `build_sandbox()`:

```rust
/// Pick the sandbox image: the built-in default falls back to
/// [`FALLBACK_SANDBOX_IMAGE`] when it hasn't been built locally; an image the
/// user configured explicitly is NEVER substituted (a missing explicit image
/// stays a launch-time `docker run` error).
fn resolve_sandbox_image(configured: &str, image_exists: impl Fn(&str) -> bool) -> String {
    use crate::runtime_config::{DEFAULT_SANDBOX_IMAGE, FALLBACK_SANDBOX_IMAGE};
    if configured == DEFAULT_SANDBOX_IMAGE && !image_exists(configured) {
        tracing::warn!(target: "sandbox",
            "default sandbox image {DEFAULT_SANDBOX_IMAGE} not found locally; \
             falling back to {FALLBACK_SANDBOX_IMAGE} — build the dev image with \
             sandbox-image/build.sh");
        return FALLBACK_SANDBOX_IMAGE.to_string();
    }
    configured.to_string()
}
```

Wire it into `build_sandbox()` by changing the policy's image field (`lib.rs:294`):

```rust
        image: resolve_sandbox_image(&cfg.sandbox_image, DockerSandbox::image_exists),
```

- [ ] **Step 4: Update the three tests that pin the old defaults**

In `agent/crates/agent-runtime-config/src/runtime_config.rs`, test `sandbox_defaults_and_round_trip` (`:1078`):

```rust
        assert_eq!(b.sandbox_image, DEFAULT_SANDBOX_IMAGE);
        // ...
        assert_eq!(b.sandbox_tmp_size, "1g");
```

(replacing the `"debian:stable-slim"` and `"256m"` assertions; add `use` of the const or reference it as `crate::runtime_config::DEFAULT_SANDBOX_IMAGE` — the tests module already has `use super::*`, so the bare name works.)

Same file, test `old_config_file_missing_sandbox_keeps_base_defaults` (`:1141`):

```rust
        assert_eq!(loaded.sandbox_image, DEFAULT_SANDBOX_IMAGE);
```

In `agent/crates/agent-runtime-config/src/lib.rs`, test `build_sandbox_auto_is_docker_descriptor` (`:412`) currently asserts the default image name, which would now be environment-dependent (present vs. absent local image). Pin an explicit image so no fallback probe runs:

```rust
    #[test]
    fn build_sandbox_auto_is_docker_descriptor() {
        let mut cfg = base_cfg();
        cfg.sandbox_mode = "auto".into();
        // Explicit image: hermetic — resolve_sandbox_image never probes Docker
        // for a non-default name.
        cfg.sandbox_image = "explicit-img:1".into();
        let d = build_sandbox(&cfg).describe();
        assert_eq!(d.mechanism, "docker");
        assert_eq!(d.image.as_deref(), Some("explicit-img:1"));
    }
```

- [ ] **Step 5: Run the crate tests**

Run: `cd agent && cargo test -p agent-runtime-config`
Expected: all PASS

- [ ] **Step 6: Run the full workspace gate**

Run: `cd agent && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS (catches any other test pinning the old default)

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-runtime-config/src/runtime_config.rs agent/crates/agent-runtime-config/src/lib.rs
git commit -m "feat(runtime-config): default sandbox image agent-sandbox-dev with fallback; tmp_size 1g"
```

---

### Task 3: `sandbox-image/`: Dockerfile + build script

**Files:**
- Create: `sandbox-image/Dockerfile`
- Create: `sandbox-image/build.sh`

**Interfaces:**
- Consumes: image name `agent-sandbox-dev:latest` (must match `DEFAULT_SANDBOX_IMAGE` from Task 2 exactly).
- Produces: a built local image; baked ENV contract that Task 4's smoke test relies on: `PATH` includes `/opt/cargo/bin` and `/usr/local/go/bin`, `CARGO_HOME=/tmp/.cargo`, `PLAYWRIGHT_BROWSERS_PATH=/opt/ms-playwright`, `NODE_PATH=/usr/lib/node_modules`, `GOTOOLCHAIN=local`.

- [ ] **Step 1: Check the current Playwright version**

Run: `npm view playwright version`
Expected: a version string (e.g. `1.5x.y`). Use it as `PLAYWRIGHT_VERSION` in the next step.

- [ ] **Step 2: Write the Dockerfile**

Create `sandbox-image/Dockerfile` (replace `<PLAYWRIGHT_VERSION>` with the Step 1 value — this is the only substitution):

```dockerfile
# Dev sandbox image for the agent runtime.
# Design: docs/superpowers/specs/2026-07-05-sandbox-dev-image-design.md
#
# Must survive the hardening flags agent-sandbox emits (agent/crates/agent-sandbox/
# src/docker.rs): --read-only rootfs, arbitrary --user with no passwd entry,
# --cap-drop ALL, --security-opt no-new-privileges, HOME=/tmp tmpfs, network
# possibly none. Hence: toolchains at world-readable fixed paths, writable state
# redirected to /tmp, all lookup paths baked as ENV (no shell profiles).

ARG GO_VERSION=1.23
FROM golang:${GO_VERSION} AS go

FROM ubuntu:24.04
ARG NODE_MAJOR=22
ARG RUST_CHANNEL=stable
ARG PLAYWRIGHT_VERSION=<PLAYWRIGHT_VERSION>

ENV DEBIAN_FRONTEND=noninteractive

# Base system: C/C++ build tools, Python, and the CLI kit.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl wget gnupg git \
    build-essential cmake pkg-config \
    python3 python3-venv python3-pip python3-dev \
    ripgrep fd-find jq tree unzip zip xz-utils less procps file sqlite3 \
    && ln -s /usr/bin/fdfind /usr/local/bin/fd \
    && rm -rf /var/lib/apt/lists/*

# Node LTS (NodeSource) + corepack (pnpm/yarn shims).
RUN curl -fsSL https://deb.nodesource.com/setup_${NODE_MAJOR}.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && corepack enable \
    && rm -rf /var/lib/apt/lists/*

# uv: single static binary.
RUN curl -LsSf https://astral.sh/uv/install.sh \
    | env UV_INSTALL_DIR=/usr/local/bin INSTALLER_NO_MODIFY_PATH=1 sh

# Go: copied from the official image.
COPY --from=go /usr/local/go /usr/local/go

# Rust: toolchain at world-readable /opt. The build-time CARGO_HOME
# (/opt/cargo) only contributes bin/; the runtime CARGO_HOME is redirected to
# the /tmp tmpfs below so registry caches never hit the read-only rootfs.
ENV RUSTUP_HOME=/opt/rustup
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | env CARGO_HOME=/opt/cargo sh -s -- -y --no-modify-path \
        --default-toolchain ${RUST_CHANNEL} --profile default \
    && chmod -R a+rX /opt/rustup /opt/cargo

# Playwright: the npm package globally (so `require("playwright")` resolves via
# NODE_PATH and its version matches the browsers) + Chromium ONLY, at a shared
# world-readable path.
ENV PLAYWRIGHT_BROWSERS_PATH=/opt/ms-playwright
RUN apt-get update \
    && npm install -g playwright@${PLAYWRIGHT_VERSION} \
    && playwright install --with-deps chromium \
    && chmod -R a+rX /opt/ms-playwright \
    && rm -rf /var/lib/apt/lists/*

# Runtime contract: any UID, read-only rootfs, HOME on tmpfs (the sandbox sets
# HOME=/tmp; baked here too so bare `docker run` behaves the same).
# GOTOOLCHAIN=local: never try to download a different Go at run time.
ENV PATH=/opt/cargo/bin:/usr/local/go/bin:${PATH} \
    CARGO_HOME=/tmp/.cargo \
    NODE_PATH=/usr/lib/node_modules \
    GOTOOLCHAIN=local \
    HOME=/tmp
```

- [ ] **Step 3: Write the build script**

Create `sandbox-image/build.sh`:

```bash
#!/usr/bin/env bash
# Build the agent's default sandbox image (see Dockerfile header for design).
set -euo pipefail
cd "$(dirname "$0")"
docker build -t agent-sandbox-dev:latest .
echo "Built agent-sandbox-dev:latest"
```

Then: `chmod +x sandbox-image/build.sh`

- [ ] **Step 4: Build the image**

Run: `sandbox-image/build.sh`
Expected: exits 0, final line `Built agent-sandbox-dev:latest`. (First build downloads several GB; allow tens of minutes.)

- [ ] **Step 5: Quick sanity check**

Run: `docker run --rm agent-sandbox-dev:latest sh -c 'node --version && python3 --version && uv --version && cargo --version && go version && rg --version | head -1'`
Expected: six version lines, exit 0.

- [ ] **Step 6: Commit**

```bash
git add sandbox-image/Dockerfile sandbox-image/build.sh
git commit -m "feat(sandbox-image): agent-sandbox-dev Dockerfile and build script"
```

---

### Task 4: `sandbox-image/smoke.sh`: verify under the real sandbox flags

**Files:**
- Create: `sandbox-image/smoke.sh`

**Interfaces:**
- Consumes: the built `agent-sandbox-dev:latest` image and its baked ENV (Task 3); the flag set from `agent/crates/agent-sandbox/src/docker.rs` (mirrored verbatim in the script).
- Produces: a pass/fail script referenced by RUNNING.md (Task 5).

- [ ] **Step 1: Write the smoke script**

Create `sandbox-image/smoke.sh`:

```bash
#!/usr/bin/env bash
# Smoke-test agent-sandbox-dev under the EXACT hardening flags agent-sandbox
# emits (agent/crates/agent-sandbox/src/docker.rs). Every check runs with
# --network none: the image must be fully usable offline.
set -uo pipefail

IMAGE="${1:-agent-sandbox-dev:latest}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

FAILED=0
run() {
  local name="$1"; shift
  if docker run --rm \
      --network none \
      --memory 2g --cpus 2 --pids-limit 512 \
      --read-only --tmpfs /tmp:rw,size=1g \
      --cap-drop ALL --security-opt no-new-privileges \
      --user "$(id -u):$(id -g)" \
      -v "$WORK:/workspace" -w /workspace \
      -e HOME=/tmp \
      "$IMAGE" "$@" >/dev/null 2>&1; then
    echo "ok   $name"
  else
    echo "FAIL $name: $*"
    FAILED=1
  fi
}

# Every toolchain reports a version.
for tool in node npm python3 go gcc cmake rg fd jq git sqlite3; do
  run "version:$tool" "$tool" --version
done
run "version:uv" uv --version
run "version:cargo" cargo --version
run "version:rustc" rustc --version

# A trivial build per language. Workspace mount is rw; caches land on /tmp.
run "build:node" node -e 'if (1 + 1 !== 2) process.exit(1)'
run "build:python" sh -c 'uv venv /tmp/v && /tmp/v/bin/python -c "print(1+1)"'
run "build:cargo" sh -c 'cargo new /tmp/hello --vcs none -q && cd /tmp/hello && cargo build -q'
run "build:go" sh -c 'mkdir -p /tmp/g && cd /tmp/g && printf "package main\nfunc main(){}\n" > main.go && go mod init g >/dev/null && go build .'
run "build:gcc" sh -c 'printf "int main(){return 0;}\n" > hello.c && gcc hello.c -o /tmp/hello && /tmp/hello'

# Playwright launches Chromium and loads a page (data: URL — works offline).
run "playwright:chromium" node -e '
const { chromium } = require("playwright");
(async () => {
  const b = await chromium.launch();
  const p = await b.newPage();
  await p.goto("data:text/html,<title>ok</title>");
  if (await p.title() !== "ok") process.exit(1);
  await b.close();
})().catch((e) => { console.error(e); process.exit(1); });
'

exit "$FAILED"
```

Then: `chmod +x sandbox-image/smoke.sh`

- [ ] **Step 2: Run it and expect failures only if the image is broken**

Run: `sandbox-image/smoke.sh`
Expected: every line starts with `ok`, exit 0. If any `FAIL` line appears, re-run that single check without `>/dev/null 2>&1` to see the error, fix the Dockerfile, rebuild (`sandbox-image/build.sh`), re-run.

- [ ] **Step 3: Commit**

```bash
git add sandbox-image/smoke.sh
git commit -m "test(sandbox-image): smoke script under the real sandbox hardening flags"
```

---

### Task 5: Docs: RUNNING.md sandbox-image section

**Files:**
- Modify: `agent/docs/RUNNING.md` (new `## Sandbox image` section, inserted after the `## Skills`/`### Sampling & thinking flags` block at the end of the file)

**Interfaces:**
- Consumes: names/paths from Tasks 2–4 verbatim: `agent-sandbox-dev:latest`, `sandbox-image/build.sh`, `sandbox-image/smoke.sh`, config keys `sandbox_image`, `sandbox_memory`, `sandbox_tmp_size`, `sandbox_pids`.

- [ ] **Step 1: Append the section**

Add to the end of `agent/docs/RUNNING.md`:

````markdown
## Sandbox image

Sandboxed commands (`sandbox_mode` `"auto"`/`"enforce"`) run inside Docker. The
default image is `agent-sandbox-dev:latest` — a locally built polyglot dev
environment: Node 22 (+ npm/corepack), Python 3.12 + uv, Playwright + Chromium,
Rust stable, Go, gcc/cmake, and ripgrep/fd/jq/git/sqlite3. Build it once:

```bash
sandbox-image/build.sh   # ~4–5 GB image; the first build takes a while
sandbox-image/smoke.sh   # optional: verify every toolchain under the real sandbox flags
```

If the image hasn't been built, the runtime warns at startup and falls back to
`debian:stable-slim` (minimal tooling). An explicitly configured `sandbox_image`
is never substituted — if it's missing, the launch fails like any other
`docker run` error.

Notes:

- Playwright's Chromium runs without its own sandbox (`chromiumSandbox: false`
  is Playwright's default) — the Docker container is the security boundary
  (`--cap-drop ALL` would break Chromium's sandbox anyway).
- Browsers live at the baked `PLAYWRIGHT_BROWSERS_PATH=/opt/ms-playwright`,
  which is read-only at run time: `npx playwright install` inside a session
  fails loudly instead of re-downloading per session. Use the preinstalled
  Playwright (`require("playwright")` resolves globally via `NODE_PATH`).
- Heavy builds or browser work may need bigger limits: `sandbox_memory`
  (default `2g`), `sandbox_tmp_size` (default `1g` — caches live on the `/tmp`
  tmpfs because `HOME=/tmp`), `sandbox_pids` (default `512`).
````

- [ ] **Step 2: Commit**

```bash
git add agent/docs/RUNNING.md
git commit -m "docs(running): sandbox dev image build, fallback, and limits"
```

---

## Final verification

- [ ] `cd agent && cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check` — PASS
- [ ] `cd web && npm run typecheck && npm test` — PASS (nothing here should touch web; this is the CI gate)
- [ ] `bash scripts/ci.sh` — PASS
- [ ] `sandbox-image/smoke.sh` — all `ok`
- [ ] Start the CLI with a default config and confirm the startup log shows either the dev image in the sandbox descriptor or the fallback warning naming `sandbox-image/build.sh`.
