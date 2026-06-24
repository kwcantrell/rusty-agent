//! Long-term semantic memory: remember/recall/forget tools over a local vector store.
mod config;
mod record;
mod scope;
mod embedder;
mod store;
mod tools;

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
