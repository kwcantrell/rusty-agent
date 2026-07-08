---
type: Practice
title: Progressive skill disclosure
description: Skills are SKILL.md directories on the virtual filesystem, disclosed in three layers — frontmatter metadata in the system prompt, full body on activation via read_file, supporting resources on demand.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Progressive skill disclosure

deepagents packages domain knowledge as skills: a directory containing
`SKILL.md` with YAML frontmatter (`name` ≤64 chars and matching the directory
name, `description` ≤1024 chars, optional `license` / `compatibility` /
`metadata` / `allowed-tools`), with convention subdirectories for `scripts/`,
`references/`, and `assets/` [1][2].

## The three layers

The injected prompt names the pattern explicitly — skills "follow a
progressive disclosure pattern" [2]:

1. **Startup** — only name + description enter the system prompt.
2. **Activation** — the agent reads the full `SKILL.md` with its ordinary
   `read_file` tool (the prompt instructs `limit=1000` because the 100-line
   default is too small for skill files) [2].
3. **On demand** — supporting resources are read as needed.

Because skills are just files, they live wherever the backend puts them —
seeded graph state, a store namespace, plain disk, or a hub repo — and the
same file tools serve them ([filesystem as context
substrate](/practices/filesystem-as-context-substrate.md)) [1].

## Why it matters for the refactor

The current runtime already has SKILL.md-shaped skills with discovery dirs
and load-on-demand — but through *bespoke tools* (`ListSkills`, `UseSkill`,
`ReadSkillFile`) that inject skill bodies as context messages, plus presets
inlined into the system prompt
([current runtime](/perspectives/current-runtime.md)) [3]. Under a virtual
filesystem, layer 2 collapses into `read_file` and the custom tool surface
shrinks; the main genuinely new elements are the frontmatter validation
limits and prompt-listed skill locations [2][1].

# Citations

1. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
2. [deepagents source (langchain-ai/deepagents)](/sources/deepagents-source.md)
3. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
