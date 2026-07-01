# rust-agent-runtime

A local-first LLM agent runtime: a Rust core drives a local model (or the Claude CLI)
through a tool/policy loop, exposed three ways — a terminal CLI, a Tauri desktop app,
and a browser SPA that reaches your *local* agent via a Cloudflare Worker.

## Repo map

Three surfaces, one agent core:

- **`agent/`** — Rust Cargo workspace (the core). Crates below.
- **`src-tauri/`** — Tauri 2 desktop app wrapping `agent-server`. Its own separate workspace.
- **`web/`** — React 19 / Vite / Tailwind SPA (the Context Explorer UI).
- **Cloud path** — `agent-server` dials a Cloudflare Worker so a browser can drive the local agent.

Crates (`agent/crates/`):

| crate | responsibility |
|-------|----------------|
| `agent-core` | agent loop, context manager, event model |
| `agent-model` | model client, tool-call protocols (native/prompted), inference types |
| `agent-tools` | shared tool vocabulary and the `Tool` trait |
| `agent-http` | outbound HTTP fetch tool; gates egress in-tool |
| `agent-mcp` | MCP client — connect to external MCP servers |
| `agent-memory` | long-term semantic memory (remember/recall/forget over a local vector store) |
| `agent-policy` | permission policy engine + approval channel |
| `agent-sandbox` | sandboxed tool/command execution |
| `agent-skills` | discover, load-on-demand, author, preload markdown skills |
| `agent-server` | daemon bridging the local agent to the Cloudflare Worker (browser UI backend) |
| `agent-cli` | terminal front-end binary |
| `agent-runtime-config` | shared loop wiring (tool registry, protocol picker, command lists) |

## Graphify first

A knowledge graph of this repo lives in `graphify-out/` (`graph.json`, `GRAPH_REPORT.md`).
It exists — use it.

- **A structural/relational question is a graph query before a grep.** "How does X work /
  what connects to Y / where does Z flow" → query the graph first. Manual search ignores a
  pre-built relational index.
- **Expand your query against the graph's own vocabulary first** — the matcher is case-folded
  substring + IDF, no synonyms. Zero hits means a vocabulary miss, not absence: re-expand and retry.
- **`EXTRACTED` = fact. `INFERRED`/`AMBIGUOUS` = a lead — verify at source.** Cite
  `source_location`; re-read live source before changing anything.
- **`graphify . --update`, never a full rebuild** (a rebuild re-pays extraction for the whole corpus).
  Seed from `GRAPH_REPORT.md`'s God Nodes and Suggested Questions.
- Full judgment lives in the `graphify-best-practices` skill — reach for it alongside any graph work.

## How we work

Non-trivial work follows the superpowers SDLC — **don't jump straight to code**:

**brainstorm → spec (`docs/superpowers/specs/`) → plan → implement**, driven by the superpowers
skills. Small, obvious fixes can skip ahead, but design-bearing changes get a spec first.

## Commands

Rust core (`cd agent` first; `source ~/.cargo/env` if `cargo` isn't on PATH):

```bash
cargo build                                  # whole workspace
cargo test -p <crate>                        # test one crate
cargo run -p agent-cli -- --backend claude-cli --model sonnet --workspace .   # run the CLI
AGENT_E2E_URL=… AGENT_E2E_MODEL=… cargo test -p agent-core --test e2e_sglang -- --ignored
```

Web (`cd web`):

```bash
npm test        # vitest
npm run build   # tsc -b && vite build
npm run typecheck
```

Desktop (repo root): `npm run desktop:dev` / `npm run desktop:build`.

See `agent/docs/RUNNING.md` for the full model-server setup (llama.cpp / SGLang / vLLM / Claude CLI).

## Conventions

- **Conventional commits**: `type(scope): summary` (e.g. `fix(memory): …`), matching existing history.
- **Commit and push only when asked.** Branch off `main` for PRs.
- **Changes ship with tests.** Run the relevant suite (`cargo test` / `npm test`) before calling it done.

## Gotchas

- **Two separate Cargo workspaces** — `agent/` and `src-tauri/`. `-p <crate>` must target the right one.
- **The graph reflects the last build and can be stale.** Read live source before editing; `--update` after doc/image changes (code changes re-extract for free).
