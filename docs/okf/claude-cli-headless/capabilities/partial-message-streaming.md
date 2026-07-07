---
type: Practice
tags: [claude-cli, capability]
---

# Partial Message Streaming

Passing `--include-partial-messages` (requires `--print` and
`--output-format stream-json`) causes the CLI to emit `stream_event` lines
that wrap Anthropic SSE events as they arrive: `message_start`,
`content_block_start`, `content_block_delta`, `content_block_stop`,
`message_delta`, and `message_stop` [1]. Without this flag only the final
`assistant` and `result` lines are emitted; no `stream_event` lines appear [1].

Text tokens arrive as `content_block_delta` stream events with
`.event.delta.type == "text_delta"` and text at `.event.delta.text` [2].
Example verbatim from the probe:

```json
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}},"session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","parent_tool_use_id":null,"uuid":"f7612ac4-3352-4d69-bdd4-deccecc0ee2b"}
```

The `assistant` message (type `"assistant"`) is emitted mid-stream when
`--include-partial-messages` is active, and again as the final whole message.
Its `content[0].text` contains the same accumulated text as all the
`text_delta` events combined. The `result.result` field also carries this same
text as a plain string [2]. Downstream consumers must choose one source —
deltas, the whole `assistant` message, or `result.result` — to avoid
double-counting.

# Citations

1. [headless-print-mode](/sources/headless-print-mode.md)
2. [probe-stream-json-2-1-195](/sources/probe-stream-json-2-1-195.md)
