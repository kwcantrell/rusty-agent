# Context Explorer — Design

**Date:** 2026-06-30
**Status:** Approved for planning
**Related:** `2026-06-24-context-dashboard-design.md`, `2026-06-23-memory-system-design.md`, `2026-06-24-memory-auto-retrieval-design.md`, `2026-06-23-skills-subsystem-design.md`, `2026-06-25-tauri-ipc-transport-design.md`, `2026-06-23-react-frontend-design.md`

## Summary

A new right-side pane/tab in the existing `web/` React app that turns the live **context window** into an interactive explorer. The context window is the spine; the **memory** and **skill** systems are two categories *inside* it, each drillable into a full management console. v1 ships visualization **and** CRUD together.

This supersedes the thin `ContextDashboard` summary bar by giving the context window a first-class, segmented, drill-in surface — and gives the memory and skill subsystems their first human-facing UI.

## Motivation

Today the operator is blind in three ways:

1. **Context window is a single number.** The backend sends one `prompt_tokens` total; there is no visibility into *what* fills the window (system prompt, skills, recalled memory, tool defs, messages).
2. **Memory is opaque and unmanaged.** `agent-memory` is a vector store (text + embedding + tags + scope) recalled by cosine similarity. There is no way to see what was recalled at a given turn, with what score, nor to browse/prune/re-tag the store. Worse, the running server wires `InMemoryStore`, so memory is ephemeral.
3. **Skills are invisible at runtime.** `discovered_skills` and `active_skills` flow to the frontend but are only rendered as a comma-joined string; there is no way to read or edit a skill body.

The Context Explorer addresses all three from one surface, anchored on the context window.

## Goals / Non-goals

**Goals (v1)**
- A `Context Explorer` tab alongside the existing `WorkspacePane`, inside `web/`.
- A live, segmented context-window breakdown (`/context`-style) updated each turn.
- Honest token accounting: real total as ground truth, per-segment estimates reconciled with an explicit `unattributed` slice.
- Memory: per-turn recalled rows with cosine scores **+** full CRUD (browse, search, filter by scope/tag, edit text/tags, delete) against a **durable** store.
- Skills: list discovered + active, view and edit `SKILL.md` bodies.
- Swap the server from `InMemoryStore` to `SqliteStore` (durability prerequisite for CRUD) — in scope.

**Non-goals (v2+)**
- Embedding-similarity 2D "corpus map" projection (graphify covers corpus-shape insight for now).
- Creating skills from scratch in-UI (edit existing only).
- Undo/history, soft-delete, audit log.
- Cross-project store management UI beyond a scope filter.

## Constraints discovered in the codebase

- **Transport is Tauri `invoke()`** (`web/src/socket.ts`): `subscribe` (event Channel), `send_input`, `approve`, `settings_get`, `settings_update`. CRUD = new commands of the same shape. No websocket/CORS work needed.
- **Context assembly is explicit and ordered** (`agent-core/src/curated.rs`, `context.rs`): `system prompt → recall block (auto-retrieved memory, capped at recall_budget) → message history`. Each block is a real, capturable unit.
- **Recall is auto-injected per turn** (`memory_auto_retrieval`), not only via the model's `recall` tool. So "memory currently in context" is well-defined.
- **Token estimator undercounts.** `message_tokens` ignores reasoning + tool_calls (known issue). The design treats it as an estimate, never as ground truth.
- **`SqliteStore` already exists** in `agent-memory/src/store.rs`; the server just doesn't wire it (`agent-server/src/setup.rs:78` uses `InMemoryStore`).

## Architecture

```
┌─ web/ (existing React app + vite) ──────────────────────────┐
│  Left: chat   │  Right: [ Workspace | Context Explorer ]    │  ← new tab
└────────────────────────────────────────────────────────────┘
   │ invoke() commands (existing bridge)     ▲ Channel events (existing)
   ▼                                          │
┌─ src-tauri bridge ──────────────────────────────────────────┐
│  NEW commands:  memory_list / memory_get / memory_update /   │
│                 memory_delete / skill_list / skill_get /     │
│                 skill_save / context_block_get               │
│  NEW event:     context_snapshot (pushed each turn)          │
└────────────────────────────────────────────────────────────┘
   │
┌─ agent-server / agent-core / agent-memory / agent-skills ────┐
│  SqliteStore wired in place of InMemoryStore                 │
│  Retriever surfaces Scored[] (id, text, scope, score)        │
│  ContextManager emits per-block token attribution            │
└────────────────────────────────────────────────────────────┘
```

## Components

### 1. Backend: context snapshot

A new structured event emitted by `ContextManager` (`curated.rs`) at the point the prompt is assembled each turn. Carries **token counts + previews only** (full text fetched lazily on drill-in).

```
ContextSnapshot {
  turn: u32,
  total_prompt_tokens: u32,      // ground truth from model usage when available
  context_limit: u32,
  segments: [
    { category: "system",   est_tokens, preview }       // preview = first N chars
    { category: "skills",   est_tokens, items: [skill_name…] }
    { category: "memory",   est_tokens, rows: [{ id, score, text_preview, scope }] }
    { category: "tools",    est_tokens, items: [tool_name…] }
    { category: "messages", est_tokens, count }
  ]
}
```

- `unattributed = max(0, total_prompt_tokens − Σ est_tokens)` is computed **frontend-side** and rendered as its own slice. The backend never fabricates a reconciliation.
- The memory `rows` require the retriever to return `Scored[]` (id, score, scope, text) alongside the rendered recall block — a small addition to the auto-retrieval path. Cap rows to the recall_budget set actually injected.
- Emitted over the existing `subscribe` Channel as a new frame `kind: "context_snapshot"` (extend `wire.ts` `Inbound` union + `state.ts` reducer).

### 2. Backend: durable store + CRUD commands

- **Swap to `SqliteStore`** in `agent-server/src/setup.rs`: db path under the workspace/agent data dir, create-if-missing, run the store's schema init. Reuse the existing embedder (mind vector dimension — a dimension change invalidates stored vectors; document the chosen embedder/dim in the plan).
- **New Tauri commands** wrapping `MemoryStore` + `SkillRegistry`:
  - `memory_list({ scope_filter, query?, tag?, limit, offset }) -> [MemoryRow]`
  - `memory_get({ id }) -> MemoryRecord` (full text)
  - `memory_update({ id, text?, tags? }) -> MemoryRecord` (re-embed on text change)
  - `memory_delete({ id }) -> bool`
  - `skill_list() -> [DiscoveredSkill]` (reuse `discovered_skills` shape)
  - `skill_get({ name }) -> { name, description, body, files }`
  - `skill_save({ name, body }) -> {}` (writes `SKILL.md` to the registry **writable root**; re-scan)
  - `context_block_get({ category, ref }) -> { text }` (lazy full-text for drill-in)
- Commands mirror the `settings_get/settings_update` request/response convention already in `socket.ts`. Scope-safety: memory writes/deletes respect the same scope guard as the `forget` tool (no cross-project deletes).

### 3. Frontend: the Context Explorer pane

Lives in `web/src/components/inspector/` (next to the existing Workspace pane); registered as a second tab in the right-side container. Reuses existing theme tokens, `MarkdownText`, and transport.

- **Header:** segmented bar (real total / limit / %), one colored segment per category + `unattributed`. Click a segment → expands its section. Driven by the latest `context_snapshot` in state.
- **Memory section:**
  - *This turn:* recalled rows with cosine score, scope badge, text preview (from snapshot).
  - *Browser:* search box + scope/tag filters → `memory_list`; row actions edit (inline text/tags, `memory_update`) and delete (`memory_delete`, confirm). Live store edits reflect on the next turn's snapshot automatically (recall re-runs each turn).
- **Skills section:** discovered list with active-marker; click → `skill_get` opens body in a viewer/editor; save → `skill_save`.
- **Empty/again states:** before the first snapshot, show the existing summary numbers (graceful degrade from `ContextDashboard`).

`ContextDashboard` is retained as the collapsed/summary affordance; the Explorer is its expanded home. No duplicate sources of truth — both read the same `usage`/snapshot state.

## Data flow (one turn)

1. Agent turn begins → `ContextManager` assembles `system → recall → history`, computes per-block estimates, and the retriever yields `Scored[]`.
2. Backend emits `context_snapshot` over the Channel → `state.ts` reducer stores it as `latestSnapshot`.
3. Explorer renders the segmented bar + sections from `latestSnapshot`.
4. User clicks a memory row → `context_block_get`/`memory_get` fetches full text on demand.
5. User edits/deletes a memory → `memory_update`/`memory_delete` → SqliteStore mutated → reflected on the next snapshot.

## Error handling

- **Missing real total:** if model `usage` is unavailable for a turn, show the estimated sum and label the total `~estimate` (no fake precision); `unattributed` slice hidden in that case.
- **Estimate over total:** clamp `unattributed` at 0; never render negative slices.
- **Command failures** (store I/O, skill write): surface inline per the `settings_error` pattern; the pane stays usable.
- **Scope guard violation:** `memory_delete`/`memory_update` on a non-current-project record is rejected backend-side; UI shows the rejection.
- **Concurrent edit vs. live recall:** acceptable — recall re-runs each turn, so a deleted memory simply stops appearing; no locking needed in v1.

## Testing

- **Rust:** snapshot emission produces correct categories + monotone-ish estimates (unit on `ContextManager`); retriever returns `Scored[]` with scores; SqliteStore CRUD + scope-guard rejection (extend existing `forget` scope tests); command handlers happy-path + error-path.
- **Frontend (vitest):** reducer folds `context_snapshot` into `latestSnapshot`; segmented bar computes `unattributed` and clamps at 0; memory edit/delete round-trips against a mocked `invoke`; skill edit save path; graceful degrade before first snapshot. Follows existing `*.test.tsx` patterns (`ContextDashboard`, `SettingsPanel`).
- **Honesty checks:** explicit test that a known estimator undercount yields a visible `unattributed` slice rather than a silently-wrong 100%.

## Open questions / risks

- **Embedder dimension on the Sqlite swap:** if the wired embedder differs in dim from any pre-existing rows, cosine returns NaN (already skipped with a warning in `store::rank`). Plan must pick one embedder and start from a clean db, or document a re-embed migration.
- **Per-category token attribution fidelity:** acceptable as estimates in v1; the `unattributed` slice makes the gap honest. Tightening `message_tokens` (reasoning + tool_calls) is tracked separately and out of scope here.

## v1 scope fence

**In:** explorer tab; `context_snapshot` event + segmented bar + `unattributed` reconciliation; memory recalled-rows-with-scores; memory browse + edit + delete; skill view + edit; `SqliteStore` swap.

**Out:** corpus-map 2D projection; in-UI skill creation; undo/history; cross-project management beyond a filter; tightening the token estimator.
