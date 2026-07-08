#!/usr/bin/env bash
# Single source of truth for the CI gate — run by .githooks/pre-push and
# .github/workflows/ci.yml. src-tauri runs conditionally: it needs GTK/WebKitGTK
# dev deps (absent on the GitHub runner, present on dev machines). Its fmt is
# never checked — src-tauri is hand-formatted by convention (src-tauri/AGENTS.md).
set -euo pipefail
cd "$(dirname "$0")/.."

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

echo "==> okf bundle check"
python3 scripts/test_okf_check.py
python3 scripts/okf_check.py docs/okf/agent-sdlc

echo "==> skills lint"
python3 scripts/test_skills_lint.py
python3 scripts/skills_lint.py

echo "==> cargo fmt --check"
(cd agent && cargo fmt --all --check)

echo "==> cargo clippy -D warnings"
(cd agent && cargo clippy --workspace --all-targets -- -D warnings)

echo "==> cargo test"
(cd agent && cargo test --workspace)

if command -v pkg-config >/dev/null 2>&1 \
   && pkg-config --exists gtk+-3.0 webkit2gtk-4.1 2>/dev/null; then
  echo "==> src-tauri clippy + test"
  (cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings)
  (cd src-tauri && cargo test --workspace)
else
  echo "==> src-tauri: SKIPPED (GTK/WebKitGTK dev deps not found)"
fi

echo "==> web typecheck + tests"
(cd web && npm ci --no-audit --no-fund && npm run typecheck && npx vitest run)

echo "CI gate passed."
