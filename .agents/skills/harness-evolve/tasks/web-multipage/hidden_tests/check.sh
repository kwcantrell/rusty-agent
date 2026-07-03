#!/usr/bin/env bash
# Sealed grading — runs INSIDE the node-offline container, cwd=/workspace.
set -euo pipefail
npx tsc --noEmit
cp hidden_tests/hidden.test.ts src/hidden.test.ts
npx vitest run
npx vite build
grep -rq "Plans & Pricing" dist/assets
grep -rq "Founded 2019" dist/assets
grep -rq "Usage Statistics" dist/assets
grep -rq "Acme Dashboard Home" dist/assets
echo "ALL CHECKS PASSED"
