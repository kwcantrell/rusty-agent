#!/usr/bin/env bash
# Hidden oracle for memory-recall. Exit 0 == pass. The token stored via `remember`
# in session 1 must be recalled from long-term memory in session 2 (a fresh window)
# and written as the final line of token.txt. Under-provisioned recall (high
# relevance_threshold / low k) fails to surface it -> drift -> wrong/UNKNOWN.
set -euo pipefail
last="$(tail -n 1 token.txt | tr -d '[:space:]')"
[ "$last" = "ZX-99-QUASAR" ]
