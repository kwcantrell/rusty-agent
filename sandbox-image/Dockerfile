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
ARG PLAYWRIGHT_VERSION=1.61.1

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
