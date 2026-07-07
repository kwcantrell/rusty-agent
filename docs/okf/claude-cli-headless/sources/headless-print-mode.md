---
type: Source
resource: https://code.claude.com/docs/en/headless
---

# Claude Code Headless / Print Mode

Source: https://code.claude.com/docs/en/headless (fetched 2026-07-07)

## Print mode basics

Pass `-p` (or `--print`) to run Claude non-interactively. All CLI options work with `-p`.

```bash
claude -p "What does the auth module do?"
```

## Output formats

`--output-format` controls how responses are returned:

- `text` (default): plain text
- `json`: structured JSON with `result`, `session_id`, and metadata
- `stream-json`: newline-delimited JSON for real-time streaming

## Streaming with partial messages

Use `--output-format stream-json --verbose --include-partial-messages` to receive tokens as generated. Each line is a JSON object representing an event:

```bash
claude -p "Explain recursion" \
  --output-format stream-json --verbose --include-partial-messages
```

Filter text deltas with jq:

```bash
claude -p "Write a poem" \
  --output-format stream-json --verbose --include-partial-messages | \
  jq -rj 'select(.type == "stream_event" and .event.delta.type? == "text_delta") | .event.delta.text'
```

### Stream event types

- `system/init`: first event, reports session metadata (model, tools, MCP servers, plugins)
- `system/status`: status updates (`requesting`, etc.)
- `system/api_retry`: emitted before retrying a failed API call; fields: `attempt`, `max_retries`, `retry_delay_ms`, `error_status`, `error`
- `stream_event`: wraps Anthropic SSE events (`message_start`, `content_block_start`, `content_block_delta`, `content_block_stop`, `message_delta`, `message_stop`)
- `assistant`: the complete assembled assistant message (emitted while streaming with `--include-partial-messages`, and as the final whole message)
- `result`: final result with `subtype`, `duration_ms`, `num_turns`, `result` (text), `usage`, `session_id`, `total_cost_usd`

### `--include-partial-messages`

Requires `--print` and `--output-format stream-json`. Emits intermediate `assistant` message states during streaming.

## Session resume / continue

```bash
# Continue most recent conversation
claude -p "Follow-up question" --continue

# Resume a specific session by ID
session_id=$(claude -p "Start a review" --output-format json | jq -r '.session_id')
claude -p "Continue that review" --resume "$session_id"
```

Session ID lookup is scoped to the current project directory and its git worktrees. Both commands must run from the same directory.

## No-session-persistence

`--no-session-persistence`: disables session persistence (print mode only); sessions are not saved to disk and cannot be resumed.

## Bare mode (for CI)

`--bare`: skips auto-discovery of hooks, skills, plugins, MCP servers, auto memory, and CLAUDE.md. Recommended for CI/scripted calls where reproducibility matters.

## Background task timeout

Background Bash tasks started during `-p` run are terminated ~5 seconds after the final result. Background subagents/workflows wait up to 10 minutes (default) before timing out (adjustable via `CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS`).
