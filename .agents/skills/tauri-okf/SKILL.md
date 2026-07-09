---
name: tauri-okf
description: >-
  Use when answering questions about Tauri v2 — its process model, IPC,
  security/capabilities, testing (mock IPC, WebDriver), distribution, updater,
  performance, or mobile story — by consulting the verified knowledge bundle at
  docs/okf/tauri/ instead of re-researching from scratch.
---

# Tauri v2 knowledge bundle — consume guide

A fact-verified OKF v0.1 bundle at `docs/okf/tauri/`: snapshotted sources
(evidence layer) + capabilities/practices/comparisons synthesized from them.
Entry point: `docs/okf/tauri/index.md`.

## How to use it

1. Start at the bundle's `index.md`, then the directory index for your topic.
2. Concept files carry `[n]` citations resolving to `/sources/<slug>.md`; each
   source's `resource:` is the live URL.
3. **Citation-trust rule:** bundle claims are point-in-time (see the version
   stamp + staleness tripwire in the root index.md). Before acting on
   version-sensitive details — API names, capability config, signing steps —
   re-check the live doc via the source's `resource:` URL.

## Scope note

This is a knowledge-lookup skill. It is NOT the build/debug workflow skill —
routing is by intent: "what does Tauri do / what's the right practice" → here.

## Maintenance

Editing the bundle follows `.agents/skills/agent-sdlc/authoring.md` conventions
(frontmatter, citations, log.md discipline); validate with
`uv run scripts/okf_check.py docs/okf/tauri`.
