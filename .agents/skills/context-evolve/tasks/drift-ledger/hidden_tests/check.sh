#!/usr/bin/env bash
# Hidden oracle for drift-ledger. Exit 0 == pass. Never present in the agent's
# workspace until the harness copies it in for the sealed post-run grading step.
# The final line of ledger.txt must be exactly the running total (107) of every
# increment the agent was given across the session — nothing else on that line.
set -euo pipefail
last="$(tail -n 1 ledger.txt | tr -d '[:space:]')"
[ "$last" = "107" ]
