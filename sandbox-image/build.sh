#!/usr/bin/env bash
# Build the agent's default sandbox image (see Dockerfile header for design).
set -euo pipefail
cd "$(dirname "$0")"
docker build -t agent-sandbox-dev:latest .
echo "Built agent-sandbox-dev:latest"
