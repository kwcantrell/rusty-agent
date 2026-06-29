#!/usr/bin/env bash
# Hidden oracle for longhaul-codename. Exit 0 == pass. The early codename
# (FALCON-9, given in the first message) must survive ~13 large user turns of
# window pressure to land as the final line of codename.txt.
set -euo pipefail
last="$(tail -n 1 codename.txt | tr -d '[:space:]')"
[ "$last" = "FALCON-9" ]
