#!/usr/bin/env bash
# Hidden oracle for offload-recall. Exit 0 == pass. The final line of found.txt
# must be exactly alpha.txt's secret (QH7-ZEBRA-4413) — recovered via context_recall
# when the realistic window offloaded alpha.txt's read out of the live context.
set -euo pipefail
last="$(tail -n 1 found.txt | tr -d '[:space:]')"
[ "$last" = "QH7-ZEBRA-4413" ]
