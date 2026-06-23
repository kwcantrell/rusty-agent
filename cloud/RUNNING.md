# Running the control plane locally

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

## 3. Open the test client (terminal C)
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
