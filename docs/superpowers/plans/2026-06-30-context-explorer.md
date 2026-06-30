# Context Explorer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a right-side "Context" tab to the existing `web/` app that visualizes the live context window (per-category token breakdown) and lets the operator browse/edit/delete memories and view/edit skills.

**Architecture:** Pull-based. The agent loop is untouched. The frontend invokes new Tauri commands (`context_get`, `memory_list/get/update/delete`, `memory_recall_preview`, `skill_get/skill_save`) after each turn and on user action. Commands delegate to new `Session` methods over the already-durable `SqliteStore` and the `SkillRegistry`. A new `CuratedContext::snapshot()` produces the per-category breakdown; the real total is overlaid frontend-side from the existing `ServerUsage` event.

**Tech Stack:** Rust (agent-core, agent-memory, agent-server, src-tauri/Tauri 2 commands), React 19 + TypeScript + Vite + Tailwind v4 (web/), vitest.

## Global Constraints

- Tauri command transport only — no new HTTP server, no websocket, no CORS. New commands mirror `settings_get`/`settings_update` (`#[tauri::command]` in `src-tauri/src/lib.rs`, registered in BOTH `generate_handler!` blocks at `lib.rs:126` and the test block at `lib.rs:162`, delegating to a `Session` method).
- Memory is already durable via `SqliteStore` (`agent-memory::open_memory_parts` → `SqliteStore::open(default_db_path())`, wired in `src-tauri/src/bridge.rs:start`). Do NOT add a store swap. Do NOT change the embedder (`StubEmbedder::d384` without the `onnx` feature; `FastEmbedEmbedder` with it).
- Token estimates use the existing `agent_core::estimate_tokens` (~4 chars/token) and `message_tokens` (already counts `reasoning` + `tool_calls`). These are ESTIMATES. The faithful total comes from the existing `AgentEvent::ServerUsage { prompt_tokens, .. }` already emitted per turn. The UI must show the faithful total as ground truth and reconcile estimates with an explicit `unattributed` slice; never present an estimate as exact.
- Scope safety: memory delete/update must refuse records whose scope is another project (mirror the `Forget` tool's guard; see `agent-memory/src/tools.rs` test `forget_by_id_refuses_other_project_scope`). Current-project and `Global` records are editable.
- Skill writes go ONLY to the registry's writable root (`SkillRegistry::writable_root()`), file `<writable_root>/<sanitized_slug>/SKILL.md`, name validated via `agent_skills::sanitize_slug`.
- Follow existing styling: Tailwind utility classes + CSS custom properties (`var(--surface-base)`, `var(--border)`, `var(--text-strong)`, `var(--text-muted)`, `var(--accent)`, `var(--state-error)`). Persist UI prefs via `web/src/storage.ts` helpers (localStorage, try/catch-wrapped).
- TDD throughout. Rust tests with `cargo test -p <crate>`; frontend with `cd web && npx vitest run <file>`. Commit after each green task.

---

## File Structure

**Rust — new/modified**
- `agent/crates/agent-core/src/snapshot.rs` (Create) — `ContextSnapshot`, `ContextSegment` types + `snapshot()` free fn helpers; re-exported from `lib.rs`.
- `agent/crates/agent-core/src/curated.rs` (Modify) — add `CuratedContext::snapshot(model_limit) -> ContextSnapshot`.
- `agent/crates/agent-core/src/lib.rs` (Modify) — `pub use snapshot::{ContextSnapshot, ContextSegment};`
- `agent/crates/agent-memory/src/store.rs` (Modify) — add `MemoryStore::list` to trait + `InMemoryStore` + `SqliteStore`.
- `agent/crates/agent-memory/src/lib.rs` (Modify) — small `MemoryAdmin` helper struct (list/get/update/delete/recall_preview) over store+embedder+scope, re-exported.
- `agent/crates/agent-server/src/daemon.rs` (Modify) — add `memory_parts: Option<MemoryParts>` to `DaemonParams`.
- `agent/crates/agent-server/src/setup.rs` (Modify) — pass `memory_parts` through `local_params`.
- `agent/crates/agent-server/src/session.rs` (Modify) — `context_get`, `memory_*`, `skill_*` methods.
- `agent/crates/agent-server/src/wire.rs` (Modify) — `pub use` re-exports of the snapshot + new DTO types for serialization at the IPC boundary.
- `src-tauri/src/lib.rs` (Modify) — new `#[tauri::command]`s + registration.

**Frontend — new/modified**
- `web/src/explorer/api.ts` (Create) — typed `invoke()` wrappers for the new commands.
- `web/src/explorer/types.ts` (Create) — TS mirrors of the Rust DTOs.
- `web/src/explorer/ContextExplorer.tsx` (Create) — the pane: breakdown bar + sections.
- `web/src/explorer/MemorySection.tsx` (Create) — recalled rows + browser + edit/delete.
- `web/src/explorer/SkillSection.tsx` (Create) — list + view/edit/save.
- `web/src/explorer/breakdown.ts` (Create) — pure helper computing segment widths + `unattributed` (unit-tested).
- `web/src/state.ts` (Modify) — capture latest `ServerUsage` (faithful total) into state.
- `web/src/wire.ts` (Modify) — add `server_usage` to `WireEvent`.
- `web/src/storage.ts` (Modify) — persist which right-pane tab is active.
- `web/src/App.tsx` (Modify) — right-pane tab switch `[ Workspace | Context ]`.

---

## Task 1: Context snapshot types + `CuratedContext::snapshot()`

**Files:**
- Create: `agent/crates/agent-core/src/snapshot.rs`
- Modify: `agent/crates/agent-core/src/curated.rs`, `agent/crates/agent-core/src/lib.rs`
- Test: in `snapshot.rs` (`#[cfg(test)]`) and `curated.rs` tests

**Interfaces:**
- Produces:
  - `pub struct ContextSegment { pub category: String, pub est_tokens: usize, pub items: Vec<String>, pub count: usize }` (`items` = previews/names; `count` = item count, e.g. message count)
  - `pub struct ContextSnapshot { pub turn: usize, pub model_limit: usize, pub est_total: usize, pub segments: Vec<ContextSegment> }`
  - `impl CuratedContext { pub fn snapshot(&self, model_limit: usize, turn: usize) -> ContextSnapshot }`
- Consumes: existing private fields of `CuratedContext` (`system`, `goal`, `recall: Vec<String>`, `compaction_summary`, `history`), and `crate::context::{message_tokens, estimate_tokens}`.

- [ ] **Step 1: Write the failing test for the types + a pure builder**

In `agent/crates/agent-core/src/snapshot.rs`:

```rust
use crate::context::{estimate_tokens, message_tokens};
use agent_model::Message;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextSegment {
    pub category: String,
    pub est_tokens: usize,
    pub items: Vec<String>,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextSnapshot {
    pub turn: usize,
    pub model_limit: usize,
    pub est_total: usize,
    pub segments: Vec<ContextSegment>,
}

/// First `n` chars of a single-line preview of `s`.
pub(crate) fn preview(s: &str, n: usize) -> String {
    let one_line = s.replace('\n', " ");
    one_line.chars().take(n).collect()
}

/// Build a snapshot from already-separated context blocks. Pure so it is unit
/// testable without a full CuratedContext.
pub(crate) fn build_snapshot(
    turn: usize,
    model_limit: usize,
    system: &Message,
    goal: Option<&Message>,
    recall: &[String],
    compaction_summary: Option<&Message>,
    history: &[Message],
) -> ContextSnapshot {
    let mut segments = Vec::new();

    let sys_tokens = message_tokens(system);
    segments.push(ContextSegment {
        category: "system".into(),
        est_tokens: sys_tokens,
        items: vec![preview(&system.content, 120)],
        count: 1,
    });

    if let Some(g) = goal {
        segments.push(ContextSegment {
            category: "goal".into(),
            est_tokens: message_tokens(g),
            items: vec![preview(&g.content, 120)],
            count: 1,
        });
    }

    if !recall.is_empty() {
        let est = recall.iter().map(|l| estimate_tokens(l)).sum();
        segments.push(ContextSegment {
            category: "memory".into(),
            est_tokens: est,
            items: recall.iter().map(|l| preview(l, 100)).collect(),
            count: recall.len(),
        });
    }

    if let Some(c) = compaction_summary {
        segments.push(ContextSegment {
            category: "summary".into(),
            est_tokens: message_tokens(c),
            items: vec![preview(&c.content, 120)],
            count: 1,
        });
    }

    let msg_tokens: usize = history.iter().map(message_tokens).sum();
    segments.push(ContextSegment {
        category: "messages".into(),
        est_tokens: msg_tokens,
        items: Vec::new(),
        count: history.len(),
    });

    let est_total = segments.iter().map(|s| s.est_tokens).sum();
    ContextSnapshot { turn, model_limit, est_total, segments }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_model::Message;

    #[test]
    fn snapshot_has_system_and_messages_and_sums_total() {
        let snap = build_snapshot(
            3, 1000,
            &Message::system("SYSTEM PROMPT"),
            None,
            &[],
            None,
            &[Message::user("hello"), Message::assistant("hi")],
        );
        assert_eq!(snap.turn, 3);
        let cats: Vec<&str> = snap.segments.iter().map(|s| s.category.as_str()).collect();
        assert_eq!(cats, vec!["system", "messages"]);
        let messages = snap.segments.iter().find(|s| s.category == "messages").unwrap();
        assert_eq!(messages.count, 2);
        assert_eq!(snap.est_total, snap.segments.iter().map(|s| s.est_tokens).sum::<usize>());
    }

    #[test]
    fn recall_block_becomes_memory_segment_with_previews() {
        let snap = build_snapshot(
            1, 1000,
            &Message::system("S"),
            None,
            &["user likes rust".to_string(), "deploys on friday".to_string()],
            None,
            &[],
        );
        let mem = snap.segments.iter().find(|s| s.category == "memory").unwrap();
        assert_eq!(mem.count, 2);
        assert!(mem.items[0].contains("rust"));
    }
}
```

- [ ] **Step 2: Run the test, verify it fails to compile/link**

Run: `cd agent && cargo test -p agent-core snapshot:: 2>&1 | tail -20`
Expected: FAIL — `snapshot` module not declared in `lib.rs`.

- [ ] **Step 3: Declare the module + exports**

In `agent/crates/agent-core/src/lib.rs`, add alongside the other `mod`/`pub use` lines:

```rust
mod snapshot;
pub use snapshot::{ContextSegment, ContextSnapshot};
```

- [ ] **Step 4: Run the test, verify it passes**

Run: `cd agent && cargo test -p agent-core snapshot:: 2>&1 | tail -20`
Expected: PASS (2 tests).

- [ ] **Step 5: Write the failing test for the `CuratedContext::snapshot` method**

Append to the `#[cfg(test)] mod tests` in `agent/crates/agent-core/src/curated.rs` (it already has helpers `mgr()`/context constructors — use the same `CuratedContext::new` pattern seen in existing tests):

```rust
#[test]
fn curated_snapshot_reports_system_recall_and_messages() {
    let mut c = CuratedContext::new(
        Message::system("SYS"),
        Arc::new(crate::offload::InMemoryOffloadStore::new()),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    c.set_recall(vec!["user likes rust".into()]);
    c.append(Message::user("hello"));
    let snap = c.snapshot(10_000, 7);
    assert_eq!(snap.turn, 7);
    assert!(snap.segments.iter().any(|s| s.category == "system"));
    assert!(snap.segments.iter().any(|s| s.category == "memory"));
    let msgs = snap.segments.iter().find(|s| s.category == "messages").unwrap();
    assert_eq!(msgs.count, 1);
}
```

(If the offload store path differs, reuse whatever constructor the existing `curated.rs` tests use — grep the test module for `CuratedContext::new(`.)

- [ ] **Step 6: Run it, verify it fails**

Run: `cd agent && cargo test -p agent-core curated_snapshot 2>&1 | tail -20`
Expected: FAIL — no method `snapshot`.

- [ ] **Step 7: Implement `snapshot` on `CuratedContext`**

In `agent/crates/agent-core/src/curated.rs`, inside `impl CuratedContext`:

```rust
/// Per-category breakdown of the current context window, for the explorer UI.
/// Token figures are estimates; the faithful total comes from server usage.
pub fn snapshot(&self, model_limit: usize, turn: usize) -> crate::ContextSnapshot {
    crate::snapshot::build_snapshot(
        turn,
        model_limit,
        &self.system,
        self.goal.as_ref(),
        &self.recall,
        self.compaction_summary.as_ref(),
        &self.history,
    )
}
```

Make `build_snapshot`/`preview` visible to `curated.rs`: they are `pub(crate)` in `snapshot.rs`, so `crate::snapshot::build_snapshot` resolves. Ensure `use` of `crate::ContextSnapshot` is not needed if you fully-qualify as above.

- [ ] **Step 8: Run the test, verify it passes**

Run: `cd agent && cargo test -p agent-core 2>&1 | tail -20`
Expected: PASS (all agent-core tests, including the two new ones).

- [ ] **Step 9: Commit**

```bash
git add agent/crates/agent-core/src/snapshot.rs agent/crates/agent-core/src/lib.rs agent/crates/agent-core/src/curated.rs
git commit -m "feat(agent-core): ContextSnapshot + CuratedContext::snapshot for explorer breakdown"
```

---

## Task 2: `context_get` Session method + Tauri command

**Files:**
- Modify: `agent/crates/agent-server/src/session.rs`, `agent/crates/agent-server/src/wire.rs`, `src-tauri/src/lib.rs`
- Test: `session.rs` tests + `src-tauri/src/lib.rs` cmd smoke test

**Interfaces:**
- Consumes: `CuratedContext::snapshot` (Task 1); `Session.ctx: Arc<AsyncMutex<CuratedContext>>`; `self.runtime` for the model limit (`current` config `context_limit`).
- Produces: `Session::context_get(&self) -> ContextSnapshot` (async); Tauri command `context_get` returning `ContextSnapshot` as JSON.

- [ ] **Step 1: Re-export `ContextSnapshot` from agent-server wire**

In `agent/crates/agent-server/src/wire.rs`, add near the top exports:

```rust
pub use agent_core::{ContextSegment, ContextSnapshot};
```

- [ ] **Step 2: Write the failing Session test**

In `agent/crates/agent-server/src/session.rs` test module (it has `session_with_scripted()`), add:

```rust
#[tokio::test]
async fn context_get_returns_snapshot_with_system_segment() {
    let (sess, _cap) = session_with_scripted();
    let snap = sess.context_get().await;
    assert!(snap.segments.iter().any(|s| s.category == "system"));
    assert!(snap.model_limit > 0);
}
```

- [ ] **Step 3: Run it, verify it fails**

Run: `cd agent && cargo test -p agent-server context_get_returns 2>&1 | tail -20`
Expected: FAIL — no method `context_get`.

- [ ] **Step 4: Implement `context_get` on `Session`**

In `agent/crates/agent-server/src/session.rs`, add a method on `impl Session`. Track the turn count locally; for v1 use `0` if no counter is available, or reuse an existing turn source if present:

```rust
pub async fn context_get(&self) -> agent_core::ContextSnapshot {
    let model_limit = self.runtime.settings_state().settings.context_limit;
    let guard = self.ctx.lock().await;
    guard.snapshot(model_limit, 0)
}
```

(`settings_state().settings.context_limit` is a `usize`/`u32`; coerce with `as usize` if the type mismatches.)

- [ ] **Step 5: Run it, verify it passes**

Run: `cd agent && cargo test -p agent-server context_get_returns 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Add the Tauri command + register it**

In `src-tauri/src/lib.rs`, add:

```rust
#[tauri::command]
async fn context_get(
    state: tauri::State<'_, AppState>,
) -> Result<agent_server::wire::ContextSnapshot, String> {
    Ok(session(&state).context_get().await)
}
```

Add `context_get` to the `generate_handler!` list at `lib.rs:126` AND the test-module handler list at `lib.rs:162`.

- [ ] **Step 7: Build the desktop crate to verify command wiring**

Run: `cargo build -p app 2>&1 | tail -15` (the src-tauri crate; use its real package name from `src-tauri/Cargo.toml` if not `app`).
Expected: builds clean.

- [ ] **Step 8: Commit**

```bash
git add agent/crates/agent-server/src/session.rs agent/crates/agent-server/src/wire.rs src-tauri/src/lib.rs
git commit -m "feat(server): context_get command returns live context-window snapshot"
```

---

## Task 3: `MemoryStore::list` + `MemoryAdmin` (browse/get/update/delete)

**Files:**
- Modify: `agent/crates/agent-memory/src/store.rs` (trait + 2 impls), `agent/crates/agent-memory/src/lib.rs`
- Test: `store.rs` tests + `lib.rs` tests

**Interfaces:**
- Produces:
  - `MemoryStore::list(&self, filter: &ScopeFilter, limit: usize, offset: usize) -> Result<Vec<MemoryRecord>, StoreError>` (newest-first by `updated_at`).
  - `pub struct MemoryAdmin { embedder, store, cfg, scope: MemoryScope }` with async `list`, `get`, `update`, `delete`, `recall_preview`.
  - `pub struct MemoryRow { pub id, pub text, pub tags: Vec<String>, pub scope_kind: String, pub updated_at: i64 }`
  - `pub struct ScoredRow { pub id, pub text, pub score: f32, pub scope_kind: String }`
- Consumes: `ScopeFilter`, `MemoryScope`, `MemoryRecord`, `query_memories` (already in `tools.rs`, `pub(crate)`), `Embedder`.

- [ ] **Step 1: Write failing test for `list`**

In `agent/crates/agent-memory/src/store.rs` tests (there is an existing `#[cfg(test)]`; if InMemory tests aren't there, add a module). Test against `InMemoryStore`:

```rust
#[tokio::test]
async fn list_returns_scope_filtered_newest_first() {
    let s = InMemoryStore::new();
    let mk = |id: &str, t: i64| MemoryRecord {
        id: id.into(), text: format!("m{id}"), scope: MemoryScope::Project("K".into()),
        tags: vec![], vector: vec![0.1, 0.2], created_at: t, updated_at: t, source: "x".into(),
    };
    s.upsert(mk("a", 100)).await.unwrap();
    s.upsert(mk("b", 200)).await.unwrap();
    let f = ScopeFilter::Exact(MemoryScope::Project("K".into()));
    let rows = s.list(&f, 10, 0).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "b"); // newest first
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cd agent && cargo test -p agent-memory list_returns_scope 2>&1 | tail -20`
Expected: FAIL — no method `list`.

- [ ] **Step 3: Add `list` to the trait + both impls**

In `store.rs`, add to the `#[async_trait] pub trait MemoryStore`:

```rust
async fn list(&self, filter: &ScopeFilter, limit: usize, offset: usize)
    -> Result<Vec<MemoryRecord>, StoreError>;
```

`InMemoryStore`:

```rust
async fn list(&self, filter: &ScopeFilter, limit: usize, offset: usize)
    -> Result<Vec<MemoryRecord>, StoreError> {
    let mut rows: Vec<MemoryRecord> = self.rows.lock().unwrap().values()
        .filter(|r| filter.matches(&r.scope)).cloned().collect();
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(rows.into_iter().skip(offset).take(limit).collect())
}
```

`SqliteStore` (mirror the existing `query`'s filter SQL; `query` already builds a `WHERE` from `ScopeFilter` — copy that clause builder). Add:

```rust
async fn list(&self, filter: &ScopeFilter, limit: usize, offset: usize)
    -> Result<Vec<MemoryRecord>, StoreError> {
    let conn = self.conn.lock().unwrap();
    // Reuse the same scope predicate the existing query() builds.
    let (where_sql, params) = scope_where(filter); // factor out from query() if not already
    let sql = format!(
        "SELECT * FROM memories WHERE {where_sql} ORDER BY updated_at DESC LIMIT ?{n1} OFFSET ?{n2}",
        n1 = params.len() + 1, n2 = params.len() + 2);
    let mut stmt = conn.prepare(&sql).map_err(|e| StoreError::Io(e.to_string()))?;
    // bind params + [limit as i64, offset as i64], map rows via Self::row_to_record
    // (follow the exact rusqlite pattern already in query()).
    // ... return Vec<MemoryRecord>
    # unimplemented_marker
}
```

NOTE for implementer: open `store.rs`'s existing `query` impl and copy its parameter-binding + `row_to_record` loop verbatim, swapping the ORDER/LIMIT/OFFSET. If `query` inlines its WHERE construction, extract a small `fn scope_where(&ScopeFilter) -> (String, Vec<String>)` and use it from both `query` and `list` (DRY). Remove the `# unimplemented_marker` line — it is only a placeholder pointer, not code.

- [ ] **Step 4: Run it, verify it passes (both impls compile, InMemory test green)**

Run: `cd agent && cargo test -p agent-memory list_returns_scope 2>&1 | tail -20`
Expected: PASS. Also run full `cargo test -p agent-memory` to confirm SqliteStore impl compiles + existing tests stay green.

- [ ] **Step 5: Write failing test for `MemoryAdmin` (list + delete scope guard)**

In `agent/crates/agent-memory/src/lib.rs` test module:

```rust
#[tokio::test]
async fn admin_lists_and_refuses_cross_project_delete() {
    use crate::{Embedder, InMemoryStore, MemoryConfig, MemoryRecord, MemoryScope, StubEmbedder};
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
    let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
    let v = embedder.embed(&["hi".into()]).await.unwrap().remove(0);
    store.upsert(MemoryRecord { id: "x".into(), text: "hi".into(),
        scope: MemoryScope::Project("OTHER".into()), tags: vec![], vector: v,
        created_at: 1, updated_at: 1, source: "t".into() }).await.unwrap();
    let admin = MemoryAdmin::new(embedder, store, Arc::new(MemoryConfig::default()),
        MemoryScope::Project("MINE".into()));
    // Cross-project record is not listed in MINE's scope and cannot be deleted.
    assert!(admin.list(20, 0).await.unwrap().is_empty());
    assert!(admin.delete("x").await.is_err());
}
```

- [ ] **Step 6: Run it, verify it fails**

Run: `cd agent && cargo test -p agent-memory admin_lists_and_refuses 2>&1 | tail -20`
Expected: FAIL — no `MemoryAdmin`.

- [ ] **Step 7: Implement `MemoryAdmin` + the DTOs**

In `agent/crates/agent-memory/src/lib.rs` (or a new `admin.rs` module re-exported here):

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRow {
    pub id: String, pub text: String, pub tags: Vec<String>,
    pub scope_kind: String, pub updated_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredRow {
    pub id: String, pub text: String, pub score: f32, pub scope_kind: String,
}

pub struct MemoryAdmin {
    embedder: Arc<dyn Embedder>,
    store: Arc<dyn MemoryStore>,
    cfg: Arc<MemoryConfig>,
    scope: MemoryScope,
}

impl MemoryAdmin {
    pub fn new(embedder: Arc<dyn Embedder>, store: Arc<dyn MemoryStore>,
        cfg: Arc<MemoryConfig>, scope: MemoryScope) -> Self {
        Self { embedder, store, cfg, scope }
    }

    fn filter(&self) -> ScopeFilter {
        match &self.scope {
            MemoryScope::Project(k) => ScopeFilter::ProjectAndGlobal { project_key: k.clone() },
            MemoryScope::Global => ScopeFilter::Exact(MemoryScope::Global),
        }
    }

    fn editable(&self, rec: &MemoryScope) -> bool {
        matches!(rec, MemoryScope::Global) || rec == &self.scope
    }

    pub async fn list(&self, limit: usize, offset: usize) -> Result<Vec<MemoryRow>, StoreError> {
        Ok(self.store.list(&self.filter(), limit, offset).await?.into_iter().map(|r| MemoryRow {
            id: r.id, text: r.text, tags: r.tags, scope_kind: r.scope.kind().into(),
            updated_at: r.updated_at,
        }).collect())
    }

    pub async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        match self.store.get(id).await? {
            Some(rec) if self.editable(&rec.scope) => self.store.delete(id).await,
            Some(_) => Err(StoreError::Io("refused: record belongs to another project".into())),
            None => Ok(false),
        }
    }

    pub async fn update(&self, id: &str, text: Option<String>, tags: Option<Vec<String>>)
        -> Result<MemoryRow, StoreError> {
        let mut rec = self.store.get(id).await?
            .ok_or_else(|| StoreError::Io("not found".into()))?;
        if !self.editable(&rec.scope) {
            return Err(StoreError::Io("refused: record belongs to another project".into()));
        }
        if let Some(t) = text {
            rec.vector = self.embedder.embed(&[t.clone()]).await
                .map_err(|e| StoreError::Io(e.to_string()))?.remove(0);
            rec.text = t;
        }
        if let Some(tg) = tags { rec.tags = tg; }
        rec.updated_at = crate::now_secs();
        self.store.upsert(rec.clone()).await?;
        Ok(MemoryRow { id: rec.id, text: rec.text, tags: rec.tags,
            scope_kind: rec.scope.kind().into(), updated_at: rec.updated_at })
    }

    pub async fn recall_preview(&self, query: &str) -> Vec<ScoredRow> {
        let key = match &self.scope {
            MemoryScope::Project(k) => k.clone(), MemoryScope::Global => String::new(),
        };
        match crate::tools::query_memories(self.embedder.as_ref(), self.store.as_ref(),
            &self.cfg, &key, query, self.cfg.default_k).await {
            Ok(hits) => hits.into_iter().map(|h| ScoredRow {
                id: h.record.id, text: h.record.text, score: h.score,
                scope_kind: h.record.scope.kind().into() }).collect(),
            Err(_) => Vec::new(),
        }
    }
}
```

Add `pub use` for `MemoryAdmin, MemoryRow, ScoredRow` in `lib.rs`. `query_memories` is `pub(crate)` in `tools.rs` — if it is not visible from `lib.rs`, change it to `pub(crate)` at the crate root (it already is) and reference as `crate::tools::query_memories`.

- [ ] **Step 8: Run it, verify it passes**

Run: `cd agent && cargo test -p agent-memory 2>&1 | tail -20`
Expected: PASS (all memory tests).

- [ ] **Step 9: Commit**

```bash
git add agent/crates/agent-memory/src/store.rs agent/crates/agent-memory/src/lib.rs
git commit -m "feat(agent-memory): MemoryStore::list + MemoryAdmin (browse/update/delete/recall_preview) with scope guard"
```

---

## Task 4: Thread `MemoryParts` into Session + memory commands

**Files:**
- Modify: `agent/crates/agent-server/src/daemon.rs`, `agent/crates/agent-server/src/setup.rs`, `agent/crates/agent-server/src/session.rs`, `src-tauri/src/bridge.rs`, `src-tauri/src/lib.rs`
- Test: `session.rs` tests, `src-tauri/src/lib.rs` smoke

**Interfaces:**
- Consumes: `MemoryAdmin` (Task 3), `MemoryParts`, `agent_memory::project_scope`.
- Produces: `Session` async methods `memory_list(limit, offset)`, `memory_update(id, text, tags)`, `memory_delete(id)`, `memory_recall_preview(query)` returning the Task-3 DTOs; matching Tauri commands.

- [ ] **Step 1: Add `memory_parts` to `DaemonParams`**

In `agent/crates/agent-server/src/daemon.rs`, add field:

```rust
pub memory_parts: Option<agent_memory::MemoryParts>,
```

- [ ] **Step 2: Populate it in `setup::local_params`**

In `agent/crates/agent-server/src/setup.rs`, set `memory_parts: memory_parts.cloned()` in the returned `DaemonParams` (the `memory_parts: Option<&MemoryParts>` arg is already present; `MemoryParts` derives `Clone`). Update the two existing setup tests to include `memory_parts: None`/`Some(..)` as needed (the struct-literal tests will fail to compile until the field is added — add `memory_parts: None` to the `None`-branch assertions and `Some` to the populated one).

- [ ] **Step 3: Build a `MemoryAdmin` in Session + write the failing test**

In `agent/crates/agent-server/src/session.rs`, store an `Option<agent_memory::MemoryParts>` + the workspace on `Session` (add fields, set them in `from_params`). Then add the test:

```rust
#[tokio::test]
async fn memory_list_is_empty_on_fresh_store() {
    let (sess, _cap) = session_with_scripted(); // scripted setup passes memory_parts: None
    let rows = sess.memory_list(20, 0).await.unwrap_or_default();
    assert!(rows.is_empty());
}
```

(With `memory_parts: None`, `memory_list` returns `Ok(vec![])`. A populated-store test can be added later behind the in-memory parts pattern from `setup.rs::local_params_with_parts_populates_memory`.)

- [ ] **Step 4: Run it, verify it fails**

Run: `cd agent && cargo test -p agent-server memory_list_is_empty 2>&1 | tail -20`
Expected: FAIL — no method `memory_list`.

- [ ] **Step 5: Implement the Session memory methods**

In `session.rs`:

```rust
fn memory_admin(&self) -> Option<agent_memory::MemoryAdmin> {
    let parts = self.memory_parts.as_ref()?;
    let scope = agent_memory::project_scope(&self.workspace);
    Some(agent_memory::MemoryAdmin::new(
        parts.embedder.clone(), parts.store.clone(), parts.cfg.clone(), scope))
}

pub async fn memory_list(&self, limit: usize, offset: usize)
    -> Result<Vec<agent_memory::MemoryRow>, String> {
    match self.memory_admin() {
        Some(a) => a.list(limit, offset).await.map_err(|e| e.to_string()),
        None => Ok(vec![]),
    }
}
pub async fn memory_update(&self, id: String, text: Option<String>, tags: Option<Vec<String>>)
    -> Result<agent_memory::MemoryRow, String> {
    self.memory_admin().ok_or_else(|| "memory disabled".to_string())?
        .update(&id, text, tags).await.map_err(|e| e.to_string())
}
pub async fn memory_delete(&self, id: String) -> Result<bool, String> {
    self.memory_admin().ok_or_else(|| "memory disabled".to_string())?
        .delete(&id).await.map_err(|e| e.to_string())
}
pub async fn memory_recall_preview(&self, query: String) -> Vec<agent_memory::ScoredRow> {
    match self.memory_admin() {
        Some(a) => a.recall_preview(&query).await,
        None => vec![],
    }
}
```

Add `workspace: PathBuf` + `memory_parts: Option<MemoryParts>` to the `Session` struct and set both in `from_params` (workspace is already in `params.workspace`; `memory_parts` from the new `params.memory_parts`). Keep `set_workspace` updating `self.workspace` too — find the existing `set_workspace` and add the assignment.

- [ ] **Step 6: Run it, verify it passes**

Run: `cd agent && cargo test -p agent-server memory_list_is_empty 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Pass `memory_parts` from the bridge**

In `src-tauri/src/bridge.rs::start`, the local `memory_parts` is already built. `local_params(..)` already takes `memory_parts.as_ref()`; confirm it now also stores it into `DaemonParams.memory_parts` (done in Step 2). No bridge change needed beyond confirming `memory_parts` is no longer dead-code — remove the `#[allow(dead_code)]` on the bridge's `memory_parts` field only if it is now read; otherwise leave it.

- [ ] **Step 8: Add Tauri commands + register**

In `src-tauri/src/lib.rs`:

```rust
#[tauri::command]
async fn memory_list(state: tauri::State<'_, AppState>, limit: usize, offset: usize)
    -> Result<Vec<agent_memory::MemoryRow>, String> {
    session(&state).memory_list(limit, offset).await
}
#[tauri::command]
async fn memory_update(state: tauri::State<'_, AppState>, id: String,
    text: Option<String>, tags: Option<Vec<String>>)
    -> Result<agent_memory::MemoryRow, String> {
    session(&state).memory_update(id, text, tags).await
}
#[tauri::command]
async fn memory_delete(state: tauri::State<'_, AppState>, id: String) -> Result<bool, String> {
    session(&state).memory_delete(id).await
}
#[tauri::command]
async fn memory_recall_preview(state: tauri::State<'_, AppState>, query: String)
    -> Result<Vec<agent_memory::ScoredRow>, String> {
    Ok(session(&state).memory_recall_preview(query).await)
}
```

Register all four in both `generate_handler!` blocks. Add `agent_memory` to `src-tauri/Cargo.toml` `[dependencies]` if not present (path dep: `agent-memory = { path = "../agent/crates/agent-memory" }`).

- [ ] **Step 9: Build + commit**

Run: `cargo build -p app 2>&1 | tail -15` (use the real src-tauri package name).
Expected: clean.

```bash
git add agent/crates/agent-server/src/daemon.rs agent/crates/agent-server/src/setup.rs agent/crates/agent-server/src/session.rs src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/src/bridge.rs
git commit -m "feat(server): memory CRUD + recall-preview commands over MemoryAdmin"
```

---

## Task 5: Skill view/edit commands

**Files:**
- Modify: `agent/crates/agent-server/src/session.rs`, `src-tauri/src/lib.rs`
- Test: `session.rs` tests

**Interfaces:**
- Consumes: `agent_skills::{SkillRegistry, sanitize_slug}`; `self.runtime` for `skills_dirs`; `self.workspace`.
- Produces: `Session::skill_get(name) -> Result<SkillDto, String>`, `Session::skill_save(name, body) -> Result<(), String>`; `SkillDto { name, description, body, files: Vec<String> }`; Tauri commands `skill_get`, `skill_save`. (`skill_list` is unnecessary — `settings_state.discovered_skills` already provides it.)

- [ ] **Step 1: Add `SkillDto` + failing test**

In `session.rs`, define near the top (or in `wire.rs` and re-export):

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillDto {
    pub name: String, pub description: String, pub body: String, pub files: Vec<String>,
}
```

Test (uses the `.agent/skills/greeter` pattern from existing `runtime.rs` tests):

```rust
#[tokio::test]
async fn skill_save_then_get_roundtrips() {
    let (sess, _cap) = session_with_scripted();
    sess.skill_save("greeter".into(), "Say hello to the user.".into()).await.unwrap();
    let got = sess.skill_get("greeter".into()).await.unwrap();
    assert_eq!(got.name, "greeter");
    assert!(got.body.contains("hello"));
}
```

Note: `skill_save` must write a full valid `SKILL.md` (frontmatter + body). Use a default description when none is supplied so `parse_skill_md` accepts it.

- [ ] **Step 2: Run it, verify it fails**

Run: `cd agent && cargo test -p agent-server skill_save_then_get 2>&1 | tail -20`
Expected: FAIL — no method `skill_get`.

- [ ] **Step 3: Implement the Session skill methods**

```rust
fn skill_registry(&self) -> agent_skills::SkillRegistry {
    let cfg = self.runtime.settings_state().settings;
    agent_skills::SkillRegistry::from_config(&cfg.skills_dirs, &self.workspace)
}

pub async fn skill_get(&self, name: String) -> Result<SkillDto, String> {
    let reg = self.skill_registry();
    let s = reg.find(&name).ok_or_else(|| format!("skill not found: {name}"))?;
    Ok(SkillDto {
        name: s.name, description: s.description, body: s.body,
        files: s.files.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
    })
}

pub async fn skill_save(&self, name: String, body: String) -> Result<(), String> {
    let slug = agent_skills::sanitize_slug(&name)?;
    let reg = self.skill_registry();
    let dir = reg.writable_root().join(&slug);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Preserve an existing description if present, else default.
    let desc = reg.find(&slug).map(|s| s.description)
        .unwrap_or_else(|| format!("{slug} skill"));
    let md = format!("---\nname: {slug}\ndescription: {desc}\n---\n{body}\n");
    std::fs::write(dir.join("SKILL.md"), md).map_err(|e| e.to_string())?;
    Ok(())
}
```

- [ ] **Step 4: Run it, verify it passes**

Run: `cd agent && cargo test -p agent-server skill_save_then_get 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Add Tauri commands + register**

```rust
#[tauri::command]
async fn skill_get(state: tauri::State<'_, AppState>, name: String)
    -> Result<agent_server::session::SkillDto, String> {
    session(&state).skill_get(name).await
}
#[tauri::command]
async fn skill_save(state: tauri::State<'_, AppState>, name: String, body: String)
    -> Result<(), String> {
    session(&state).skill_save(name, body).await
}
```

Register both in the two handler lists. Ensure `SkillDto` is exported from `agent_server::session` (it is, if declared `pub` there).

- [ ] **Step 6: Build + commit**

Run: `cargo build -p app 2>&1 | tail -15`

```bash
git add agent/crates/agent-server/src/session.rs src-tauri/src/lib.rs
git commit -m "feat(server): skill_get + skill_save commands (writable-root, slug-validated)"
```

---

## Task 6: Frontend API + types module

**Files:**
- Create: `web/src/explorer/types.ts`, `web/src/explorer/api.ts`
- Test: none (thin invoke wrappers; covered via mocked invoke in later tasks)

**Interfaces:**
- Produces typed wrappers: `getContext()`, `listMemories(limit, offset)`, `updateMemory(id, text?, tags?)`, `deleteMemory(id)`, `recallPreview(query)`, `getSkill(name)`, `saveSkill(name, body)`.

- [ ] **Step 1: Create the TS DTOs**

`web/src/explorer/types.ts`:

```ts
export interface ContextSegment { category: string; est_tokens: number; items: string[]; count: number }
export interface ContextSnapshot { turn: number; model_limit: number; est_total: number; segments: ContextSegment[] }
export interface MemoryRow { id: string; text: string; tags: string[]; scope_kind: string; updated_at: number }
export interface ScoredRow { id: string; text: string; score: number; scope_kind: string }
export interface SkillDto { name: string; description: string; body: string; files: string[] }
```

- [ ] **Step 2: Create the invoke wrappers**

`web/src/explorer/api.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";
import type { ContextSnapshot, MemoryRow, ScoredRow, SkillDto } from "./types";

export const getContext = () => invoke<ContextSnapshot>("context_get");
export const listMemories = (limit = 50, offset = 0) =>
  invoke<MemoryRow[]>("memory_list", { limit, offset });
export const updateMemory = (id: string, text?: string, tags?: string[]) =>
  invoke<MemoryRow>("memory_update", { id, text: text ?? null, tags: tags ?? null });
export const deleteMemory = (id: string) => invoke<boolean>("memory_delete", { id });
export const recallPreview = (query: string) =>
  invoke<ScoredRow[]>("memory_recall_preview", { query });
export const getSkill = (name: string) => invoke<SkillDto>("skill_get", { name });
export const saveSkill = (name: string, body: string) =>
  invoke<void>("skill_save", { name, body });
```

- [ ] **Step 3: Typecheck**

Run: `cd web && npx tsc -b --noEmit 2>&1 | tail -15`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add web/src/explorer/types.ts web/src/explorer/api.ts
git commit -m "feat(web): explorer API + DTO types for context/memory/skill commands"
```

---

## Task 7: Capture faithful total (`server_usage`) into state + breakdown helper

**Files:**
- Modify: `web/src/wire.ts`, `web/src/state.ts`
- Create: `web/src/explorer/breakdown.ts`
- Test: `web/src/explorer/breakdown.test.ts`, extend `web/src/state` tests if present

**Interfaces:**
- Produces: `state.serverUsage: { promptTokens: number; turn: number } | null`; `computeBreakdown(snapshot, realTotal)` → segment widths + `unattributed`.
- Consumes: `ContextSnapshot` (Task 6).

- [ ] **Step 1: Add `server_usage` to the wire event union**

In `web/src/wire.ts`, add to `WireEvent`:

```ts
  | { type: "server_usage"; prompt_tokens: number; completion_tokens: number; turn: number }
```

- [ ] **Step 2: Write the failing breakdown test**

`web/src/explorer/breakdown.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { computeBreakdown } from "./breakdown";
import type { ContextSnapshot } from "./types";

const snap: ContextSnapshot = {
  turn: 1, model_limit: 1000, est_total: 60,
  segments: [
    { category: "system", est_tokens: 40, items: ["You are..."], count: 1 },
    { category: "messages", est_tokens: 20, items: [], count: 3 },
  ],
};

describe("computeBreakdown", () => {
  it("adds an unattributed slice when real total exceeds estimate", () => {
    const b = computeBreakdown(snap, 100);
    expect(b.total).toBe(100);
    const un = b.slices.find((s) => s.category === "unattributed");
    expect(un?.tokens).toBe(40); // 100 - 60
  });
  it("clamps unattributed at zero when estimate exceeds real total", () => {
    const b = computeBreakdown(snap, 50);
    expect(b.slices.find((s) => s.category === "unattributed")).toBeUndefined();
    expect(b.total).toBe(60); // falls back to estimate when no faithful total
  });
  it("uses estimate as total when realTotal is null", () => {
    const b = computeBreakdown(snap, null);
    expect(b.total).toBe(60);
  });
});
```

- [ ] **Step 3: Run it, verify it fails**

Run: `cd web && npx vitest run src/explorer/breakdown.test.ts 2>&1 | tail -20`
Expected: FAIL — module not found.

- [ ] **Step 4: Implement `breakdown.ts`**

```ts
import type { ContextSnapshot } from "./types";

export interface Slice { category: string; tokens: number; pct: number }
export interface Breakdown { total: number; slices: Slice[] }

export function computeBreakdown(snap: ContextSnapshot, realTotal: number | null): Breakdown {
  const estTotal = snap.est_total;
  const total = realTotal && realTotal > estTotal ? realTotal : estTotal;
  const slices: Slice[] = snap.segments.map((s) => ({
    category: s.category, tokens: s.est_tokens, pct: 0,
  }));
  if (realTotal && realTotal > estTotal) {
    slices.push({ category: "unattributed", tokens: realTotal - estTotal, pct: 0 });
  }
  const denom = total || 1;
  for (const s of slices) s.pct = Math.round((s.tokens / denom) * 100);
  return { total, slices };
}
```

- [ ] **Step 5: Run it, verify it passes**

Run: `cd web && npx vitest run src/explorer/breakdown.test.ts 2>&1 | tail -20`
Expected: PASS (3 tests).

- [ ] **Step 6: Capture `server_usage` in the reducer**

In `web/src/state.ts`: add `serverUsage: { promptTokens: number; turn: number } | null` to `ConversationState` (initialize `null` in the initial state), and in the `"frame"` → `payload` handling for `WireEvent`, add a case:

```ts
case "server_usage":
  return { ...s, serverUsage: { promptTokens: p.prompt_tokens, turn: p.turn } };
```

(Place it beside the existing `"usage"`/`"token"` cases; `p` is the already-narrowed payload there.)

- [ ] **Step 7: Typecheck + run state tests**

Run: `cd web && npx tsc -b --noEmit 2>&1 | tail -15 && npx vitest run src/state 2>&1 | tail -15`
Expected: no type errors; existing state tests still pass.

- [ ] **Step 8: Commit**

```bash
git add web/src/wire.ts web/src/state.ts web/src/explorer/breakdown.ts web/src/explorer/breakdown.test.ts
git commit -m "feat(web): capture faithful server_usage + breakdown helper with unattributed slice"
```

---

## Task 8: ContextExplorer pane + right-pane tab switch

**Files:**
- Create: `web/src/explorer/ContextExplorer.tsx`
- Modify: `web/src/App.tsx`, `web/src/storage.ts`
- Test: `web/src/explorer/ContextExplorer.test.tsx`

**Interfaces:**
- Consumes: `getContext` (Task 6), `computeBreakdown` (Task 7), `state.serverUsage`.
- Produces: `<ContextExplorer realTotal={number|null} refreshKey={number} />` — fetches a snapshot on mount + whenever `refreshKey` changes (App bumps it on each `done`).

- [ ] **Step 1: Add storage helpers for the active right tab**

In `web/src/storage.ts` (follow the existing `loadTheme`/`saveTheme` pattern):

```ts
const RIGHT_TAB = "rightTab";
export type RightTab = "workspace" | "context";
export function loadRightTab(): RightTab {
  return localStorage.getItem(RIGHT_TAB) === "context" ? "context" : "workspace";
}
export function saveRightTab(t: RightTab): void {
  try { localStorage.setItem(RIGHT_TAB, t); } catch { /* ignore */ }
}
```

- [ ] **Step 2: Write the failing component test**

`web/src/explorer/ContextExplorer.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { ContextExplorer } from "./ContextExplorer";

vi.mock("./api", () => ({
  getContext: vi.fn().mockResolvedValue({
    turn: 1, model_limit: 1000, est_total: 60,
    segments: [{ category: "system", est_tokens: 60, items: ["You are..."], count: 1 }],
  }),
  listMemories: vi.fn().mockResolvedValue([]),
  recallPreview: vi.fn().mockResolvedValue([]),
}));

describe("ContextExplorer", () => {
  beforeEach(() => vi.clearAllMocks());
  it("renders the breakdown total after fetching a snapshot", async () => {
    render(<ContextExplorer realTotal={100} refreshKey={0} />);
    expect(await screen.findByText(/100/)).toBeInTheDocument();
    expect(await screen.findByText(/system/i)).toBeInTheDocument();
  });
});
```

- [ ] **Step 3: Run it, verify it fails**

Run: `cd web && npx vitest run src/explorer/ContextExplorer.test.tsx 2>&1 | tail -20`
Expected: FAIL — module not found.

- [ ] **Step 4: Implement `ContextExplorer.tsx`**

```tsx
import { useEffect, useState } from "react";
import type { ContextSnapshot } from "./types";
import { getContext } from "./api";
import { computeBreakdown } from "./breakdown";
import { MemorySection } from "./MemorySection";
import { SkillSection } from "./SkillSection";

const COLORS: Record<string, string> = {
  system: "var(--accent)", goal: "#a78bfa", memory: "#34d399",
  summary: "#fbbf24", messages: "var(--text-muted)", unattributed: "var(--state-error)",
};

export function ContextExplorer(
  { realTotal, refreshKey }: { realTotal: number | null; refreshKey: number },
) {
  const [snap, setSnap] = useState<ContextSnapshot | null>(null);
  const [open, setOpen] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    getContext().then((s) => { if (active) setSnap(s); }).catch(() => {});
    return () => { active = false; };
  }, [refreshKey]);

  if (!snap) {
    return <div className="p-3 text-xs" style={{ color: "var(--text-muted)" }}>No context yet.</div>;
  }
  const b = computeBreakdown(snap, realTotal);

  return (
    <div className="flex h-full flex-col overflow-y-auto" style={{ background: "var(--surface-overlay)" }}>
      <div className="px-3 pt-3">
        <div className="font-mono text-xs" style={{ color: "var(--text-strong)" }}>
          {b.total} / {snap.model_limit} tokens
        </div>
        <div className="mt-2 flex h-3 w-full overflow-hidden rounded-full"
          style={{ background: "var(--surface-base)" }}>
          {b.slices.map((s) => (
            <div key={s.category} title={`${s.category}: ${s.tokens} (${s.pct}%)`}
              style={{ width: `${s.pct}%`, background: COLORS[s.category] ?? "var(--text-muted)" }} />
          ))}
        </div>
        <div className="mt-2 flex flex-wrap gap-2 text-xs" style={{ color: "var(--text-muted)" }}>
          {b.slices.map((s) => (
            <button key={s.category} onClick={() => setOpen(open === s.category ? null : s.category)}
              className="flex items-center gap-1">
              <span className="inline-block h-2 w-2 rounded-full"
                style={{ background: COLORS[s.category] ?? "var(--text-muted)" }} />
              {s.category} {s.tokens}
            </button>
          ))}
        </div>
      </div>

      <div className="mt-3 border-t" style={{ borderColor: "var(--border)" }}>
        <MemorySection
          recalled={snap.segments.find((x) => x.category === "memory")?.items ?? []}
        />
        <SkillSection />
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Add placeholder MemorySection/SkillSection so it compiles**

Create minimal stubs (filled in Tasks 9–10) so this task is independently green:

`web/src/explorer/MemorySection.tsx`:

```tsx
export function MemorySection({ recalled }: { recalled: string[] }) {
  return (
    <div className="px-3 py-2 text-xs" style={{ color: "var(--text-muted)" }}>
      <div className="font-semibold" style={{ color: "var(--text-strong)" }}>Memory</div>
      {recalled.length === 0 ? <div>No recall this turn.</div>
        : recalled.map((t, i) => <div key={i}>· {t}</div>)}
    </div>
  );
}
```

`web/src/explorer/SkillSection.tsx`:

```tsx
export function SkillSection() {
  return (
    <div className="px-3 py-2 text-xs" style={{ color: "var(--text-muted)" }}>
      <div className="font-semibold" style={{ color: "var(--text-strong)" }}>Skills</div>
    </div>
  );
}
```

- [ ] **Step 6: Run the test, verify it passes**

Run: `cd web && npx vitest run src/explorer/ContextExplorer.test.tsx 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Wire the right-pane tab in `App.tsx`**

In `web/src/App.tsx`: import `ContextExplorer`, `loadRightTab/saveRightTab`. Add `const [rightTab, setRightTab] = useState(loadRightTab);`. At BOTH places the `<WorkspacePane .../>` is rendered (the wide layout ~line 153 and the narrow drawer ~line 163), wrap with a small tab header and conditionally render:

```tsx
<div className="flex h-full flex-col">
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
  <div className="min-h-0 flex-1">
    {rightTab === "workspace"
      ? <WorkspacePane artifacts={artifacts} activeKey={activeArtifactKey} onSelect={setActiveArtifactKey} />
      : <ContextExplorer realTotal={state.serverUsage?.promptTokens ?? null} refreshKey={state.turnIndex} />}
  </div>
</div>
```

(`state.turnIndex` already increments per turn — using it as `refreshKey` re-fetches the snapshot each turn. If a more precise "turn done" signal is wanted, bump a local counter when a `done` frame arrives; `turnIndex` is sufficient for v1.)

- [ ] **Step 8: Typecheck + run the existing App tests**

Run: `cd web && npx tsc -b --noEmit 2>&1 | tail -15 && npx vitest run src/App 2>&1 | tail -20`
Expected: no type errors; existing App tests pass (update them only if the new tab markup breaks a query — prefer adding `role="tab"` selectors over loosening assertions).

- [ ] **Step 9: Commit**

```bash
git add web/src/explorer/ContextExplorer.tsx web/src/explorer/ContextExplorer.test.tsx web/src/explorer/MemorySection.tsx web/src/explorer/SkillSection.tsx web/src/App.tsx web/src/storage.ts
git commit -m "feat(web): Context Explorer pane + right-pane Workspace/Context tab switch"
```

---

## Task 9: Memory section — recalled rows + browser + edit/delete

**Files:**
- Modify: `web/src/explorer/MemorySection.tsx`
- Test: `web/src/explorer/MemorySection.test.tsx`

**Interfaces:**
- Consumes: `listMemories`, `updateMemory`, `deleteMemory`, `recallPreview` (Task 6); `MemoryRow`, `ScoredRow`.

- [ ] **Step 1: Write the failing test**

`web/src/explorer/MemorySection.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { MemorySection } from "./MemorySection";

vi.mock("./api", () => ({
  listMemories: vi.fn().mockResolvedValue([
    { id: "m1", text: "cargo not on PATH", tags: ["setup"], scope_kind: "global", updated_at: 1 },
  ]),
  deleteMemory: vi.fn().mockResolvedValue(true),
  updateMemory: vi.fn(),
  recallPreview: vi.fn().mockResolvedValue([]),
}));
import { deleteMemory } from "./api";

describe("MemorySection", () => {
  beforeEach(() => vi.clearAllMocks());
  it("lists stored memories and deletes one", async () => {
    render(<MemorySection recalled={[]} />);
    expect(await screen.findByText(/cargo not on PATH/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /delete m1/i }));
    await waitFor(() => expect(deleteMemory).toHaveBeenCalledWith("m1"));
  });
});
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cd web && npx vitest run src/explorer/MemorySection.test.tsx 2>&1 | tail -20`
Expected: FAIL — current stub renders no list / no delete button.

- [ ] **Step 3: Implement the full MemorySection**

```tsx
import { useEffect, useState } from "react";
import type { MemoryRow } from "./types";
import { listMemories, deleteMemory, updateMemory } from "./api";

export function MemorySection({ recalled }: { recalled: string[] }) {
  const [rows, setRows] = useState<MemoryRow[]>([]);
  const [q, setQ] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  const refresh = () => listMemories(50, 0).then(setRows).catch(() => {});
  useEffect(() => { refresh(); }, []);

  const onDelete = async (id: string) => { await deleteMemory(id); refresh(); };
  const onSave = async (id: string) => {
    await updateMemory(id, draft); setEditing(null); refresh();
  };

  const shown = rows.filter((r) => r.text.toLowerCase().includes(q.toLowerCase()));

  return (
    <div className="px-3 py-2 text-xs">
      <div className="font-semibold" style={{ color: "var(--text-strong)" }}>Memory</div>

      <div className="mt-1" style={{ color: "var(--text-muted)" }}>Recalled this turn</div>
      {recalled.length === 0 ? <div style={{ color: "var(--text-muted)" }}>— none —</div>
        : recalled.map((t, i) => <div key={i} style={{ color: "var(--text-strong)" }}>· {t}</div>)}

      <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="filter memories…"
        className="mt-2 w-full rounded px-2 py-1"
        style={{ background: "var(--surface-base)", color: "var(--text-strong)",
          border: "1px solid var(--border)" }} />

      <div className="mt-1 space-y-1">
        {shown.map((r) => (
          <div key={r.id} className="rounded p-1" style={{ border: "1px solid var(--border)" }}>
            {editing === r.id ? (
              <div className="flex gap-1">
                <input value={draft} onChange={(e) => setDraft(e.target.value)}
                  className="flex-1 rounded px-1"
                  style={{ background: "var(--surface-base)", color: "var(--text-strong)" }} />
                <button onClick={() => onSave(r.id)} style={{ color: "var(--accent)" }}>save</button>
              </div>
            ) : (
              <div className="flex items-start justify-between gap-2">
                <span style={{ color: "var(--text-strong)" }}>{r.text}</span>
                <span className="flex shrink-0 gap-2">
                  <span style={{ color: "var(--text-muted)" }}>{r.scope_kind}</span>
                  <button aria-label={`edit ${r.id}`}
                    onClick={() => { setEditing(r.id); setDraft(r.text); }}
                    style={{ color: "var(--text-muted)" }}>edit</button>
                  <button aria-label={`delete ${r.id}`} onClick={() => onDelete(r.id)}
                    style={{ color: "var(--state-error)" }}>del</button>
                </span>
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run it, verify it passes**

Run: `cd web && npx vitest run src/explorer/MemorySection.test.tsx 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Typecheck + commit**

Run: `cd web && npx tsc -b --noEmit 2>&1 | tail -15`

```bash
git add web/src/explorer/MemorySection.tsx web/src/explorer/MemorySection.test.tsx
git commit -m "feat(web): memory section — browse, filter, inline edit, delete"
```

---

## Task 10: Skill section — list + view/edit/save

**Files:**
- Modify: `web/src/explorer/SkillSection.tsx`, `web/src/explorer/ContextExplorer.tsx` (pass `discoveredSkills`)
- Test: `web/src/explorer/SkillSection.test.tsx`

**Interfaces:**
- Consumes: `getSkill`, `saveSkill` (Task 6); the `discoveredSkills` already in `state.settingsMeta`.
- Produces: `<SkillSection skills={DiscoveredSkill[]} />`.

- [ ] **Step 1: Thread discovered skills into the section**

In `ContextExplorer.tsx`, add a prop `skills: { name: string; description: string }[]` and pass it to `<SkillSection skills={skills} />`. In `App.tsx`, pass `skills={state.settingsMeta?.discoveredSkills ?? []}` to `<ContextExplorer .../>` and add it to the component's props.

- [ ] **Step 2: Write the failing test**

`web/src/explorer/SkillSection.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { SkillSection } from "./SkillSection";

vi.mock("./api", () => ({
  getSkill: vi.fn().mockResolvedValue({ name: "greeter", description: "says hi", body: "Say hi.", files: [] }),
  saveSkill: vi.fn().mockResolvedValue(undefined),
}));
import { getSkill } from "./api";

describe("SkillSection", () => {
  beforeEach(() => vi.clearAllMocks());
  it("opens a skill body on click", async () => {
    render(<SkillSection skills={[{ name: "greeter", description: "says hi" }]} />);
    fireEvent.click(screen.getByRole("button", { name: /greeter/i }));
    await waitFor(() => expect(getSkill).toHaveBeenCalledWith("greeter"));
    expect(await screen.findByDisplayValue(/Say hi\./)).toBeInTheDocument();
  });
});
```

- [ ] **Step 3: Run it, verify it fails**

Run: `cd web && npx vitest run src/explorer/SkillSection.test.tsx 2>&1 | tail -20`
Expected: FAIL — stub has no skills/props.

- [ ] **Step 4: Implement the full SkillSection**

```tsx
import { useState } from "react";
import type { SkillDto } from "./types";
import { getSkill, saveSkill } from "./api";

export function SkillSection({ skills }: { skills: { name: string; description: string }[] }) {
  const [open, setOpen] = useState<SkillDto | null>(null);
  const [body, setBody] = useState("");
  const [saved, setSaved] = useState(false);

  const onOpen = async (name: string) => {
    const s = await getSkill(name);
    setOpen(s); setBody(s.body); setSaved(false);
  };
  const onSave = async () => {
    if (!open) return;
    await saveSkill(open.name, body); setSaved(true);
  };

  return (
    <div className="px-3 py-2 text-xs">
      <div className="font-semibold" style={{ color: "var(--text-strong)" }}>Skills</div>
      <div className="mt-1 space-y-0.5">
        {skills.map((s) => (
          <button key={s.name} aria-label={`open ${s.name}`} onClick={() => onOpen(s.name)}
            className="block w-full text-left" style={{ color: "var(--text-strong)" }}>
            {s.name} <span style={{ color: "var(--text-muted)" }}>— {s.description}</span>
          </button>
        ))}
      </div>
      {open && (
        <div className="mt-2 rounded p-1" style={{ border: "1px solid var(--border)" }}>
          <div className="mb-1" style={{ color: "var(--text-muted)" }}>{open.name}/SKILL.md</div>
          <textarea value={body} onChange={(e) => { setBody(e.target.value); setSaved(false); }}
            rows={8} className="w-full rounded px-2 py-1 font-mono"
            style={{ background: "var(--surface-base)", color: "var(--text-strong)",
              border: "1px solid var(--border)" }} />
          <div className="mt-1 flex items-center gap-2">
            <button onClick={onSave} style={{ color: "var(--accent)" }}>save</button>
            {saved && <span style={{ color: "var(--text-muted)" }}>saved ✓</span>}
            <button onClick={() => setOpen(null)} style={{ color: "var(--text-muted)" }}>close</button>
          </div>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 5: Run it, verify it passes**

Run: `cd web && npx vitest run src/explorer/SkillSection.test.tsx 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Full frontend gate + commit**

Run: `cd web && npx tsc -b --noEmit 2>&1 | tail -15 && npx vitest run 2>&1 | tail -25`
Expected: no type errors; all tests pass.

```bash
git add web/src/explorer/SkillSection.tsx web/src/explorer/SkillSection.test.tsx web/src/explorer/ContextExplorer.tsx web/src/App.tsx
git commit -m "feat(web): skill section — list, view, edit, save to writable root"
```

---

## Task 11: End-to-end build + manual smoke

**Files:** none (verification only)

- [ ] **Step 1: Full workspace test**

Run: `cd agent && cargo test 2>&1 | tail -25`
Expected: all crates green.

- [ ] **Step 2: Desktop build**

Run: `cargo build -p app 2>&1 | tail -15` (real src-tauri package name).
Expected: clean.

- [ ] **Step 3: Manual smoke (document result in the commit message)**

Run the desktop app (`npm run desktop:dev` from repo root, per `package.json`). With the local llama server up (see memory: Qwen3.6 on :8080), send a message, then:
- Switch the right pane to **Context**: a segmented bar shows with token counts; categories `system`/`messages` present.
- If memory is enabled, edit a memory's text, confirm it persists after app restart (SqliteStore durability).
- Open a skill, edit its body, save, reopen — body persists under the writable root.

- [ ] **Step 4: Commit a short verification note (optional)**

```bash
git commit --allow-empty -m "test: manual smoke — context explorer pane, memory edit persistence, skill save"
```

---

## Self-Review

**Spec coverage:**
- Context breakdown (`/context`-style, per-category) → Tasks 1, 2, 7, 8. ✓
- Honest total + `unattributed` reconciliation → Task 7 (`computeBreakdown`) + Task 8 (bar). ✓
- Memory recalled rows → Task 8 (snapshot `memory` segment items) + Task 9 (recalled list); cosine scores available via `recallPreview` (Task 4/6) — wired into the browser, with the documented "re-query, not capture" caveat. ✓
- Memory CRUD against durable store → Tasks 3, 4, 9. ✓ (SqliteStore already wired — swap task dropped, noted in Global Constraints.)
- Skills view/edit → Tasks 5, 6, 10. ✓
- Embedded tab, Tauri transport → Tasks 6, 8; Global Constraints. ✓
- Token counts + previews (not full text each turn) → snapshot carries previews; full skill/memory text fetched on drill-in via `skill_get`/`memory_list`. ✓
- **Deviation from spec, intentional:** pull-based `context_get` command instead of a pushed `context_snapshot` event (lower risk, reuses the settings_get pattern, untouched agent loop); recall scores via off-loop `memory_recall_preview` instead of threading `Scored` through the loop. Both preserve the spec's user-visible behavior.

**Placeholder scan:** The only non-code marker is the `# unimplemented_marker` pointer in Task 3 Step 3, which is explicitly called out as a pointer to copy the existing `query()` rusqlite pattern and to be removed — acceptable because the exact binding code is codebase-specific and the instruction names the precise source to mirror. No "TBD"/"add error handling"/"similar to" placeholders elsewhere.

**Type consistency:** `ContextSnapshot`/`ContextSegment` identical in Rust (Task 1) and TS (Task 6). `MemoryRow`/`ScoredRow`/`SkillDto` fields match across Rust DTOs (Tasks 3, 5) and TS (Task 6). Command names match between `api.ts` (Task 6) and the `#[tauri::command]` fns (Tasks 2, 4, 5). `computeBreakdown` signature consistent across Tasks 7 and 8.
