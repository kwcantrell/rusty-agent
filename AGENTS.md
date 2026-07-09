# rust-agent-runtime — agent guide

A local-first LLM agent runtime: a Rust core drives a local model (or the Claude CLI)
through a tool/policy loop, exposed two ways — a terminal CLI and a Tauri desktop app
whose browser-based UI reaches your *local* agent over Tauri IPC.

## Repo map

Three surfaces, one agent core — each surface has its own `AGENTS.md` with commands
and surface-local gotchas:

- **[`agent/`](agent/AGENTS.md)** — Rust Cargo workspace (the core); crate map inside.
- **[`src-tauri/`](src-tauri/AGENTS.md)** — Tauri 2 desktop app wrapping `agent-server`.
  Its own separate Cargo workspace.
- **[`web/`](web/AGENTS.md)** — React 19 / Vite / Tailwind SPA (the Context Explorer UI);
  the Tauri desktop app's frontend, reaching the local agent over **Tauri IPC** (`agent-server`).

> **No cloud/Worker path.** An earlier Cloudflare Worker control plane (browser → Worker →
> local agent) was removed in `7245526` — `agent-server` is now a library-only crate driven
> over Tauri IPC. The design docs under
> `docs/superpowers/{specs,plans}/2026-06-22-cloudflare-control-plane*` are historical.

## How we work

Non-trivial work follows the superpowers SDLC — **don't jump straight to code**:

**brainstorm → spec (`docs/superpowers/specs/`) → plan (`docs/superpowers/plans/`) →
implement.** Small, obvious fixes can skip ahead, but design-bearing changes get a
spec first.

### Adversarial review of the spec

Before `spec → plan`, run an **adversarial panel on the spec** — the earliest,
least-reversible artifact, where catching a wrong assumption is cheapest. A flawed
spec makes a *perfect* plan build the wrong thing.

Fan out (via `dispatching-parallel-agents`) reviewers with **distinct** mandates —
not clones:

- **Requirements** — what could be built into the wrong thing?
- **Assumptions** — what's assumed true but unverified?
- **Failure & abuse** — how does this break or get misused? Threat model?
- **Scope & simpler design** — what's over-built (YAGNI), what simpler approach is skipped?

Calibrate them **skeptical** — default to finding a path to a wrong/broken outcome
before approving (the opposite of the stock "approve unless broken" reviewers).
Then **synthesize** (dedup, resolve conflicts, rank by severity) and feed that into
the user's spec-review gate — the panel *arms* the human gate, it doesn't replace it.

Disposition rule: a finding that conflicts with the user's stated mandate is
**escalated to the gate as an explicit decision, never silently adopted or
dismissed** — the panel doesn't outrank the owner, and the mandate doesn't
grade its own work. Record every disposition in the artifact's dated
**"Panel & review log"** section, in three buckets: blockers/majors *fixed in
place*, findings *escalated to the gate* (with the eventual decision), and
minors *accepted as residual*.

When fanning out sub-agents (here or anywhere), **match model tier to the task**:
light models (haiku/sonnet) for fetch-and-compare verification and mechanical
sweeps; heavyweight models for synthesis and adversarial judgment.

Keep plan review as-is (single reviewer: spec coverage, decomposition, buildability).
Only add a *lighter* adversarial pass on the plan — scoped to architecture/decomposition,
**not** requirements — if real design decisions leak downstream into the plan.

### Research artifacts feed specs — verify them first

Research/knowledge artifacts a spec will consume (OKF bundles, gap analyses,
architecture comparisons) get two review shapes **before** they count as spec
input: an adversarial fact-verification pass (independent skeptics refuting
claims against primary sources / live code) and a final consistency +
completeness review. They're complementary — per-claim refuters can't see
stale copies of corrected claims; a consistency pass can't detect false
facts. Record both as dated entries in the artifact's log; design judgments
stay unverified and go to the spec panel. Mechanics:
`.agents/skills/agent-sdlc/authoring.md`.

### Post-gate edits get a consistency read, not a re-panel

The same complementarity applies downstream: when gate decisions or review
fixes are applied as **targeted edits** to an already-reviewed artifact (a
spec after its gate, a plan after its review), run one **light-tier
consistency reviewer** over the final document before it feeds the next
stage — stale pre-decision language, contradictions between dispositions and
normative sections, broken cross-references. A full re-panel is warranted
only for **structural rework**; disposition-recording prose is not that.

### Docs-only exception

Docs/ledger-only campaigns (audits, triage records, campaign ledgers, memory
bookkeeping) may commit directly to `main` without a feature branch.
Compensating control: `main` is never pushed automatically, and the
whole-campaign review must pass before any push — review findings land as fix
waves on main, not silent history edits. Anything touching code, tests, or CI
still branches.

## Conventions

- **Conventional commits**: `type(scope): summary` (e.g. `fix(memory): …`), matching
  existing history.
- **Commit and push only when asked.** Branch off `main` for PRs.
- **Changes ship with tests.** Run the relevant suite (`cargo test` / `npm test`)
  before calling it done.
- **`file:line` anchors in specs/plans/briefs go stale** the moment earlier
  work inserts code. Anchors are for orientation; **locate the quoted code by
  content before editing**, and say so in any prompt you hand a sub-agent.

## CI gate

```bash
bash scripts/ci.sh   # okf check + skills lint + fmt + clippy + cargo test (agent/) + conditional src-tauri + web typecheck/vitest
```

Also runs as a pre-push hook — enable once per clone with
`git config core.hooksPath .githooks`.

## Gotchas

- **Two separate Cargo workspaces** — `agent/` and `src-tauri/`. `-p <crate>` must
  target the right one.
- **Three skill trees — author in the right one:**
  - `.agents/skills/` — canonical home of skills for working *on* this repo. Author here.
  - `.claude/skills/` — relative symlinks into `.agents/skills/` so Claude Code
    discovers them. Never author here; add a symlink for each new skill
    (`scripts/skills_lint.py` enforces both).
  - `<workspace>/.agent/skills` and `~/.agent/skills` — loaded by the runtime's *own*
    agent (`agent-skills/src/registry.rs`). Unrelated to working on this repo.
