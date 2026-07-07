---
type: Practice
tags: [claude-cli, practice]
---

# Delta resume (persist on second use)

The client spawns a fresh CLI process per model call, so "session reuse" means
resuming the CLI's persisted conversation instead of re-piping the whole
transcript. State machine:

1. **First call** on a transcript: `--no-session-persistence`, full transcript
   piped. One-shot workloads (compaction, evals) never write session files.
2. **First append-only extension**: full transcript again, persistence ON;
   the `system/init` event's `session_id` is recorded [1].
3. **Later extensions**: `--resume <id>` with only the *suffix* piped
   (assistant turns skipped — the CLI session already holds its own replies).
   Resumed calls show `cache_read_input_tokens > 0` [2].

Cost shape: one extra full send per session (step 2) buys suffix-only sends for
every later round; no disk writes for single-shot callers. Any non-extension
(history rewritten by curation/compaction) resets to step 1 automatically —
see [prefix-invalidation](prefix-invalidation.md).

# Citations

1. [probe-resume-2-1-195](/sources/probe-resume-2-1-195.md)
2. [probe-resume-2-1-195](/sources/probe-resume-2-1-195.md)
