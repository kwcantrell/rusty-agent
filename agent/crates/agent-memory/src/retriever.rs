use crate::config::MemoryConfig;
use crate::embedder::Embedder;
use crate::store::MemoryStore;
use crate::tools::query_memories;
use agent_core::Retriever;
use async_trait::async_trait;
use std::sync::Arc;

/// Auto-retrieval adapter: implements `agent_core::Retriever` by running the same
/// query the `recall` tool runs, returning plain fact strings (no formatting).
pub struct MemoryRetriever {
    pub embedder: Arc<dyn Embedder>,
    pub store: Arc<dyn MemoryStore>,
    pub cfg: Arc<MemoryConfig>,
    pub project_key: String,
}

#[async_trait]
impl Retriever for MemoryRetriever {
    async fn retrieve(&self, query: &str) -> Vec<String> {
        match query_memories(
            self.embedder.as_ref(), self.store.as_ref(), &self.cfg,
            &self.project_key, query, self.cfg.default_k,
        ).await {
            Ok(hits) => hits.into_iter().map(|h| h.record.text).collect(),
            Err(e) => {
                tracing::warn!(target: "memory", "auto-retrieval failed: {e}");
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::StubEmbedder;
    use crate::record::{now_secs, MemoryRecord, MemoryScope};
    use crate::store::InMemoryStore;

    async fn seed(store: &InMemoryStore, embedder: &dyn Embedder, key: &str, text: &str) {
        let v = embedder.embed(&[text.to_string()]).await.unwrap().remove(0);
        store.upsert(MemoryRecord {
            id: uuid::Uuid::new_v4().to_string(), text: text.into(),
            scope: MemoryScope::Project(key.into()), tags: vec![], vector: v,
            created_at: now_secs(), updated_at: now_secs(), source: "test".into(),
        }).await.unwrap();
    }

    #[tokio::test]
    async fn retrieve_returns_plain_fact_lines() {
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let store = Arc::new(InMemoryStore::new());
        seed(&store, embedder.as_ref(), "K", "user prefers rust").await;
        let r = MemoryRetriever {
            embedder: embedder.clone(), store: store.clone(),
            cfg: Arc::new(MemoryConfig::default()), project_key: "K".into(),
        };
        // StubEmbedder only matches exact text above the relevance threshold.
        let lines = r.retrieve("user prefers rust").await;
        assert!(lines.iter().any(|l| l == "user prefers rust"));
        // Plain text only — no score/age/tag formatting.
        assert!(lines.iter().all(|l| !l.starts_with('[')));
    }

    #[tokio::test]
    async fn retrieve_is_empty_when_store_is_empty() {
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::d384());
        let store = Arc::new(InMemoryStore::new());
        let r = MemoryRetriever {
            embedder, store, cfg: Arc::new(MemoryConfig::default()), project_key: "K".into(),
        };
        assert!(r.retrieve("anything").await.is_empty());
    }

    /// The unit tests above use `StubEmbedder`, which only scores *exact* text
    /// above threshold — so they never prove semantic retrieval, the whole point
    /// of auto-retrieval. This test uses the real BGE-Small model: it seeds
    /// distinctly-worded memories and queries with related-but-not-identical text,
    /// asserting the semantically-matching memory comes back. Ignored by default
    /// (downloads the model on first run); run with `cargo test -- --ignored`.
    #[cfg(feature = "onnx")]
    #[tokio::test]
    #[ignore = "downloads/loads the real BGE-Small embedding model (network on first run)"]
    async fn real_embedder_retrieves_semantically_related_memory() {
        let cfg = Arc::new(MemoryConfig::default());
        let embedder: Arc<dyn Embedder> =
            Arc::new(crate::embedder::FastEmbedEmbedder::new(&cfg).expect("load BGE model"));
        let store = Arc::new(InMemoryStore::new());
        seed(&store, embedder.as_ref(), "K", "the user's favorite programming language is Rust").await;
        seed(&store, embedder.as_ref(), "K", "deployments happen on Friday afternoons").await;
        seed(&store, embedder.as_ref(), "K", "the production database is PostgreSQL 16").await;

        let r = MemoryRetriever { embedder, store, cfg, project_key: "K".into() };
        // Related to the Rust memory but NOT its stored wording — exactly what a
        // StubEmbedder cannot match.
        let lines = r.retrieve("which language should I write this service in?").await;
        assert!(
            lines.iter().any(|l| l.contains("Rust")),
            "expected the Rust memory retrieved by semantic similarity; got: {lines:?}"
        );
    }
}
