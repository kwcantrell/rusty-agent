#!/usr/bin/env bash
# Sealed grading — runs INSIDE the node-offline container, cwd=/workspace.
set -euo pipefail
npx tsc --noEmit
cp hidden_tests/hidden.test.ts src/hidden.test.ts
# Path-filter to src: vitest's default include would otherwise also collect
# hidden_tests/hidden.test.ts IN PLACE, where its ./router import can't resolve.
npx vitest run src
npx vite build
grep -rq "Plans & Pricing" dist/assets
grep -rq "Founded 2019" dist/assets
grep -rq "Usage Statistics" dist/assets
grep -rq "Acme Dashboard Home" dist/assets
echo "ALL CHECKS PASSED"
