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

## YAML gotcha: inline lists only

The checker's frontmatter parser accepts only flat `key: value` lines and
**inline** lists — `tags: [a, b]`. Block-style lists fail validation:

```yaml
tags:            # ✗ FAILS okf_check
  - building-agents
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
   python3 scripts/okf_check.py docs/okf/agent-sdlc
   ```

   Require `OK`. It exits 1 with one error per line otherwise — fix every
   line and re-run.
