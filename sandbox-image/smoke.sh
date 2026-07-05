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
# Note: 'go' uses 'go version' (subcommand), not '--version'; handled separately.
for tool in node npm python3 gcc cmake rg fd jq git sqlite3; do
  run "version:$tool" "$tool" --version
done
run "version:go" go version
run "version:uv" uv --version
run "version:cargo" cargo --version
run "version:rustc" rustc --version

# A trivial build per language. Workspace mount is rw; caches land on /tmp.
run "build:node" node -e 'if (1 + 1 !== 2) process.exit(1)'
run "build:python" sh -c 'uv venv /tmp/v && /tmp/v/bin/python -c "print(1+1)"'
run "build:cargo" sh -c 'cargo new /tmp/hello --vcs none -q && cd /tmp/hello && cargo build -q'
run "build:go" sh -c 'mkdir -p /tmp/g && cd /tmp/g && printf "package main\nfunc main(){}\n" > main.go && go mod init g >/dev/null && go build .'
# Note: /tmp is mounted noexec by Docker --tmpfs; compile output goes to /workspace (cwd).
run "build:gcc" sh -c 'printf "int main(){return 0;}\n" > hello.c && gcc hello.c -o hello && ./hello'

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
