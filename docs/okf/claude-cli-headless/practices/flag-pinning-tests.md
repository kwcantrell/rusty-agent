---
type: Practice
tags: [claude-cli, practice]
---

# Flag Pinning Tests

Every flag the client passes to the CLI subprocess is load-bearing: removing one
silently changes behavior in ways that may not surface as an immediate error. The
probe commands establish which flags are required for correct headless operation [1]:

- `--output-format stream-json`: without this, output is plain text or JSON blob;
  the stream parser would fail or see no events.
- `--verbose`: required to get the `system/init` event with the `session_id`.
- `--include-partial-messages`: without this, no `stream_event` lines are emitted;
  token streaming is unavailable.
- `--allowedTools ""`: prevents the CLI from running tools the agent-sandbox and
  agent-policy layers haven't approved.
- `--no-session-persistence`: for one-shot calls (evals, compaction); omitting it
  writes unnecessary session files.
- `--setting-sources project`: scopes configuration loading; omitting it may load
  user-level hooks and settings that alter runtime behavior.
- `--strict-mcp-config`: prevents the CLI from discovering MCP servers beyond
  those explicitly configured; omitting it may attach unexpected tool sets.

The test pattern uses a fake CLI binary (a script or Rust subprocess mock) that
inspects the argv it receives and immediately exits with a failing stream if a
required flag is absent. This is a proc-level integration test that catches flag
regressions without needing a live Claude account.

# Citations

1. [probe-stream-json-2-1-195](/sources/probe-stream-json-2-1-195.md)
