//! Long-term semantic memory: remember/recall/forget tools over a local vector store.
mod config;
mod record;
mod scope;
mod embedder;
mod store;
mod tools;

pub use config::{default_db_path, MemoryConfig};
pub use record::{now_secs, MemoryRecord, MemoryScope, ScopeFilter, Scored};
pub use scope::project_scope;
pub use embedder::{cosine, EmbedError, Embedder, StubEmbedder};
pub use store::{InMemoryStore, MemoryStore, SqliteStore, StoreError};
pub use tools::Remember;
pub use tools::Recall;
