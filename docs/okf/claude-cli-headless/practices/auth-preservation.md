---
type: Practice
tags: [claude-cli, practice]
---

# Auth Preservation

Passing `--bare` to the CLI skips auto-discovery of hooks, skills, plugins, MCP
servers, auto memory, and CLAUDE.md [1]. This makes it suitable for CI but
creates a footgun for subscription-based users: `--bare` forces API-key
authentication, bypassing the Claude subscription piggyback that the normal
launch path uses. The claude_cli backend omits `--bare` deliberately, preserving
subscription auth.

`AGENT_API_KEY` is explicitly removed from the CLI subprocess environment before
launch. Leaving it set would push the CLI onto API-key billing even when the
user's subscription auth would otherwise be available via the normal config path.

`--setting-sources project` (or a restricted subset) is passed to skip loading
the user-level settings that trigger SessionStart hooks and other side effects
that would interfere with programmatic use [2]. This gives the client a
reproducible configuration without the full isolation penalty of `--bare`.

# Citations

1. [headless-print-mode](/sources/headless-print-mode.md)
2. [cli-reference](/sources/cli-reference.md)
