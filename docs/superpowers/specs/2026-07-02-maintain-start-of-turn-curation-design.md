# Maintain-at-start-of-turn curation — design

**Date:** 2026-07-02
**Status:** approved for implementation (autonomous campaign round; design constraints
pre-set by the context-evolve resume brief)
**Scope:** `agent-core` (`loop_.rs`, `curated.rs`) + eval re-baseline protocol

## Problem

`maintain()` never runs on text-only turns. The text-reply exit in
`AgentLoop::run_with_cancel` (`loop_.rs`, the `all_calls.is_empty()` arm) returns
before the end-of-turn maintain at the loop bottom, so any run that ends in a plain
text reply leaves history uncurated — and a chat-only session (every run text-only)
gets **zero** curation ever: no offload, no compaction, no eviction visibility. The
only thing bounding its window is `build()`'s silent token eviction.

This is both a real product gap (chat sessions accumulate unbounded, uncurated
history) and the blocker for the context-evolve campaign's open discriminator
(longhaul-manifest): any fold/marker mechanism can only help if it runs *before*
the model call that needs it, and today curation never runs during the 20
fact-bearing ack turns at all.

### Why the naive fix failed (2026-07-02 evening round, reverted)

Moving maintain to start-of-turn exposes per-turn compaction:

- `over_high_water` is measured on the **built** context, which `build()` fills to
  the budget — once saturated it is ~always true, so maintain-every-turn ⇒
  compact-every-turn.
- Re-running the summarizer over `prior summary + one trivial ack` is generation
  loss: the 3B summarizer degrades the prior instead of extending it, collapsing a
  12-entry running summary to "No new information provided" within ~16 passes.
  `compaction_is_worthwhile` *accepts* the collapse (it is a huge token win).
- The attic's counter-measure — a flat `MIN_RECOMPACTION_SPAN_TOKENS=256` floor on
  any re-compaction — throttled **tool-bearing** spans too (post-offload portmap
  chatter is tiny), delaying compaction enough to regress locked-portmap 10/10→~4/6
  the other way.

The cadence is load-bearing in both directions. The fix must distinguish
*degenerate* spans (pure ack chatter) from *substantive* ones (tool-bearing), and
must make summary collapse impossible rather than merely less frequent.

## Design

Three coupled changes. (1) is the ordering fix; (2) and (3) are the cadence/
fidelity guards that make it safe.

### 1. Maintain at start-of-turn (`loop_.rs`)

Move the `MaintCtx` + `ctx.maintain(&deps).await` block from the loop bottom to the
top of the turn loop, after the cancel check and **before** `ctx.build(...)`. Remove
the loop-bottom copy. The overflow-recovery maintain (inside the completion retry
loop) stays as-is.

Consequences:

- **Every model call sees a freshly curated window**, including the just-appended
  user prompt — the invariant phase 2's fold trigger needs (a curation decision can
  react to the incoming prompt before the call it belongs to).
- Text-only runs now get exactly one maintain (at turn 0). Chat sessions are
  curated once per run.
- For tool-bearing runs the maintain count is unchanged; each maintain shifts one
  slot earlier (before the call instead of after the previous turn's results).
- The final reply of a run is never followed by a maintain; it is curated at the
  start of the next run. This is strictly consistent with the invariant above.
- Latency note: a compaction now sometimes runs before the first completion of a
  run (time-to-first-token cost ~one summarizer call) instead of after the reply.
  Accepted.

Alternatives considered:

- **Exit-path maintain** (keep loop-bottom maintain; add one at the text-only
  return): smaller behavioral delta (tool runs byte-identical), but leaves the
  pre-call blind spot — curation never sees the prompt of the call it serves, so a
  phase-2 fold triggered by the final assembly prompt would fire one run too late.
  Kept as the documented **fallback** if re-baseline regresses under (1).
- **Fidelity guard only** (no ordering change): does not fix the structural gap.
- **Both call sites** (start and end): doubles maintenance per tool turn. Rejected.

**Amendment 2 (re-baseline outcome): the fallback SHIPPED.** Start-of-turn held
every ceiling except memory-roster: paired at N=10 on the same server window,
v3 = 10/10 vs start-of-turn = 6/10. Every failure was a session-1 storage miss —
the model acked a "remember X" prompt without issuing the `remember` call, with
the instruction verbatim in-window. Mechanism: maintaining with the fresh user
prompt already appended pushes the split (`len - keep_recent`) one message
deeper, so the PREVIOUS remember tool-turn enters the compactable span one run
earlier; at the decision call the model's most recent visible template is
"user asks to remember → assistant just acks" (the tool call summarized away),
and it imitates it. The exit-path variant maintains before the next prompt
lands, so the newest tool unit snaps into the recent window and the template
survives. Ordering (1) is therefore replaced by: loop-bottom maintain stays
(tool turns byte-identical to v3) plus a maintain at the text-only exit, before
`Done`. Guards (2) and (3) are unchanged and not implicated. Phase-2 note: the
pre-call blind spot returns — a fold triggered by history growth is created at
the PREVIOUS run's exit, so fold hysteresis must leave headroom for one
incoming prompt.

### 2. Trivial-chatter skip (`curated.rs::compact_old_span`)

After the durable/summarizable partition, skip the summarizer when the span is
*degenerate*:

```
degenerate := every message in to_summarize has Role::Assistant
              AND span est-tokens < TRIVIAL_CHATTER_SPAN_TOKENS (256)
```

Return with history intact (like the existing `to_summarize.is_empty()` early
return). The chatter accumulates and is retried when the span grows past the floor
or gains a tool-bearing unit.

Unlike the attic's flat floor, this **exempts tool-bearing spans**: any span
containing a `Role::Tool` message (or a tool_calls parent unit) summarizes exactly
as today, regardless of size — portmap/drift cadence is preserved by construction.
The skip applies whether or not a prior summary exists (without one there is no
decay risk, but summarizing three "OK"s is a wasted model call either way; tokens
are the campaign tiebreak).

**Amendment (implementation finding):** an explicit `request_compaction()` — the
overflow-recovery imperative — **bypasses** the skip. The skip is a cadence
heuristic for routine high-water passes; a forced "shrink now" outranks it
(pinned by `tests/compaction_routing.rs` and a new unit test).

### 3. Monotone prior guard (`curated.rs::compact_old_span`)

In the `Ok(summary)` arm: if a prior summary exists and the candidate's est-token
size is **smaller than the prior's**, discard the candidate — keep the prior
summary and leave history untouched (same handling as "not worthwhile"). The
compaction prompt mandates a strict superset; a shrinking output is by definition
lossy. This makes collapse impossible rather than unlikely: a degrading pass is
rejected and simply retried later with a larger span, so no sequence of passes can
erase accumulated facts.

Bounded-growth note: the summary can now only grow while a prior exists, until
`compaction_is_worthwhile` stops accepting — at which point chatter accumulates in
history and `build()`'s priority eviction handles overflow (the existing
long-horizon behavior; genuinely unbounded sessions are phase-2 territory).

## What this change is NOT

- Not a champion promotion. It is a **baseline shift**, like calibrated budgeting:
  the semantics every admitted verdict was measured under change, so every admitted
  task is re-run and program.md gets a new baseline block (v3 code + these numbers).
- Not a manifest fix. longhaul-manifest is expected to stay ~0/5 after this; the
  change only enables phase 2's re-attempt.
- No fold/marker mechanism, no `over_high_water` redefinition, no summarizer prompt
  change. One hypothesis per iteration.

## Testing

Unit tests (ship with the change):

- `loop_.rs`: a text-only run performs a maintain; maintain happens before the
  model call (mock ctx already counts `maintains` — extend to assert ordering
  vs `build`).
- `curated.rs`:
  - degenerate span (tiny all-assistant chatter, prior present) → summarizer NOT
    invoked, prior + history intact;
  - same-size span containing a tool unit → summarizer IS invoked (cadence
    preservation);
  - scripted summarizer returns a shorter-than-prior summary → candidate discarded,
    prior kept, history intact;
  - larger-than-prior summary → accepted (existing tests cover the accept path).
- `stress_context_management.rs::repeated_compaction_*`: re-check the compaction
  count assertion under the new guards; adjust the bound with a comment if the
  batching legitimately lowers it (the attic precedent: ≥50 → ≥25).

Gate: `cargo fmt`, `bash scripts/ci.sh` green before merge.

## Re-baseline protocol (mandatory, before merge)

Server: llama-agent on :8080 (no /v1). All env paths ABSOLUTE. Window from the
config's `context_limit` (4000 ⇒ effective ~1000 est tok). Memory tasks:
`EVAL_REAL_EMBEDDINGS=1` + `FASTEMBED_CACHE=src-tauri/.fastembed_cache`.

Run the new-code champion (v3 config, `champion_v3.json`) on **every** admitted
task at the recorded N:

| task | N | v3 ceiling (hard gate) |
|---|---|---|
| locked-portmap | 10 | 10/10 |
| drift-ledger | 6 | 6/6 |
| offload-recall | 5 | 5/5 |
| longhaul-codename | 5 | 5/5 |
| memory-recall (real emb) | 5 | 5/5 |
| memory-roster (real emb) | 5 | 5/5 |
| longhaul-manifest | 5 | 0/5 (expected unchanged) |

Acceptance: **no admitted task loses a pass vs its v3 ceiling** (equal N;
`eval_gate` compares absolute counts). Manifest may improve but is not required to.
On any regression: first swap ordering (1) for the exit-path fallback and re-run;
if still regressed, revert the branch and log the dead end in program.md.

The resulting numbers become the new champion baseline block in program.md (still
v3 code+config plus this fix; NOT a v4 unless correctness improves somewhere).

## Files touched

- `agent/crates/agent-core/src/loop_.rs` — maintain relocation.
- `agent/crates/agent-core/src/curated.rs` — `TRIVIAL_CHATTER_SPAN_TOKENS`,
  degenerate-span skip, monotone prior guard, tests.
- `agent/crates/agent-runtime-config/tests/stress_context_management.rs` — bound
  adjustment if needed.
- `.agents/skills/context-evolve/program.md` — new baseline block (merge commit).
