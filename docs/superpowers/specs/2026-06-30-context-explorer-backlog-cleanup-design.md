# Context Explorer backlog cleanup — design

**Date:** 2026-06-30
**Source backlog:** `docs/superpowers/specs/2026-06-30-context-explorer-backlog.md`
**Status:** design approved; ready for implementation plan.

## Goal

Clear all 18 deferred Minor/polish findings from the Context Explorer review. None affect
correctness; the feature is merged, green, and live. This sweep hardens error paths, removes
dead/ drift-prone code, adds missing tests, and documents intentional-by-design decisions so
they are not "fixed" later.

## Scope decisions (locked)

- **All 18 items** are in scope. Items marked *intentional* (3, 11, 13) are addressed with a
  clarifying code comment — no behavior change.
- **Item 1** (`MemoryAdmin::get`): wire it in (make it live) rather than delete. See below.
- **Item 17** (handler-list drift): a shared macro, not just adding the missing entries.

## Delivery

One branch: `chore/context-explorer-backlog`. Four commits, by area, each building
independently with `cargo test` / frontend build green:

1. Rust backend fixes (items 1–5)
2. Frontend fixes (items 6–11)
3. Handler macro (item 17) — **before** the new tests so they run against the drift-proof list
4. Tests (items 12–16, 18)

No user-visible behavior change except the item-1 existence-leak fix and the newly surfaced
load-error messages (items 8, 10).

---

## Commit 1 — Rust backend (`agent/crates/agent-memory`, `agent/crates/agent-server`)

### Item 1 — `MemoryAdmin::get` wired in; existence leak closed

`MemoryAdmin::get` is currently dead (no caller); `update`/`delete` re-implement
`store.get` + `editable`. `get` returns a `MemoryRow` projection which lacks the `vector`
that `update` needs to re-embed — so the three methods share a **private** helper, not the
public `get`.

Introduce:

```rust
// Returns the full record iff it exists AND is editable in this scope.
// Ok(None) for both "missing" and "out of scope" — callers cannot distinguish
// the two, so we never leak that an out-of-scope record exists.
async fn fetch_editable(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError> {
    Ok(match self.store.get(id).await? {
        Some(rec) if self.editable(&rec.scope) => Some(rec),
        _ => None,
    })
}
```

Rewire:

- `get(id)` → `fetch_editable` → map `MemoryRecord` to `MemoryRow` (was: `Some(_) => Err(refused)`).
- `delete(id)` → `fetch_editable(id)?.is_some()` ? `store.delete(id)` : `Ok(false)`.
- `update(id, …)` → `fetch_editable(id)?` → `Some(rec)` re-embed + upsert → `MemoryRow`;
  `None` → `Err(StoreError::Io("not found".into()))`.

**Contract change (intentional):** out-of-scope records were previously distinguishable via a
`"refused: record belongs to another project"` error from `get`/`update`/`delete`. After this
change they are indistinguishable from missing: `delete` → `Ok(false)`, `update` → `not found`,
`get` → `Ok(None)`. This removes the existence leak across all three, not just `get`. The
`editable`/`refused` string is no longer returned; grep for any test or caller asserting on it
and update. (No production caller relies on distinguishing the two — the admin surface is the
Tauri command layer, which treats both as "nothing to do".)

### Item 2 — `recall_preview` logs before swallowing

Add `tracing::warn!(error = %e, "recall_preview failed")` (or equivalent) before the
`Err(_) => Vec::new()` so embedder/store/dimension-mismatch failures are visible.

### Item 3 — comment on intentional `Session.workspace` divergence

One-line comment on the `Session.workspace` field in `agent-server/src/session.rs` explaining
that `set_workspace` intentionally updates only the `Session` copy (so memory scope follows the
current workspace) and not `RuntimeState`'s copy (the run loop keeps its own).

### Item 4 — `skill_get` normalizes lookup key

In `session.rs`, `sanitize_slug` the incoming `name` before `find()`; on sanitize error, fall
through to the raw key. Preserves current behavior for already-slugged names, hardens non-slug
callers.

### Item 5 — `skill_save` newline guard

Defensive `desc.replace('\n', " ")` before interpolation in `skill_save`. Very low risk today
(`parse_skill_md` only yields single-line descriptions) but hardens the interpolation.

---

## Commit 2 — Frontend (`web/src`)

### Item 6 — segment colors → CSS custom properties

Replace hardcoded hex (`goal`/`memory`/`summary` → `#a78bfa`/`#34d399`/`#fbbf24`) in
`explorer/ContextExplorer.tsx` with CSS custom properties (theme tokens), matching the existing
theme-token convention so they adapt to themes.

### Item 7 — extract `<RightPaneTabs>`

Extract the duplicated tab-header JSX (wide layout + narrow drawer) in `App.tsx` into a single
`<RightPaneTabs>` component. Keeps the two render sites in sync.

### Item 8 — surface initial-load errors

`MemorySection.tsx` mount (`listMemories().catch(() => {})`) and `SkillSection` skill-open read
path currently swallow fetch errors → silent empty. Render an inline error on failure, matching
the already-handled mutation-error pattern (FIX D). Mount/read path only.

### Item 9 — inline-edit cancel + skill loading indicator

Add a cancel button to the memory inline-edit in `MemorySection.tsx`; add a loading indicator
during `getSkill` in `SkillSection.tsx`.

### Item 10 — guard `loadRightTab` (+ siblings)

Wrap `loadRightTab` and its sibling `load*` helpers in `storage.ts` in try/catch so a blocked-
`localStorage` (private browsing) `SecurityError` returns a default instead of throwing.

### Item 11 — comment on intentional `completion_tokens` drop

Comment in `state.ts` explaining that the `server_usage` handler intentionally keeps only
`promptTokens` (the breakdown needs the prompt total only); revisit if a future chart needs
completion tokens.

---

## Commit 3 — Handler macro (item 17)

In `src-tauri/src/lib.rs`, the production `generate_handler![…]` (≈ line 176) and the
`#[cfg(test)]` list (≈ line 219) drift — the test list omits `get_workspace`, `pick_workspace`,
and `llama_health`.

Introduce a macro that expands to the `generate_handler!` invocation, used at both sites:

```rust
macro_rules! all_handlers {
    () => {
        tauri::generate_handler![
            subscribe, send_input, approve, cancel,
            settings_get, settings_update, context_get,
            get_workspace, pick_workspace, llama_health,
            memory_list, memory_update, memory_delete, memory_recall_preview,
            skill_get, skill_save
        ]
    };
}
```

Both call sites become `.invoke_handler(all_handlers!())`. Drift is now impossible; the test
list gains the three missing handlers for free.

---

## Commit 4 — Tests (items 12–16, 18)

Runs against the drift-proof handler list from commit 3.

- **12** Direct unit test for `snapshot.rs::preview()` in isolation: newline-collapse and the
  `n = 0` edge (currently only covered indirectly via `build_snapshot`).
- **13** Comment in `snapshot.rs` messages segment explaining `items` is intentionally empty
  (history is rendered elsewhere), so it is not "fixed."
- **14** `curated.rs` snapshot test: add `assert_eq!` for `model_limit` passthrough (only `turn`
  is asserted today).
- **15** `agent-memory/src/store.rs`: a `SqliteStore`-level `list` test exercising the SQL
  `ORDER BY … LIMIT … OFFSET` path (only `InMemoryStore` is tested today).
- **16** `agent-server/src/session.rs`: a populated-store `memory_list` happy-path test at the
  `Session` level (only the disabled/`None` path is exercised end-to-end today).
- **18** `ContextExplorer` test for clicking the synthetic `unattributed` slice → maps to no
  `snap.segments` entry → the "gap" fallback panel.

Item 13 is a comment (no test), grouped here for locality with the other `snapshot.rs` work.

---

## Testing strategy

- Commits 1, 3, 4: `cargo test` green (workspace).
- Commit 2: frontend build green; items 6–10 verified against the existing live L1 context-
  explorer smoke where applicable.
- Each commit builds and tests independently (bisect-friendly).

## Out of scope

- Any behavior change beyond the item-1 leak fix and the newly surfaced load-error messages.
- Unrelated refactoring not named in the backlog.
- Re-opening the "Already fixed" items in the backlog's top section.
