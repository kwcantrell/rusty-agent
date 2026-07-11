//! Test-only "commit an answer the way the CLI reopen path does" helper
//! (spec §2.4 scenario 6 descope: the commit-window state is produced by
//! the REAL writer, `agent_core::checkpoint::write_answer` — the exact
//! function `agent_server::resume::commit_answer` bottoms out into, and the
//! same one `agent-cli`'s `run_sessions_reopen` calls). This module never
//! reimplements the MAC math; it only forwards to the real writer.
//!
//! Forging/tampering helpers always take `key: &[u8; 32]` explicitly (spec
//! §2.2 item 4) — never a path defaulting to `metadata_root()`.
use std::path::Path;

/// Durably commit an answer against a parked ask's checkpoint dir, exactly
/// as `run_sessions_reopen` does via `agent_server::resume::commit_answer`
/// (which itself calls `agent_core::checkpoint::write_answer(&ask.dir, key,
/// decision.approve, decision.feedback.as_deref())` — see
/// `agent-server/src/resume.rs`). `ckpt_dir` is the root/ask checkpoint dir
/// (i.e. `rig::ckpt(&session_dir)` for a root-level ask).
pub fn commit_answer_like_cli(
    ckpt_dir: &Path,
    key: &[u8; 32],
    approve: bool,
    feedback: Option<&str>,
) {
    agent_core::checkpoint::write_answer(ckpt_dir, key, approve, feedback)
        .expect("write_answer must succeed against a freshly-parked checkpoint dir");
}
