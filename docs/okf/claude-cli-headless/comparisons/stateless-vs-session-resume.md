---
type: Comparison
tags: [claude-cli]
---

# Stateless vs Session Resume

**Stateless** (full-send-every-call): every model call re-pipes the entire
transcript from scratch. Each call pays full input-token cost for every prior
message in the conversation. This is simple — no session file management, no
session ID tracking, no prefix fingerprinting — and is always safe regardless
of how the context manager modifies history between calls.

**Session resume** (delta-resume): the first extension call pays full input-token
cost to populate the session; subsequent calls pipe only the new suffix [1].
The cache evidence from the resume probe shows `cache_read_input_tokens: 22288`
on the resumed call, compared to `input_tokens: 2` for the new suffix alone [1].
For a conversation with many tool rounds this compounds: N rounds after the
session is warm = N suffix-only sends instead of N full-context sends.
Total API-call duration also improves: the resume probe measured `duration_api_ms`
of 2492 on the full first leg versus 1316 on the resumed leg [1].

The cost of a context rewrite (curation or compaction event): two full sends —
the compaction call itself (one-shot, `--no-session-persistence`) plus the
following extension call that re-populates the session [1]. After that, the
session is warm again for subsequent rounds.

When stateless is still right: `claude_session_reuse: false` in the runtime
config disables session resume entirely. Useful for evals, CI pipelines, and any
workload where reproducibility or session-file disk usage matters more than
per-round token savings.

# Citations

1. [probe-resume-2-1-195](/sources/probe-resume-2-1-195.md)
