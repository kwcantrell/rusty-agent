---
type: Practice
tags: [claude-cli, capability]
---

# Session Resume

The Claude CLI persists each print-mode session to disk and exposes it for
resumption via `--resume <id>`. The session ID is emitted in the first
`system/init` event as the `session_id` field [1]. Session ID lookup is scoped to
the current project directory and its git worktrees — both the initial call and
the `--resume` call must run from the same directory [2].

`--resume <id>` reuses the original session ID: the resumed call's `system/init`
event carries the same `session_id` as the original [1]. The client does not need
to re-capture a new ID after resuming, though re-capturing is harmless.
`--no-session-persistence` disables on-disk saving entirely (print mode only) [3];
sessions started with that flag cannot be resumed [2].

Session files are stored at `~/.claude/projects/<cwd-slug>/<session_id>.jsonl`
where `<cwd-slug>` replaces path separators with dashes. Passing a
non-existent ID to `--resume` causes the CLI to emit a plain-text error on
stderr (`No conversation found with session ID: <id>`), produce no JSON on
stdout, and exit with code 1 [1].

# Citations

1. [probe-resume-2-1-195](/sources/probe-resume-2-1-195.md)
2. [headless-print-mode](/sources/headless-print-mode.md)
3. [cli-reference](/sources/cli-reference.md)
