# Running the control plane locally

> **The real browser client is the React web UI — see [§4](#4-the-web-ui-subsystem-6).**
> §1–§2 bring up the cloud + daemon (needed either way); §3 is a *legacy* throwaway HTML
> harness kept only for minimal/no-build verification. For the actual product experience,
> start the cloud (§1) + daemon (§2), then jump to §4.

## 1. Start the cloud (terminal A)
cd cloud
npm install                       # applies no patches now; clean install
echo 'BOOTSTRAP_SECRET=dev-secret-change-me' > .dev.vars   # gitignored; only if missing
npm run db:init                   # apply schema.sql to local D1
npx wrangler dev                  # Worker on http://localhost:8787 (DO/D1/R2 emulated)

## 2. Enroll + run the daemon (terminal B)
cd agent
source "$HOME/.cargo/env"
cargo run -p agent-server -- --config ../agent-server.json \
  enroll --worker-url http://localhost:8787 --bootstrap-secret dev-secret-change-me
# note the printed pairing code, then:
cargo run -p agent-server -- --config ../agent-server.json \
  run --base-url http://localhost:8080 --model qwen3.6-35b-a3b \
      --workspace /tmp/agent-ws --context-limit 32768
# Or drive the daemon off your Claude subscription instead of an inference server
# (same --backend claude-cli path as the CLI — see agent/docs/RUNNING.md §1):
#   run --backend claude-cli --model sonnet --workspace /tmp/agent-ws

## 3. Open the test client (terminal C) — LEGACY throwaway harness (prefer §4)
# This single-file HTML page predates the React UI (§4) and hits the cross-origin wall
# below. Use it only for a quick no-build smoke test; otherwise skip straight to §4.
cd cloud/testpage && python3 -m http.server 8081
# browse http://localhost:8081, enter the pairing code, Pair, send a prompt.
#
# CROSS-ORIGIN NOTE: the page is served from :8081 but the Worker is on :8787,
# and the Worker sends no CORS headers, so the browser BLOCKS `fetch('/pair')`.
# Work around it without changing the Worker by driving the flow from the Worker's
# OWN origin: open http://localhost:8787/ (any path; the 404 body is irrelevant)
# and run the pair + WebSocket from that page context (same-origin -> no CORS).
# This is how the automated browser E2E is driven. The polished React frontend
# (subsystem #6) will be served same-origin / with deliberate CORS, making this moot.

## Verify
- [ ] Browser shows `[presence online=true]` once the daemon is running.
- [ ] A prompt streams tokens into the log.
- [ ] A command tool (e.g. ask it to run `echo hi > out.txt`) raises an Approval; Approve runs it in the daemon and the result streams back.
- [ ] Reload the browser, re-pair → buffered/R2 events replay.
- [ ] Stop the daemon → browser shows `[presence online=false]`.
- [ ] `npx wrangler d1 execute agent-cp --local --command "SELECT id,online FROM agents"` shows the row.
- [ ] R2 objects exist: `ls .wrangler/state/**/r2/**` (or inspect via the dashboard emulator).
- [ ] `.dev.vars` is gitignored — for a real deploy use `npx wrangler secret put BOOTSTRAP_SECRET`.

## 4. The web UI (subsystem #6)

Prereq: bring up §1 (cloud) + §2 (daemon) first so there's an agent to pair with.

> **Shortcut:** `scripts/launch-web-ui.sh [claude|local]` brings up all three (Worker +
> Vite + daemon) in one command, auto-enrolling if needed, and prints the URL + pairing
> code. Ctrl-C tears it all down. Logs in `/tmp/agent-web-ui/`. The manual steps below
> are still useful when you want each process in its own terminal.

> If a previous `wrangler dev` is still running on :8787, **stop it first** — a stale
> instance from before `web/dist` existed (or before the `assets` binding) serves the API
> but 500s the SPA. `npx wrangler dev` won't bind a busy port; kill the old one.

Dev (HMR, two processes):
- terminal A: `cd cloud && npx wrangler dev`            # API + WS on :8787
- terminal B: `cd web && npm run dev`                   # UI on :5173, proxies /enroll,/pair,/agent,/browser to :8787 (incl. the WS routes)
- browse http://localhost:5173 — same-origin via the Vite proxy (no CORS). Enter the daemon's pairing code.
- If :5173 is taken, Vite auto-increments to the next free port (5174, 5175, …) — **use the URL Vite prints in terminal B**, not a hardcoded :5173.

Production-like (single origin, served by the Worker):
- `cd web && npm run build`                             # writes web/dist (gitignored output; the committed .gitkeep keeps the dir present)
- `cd cloud && npx wrangler dev`                        # serves the SPA + API on :8787 (run_worker_first routes the API)
- browse http://localhost:8787

Deploy ships both together: `cd web && npm run build && cd ../cloud && npx wrangler deploy`.

Both flows are validated live (chrome-driven, against the real model): pair → stream tokens
→ a command tool raises an approval → Approve runs it **on the local machine** → terminal
output + diff render → reconnect-replay + presence work.
