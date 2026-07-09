# Authoring in the agent-sdlc bundle

Rules for adding or editing nodes in `docs/okf/agent-sdlc/` without breaking
validation. The checker, `scripts/okf_check.py`, is deliberately stricter
than the OKF spec — follow these shapes exactly.

## Frontmatter shapes

Every non-reserved `.md` file needs YAML frontmatter with a non-empty `type`.

`Source` nodes (`sources/`):

```yaml
---
type: Source
title: Demystifying evals for AI agents
description: Anthropic material on the SDLC of AI agents.
resource: https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents
org: Anthropic
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---
```

Concept nodes — `type` is one of `Practice` (practices/), `Lifecycle Phase`
(phases/), `Perspective` (perspectives/), `Comparison` (comparisons/):

```yaml
---
type: Practice
title: Eval-driven development
description: One-line summary; this is what directory indexes display and what agents route on — make it carry the whole idea.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---
```

## YAML style: inline lists, quote colons

The checker parses frontmatter with real YAML (PyYAML via uv). House style is
still **inline** lists — `tags: [a, b]` — and any value containing a colon must
be quoted:

```yaml
title: "AgentBench: Evaluating LLMs as Agents"   # unquoted colon = parse error
```

## Citation rule

Every concept under `phases/`, `practices/`, `perspectives/`, or
`comparisons/` MUST contain a `# Citations` heading (conventionally placed
at the end of the document) with at least one link whose target starts with
`/sources/`. Claims in the body carry `[n]` markers
pointing at that list.

**Verify claims against the source text before writing them into a
concept** — no unsupported assertions. If a claim needs a source the bundle
lacks, add the source node first.

## Link rule

Every link must resolve to an existing file inside the bundle — no links
escaping it. The checker resolves bundle-root-absolute targets
(`/practices/foo.md` = `docs/okf/agent-sdlc/practices/foo.md`) and relative
targets (from the linking file's directory); house style is bundle-root
absolute, so write new links that way. One hard exception: citation links
MUST literally start with `/sources/` — the checker matches the raw link
target, not the resolved path, so a relative citation path silently fails
the citation check even when it resolves.

## Reserved files

- `index.md` — no frontmatter. Exception: the bundle-root `index.md` may
  declare only `okf_version`.
- `log.md` — no frontmatter.

## Workflow for any change

1. Write or edit the node (frontmatter + cited body per the rules above).
2. Update the directory `index.md` (one line: link + description). If you
   added a directory or a major entry, update the root `index.md` too.
3. Add a dated entry to `log.md` describing the change.
4. Validate:

   ```bash
   uv run scripts/okf_check.py docs/okf/agent-sdlc
   ```

   Require `OK`. It exits 1 with one error per line otherwise — fix every
   line and re-run.

## What the checker does not catch

`okf_check.py` verifies structure only (frontmatter shape, type vocabulary,
`resource:` on Sources, link/citation/index integrity). Whether a node's claims
still match its live `resource:` is **semantic drift** — re-verify against the
source periodically and record the pass as a dated `log.md` entry.

## Verification passes for research bundles

Before any bundle counts as spec input (AGENTS.md § How we work), run two
review shapes — single-pass synthesis ships confident errors — and log each
as a dated `log.md` entry:

1. **Adversarial fact-verification.** Independent skeptical sub-agents, one
   per claim domain, prompted to *refute* (not confirm) each factual claim:
   external claims against live primary sources, repo claims against live
   code with file:line evidence. Verdicts per claim: CONFIRMED / REFUTED
   (with correction) / UNVERIFIABLE. Light models (sonnet-tier) suffice —
   this is fetch-and-compare work.
2. **Final consistency + completeness review.** After corrections land, one
   read-only pass over the whole bundle: hunt stale copies of corrected
   claims and internal contradictions (do tables agree with prose? do
   numbers match everywhere?); then ask what's *missing* — uncovered
   capabilities, uncited rows, claims with no upstream support in the
   bundle, index promises the files don't keep.

The shapes are complementary: per-claim refuters can't see stale copies of
their own corrections propagated elsewhere; a consistency pass can't detect
false facts. Design judgments (sequencing, scope calls) are **not** verified
this way — mark them unvalidated in `log.md` and route them to the
spec-phase adversarial panel. Worked example: `docs/okf/deepagents-refactor/log.md`
(2026-07-08 entries).
