#!/usr/bin/env bash
#
# launch-web-ui.sh — bring up the full browser-driven stack in one command:
#   1. Cloudflare Worker control plane  (wrangler dev, :8787)
#   2. React web UI                     (vite dev server, :5173+, proxies to :8787)
#   3. agent-server daemon              (dials the Worker; the agent you pair with)
#
# Usage:
#   scripts/launch-web-ui.sh [claude|local] [workspace]
#     backend   claude  -> --backend claude-cli --model sonnet         (default; uses your subscription)
#               local   -> --backend openai --base-url :8080 ...        (local llama.cpp on :8080)
#     workspace defaults to /tmp/agent-ws
#
# Ctrl-C tears down all three (and their child processes). Logs: /tmp/agent-web-ui/.
set -euo pipefail

# ---- config ---------------------------------------------------------------
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG="$ROOT/agent-server.json"
WORKER_URL="http://localhost:8787"
BOOTSTRAP_SECRET="dev-secret-change-me"
LOGDIR="/tmp/agent-web-ui"

BACKEND="${1:-claude}"
WORKSPACE="${2:-/tmp/agent-ws}"

case "$BACKEND" in
  claude) DAEMON_BACKEND=(--backend claude-cli --model sonnet) ;;
  local)  DAEMON_BACKEND=(--backend openai --base-url http://localhost:8080 \
                          --model qwen3.6-35b-a3b --context-limit 32768) ;;
  *) echo "unknown backend '$BACKEND' (use: claude | local)" >&2; exit 2 ;;
esac

# ---- process supervision --------------------------------------------------
PIDS=()
kill_tree() {  # kill a pid and all of its descendants (node/cargo/workerd children)
  local pid=$1 child
  for child in $(pgrep -P "$pid" 2>/dev/null || true); do kill_tree "$child"; done
  kill "$pid" 2>/dev/null || true
}
cleanup() {
  trap - INT TERM EXIT
  echo; echo "shutting down…"
  local pid
  for pid in "${PIDS[@]:-}"; do [[ -n "$pid" ]] && kill_tree "$pid"; done
  wait 2>/dev/null || true
}
trap cleanup INT TERM EXIT

launch() {  # launch <cwd> <logfile> <cmd...>  -> records pid, echoes it
  local cwd="$1" log="$2"; shift 2
  ( cd "$cwd" && exec "$@" ) >"$log" 2>&1 &
  local pid=$!
  PIDS+=("$pid")
  printf '%s' "$pid"
}

# ---- readiness helpers ----------------------------------------------------
http_up()  { curl -s -o /dev/null --max-time 2 "$1"; }  # exit 0 if anything answers
wait_for_http() {
  local url="$1" tries="${2:-60}" i
  for ((i = 0; i < tries; i++)); do http_up "$url" && return 0; sleep 0.5; done
  return 1
}
wait_for_vite_url() {
  local log="$1" tries="${2:-120}" i url
  for ((i = 0; i < tries; i++)); do
    url=$(grep -oE 'http://localhost:[0-9]+' "$log" 2>/dev/null | head -1 || true)
    [[ -n "$url" ]] && { printf '%s' "$url"; return 0; }
    sleep 0.5
  done
  return 1
}
read_pairing() {  # pull pairing_code out of agent-server.json without jq
  grep -oE '"pairing_code"[[:space:]]*:[[:space:]]*"[0-9]+"' "$CONFIG" 2>/dev/null \
    | grep -oE '[0-9]+' | head -1 || true
}
has_enrollment() {
  [[ -f "$CONFIG" ]] && grep -Eq '"agent_token"[[:space:]]*:[[:space:]]*"[^"]+"' "$CONFIG"
}

# ---- preflight ------------------------------------------------------------
echo "▸ preflight"
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
mkdir -p "$LOGDIR" "$WORKSPACE"

[[ -d "$ROOT/cloud/node_modules" ]] || { echo "  installing cloud deps…"; ( cd "$ROOT/cloud" && npm install ); }
[[ -d "$ROOT/web/node_modules"   ]] || { echo "  installing web deps…";   ( cd "$ROOT/web"   && npm install ); }

# .dev.vars (gitignored) must hold the bootstrap secret for enroll to succeed.
DEV_VARS="$ROOT/cloud/.dev.vars"
if ! grep -q '^BOOTSTRAP_SECRET=' "$DEV_VARS" 2>/dev/null; then
  echo "  writing $DEV_VARS"
  echo "BOOTSTRAP_SECRET=$BOOTSTRAP_SECRET" >> "$DEV_VARS"
fi

# Apply the D1 schema (idempotent — schema.sql is all IF NOT EXISTS).
echo "  applying D1 schema…"
( cd "$ROOT/cloud" && npm run db:init ) >"$LOGDIR/db-init.log" 2>&1

# Build the daemon up front so compile/link errors surface now, and so we launch
# the real binary (and own its PID) instead of the `cargo run` wrapper.
echo "▸ building agent-server (cargo build)…"
( cd "$ROOT/agent" && cargo build -p agent-server )
BIN="$ROOT/agent/target/debug/agent-serverd"

# ---- 1. Worker ------------------------------------------------------------
if http_up "$WORKER_URL"; then
  echo "▸ Worker already answering on :8787 — reusing it (not starting a second)"
else
  echo "▸ starting Worker (wrangler dev) → $LOGDIR/worker.log"
  launch "$ROOT/cloud" "$LOGDIR/worker.log" npx wrangler dev >/dev/null
  wait_for_http "$WORKER_URL" || { echo "  Worker never came up — see $LOGDIR/worker.log" >&2; exit 1; }
fi
echo "  Worker ready on $WORKER_URL"

# ---- 2. Enroll (only if needed) ------------------------------------------
if has_enrollment; then
  echo "▸ enrollment present (pairing code $(read_pairing)) — skipping enroll"
else
  echo "▸ enrolling daemon with the Worker…"
  "$BIN" --config "$CONFIG" enroll --worker-url "$WORKER_URL" --bootstrap-secret "$BOOTSTRAP_SECRET"
fi

# ---- 3. Web UI ------------------------------------------------------------
echo "▸ starting web UI (vite dev) → $LOGDIR/web.log"
launch "$ROOT/web" "$LOGDIR/web.log" npm run dev >/dev/null
WEB_URL="$(wait_for_vite_url "$LOGDIR/web.log" || true)"
[[ -z "$WEB_URL" ]] && WEB_URL="http://localhost:5173  (check $LOGDIR/web.log for the actual port)"

# ---- 4. Daemon ------------------------------------------------------------
# Run with cwd = the workspace (config/binary are absolute), so the daemon's cwd
# matches its --workspace and any relative artifact (e.g. agent-runtime.json) lands
# in the workspace, not the launch dir. Tools are scoped to --workspace regardless.
echo "▸ starting agent daemon (backend: $BACKEND) → $LOGDIR/daemon.log"
launch "$WORKSPACE" "$LOGDIR/daemon.log" \
  "$BIN" --config "$CONFIG" run "${DAEMON_BACKEND[@]}" --workspace "$WORKSPACE" >/dev/null

# ---- summary --------------------------------------------------------------
cat <<EOF

────────────────────────────────────────────────────────────
  Web UI stack is up.

  Open:          $WEB_URL
  Pairing code:  $(read_pairing)
  Worker:        $WORKER_URL
  Workspace:     $WORKSPACE
  Backend:       $BACKEND

  Logs:  $LOGDIR/{worker,web,daemon}.log
  Press Ctrl-C to stop everything.
────────────────────────────────────────────────────────────
EOF

wait
