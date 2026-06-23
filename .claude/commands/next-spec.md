---
description: Start a fresh brainstorm→spec session for the next deferred subsystem of this platform
argument-hint: "[subsystem: http-tool | os-sandboxing | mcp-client | memory-system | settings | a follow-up — or leave blank to choose]"
---

I'm continuing work on a local-first AI coding-agent platform. Three slices are already
built, reviewed, merged to `main`, and validated end-to-end against a real local model:
the Rust "agent core", the Cloudflare control plane (#5), and the React frontend (#6).
I want to brainstorm and spec the NEXT subsystem.

## Repo
/home/kalen/rust-agent-runtime  (git repo, on `main`)

## What already exists (don't re-derive — read the docs below)
A Cargo workspace under `agent/` of 7 trait-decoupled crates: the original 5 —
agent-tools (Tool trait, registry, fs/shell/git tools with a workspace path guard +
command allow/deny policy), agent-policy (PolicyEngine + ApprovalChannel), agent-model
(OpenAI-compatible SSE streaming client + native & prompted tool-call protocols, plus a
`ClaudeCliClient` backend that drives an authenticated Claude Code CLI as a pure text
generator — subscription-auth, prompted-only — selected via `--backend claude-cli`),
agent-core (the ReAct loop, token-windowed ContextManager, EventSink/AgentEvent model),
agent-cli (terminal renderer + approval + REPL) — plus two added by the control-plane
work: agent-server (an outbound-WebSocket *daemon* that drives AgentLoop and bridges the
core's seams over the wire) and agent-runtime-config (loop-wiring helpers shared by the
CLI and the daemon). 71 Rust tests pass, clippy `-D warnings` clean.

Beyond the Rust workspace:
- `cloud/` — the Cloudflare control plane: a Worker (`/enroll`,`/pair`,`/agent`,`/browser`)
  + an `AgentSession` Durable Object (WebSocket Hibernation API, durable SQLite seq) + D1
  (users/agents/sessions) + R2 (event log), all runnable under `wrangler dev` (wrangler 4,
  vitest-pool-workers 0.16; 11 tests). The daemon dials OUT to the Worker; Cloudflare only
  relays + records — it never runs the loop or touches the machine.
- `web/` — a React + Vite + TS + Tailwind SPA, a PURE WebSocket client of the Worker,
  served SAME-ORIGIN by the Worker via Workers static assets (so no CORS; 31 tests).

Every cross-crate seam is still a trait, and new subsystems bolt on ADDITIVELY: `EventSink`
(observe/stream), `ApprovalChannel` (gate tools through a different UI), `Tool` +
`ToolRegistry` (new capabilities), `ContextManager` (alternate context strategies). The
core crates (agent-core/model/tools/policy) were deliberately left UNMODIFIED through #5
and #6 — that "attach via the seams, don't touch the core" discipline is the showcase.

It's been driven end-to-end against a real local model (Qwen3.6-35B-A3B via llama.cpp):
the CLI loop + native tool-calling + approvals + diffs + execute_command, AND the full
browser path (pair → stream tokens → approve a command that runs on the local machine →
R2 reconnect-replay → presence) all work live. Treat all three slices as proven.

## Orient fast with the knowledge graph (then read the docs)
This repo ships a graphify knowledge graph of the whole codebase + docs in `graphify-out/`
(`graph.html` interactive map, `GRAPH_REPORT.md` audit, `graph.json` raw). Use it to find
your way around before reading files top-to-bottom:
- `/graphify query "how does the ReAct loop dispatch a tool through the policy + approval seam?"`
  — answers from the graph, citing source file:line. Ask it whatever you'd otherwise grep for.
- `/graphify explain "<symbol>"` for one node; `/graphify path "<A>" "<B>"` to trace how two
  pieces connect. Skim `GRAPH_REPORT.md` for god nodes (core abstractions) + community map.
- The graph is the map, NOT the authority — the docs below and the code are ground truth.
  `.graphifyignore` excludes the SDD task ledger (`.superpowers/sdd/`) and generated Wrangler
  artifacts (`.wrangler/`), so the graph reflects authored architecture, not build output.
- Refresh the graph with `/graphify . --update` (re-extracts only changed files) ONCE after
  ALL tasks of the subsystem are complete — not per task. Run it after the subsystem is
  finished (e.g. just before/after merge) so a single update captures the whole subsystem.

## Read these first (authoritative context)
- Deferred-subsystem primers + build order + what's already built:
                       docs/superpowers/context/README.md
- Core:                docs/superpowers/specs/2026-06-22-rust-agent-core-design.md
                       docs/superpowers/plans/2026-06-22-rust-agent-core.md
- Cloudflare control plane (#5, built): specs/2026-06-22-cloudflare-control-plane-design.md
                       + plans/2026-06-22-cloudflare-control-plane.md, and the later
                       best-practices revision (…-bestpractices-revision-design.md / -revision.md)
- React frontend (#6, built):           specs/2026-06-23-react-frontend-design.md
                       + plans/2026-06-23-react-frontend.md
- Claude CLI inference backend (built):  specs/2026-06-23-claude-cli-inference-backend-design.md
                       + plans/2026-06-23-claude-cli-inference-backend.md, spike +
                       follow-ups in context/claude-cli-inference.md
- How to run: agent/docs/RUNNING.md (the CLI + live model) and cloud/RUNNING.md
                       (the control plane, the chrome-driven E2E, and the web UI dev/build flow)

## Project intent / constraints (carry these forward)
- Learning/portfolio, local-first. Optimize for a clean, well-architected vertical
  slice over production hardening; the trait-decoupled architecture is the showcase.
- Each subsystem is its own spec → plan → implement cycle. Do ONE subsystem now.
- Keep the core untouched where possible; attach via the existing seams. (#5 and #6 each
  changed zero lines of the core crates — hold that bar.)
- Carry these notes from the core's final review (relevant to sandboxing/MCP specs):
  the command allow/deny policy is a guardrail, not a sandbox; the workspace path guard
  is lexical (no symlink resolution); OS-level sandboxing is deliberately deferred.
- Ground Cloudflare/Workers/Wrangler and browser work in the official docs (the cloudflare
  + chrome-devtools plugins/MCP are installed) rather than memory — the platform moves fast.

## Subsystem to spec this session
Requested: $ARGUMENTS

If the "Requested:" line above is blank, help me choose ONE — otherwise spec the
requested subsystem. Per the build order in context/README.md, #5 (Cloudflare control
plane) and #6 (React frontend) are DONE. What remains are the independent local
deepeners, doable in any order:
   #1 http-tool        — a `Tool` that fetches/parses web content (smallest; good warm-up)
   #2 os-sandboxing    — OS-level isolation at the intent()/exec boundary
   #3 mcp-client       — connect to MCP servers, surface their tools via ToolRegistry
   #4 memory-system    — vector / long-term memory via ContextManager + a Tool
A natural next step is also a "Settings" capability (deferred out of #6): editing model/
endpoint/policy from the browser, which needs a new inbound daemon-config channel.

## Known follow-ups (smaller than a subsystem — pick one if not specing a subsystem)
Open tech-debt items from shipped work. Project-wide canonical, always-current ledger
(per-subsystem review findings, Open/Accepted/Resolved): docs/superpowers/context/follow-ups.md
(populated automatically at the end of each SDD cycle — see "Track review findings durably"
below). The per-backend list docs/superpowers/context/claude-cli-inference.md
→ "Follow-ups / known limitations" remains the source of truth for claude-cli detail.
Keep both current; the boxes below are a pointer, not a second copy.
   [x] P1  RESOLVED — AgentLoop now enforces a per-turn idle (inter-chunk) timeout in
           one_completion (wraps stream-open + each chunk in tokio::time::timeout →
           retryable ModelError::Timeout). Config: LoopConfig.stream_idle_timeout
           (default 120s) / CLI --stream-timeout-secs. Covers SGLang + claude-cli.
           specs/2026-06-23-agent-loop-stream-timeout-design.md (merged 2026-06-23).
   [ ] P2  claude-cli: rate-limit strategy for the 5-hour subscription cap (detect
           rate_limit_event → typed ModelError + backoff) before sustained loops.
   [ ] P2  claude-cli: pin the subprocess CWD (Command::current_dir to an empty scratch
           dir) so project-local hooks in the launch dir can't load. Small, self-contained.
   [ ] P3  claude-cli: guard BARE_SYSTEM_PROMPT acceptance with an #[ignore]-gated
           real-CLI test, so a future guardrail change doesn't break it silently.

## Your task
Use the brainstorming skill to turn the chosen subsystem's primer into an approved,
written design spec (saved under docs/superpowers/specs/), then stop at the
writing-plans handoff. Read the relevant context primer first. Ask clarifying questions
one at a time. Do NOT write code or scaffold anything until I approve the design.

## Track review findings durably (don't lose Minors)
When this subsystem is later executed (subagent-driven-development), the SDD progress
ledger under `.superpowers/sdd/` is gitignored SCRATCH — never the resting place for
findings. Before finishing the branch, record EVERY Minor finding from the final
whole-branch review (plus any Important/accepted won't-fix items) into the durable,
committed ledger `docs/superpowers/context/follow-ups.md` (create it if absent), and
commit it as part of finishing. Use the same Open / Accepted (won't-fix) / Resolved
convention as `claude-cli-inference.md` → "Follow-ups / known limitations":
- One dated `## YYYY-MM-DD <subsystem>` section per cycle.
- Each item: short title, file:line ref, status (Open / Accepted / Resolved), and a
  one-line reason; mark items fixed during the cycle as Resolved with the commit SHA.
This `follow-ups.md` is the project-wide source of truth for review follow-ups
(the per-backend `claude-cli-inference.md` list remains for claude-cli detail).

## Environment notes
- Rust 1.96 via rustup, but cargo is NOT on PATH — run `source "$HOME/.cargo/env"`
  before any cargo command. Build/test from `agent/`: `cargo test --workspace`,
  `cargo clippy --all-targets -- -D warnings`.
- Node 22 + npm are available for the cloud/web sides. `cloud/`: `npm test` (Miniflare),
  `npx wrangler dev`. `web/`: `npm test` (Vitest+jsdom), `npm run dev` / `npm run build`.
  Full-stack run + the chrome-driven browser E2E: see cloud/RUNNING.md.
- A live model is available for testing if relevant: llama.cpp server
  (OpenAI-compatible) on http://localhost:8080, model id `qwen3.6-35b-a3b`, running in
  Docker as container `llama-agent` at 256k context. If it's not up:
  `docker start llama-agent` (or see the launch command in agent/docs/RUNNING.md).
- Drive the CLI: `cargo run -p agent-cli -- --base-url http://localhost:8080 --model
  qwen3.6-35b-a3b --workspace <dir> --context-limit 32768`. Keep --context-limit well
  below the server's capacity for latency (see RUNNING.md for the -c vs --context-limit
  note). To skip the model server entirely and use a Claude subscription instead, add
  `--backend claude-cli --model sonnet` (no --base-url; see agent/docs/RUNNING.md §1).
