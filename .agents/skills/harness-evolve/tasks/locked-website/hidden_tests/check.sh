#!/usr/bin/env bash
# Sealed grading — runs INSIDE the node-offline container, cwd=/workspace.
set -euo pipefail
npx tsc --noEmit
# The agent must have authored its own spec with real assertions.
test -f src/catalog.test.ts
grep -q "expect(" src/catalog.test.ts
cp hidden_tests/hidden.test.ts src/hidden.test.ts
# Path-filter to src: vitest's default include would otherwise also collect
# hidden_tests/hidden.test.ts IN PLACE, where its relative imports can't resolve.
npx vitest run src
npx vite build
grep -rq "Orbit Books Home" dist/assets
grep -rq "Book Catalog" dist/assets
grep -rq "Contact Orbit" dist/assets
echo "ALL CHECKS PASSED"
