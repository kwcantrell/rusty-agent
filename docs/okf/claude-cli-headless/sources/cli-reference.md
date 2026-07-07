---
type: Source
resource: https://code.claude.com/docs/en/cli-reference
---

# Claude Code CLI Reference — Selected Flag Semantics

Source: https://code.claude.com/docs/en/cli-reference (fetched 2026-07-07)

## Session flags

### `--resume` / `-r`
Resume a specific session by ID or name. Without an argument, shows an interactive picker.  
Session ID search is scoped to the current project directory and its git worktrees.  
Background sessions appear marked with `bg` (v2.1.144+).

```bash
claude --resume auth-refactor
claude -p --resume "$session_id" ...
```

### `--session-id`
Use a specific UUID for the conversation instead of auto-generating one.

```bash
claude --session-id "550e8400-e29b-41d4-a716-446655440000"
```

### `--fork-session`
When resuming (with `--resume` or `--continue`), create a new session ID instead of reusing the original. Allows branching from an existing session without modifying it.

### `--continue`
Continue the most recent conversation.

### `--no-session-persistence`
Disable session persistence (print mode only). Sessions not saved to disk; cannot be resumed.  
Alternative: `CLAUDE_CODE_SKIP_PROMPT_HISTORY` env var (works in any mode).

## Streaming flags

### `--include-partial-messages`
Include partial streaming events in output.  
Requires `--print` (`-p`) and `--output-format stream-json`.  
Emits intermediate `assistant` message states during streaming alongside `stream_event` lines.

## Model / effort flags

### `--effort`
Set the effort level for the current session. Does not persist (overrides `effortLevel` setting for this invocation only).  
Valid values: `low`, `medium`, `high`, `xhigh`, `max`  
Available levels depend on the model.

```bash
claude --effort high
```

### `--fallback-model`
Enable automatic fallback to specified model(s) when the primary model is unavailable (overloaded or retired).  
Accepts a comma-separated list tried in order. Overrides the persistent `fallbackModel` setting.

```bash
claude --fallback-model sonnet,haiku
```

## Config / isolation flags

### `--setting-sources`
Comma-separated list of setting sources to load. Options: `user`, `project`, `local`.  
Controls which configuration files are read.

```bash
claude --setting-sources user,project
```

### `--strict-mcp-config`
Only use MCP servers from `--mcp-config`; ignore all other MCP configurations.  
Must be paired with `--mcp-config` to have effect.

```bash
claude --strict-mcp-config --mcp-config ./mcp.json
```
