---
okf_version: 0.1
---

# deepagents-refactor

Research bundle for refactoring the `agent/` Cargo workspace toward the
architecture of LangChain's [deepagents](https://docs.langchain.com/oss/python/deepagents/overview)
harness. Compiled 2026-07-08 from the deepagents docs and source, the
LangChain deep-agents and harness-engineering blog posts, and a very-thorough
live-source map of the current runtime. This bundle is the **knowledge base
for the refactor agent**: read it before writing the spec.

Intra-bundle links are bundle-root absolute: `/practices/foo.md` means
`docs/okf/deepagents-refactor/practices/foo.md`.

## Where to start

1. [Capability gap analysis](/comparisons/capability-gap-analysis.md) — the
   verdict table; the whole delta reduces to three structural gaps.
2. [Refactor priorities and sequencing](/comparisons/refactor-priorities.md) —
   what to build in what order, and what must not regress.
3. The two [perspectives](/perspectives/index.md) — target shape vs starting
   point — when you need either side as a coherent whole.
4. The nine [practices](/practices/index.md) — one node per deepagents
   capability pattern, each with its "why it matters for the refactor" delta.
5. [Sources](/sources/index.md) — condensed provenance notes; follow
   `resource:` URLs for ground truth. Current-runtime claims cite the repo
   itself — re-read live source before editing.

## Directories

- [comparisons/](/comparisons/index.md) — gap analysis + refactor sequencing (start here).
- [perspectives/](/perspectives/index.md) — deepagents target architecture; current runtime snapshot.
- [practices/](/practices/index.md) — the nine capability patterns to encode.
- [sources/](/sources/index.md) — five provenance nodes.

## Relationship to sibling knowledge

The `docs/okf/agent-sdlc/` bundle holds the broader cross-org evidence base
(context engineering, tool design, multi-agent decomposition); this bundle is
the deepagents-specific comparison layer. Links cannot cross bundles — cite
`agent-sdlc` by repo path in specs when both apply.
