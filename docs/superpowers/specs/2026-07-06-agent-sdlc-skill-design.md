# agent-sdlc skill — design

**Date:** 2026-07-06
**Status:** approved (brainstorm complete)

## Goal

A Claude-facing skill that teaches agents working in this repo how to **consume**
and **maintain** the agent-sdlc knowledge bundle at `docs/okf/agent-sdlc/`
(OKF v0.1: 36 first-party sources, 23 cited concepts).

## Decisions made during brainstorming

- **Scope:** consume + maintain in one skill.
- **Relationship to `harness-engineering`:** complementary. `harness-engineering`
  is the *playbook* layer for doing harness work on this repo; this skill is the
  *knowledge lookup* layer over the evidence base. Each cross-references the other.
- **Structure:** `SKILL.md` (always-loaded reading/citing guidance) +
  `authoring.md` (maintenance rules, loaded on demand). Progressive disclosure,
  matching the existing `harness-engineering` router convention. A separate
  task→concept query map was rejected: the bundle's own `index.md` files are
  already the lookup table and a duplicate would drift.

## Location

`.agents/skills/agent-sdlc/` — the **Claude-facing** skill tree. Not
`.agent/skills` (that tree belongs to the runtime's own agent; see CLAUDE.md
"Two skill trees" gotcha).

```
.agents/skills/agent-sdlc/
├── SKILL.md      # when/how to read and cite the bundle
└── authoring.md  # how to extend it without breaking okf_check
```

## SKILL.md (~100 lines)

**Frontmatter:** `name: agent-sdlc`; `description` triggers on:

- any question about how to build, evaluate, deploy, or operate AI agents, or
  how agents should run a software lifecycle (evals, tool design, context
  engineering, multi-agent decomposition, memory, human-in-the-loop gates,
  monitoring, harness design, spec-first workflows);
- wanting source-backed evidence for an agent-architecture decision or review;
- adding to or updating the bundle itself.

**Body sections:**

1. **What the bundle is.** OKF v0.1 bundle at `docs/okf/agent-sdlc/`: 36
   sources (Anthropic, Google, LangChain, arXiv/peer-reviewed), 23 concepts,
   every claim cited. Flag the "two meanings" split — the SDLC *for building
   agents* vs the SDLC *run by agents* — and point at
   `comparisons/two-meanings.md` as the orientation read.
2. **Navigation map.** `phases/` = lifecycle stages; `practices/` = 13 named
   cross-org practices (the most useful entry point for design questions);
   `perspectives/` = per-org syntheses; `comparisons/`; `sources/` = provenance
   nodes. Start from `index.md` files; each concept's `description:` frontmatter
   line is the routing signal.
3. **Link convention (critical).** Intra-bundle links are *bundle-root
   absolute*: `/practices/foo.md` resolves to
   `docs/okf/agent-sdlc/practices/foo.md` — not a filesystem path, not a
   repo-root path.
4. **Trust model.** Every concept claim carries `[n]` citations resolving into
   `sources/`; concepts were verified against sources (conformance clean as of
   2026-07-06). For load-bearing decisions, follow the citation into the source
   file; the source's `resource:` URL is the ultimate ground truth (source
   files are condensed notes, not the original documents). When using bundle
   material in specs or reviews, cite bundle paths so claims stay traceable.
5. **Relationship to harness-engineering.** That skill = how to do harness work
   here; this bundle = the evidence behind it. Pull both for harness work.
6. **Extending the bundle → read `authoring.md`.**

## authoring.md (~70 lines)

1. **Node types and frontmatter shapes.**
   - `Source`: `type, title, description, resource (URL), org, tags, timestamp`
   - Concepts: `type` one of `Practice` / `Lifecycle Phase` / `Perspective` /
     `Comparison`, plus `title, description, tags, timestamp`.
2. **YAML gotcha:** `scripts/okf_check.py` parses only flat `key: value` and
   *inline* lists (`tags: [a, b]`). Block-style lists fail validation.
3. **Citation rule:** every concept under `phases/`, `practices/`,
   `perspectives/`, `comparisons/` must have a `# Citations` section with ≥1
   link resolving into `/sources/`. Verify claims against the source text
   before writing them into a concept — no unsupported assertions.
4. **Reserved files:** `index.md` carries no frontmatter (bundle-root
   `index.md` may declare only `okf_version`); `log.md` carries no frontmatter.
5. **Workflow for any change:** write/edit the node → update the directory
   `index.md` (and root `index.md` if a new directory or major entry) → add a
   dated entry to `log.md` → run
   `python3 scripts/okf_check.py docs/okf/agent-sdlc` and require `OK`.
6. **Links:** bundle-root absolute; every intra-bundle link must resolve to an
   existing file inside the bundle.

## Cross-reference back-edge

Add a one-line pointer in `.agents/skills/harness-engineering/SKILL.md`
directing readers to the `agent-sdlc` skill/bundle as the evidence base.
(Single-line addition; no other changes to that skill.)

## Out of scope

- No changes to bundle content itself.
- No task→concept query map (rejected — duplicates indexes, would drift).
- No runtime-agent-facing copy under `.agent/skills`.
- Graphify ingestion of the bundle (tracked as a separate follow-up in memory).

## Testing / verification

- Skill loads: frontmatter parses, description present, files well-formed.
- `python3 scripts/okf_check.py docs/okf/agent-sdlc` still prints `OK`
  (sanity: the skill must not touch bundle files).
- Behavioral spot-check: ask a fresh agent an agent-SDLC question (e.g. "how
  should we seed an eval dataset?") and confirm it routes through the bundle
  and cites `docs/okf/agent-sdlc/...` paths.
