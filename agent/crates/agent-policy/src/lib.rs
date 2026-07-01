//! Permission policy engine and approval channel abstraction.
mod engine;
mod command;
pub use engine::*;
pub use command::{hard_floor_violation, is_auto_allowed};
