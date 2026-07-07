---
type: Practice
tags: [claude-cli, capability]
---

# Caching Economics

When a session is resumed with `--resume <id>`, the CLI session holds the prior
conversation on the server side and only the suffix (the new user turn) needs to
be transmitted. This is confirmed by the resume probe: leg 2 (the resumed call)
showed `cache_read_input_tokens: 22288` in both the `assistant` and `result`
lines, while `input_tokens` was only 2 (the new prompt tokens alone) [1].

Leg 1 (the initial call, no `--resume`) showed `cache_read_input_tokens: 15934`
with `cache_creation_input_tokens: 6354` — the system prompt and prior context
were split between a cache hit and a new write [1]. On leg 2, with the session
already warm, virtually the entire context (22288 tokens) came from the cache
read at a fraction of the input-token cost.

The economics favour resuming for any conversation that extends beyond a single
round: one extra full-context send (to populate the session) buys suffix-only
sends for all subsequent rounds. One-shot workloads (evals, single compaction
calls) should use `--no-session-persistence` to avoid writing unnecessary session
files [1].

# Citations

1. [probe-resume-2-1-195](/sources/probe-resume-2-1-195.md)
