use std::collections::HashMap;
use std::sync::Mutex;

pub type OffloadId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OffloadKind {
    Error,
    Output,
}

#[derive(Debug, Clone)]
pub struct OffloadEntry {
    pub id: OffloadId,
    pub tool_call_id: String,
    pub tool_name: String,
    pub kind: OffloadKind,
    pub content: String,
    pub bytes: usize,
    pub turn: usize,
}

/// A retrievable side-table for content lifted out of the live context window.
/// `put` uses interior mutability so the store can be shared (`Arc<dyn OffloadStore>`)
/// between the context manager and the `context_recall` tool.
pub trait OffloadStore: Send + Sync {
    /// Store full content; returns the assigned id. `entry.id` is ignored on input.
    fn put(&self, entry: OffloadEntry) -> OffloadId;
    fn get(&self, id: OffloadId) -> Option<OffloadEntry>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

struct Inner {
    next: OffloadId,
    map: HashMap<OffloadId, OffloadEntry>,
}

/// Process-local, per-session offload table. v1 impl; the `OffloadStore` trait
/// is the seam for a persisted/semantic store later.
pub struct InMemoryOffloadStore {
    inner: Mutex<Inner>,
}

impl InMemoryOffloadStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                next: 1,
                map: HashMap::new(),
            }),
        }
    }
}

impl Default for InMemoryOffloadStore {
    fn default() -> Self {
        Self::new()
    }
}

impl OffloadStore for InMemoryOffloadStore {
    fn put(&self, mut entry: OffloadEntry) -> OffloadId {
        let mut g = self.inner.lock().unwrap();
        let id = g.next;
        g.next += 1;
        entry.id = id;
        g.map.insert(id, entry);
        id
    }

    fn get(&self, id: OffloadId) -> Option<OffloadEntry> {
        self.inner.lock().unwrap().map.get(&id).cloned()
    }

    fn len(&self) -> usize {
        self.inner.lock().unwrap().map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(content: &str) -> OffloadEntry {
        OffloadEntry {
            id: 0,
            tool_call_id: "call-1".into(),
            tool_name: "shell".into(),
            kind: OffloadKind::Error,
            content: content.into(),
            bytes: content.len(),
            turn: 0,
        }
    }

    #[test]
    fn put_assigns_increasing_ids_and_get_round_trips() {
        let store = InMemoryOffloadStore::new();
        let id1 = store.put(entry("first"));
        let id2 = store.put(entry("second"));
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(store.get(id1).unwrap().content, "first");
        assert_eq!(store.get(id2).unwrap().content, "second");
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn get_unknown_id_is_none() {
        let store = InMemoryOffloadStore::new();
        assert!(store.get(42).is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn put_overwrites_input_id_field() {
        let store = InMemoryOffloadStore::new();
        let mut e = entry("x");
        e.id = 999; // should be ignored
        let id = store.put(e);
        assert_eq!(id, 1);
        assert_eq!(store.get(1).unwrap().id, 1);
    }
}
