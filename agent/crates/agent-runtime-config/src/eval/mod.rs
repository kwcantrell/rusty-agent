//! Self-contained evaluation logic for the `context-evolve` optimization loop.
//! Pure (no live model): config presets, run/batch results, the promotion gate,
//! two-sided task admissibility, and the task manifest. The live driver lives in
//! `tests/eval_context.rs`.
pub mod admissibility;
pub mod config;
pub mod gate;
pub mod result;
pub mod task;

pub use admissibility::{admit, Admissibility};
pub use config::CandidateConfig;
pub use gate::{gate, heldout_ok, Verdict};
pub use result::{trajectory_matches_gold, BatchResult, RunResult, TrajectoryStep};
pub use task::{SeedFile, SessionSpec, TaskSpec};
