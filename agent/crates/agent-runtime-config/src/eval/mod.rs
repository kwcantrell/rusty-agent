//! Self-contained evaluation logic for the `context-evolve` optimization loop.
//! Pure (no live model): config presets, run/batch results, the promotion gate,
//! two-sided task admissibility, and the task manifest. The live driver lives in
//! `tests/eval_context.rs`.
pub mod config;

pub use config::CandidateConfig;
