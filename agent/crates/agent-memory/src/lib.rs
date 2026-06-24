//! Long-term semantic memory: remember/recall/forget tools over a local vector store.
mod record;
mod scope;

pub use record::{now_secs, MemoryRecord, MemoryScope, ScopeFilter, Scored};
pub use scope::project_scope;
