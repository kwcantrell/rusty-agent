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
