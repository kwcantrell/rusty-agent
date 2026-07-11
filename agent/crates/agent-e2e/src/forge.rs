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

/// Flip one byte mid-file — synthesizes bit-rot/truncation-adjacent
/// corruption on any checkpoint payload (`parked.json`, `manifest.json`).
/// Panics if the file can't be read/written (a test bug, not a scenario).
pub fn flip_byte(path: &Path) {
    let mut b = std::fs::read(path).unwrap();
    assert!(!b.is_empty(), "cannot flip a byte in an empty file");
    let i = b.len() / 2;
    b[i] ^= 0xFF;
    std::fs::write(path, b).unwrap();
}

/// Write a forged `answer.json`: valid shape, wrong MAC — standing in for an
/// answer signed by an attacker who lacks the real key. Deviates from the
/// brief's literal "call write_answer with a DIFFERENT key" suggestion:
/// `write_answer` first calls `verified_manifest(dir, key)` against the
/// checkpoint's manifest, which was written under the REAL key, so calling
/// it with a wrong key fails closed at THAT check (`HMAC mismatch`) and
/// never reaches the answer write at all — there is no live checkpoint tree
/// a wrong key can successfully write into. Instead: call the REAL writer
/// with the REAL key (a legitimately shaped, correctly-MAC'd answer), then
/// flip one hex nibble of the resulting `mac` field on disk — still valid
/// JSON, still the real shape, but the MAC field is a value the real
/// formula never produced. `take_answer` recomputes the MAC independently
/// and must reject it exactly like a wrong-key forgery would.
pub fn forged_answer(ckpt_dir: &Path, key: &[u8; 32], approve: bool) {
    agent_core::checkpoint::write_answer(ckpt_dir, key, approve, None)
        .expect("write_answer must succeed against a freshly-parked checkpoint dir");
    let path = ckpt_dir.join("answer.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut bytes = v["mac"].as_str().unwrap().to_string().into_bytes();
    // Toggle one hex digit to a different, still-valid hex digit — the mac
    // string stays well-formed hex, but no longer the value the real
    // formula produced.
    let i = bytes.len() / 2;
    bytes[i] = if bytes[i] == b'0' { b'1' } else { b'0' };
    v["mac"] = serde_json::Value::String(String::from_utf8(bytes).unwrap());
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
}

/// Write a legacy-format `answer.json` (pre-versioning: no `feedback` key,
/// `mac` computed by no formula this version recognizes) — the current
/// versioned MAC formula (`[2, approve] ++ feedback-tag ++ manifest_hmac`)
/// differs in both length and leading bytes from any prior shape, so ANY
/// mac value here fails closed under `take_answer`, real key or not. Mirrors
/// `legacy_answer_without_feedback_field_fails_new_mac_closed`
/// (agent-core/src/checkpoint.rs).
pub fn legacy_answer(ckpt_dir: &Path, approve: bool) {
    let body = serde_json::json!({"approve": approve, "mac": "00".repeat(32)});
    std::fs::write(
        ckpt_dir.join("answer.json"),
        serde_json::to_vec(&body).unwrap(),
    )
    .unwrap();
}
