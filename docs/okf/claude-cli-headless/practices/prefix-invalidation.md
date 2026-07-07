---
type: Practice
tags: [claude-cli, practice]
---

# Prefix Invalidation

The delta-resume state machine is only valid when the new transcript is a
strict extension of the prior one — meaning every message the CLI session
already holds is byte-for-byte identical to the corresponding message in the
current transcript. If the context manager curates, compacts, or otherwise
rewrites history, the stored session no longer matches the new transcript prefix,
and resuming it would corrupt the conversation.

The client implements a fingerprint per message (hash of role + name + content +
reasoning) and checks that the stored session prefix matches the current
transcript up to the last recorded message. If any fingerprint diverges, the
session is treated as invalidated: the stored session ID is discarded, and the
next call falls back to a full-context send without `--resume`. The
`--resume` failure path is observable — a bogus session ID causes the CLI to
exit 1 with a plain-text stderr error and no JSON on stdout [1]; this is the
same outcome an invalidated session produces, and the client's error handler
resets to step 1 of the delta-resume state machine on any exit-1 from
`--resume`.

Decoupling prefix-matching from the context manager means curation and
compaction decisions in `agent-core` require no special signaling to the
`agent-model` claude_cli layer: the client detects the rewrite automatically on
the next call and recovers silently. The loop's retry for that turn lands on a
fresh session (full-context send), so no turn is dropped.

# Citations

1. [probe-resume-2-1-195](/sources/probe-resume-2-1-195.md)
