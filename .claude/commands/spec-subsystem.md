---
description: Brainstorm → design spec for the next deferred subsystem (stops at the writing-plans handoff)
argument-hint: "[subsystem: os-sandboxing | memory-system | a follow-up — or leave blank to choose]"
---

I'm continuing work on a production-grade, local-first AI coding-agent platform. Several
slices are already built, reviewed, merged to `main`, and validated end-to-end against a
real local model. I want to brainstorm and spec the NEXT subsystem — design only; we stop
at the writing-plans handoff and build it later with `/implement-subsystem`.

## Repo
/home/kalen/rust-agent-runtime  (git repo, on `main`)

## Orient with the knowledge graph (then read the docs)
There is intentionally NO prose inventory of what exists here — discover it from the graph.
This repo ships a graphify knowledge graph of the whole codebase + docs in `graphify-out/`
(`graph.html` interactive map, `GRAPH_REPORT.md` audit, `graph.json` raw):
- Enumerate what's built: `graphify query "what crates exist and what each one does"` —
  answers from the graph, citing source file:line. Skim `GRAPH_REPORT.md` for god nodes
  (core abstractions) + the community map.
- `graphify query "<question>"` for anything you'd otherwise grep; `graphify explain "<symbol>"`
  for one node; `graphify path "<A>" "<B>"` to trace how two pieces connect.
- The graph is the map, NOT the authority — the docs below and the code are ground truth.

## Project intent / constraints (carry these forward)
- Production-grade, local-first. Optimize for production hardening — correctness under
  failure, security/isolation, observability, resource limits, and clean error paths — not
  just a working happy-path slice. Every spec must make its failure modes, threat surface,
  and operational concerns explicit.
- Each subsystem is its own spec → plan → implement cycle. Do ONE subsystem now.
- The trait seams (EventSink, ApprovalChannel, Tool/ToolRegistry, ContextManager) are tools
  for clean design, NOT a freeze on the core — refactor the core where hardening calls for
  it. 
- Ground Cloudflare/Workers/Wrangler and browser work in the official docs (the cloudflare
  + chrome-devtools plugins/MCP are installed) rather than memory — the platform moves fast.

## Subsystem to spec this session
Requested: $ARGUMENTS

If the "Requested:" line above is blank, help me choose ONE — otherwise spec the requested
subsystem. Determine what's genuinely unbuilt by reading the ledgers, not by trusting a
hardcoded list: `docs/superpowers/context/README.md` (build order) +
`docs/superpowers/context/follow-ups.md` (every shipped cycle).

## Your task
Use the brainstorming skill to turn the chosen subsystem's primer into an approved, written
design spec (saved under docs/superpowers/specs/YYYY-MM-DD-<slug>-design.md), then STOP at
the writing-plans handoff. Read the relevant context primer first. Ask clarifying questions
one at a time. Do NOT write code or scaffold anything until I approve the design. Every
design must explicitly address failure modes, threat surface, resource limits, and
observability. When the spec is approved, run `/implement-subsystem` to build it.

## Follow-ups
Not specing a full subsystem? Smaller tech-debt items live in the project-wide ledger
docs/superpowers/context/follow-ups.md — pick one from there. ALL review follow-ups are
saved in that file (populated at the end of each `/implement-subsystem` cycle).

## Environment notes
- Rust 1.96 via rustup, but cargo is NOT on PATH — run `source "$HOME/.cargo/env"`
  before any cargo command. Build/test from `agent/`: `cargo test --workspace`,
  `cargo clippy --all-targets -- -D warnings`.
- Node 22 + npm are available for the cloud/web sides. `cloud/`: `npm test` (Miniflare),
  `npx wrangler dev`. `web/`: `npm test` (Vitest+jsdom), `npm run dev` / `npm run build`.
  Full-stack run + the chrome-driven browser E2E: see cloud/RUNNING.md.
