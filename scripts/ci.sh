#!/usr/bin/env bash
# Single source of truth for the CI gate — run by .githooks/pre-push and
# .github/workflows/ci.yml. src-tauri is intentionally excluded (GTK deps).
set -euo pipefail
cd "$(dirname "$0")/.."

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

echo "==> okf bundle check"
python3 scripts/test_okf_check.py
python3 scripts/okf_check.py docs/okf/agent-sdlc

echo "==> cargo fmt --check"
(cd agent && cargo fmt --all --check)

echo "==> cargo clippy -D warnings"
(cd agent && cargo clippy --workspace --all-targets -- -D warnings)

echo "==> cargo test"
(cd agent && cargo test --workspace)

echo "==> web typecheck + tests"
(cd web && npm ci --no-audit --no-fund && npm run typecheck && npx vitest run)

echo "CI gate passed."
