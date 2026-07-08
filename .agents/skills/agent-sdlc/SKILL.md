---
name: agent-sdlc
description: >-
  Use for any question about how to build, evaluate, deploy, or operate AI
  agents — evals, tool design, context engineering, multi-agent decomposition,
  memory, human-in-the-loop gates, monitoring — or how agents should run a
  software lifecycle (spec-first workflows, verification-first coding). Routes
  to the source-backed knowledge bundle at docs/okf/agent-sdlc/ (36 first-party
  sources, 23 cited concepts): the EVIDENCE layer behind agent-architecture
  decisions, specs, and reviews. For hands-on design/build/audit work on THIS
  repo's harness, use harness-engineering (the playbook layer) and load this
  bundle alongside it for citations. Also use when extending or fixing the
  bundle (see authoring.md).
---

# agent-sdlc

A knowledge bundle of research findings on the software development lifecycle
for AI agents lives at `docs/okf/agent-sdlc/` (OKF v0.1). It compiles **36
first-party sources** — Anthropic, Google, LangChain, and peer-reviewed
research — into **23 concept pages** where every claim carries a citation.
Use it as the evidence base for any agent-shaped design decision: don't
re-derive best practices from memory when a cited concept already answers the
question.

**Do not** use this skill to *do* harness design/build/audit work in this
repo — that is `harness-engineering`'s playbooks; this bundle is the evidence
layer behind them. **Do not** edit bundle files without reading
[authoring.md](authoring.md) and re-running `python3 scripts/okf_check.py
docs/okf/agent-sdlc` — frontmatter, citation, and link rules are
machine-checked.

"Agent SDLC" means two things, and the bundle covers both:

1. **The SDLC for *building* agent products** — evals, tools, context,
   memory, deployment, monitoring.
2. **The SDLC as *run by* agents** — spec-first workflows, human-in-the-loop
   gates, verification-first coding.

If the distinction matters to your task, read
`docs/okf/agent-sdlc/comparisons/two-meanings.md` first.

## Navigation

Start at `docs/okf/agent-sdlc/index.md`. Every directory has its own
`index.md` listing each node with a one-line description — those lines are
the routing signal. Scan the index, then open only the concepts you need.

| Directory | Contents | Reach for it when… |
|---|---|---|
| `practices/` | 12 named cross-org practices | you have a design question ("how should we do evals / tools / memory / multi-agent?") — the most useful entry point |
| `phases/` | 6 lifecycle stages, design → monitoring | you want to know what a stage involves or which practices belong to it |
| `perspectives/` | per-org syntheses (Anthropic, Google, LangChain, academia) | you want one org's coherent view, or to contrast orgs |
| `comparisons/` | cross-cutting comparisons | orientation — start with `two-meanings.md` |
| `sources/` | 36 provenance nodes: condensed notes per source, each with a `resource:` URL | verifying a claim, or going deeper than a concept |

## Link convention — read this or you will chase dead paths

Intra-bundle links are **bundle-root absolute**: `/practices/foo.md` means
`docs/okf/agent-sdlc/practices/foo.md`. They are not filesystem paths and
not repo-root paths.

## Trust model

- Every concept claim carries `[n]` citation markers resolving to a
  `# Citations` section that links into `sources/`. Concepts were verified
  against their sources (conformance clean as of 2026-07-06).
- Source files are condensed notes, not the original documents. For
  load-bearing decisions, follow the citation into the source file; its
  `resource:` URL is the ultimate ground truth.
- When you use bundle material in a spec, review, or design argument, **cite
  the bundle path** (e.g.
  `docs/okf/agent-sdlc/practices/eval-driven-development.md`) so the claim
  stays traceable.

## Relationship to harness-engineering

`.agents/skills/harness-engineering/` is the *playbook* layer — how to
audit, design, build, and eval this runtime's harness. This bundle is the
*evidence* layer behind those playbooks. Doing harness work? Load both: the
playbook for procedure, the bundle for source-backed justification.

## Extending the bundle

Adding a source, adding or editing a concept, or fixing links? Read
[authoring.md](authoring.md) first — the bundle is validated by
`scripts/okf_check.py` and has strict frontmatter, citation, and link rules.
