//! Agent loop, context manager, and event model.
mod compactor;
mod context;
mod context_tools;
mod curated;
pub mod dispatch;
mod event;
mod loop_;
mod offload;
mod offload_policy;
mod recall;
mod snapshot;
pub mod stats;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub use compactor::*;
pub use context::*;
pub use context_tools::*;
pub use curated::*;
pub use dispatch::*;
pub use event::*;
pub use loop_::*;
pub use offload::*;
pub use offload_policy::*;
pub use recall::*;
pub use snapshot::{ContextSegment, ContextSnapshot};
pub use stats::SessionStats;
