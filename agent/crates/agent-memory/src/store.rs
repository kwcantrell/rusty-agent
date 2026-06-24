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
            conn.execute("DELETE FROM memories WHERE id = ?1", [id.as_str()])
                .map_err(|e| StoreError::Io(e.to_string()))?;
        }
        Ok(id)
    }
}

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
