//! Caller-owned artifact stores (spec §5.3, E6). The caller owns this handle
//! and passes the SAME one across loop rebuilds (server settings change) so
//! the conversation's offloaded artifacts survive — the successor of the
//! offload_store survival contract. Two stores because the composite strips
//! mount prefixes (E6): a single backend mounted twice would merge namespaces.
use agent_tools::backend::{Backend, MemBackend};
use std::sync::Arc;

pub struct SessionArtifacts {
    /// Backing store for the `large_tool_results/` mount (privileged handle).
    pub results: Arc<dyn Backend>,
    /// Backing store for the `conversation_history/` mount (privileged handle).
    pub history: Arc<dyn Backend>,
}

impl SessionArtifacts {
    pub fn new() -> Self {
        Self {
            results: Arc::new(MemBackend::new()),
            history: Arc::new(MemBackend::new()),
        }
    }
}

impl Default for SessionArtifacts {
    fn default() -> Self {
        Self::new()
    }
}
