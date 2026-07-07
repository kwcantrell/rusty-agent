# Agent-SDLC research → OKF bundle — design

**Date:** 2026-07-06
**Status:** approved design, pending implementation plan

## Goal

Research the state of the art on "SDLC for AI agents" from first-party, reliable
sources and compile the findings as an **Open Knowledge Format (OKF) v0.1** bundle
committed to this repo at `docs/okf/agent-sdlc/`.

"SDLC for AI agents" covers **both** meanings, explicitly compared:

1. **SDLC for building agents** — how teams develop agent systems: design,
   prompt/tool iteration, evals, testing, deployment, monitoring/AgentOps.
2. **SDLC run by agents** — how AI coding agents participate in or drive the
   lifecycle: spec-first workflows, plan–execute–review loops, human gates.

## Requirements

- **Sources (allowlist):** first-party material from Google (cloud/developers/research
  blogs, official docs), Anthropic (engineering blog, research, docs), and LangChain
  (blog, docs), **plus** peer-reviewed papers (arXiv and published venues) and official
  research posts from major labs (DeepMind, OpenAI, Microsoft Research). No secondary
  blogs, Medium posts, or SEO content. Allowlist is enforced at curation time.
- **Depth:** comprehensive — ~25–40 curated sources, roughly 30–60 concept files.
- **Format:** OKF v0.1 as specified at
  `https://github.com/GoogleCloudPlatform/knowledge-catalog/tree/main/okf` (`SPEC.md`).
  Key conformance rules: every non-reserved `.md` file has parseable YAML frontmatter
  with a non-empty `type`; `index.md` / `log.md` are reserved (index files carry no
  frontmatter except `okf_version` in the bundle root); cross-links are root-relative
  markdown links; sourced claims list sources under a `# Citations` heading.
- **Subagent model tiering (explicit user requirement):** cheap models for mechanical
  work (fetching, extraction, listing), Fable/Opus only for deep reasoning (taxonomy,
  synthesis, the comparison), mid-tier for verification.

## Deliverable — bundle shape

```
docs/okf/agent-sdlc/
├── index.md                  # navigation + okf_version: "0.1"
├── log.md                    # change history, ISO-dated headings, newest first
├── sources/                  # type: Source — one file per research source (~25–40)
│   └── index.md
├── phases/                   # type: Lifecycle Phase — design, prototyping, evals,
│   └── index.md              #   deployment, monitoring/AgentOps, …
├── practices/                # type: Practice — eval-driven development, spec-first
│   └── index.md              #   workflows, human-in-the-loop gates, …
├── perspectives/             # type: Perspective — per-org synthesis (Google,
│   └── index.md              #   Anthropic, LangChain, academia)
└── comparisons/              # type: Comparison — building-agents vs agent-driven
    └── index.md              #   SDLC, and where they intersect
```

Design decisions:

- **Sources are concept nodes.** Each source file carries `resource:` (the URL),
  org, date, and a body summarizing what it contributes. Provenance is therefore part
  of the knowledge graph: every claim in a phase/practice/perspective/comparison file
  links to `/sources/<slug>.md` and lists them under `# Citations`.
- **The two-meanings comparison is a first-class concept** in `comparisons/`, not
  prose buried in an index.
- Frontmatter per concept: `type` (required), plus `title`, `description`, `tags`,
  `timestamp`; `resource` on sources. Extra keys (e.g. `org`, `published`) allowed
  per the spec's extension rule.

## Research pipeline

A `Workflow`-orchestrated pipeline (the user explicitly requested tiered subagents):

| Stage | What happens | Executor / model |
|---|---|---|
| 0. Ground | Read full OKF `SPEC.md` + one sample bundle from the repo; pin exact conformance details. Define per-org search seeds. | main loop, inline |
| 1. Discover | Parallel sweeps per source family: Anthropic engineering+research; Google cloud/developers/research blogs; LangChain blog/docs; arXiv + major-lab papers. Each agent returns a structured candidate list: URL, title, org, date, which SDLC meaning(s), 1-line relevance. | subagents, **haiku** |
| 2. Curate | Dedup candidates; enforce the allowlist; select ~25–40 spanning both meanings and all orgs. | main loop, inline |
| 3. Extract | Per source: fetch and produce structured notes — claims with supporting quotes, named practices, lifecycle phases touched, key terms. Structured-output schema. | subagents, **haiku** |
| 4. Synthesize | Taxonomy design from the full note set (which phases/practices/perspectives exist, what merges) happens in the main loop — deep reasoning. Writer agents then draft concept files per cluster from the notes; the comparison concept gets the highest-tier model. | main loop + subagents, **Fable/Opus** |
| 5. Verify | Per concept file: every claim traces to its cited source's notes, with re-fetch spot-checks against live URLs. Plus the deterministic conformance check (see Validation). | subagents, **sonnet** + script |
| 6. Finalize | Write `index.md` files and `log.md`; run the checker; commit. | main loop, inline |

Pipeline over barriers where stages are per-item independent (extract → write →
verify can flow per cluster); a barrier only where cross-item context is genuinely
needed (curation, taxonomy design).

## Data flow

Intermediate artifacts live in the session scratchpad, **not** the repo:

- `sources.json` — the curated, allowlisted source list (stage 2 output).
- `notes/<slug>.json` — per-source extraction notes with quotes and URLs (stage 3
  output). This is the provenance chain consumed by both writers (stage 4) and
  verifiers (stage 5).

Only the bundle itself plus the checker script are committed.

## Error handling

- **Fetch failure:** retry once; then drop the source and `log()` it visibly — no
  silent truncation. Discovery surplus (curate keeps a ranked reserve) provides
  replacements so the 25–40 target holds.
- **Verification failure:** a claim that doesn't trace to its source is reworded to
  match the source or removed. A concept that loses its evidentiary basis is dropped
  and the drop noted in `log.md`.
- **Allowlist violation:** rejected at curation (stage 2), never discovered
  downstream.

## Validation & acceptance criteria

A small checker, **committed** as `scripts/okf_check.py` (stdlib-only), validates:

1. every non-reserved `.md` file has parseable YAML frontmatter with non-empty `type`;
2. index files carry no frontmatter (except `okf_version` at the bundle root);
3. all intra-bundle links resolve to existing files;
4. every non-source concept making sourced claims has a `# Citations` section whose
   links resolve.

The checker is the bundle's regression test and stays useful for future bundle
updates.

**Done means:**

- `scripts/okf_check.py docs/okf/agent-sdlc/` passes;
- 25–40 sources, all from the allowlist;
- both SDLC meanings covered, with an explicit comparison concept;
- every non-obvious claim cited to a source concept with a live URL;
- bundle + checker committed (`docs(okf): …` / `feat(scripts): …` per conventions).

## Out of scope (possible follow-ups)

- Ingesting the bundle into this repo's graphify knowledge graph.
- Serving the bundle to the rust-agent-runtime's own agent as context
  (OKF-as-agent-context experiment).
