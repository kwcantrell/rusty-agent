# Context Explorer Backlog Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clear all 18 deferred Minor/polish findings from the Context Explorer review — hardened error paths, removed dead/drift-prone code, added missing tests, and documented intentional-by-design decisions.

**Architecture:** Four commits on branch `chore/context-explorer-backlog` (already created), by area: (1) Rust backend memory/skill fixes, (2) frontend fixes, (3) a shared handler macro that makes the Tauri command lists drift-proof, (4) new tests + explanatory comments. The handler macro (commit 3) lands before the new tests (commit 4) so the tests run against the drift-proof list.

**Tech Stack:** Rust (tokio, rusqlite, tracing, tauri), TypeScript/React, Vitest + @testing-library/react.

## Global Constraints

- Cargo is not on PATH by default: run `source ~/.cargo/env` before any `cargo` command.
- Rust test command (workspace root `agent/`): `cargo test -p <crate>` or `cargo test` for all.
- Web package manager is **npm** (`web/package-lock.json`). Test command: `npm test` (runs `vitest run`); type/build check: `npm run build`.
- No behavior change is permitted beyond the item-1 existence-leak fix and the newly surfaced load-error messages (items 8, 10).
- Follow existing code style: terse multi-item struct lines, `var(--token)` inline styles in TSX, `#[tokio::test]` for async Rust tests.
- Spec: `docs/superpowers/specs/2026-06-30-context-explorer-backlog-cleanup-design.md`.
- Backlog source of truth: `docs/superpowers/specs/2026-06-30-context-explorer-backlog.md`.

---

## Task 1: Rust backend — memory admin + skill hardening (items 1–5)

**Files:**
- Modify: `agent/crates/agent-memory/src/lib.rs` (items 1, 2; and the existing `admin_tests` block)
- Modify: `agent/crates/agent-server/src/session.rs` (items 3, 4, 5)

**Interfaces:**
- Produces: `MemoryAdmin::fetch_editable(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError>` (private). `get`/`update`/`delete` keep their existing public signatures; their error contract changes (out-of-scope now indistinguishable from missing).
- Consumes: existing `agent_skills::sanitize_slug(&str) -> Result<String, String>`, `SkillRegistry::find(&str) -> Option<Skill>`.

### Item 1 — wire `MemoryAdmin::get` in via a private `fetch_editable`; close the existence leak

- [ ] **Step 1: Update the existing failing test to the new no-leak contract**

In `agent/crates/agent-memory/src/lib.rs`, in `mod admin_tests`, replace the body assertions of `admin_lists_and_refuses_cross_project_delete` (currently lines ~229–232) and rename the test to reflect the new semantics:

```rust
    #[tokio::test]
    async fn admin_lists_and_hides_cross_project_records() {
        use crate::{Embedder, InMemoryStore, MemoryConfig, MemoryRecord, MemoryScope, StubEmbedder};
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
        let v = embedder.embed(&["hi".into()]).await.unwrap().remove(0);
        store.upsert(MemoryRecord { id: "x".into(), text: "hi".into(),
            scope: MemoryScope::Project("OTHER".into()), tags: vec![], vector: v,
            created_at: 1, updated_at: 1, source: "t".into() }).await.unwrap();
        let admin = MemoryAdmin::new(embedder, store, Arc::new(MemoryConfig::default()),
            MemoryScope::Project("MINE".into()));
        // A cross-project record is invisible AND indistinguishable from missing:
        // no method leaks that it exists.
        assert!(admin.list(20, 0).await.unwrap().is_empty());
        assert!(admin.get("x").await.unwrap().is_none());          // Ok(None), not Err
        assert_eq!(admin.delete("x").await.unwrap(), false);       // silent no-op, not Err
        assert!(admin.update("x", Some("new text".into()), None).await.is_err()); // "not found"
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `source ~/.cargo/env && cargo test -p agent-memory admin_lists_and_hides_cross_project_records`
Expected: FAIL — `get` currently returns `Err` for the out-of-scope record (and `delete` returns `Err`), so `get(...).unwrap()` / `delete(...).unwrap()` panic.

- [ ] **Step 3: Introduce `fetch_editable` and rewire `get`/`delete`/`update`**

In `agent/crates/agent-memory/src/lib.rs`, replace the current `get`, `delete`, and `update` method bodies (lines ~69–105) with:

```rust
    /// Full record iff it exists AND is editable in this scope. `Ok(None)` for both
    /// "missing" and "out of scope" — callers cannot distinguish the two, so we never
    /// leak that an out-of-scope record exists.
    async fn fetch_editable(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError> {
        Ok(match self.store.get(id).await? {
            Some(rec) if self.editable(&rec.scope) => Some(rec),
            _ => None,
        })
    }

    pub async fn get(&self, id: &str) -> Result<Option<MemoryRow>, StoreError> {
        Ok(self.fetch_editable(id).await?.map(|rec| MemoryRow {
            id: rec.id, text: rec.text, tags: rec.tags,
            scope_kind: rec.scope.kind().into(), updated_at: rec.updated_at,
        }))
    }

    pub async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        match self.fetch_editable(id).await? {
            Some(_) => self.store.delete(id).await,
            None => Ok(false),
        }
    }

    pub async fn update(&self, id: &str, text: Option<String>, tags: Option<Vec<String>>)
        -> Result<MemoryRow, StoreError> {
        let mut rec = self.fetch_editable(id).await?
            .ok_or_else(|| StoreError::Io("not found".into()))?;
        if let Some(t) = text {
            rec.vector = self.embedder.embed(&[t.clone()]).await
                .map_err(|e| StoreError::Io(e.to_string()))?.remove(0);
            rec.text = t;
        }
        if let Some(tg) = tags { rec.tags = tg; }
        rec.updated_at = now_secs();
        self.store.upsert(rec.clone()).await?;
        Ok(MemoryRow { id: rec.id, text: rec.text, tags: rec.tags,
            scope_kind: rec.scope.kind().into(), updated_at: rec.updated_at })
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `source ~/.cargo/env && cargo test -p agent-memory admin_lists_and_hides_cross_project_records`
Expected: PASS.

### Item 2 — `recall_preview` logs before swallowing

- [ ] **Step 5: Add a `tracing::warn!` before the empty return**

In `agent/crates/agent-memory/src/lib.rs`, in `recall_preview`, change the `Err(_) => Vec::new()` arm (line ~116) to log first:

```rust
            Err(e) => {
                tracing::warn!(error = %e, "recall_preview failed; returning no results");
                Vec::new()
            }
```

(`tracing` is already a dependency — see `agent/crates/agent-memory/Cargo.toml`.)

### Item 3 — comment the intentional `Session.workspace` divergence

- [ ] **Step 6: Add the explanatory comment**

In `agent/crates/agent-server/src/session.rs`, add a comment above the `workspace` field in `struct Session` (line ~37):

```rust
    /// The live workspace for this session. `set_workspace` updates only this copy
    /// (so memory scope + skills follow the current workspace); it intentionally does
    /// NOT touch `RuntimeState`'s own workspace, which the run loop owns. Do not "sync"
    /// them — the divergence is by design.
    workspace: Mutex<PathBuf>,
```

### Item 4 — `skill_get` normalizes the lookup key

- [ ] **Step 7: Slug the lookup key, fall through to raw**

In `agent/crates/agent-server/src/session.rs`, replace the `skill_get` lookup (lines ~159–161) so it tries the sanitized slug first, then the raw name:

```rust
    pub async fn skill_get(&self, name: String) -> Result<SkillDto, String> {
        let reg = self.skill_registry();
        // Normalize to a slug for lookup (ignore errors), then fall through to the raw
        // name so non-slug callers still resolve.
        let slug = agent_skills::sanitize_slug(&name).ok();
        let s = slug.as_deref().and_then(|sl| reg.find(sl))
            .or_else(|| reg.find(&name))
            .ok_or_else(|| format!("skill not found: {name}"))?;
        Ok(SkillDto {
            name: s.name,
            description: s.description,
            body: s.body,
            files: s.files.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        })
    }
```

### Item 5 — `skill_save` newline guard

- [ ] **Step 8: Collapse newlines in the preserved description**

In `agent/crates/agent-server/src/session.rs`, in `skill_save`, add a newline guard right after `desc` is computed (after line ~180, before the `format!`):

```rust
        let desc = reg.find(&name)
            .or_else(|| reg.find(&slug))
            .map(|s| s.description)
            .unwrap_or_else(|| format!("{slug} skill"));
        let desc = desc.replace('\n', " "); // frontmatter is single-line; harden interpolation
        let md = format!("---\nname: {slug}\ndescription: {desc}\n---\n{body}\n");
```

### Wrap up Task 1

- [ ] **Step 9: Build and run the full backend suite**

Run: `source ~/.cargo/env && cargo test -p agent-memory -p agent-server`
Expected: PASS (all existing + updated tests green).

- [ ] **Step 10: Commit**

```bash
git add agent/crates/agent-memory/src/lib.rs agent/crates/agent-server/src/session.rs
git commit -m "fix(memory,skills): wire MemoryAdmin::get, close existence leak, log recall_preview, harden skill lookup/save (backlog 1-5)"
```

---

## Task 2: Frontend — theming, dedup, error surfacing, UX polish (items 6–11)

**Files:**
- Modify: `web/src/index.css` (item 6 — new theme tokens)
- Modify: `web/src/explorer/ContextExplorer.tsx` (item 6 — use tokens)
- Create: `web/src/components/RightPaneTabs.tsx` (item 7)
- Modify: `web/src/App.tsx` (item 7 — use the new component)
- Modify: `web/src/explorer/MemorySection.tsx` (items 8, 9)
- Modify: `web/src/explorer/SkillSection.tsx` (item 9)
- Modify: `web/src/storage.ts` (item 10)
- Modify: `web/src/state.ts` (item 11 — comment)

**Interfaces:**
- Produces: `RightPaneTabs` React component — `export function RightPaneTabs({ rightTab, setRightTab }: { rightTab: RightTab; setRightTab: (t: RightTab) => void }): JSX.Element`. It renders the `role="tablist"` header and persists the choice via `saveRightTab`.
- Consumes: `RightTab`, `saveRightTab` from `web/src/storage.ts`.

### Item 6 — segment colors → CSS custom properties

- [ ] **Step 1: Add context-segment tokens to both themes**

In `web/src/index.css`, add these three lines inside **both** `:root[data-theme="light"]` and `:root[data-theme="dark"]` blocks (e.g. after `--state-error`):

```css
  --ctx-goal: #a78bfa;
  --ctx-memory: #34d399;
  --ctx-summary: #fbbf24;
```

- [ ] **Step 2: Reference the tokens in ContextExplorer**

In `web/src/explorer/ContextExplorer.tsx`, replace the `COLORS` map (lines ~8–11):

```tsx
const COLORS: Record<string, string> = {
  system: "var(--accent)", goal: "var(--ctx-goal)", memory: "var(--ctx-memory)",
  summary: "var(--ctx-summary)", messages: "var(--text-muted)", unattributed: "var(--state-error)",
};
```

### Item 7 — extract `<RightPaneTabs>`

- [ ] **Step 3: Create the component**

Create `web/src/components/RightPaneTabs.tsx`:

```tsx
import type { RightTab } from "../storage";
import { saveRightTab } from "../storage";

export function RightPaneTabs(
  { rightTab, setRightTab }: { rightTab: RightTab; setRightTab: (t: RightTab) => void },
) {
  return (
    <div className="flex gap-1 px-2 pt-2" role="tablist" style={{ borderBottom: "1px solid var(--border)" }}>
      {(["workspace", "context"] as const).map((t) => (
        <button key={t} role="tab" aria-selected={rightTab === t}
          onClick={() => { setRightTab(t); saveRightTab(t); }}
          className="rounded-t-lg px-3 py-1.5 text-xs"
          style={{ color: rightTab === t ? "var(--text-strong)" : "var(--text-muted)",
            fontWeight: rightTab === t ? 600 : 400 }}>
          {t === "workspace" ? "Workspace" : "Context"}
        </button>
      ))}
    </div>
  );
}
```

- [ ] **Step 4: Use it at both render sites in App.tsx**

In `web/src/App.tsx`, add the import near the other component imports (after line ~13):

```tsx
import { RightPaneTabs } from "./components/RightPaneTabs";
```

Replace the wide-layout tablist `<div className="flex gap-1 px-2 pt-2" role="tablist" …>…</div>` block (lines ~164–174) with:

```tsx
              <RightPaneTabs rightTab={rightTab} setRightTab={setRightTab} />
```

Replace the narrow-drawer tablist block (lines ~191–201) with the identical line:

```tsx
                <RightPaneTabs rightTab={rightTab} setRightTab={setRightTab} />
```

- [ ] **Step 5: Verify the app still type-checks and the tab tests pass**

Run: `cd web && npm run build && npm test`
Expected: build PASS; existing `App.test.tsx` / tab tests PASS (the rendered DOM is unchanged).

### Item 8 — surface MemorySection initial-load errors

- [ ] **Step 6: Write the failing test**

In `web/src/explorer/MemorySection.test.tsx`, add a test asserting the mount-time load error is shown. Match the existing mock style in that file (it already `vi.mock("./api", …)`). Add this case (adapt the mock to reject `listMemories` for this test):

```tsx
  it("surfaces an error when the initial memory load fails", async () => {
    const api = await import("./api");
    (api.listMemories as unknown as { mockRejectedValueOnce: (e: Error) => void })
      .mockRejectedValueOnce(new Error("boom"));
    render(<MemorySection recalled={[]} lastQuery={null} />);
    expect(await screen.findByText(/boom/)).toBeInTheDocument();
  });
```

(If the file's mock is defined so `listMemories` isn't a `vi.fn()`, change the top-of-file `vi.mock("./api", …)` to make `listMemories: vi.fn().mockResolvedValue([])` so `mockRejectedValueOnce` is available — mirror how `ContextExplorer.test.tsx` builds its mock.)

- [ ] **Step 7: Run it to verify it fails**

Run: `cd web && npm test -- MemorySection`
Expected: FAIL — the error is swallowed (`.catch(() => {})`), so `/boom/` never appears.

- [ ] **Step 8: Surface the error in `refresh` and the recall preview**

In `web/src/explorer/MemorySection.tsx`, change `refresh` (line ~13) and the recall-preview effect (line ~19) to record the error:

```tsx
  const refresh = () =>
    listMemories(50, 0).then((r) => { setRows(r); setError(null); })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  useEffect(() => { refresh(); }, []);

  useEffect(() => {
    if (!lastQuery) { setScoredRows([]); return; }
    let active = true;
    recallPreview(lastQuery)
      .then((r) => { if (active) setScoredRows(r); })
      .catch((e) => { if (active) setError(e instanceof Error ? e.message : String(e)); });
    return () => { active = false; };
  }, [lastQuery]);
```

- [ ] **Step 9: Run it to verify it passes**

Run: `cd web && npm test -- MemorySection`
Expected: PASS.

### Item 9 — inline-edit cancel button (Memory) + skill loading indicator

- [ ] **Step 10: Add a cancel button to the memory inline editor**

In `web/src/explorer/MemorySection.tsx`, in the `editing === r.id` branch (lines ~75–80), add a cancel button next to `save`:

```tsx
              <div className="flex gap-1">
                <input value={draft} onChange={(e) => setDraft(e.target.value)}
                  className="flex-1 rounded px-1"
                  style={{ background: "var(--surface-base)", color: "var(--text-strong)" }} />
                <button onClick={() => onSave(r.id)} style={{ color: "var(--accent)" }}>save</button>
                <button onClick={() => { setEditing(null); setError(null); }}
                  style={{ color: "var(--text-muted)" }}>cancel</button>
              </div>
```

- [ ] **Step 11: Add a loading indicator to SkillSection during `getSkill`**

In `web/src/explorer/SkillSection.tsx`, add a `loading` state and toggle it around `getSkill`:

```tsx
  const [loading, setLoading] = useState(false);

  const onOpen = async (name: string) => {
    setLoading(true);
    try {
      const s = await getSkill(name);
      setOpen(s); setBody(s.body); setSaved(false); setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };
```

Then render the indicator right after the error line (after line ~32):

```tsx
      {loading && <div style={{ color: "var(--text-muted)" }}>loading…</div>}
```

- [ ] **Step 12: Run the affected suites**

Run: `cd web && npm test -- MemorySection SkillSection`
Expected: PASS (existing tests unaffected; new cancel button / loading text are additive).

### Item 10 — guard `load*` against `localStorage` `SecurityError`

- [ ] **Step 13: Write the failing test**

Create `web/src/storage.securityerror.test.ts`:

```ts
import { describe, it, expect, afterEach, vi } from "vitest";
import { loadRightTab, loadTheme } from "./storage";

describe("storage load helpers under a blocked localStorage", () => {
  afterEach(() => vi.restoreAllMocks());

  it("loadRightTab falls back to 'workspace' when getItem throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new DOMException("blocked", "SecurityError"); });
    expect(loadRightTab()).toBe("workspace");
  });

  it("loadTheme falls back to null when getItem throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new DOMException("blocked", "SecurityError"); });
    expect(loadTheme()).toBeNull();
  });
});
```

- [ ] **Step 14: Run it to verify it fails**

Run: `cd web && npm test -- storage.securityerror`
Expected: FAIL — the unguarded `getItem` throws out of `loadRightTab`/`loadTheme`.

- [ ] **Step 15: Wrap the read helpers**

In `web/src/storage.ts`, make each `load*` helper swallow a throwing `getItem` and return its default. Rewrite these functions:

```ts
export function loadTheme(): Theme | null {
  try {
    const v = localStorage.getItem(THEME_KEY);
    return v === "light" || v === "dark" ? v : null;
  } catch { return null; }
}
```
```ts
export function loadToken(): string | null {
  try { return localStorage.getItem(TOKEN); } catch { return null; }
}
export function loadSessionId(): string | null {
  try { return localStorage.getItem(SID); } catch { return null; }
}
```
```ts
export function loadUserMsgs(sessionId: string): string[] {
  try {
    const raw = localStorage.getItem(MSGS(sessionId));
    if (!raw) return [];
    const v = JSON.parse(raw);
    return Array.isArray(v) ? (v as string[]) : [];
  } catch {
    return [];
  }
}
```
```ts
export function loadWorkspaceView(): WorkspaceView {
  try {
    const raw = localStorage.getItem(WORKSPACE_VIEW);
    if (!raw) return { ...DEFAULT_VIEW };
    const v = JSON.parse(raw) as Partial<WorkspaceView>;
    const mode = v.mode === "code" ? "code" : "preview";
    const viewport = v.viewport === "tablet" || v.viewport === "mobile" ? v.viewport : "desktop";
    return { mode, viewport };
  } catch {
    return { ...DEFAULT_VIEW };
  }
}
```
```ts
export function loadDashExpanded(): boolean {
  try { return localStorage.getItem(DASH_EXPANDED) === "1"; } catch { return false; }
}
```
```ts
export function loadRightTab(): RightTab {
  try { return localStorage.getItem(RIGHT_TAB) === "context" ? "context" : "workspace"; }
  catch { return "workspace"; }
}
```

- [ ] **Step 16: Run it to verify it passes**

Run: `cd web && npm test -- storage`
Expected: PASS (new test + existing `storage.workspace.test.ts`).

### Item 11 — comment the intentional `completion_tokens` drop

- [ ] **Step 17: Add the explanatory comment**

In `web/src/state.ts`, add a comment above the `server_usage` case (line ~93):

```ts
    // The breakdown only needs the prompt total, so we intentionally keep only
    // promptTokens here and drop completion_tokens. Revisit if a chart needs it.
    case "server_usage":
      return { ...s, serverUsage: { promptTokens: p.prompt_tokens, turn: p.turn } };
```

### Wrap up Task 2

- [ ] **Step 18: Full frontend build + test**

Run: `cd web && npm run build && npm test`
Expected: build PASS; all suites PASS.

- [ ] **Step 19: Commit**

```bash
git add web/src/index.css web/src/explorer/ContextExplorer.tsx web/src/components/RightPaneTabs.tsx web/src/App.tsx web/src/explorer/MemorySection.tsx web/src/explorer/MemorySection.test.tsx web/src/explorer/SkillSection.tsx web/src/storage.ts web/src/storage.securityerror.test.ts web/src/state.ts
git commit -m "fix(web): theme tokens for segments, RightPaneTabs dedup, surface load errors, edit-cancel + skill loading, guard localStorage (backlog 6-11)"
```

---

## Task 3: Shared Tauri handler macro (item 17)

**Files:**
- Modify: `src-tauri/src/lib.rs` (production `generate_handler!` at ~line 176; `#[cfg(test)]` list at ~line 219)

**Interfaces:**
- Produces: a `macro_rules! all_handlers` expanding to `tauri::generate_handler![…]`, used at both invoke-handler sites. No signature changes.

### Item 17 — one handler list, used twice

- [ ] **Step 1: Define the macro**

In `src-tauri/src/lib.rs`, add near the top of the file (after the imports, before `run()`):

```rust
/// Single source of truth for the Tauri command surface. Used by both the
/// production builder and the `#[cfg(test)]` mock app so the two lists cannot drift.
macro_rules! all_handlers {
    () => {
        tauri::generate_handler![
            subscribe,
            send_input,
            approve,
            cancel,
            settings_get,
            settings_update,
            context_get,
            get_workspace,
            pick_workspace,
            llama_health,
            memory_list,
            memory_update,
            memory_delete,
            memory_recall_preview,
            skill_get,
            skill_save
        ]
    };
}
```

- [ ] **Step 2: Use the macro at the production site**

In `src-tauri/src/lib.rs`, replace the production `.invoke_handler(tauri::generate_handler![ … ])` block (lines ~176–193) with:

```rust
        .invoke_handler(all_handlers!())
```

- [ ] **Step 3: Use the macro at the test site**

In `src-tauri/src/lib.rs`, in `mod cmd_tests`, replace the `.invoke_handler(tauri::generate_handler![ … ])` block (lines ~219–223) with:

```rust
            .invoke_handler(all_handlers!())
```

- [ ] **Step 4: Build + run the Tauri crate tests**

Run: `source ~/.cargo/env && cargo test -p rust-agent-runtime-desktop`
Expected: PASS — the mock app now registers the full command surface (including the previously-missing `get_workspace`, `pick_workspace`, `llama_health`), and the existing smoke test still resolves its command.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "refactor(desktop): share Tauri handler list via all_handlers! macro so prod/test can't drift (backlog 17)"
```

---

## Task 4: Tests + explanatory comments (items 12–16, 18, 13)

**Files:**
- Modify: `agent/crates/agent-core/src/snapshot.rs` (items 12, 13)
- Modify: `agent/crates/agent-core/src/curated.rs` (item 14)
- Modify: `agent/crates/agent-memory/src/store.rs` (item 15)
- Modify: `agent/crates/agent-server/src/session.rs` (item 16)
- Modify: `web/src/explorer/ContextExplorer.test.tsx` (item 18)

**Interfaces:**
- Consumes: `agent_core::snapshot::preview(&str, usize) -> String` and `build_snapshot(...)`; `SqliteStore::open`, `MemoryStore::{upsert,list}`; `Session::memory_list`; `agent_memory::MemoryParts`, `StubEmbedder::new`, `MemoryRecord`, `MemoryScope`, `now_secs`.

### Item 12 — direct unit test for `preview()`

- [ ] **Step 1: Add the test**

In `agent/crates/agent-core/src/snapshot.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn preview_collapses_newlines_and_truncates() {
        assert_eq!(preview("a\nb\nc", 100), "a b c");     // newlines → single spaces
        assert_eq!(preview("hello world", 5), "hello");   // truncates to n chars
        assert_eq!(preview("anything", 0), "");           // n = 0 → empty
        assert_eq!(preview("", 10), "");                  // empty input → empty
    }
```

- [ ] **Step 2: Run it**

Run: `source ~/.cargo/env && cargo test -p agent-core preview_collapses_newlines_and_truncates`
Expected: PASS (pins the existing `preview` contract).

### Item 13 — comment the intentionally-empty messages `items`

- [ ] **Step 3: Add the comment**

In `agent/crates/agent-core/src/snapshot.rs`, in `build_snapshot`, annotate the messages segment's empty `items` (line ~80):

```rust
    segments.push(ContextSegment {
        category: "messages".into(),
        est_tokens: msg_tokens,
        // Intentionally empty: message bodies are rendered in the main transcript,
        // not the explorer drill-in. Only the count/token total is surfaced here.
        items: Vec::new(),
        count: history.len(),
    });
```

### Item 14 — assert `model_limit` passthrough

- [ ] **Step 4: Add the assertion**

In `agent/crates/agent-core/src/curated.rs`, in `curated_snapshot_reports_system_recall_and_messages`, add after the `snap.turn` assertion (line ~395):

```rust
        assert_eq!(snap.turn, 7);
        assert_eq!(snap.model_limit, 10_000);
```

- [ ] **Step 5: Run the two agent-core additions**

Run: `source ~/.cargo/env && cargo test -p agent-core`
Expected: PASS.

### Item 15 — SqliteStore `list` ordering/paging test

- [ ] **Step 6: Add the test**

In `agent/crates/agent-memory/src/store.rs`, inside `mod sqlite_tests`, add a test exercising `ORDER BY updated_at DESC LIMIT … OFFSET …`:

```rust
    #[tokio::test]
    async fn list_orders_newest_first_and_pages() {
        let tmp = tempfile::tempdir().unwrap();
        let s = SqliteStore::open(&tmp.path().join("m.db")).unwrap();
        let sc = MemoryScope::Project("A".into());
        for (id, t) in [("a", 100i64), ("b", 200), ("c", 300)] {
            let mut r = rec(id, sc.clone(), vec![1.0, 0.0]);
            r.updated_at = t;
            s.upsert(r).await.unwrap();
        }
        let f = ScopeFilter::Exact(sc);
        // Newest first, full page.
        let all = s.list(&f, 10, 0).await.unwrap();
        let ids: Vec<&str> = all.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "b", "a"]);
        // LIMIT 1 OFFSET 1 → the second-newest.
        let page = s.list(&f, 1, 1).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, "b");
    }
```

- [ ] **Step 7: Run it**

Run: `source ~/.cargo/env && cargo test -p agent-memory list_orders_newest_first_and_pages`
Expected: PASS.

### Item 16 — populated-store `memory_list` happy-path at the Session level

- [ ] **Step 8: Add the test**

In `agent/crates/agent-server/src/session.rs`, inside `mod tests`, add a test that builds a `Session` with a populated `MemoryParts` and asserts `memory_list` returns the seeded row. Seed a **Global**-scoped row so it is visible regardless of the temp workspace's project scope:

```rust
    #[tokio::test]
    async fn memory_list_returns_seeded_rows() {
        use agent_memory::{MemoryConfig, MemoryParts, MemoryRecord, MemoryScope, MemoryStore,
            InMemoryStore, StubEmbedder, now_secs};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
        store.upsert(MemoryRecord {
            id: "seed".into(), text: "hello world".into(), scope: MemoryScope::Global,
            tags: vec![], vector: vec![0.1, 0.2], created_at: now_secs(), updated_at: now_secs(),
            source: "test".into(),
        }).await.unwrap();
        let parts = MemoryParts {
            embedder: Arc::new(StubEmbedder::new(384)),
            store: store.clone(),
            cfg: Arc::new(MemoryConfig::default()),
        };
        let params = crate::setup::local_params(
            dir.path().to_path_buf(), dir.path().join("rt.json"),
            "http://localhost:8080".into(), "m".into(), Some(&parts));
        let sess = Session::from_params(params);
        std::mem::forget(dir); // keep temp dir alive for the test process
        let rows = sess.memory_list(20, 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "seed");
        assert_eq!(rows[0].scope_kind, "global");
    }
```

- [ ] **Step 9: Run it**

Run: `source ~/.cargo/env && cargo test -p agent-server memory_list_returns_seeded_rows`
Expected: PASS. (If `StubEmbedder::new`/`MemoryStore` are not re-exported from `agent_memory`, adjust the `use` to the correct path — `StubEmbedder` and `MemoryStore` are both re-exported at the crate root per `agent-memory/src/lib.rs`.)

### Item 18 — clicking the synthetic `unattributed` slice shows the gap panel

- [ ] **Step 10: Add the test**

In `web/src/explorer/ContextExplorer.test.tsx`, add a case. The existing `vi.mock("./api", …)` returns a snapshot whose segments sum to 60; rendering with `realTotal={100}` makes `computeBreakdown` synthesize a 40-token `unattributed` slice that maps to no segment:

```tsx
  it("clicking the synthetic unattributed slice shows the gap panel", async () => {
    render(<ContextExplorer realTotal={100} refreshKey={0} skills={[]} lastQuery={null} />);
    // realTotal (100) > est_total (60) → an "unattributed" legend button appears.
    const btn = await screen.findByRole("button", { name: /unattributed/i });
    fireEvent.click(btn);
    expect(await screen.findByText(/Gap between server total and estimated sum/)).toBeInTheDocument();
    expect(screen.getByText(/40 tokens unaccounted/)).toBeInTheDocument();
  });
```

- [ ] **Step 11: Run it**

Run: `cd web && npm test -- ContextExplorer`
Expected: PASS.

### Wrap up Task 4

- [ ] **Step 12: Full workspace + frontend test**

Run: `source ~/.cargo/env && cargo test` (from `agent/`) and `cd web && npm test`
Expected: all PASS.

- [ ] **Step 13: Commit**

```bash
git add agent/crates/agent-core/src/snapshot.rs agent/crates/agent-core/src/curated.rs agent/crates/agent-memory/src/store.rs agent/crates/agent-server/src/session.rs web/src/explorer/ContextExplorer.test.tsx
git commit -m "test: preview()/model_limit/sqlite-list/session memory_list/unattributed-gap + intentional-empty comment (backlog 12-16,18,13)"
```

---

## Definition of Done

- All 18 backlog items addressed: 1–5 (Task 1), 6–11 (Task 2), 17 (Task 3), 12–16 + 18 + 13 (Task 4).
- `cargo test` (workspace) and `npm test` (web) both green.
- `npm run build` green.
- Four commits on `chore/context-explorer-backlog`, each building independently.
- No behavior change beyond the item-1 leak fix and the surfaced load-error messages.
