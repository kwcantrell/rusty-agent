//! Agent loop, context manager, and event model.
mod artifacts;
pub mod checkpoint;
mod compactor;
mod context;
mod context_tools;
mod curated;
pub mod dispatch;
mod event;
mod loop_;
mod middleware;
mod offload_policy;
mod response_format;
mod snapshot;
pub mod stats;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
mod todos;
pub use artifacts::SessionArtifacts;
pub use checkpoint::{
    dump_artifacts, restore_artifacts, sanitize_dir_key, AskGuard, Checkpoint, CheckpointError,
    Checkpointer, GateRecord, Guardrails, InvalidParked, ParkedTurn, PendingSnapshot,
    CHECKPOINT_VERSION,
};
pub use compactor::*;
pub use context::*;
pub use context_tools::*;
pub use curated::*;
pub use dispatch::*;
pub use event::*;
pub use loop_::*;
pub use middleware::*;
pub use offload_policy::*;
pub use response_format::*;
pub use snapshot::{ContextSegment, ContextSnapshot};
pub use stats::SessionStats;
pub use todos::*;
