# Vector / Long-Term Memory System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the agent persistent, semantically-searchable long-term memory via explicit `remember` / `recall` / `forget` tools, backed by an in-process embedder and a single local SQLite store, wired into both binaries — with zero changes to `agent-core`.

**Architecture:** A new `agent-memory` crate exposes two trait seams — `Embedder` (in-process ONNX via fastembed behind a default `onnx` feature; deterministic `StubEmbedder` for tests) and `MemoryStore` (`SqliteStore` + `InMemoryStore`) — plus three `Tool` impls. Tools are constructed once at startup (the embedder loads a model), then injected into the `ToolRegistry` the same way MCP tools already are, so they survive daemon settings-reconfigure.

**Tech Stack:** Rust (edition 2021), `rusqlite` (bundled), `fastembed` (optional, feature `onnx`), `sha2`, `uuid`, `async-trait`, `serde_json`, `tokio`, `tracing`. Spec: [`../specs/2026-06-23-memory-system-design.md`](../specs/2026-06-23-memory-system-design.md).

## Global Constraints

- **cargo is not on PATH:** run `source "$HOME/.cargo/env"` before any cargo command.
- **Build/test from `agent/`:** gates are `cargo test --workspace` and `cargo clippy --all-targets -- -D warnings` (must be clean).
- **Workspace deps:** use `{ workspace = true }` for any dep already in `agent/Cargo.toml [workspace.dependencies]` (tokio, async-trait, serde, serde_json, tracing, thiserror, tempfile).
- **Zero `agent-core` changes.** Attach only via the existing `agent_tools::Tool` seam + binary wiring in `agent-runtime-config`.
- **Test embedder discipline:** every unit/integration test uses `StubEmbedder` (deterministic). The real `FastEmbedEmbedder` is exercised *only* by an `#[ignore]`-gated live test. `agent-memory` ships fastembed behind a **default** feature `onnx`; the test suite never needs the model.
- **Memory is best-effort, never fatal:** a tool error returns `ToolError` and the loop continues; a failed memory *construction* (e.g. model unavailable offline) disables memory (registers no tools), never aborts the binary.
- **Intent / approval:** memory tools declare `Access::Read` with empty `paths` and no `command`, so `RulePolicy::check` auto-`Allow`s them (memory ops touch only the private local store; approval-gating is deferred per spec §1). The `summary` string is truthful about writes for audit/logging. Flipping to approval-gated later is a one-line change to `Access::Write`.
- **Tool names are exactly** `remember`, `recall`, `forget`.
- **DB:** single file at `~/.agent/memory.db`, mode `0600`, with `scope_kind`/`scope_key` columns. Defaults from spec §6 live in `MemoryConfig`.

---

## File Structure

```
agent/crates/agent-memory/
  Cargo.toml          new crate manifest (rusqlite, sha2, uuid, fastembed[onnx])
  src/lib.rs          re-exports; build_tools / build_tools_with; MemoryInitError
  src/record.rs       MemoryRecord, MemoryScope, Scored, ScopeFilter, now_secs
  src/scope.rs        project_scope(workspace) — git-toplevel/canonical → sha256 key
  src/embedder.rs     Embedder trait, EmbedError, StubEmbedder, cosine; FastEmbedEmbedder (feature onnx)
  src/store.rs        MemoryStore trait, StoreError, InMemoryStore, SqliteStore
  src/config.rs       MemoryConfig (paths, model, k, thresholds, caps)
  src/tools.rs        Remember, Recall, Forget (impl Tool)
  tests/live_embed.rs #[ignore] real-model paraphrase DoD
```

Modified for wiring:
- `agent/crates/agent-runtime-config/Cargo.toml` — add `agent-memory` dep.
- `agent/crates/agent-runtime-config/src/lib.rs` — add `build_memory(...)`.
- `agent/crates/agent-cli/src/main.rs` — `--memory*` flags + register.
- `agent/crates/agent-server/src/main.rs`, `src/runtime.rs`, `src/daemon.rs` — `--memory*` flags + inject `memory_tools` slice into `build_loop`.

---

## Task 1: Scaffold crate + scope/record types

**Files:**
- Create: `agent/crates/agent-memory/Cargo.toml`
- Create: `agent/crates/agent-memory/src/lib.rs`
- Create: `agent/crates/agent-memory/src/record.rs`
- Create: `agent/crates/agent-memory/src/scope.rs`

**Interfaces:**
- Produces: `MemoryScope::{Project(String), Global}`; `ScopeFilter::{Exact(MemoryScope), ProjectAndGlobal{project_key:String}}`; `MemoryRecord { id, text, scope, tags, vector, created_at, updated_at, source }`; `Scored { record: MemoryRecord, score: f32 }`; `now_secs() -> i64`; `project_scope(workspace: &Path) -> MemoryScope`.

- [ ] **Step 1: Create the crate manifest**

`agent/crates/agent-memory/Cargo.toml`:
```toml
[package]
name = "agent-memory"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
agent-tools = { path = "../agent-tools" }
async-trait = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
thiserror = { workspace = true }
rusqlite = { version = "0.31", features = ["bundled"] }
sha2 = "0.10"
uuid = { version = "1", features = ["v4"] }
fastembed = { version = "4", optional = true }

[features]
default = ["onnx"]
onnx = ["dep:fastembed"]

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 2: Write the failing test for record/scope types**

`agent/crates/agent-memory/src/record.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MemoryScope {
    Project(String), // sha256 hex of the canonical project root
    Global,
}

impl MemoryScope {
    pub fn kind(&self) -> &'static str {
        match self { MemoryScope::Project(_) => "project", MemoryScope::Global => "global" }
    }
    pub fn key(&self) -> &str {
        match self { MemoryScope::Project(k) => k, MemoryScope::Global => "" }
    }
}

/// How a query selects rows. `Exact` is used for dedup (same scope only);
/// `ProjectAndGlobal` is used for recall (current project + the global tier).
#[derive(Debug, Clone)]
pub enum ScopeFilter {
    Exact(MemoryScope),
    ProjectAndGlobal { project_key: String },
}

impl ScopeFilter {
    /// Does a stored record's scope satisfy this filter?
    pub fn matches(&self, scope: &MemoryScope) -> bool {
        match self {
            ScopeFilter::Exact(s) => s == scope,
            ScopeFilter::ProjectAndGlobal { project_key } => match scope {
                MemoryScope::Global => true,
                MemoryScope::Project(k) => k == project_key,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryRecord {
    pub id: String,
    pub text: String,
    pub scope: MemoryScope,
    pub tags: Vec<String>,
    pub vector: Vec<f32>,
    pub created_at: i64,
    pub updated_at: i64,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct Scored {
    pub record: MemoryRecord,
    pub score: f32,
}

pub fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_and_global_filter_admits_global_and_matching_project_only() {
        let f = ScopeFilter::ProjectAndGlobal { project_key: "abc".into() };
        assert!(f.matches(&MemoryScope::Global));
        assert!(f.matches(&MemoryScope::Project("abc".into())));
        assert!(!f.matches(&MemoryScope::Project("other".into())));
    }

    #[test]
    fn exact_filter_admits_only_same_scope() {
        let f = ScopeFilter::Exact(MemoryScope::Project("abc".into()));
        assert!(f.matches(&MemoryScope::Project("abc".into())));
        assert!(!f.matches(&MemoryScope::Global));
    }
}
```

- [ ] **Step 3: Create `scope.rs` (project-key derivation)**

`agent/crates/agent-memory/src/scope.rs`:
```rust
use crate::record::MemoryScope;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

/// Derive the project scope for a workspace: prefer the git top-level (stable across
/// subdirs), else the canonicalized workspace root; hash the path so raw filesystem
/// paths are never stored.
pub fn project_scope(workspace: &Path) -> MemoryScope {
    let canonical = workspace.canonicalize().unwrap_or_else(|_| workspace.to_path_buf());
    let root = git_toplevel(&canonical).unwrap_or(canonical);
    let mut h = Sha256::new();
    h.update(root.to_string_lossy().as_bytes());
    MemoryScope::Project(format!("{:x}", h.finalize()))
}

fn git_toplevel(dir: &Path) -> Option<std::path::PathBuf> {
    let out = Command::new("git")
        .arg("-C").arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output().ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(std::path::PathBuf::from(s)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn same_key_from_repo_root_and_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert!(Command::new("git").arg("-C").arg(root).arg("init").output().unwrap().status.success());
        let sub = root.join("crates/inner");
        std::fs::create_dir_all(&sub).unwrap();
        let a = project_scope(root);
        let b = project_scope(&sub);
        assert_eq!(a, b, "subdir must map to the same project scope as the repo root");
        assert!(matches!(a, MemoryScope::Project(ref k) if k.len() == 64));
    }

    #[test]
    fn non_git_dir_uses_canonical_path() {
        let tmp = tempfile::tempdir().unwrap();
        let s = project_scope(tmp.path());
        assert!(matches!(s, MemoryScope::Project(_)));
    }
}
```

- [ ] **Step 4: Create `lib.rs` wiring the modules**

`agent/crates/agent-memory/src/lib.rs`:
```rust
//! Long-term semantic memory: remember/recall/forget tools over a local vector store.
mod record;
mod scope;

pub use record::{now_secs, MemoryRecord, MemoryScope, ScopeFilter, Scored};
pub use scope::project_scope;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-memory`
Expected: PASS (4 tests across record + scope).

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-memory
git commit -m "feat(memory): scaffold agent-memory crate with scope + record types"
```

---

## Task 2: Embedder trait + StubEmbedder + cosine

**Files:**
- Create: `agent/crates/agent-memory/src/embedder.rs`
- Modify: `agent/crates/agent-memory/src/lib.rs` (add `mod embedder; pub use ...`)

**Interfaces:**
- Consumes: nothing.
- Produces: `Embedder` trait (`async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>`, `fn dim(&self) -> usize`); `EmbedError`; `StubEmbedder::new(dim)` / `StubEmbedder::d384()`; `cosine(a, b) -> f32`.

- [ ] **Step 1: Write the failing test**

`agent/crates/agent-memory/src/embedder.rs`:
```rust
use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("embedding failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
    fn dim(&self) -> usize;
}

/// Cosine similarity. Returns NaN on a dimension mismatch (caller treats NaN as "skip"),
/// and 0.0 when either vector has zero magnitude.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::NAN;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Deterministic, dependency-free embedder for tests: identical text → identical vector
/// (cosine 1.0), distinct text → near-orthogonal vectors. NOT semantic — paraphrase
/// matching is only validated by the live `#[ignore]` test against the real model.
pub struct StubEmbedder {
    dim: usize,
}

impl StubEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
    pub fn d384() -> Self {
        Self { dim: 384 }
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[async_trait]
impl Embedder for StubEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0f32; self.dim];
                for (i, slot) in v.iter_mut().enumerate() {
                    let h = fnv1a(format!("{i}:{t}").as_bytes());
                    *slot = ((h % 2000) as f32 / 1000.0) - 1.0; // [-1, 1)
                }
                let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if n > 0.0 {
                    for x in &mut v {
                        *x /= n;
                    }
                }
                v
            })
            .collect())
    }
    fn dim(&self) -> usize {
        self.dim
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn identical_text_is_cosine_one_distinct_is_low() {
        let e = StubEmbedder::d384();
        let v = e.embed(&["alpha".into(), "alpha".into(), "totally different".into()]).await.unwrap();
        assert_eq!(v[0].len(), 384);
        assert!((cosine(&v[0], &v[1]) - 1.0).abs() < 1e-5, "same text → 1.0");
        assert!(cosine(&v[0], &v[2]) < 0.5, "distinct text → low similarity");
    }

    #[test]
    fn cosine_dimension_mismatch_is_nan() {
        assert!(cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]).is_nan());
    }
}
```

- [ ] **Step 2: Add modules to `lib.rs`**

In `agent/crates/agent-memory/src/lib.rs`, after `mod scope;` add:
```rust
mod embedder;
```
and extend the re-exports:
```rust
pub use embedder::{cosine, EmbedError, Embedder, StubEmbedder};
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-memory embedder`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add agent/crates/agent-memory/src
git commit -m "feat(memory): Embedder trait, deterministic StubEmbedder, cosine"
```

---

## Task 3: MemoryStore trait + InMemoryStore

**Files:**
- Create: `agent/crates/agent-memory/src/store.rs`
- Modify: `agent/crates/agent-memory/src/lib.rs`

**Interfaces:**
- Consumes: `MemoryRecord`, `MemoryScope`, `ScopeFilter`, `Scored`, `cosine`.
- Produces: `MemoryStore` trait + `StoreError` + `InMemoryStore::new()`.
  - `async fn upsert(&self, rec: MemoryRecord) -> Result<(), StoreError>`
  - `async fn query(&self, vector: &[f32], k: usize, filter: &ScopeFilter) -> Result<Vec<Scored>, StoreError>`
  - `async fn get(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError>`
  - `async fn delete(&self, id: &str) -> Result<bool, StoreError>`
  - `async fn count(&self, filter: &ScopeFilter) -> Result<usize, StoreError>`
  - `async fn evict_oldest(&self, scope: &MemoryScope) -> Result<Option<String>, StoreError>` (delete + return the least-recently-updated id in that exact scope)

- [ ] **Step 1: Write the failing test**

`agent/crates/agent-memory/src/store.rs`:
```rust
use crate::embedder::cosine;
use crate::record::{MemoryRecord, MemoryScope, ScopeFilter, Scored};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("store i/o: {0}")]
    Io(String),
}

#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn upsert(&self, rec: MemoryRecord) -> Result<(), StoreError>;
    async fn query(&self, vector: &[f32], k: usize, filter: &ScopeFilter)
        -> Result<Vec<Scored>, StoreError>;
    async fn get(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError>;
    async fn delete(&self, id: &str) -> Result<bool, StoreError>;
    async fn count(&self, filter: &ScopeFilter) -> Result<usize, StoreError>;
    async fn evict_oldest(&self, scope: &MemoryScope) -> Result<Option<String>, StoreError>;
}

/// Score a candidate set against a query vector, skipping dimension-mismatched rows
/// (NaN cosine) with a one-time-ish warning, sorted best-first, truncated to `k`.
pub(crate) fn rank(rows: Vec<MemoryRecord>, vector: &[f32], k: usize) -> Vec<Scored> {
    let mut scored: Vec<Scored> = rows
        .into_iter()
        .filter_map(|r| {
            let s = cosine(vector, &r.vector);
            if s.is_nan() {
                tracing::warn!(target: "memory", id = %r.id, "skipping row with mismatched embedding dimension");
                None
            } else {
                Some(Scored { record: r, score: s })
            }
        })
        .collect();
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored
}

#[derive(Default)]
pub struct InMemoryStore {
    rows: Mutex<HashMap<String, MemoryRecord>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MemoryStore for InMemoryStore {
    async fn upsert(&self, rec: MemoryRecord) -> Result<(), StoreError> {
        self.rows.lock().unwrap().insert(rec.id.clone(), rec);
        Ok(())
    }
    async fn query(&self, vector: &[f32], k: usize, filter: &ScopeFilter)
        -> Result<Vec<Scored>, StoreError> {
        let rows: Vec<MemoryRecord> = self.rows.lock().unwrap().values()
            .filter(|r| filter.matches(&r.scope)).cloned().collect();
        Ok(rank(rows, vector, k))
    }
    async fn get(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError> {
        Ok(self.rows.lock().unwrap().get(id).cloned())
    }
    async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        Ok(self.rows.lock().unwrap().remove(id).is_some())
    }
    async fn count(&self, filter: &ScopeFilter) -> Result<usize, StoreError> {
        Ok(self.rows.lock().unwrap().values().filter(|r| filter.matches(&r.scope)).count())
    }
    async fn evict_oldest(&self, scope: &MemoryScope) -> Result<Option<String>, StoreError> {
        let mut g = self.rows.lock().unwrap();
        let oldest = g.values().filter(|r| &r.scope == scope)
            .min_by_key(|r| r.updated_at).map(|r| r.id.clone());
        if let Some(id) = &oldest {
            g.remove(id);
        }
        Ok(oldest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::now_secs;

    fn rec(id: &str, scope: MemoryScope, vector: Vec<f32>, updated: i64) -> MemoryRecord {
        MemoryRecord { id: id.into(), text: id.into(), scope, tags: vec![], vector,
                       created_at: updated, updated_at: updated, source: "test".into() }
    }

    #[tokio::test]
    async fn query_respects_scope_and_orders_by_similarity() {
        let s = InMemoryStore::new();
        s.upsert(rec("p1", MemoryScope::Project("A".into()), vec![1.0, 0.0], 1)).await.unwrap();
        s.upsert(rec("g1", MemoryScope::Global, vec![0.0, 1.0], 2)).await.unwrap();
        s.upsert(rec("p2", MemoryScope::Project("B".into()), vec![1.0, 0.0], 3)).await.unwrap();

        let hits = s.query(&[1.0, 0.0], 10,
            &ScopeFilter::ProjectAndGlobal { project_key: "A".into() }).await.unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.record.id.as_str()).collect();
        assert!(ids.contains(&"p1") && ids.contains(&"g1"), "project A + global visible");
        assert!(!ids.contains(&"p2"), "project B hidden");
        assert_eq!(hits[0].record.id, "p1", "best match first");
    }

    #[tokio::test]
    async fn evict_oldest_removes_least_recently_updated_in_scope() {
        let s = InMemoryStore::new();
        let sc = MemoryScope::Project("A".into());
        s.upsert(rec("old", sc.clone(), vec![1.0, 0.0], 1)).await.unwrap();
        s.upsert(rec("new", sc.clone(), vec![1.0, 0.0], now_secs())).await.unwrap();
        assert_eq!(s.evict_oldest(&sc).await.unwrap().as_deref(), Some("old"));
        assert!(s.get("old").await.unwrap().is_none());
        assert!(s.get("new").await.unwrap().is_some());
    }
}
```

- [ ] **Step 2: Add module + exports to `lib.rs`**

Add `mod store;` and:
```rust
pub use store::{InMemoryStore, MemoryStore, StoreError};
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-memory store`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add agent/crates/agent-memory/src
git commit -m "feat(memory): MemoryStore trait + InMemoryStore with scope-filtered ranking"
```

---

## Task 4: SqliteStore (persistence + parity)

**Files:**
- Modify: `agent/crates/agent-memory/src/store.rs`
- Modify: `agent/crates/agent-memory/src/lib.rs`

**Interfaces:**
- Consumes: `MemoryStore` trait + `rank`.
- Produces: `SqliteStore::open(path: &Path) -> Result<SqliteStore, StoreError>` (creates parent dir + schema; sets `0600` on Unix; busy_timeout; WAL).

- [ ] **Step 1: Write the failing test**

Append to `agent/crates/agent-memory/src/store.rs` (new test module section + impl). Tests first:
```rust
#[cfg(test)]
mod sqlite_tests {
    use super::*;
    use crate::record::now_secs;

    fn rec(id: &str, scope: MemoryScope, vector: Vec<f32>) -> MemoryRecord {
        MemoryRecord { id: id.into(), text: format!("text-{id}"), scope, tags: vec!["t".into()],
                       vector, created_at: now_secs(), updated_at: now_secs(), source: "test".into() }
    }

    #[tokio::test]
    async fn persists_across_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("memory.db");
        {
            let s = SqliteStore::open(&path).unwrap();
            s.upsert(rec("a", MemoryScope::Global, vec![1.0, 0.0, 0.0])).await.unwrap();
        }
        // Fresh process simulation: reopen the same file.
        let s2 = SqliteStore::open(&path).unwrap();
        let got = s2.get("a").await.unwrap().expect("row survives reopen");
        assert_eq!(got.text, "text-a");
        assert_eq!(got.vector, vec![1.0, 0.0, 0.0]);
        assert_eq!(got.tags, vec!["t".to_string()]);
    }

    #[tokio::test]
    async fn query_scopes_and_dimension_mismatch_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let s = SqliteStore::open(&tmp.path().join("m.db")).unwrap();
        s.upsert(rec("p", MemoryScope::Project("A".into()), vec![1.0, 0.0])).await.unwrap();
        s.upsert(rec("g", MemoryScope::Global, vec![1.0, 0.0])).await.unwrap();
        // A stale 3-dim row from an old model: must be skipped, not panic.
        s.upsert(rec("stale", MemoryScope::Global, vec![1.0, 0.0, 0.0])).await.unwrap();

        let hits = s.query(&[1.0, 0.0], 10,
            &ScopeFilter::ProjectAndGlobal { project_key: "A".into() }).await.unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.record.id.as_str()).collect();
        assert!(ids.contains(&"p") && ids.contains(&"g"));
        assert!(!ids.contains(&"stale"), "mismatched-dim row skipped");
    }

    #[tokio::test]
    async fn count_delete_evict_oldest() {
        let tmp = tempfile::tempdir().unwrap();
        let s = SqliteStore::open(&tmp.path().join("m.db")).unwrap();
        let sc = MemoryScope::Project("A".into());
        let mut old = rec("old", sc.clone(), vec![1.0, 0.0]); old.updated_at = 1;
        s.upsert(old).await.unwrap();
        s.upsert(rec("new", sc.clone(), vec![1.0, 0.0])).await.unwrap();
        assert_eq!(s.count(&ScopeFilter::Exact(sc.clone())).await.unwrap(), 2);
        assert_eq!(s.evict_oldest(&sc).await.unwrap().as_deref(), Some("old"));
        assert!(s.delete("new").await.unwrap());
        assert_eq!(s.count(&ScopeFilter::Exact(sc)).await.unwrap(), 0);
    }
}
```

- [ ] **Step 2: Run the test to confirm it fails**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-memory sqlite_tests`
Expected: FAIL (`SqliteStore` not found).

- [ ] **Step 3: Implement `SqliteStore`**

Append to `agent/crates/agent-memory/src/store.rs`:
```rust
use crate::record::ScopeFilter as _ScopeFilterAlias; // (no-op alias to keep imports explicit)
use rusqlite::Connection;
use std::path::Path;
use std::time::Duration;

/// f32 vector ↔ little-endian BLOB.
fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}
fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let conn = Connection::open(path).map_err(|e| StoreError::Io(e.to_string()))?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(|e| StoreError::Io(e.to_string()))?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| StoreError::Io(e.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                scope_kind TEXT NOT NULL,
                scope_key TEXT NOT NULL,
                text TEXT NOT NULL,
                tags TEXT NOT NULL,
                vector BLOB NOT NULL,
                dim INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                source TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_scope ON memories(scope_kind, scope_key);",
        ).map_err(|e| StoreError::Io(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<MemoryRecord> {
        let kind: String = row.get("scope_kind")?;
        let key: String = row.get("scope_key")?;
        let scope = if kind == "global" { MemoryScope::Global } else { MemoryScope::Project(key) };
        let tags_json: String = row.get("tags")?;
        let blob: Vec<u8> = row.get("vector")?;
        Ok(MemoryRecord {
            id: row.get("id")?,
            text: row.get("text")?,
            scope,
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            vector: blob_to_vec(&blob),
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            source: row.get("source")?,
        })
    }
}

/// Build the WHERE clause + params for a ScopeFilter.
fn scope_where(filter: &ScopeFilter) -> (String, Vec<String>) {
    match filter {
        ScopeFilter::Exact(scope) => (
            "scope_kind = ?1 AND scope_key = ?2".into(),
            vec![scope.kind().into(), scope.key().into()],
        ),
        ScopeFilter::ProjectAndGlobal { project_key } => (
            "scope_kind = 'global' OR (scope_kind = 'project' AND scope_key = ?1)".into(),
            vec![project_key.clone()],
        ),
    }
}

#[async_trait]
impl MemoryStore for SqliteStore {
    async fn upsert(&self, rec: MemoryRecord) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO memories (id,scope_kind,scope_key,text,tags,vector,dim,created_at,updated_at,source)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(id) DO UPDATE SET
                scope_kind=?2, scope_key=?3, text=?4, tags=?5, vector=?6, dim=?7, updated_at=?9, source=?10",
            rusqlite::params![
                rec.id, rec.scope.kind(), rec.scope.key(), rec.text,
                serde_json::to_string(&rec.tags).unwrap_or_else(|_| "[]".into()),
                vec_to_blob(&rec.vector), rec.vector.len() as i64,
                rec.created_at, rec.updated_at, rec.source,
            ],
        ).map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }

    async fn query(&self, vector: &[f32], k: usize, filter: &ScopeFilter)
        -> Result<Vec<Scored>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let (clause, params) = scope_where(filter);
        let sql = format!("SELECT * FROM memories WHERE {clause}");
        let mut stmt = conn.prepare(&sql).map_err(|e| StoreError::Io(e.to_string()))?;
        let pref: Vec<&dyn rusqlite::ToSql> = params.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(pref.as_slice(), Self::row_to_record)
            .map_err(|e| StoreError::Io(e.to_string()))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(rank(rows, vector, k))
    }

    async fn get(&self, id: &str) -> Result<Option<MemoryRecord>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM memories WHERE id = ?1")
            .map_err(|e| StoreError::Io(e.to_string()))?;
        let mut rows = stmt.query_map([id], Self::row_to_record)
            .map_err(|e| StoreError::Io(e.to_string()))?;
        match rows.next() {
            Some(r) => Ok(Some(r.map_err(|e| StoreError::Io(e.to_string()))?)),
            None => Ok(None),
        }
    }

    async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM memories WHERE id = ?1", [id])
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(n > 0)
    }

    async fn count(&self, filter: &ScopeFilter) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        let (clause, params) = scope_where(filter);
        let sql = format!("SELECT COUNT(*) FROM memories WHERE {clause}");
        let pref: Vec<&dyn rusqlite::ToSql> = params.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let n: i64 = conn.query_row(&sql, pref.as_slice(), |r| r.get(0))
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(n as usize)
    }

    async fn evict_oldest(&self, scope: &MemoryScope) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let id: Option<String> = conn.query_row(
            "SELECT id FROM memories WHERE scope_kind=?1 AND scope_key=?2 ORDER BY updated_at ASC LIMIT 1",
            rusqlite::params![scope.kind(), scope.key()],
            |r| r.get(0),
        ).ok();
        if let Some(id) = &id {
            conn.execute("DELETE FROM memories WHERE id = ?1", [id])
                .map_err(|e| StoreError::Io(e.to_string()))?;
        }
        Ok(id)
    }
}
```

Then delete the no-op alias line (`use crate::record::ScopeFilter as _ScopeFilterAlias;`) — it was only a placeholder reminder; `ScopeFilter` is already in scope via the top-of-file `use`. Add `pub use store::SqliteStore;` to `lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-memory`
Expected: PASS (record + embedder + store + sqlite_tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-memory/src
git commit -m "feat(memory): SqliteStore (BLOB vectors, scope SQL, persistence, dim-skip)"
```

---

## Task 5: MemoryConfig (defaults + limits)

**Files:**
- Create: `agent/crates/agent-memory/src/config.rs`
- Modify: `agent/crates/agent-memory/src/lib.rs`

**Interfaces:**
- Produces: `MemoryConfig` with all spec-§6 defaults; `MemoryConfig::default()`; helper `default_db_path() -> PathBuf` (`~/.agent/memory.db`).

- [ ] **Step 1: Write the failing test + implementation**

`agent/crates/agent-memory/src/config.rs`:
```rust
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub db_path: PathBuf,
    pub model_cache_dir: Option<PathBuf>,
    pub default_k: usize,
    pub max_k: usize,
    pub relevance_threshold: f32,
    pub dedup_threshold: f32,
    pub forget_threshold: f32,
    pub max_text_len: usize,
    pub max_tags: usize,
    pub max_tag_len: usize,
    pub max_memories_per_scope: usize,
    pub max_recall_chars: usize,
    pub candidate_warn_threshold: usize,
}

pub fn default_db_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".agent").join("memory.db")
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            model_cache_dir: None,
            default_k: 5,
            max_k: 20,
            relevance_threshold: 0.3,
            dedup_threshold: 0.95,
            forget_threshold: 0.85,
            max_text_len: 8 * 1024,
            max_tags: 16,
            max_tag_len: 64,
            max_memories_per_scope: 10_000,
            max_recall_chars: 4 * 1024,
            candidate_warn_threshold: 50_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec_table() {
        let c = MemoryConfig::default();
        assert_eq!(c.default_k, 5);
        assert_eq!(c.max_k, 20);
        assert_eq!(c.max_text_len, 8 * 1024);
        assert_eq!(c.max_memories_per_scope, 10_000);
        assert_eq!(c.max_recall_chars, 4 * 1024);
        assert!((c.dedup_threshold - 0.95).abs() < 1e-6);
        assert!((c.relevance_threshold - 0.3).abs() < 1e-6);
        assert!((c.forget_threshold - 0.85).abs() < 1e-6);
    }
}
```

Add `mod config;` and `pub use config::{default_db_path, MemoryConfig};` to `lib.rs`.

- [ ] **Step 2: Run tests**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-memory config`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add agent/crates/agent-memory/src
git commit -m "feat(memory): MemoryConfig with spec-default limits/thresholds"
```

---

## Task 6: `remember` tool

**Files:**
- Create: `agent/crates/agent-memory/src/tools.rs`
- Modify: `agent/crates/agent-memory/src/lib.rs`

**Interfaces:**
- Consumes: `Embedder`, `MemoryStore`, `MemoryConfig`, `MemoryScope`, `MemoryRecord`, `ScopeFilter`, `now_secs`.
- Produces: `Remember { embedder: Arc<dyn Embedder>, store: Arc<dyn MemoryStore>, cfg: Arc<MemoryConfig>, project_key: String }`; shared helper `parse_scope(args, project_key) -> MemoryScope` and `parse_tags(args, &cfg) -> Vec<String>` reused by Recall/Forget.

- [ ] **Step 1: Write the failing test**

Create `agent/crates/agent-memory/src/tools.rs` with the shared scaffolding + Remember + tests:
```rust
use crate::config::MemoryConfig;
use crate::embedder::{cosine, Embedder};
use crate::record::{now_secs, MemoryRecord, MemoryScope, ScopeFilter};
use crate::store::MemoryStore;
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Memory ops touch only the private local store, so they declare a Read-access,
/// path-less, command-less intent → `RulePolicy` auto-allows them. Approval-gating
/// memory writes is deferred per spec §1; `summary` stays truthful for the audit log.
fn read_intent(tool: &str, summary: String) -> ToolIntent {
    ToolIntent { tool: tool.into(), access: Access::Read, paths: vec![], command: None, summary }
}

pub(crate) fn parse_scope(args: &Value, project_key: &str) -> MemoryScope {
    match args.get("scope").and_then(Value::as_str) {
        Some("global") => MemoryScope::Global,
        _ => MemoryScope::Project(project_key.to_string()),
    }
}

pub(crate) fn parse_tags(args: &Value, cfg: &MemoryConfig) -> Vec<String> {
    args.get("tags").and_then(Value::as_array).map(|a| {
        a.iter().filter_map(Value::as_str)
            .map(|s| s.chars().take(cfg.max_tag_len).collect::<String>())
            .take(cfg.max_tags).collect()
    }).unwrap_or_default()
}

fn embed_failed(e: impl std::fmt::Display) -> ToolError {
    ToolError::Failed { message: format!("embedding failed: {e}"), stderr: None }
}
fn store_failed(e: impl std::fmt::Display) -> ToolError {
    ToolError::Failed { message: format!("memory store error: {e}"), stderr: None }
}

pub struct Remember {
    pub embedder: Arc<dyn Embedder>,
    pub store: Arc<dyn MemoryStore>,
    pub cfg: Arc<MemoryConfig>,
    pub project_key: String,
}

#[async_trait]
impl Tool for Remember {
    fn name(&self) -> &str { "remember" }
    fn description(&self) -> &str {
        "Store a fact in long-term memory for recall in future sessions. \
         Args: text (required), tags (optional string array), scope ('project'|'global', default project)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "remember".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "The fact to remember"},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "scope": {"type": "string", "enum": ["project", "global"]}
                },
                "required": ["text"]
            }),
        }
    }
    fn intent(&self, _args: &Value) -> Result<ToolIntent, ToolError> {
        Ok(read_intent("remember", "write to long-term memory store".into()))
    }
    async fn execute(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let text = args.get("text").and_then(Value::as_str)
            .map(str::trim).filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::InvalidArgs("missing non-empty 'text'".into()))?;
        if text.len() > self.cfg.max_text_len {
            return Err(ToolError::InvalidArgs(format!(
                "text too long ({} bytes; max {})", text.len(), self.cfg.max_text_len)));
        }
        let scope = parse_scope(&args, &self.project_key);
        let tags = parse_tags(&args, &self.cfg);
        let vector = self.embedder.embed(&[text.to_string()]).await
            .map_err(embed_failed)?.into_iter().next().unwrap();

        // Dedup: supersede a near-identical memory in the same scope instead of duplicating.
        let near = self.store.query(&vector, 1, &ScopeFilter::Exact(scope.clone()))
            .await.map_err(store_failed)?;
        if let Some(top) = near.first() {
            if top.score >= self.cfg.dedup_threshold {
                let mut rec = top.record.clone();
                rec.text = text.to_string();
                rec.tags = tags;
                rec.vector = vector;
                rec.updated_at = now_secs();
                let id = rec.id.clone();
                self.store.upsert(rec).await.map_err(store_failed)?;
                tracing::info!(target: "memory", %id, scope = scope.kind(), "remember: superseded");
                return Ok(ToolOutput { content: format!("Updated existing memory {id}."), display: None });
            }
        }

        // Cap: evict least-recently-updated while at the per-scope ceiling.
        while self.store.count(&ScopeFilter::Exact(scope.clone())).await.map_err(store_failed)?
            >= self.cfg.max_memories_per_scope {
            if let Some(ev) = self.store.evict_oldest(&scope).await.map_err(store_failed)? {
                tracing::warn!(target: "memory", evicted = %ev, "remember: scope cap reached, evicted oldest");
            } else { break; }
        }

        let now = now_secs();
        let id = Uuid::new_v4().to_string();
        let rec = MemoryRecord { id: id.clone(), text: text.to_string(), scope: scope.clone(),
            tags, vector, created_at: now, updated_at: now, source: "remember".into() };
        self.store.upsert(rec).await.map_err(store_failed)?;
        tracing::info!(target: "memory", %id, scope = scope.kind(), "remember: stored new");
        Ok(ToolOutput { content: format!("Stored memory {id}."), display: None })
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::embedder::StubEmbedder;
    use crate::store::InMemoryStore;

    pub fn remember(project_key: &str) -> (Remember, Arc<dyn MemoryStore>, Arc<dyn Embedder>, Arc<MemoryConfig>) {
        let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let cfg = Arc::new(MemoryConfig::default());
        let r = Remember { embedder: embedder.clone(), store: store.clone(), cfg: cfg.clone(),
            project_key: project_key.into() };
        (r, store, embedder, cfg)
    }

    pub fn ctx() -> ToolCtx {
        ToolCtx {
            workspace: std::path::PathBuf::from("/tmp"),
            timeout: std::time::Duration::from_secs(5),
            cancel: tokio_util::sync::CancellationToken::new(),
            sandbox: Arc::new(agent_tools::HostExecutor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::record::ScopeFilter;

    #[tokio::test]
    async fn stores_new_then_supersedes_identical() {
        let (r, store, _e, _c) = remember("A");
        r.execute(json!({"text": "the build uses cargo"}), &ctx()).await.unwrap();
        // Identical text → cosine 1.0 ≥ dedup_threshold → supersede, not duplicate.
        r.execute(json!({"text": "the build uses cargo"}), &ctx()).await.unwrap();
        let scope = MemoryScope::Project("A".into());
        assert_eq!(store.count(&ScopeFilter::Exact(scope)).await.unwrap(), 1, "deduped");
    }

    #[tokio::test]
    async fn distinct_text_inserts_separately() {
        let (r, store, _e, _c) = remember("A");
        r.execute(json!({"text": "fact one about networking"}), &ctx()).await.unwrap();
        r.execute(json!({"text": "an unrelated fact about cooking"}), &ctx()).await.unwrap();
        let scope = MemoryScope::Project("A".into());
        assert_eq!(store.count(&ScopeFilter::Exact(scope)).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn oversized_text_is_rejected() {
        let (r, _s, _e, cfg) = remember("A");
        let big = "x".repeat(cfg.max_text_len + 1);
        let err = r.execute(json!({"text": big}), &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn empty_text_is_rejected() {
        let (r, _s, _e, _c) = remember("A");
        let err = r.execute(json!({"text": "   "}), &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }
}
```

- [ ] **Step 2: Run the test to confirm it fails, then passes**

Add `mod tools;` and `pub use tools::Remember;` to `lib.rs`. Then:
Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-memory tools`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add agent/crates/agent-memory/src
git commit -m "feat(memory): remember tool (dedup-supersede, cap eviction, validation)"
```

---

## Task 7: `recall` tool

**Files:**
- Modify: `agent/crates/agent-memory/src/tools.rs`
- Modify: `agent/crates/agent-memory/src/lib.rs`

**Interfaces:**
- Consumes: same shared helpers + `cosine` (not needed) and `render_age`.
- Produces: `Recall { embedder, store, cfg, project_key }`; private `fn render_hits(hits, max_chars) -> String`.

- [ ] **Step 1: Write the failing test**

Append to `tools.rs`:
```rust
fn render_age(updated_at: i64) -> String {
    let secs = (now_secs() - updated_at).max(0);
    if secs < 60 { "just now".into() }
    else if secs < 3600 { format!("{}m ago", secs / 60) }
    else if secs < 86400 { format!("{}h ago", secs / 3600) }
    else { format!("{}d ago", secs / 86400) }
}

pub struct Recall {
    pub embedder: Arc<dyn Embedder>,
    pub store: Arc<dyn MemoryStore>,
    pub cfg: Arc<MemoryConfig>,
    pub project_key: String,
}

#[async_trait]
impl Tool for Recall {
    fn name(&self) -> &str { "recall" }
    fn description(&self) -> &str {
        "Search long-term memory for facts relevant to a query. Returns the most similar \
         stored memories from this project and the global tier. Args: query (required), k (optional)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "recall".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "k": {"type": "integer", "minimum": 1}
                },
                "required": ["query"]
            }),
        }
    }
    fn intent(&self, _args: &Value) -> Result<ToolIntent, ToolError> {
        Ok(read_intent("recall", "search long-term memory".into()))
    }
    async fn execute(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let query = args.get("query").and_then(Value::as_str)
            .map(str::trim).filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::InvalidArgs("missing non-empty 'query'".into()))?;
        let k = args.get("k").and_then(Value::as_u64).map(|n| n as usize)
            .unwrap_or(self.cfg.default_k).clamp(1, self.cfg.max_k);
        let qv = self.embedder.embed(&[query.to_string()]).await
            .map_err(embed_failed)?.into_iter().next().unwrap();

        let filter = ScopeFilter::ProjectAndGlobal { project_key: self.project_key.clone() };
        let mut hits = self.store.query(&qv, self.cfg.max_k, &filter).await.map_err(store_failed)?;
        hits.retain(|h| h.score >= self.cfg.relevance_threshold);
        hits.truncate(k);
        tracing::info!(target: "memory", returned = hits.len(),
            top = hits.first().map(|h| h.score).unwrap_or(0.0), "recall");

        if hits.is_empty() {
            return Ok(ToolOutput { content: "No relevant memories found.".into(), display: None });
        }
        let body = render_hits(&hits, self.cfg.max_recall_chars);
        Ok(ToolOutput { content: body, display: None })
    }
}

fn render_hits(hits: &[crate::record::Scored], max_chars: usize) -> String {
    let mut out = String::new();
    for h in hits {
        let tags = if h.record.tags.is_empty() { String::new() }
                   else { format!("; tags: {}", h.record.tags.join(",")) };
        let line = format!("[{:.2}] {} ({}{})\n",
            h.score, h.record.text, render_age(h.record.updated_at), tags);
        if out.len() + line.len() > max_chars {
            out.push_str("[truncated: more memories matched]\n");
            break;
        }
        out.push_str(&line);
    }
    out
}
```

And the tests:
```rust
#[cfg(test)]
mod recall_tests {
    use super::test_support::ctx;
    use super::*;
    use crate::config::MemoryConfig;
    use crate::embedder::StubEmbedder;
    use crate::store::InMemoryStore;

    async fn seed() -> (Recall, Arc<dyn MemoryStore>) {
        let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let cfg = Arc::new(MemoryConfig::default());
        let rem = Remember { embedder: embedder.clone(), store: store.clone(), cfg: cfg.clone(),
            project_key: "A".into() };
        rem.execute(json!({"text": "deploys run on fridays"}), &ctx()).await.unwrap();
        rem.execute(json!({"text": "user prefers tabs", "scope": "global"}), &ctx()).await.unwrap();
        let rec = Recall { embedder, store: store.clone(), cfg, project_key: "A".into() };
        (rec, store)
    }

    #[tokio::test]
    async fn exact_query_returns_match_unrelated_returns_none() {
        let (rec, _s) = seed().await;
        // Exact stored text → cosine 1.0 ≥ relevance_threshold.
        let hit = rec.execute(json!({"query": "deploys run on fridays"}), &ctx()).await.unwrap();
        assert!(hit.content.contains("deploys run on fridays"));
        // Unrelated query → below threshold → "no relevant memories".
        let miss = rec.execute(json!({"query": "zxcv qwerty nonsense token"}), &ctx()).await.unwrap();
        assert!(miss.content.contains("No relevant memories"));
    }

    #[tokio::test]
    async fn global_visible_but_other_projects_hidden() {
        let (rec, store) = seed().await;
        // Add a project-B memory directly; project-A recall must not see it.
        let embedder = StubEmbedder::d384();
        let v = embedder.embed(&["secret from project b".to_string()]).await.unwrap().pop().unwrap();
        store.upsert(MemoryRecord { id: "b1".into(), text: "secret from project b".into(),
            scope: MemoryScope::Project("B".into()), tags: vec![], vector: v,
            created_at: 1, updated_at: 1, source: "test".into() }).await.unwrap();
        let out = rec.execute(json!({"query": "secret from project b"}), &ctx()).await.unwrap();
        assert!(!out.content.contains("secret from project b"), "cross-project leak");
        // But the global memory is reachable.
        let g = rec.execute(json!({"query": "user prefers tabs"}), &ctx()).await.unwrap();
        assert!(g.content.contains("user prefers tabs"));
    }

    #[tokio::test]
    async fn render_budget_truncates() {
        use crate::record::Scored;
        let hits: Vec<Scored> = (0..100).map(|i| Scored {
            record: MemoryRecord { id: i.to_string(), text: "x".repeat(100),
                scope: MemoryScope::Global, tags: vec![], vector: vec![1.0],
                created_at: 0, updated_at: 0, source: "t".into() },
            score: 0.9,
        }).collect();
        let body = render_hits(&hits, 512);
        assert!(body.len() <= 512 + 64);
        assert!(body.contains("[truncated"));
    }
}
```

- [ ] **Step 2: Run, then pass**

Add `pub use tools::Recall;` to `lib.rs`. Run:
`source "$HOME/.cargo/env" && cd agent && cargo test -p agent-memory recall`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add agent/crates/agent-memory/src
git commit -m "feat(memory): recall tool (threshold, scope visibility, result budget)"
```

---

## Task 8: `forget` tool

**Files:**
- Modify: `agent/crates/agent-memory/src/tools.rs`
- Modify: `agent/crates/agent-memory/src/lib.rs`

**Interfaces:**
- Produces: `Forget { embedder, store, cfg, project_key }`.

- [ ] **Step 1: Write the failing test**

Append to `tools.rs`:
```rust
pub struct Forget {
    pub embedder: Arc<dyn Embedder>,
    pub store: Arc<dyn MemoryStore>,
    pub cfg: Arc<MemoryConfig>,
    pub project_key: String,
}

#[async_trait]
impl Tool for Forget {
    fn name(&self) -> &str { "forget" }
    fn description(&self) -> &str {
        "Remove a memory. Args: either id (exact) or query (deletes the single best match \
         only if confidently similar). Never mass-deletes."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "forget".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "query": {"type": "string"}
                }
            }),
        }
    }
    fn intent(&self, _args: &Value) -> Result<ToolIntent, ToolError> {
        Ok(read_intent("forget", "remove from long-term memory".into()))
    }
    async fn execute(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        if let Some(id) = args.get("id").and_then(Value::as_str) {
            let removed = self.store.delete(id).await.map_err(store_failed)?;
            return if removed {
                Ok(ToolOutput { content: format!("Removed memory {id}."), display: None })
            } else {
                Err(ToolError::NotFound(format!("no memory with id {id}")))
            };
        }
        let query = args.get("query").and_then(Value::as_str)
            .map(str::trim).filter(|s| !s.is_empty())
            .ok_or_else(|| ToolError::InvalidArgs("provide 'id' or non-empty 'query'".into()))?;
        let qv = self.embedder.embed(&[query.to_string()]).await
            .map_err(embed_failed)?.into_iter().next().unwrap();
        let filter = ScopeFilter::ProjectAndGlobal { project_key: self.project_key.clone() };
        let hits = self.store.query(&qv, 1, &filter).await.map_err(store_failed)?;
        match hits.first() {
            Some(top) if top.score >= self.cfg.forget_threshold => {
                let id = top.record.id.clone();
                self.store.delete(&id).await.map_err(store_failed)?;
                tracing::info!(target: "memory", %id, "forget: removed by query");
                Ok(ToolOutput { content: format!("Removed memory {id}: {}", top.record.text), display: None })
            }
            _ => Ok(ToolOutput {
                content: "No confident match; nothing removed. Use a more specific query or an id.".into(),
                display: None }),
        }
    }
}
```

Tests:
```rust
#[cfg(test)]
mod forget_tests {
    use super::test_support::ctx;
    use super::*;
    use crate::config::MemoryConfig;
    use crate::embedder::StubEmbedder;
    use crate::store::InMemoryStore;
    use crate::record::ScopeFilter;

    async fn seeded() -> (Forget, Arc<dyn MemoryStore>, String) {
        let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let cfg = Arc::new(MemoryConfig::default());
        let rem = Remember { embedder: embedder.clone(), store: store.clone(), cfg: cfg.clone(),
            project_key: "A".into() };
        let out = rem.execute(json!({"text": "delete me please"}), &ctx()).await.unwrap();
        let id = out.content.trim_start_matches("Stored memory ").trim_end_matches('.').to_string();
        let f = Forget { embedder, store: store.clone(), cfg, project_key: "A".into() };
        (f, store, id)
    }

    #[tokio::test]
    async fn forget_by_id() {
        let (f, store, id) = seeded().await;
        f.execute(json!({"id": id}), &ctx()).await.unwrap();
        assert_eq!(store.count(&ScopeFilter::ProjectAndGlobal { project_key: "A".into() }).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn forget_by_query_above_threshold_deletes_one() {
        let (f, store, _id) = seeded().await;
        f.execute(json!({"query": "delete me please"}), &ctx()).await.unwrap(); // exact → 1.0
        assert_eq!(store.count(&ScopeFilter::ProjectAndGlobal { project_key: "A".into() }).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn forget_by_weak_query_deletes_nothing() {
        let (f, store, _id) = seeded().await;
        let out = f.execute(json!({"query": "qwerty unrelated zxcv"}), &ctx()).await.unwrap();
        assert!(out.content.contains("nothing removed"));
        assert_eq!(store.count(&ScopeFilter::ProjectAndGlobal { project_key: "A".into() }).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn forget_unknown_id_is_not_found() {
        let (f, _s, _id) = seeded().await;
        let err = f.execute(json!({"id": "nope"}), &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }
}
```

- [ ] **Step 2: Run, then pass**

Add `pub use tools::Forget;` to `lib.rs`. Run:
`source "$HOME/.cargo/env" && cd agent && cargo test -p agent-memory forget`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add agent/crates/agent-memory/src
git commit -m "feat(memory): forget tool (by-id + confident-query, never mass-delete)"
```

---

## Task 9: FastEmbedEmbedder + `build_tools` builder

**Files:**
- Modify: `agent/crates/agent-memory/src/embedder.rs`
- Modify: `agent/crates/agent-memory/src/lib.rs`
- Create: `agent/crates/agent-memory/tests/live_embed.rs`

**Interfaces:**
- Produces:
  - `FastEmbedEmbedder::new(cfg: &MemoryConfig) -> Result<Self, EmbedError>` (feature `onnx`).
  - `MemoryInitError`.
  - `build_tools_with(embedder: Arc<dyn Embedder>, store: Arc<dyn MemoryStore>, cfg: Arc<MemoryConfig>, scope: MemoryScope) -> Vec<Arc<dyn Tool>>` (the 3 tools).
  - `build_tools(cfg: MemoryConfig, workspace: &Path) -> Result<Vec<Arc<dyn Tool>>, MemoryInitError>` (prod: opens SqliteStore + constructs the real embedder under `onnx`, or `StubEmbedder` when the feature is off so the workspace still links).

- [ ] **Step 1: Implement `FastEmbedEmbedder` (feature-gated)**

Append to `embedder.rs`:
```rust
#[cfg(feature = "onnx")]
pub struct FastEmbedEmbedder {
    model: std::sync::Mutex<fastembed::TextEmbedding>,
    dim: usize,
}

#[cfg(feature = "onnx")]
impl FastEmbedEmbedder {
    /// Load BGE-Small-EN-v1.5 (384-dim). Downloads the ONNX model to the cache dir on
    /// first use (network required once); cached thereafter. Returns Err offline-with-no-cache.
    pub fn new(cfg: &crate::config::MemoryConfig) -> Result<Self, EmbedError> {
        use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
        let mut opts = InitOptions::new(EmbeddingModel::BGESmallENV15);
        if let Some(dir) = &cfg.model_cache_dir {
            opts = opts.with_cache_dir(dir.clone());
        }
        let model = TextEmbedding::try_new(opts).map_err(|e| EmbedError::Failed(e.to_string()))?;
        Ok(Self { model: std::sync::Mutex::new(model), dim: 384 })
    }
}

#[cfg(feature = "onnx")]
#[async_trait]
impl Embedder for FastEmbedEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let owned: Vec<String> = texts.to_vec();
        // fastembed's embed is blocking CPU work — keep it off the async reactor.
        let model = &self.model;
        let res = {
            let guard = model.lock().unwrap();
            guard.embed(owned, None)
        };
        res.map_err(|e| EmbedError::Failed(e.to_string()))
    }
    fn dim(&self) -> usize { self.dim }
}
```
> Note for the implementer: if `cargo` reports `TextEmbedding` is not `Send`/`Sync`, the `Mutex` wrapper above already makes the struct `Sync`; the blocking `embed` call holds the lock synchronously (no `.await` while held), so this is sound. If reactor-blocking is observed under load, wrap the locked call in `tokio::task::spawn_blocking` with an `Arc<Mutex<TextEmbedding>>` clone — out of scope for this slice (low call volume).

- [ ] **Step 2: Implement the builders in `lib.rs`**

Add to `agent/crates/agent-memory/src/lib.rs`:
```rust
use agent_tools::Tool;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum MemoryInitError {
    #[error("embedder init: {0}")]
    Embedder(String),
    #[error("store init: {0}")]
    Store(String),
}

/// Assemble the three tools from already-constructed parts (used by prod + tests).
pub fn build_tools_with(
    embedder: Arc<dyn Embedder>,
    store: Arc<dyn MemoryStore>,
    cfg: Arc<MemoryConfig>,
    scope: MemoryScope,
) -> Vec<Arc<dyn Tool>> {
    let key = match &scope { MemoryScope::Project(k) => k.clone(), MemoryScope::Global => String::new() };
    vec![
        Arc::new(tools::Remember { embedder: embedder.clone(), store: store.clone(), cfg: cfg.clone(), project_key: key.clone() }),
        Arc::new(tools::Recall { embedder: embedder.clone(), store: store.clone(), cfg: cfg.clone(), project_key: key.clone() }),
        Arc::new(tools::Forget { embedder, store, cfg, project_key: key }),
    ]
}

/// Production entry point: open the SQLite store + construct the embedder, returning the
/// three tools. Errors here mean "disable memory" (caller registers nothing) — never fatal.
pub fn build_tools(cfg: MemoryConfig, workspace: &Path) -> Result<Vec<Arc<dyn Tool>>, MemoryInitError> {
    let store = SqliteStore::open(&cfg.db_path).map_err(|e| MemoryInitError::Store(e.to_string()))?;
    #[cfg(feature = "onnx")]
    let embedder: Arc<dyn Embedder> = Arc::new(
        embedder::FastEmbedEmbedder::new(&cfg).map_err(|e| MemoryInitError::Embedder(e.to_string()))?);
    #[cfg(not(feature = "onnx"))]
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
    let scope = project_scope(workspace);
    Ok(build_tools_with(embedder, Arc::new(store), Arc::new(cfg), scope))
}
```
Also add `pub use embedder::FastEmbedEmbedder;` under `#[cfg(feature = "onnx")]`, and make the `tools` module fields `pub` (the structs already have `pub` fields; ensure `Remember`/`Recall`/`Forget` are re-exported, done in Tasks 6–8).

- [ ] **Step 3: Write the builder unit test + live #[ignore] test**

Add to `lib.rs` test module:
```rust
#[cfg(test)]
mod build_tests {
    use super::*;

    #[test]
    fn build_tools_with_returns_three_named_tools() {
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
        let tools = build_tools_with(embedder, store, Arc::new(MemoryConfig::default()),
            MemoryScope::Project("A".into()));
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        for n in ["remember", "recall", "forget"] {
            assert!(names.contains(&n), "missing {n}");
        }
    }
}
```

`agent/crates/agent-memory/tests/live_embed.rs`:
```rust
//! Live DoD: real embedding model, cross-process semantic recall by paraphrase.
//! Run with: cargo test -p agent-memory --test live_embed -- --ignored
#![cfg(feature = "onnx")]
use agent_memory::*;
use std::sync::Arc;

#[tokio::test]
#[ignore = "downloads + runs the real embedding model"]
async fn paraphrase_recall_across_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = MemoryConfig::default();
    cfg.db_path = tmp.path().join("memory.db");
    cfg.model_cache_dir = Some(tmp.path().join("models"));

    // Session 1: remember, then drop everything.
    {
        let tools = build_tools(cfg.clone(), tmp.path()).unwrap();
        let remember = tools.iter().find(|t| t.name() == "remember").unwrap();
        let ctx = test_ctx();
        remember.execute(serde_json::json!({
            "text": "The deployment pipeline runs every Friday afternoon."
        }), &ctx).await.unwrap();
    }
    // Session 2: fresh build over the same DB; recall by a paraphrase (no lexical overlap).
    let tools = build_tools(cfg, tmp.path()).unwrap();
    let recall = tools.iter().find(|t| t.name() == "recall").unwrap();
    let out = recall.execute(serde_json::json!({
        "query": "When do we ship releases?"
    }), &test_ctx()).await.unwrap();
    assert!(out.content.contains("Friday"), "paraphrase failed to retrieve: {}", out.content);
}

fn test_ctx() -> agent_tools::ToolCtx {
    agent_tools::ToolCtx {
        workspace: std::path::PathBuf::from("."),
        timeout: std::time::Duration::from_secs(30),
        cancel: tokio_util::sync::CancellationToken::new(),
        sandbox: Arc::new(agent_tools::HostExecutor),
    }
}
```
> Add `tokio-util = { workspace = true }` to `agent-memory` `[dev-dependencies]` for the test ctx (and `agent-tools` is already a dep).

- [ ] **Step 4: Run the hermetic suite (live test stays ignored)**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-memory`
Expected: PASS; the live test is skipped (ignored).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-memory
git commit -m "feat(memory): FastEmbedEmbedder + build_tools; live #[ignore] paraphrase DoD"
```

---

## Task 10: Wire into `agent-runtime-config` + `agent-cli`

**Files:**
- Modify: `agent/crates/agent-runtime-config/Cargo.toml`
- Modify: `agent/crates/agent-runtime-config/src/lib.rs`
- Modify: `agent/crates/agent-cli/src/main.rs`

**Interfaces:**
- Consumes: `agent_memory::{build_tools, MemoryConfig, default_db_path}`.
- Produces: `pub fn build_memory(enabled: bool, db_path: Option<PathBuf>, model_dir: Option<PathBuf>, workspace: &Path) -> Vec<Arc<dyn Tool>>` in `agent-runtime-config`.

- [ ] **Step 1: Add the dependency**

In `agent/crates/agent-runtime-config/Cargo.toml` `[dependencies]`, add:
```toml
agent-memory = { path = "../agent-memory" }
```

- [ ] **Step 2: Write the failing test for `build_memory`**

In `agent/crates/agent-runtime-config/src/lib.rs` test module add:
```rust
#[test]
fn build_memory_disabled_returns_no_tools() {
    let tools = build_memory(false, None, None, std::path::Path::new("/tmp/ws"));
    assert!(tools.is_empty());
}

#[test]
fn build_memory_enabled_returns_three_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("memory.db");
    let cache = tmp.path().join("models");
    let tools = build_memory(true, Some(db), Some(cache), tmp.path());
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    for n in ["remember", "recall", "forget"] {
        assert!(names.contains(&n), "missing {n}");
    }
}
```
> Add `tempfile = { workspace = true }` to `agent-runtime-config` `[dev-dependencies]` if absent.
> Note: this test builds the real embedder (feature `onnx` is default) and will download the model on first run; if the CI sandbox is offline, gate the second test with `#[ignore]`. The disabled-path test never needs the model.

- [ ] **Step 3: Implement `build_memory`**

In `agent/crates/agent-runtime-config/src/lib.rs`:
```rust
use agent_memory::{build_tools, MemoryConfig};

/// Build the three memory tools, or an empty vec when disabled or when construction fails
/// (model unavailable, DB unopenable). Memory is best-effort: failure disables it, never aborts.
pub fn build_memory(
    enabled: bool,
    db_path: Option<PathBuf>,
    model_dir: Option<PathBuf>,
    workspace: &Path,
) -> Vec<Arc<dyn Tool>> {
    if !enabled {
        return Vec::new();
    }
    let mut cfg = MemoryConfig::default();
    if let Some(p) = db_path {
        cfg.db_path = p;
    }
    cfg.model_cache_dir = model_dir;
    match build_tools(cfg, workspace) {
        Ok(tools) => tools,
        Err(e) => {
            tracing::warn!(target: "memory", "disabled: {e}");
            Vec::new()
        }
    }
}
```

- [ ] **Step 4: Add CLI flags + registration in `agent-cli`**

In `agent/crates/agent-cli/src/main.rs`, add to the `Cli` struct (near the sandbox flags):
```rust
/// Enable long-term memory (remember/recall/forget tools).
#[arg(long, default_value_t = false)]
memory: bool,
/// Override the memory DB path (default ~/.agent/memory.db).
#[arg(long)]
memory_db: Option<std::path::PathBuf>,
/// Override the embedding-model cache dir.
#[arg(long)]
memory_model_dir: Option<std::path::PathBuf>,
```
Add `build_memory` to the `use agent_runtime_config::{...}` import. After the skills registration block (`for t in skill_tools { registry.register(t); }`), add:
```rust
// Long-term memory: construct once (loads the embedding model) and register.
for t in agent_runtime_config::build_memory(
    cli.memory, cli.memory_db.clone(), cli.memory_model_dir.clone(), &workspace) {
    registry.register(t);
}
```

- [ ] **Step 5: Run gates**

Run:
```
source "$HOME/.cargo/env" && cd agent
cargo test -p agent-runtime-config build_memory_disabled
cargo build -p agent-cli
cargo clippy -p agent-memory -p agent-runtime-config -p agent-cli --all-targets -- -D warnings
```
Expected: disabled test PASSES; cli builds; clippy clean. (The enabled test needs the model — run it when online: `cargo test -p agent-runtime-config build_memory_enabled`.)

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-runtime-config agent/crates/agent-cli
git commit -m "feat(memory): wire memory tools into runtime-config + agent-cli (--memory)"
```

---

## Task 11: Wire into `agent-server` (survives reconfigure)

**Files:**
- Modify: `agent/crates/agent-server/src/main.rs`
- Modify: `agent/crates/agent-server/src/runtime.rs`
- Modify: `agent/crates/agent-server/src/daemon.rs`

**Interfaces:**
- Consumes: `build_memory`.
- Produces: a `memory_tools: Arc<[Arc<dyn Tool>]>` slice threaded through `RuntimeState`/`build_loop` exactly like the existing `mcp_tools`.

- [ ] **Step 1: Construct memory tools once in `main` and add flags**

In `agent/crates/agent-server/src/main.rs`, add the same three `--memory*` clap flags as Task 10. Where `mcp_tools` is built (around the `mcp_manager` block, ~line 156), add:
```rust
let memory_tools: std::sync::Arc<[std::sync::Arc<dyn agent_tools::Tool>]> =
    std::sync::Arc::from(agent_runtime_config::build_memory(
        cli.memory, cli.memory_db.clone(), cli.memory_model_dir.clone(), &workspace));
```
Pass `memory_tools` into the `RuntimeState`/params constructor alongside `mcp_tools`.

- [ ] **Step 2: Thread `memory_tools` through `runtime.rs`**

In `agent/crates/agent-server/src/runtime.rs`:
- Add `memory_tools: Arc<[Arc<dyn Tool>]>` field to the `RuntimeState` struct (next to `mcp_tools`).
- Add the parameter to `RuntimeState::new(...)` and store it.
- Add `memory_tools: &[Arc<dyn Tool>]` to `build_loop(...)`'s signature; pass `&self.memory_tools` at the call site (next to `&self.mcp_tools`).
- Inside `build_loop`, after the `for t in mcp_tools { registry.register(t.clone()); }` loop, add:
```rust
for t in memory_tools {
    registry.register(t.clone());
}
```

- [ ] **Step 3: Pass through `daemon.rs`**

In `agent/crates/agent-server/src/daemon.rs`, if `RuntimeParams` carries `mcp_tools` (it does, ~line 28), add a parallel `pub memory_tools: Arc<[Arc<dyn Tool>]>` field and forward it wherever `mcp_tools` is forwarded (~line 57).

- [ ] **Step 4: Write a smoke test**

In `agent/crates/agent-server/src/runtime.rs` test module, add (mirroring an existing `build_loop` smoke test):
```rust
#[test]
fn build_loop_registers_injected_memory_tools() {
    // A trivial stub tool stands in for a memory tool to avoid loading a model.
    use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
    use async_trait::async_trait;
    struct FakeMem;
    #[async_trait]
    impl Tool for FakeMem {
        fn name(&self) -> &str { "remember" }
        fn description(&self) -> &str { "fake" }
        fn schema(&self) -> ToolSchema {
            ToolSchema { name: "remember".into(), description: "fake".into(),
                parameters: serde_json::json!({"type":"object"}) }
        }
        fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent { tool: "remember".into(), access: Access::Read, paths: vec![],
                command: None, summary: "x".into() })
        }
        async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx)
            -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput { content: "ok".into(), display: None })
        }
    }
    // Follow the file's existing build_loop test harness to call build_loop with
    // memory_tools = &[Arc::new(FakeMem)] and assert the resulting registry has "remember".
    // (Use the same RuntimeConfig/sink/approval fixtures the neighboring smoke test uses.)
}
```
> Implementer: complete the body using the exact fixtures the existing `build_loop` smoke test in this file already constructs (sink, approval, workspace). The assertion is `built.registry.get("remember").is_some()` (or the file's equivalent accessor).

- [ ] **Step 5: Run gates**

Run:
```
source "$HOME/.cargo/env" && cd agent
cargo build -p agent-server
cargo test -p agent-server
cargo clippy -p agent-server --all-targets -- -D warnings
```
Expected: builds; tests pass; clippy clean.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-server
git commit -m "feat(memory): inject memory tools into the daemon (survives reconfigure)"
```

---

## Task 12: Docs + full-workspace gates

**Files:**
- Create: `agent/crates/agent-memory/README.md`
- Modify: `docs/superpowers/context/README.md` (mark subsystem #4 built)

- [ ] **Step 1: Write the crate README**

`agent/crates/agent-memory/README.md`: one screen covering purpose, the `Embedder`/`MemoryStore` seams, the three tools, the `--memory` flags, the `onnx` feature, the DB location, and the deferred items (auto-retrieval, RuntimeConfig/web persistence). Keep it factual and short.

- [ ] **Step 2: Update the build-order ledger**

In `docs/superpowers/context/README.md`, change subsystem #4's Status from `deferred` to `✅ built & merged` and add a row to the "Completed subsystems" table pointing at this spec + plan. Update the prose line that says "What remains … #4 (vector / long-term memory)".

- [ ] **Step 3: Run the full workspace gates**

Run:
```
source "$HOME/.cargo/env" && cd agent
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```
Expected: all tests pass (live `#[ignore]` test skipped); clippy clean.
> If `cargo test --workspace` cannot build `fastembed`/`ort` in an offline sandbox, that is a build-environment issue, not a code defect: document it in the cycle follow-ups and run the suite where the ort binary can be fetched. All *logic* tests use `StubEmbedder` and do not need the model.

- [ ] **Step 4: Commit**

```bash
git add agent/crates/agent-memory/README.md docs/superpowers/context/README.md
git commit -m "docs(memory): crate README + mark subsystem #4 built in the ledger"
```

---

## Self-Review

**Spec coverage:**
- §1 tools-first scope (remember/recall/forget, no auto-retrieval) → Tasks 6–8; auto-retrieval explicitly deferred (docs Task 12). ✓
- §2 `agent-memory` crate, `Embedder` (fastembed + stub, feature-gated), `MemoryStore` (Sqlite + InMemory), dual-binary wiring → Tasks 1–4, 9–11. ✓
- §3 project-key (git-toplevel→sha256), remember/recall/forget data flow, single `~/.agent/memory.db` → Tasks 1, 4, 6–8. ✓
- §4 failure modes: embedder/store errors → `ToolError::Failed` + loop continues (Tasks 6–8); construction failure disables memory (`build_memory`, Task 10); dimension-mismatch skip (Tasks 3–4). ✓
- §5 threat surface: scope isolation SQL-enforced + tested (Tasks 3,4,7); intent auto-allow rationale (Task 6); 0600 DB (Task 4). ✓
- §6 resource limits: all defaults in `MemoryConfig` (Task 5); enforced in remember/recall (Tasks 6,7). ✓
- §7 observability: `tracing` on every op (Tasks 6–8), content never logged at info. ✓
- §8 testing: stub embedder, store parity + persistence, tool tests, failure paths, wiring tests, live `#[ignore]` DoD → Tasks 2–4, 6–11. ✓
- §9 DoD: cross-process paraphrase recall → live test (Task 9). ✓

**Placeholder scan:** the only "implementer completes" note is Task 11 Step 4's smoke-test body, which must reuse the file's existing `build_loop` test fixtures (those are file-local and can't be reproduced blind without duplicating unrelated harness code) — the assertion and stub tool are fully specified. No `TODO`/`TBD`/"add error handling"/vague steps elsewhere.

**Type consistency:** `MemoryScope`/`ScopeFilter`/`MemoryRecord`/`Scored`/`Embedder`/`MemoryStore`/`MemoryConfig`/`build_tools(_with)`/`build_memory` signatures are identical across all tasks that consume them. Tool field structs (`embedder`,`store`,`cfg`,`project_key`) match between Tasks 6–9 and the `build_tools_with` constructor.
