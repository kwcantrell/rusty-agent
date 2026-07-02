---
name: context-management
description: >-
  Use when an agent's context window is filling up, when you see a
  `[tool_result#N offloaded ...]` placeholder, or when recent actions feel like
  they've drifted from the original goal. Explains how to wield this runtime's
  curating context manager (offload table, compaction, re-grounding) well —
  what to trust, when to recall, and when to compact.
---

# Using the context manager well

The runtime curates its own context window every turn: it offloads stale tool
errors and large tool outputs into a retrievable table, compacts old history into
a summary when the window fills, and pins a re-grounding block restating the
original goal. This skill is judgment: how to work *with* that machinery.

**Do not** use this skill to run the optimization campaign (→ `context-evolve`)
or as internals documentation for the runtime's Rust context code — it is
generic judgment for working with the curating context manager at run time.

## The one big idea

**The live window is a working set, not a transcript.** The offload table is the
durable record. Pull detail back only when you actually need it.

## Non-negotiables

- **A `[tool_result#N offloaded ...]` stub is a pointer, not a loss.** Call
  `context_recall(N)` to rehydrate the full content. Never re-run a tool to
  regenerate output you already produced and offloaded.
- **Compaction is lossy on specifics.** The "Summary of earlier conversation"
  block keeps decisions and open threads, but drops verbatim detail. Before you
  rely on an exact value (a path, a number, an error message) for an edit,
  `context_recall` the raw entry by its id.
- **The re-grounding block is the original goal.** If your last few actions don't
  serve "Original goal: ...", you've drifted — stop and re-read it before acting.
- **Don't hoard.** Recall the one entry you need, not everything. Mass-recall
  refills the window with the bloat the offload pass just cleared.

## When to let it work vs. intervene

- **Let it work:** routine offload and windowing are automatic and safe — you do
  not need to manage them. Most turns need nothing from you.
- **Request `context_compact()`** only at a genuine high-water point or right
  before starting a long new sub-task, when the window is full of *resolved*
  sub-tasks whose detail you won't need verbatim.
- **Keep writes single-threaded.** If you delegate exploration to a subagent, let
  it explore read-only and hand back a short summary; don't fan out parallel
  writers (they disperse decisions and lose shared context).

## Reading the signals

- `Offloaded { id, bytes, tool }` — content moved to the table; recall by `id`.
- `Compacted { turns_replaced, tokens_before, tokens_after }` — old span summarized.
- `CompactionFailed { reason }` — nothing changed; the window still holds raw history.

## Red flags

| Thought | Reality |
|---------|---------|
| "I'll just re-run the tool to see that output again" | It's offloaded — `context_recall(N)` is cheaper and exact. |
| "The summary says the path was X, so I'll edit X" | Compaction is lossy — recall the raw entry by id and confirm. |
| "Context is full, I should give up" | Call `context_compact()` and continue. |
| "Let me recall everything to be safe" | That refills the window — recall only what this step needs. |
