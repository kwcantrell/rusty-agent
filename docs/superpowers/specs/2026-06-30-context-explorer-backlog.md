# Context Explorer — deferred Minor findings (backlog)

**Date:** 2026-06-30
**Source:** per-task reviews + the whole-branch (opus) review during the Context Explorer SDD run.
**Status:** all items below are **Minor / polish** — none affect correctness. The feature is merged on `main`, builds green, and is verified live. This is the "address later" list.

Line numbers are as-of-implementation and may have drifted; treat them as pointers.

## Already fixed (do NOT re-open — listed so we don't re-litigate)
- Segment drill-in: the dead `open`/`setOpen` state in `ContextExplorer.tsx` was **wired** (FIX B) — clicking a segment reveals its items/count.
- Mutation error surfacing: `MemorySection` / `SkillSection` mutation handlers (`onDelete`/`onSave`/`onOpen`) now try/catch and render an inline error (FIX D).
- Skill description clobber on edit: `skill_save` now preserves the description via `find(&name).or_else(find(&slug))` (commit `0a568f9`).
- The Critical `ServerUsage`-dropped bug and the two pre-existing crashes (`send_input` async, `write_file` truncation) are all fixed on `main`.

## Open — Rust backend
1. **`MemoryAdmin::get` is effectively dead + leaks existence** (`agent-memory/src/lib.rs`). No caller (`update`/`delete` use `store.get` directly). It returns `Err` for cross-project records rather than `Ok(None)`, which leaks that a record exists. Either wire it or delete it; if kept, return `Ok(None)` for out-of-scope records.
2. **`MemoryAdmin::recall_preview` swallows all errors** (`agent-memory/src/lib.rs`) → `Vec::new()` with no log. Embedder/store/dimension-mismatch failures are invisible. Add a `tracing::warn!` before the empty return.
3. **`Session.workspace` can silently diverge from `RuntimeState`'s copy** (`agent-server/src/session.rs`). `set_workspace` updates the `Session` copy (so memory scope follows the current workspace) but not `RuntimeState`'s (the run loop keeps its own). This is intentional — add a one-line comment on the field so a future maintainer doesn't "fix" it.
4. **`skill_get` does not normalize `name` → slug before `find()`** (`agent-server/src/session.rs`). Fine in practice (the frontend passes already-slugged `discovered_skills` names), but an ergonomic trap for non-slug callers. Optional: `sanitize_slug` the lookup key (ignore errors, fall through to raw).
5. **`skill_save` description interpolation has no newline guard** (`agent-server/src/session.rs`). Very low risk — `parse_skill_md` only yields single-line descriptions, so the input space can't currently contain `\n`. A defensive `desc.replace('\n', " ")` would harden it.

## Open — frontend
6. **Hardcoded hex segment colors** in `ContextExplorer.tsx` (`goal`/`memory`/`summary` → `#a78bfa`/`#34d399`/`#fbbf24`) bypass the theme-token convention. Move to CSS custom properties so they adapt to themes.
7. **Duplicated tab-header JSX** at both `<ContextExplorer>` render sites in `App.tsx` (wide layout + narrow drawer). Extract a `<RightPaneTabs>` component if it grows a third tab / badge / styling.
8. **Initial-load errors swallowed** (`MemorySection.tsx` mount `listMemories().catch(() => {})`; `SkillSection` skill open) → renders empty with no user feedback on a failed fetch. (Mutation errors are already handled; this is the mount/read path.)
9. **No cancel button on memory inline-edit** (`MemorySection.tsx`) and **no loading indicator during `getSkill`** (`SkillSection.tsx`). UX polish.
10. **`loadRightTab` not try/catch-wrapped** (`storage.ts`). Matches the existing `load*` convention (all unguarded), but a blocked-`localStorage` (private browsing) `SecurityError` would throw. Wrap it (and siblings) defensively.
11. **`state.ts` drops `completion_tokens`** from the `server_usage` event (keeps only `promptTokens`). Intentional — the breakdown only needs the prompt total. Revisit only if a future chart needs completion tokens.

## Open — tests
12. **`snapshot.rs`: `preview()` not unit-tested in isolation** (newline-collapse, `n=0` edge). Covered indirectly via `build_snapshot`; add a direct test to pin the contract.
13. **`snapshot.rs` messages segment: `items` intentionally empty** — add a comment explaining why (history is rendered elsewhere), so it isn't "fixed."
14. **`curated.rs` snapshot test doesn't assert `model_limit` passthrough** (only `turn`). One-line `assert_eq!`.
15. **No SqliteStore-level `list` test** (`agent-memory/src/store.rs`) — only `InMemoryStore` is tested; the SQL `ORDER BY … LIMIT … OFFSET` path has no dedicated sqlite test.
16. **No populated-store `memory_list` happy-path test** at the `Session` level (`agent-server/src/session.rs`) — only the disabled/`None` path is exercised end-to-end.
17. **`#[cfg(test)]` `generate_handler!` list omits `get_workspace`/`pick_workspace`** (`src-tauri/src/lib.rs`) — a pre-existing divergence from the production list, now wider. Consider a shared macro so the two lists can't drift.
18. **No test for clicking the synthetic `unattributed` slice** in `ContextExplorer` (it maps to no `snap.segments` entry → the "gap" fallback panel).
