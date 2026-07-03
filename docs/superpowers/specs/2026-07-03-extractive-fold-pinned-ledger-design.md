# Extractive fold to a pinned ledger — design

**Date:** 2026-07-03
**Status:** approved for implementation (autonomous campaign round, phase 2 of the
2026-07-02 maintain-ordering arc)
**Scope:** `agent-core` (`curated.rs`) + context-evolve eval round

## Problem

longhaul-manifest (admitted discriminator): favorable 5/5, champion 0/5. Twenty
padded fact-bearing user turns (~81 est tok each, ~1700 total) overflow the
effective window (calibrated limit ≈ 1590, retention budget ≈ 1350 after pinned).
CE_DEBUG diagnosis of the final assembly call under the current baseline
(2026-07-03):

- Window = system + goal block (pins entry #1) + a 28-tok vacuous summary +
  entries ~#6..#13-20 verbatim + assembly prompt. Entries **#2-5 are evicted with
  no surviving representation** — `plan_retention`'s user ladder drops oldest.
- The model transcribes **everything it can see, from both pinned and windowed
  sources, with zero dropout** (writes exactly goal#1 + windowed tail = 16/20).
  One observed run CONFABULATED the missing entries. The failure is pure absence.
- Pinned DATA is demonstrably used (the goal block's entry #1 appears in every
  manifest) even though pinned MARKERS demonstrably fail to elicit actions
  (attic #6b: 0/5, zero recalls).

Constraint math: verbatim retention of 20×81 tok cannot fit 1350; condensed
`name = value` lines ≈ 8 tok each → all 20 ≈ 160 tok fit trivially. The missing
entries need a compact, pinned, always-visible representation created before the
final call — not a recall action, not prose inside the running summary.

### Why not the attic family (all logged 0/5, never retried)

- #6a folded into the RUNNING SUMMARY: facts carried faithfully but embedded in
  prose; model transcribed 16/20 from it. Here the representation is a dedicated
  block in manifest-ready line format, not summary prose.
- #6b/#6e/#6f relied on `context_recall` elicitation (pinned marker ignored;
  in-history marker over-elicits; payload ≈ 1200 est tok can't fit the window
  anyway — recall-based designs are dimensionally doomed at this window).
- This design requires NO action from the model: the facts are simply present,
  like the goal block.

## Design

One mechanism in `CuratedContext`: **fold user units that retention would evict
into a pinned, append-only, extracted-facts ledger.**

### Trigger (sync, every maintain)

After the offload passes, before compaction: compute `plan_retention(history,
model_limit - pinned_tokens)`. If any `Role::User` unit falls outside the kept
ranges, fold. (Same imminent-eviction predicate as the attic fold; the exit
maintain from phase 1 makes this fire during chat-only ack runs, and the
low-watermark hysteresis leaves headroom for the next incoming prompt.)

### Selection (oldest-first, hysteresis)

Fold the OLDEST non-marker user units (never touching the `keep_recent` tail)
until the remaining user units fit half the user share of the budget
(`USER_FOLD_LOW_WATERMARK_PCT = 0.25` of model_limit — i.e. fold down to ~25%
so one more ~80-tok prompt cannot immediately re-trigger).

### Extraction (async, summarizer-tier model call)

One model call per fold batch (`deps.model`, like compaction) with a dedicated
system prompt: copy every concrete fact/setting/name/value from the given user
messages, one line per fact, verbatim values, no commentary; output only the
lines. Extracted lines are APPENDED to `folded_facts: Vec<String>`.

- On model failure: leave history untouched (retry at the next maintain), emit
  `CompactionFailed`-style debug log. Fold is all-or-nothing per batch.
- The ledger is **append-only and never re-summarized** — no generation loss by
  construction (the monotone lesson applied structurally).

### Ledger rendering (pinned)

`pinned()` renders, after the compaction summary, a System message:

```
Ledger of earlier user instructions (extracted verbatim; originals stored,
retrievable with context_recall(<id>) if needed):
- <line 1>
- <line 2>
```

Non-eliciting phrasing (the recall mention is informational). `pinned_tokens()`
includes it. Capped at `FOLDED_FACTS_MAX_TOKENS = 512` est tok: when appending
would exceed the cap, drop OLDEST lines first (and say nothing — the originals
remain in the offload store; a cap eviction is strictly no worse than today's
silent full eviction).

### Verbatim originals

The folded units' full text goes to the offload store as one entry
(`tool_name: "user_history"`, kind Output) per batch; the entry id is mentioned
in the ledger header. Folded units are then REMOVED from history (whole units).

### Interactions

- Compaction: runs after folding in the same maintain; folded units are gone
  from history so the durable-partition sees only surviving users. No change to
  the summarizer path, the trivia skip, or the monotone guard.
- `build()`: unchanged — the ledger arrives via `pinned()`.
- Tasks where users always fit (portmap ~240 tok, drift ~small): the trigger
  never fires; those cadences are untouched by construction. codename (13×280
  tok fillers): folds WILL fire; the extraction of filler prose may produce
  junk lines (cap bounds the cost); the codename fact itself is protected by
  the goal block. Guard sweep verifies.

## Testing

- Fold trigger: tight budget + user units beyond it → extraction called, oldest
  units removed, ledger lines present in `build()` output, store holds the
  verbatim originals.
- No-op: users fit → no model call, history untouched.
- Failure path: extraction model errors → history untouched, no ledger change.
- Ledger cap: appending past `FOLDED_FACTS_MAX_TOKENS` drops oldest lines.
- Ledger survives compaction untouched (pinned, not part of the summary).
- Existing suites green; `bash scripts/ci.sh` before merge.

## Eval protocol (campaign discipline)

- Paired longhaul-manifest N=5: champion leg = snapshot binary
  `eval_context_gated` (current main), candidate leg = new binary. Promote needs
  strictly more passes (champion is 0/5 → any pass wins correctness; `gate`'s
  0-passes token artifact expected).
- Full guard sweep at admitted N: portmap 10, drift 6, offload 5, codename 5,
  mem-recall 5, mem-roster 10 (real emb). No ceiling may regress (roster: judge
  by prefix-identity attribution + paired batch as per program.md).
- On promotion: champion v4 (config file unchanged params → `champion_v4.json`
  frozen copy), program.md champion block + iteration log, campaign-state memory
  re-stamp. On failure: log the dead end; the 14-entry smaller discriminator is
  the documented next step for signal headroom.

## Outcome (2026-07-03, post-implementation)

Three rendering iterations (standalone pinned block 0/5 → numbered 0/5 →
**merged into the goal block 5/5 at 20/20 entries per run**). Extraction
fidelity was perfect throughout; the variable was pinned-block salience — the
goal block is the only pinned region the model reads attentively every run.
Guard sweep clean (portmap 10/10, codename 5/5 with folds firing, drift 11/12
with the single miss model-bound, roster 9/10 == baseline, offload/recall 5/5).
Promoted as champion v4. Full record: program.md.
