# Advanced Context Management — Design Spec

**Date:** 2026-06-25
**Status:** Approved (brainstorming) → ready for writing-plans
**Crates touched:** `agent-core` (context + loop), `agent-tools` (context tools)
**Companion artifact:** `.agents/skills/context-management/SKILL.md`

## Problem

The runtime's only context strategy today is naive sliding-window eviction in
`agent-core/src/context.rs`: `WindowContext` pins the system prompt + a capped
recall block, then evicts oldest history turns newest-first until the estimate
fits `model_limit` (`build_keeps_system_and_drops_oldest_when_over_limit`). This
has two failure modes for long-running agents:

1. **Bloat from tool output.** Every tool call appends one `Message::tool(...)`
   carrying the full result, including verbose `ERROR: …` text
   (`loop_.rs:288`, error path `:363`). A single multi-tool step can dump
   kilobytes of stack traces that crowd out useful history and get evicted
   wholesale (losing recoverable signal) the moment the window fills.
2. **Context drift / context rot.** As the window fills with stale turns, the
   model's attention smears away from the original goal. Eviction is recency-only;
   it has no notion of what serves the task.

## Goal

A context-management subsystem that **dynamically curates** the window:

- Auto-offload stale tool errors and large tool outputs into a retrievable
  table, leaving a compact placeholder — reclaiming tokens without destroying
  the data.
- Compact an over-full window into a high-fidelity summary, and re-ground the
  model on the original prompt to fight drift.
- Give the model agency to pull offloaded detail back (`context_recall`) and to
  request compaction (`context_compact`), while the runtime handles the cheap,
  safe ops deterministically every turn.

## Research grounding

From the deep-research pass (24 verified claims; sources: Anthropic
context-engineering & multi-agent-research blogs, the Claude context-editing docs
[`clear_tool_uses_20250919`, `memory_20250818`], Cognition "don't build
multi-agents", MemGPT/Letta arXiv:2310.08560, Chroma context-rot). Five proven
SOTA techniques, mapped to this design:

| Technique | Source | Where it lands here |
|---|---|---|
| Tool-result clearing (`clear_tool_uses_20250919`) | Anthropic | `OffloadPolicy` — replace tool_result content with a placeholder, keep the tool_use record |
| Context offloading / structured note-taking (Memory tool) | Anthropic | `OffloadStore` + `context_recall` |
| Compaction (summarize near-full window) | Anthropic / Cognition | `Compactor` |
| Recitation / re-grounding on the goal | Anthropic | re-grounding block in `CuratedContext::build` |
| Subagent context isolation, **read-only** | Anthropic / Cognition | `Compactor` runs as a read-only summarizer pass |

Honest caveats carried into the design:
- Compaction is **lossy on obscure detail** → every offloaded raw entry stays
  recoverable by id, so detail is never truly lost.
- **No proven content-relevance *scoring* mechanism** exists → we deliberately do
  **not** build per-turn keep/drop/compress verdicts in v1 (YAGNI; experimental).
- A clean-context **review** subagent was *refuted* (0-3) as improving quality —
  but our subagent does **compaction**, which Cognition explicitly endorses, and
  it is **read-only** (proposes; the loop applies). Writes stay single-threaded.

## Decisions (locked in brainstorming)

- **Deliverable:** Rust subsystem **and** a `graphify-best-practices`-style
  guidance skill.
- **Control model:** **hybrid** — deterministic offload/eviction runs every turn
  with no model call; the model gets tools for the judgment calls.
- **Offload scope:** stale tool **errors** + **large** tool outputs, size-gated,
  with an `exclude_tools` escape hatch. Rewrites only `tool_result` content.
- **Store:** `OffloadStore` trait, `InMemoryOffloadStore` for v1, pluggable later
  (can be backed by `agent-memory` or a DB without touching the loop).
- **Filter subagent:** compaction summary **+** re-grounding block. Per-turn
  relevance verdicts **cut** for v1.
- **Safety:** maintenance is best-effort and must **never break a turn**;
  windowing remains the floor that provably fits `model_limit`.

## Architecture

Keep the existing sync `ContextManager` trait + `WindowContext` as the windowing
primitive. Add a curation layer on top. The only trait change is one **defaulted
async method**, so `WindowContext` and every existing test keep working untouched.

### Components

1. **`OffloadStore` trait + `InMemoryOffloadStore`** (`agent-core`)
   - `put(entry) -> OffloadId`, `get(id) -> Option<OffloadEntry>`, `list()`.
   - `OffloadEntry { id, tool_call_id, tool_name, kind: Error|Output, content, bytes, turn }`.
   - v1 impl is a `HashMap` behind the trait. Shared with tools via `Arc<dyn OffloadStore>`
     (interior mutability, e.g. `Mutex`, matching the `MemoryStore` pattern).

2. **`OffloadPolicy`** (`agent-core`) — pure, sync, no I/O.
   - Input: `&[Message]` + `OffloadConfig { error_min_bytes, output_min_bytes,
     stale_after_turns, keep_recent, exclude_tools }`.
   - Output: `Vec<OffloadHit { history_index, entry, placeholder }>`.
   - Placeholder example:
     `"[tool_result#7 offloaded: 3.1KB error from \"shell\" — recall with context_recall(7)]"`.
   - **Only** targets `tool_result` messages; rewrites content, preserves
     `tool_call_id` + name so the assistant↔tool linkage stays API-valid.

3. **`Compactor`** (`agent-core`) — async.
   - Input: old history span + original goal + `Arc<dyn ModelClient>` + cancel token.
   - Output: `Result<Message>` — one high-fidelity summary (decisions / open
     threads / key facts). Reuses the loop's model client (or a configurable
     cheaper summarizer). Also refreshes the re-grounding block from the goal.

4. **`CuratedContext`** (`agent-core`) — new `ContextManager` impl.
   - Holds: `system`, `goal: Option<Message>`, `history`, `recall`,
     `compaction_summary: Option<Message>`, `store: Arc<dyn OffloadStore>`,
     `config`, `high_water_pct`.
   - `build(model_limit)` assembly order (pinned items survive eviction like the
     recall block does today):
     `system → re-grounding → recall → compaction_summary → windowed recent history`.
   - `set_goal(text)` — captured from the first user input.
   - `maintain(&mut self, deps) -> MaintReport` (async): offload pass (sync,
     every turn) + compaction pass (async, gated by high-water mark).

5. **Model-facing tools** (`agent-tools`)
   - `context_recall(id)` → `store.get(id)` → returns full content as that call's
     `ToolOutput`; unknown id → a normal tool error result.
   - `context_compact()` → signals a compaction pass on the next `maintain`.

6. **Trait + loop change** (`agent-core`)
   - Add to `ContextManager`:
     `async fn maintain(&mut self, _deps: &MaintCtx<'_>) -> MaintReport { MaintReport::default() }`
     (defaulted no-op; object-safe via `async_trait`, already a workspace dep).
   - `MaintCtx<'a> { model: &'a Arc<dyn ModelClient>, goal, sink, cancel }`.
   - `loop_.rs` calls `ctx.maintain(...)` once per turn after tool results are
     appended, before the next `build()`.
   - New `ContextEvent` variants on the event sink: `Offloaded`, `Compacted`,
     `CompactionFailed`.

## Data flow

### Per-turn lifecycle (`loop_.rs`; ★ = new)

```
run_with_cancel(ctx, user_input):
  1. retriever recall  → ctx.set_recall(lines)        [exists :156]
  2. ctx.append(user)                                  [exists :159]
  2b. ★ first turn: ctx.set_goal(user_input)
  loop:
    3. messages = ctx.build(model_limit)               [exists :173]
    4. model.stream(req) → assistant turn              [exists :92]
    5. ctx.append(assistant)                           [exists :206/229]
    6. if no tool_calls → done
    7. execute tool_calls (parallel)                   [exists :256]
    8. ctx.append(tool result) per call                [exists :288]
    9. ★ ctx.maintain(&MaintCtx{ model, goal, sink, cancel })
    → back to 3
```

### `maintain()` internals

```
maintain(deps):
  # (a) deterministic offload — sync, cheap, every turn
  for hit in OffloadPolicy::select(&history, &config):
      id = store.put(hit.entry)
      history[hit.index].content = hit.placeholder      # keep id + name
      emit ContextEvent::Offloaded { id, bytes, tool }

  # (b) compaction — async, gated
  if built_tokens(&self.build(limit)) > limit * high_water_pct:   # reuse built_tokens()
      span = old history before keep_recent turns
      summary = Compactor::run(span, &self.goal, deps.model, deps.cancel).await?
      replace span with [summary]; self.compaction_summary = summary
      emit ContextEvent::Compacted { turns_replaced, tokens_before, tokens_after }
```

### Recall path (model agency)

`context_recall(7)` → `store.get(7)` → full content returned as a fresh tool
result at the live tail. A stub offloaded 20 turns ago returns *recent* exactly
when the model decides it needs the detail. The table is the durable record; the
live window is the working set.

## Error handling

Curation is best-effort. A failure degrades to "more tokens in context," never a
crashed loop or malformed history.

| Failure | Handling |
|---|---|
| Compactor model call fails / times out / cancelled | `Compactor::run` → `Err`; `maintain` logs `warn`, emits `CompactionFailed`, **leaves history untouched**. Next turn windowing still evicts oldest → degrades to today's behavior. No partial replacement committed. |
| Compaction produces empty / larger-than-span summary | Discard; keep raw turns. Compaction commits only on a net token win. |
| `context_recall(id)` unknown / evicted id | Tool returns `"no offloaded entry #id (may have been cleared)"` — fed back like any tool error (`loop_.rs:363`); model continues. |
| Offload would break tool linkage | Policy only rewrites `tool_result` content; never drops a message or touches assistant `tool_calls`. Asserted in tests. |
| High-water not reached but window overflows | `build()` keeps existing newest-first eviction as the backstop — compaction is an optimization over a windowing floor that already fits `model_limit`. |
| Recall re-inflates past limit | Recalled content is a normal tool result, windowed on the next `build()`; can't permanently blow the budget. |

## Testing

Mirrors the repo's split: pure unit tests in `context.rs`, async e2e in
`loop_.rs` / `runtime-config` with a mock `ModelClient`.

- **`OffloadPolicy` (unit, no async):** error under threshold not offloaded;
  large success offloaded once stale; recent N never offloaded; `exclude_tools`
  honored; placeholder preserves `tool_call_id` + name; assistant↔tool_result
  pairing intact after offload.
- **`InMemoryOffloadStore` (unit):** put→get round-trips full content; unknown id
  → `None`; ids stable/unique.
- **`CuratedContext::build` (unit):** assembly order; re-grounding block survives a
  tiny `model_limit` (pinned, cf. `context.rs:238`); summary counts against budget.
- **`maintain` offload pass (unit, stub store, no model):** stale errors / large
  outputs become placeholders and land in the store; idempotent (no double-offload).
- **`Compactor` + compaction (e2e, mock model):** scripted summarizer shrinks a
  span; token count drops; recent turns kept verbatim; **failure path** (`Err`)
  leaves history unchanged.
- **Recall round-trip (e2e):** offload → model calls `context_recall(id)` → full
  content reappears as a fresh tool result. Extends the parallel-tools e2e harness.
- **Regression:** existing `WindowContext` tests pass unchanged (defaulted
  `maintain` = no-op proves backward compat).

## Guidance skill: `.agents/skills/context-management/SKILL.md`

A judgment skill in the `graphify-best-practices` mold (when/how to wield the
capability well), complementing — not restating — the code. Audience: any agent
equipped with the context tools.

Planned sections:
- **Frontmatter** `name` + `description` with triggers (context filling, seeing
  an offload placeholder, output drifting off-goal).
- **One big idea:** the live window is a *working set*, not a transcript; the
  offload table is the durable record — pull detail back only when needed.
- **Non-negotiables:**
  - A `[tool_result#N offloaded …]` stub is a *pointer, not a loss* —
    `context_recall(N)` rehydrates it; don't re-run the tool to regenerate output.
  - Compaction is lossy on specifics — recall the raw entry by id before trusting
    a summarized fact for an edit.
  - The re-grounding block is the original goal; if recent actions don't serve it,
    you've drifted — re-read it.
  - Don't hoard — recall the one entry you need, not everything.
- **When to let it work vs. intervene:** trust deterministic offload/eviction;
  reach for `context_compact()` only at genuine high-water or before a long new
  sub-task; keep writes single-threaded (Cognition caveat).
- **Reading placeholders & events:** interpreting `Offloaded` / `Compacted` /
  `CompactionFailed`.
- **Red-flags table:** "I'll just re-run the tool" → recall it; "the summary said
  X so X" → verify raw by id; "context's full, give up" → compact then continue.

## Out of scope (v1 / YAGNI)

- Per-turn relevance keep/drop/compress scoring (unproven; experimental).
- Cross-session / persisted offload store (trait seam left for it; not built).
- Semantic recall over offloaded entries (recall is by explicit id in v1).
- Server-side Anthropic context-editing integration (this is a client-side,
  local-runtime design).

## Build order (for writing-plans)

1. `OffloadStore` trait + `InMemoryOffloadStore` + `OffloadEntry` (+ unit tests).
2. `OffloadConfig` + `OffloadPolicy` pure selection (+ unit tests).
3. `CuratedContext` skeleton: fields, `build()` assembly, `set_goal` (+ unit tests);
   defaulted `maintain` on the trait.
4. Wire `maintain` offload pass + `ContextEvent`s into `CuratedContext` and the
   loop (+ unit/e2e).
5. `Compactor` + gated compaction pass + failure handling (+ e2e with mock model).
6. `context_recall` / `context_compact` tools + registry wiring (+ e2e round-trip).
7. `.agents/skills/context-management/SKILL.md`.
