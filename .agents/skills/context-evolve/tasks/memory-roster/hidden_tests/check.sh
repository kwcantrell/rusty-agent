#!/usr/bin/env bash
# Hidden oracle for memory-roster. Exit 0 == pass. ALL 8 stored ops codes must be
# recalled from long-term memory (a fresh session 2) and written to facts.txt.
# Under-provisioned recall (default_k=5 < 8 facts needed) surfaces only a subset
# -> incomplete -> fail. The model can't fabricate codes it never recalled.
set -euo pipefail
for c in ZK-301 QW-882 HX-457 RV-219 TP-640 MB-073 LN-915 CF-528; do
  grep -q "$c" facts.txt || { echo "missing $c"; exit 1; }
done
