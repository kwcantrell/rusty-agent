//! Permission policy engine and approval channel abstraction.
mod command;
mod engine;
pub use command::{hard_floor_violation, is_auto_allowed};
pub use engine::*;
