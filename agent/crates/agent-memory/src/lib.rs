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
