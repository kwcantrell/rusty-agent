#!/usr/bin/env bash
# TRUSTED, AUTHORING-TIME ONLY. Builds the offline seed the eval copies into
# each run's workspace. The agent never installs — runs are network-none.
set -euo pipefail
cd "$(dirname "$0")/seed"
npm install
echo "seed ready: $(du -sh node_modules | cut -f1) node_modules"
