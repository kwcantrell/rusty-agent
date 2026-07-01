//! Long-term semantic memory: remember/recall/forget tools over a local vector store.
mod config;
mod record;
mod scope;
mod embedder;
mod store;
mod tools;
mod retriever;

use std::path::Path;
use std::sync::Arc;

pub use config::{default_db_path, MemoryConfig};
pub use record::{now_secs, MemoryRecord, MemoryScope, ScopeFilter, Scored};
pub use scope::project_scope;
pub use embedder::{cosine, EmbedError, Embedder, StubEmbedder};
#[cfg(feature = "onnx")]
pub use embedder::FastEmbedEmbedder;
pub use store::{InMemoryStore, MemoryStore, SqliteStore, StoreError};
pub use tools::Remember;
pub use tools::Recall;
pub use tools::Forget;
pub use retriever::MemoryRetriever;

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

    pub async fn recall_preview(&self, query: &str) -> Vec<ScoredRow> {
        let key = match &self.scope {
            MemoryScope::Project(k) => k.clone(), MemoryScope::Global => String::new(),
        };
        match crate::tools::query_memories(self.embedder.as_ref(), self.store.as_ref(),
            &self.cfg, &key, query, self.cfg.default_k).await {
            Ok(hits) => hits.into_iter().map(|h| ScoredRow {
                id: h.record.id, text: h.record.text, score: h.score,
                scope_kind: h.record.scope.kind().into() }).collect(),
            Err(e) => {
                tracing::warn!(error = %e, "recall_preview failed; returning no results");
                Vec::new()
            }
        }
    }
}

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
) -> Vec<Arc<dyn agent_tools::Tool>> {
    let key = match &scope {
        MemoryScope::Project(k) => k.clone(),
        MemoryScope::Global => String::new(),
    };
    vec![
        Arc::new(tools::Remember { embedder: embedder.clone(), store: store.clone(), cfg: cfg.clone(), project_key: key.clone() }),
        Arc::new(tools::Recall { embedder: embedder.clone(), store: store.clone(), cfg: cfg.clone(), project_key: key.clone() }),
        Arc::new(tools::Forget { embedder, store, cfg, project_key: key }),
    ]
}

/// Production entry point: open the SQLite store + construct the embedder, returning the
/// three tools. Errors here mean "disable memory" (caller registers nothing) — never fatal.
pub fn build_tools(cfg: MemoryConfig, workspace: &Path) -> Result<Vec<Arc<dyn agent_tools::Tool>>, MemoryInitError> {
    let store = SqliteStore::open(&cfg.db_path).map_err(|e| MemoryInitError::Store(e.to_string()))?;
    #[cfg(feature = "onnx")]
    let embedder: Arc<dyn Embedder> = Arc::new(
        embedder::FastEmbedEmbedder::new(&cfg).map_err(|e| MemoryInitError::Embedder(e.to_string()))?);
    #[cfg(not(feature = "onnx"))]
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
    let scope = project_scope(workspace);
    Ok(build_tools_with(embedder, Arc::new(store), Arc::new(cfg), scope))
}

/// Like `build_tools`, but also returns a `MemoryRetriever` sharing the SAME
/// store + embedder, for auto-retrieval. Errors disable memory (caller falls back).
pub fn build_tools_and_retriever(
    cfg: MemoryConfig,
    workspace: &Path,
) -> Result<(Vec<Arc<dyn agent_tools::Tool>>, Arc<dyn agent_core::Retriever>), MemoryInitError> {
    let parts = open_memory_parts(cfg)?;
    Ok(assemble_memory(&parts, workspace))
}

/// The expensive, workspace-independent half of memory construction: the embedding
/// model and the store handle. Build once; assemble per workspace via `assemble_memory`.
#[derive(Clone)]
pub struct MemoryParts {
    pub embedder: Arc<dyn Embedder>,
    pub store: Arc<dyn MemoryStore>,
    pub cfg: Arc<MemoryConfig>,
}

/// Open the store + load the embedder once (network on first run for the model).
/// Errors mean "disable memory" — never fatal.
pub fn open_memory_parts(cfg: MemoryConfig) -> Result<MemoryParts, MemoryInitError> {
    let store: Arc<dyn MemoryStore> =
        Arc::new(SqliteStore::open(&cfg.db_path).map_err(|e| MemoryInitError::Store(e.to_string()))?);
    #[cfg(feature = "onnx")]
    let embedder: Arc<dyn Embedder> = Arc::new(
        embedder::FastEmbedEmbedder::new(&cfg).map_err(|e| MemoryInitError::Embedder(e.to_string()))?);
    #[cfg(not(feature = "onnx"))]
    let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
    Ok(MemoryParts { embedder, store, cfg: Arc::new(cfg) })
}

/// Cheap, workspace-scoped assembly: derive the project scope from `workspace`,
/// then build the three tools and the auto-retrieval retriever. No model load.
pub fn assemble_memory(
    parts: &MemoryParts,
    workspace: &Path,
) -> (Vec<Arc<dyn agent_tools::Tool>>, Arc<dyn agent_core::Retriever>) {
    let scope = project_scope(workspace);
    let key = match &scope {
        MemoryScope::Project(k) => k.clone(),
        MemoryScope::Global => String::new(),
    };
    let tools = build_tools_with(parts.embedder.clone(), parts.store.clone(), parts.cfg.clone(), scope);
    let retriever: Arc<dyn agent_core::Retriever> = Arc::new(retriever::MemoryRetriever {
        embedder: parts.embedder.clone(),
        store: parts.store.clone(),
        cfg: parts.cfg.clone(),
        project_key: key,
    });
    (tools, retriever)
}

#[cfg(test)]
mod admin_tests {
    use super::*;
    use std::sync::Arc;

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
}

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

    #[tokio::test]
    async fn assemble_memory_scopes_to_workspace() {
        use crate::record::{MemoryRecord, MemoryScope, now_secs};
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let store: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::new());
        // Seed a memory scoped to workspace A's project key.
        let key_a = match project_scope(Path::new("/tmp/ws-a")) {
            MemoryScope::Project(k) => k, MemoryScope::Global => unreachable!(),
        };
        let v = embedder.embed(&["alpha fact".to_string()]).await.unwrap().remove(0);
        store.upsert(MemoryRecord { id: "1".into(), text: "alpha fact".into(),
            scope: MemoryScope::Project(key_a), tags: vec![], vector: v,
            created_at: now_secs(), updated_at: now_secs(), source: "t".into() }).await.unwrap();
        let parts = MemoryParts { embedder, store, cfg: Arc::new(MemoryConfig::default()) };

        // Workspace A assembles three tools and a retriever that finds the memory.
        let (tools_a, retr_a) = assemble_memory(&parts, Path::new("/tmp/ws-a"));
        let names: Vec<&str> = tools_a.iter().map(|t| t.name()).collect();
        for n in ["remember", "recall", "forget"] { assert!(names.contains(&n), "missing {n}"); }
        assert!(retr_a.retrieve("alpha fact").await.iter().any(|l| l == "alpha fact"));

        // Workspace B has a different project scope → does not see A's memory.
        let (_tools_b, retr_b) = assemble_memory(&parts, Path::new("/tmp/ws-b"));
        assert!(retr_b.retrieve("alpha fact").await.is_empty(), "cross-workspace leak");
    }
}
