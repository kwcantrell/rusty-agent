# Next-Spec Handoff Prompt

This handoff prompt is now a slash command: **`/next-spec`** (defined in
`.claude/commands/next-spec.md`).

**How to use:** in a fresh agent session, run

```
/next-spec <subsystem>
```

where `<subsystem>` is one of `http-tool`, `os-sandboxing`, `mcp-client`,
`memory-system`, `settings`, or a follow-up — or run `/next-spec` with no argument to
have the agent help you choose from the build order. The command expands to the full
handoff prompt (repo state, what's built, what to read, project constraints, your task,
environment notes) and starts a brainstorm→spec cycle for the next deferred subsystem.

**Keep it updated:** the prompt body lives in `.claude/commands/next-spec.md` — edit that
file as the project state changes (crates added, subsystems completed, environment
changes). When code changes, also refresh the knowledge graph with `/graphify . --update`
so the next session's map matches reality. This doc is just a pointer; don't re-paste the
prompt here (one source of truth avoids drift).
