---
type: Practice
tags: [claude-cli, capability]
---

# Thinking Output

When thinking is active, the Anthropic SSE protocol emits a
`content_block_delta` event with `.event.delta.type == "thinking_delta"` and
partial reasoning at `.event.delta.thinking`. The final assembled `assistant`
message includes a `{"type":"thinking","thinking":"...full reasoning..."}` block
in its `content[]` array alongside the text block [1].

**These shapes were not live-validated at claude 2.1.195.** The probe was run
twice — once with `--model opus --effort high` and once with
`--model sonnet --effort high` — and neither run emitted `thinking_delta` events
or a `thinking` content block. The `message_delta` usage field showed
`output_tokens_details.thinking_tokens: 0` in both cases [1].

Task 3 retains parser arms for both the `thinking_delta` stream event and the
`thinking` content block based on the documented Anthropic SSE shapes. If a
future CLI version or configuration elicits thinking, these arms will handle it;
until then they are dead code paths. Re-probe after any CLI major-version bump
that changes extended thinking behavior.

# Citations

1. [probe-stream-json-2-1-195](/sources/probe-stream-json-2-1-195.md)
