use async_trait::async_trait;

/// Port for pulling relevant long-term memories into context at the start of a turn.
/// Implemented by `agent-memory`'s `MemoryRetriever`; defined here so `agent-core`
/// has no dependency on the memory crate.
///
/// Implementations MUST swallow their own errors and return an empty `Vec` on
/// failure — retrieval must never break a turn.
#[async_trait]
pub trait Retriever: Send + Sync {
    /// Return memory facts relevant to `query`, best-first, one plain string per
    /// memory (no formatting — the context manager owns presentation).
    async fn retrieve(&self, query: &str) -> Vec<String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Two;
    #[async_trait]
    impl Retriever for Two {
        async fn retrieve(&self, _q: &str) -> Vec<String> {
            vec!["a".into(), "b".into()]
        }
    }

    #[tokio::test]
    async fn retriever_is_object_safe_and_returns_lines() {
        let r: Arc<dyn Retriever> = Arc::new(Two);
        assert_eq!(
            r.retrieve("q").await,
            vec!["a".to_string(), "b".to_string()]
        );
    }
}
