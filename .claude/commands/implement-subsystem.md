---
description: Take an approved design spec and drive it to a reviewed, merge-ready branch (worktree → plan → subagent-driven build → review → finish)
argument-hint: "[spec slug or path, e.g. os-sandboxing — or leave blank to choose]"
---

I'm implementing an already-approved design spec for this production-grade, local-first AI
coding-agent platform. The spec exists under docs/superpowers/specs/; I want to plan (if
needed) and execute it to a reviewed, merge-ready branch.

## Repo
/home/kalen/rust-agent-runtime  (git repo, on `main`)

## Spec to implement
Requested: $ARGUMENTS

Resolve this to the design spec at docs/superpowers/specs/YYYY-MM-DD-<slug>-design.md (and
its plan at docs/superpowers/plans/YYYY-MM-DD-<slug>.md if one exists). If the "Requested:"
line is blank, ask which spec to implement.

## Orient with the knowledge graph
There is intentionally NO prose inventory of what exists here — discover it from the graph.
This repo ships a graphify knowledge graph of the whole codebase + docs in `graphify-out/`
(`graph.html` interactive map, `GRAPH_REPORT.md` audit, `graph.json` raw):
- Enumerate what's built / find the seams you'll attach to:
  `graphify query "what crates exist and what each one does"`,
  `graphify explain "<symbol>"`, `graphify path "<A>" "<B>"`. Skim `GRAPH_REPORT.md` for god
  nodes (core abstractions) + the community map.
- The graph is the map, NOT the authority — the spec/plan and the code are ground truth.
- `.graphifyignore` excludes the SDD task ledger (`.superpowers/sdd/`) and generated Wrangler
  artifacts (`.wrangler/`), so the graph reflects authored architecture, not build output.
- Refresh the graph with `/graphify . --update` (re-extracts only changed files) ONCE after
  ALL tasks of the subsystem are complete — not per task. Run it near merge so a single
  update captures the whole subsystem.

## Project intent / constraints (carry these forward)
- Production-grade, local-first. Optimize for production hardening — correctness under
  failure, security/isolation, observability, resource limits, and clean error paths — not
  just a working happy-path slice. The plan must make its failure modes, threat surface, and
  operational concerns explicit.
- The trait seams (EventSink, ApprovalChannel, Tool/ToolRegistry, ContextManager) are tools
  for clean design, NOT a freeze on the core — refactor the core where hardening calls for
  it. 
- Ground Cloudflare/Workers/Wrangler and browser work in the official docs (the cloudflare
  + chrome-devtools plugins/MCP are installed) rather than memory — the platform moves fast.

## Your task
Drive the approved spec to a reviewed, merge-ready branch:
1. using-git-worktrees — cut an isolated worktree/branch off `main` before touching code.
2. writing-plans — if no plan exists yet for this spec, write one (saved at
   docs/superpowers/plans/<same date+slug>.md, referencing the spec under Global Constraints).
   If a plan already exists, use it.
3. subagent-driven-development (recommended) or executing-plans — drive the plan task-by-task,
   TDD per task (RED → GREEN), with a per-task requesting-code-review between tasks.
4. requesting-code-review — a final whole-branch review (opus) before finishing.
5. finishing-a-development-branch — gated by the plan's Done criteria: `cargo test --workspace`
   green; `cargo clippy --all-targets -- -D warnings` clean; the spec's DoD validated against
   the live model where relevant.
6. After ALL tasks: run `/graphify . --update` ONCE near merge to refresh the knowledge graph.

The `.superpowers/sdd/` progress ledger is gitignored SCRATCH — never the resting place for
findings (see below).

## Track review findings durably (don't lose Minors) — ALL follow-ups live in follow-ups.md
Before finishing the branch, record EVERY Minor finding from the final whole-branch review
(plus any Important/accepted won't-fix items) into the durable, committed ledger
`docs/superpowers/context/follow-ups.md`, and commit it as part of finishing. This file is
the SINGLE project-wide source of truth for review follow-ups — every cycle's findings go
here. Use the same convention as the rest of the file:
- One dated `## YYYY-MM-DD <subsystem>` section per cycle.
- Each item: short title, file:line ref, status (Open / Accepted / Resolved), and a
  one-line reason; mark items fixed during the cycle as Resolved with the commit SHA.

## Environment notes
- Rust 1.96 via rustup, but cargo is NOT on PATH — run `source "$HOME/.cargo/env"`
  before any cargo command. Build/test from `agent/`: `cargo test --workspace`,
  `cargo clippy --all-targets -- -D warnings`.
- Node 22 + npm are available for the cloud/web sides. `cloud/`: `npm test` (Miniflare),
  `npx wrangler dev`. `web/`: `npm test` (Vitest+jsdom), `npm run dev` / `npm run build`.
  Full-stack run + the chrome-driven browser E2E: see cloud/RUNNING.md.
