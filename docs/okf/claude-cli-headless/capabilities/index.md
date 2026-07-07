# Capabilities

What the Claude CLI's headless (`-p`) surface offers to a programmatic caller.

- [session-resume](session-resume.md) — `--resume <id>` semantics, session file location, failure shape
- [partial-message-streaming](partial-message-streaming.md) — `stream_event` lines, text_delta path, dedup observation
- [thinking-output](thinking-output.md) — documented thinking_delta shape; not live-elicited at 2.1.195
- [model-knobs](model-knobs.md) — `--effort` allowed values and soft-warn behavior, `--fallback-model`
- [caching-economics](caching-economics.md) — `cache_read_input_tokens` evidence from the resume probe
