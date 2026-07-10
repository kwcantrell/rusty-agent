# Durable HITL Slice 4B-0 — Session-Descriptor Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every session a durable, restart-stable identity on disk — a
per-session directory with a `descriptor.json` (id, workspace, config
provenance) that the daemon can index at startup — plus the daemon-local
secret file the 4B-1 checkpoint HMAC will key from. TraceWriter stops
minting identity and consumes it instead.

**Architecture:** One new module `session_meta.rs` in `agent-runtime-config`
(the crate both frontends already share and where `build_trace` lives) owns
minting, descriptor I/O, scanning, pruning, and the secret. `RuntimeState`
(server) and `main.rs` (CLI) mint the id at construction, write the
descriptor, then pass the id into `build_trace`. No wire changes, no
frontend changes, no loop changes.

**Tech Stack:** Rust (agent/ Cargo workspace), serde/serde_json, sha2
(already a dep of agent-runtime-config), std-only entropy (/dev/urandom
with a hashed fallback), tempfile (dev-dep, already used).

**Spec:** `docs/superpowers/specs/2026-07-10-durable-hitl-design.md` §0
(Slice 4B-0), §2.1, §3.6, §3.1. All spec/plan `file:line` anchors are
orientation only — **locate quoted code by content before editing** (repo
convention).

## Global Constraints

- Session dirs `0o700`; all files `0o600` **including atomic-rename temp
  files** (spec §2.1; trace precedent in `TraceWriter::create`).
- Atomic writes = temp + `fs::rename`; temp name appends the FULL filename
  (`descriptor.json.tmp`, never `descriptor.tmp`) — 4A-1 collision gotcha.
- Session identity is **trace-independent**: descriptor is written even
  when `cfg.trace == false` (spec §1 baseline gap). Placement honors
  `cfg.trace_dir` override so tests stay in tempdirs.
- Session id has **no PID component** and keeps an epoch-seconds name
  prefix (trace retention pruning name-sorts and relies on chronological
  ordering — comment on `prune_retention`).
- Trace JSONL **filename, shape, and 0o600 mode unchanged** (spec §3.6):
  still flat `sessions/{id}.jsonl` beside the new `sessions/{id}/` dirs.
- Runs that never park gain no behavior change (spec §3.1) — this slice is
  identity + files only.
- Two Cargo workspaces: all `cargo` commands here run in `agent/`
  (`cargo test -p agent-runtime-config` etc.).
- Conventional commits `type(scope): summary`. Full
  `bash scripts/ci.sh` green before merge.

---

### Task 0: Branch

**Files:** none (git only)

- [ ] **Step 1: Branch off main**

```bash
cd /home/kalen/rust-agent-runtime
git checkout main && git checkout -b feature/durable-hitl-4b0
```

- [ ] **Step 2: Verify clean base**

Run: `git status --short` — Expected: empty output.

---

### Task 1: `session_meta.rs` — id minting, descriptor round-trip, modes

**Files:**
- Create: `agent/crates/agent-runtime-config/src/session_meta.rs`
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (add
  `mod session_meta;` next to `mod project_key;` and re-export:
  `pub use session_meta::{mint_session_id, metadata_root, sessions_root, session_dir, write_descriptor, load_descriptor, scan_descriptors, prune_session_dirs, load_or_create_secret, SessionDescriptor, DESCRIPTOR_SCHEMA};`)
- Test: inline `#[cfg(test)] mod tests` in `session_meta.rs`

**Interfaces:**
- Consumes: `crate::RuntimeConfig` (fields `trace_dir: Option<String>`),
  `sha2` dep, `tempfile` dev-dep.
- Produces (later tasks + 4B-1 rely on these exact names):

```rust
pub const DESCRIPTOR_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionDescriptor {
    pub schema: u32,
    pub session_id: String,
    pub workspace: PathBuf,
    pub created_ms: u64,
    pub config_path: Option<PathBuf>,
}

pub fn mint_session_id() -> String;                       // "{secs}-{08x}", no PID
pub fn metadata_root() -> Option<PathBuf>;                // $HOME/.rusty-agent
pub fn sessions_root(cfg: &RuntimeConfig) -> Option<PathBuf>;
pub fn session_dir(root: &Path, session_id: &str) -> PathBuf;
pub fn write_descriptor(root: &Path, d: &SessionDescriptor) -> std::io::Result<()>;
pub fn load_descriptor(dir: &Path) -> Option<SessionDescriptor>;
// Task 2 adds: scan_descriptors, prune_session_dirs
// Task 3 adds: load_or_create_secret
```

- [ ] **Step 1: Write the failing tests**

Create `session_meta.rs` with only the test module (plus `use` lines) so
the file compiles test-first:

```rust
//! Durable session metadata: restart-stable session identity, per-session
//! descriptor dirs, and the daemon-local secret (4B-0, spec 2026-07-10 §2.1).
//! Identity is trace-independent: this module, not TraceWriter, mints ids.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_epoch_prefixed_hex_suffixed_and_pid_free() {
        let id = mint_session_id();
        let (secs, suffix) = id.split_once('-').expect("secs-suffix shape");
        assert!(secs.parse::<u64>().is_ok(), "epoch prefix: {id}");
        assert_eq!(suffix.len(), 8, "8 hex chars: {id}");
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(suffix, format!("{:08x}", std::process::id()), "no PID");
    }

    #[test]
    fn session_ids_do_not_collide_within_one_second() {
        let a = mint_session_id();
        let b = mint_session_id();
        assert_ne!(a, b);
    }

    #[test]
    fn descriptor_round_trips_via_atomic_write() {
        let root = tempfile::tempdir().unwrap();
        let d = SessionDescriptor {
            schema: DESCRIPTOR_SCHEMA,
            session_id: "1751-abcd1234".into(),
            workspace: PathBuf::from("/tmp/ws"),
            created_ms: 42,
            config_path: Some(PathBuf::from("/tmp/rt.json")),
        };
        write_descriptor(root.path(), &d).unwrap();
        let dir = session_dir(root.path(), &d.session_id);
        assert_eq!(load_descriptor(&dir), Some(d));
        // atomic: no temp residue
        assert!(!dir.join("descriptor.json.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_dir_is_0700_and_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let d = SessionDescriptor {
            schema: DESCRIPTOR_SCHEMA,
            session_id: "1751-00000001".into(),
            workspace: PathBuf::from("/w"),
            created_ms: 1,
            config_path: None,
        };
        write_descriptor(root.path(), &d).unwrap();
        let dir = session_dir(root.path(), &d.session_id);
        let dmode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        let fmode = std::fs::metadata(dir.join("descriptor.json"))
            .unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700);
        assert_eq!(fmode, 0o600);
    }

    #[test]
    fn load_descriptor_none_on_missing_or_corrupt() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(load_descriptor(&root.path().join("nope")), None);
        let dir = session_dir(root.path(), "1751-deadbeef");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("descriptor.json"), b"{not json").unwrap();
        assert_eq!(load_descriptor(&dir), None);
    }

    #[test]
    fn sessions_root_honors_trace_dir_override() {
        let mut cfg = crate::RuntimeConfig::from_launch(
            "openai".into(), "http://localhost:8080".into(),
            "m1".into(), "native".into(), 8192,
        );
        cfg.trace_dir = Some("/custom/dir".into());
        assert_eq!(sessions_root(&cfg), Some(PathBuf::from("/custom/dir")));
        cfg.trace_dir = None;
        let root = sessions_root(&cfg);
        // HOME-based default ends in .rusty-agent/sessions
        if let Some(r) = root {
            assert!(r.ends_with(".rusty-agent/sessions"), "{}", r.display());
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-runtime-config session_meta -- --nocapture`
Expected: COMPILE ERROR (`mint_session_id` etc. not found). Add the
`mod session_meta;` + re-export lines to `lib.rs` first so the module is
reachable; the errors must be missing items, not a missing module.

- [ ] **Step 3: Implement**

Add above the test module:

```rust
pub const DESCRIPTOR_SCHEMA: u32 = 1;

/// One session's durable identity record: `sessions/<id>/descriptor.json`.
/// Written at session construction, rewritten on workspace switch; the
/// startup index (4B-1 attach-to-resume) scans these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDescriptor {
    pub schema: u32,
    pub session_id: String,
    pub workspace: PathBuf,
    pub created_ms: u64,
    pub config_path: Option<PathBuf>,
}

/// "{epoch_secs}-{8 hex}": chronological name-sort (retention pruning
/// relies on the epoch prefix) with a random suffix instead of the old
/// PID (a restarted daemon must be able to own dirs it did not create,
/// and PIDs recycle).
pub fn mint_session_id() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut b = [0u8; 4];
    entropy(&mut b);
    format!("{secs}-{:02x}{:02x}{:02x}{:02x}", b[0], b[1], b[2], b[3])
}

/// Fill `buf` with OS entropy. /dev/urandom on unix; fallback (non-unix or
/// read failure) folds RandomState + time through SHA-256 — good enough for
/// id-suffix uniqueness and secret generation on the platforms we ship.
fn entropy(buf: &mut [u8]) {
    #[cfg(unix)]
    {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            if f.read_exact(buf).is_ok() {
                return;
            }
        }
    }
    use sha2::{Digest, Sha256};
    use std::hash::{BuildHasher, Hasher};
    let mut h = Sha256::new();
    for _ in 0..(buf.len() / 32 + 1) {
        let r = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        h.update(r.to_le_bytes());
        h.update(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                .to_le_bytes(),
        );
    }
    let d = h.finalize();
    for (i, byte) in buf.iter_mut().enumerate() {
        *byte = d[i % d.len()];
    }
}

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// $HOME/.rusty-agent (the 4A-0 metadata root).
pub fn metadata_root() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".rusty-agent"))
}

/// Where session artifacts (trace jsonl, descriptor dirs) live. Honors the
/// `trace_dir` override so tests and custom setups stay self-contained —
/// but is NOT gated on `cfg.trace`: identity exists even with tracing off.
pub fn sessions_root(cfg: &crate::RuntimeConfig) -> Option<PathBuf> {
    match &cfg.trace_dir {
        Some(d) => Some(PathBuf::from(d)),
        None => Some(metadata_root()?.join("sessions")),
    }
}

pub fn session_dir(root: &Path, session_id: &str) -> PathBuf {
    root.join(session_id)
}

/// Atomic (temp + rename), dir 0o700, file 0o600 incl. the temp file.
pub fn write_descriptor(root: &Path, d: &SessionDescriptor) -> std::io::Result<()> {
    let dir = session_dir(root, &d.session_id);
    create_dir_0700(&dir)?;
    let body = serde_json::to_vec_pretty(d)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    atomic_write_0600(&dir.join("descriptor.json"), &body)
}

pub fn load_descriptor(dir: &Path) -> Option<SessionDescriptor> {
    let bytes = std::fs::read(dir.join("descriptor.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn create_dir_0700(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
            Err(e) => return Err(e),
        }
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)
}

/// Temp name appends the FULL filename ("descriptor.json.tmp") — 4A-1
/// cross-path same-stem collision gotcha. Temp is created 0o600 so the
/// rename never widens modes.
fn atomic_write_0600(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "no filename"))?;
    let tmp = path.with_file_name(format!("{file_name}.tmp"));
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
```

Note on `create_dir_0700`: `DirBuilder::recursive(true)` applies the mode
to created dirs only; a pre-existing dir keeps its mode — matching the
trace precedent ("existing files keep their perms").

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-runtime-config session_meta`
Expected: all Task-1 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/session_meta.rs agent/crates/agent-runtime-config/src/lib.rs
git commit -m "feat(runtime-config): session_meta — durable session id + descriptor (4B-0)"
```

---

### Task 2: Startup scan + session-dir pruning

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/session_meta.rs`
- Test: same inline module

**Interfaces:**
- Produces:

```rust
/// All readable descriptors under root (corrupt/missing skipped), newest
/// first by session-id name order.
pub fn scan_descriptors(root: &Path) -> Vec<SessionDescriptor>;
/// Keep the newest `keep` session DIRS (name-sorted); never remove a dir
/// containing checkpoint/parked.json (a parked run outlives retention).
pub fn prune_session_dirs(root: &Path, keep: usize);
```

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn scan_returns_descriptors_newest_first_and_skips_corrupt() {
        let root = tempfile::tempdir().unwrap();
        for (id, ms) in [("100-aaaaaaaa", 1u64), ("200-bbbbbbbb", 2)] {
            write_descriptor(root.path(), &SessionDescriptor {
                schema: DESCRIPTOR_SCHEMA, session_id: id.into(),
                workspace: PathBuf::from("/w"), created_ms: ms, config_path: None,
            }).unwrap();
        }
        // corrupt dir: descriptor unreadable — skipped, not fatal
        let bad = session_dir(root.path(), "150-cccccccc");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("descriptor.json"), b"junk").unwrap();
        // stray non-session file at root — ignored
        std::fs::write(root.path().join("300-dddddddd.jsonl"), b"{}").unwrap();
        let got = scan_descriptors(root.path());
        let ids: Vec<&str> = got.iter().map(|d| d.session_id.as_str()).collect();
        assert_eq!(ids, vec!["200-bbbbbbbb", "100-aaaaaaaa"]);
    }

    #[test]
    fn prune_keeps_newest_dirs_and_never_removes_parked() {
        let root = tempfile::tempdir().unwrap();
        for id in ["100-aaaaaaaa", "200-bbbbbbbb", "300-cccccccc"] {
            write_descriptor(root.path(), &SessionDescriptor {
                schema: DESCRIPTOR_SCHEMA, session_id: id.into(),
                workspace: PathBuf::from("/w"), created_ms: 1, config_path: None,
            }).unwrap();
        }
        // oldest is parked → protected even though it would be pruned
        let parked = session_dir(root.path(), "100-aaaaaaaa").join("checkpoint");
        std::fs::create_dir_all(&parked).unwrap();
        std::fs::write(parked.join("parked.json"), b"{}").unwrap();
        // a trace .jsonl at root must be untouched by DIR pruning
        std::fs::write(root.path().join("050-eeeeeeee.jsonl"), b"{}").unwrap();
        prune_session_dirs(root.path(), 1);
        assert!(session_dir(root.path(), "100-aaaaaaaa").exists(), "parked kept");
        assert!(!session_dir(root.path(), "200-bbbbbbbb").exists(), "old pruned");
        assert!(session_dir(root.path(), "300-cccccccc").exists(), "newest kept");
        assert!(root.path().join("050-eeeeeeee.jsonl").exists(), "jsonl untouched");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-runtime-config session_meta`
Expected: COMPILE ERROR — `scan_descriptors` / `prune_session_dirs` not found.

- [ ] **Step 3: Implement**

```rust
pub fn scan_descriptors(root: &Path) -> Vec<SessionDescriptor> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs.reverse(); // epoch-prefixed names ⇒ newest first
    dirs.iter().filter_map(|d| load_descriptor(d)).collect()
}

pub fn prune_session_dirs(root: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    if dirs.len() <= keep {
        return;
    }
    let excess = dirs.len() - keep;
    for d in dirs.into_iter().take(excess) {
        if d.join("checkpoint").join("parked.json").exists() {
            continue; // a parked run outlives retention (4B-1 resumes it)
        }
        let _ = std::fs::remove_dir_all(d);
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-runtime-config session_meta`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/session_meta.rs
git commit -m "feat(runtime-config): descriptor startup scan + parked-aware dir pruning (4B-0)"
```

---

### Task 3: Daemon-local secret (E6b key material)

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/session_meta.rs`
- Test: same inline module

**Interfaces:**
- Produces: `pub fn load_or_create_secret(metadata_root: &Path) -> std::io::Result<[u8; 32]>`
  — 4B-1's checkpoint HMAC keys from this. File: `<metadata_root>/secret`.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn secret_created_once_then_stable() {
        let root = tempfile::tempdir().unwrap();
        let a = load_or_create_secret(root.path()).unwrap();
        let b = load_or_create_secret(root.path()).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, [0u8; 32], "not all-zero");
    }

    #[cfg(unix)]
    #[test]
    fn secret_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        load_or_create_secret(root.path()).unwrap();
        let mode = std::fs::metadata(root.path().join("secret"))
            .unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn wrong_length_secret_is_a_loud_error_not_silent_regen() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("secret"), b"short").unwrap();
        let err = load_or_create_secret(root.path()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        // silent regeneration would invalidate every existing checkpoint MAC
        assert_eq!(std::fs::read(root.path().join("secret")).unwrap(), b"short");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-runtime-config session_meta`
Expected: COMPILE ERROR — `load_or_create_secret` not found.

- [ ] **Step 3: Implement**

```rust
/// 32 bytes of key material at <metadata_root>/secret, created 0o600 on
/// first use. A wrong-length file is InvalidData — never silently
/// regenerated (that would invalidate every existing checkpoint MAC).
pub fn load_or_create_secret(metadata_root: &Path) -> std::io::Result<[u8; 32]> {
    let path = metadata_root.join("secret");
    match std::fs::read(&path) {
        Ok(bytes) => bytes.as_slice().try_into().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{}: expected 32 bytes, found {}", path.display(), bytes.len()),
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            create_dir_0700(metadata_root)?;
            let mut key = [0u8; 32];
            entropy(&mut key);
            atomic_write_0600(&path, &key)?;
            Ok(key)
        }
        Err(e) => Err(e),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-runtime-config session_meta`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/session_meta.rs
git commit -m "feat(runtime-config): daemon-local secret for checkpoint HMAC (4B-0, E6b)"
```

---

### Task 4: TraceWriter consumes the session id

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/trace.rs`
- Test: existing inline tests in `trace.rs` (updated call sites + one new pin)

**Interfaces:**
- Consumes: `crate::session_meta::mint_session_id` (tests only).
- Produces (Tasks 5–6 rely on):
  - `TraceWriter::create(dir: &Path, max_mb: u64, session_id: String) -> Option<Arc<TraceWriter>>`
  - `pub fn build_trace(cfg: &RuntimeConfig, session_id: &str) -> Option<Arc<TraceWriter>>`

- [ ] **Step 1: Change the signatures (this is refactor-first, tests updated in the same step)**

In `trace.rs`:
1. `TraceWriter::create` gains `session_id: String` as the third
   parameter; delete the line `let session_id = mint_session_id();`
   (locate by content). Everything else in `create` is unchanged — same
   filename `format!("{session_id}.jsonl")`, same 0o600, same header.
2. Delete `fn mint_session_id()` from `trace.rs` entirely (identity now
   lives in `session_meta`; the old PID-based minting must not survive as
   dead code).
3. `build_trace` gains `session_id: &str` and passes it through — replace
   the body's final line with:

```rust
    TraceWriter::create(&dir, cfg.trace_max_mb, session_id.to_string())
```

   Keep the `sessions_root` logic in `build_trace` delegating to the new
   helper — replace the inline `dir` computation (locate by content:
   `match &cfg.trace_dir`) with:

```rust
    let dir = crate::session_meta::sessions_root(cfg)?;
```

4. Update every `TraceWriter::create(` / `build_trace(` call site in
   `trace.rs`'s test module: pass
   `crate::session_meta::mint_session_id()` (or a literal like
   `"100-aaaaaaaa".to_string()` where the test asserts on the filename).
   Find them all: `grep -n "TraceWriter::create\|build_trace(" agent/crates/agent-runtime-config/src/trace.rs`

- [ ] **Step 2: Add the parity pin test**

In `trace.rs` tests:

```rust
    #[test]
    fn trace_filename_is_flat_session_id_jsonl_beside_descriptor_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let t = TraceWriter::create(dir.path(), 10, "123-cafebabe".into()).unwrap();
        assert_eq!(t.session_id(), "123-cafebabe");
        // flat file, NOT inside the per-session dir (spec §3.6: trace
        // naming/shape unchanged; descriptor dirs sit beside it)
        assert!(dir.path().join("123-cafebabe.jsonl").is_file());
        assert!(!dir.path().join("123-cafebabe").join("123-cafebabe.jsonl").exists());
    }
```

- [ ] **Step 3: Fix the two production call sites so the crate compiles**

`build_trace` callers (only two exist; verify with
`grep -rn "build_trace(" agent/crates --include=*.rs`):
- `agent-server/src/runtime.rs` (in `RuntimeState::new`, locate by content
  `let trace = agent_runtime_config::build_trace(&config);`) — TEMPORARY
  bridge so this task compiles standalone; Task 5 replaces it:

```rust
        let session_id = agent_runtime_config::mint_session_id();
        let trace = agent_runtime_config::build_trace(&config, &session_id);
```

- `agent-cli/src/main.rs` (locate by content
  `let trace = agent_runtime_config::build_trace(&rt);`) — same TEMPORARY bridge:

```rust
    let session_id = agent_runtime_config::mint_session_id();
    let trace = agent_runtime_config::build_trace(&rt, &session_id);
```

- [ ] **Step 4: Run the crate tests**

Run: `cd agent && cargo test -p agent-runtime-config && cargo build -p agent-server -p agent-cli`
Expected: all PASS / clean build. Existing pins
(`trace_file_is_created_0600`, `trace_writes_parseable_jsonl_with_header`,
`trace_prunes_to_retention`) must pass unmodified in their assertions —
only their `create` calls changed.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/trace.rs agent/crates/agent-server/src/runtime.rs agent/crates/agent-cli/src/main.rs
git commit -m "refactor(trace): TraceWriter consumes session id instead of minting it (4B-0)"
```

---

### Task 5: RuntimeState owns identity; descriptor at construction + workspace rewrite

**Files:**
- Modify: `agent/crates/agent-server/src/runtime.rs` (RuntimeState field +
  new(), getters), `agent/crates/agent-server/src/session.rs`
  (`set_workspace` rewrite hook)
- Test: inline tests in `runtime.rs` and `session.rs`

**Interfaces:**
- Consumes: Task 1–2 (`mint_session_id`, `sessions_root`,
  `write_descriptor`, `prune_session_dirs`, `load_descriptor`,
  `SessionDescriptor`, `DESCRIPTOR_SCHEMA`), Task 4 (`build_trace(cfg, id)`).
- Produces (4B-1 relies on): `RuntimeState::session_id(&self) -> &str`,
  `RuntimeState::rewrite_descriptor_workspace(&self, workspace: &Path)`.

- [ ] **Step 1: Write the failing tests**

In `runtime.rs` tests (the module already has `make_with_tools()`; add a
sibling helper that sets `trace_dir` to a tempdir so descriptors land
there — do NOT reuse `make_with_tools`, whose config leaves
`trace_dir: None` and would write to real `$HOME`):

```rust
    fn make_with_trace_dir() -> (RuntimeState, tempfile::TempDir, tempfile::TempDir) {
        let (sink, approval) = parts();
        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let path = ws.path().join("rt.json");
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
        );
        cfg.trace_dir = Some(sessions.path().to_string_lossy().into_owned());
        let rs = RuntimeState::new(
            cfg,
            sink,
            approval,
            ws.path().to_path_buf(),
            None,
            "claude".into(),
            path,
            Arc::from(Vec::<Arc<dyn Tool>>::new()),
            crate::daemon::SYSTEM_PROMPT.to_string(),
        );
        (rs, ws, sessions)
    }

    #[test]
    fn runtime_writes_descriptor_at_construction_and_exposes_id() {
        let (rs, ws, sessions) = make_with_trace_dir();
        let id = rs.session_id().to_string();
        assert!(!id.is_empty());
        let dir = agent_runtime_config::session_dir(sessions.path(), &id);
        let d = agent_runtime_config::load_descriptor(&dir).expect("descriptor written");
        assert_eq!(d.session_id, id);
        assert_eq!(d.workspace, ws.path());
        assert_eq!(d.schema, agent_runtime_config::DESCRIPTOR_SCHEMA);
        assert!(d.config_path.is_some());
        // trace (enabled by from_launch) shares the SAME id
        assert!(sessions.path().join(format!("{id}.jsonl")).exists());
    }

    #[test]
    fn rewrite_descriptor_workspace_updates_only_workspace() {
        let (rs, _ws, sessions) = make_with_trace_dir();
        let id = rs.session_id().to_string();
        let dir = agent_runtime_config::session_dir(sessions.path(), &id);
        let before = agent_runtime_config::load_descriptor(&dir).unwrap();
        rs.rewrite_descriptor_workspace(std::path::Path::new("/elsewhere"));
        let after = agent_runtime_config::load_descriptor(&dir).unwrap();
        assert_eq!(after.workspace, std::path::Path::new("/elsewhere"));
        assert_eq!(after.session_id, before.session_id);
        assert_eq!(after.created_ms, before.created_ms);
    }
```

In `session.rs` tests (the module constructs sessions via
`Session::from_params`; find the existing params builder near
`let sess = Session::from_params(params);` and mirror how it builds
`DaemonParams`, adding a `trace_dir` override into `params.config`):

```rust
    #[tokio::test]
    async fn set_workspace_rewrites_descriptor() {
        // Build params exactly like the existing session tests do, but with
        // config.trace_dir pointed at a tempdir (see make_with_trace_dir in
        // runtime.rs for the pattern). Then:
        // let sess = Session::from_params(params);
        // sess.set_workspace(PathBuf::from("/elsewhere")).await;
        // load_descriptor(&session_dir(sessions.path(), sess_id)) →
        //     workspace == "/elsewhere"
    }
```

(The comment block above is the shape; write the real test against the
actual params builder: **`crate::setup::local_params(...)`** — not a raw
`DaemonParams` literal. **Overlay hazard (plan-review finding 2):**
`Session::from_params` calls
`RuntimeConfig::load_over(params.config.clone(), &params.config_path)`,
which overlays the on-disk file over the in-memory config — so the test
must (a) point `params.config_path` at a **non-existent** file (the
existing tests' `dir.path().join("rt.json")` pattern) and (b) mutate
`params.config.trace_dir = Some(<tempdir>)` AFTER building params, or the
overlay wipes it. A `session_id()` accessor on `Session` delegating to
`self.runtime.session_id()` may be added `pub(crate)` for the test.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-server descriptor`
Expected: COMPILE ERROR — `session_id`/`rewrite_descriptor_workspace` not found.

- [ ] **Step 3: Implement in `runtime.rs`**

Add to the `RuntimeState` struct (near the `trace` field, matching its
session-stable comment style):

```rust
    /// Durable session identity (4B-0): minted once here, shared with the
    /// TraceWriter, recorded in sessions/<id>/descriptor.json. The
    /// descriptor — not the trace — is what a restarted daemon indexes.
    session_id: String,
```

In `RuntimeState::new`, replace the Task-4 bridge (locate by content
`let session_id = agent_runtime_config::mint_session_id();`) with:

```rust
        let session_id = agent_runtime_config::mint_session_id();
        if let Some(root) = agent_runtime_config::sessions_root(&config) {
            let d = agent_runtime_config::SessionDescriptor {
                schema: agent_runtime_config::DESCRIPTOR_SCHEMA,
                session_id: session_id.clone(),
                workspace: workspace.clone(),
                created_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                config_path: Some(config_path.clone()),
            };
            if let Err(e) = agent_runtime_config::write_descriptor(&root, &d) {
                tracing::warn!(target: "session", error = %e,
                    "cannot write session descriptor; run will not be resumable");
            }
            agent_runtime_config::prune_session_dirs(&root, 50);
        }
        let trace = agent_runtime_config::build_trace(&config, &session_id);
```

and add `session_id` to the `Self { ... }` literal. Add the methods (near
`stats()`):

```rust
    /// Durable session identity (4B-0). Stable for the daemon's lifetime;
    /// a restarted daemon re-learns it from descriptor.json, not from us.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Rewrite descriptor.json with a new workspace (workspace switch).
    /// Resume must bind to the CURRENT workspace (spec §2.1/§3.3).
    pub fn rewrite_descriptor_workspace(&self, workspace: &std::path::Path) {
        let config = self.config.lock().unwrap().clone();
        let Some(root) = agent_runtime_config::sessions_root(&config) else {
            return;
        };
        let dir = agent_runtime_config::session_dir(&root, &self.session_id);
        let Some(mut d) = agent_runtime_config::load_descriptor(&dir) else {
            return; // construction never wrote one (warned there already)
        };
        d.workspace = workspace.to_path_buf();
        if let Err(e) = agent_runtime_config::write_descriptor(&root, &d) {
            tracing::warn!(target: "session", error = %e,
                "cannot rewrite session descriptor on workspace switch");
        }
    }
```

(If the config field on `RuntimeState` is not a `Mutex<RuntimeConfig>` as
shown in the struct at the top of the file, read the actual field and
adapt the lock line — locate by content, not by this snippet.)

- [ ] **Step 4: Implement in `session.rs`**

In `Session::set_workspace` (locate by content
`*self.workspace.lock().unwrap() = dir;`), add immediately after that line:

```rust
        self.runtime.rewrite_descriptor_workspace(&dir);
```

(`dir` is moved into the mutex on the existing line — adjust to
`*self.workspace.lock().unwrap() = dir.clone();` then pass `&dir`, or
call the rewrite BEFORE the store; either is fine, keep the borrow checker
happy with the smallest diff.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-server`
Expected: new tests PASS, all existing agent-server tests PASS.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-server/src/runtime.rs agent/crates/agent-server/src/session.rs
git commit -m "feat(server): RuntimeState owns durable session identity + descriptor (4B-0)"
```

---

### Task 6: CLI descriptor + docs line

**Files:**
- Modify: `agent/crates/agent-cli/src/main.rs`, `agent/AGENTS.md`
- Test: build + manual smoke (CLI has no session-construction unit tests;
  the descriptor logic itself is covered by Tasks 1–2)

**Interfaces:**
- Consumes: Tasks 1–2, 4.
- Note (plan-review finding 1): the CLI builds its config entirely from
  clap flags via `runtime_config_from_cli(&cli, ...)` — there is **no
  `--config` flag and no `config_path` variable** in `main.rs`. The CLI
  descriptor's `config_path` is therefore **`None` by construction**
  (flag-derived config has no file provenance). Do NOT copy Task 5's
  `config_path.is_some()` assertion to any CLI test.

- [ ] **Step 1: Replace the Task-4 bridge in `main.rs`**

Locate by content `let session_id = agent_runtime_config::mint_session_id();`
(the Task-4 bridge) and expand it to write the descriptor (the workspace
variable is whatever `--workspace` resolves to earlier in `main` — read
the surrounding code for its actual name):

```rust
    let session_id = agent_runtime_config::mint_session_id();
    if let Some(root) = agent_runtime_config::sessions_root(&rt) {
        let d = agent_runtime_config::SessionDescriptor {
            schema: agent_runtime_config::DESCRIPTOR_SCHEMA,
            session_id: session_id.clone(),
            workspace: workspace.clone(), // ← the CLI's resolved workspace variable
            created_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            config_path: None, // CLI config is flag-derived — no file provenance
        };
        if let Err(e) = agent_runtime_config::write_descriptor(&root, &d) {
            eprintln!("warning: cannot write session descriptor: {e}");
        }
        agent_runtime_config::prune_session_dirs(&root, 50);
    }
    let trace = agent_runtime_config::build_trace(&rt, &session_id);
```

The existing `trace:` eprintln line stays byte-identical.

- [ ] **Step 2: Docs line**

In `agent/AGENTS.md`, extend the sessions bullet (locate by content
"Session traces land in"):

```markdown
- Session traces land in `~/.rusty-agent/sessions/<id>.jsonl` (disable with `"trace": false`
  in the runtime config). Each session also writes
  `~/.rusty-agent/sessions/<id>/descriptor.json` — its durable identity
  (workspace + config provenance), written even when tracing is off.
```

- [ ] **Step 3: Build + smoke**

Run: `cd agent && cargo build -p agent-cli && cargo test -p agent-cli`
Expected: clean build, tests pass.

Smoke (the CLI has **no `--config` or `--trace-dir` flag** — plan-review
finding 1 — so redirect `$HOME` at a tempdir instead):

```bash
cd /home/kalen/rust-agent-runtime/agent
TMPD=$(mktemp -d)
HOME="$TMPD" cargo run -p agent-cli -- --backend openai \
  --base-url http://localhost:1 --model m1 --workspace . <<< "" || true
ls "$TMPD/.rusty-agent/sessions"
# expect: <id>.jsonl and <id>/descriptor.json with the SAME <id>
```

(Exact CLI flags: check `cargo run -p agent-cli -- --help` — the point of
the smoke is one id shared by the flat jsonl and the descriptor dir; the
model endpoint may fail after startup, which is fine.)

- [ ] **Step 4: Commit**

```bash
git add agent/crates/agent-cli/src/main.rs agent/AGENTS.md
git commit -m "feat(cli): write session descriptor at startup (4B-0)"
```

---

### Task C: Full CI + branch finish

**Files:** none (verification only)

- [ ] **Step 1: Full CI**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: ALL legs green (okf check, skills lint, fmt, clippy, cargo test
agent/, conditional src-tauri, web typecheck/vitest). `cargo fmt` runs as
part of ci.sh for `agent/` — if fmt rewrites `session_meta.rs`, commit the
formatting before proceeding.

- [ ] **Step 2: Whole-branch review + merge**

Per repo SDLC: whole-branch review, then merge `--no-ff` to main, branch
deleted, merged-tree hash verified identical to the green branch tip.
(Use superpowers:finishing-a-development-branch.)

---

## Self-review notes (author, 2026-07-10)

- **Spec coverage (4B-0 items, spec §0):** durable id owned by session ✅
  (T5/T6); per-session dir + descriptor.json with id/workspace/created/
  config provenance ✅ (T1); dirs 0o700 / files 0o600 incl. temps ✅ (T1);
  TraceWriter/CLI consume the id, `{secs}-{pid}` minting deleted ✅ (T4);
  trace naming/shape unchanged ✅ (T4 pin); startup index/scan ✅ (T2 —
  wiring into attach is 4B-1 by design); secret file ✅ (T3).
- **Not in 4B-0 (spec):** checkpoint.rs, park/resume, attribution, wire
  frames, frontends — 4B-1/4B-2.
- **Descriptor lifecycle decision recorded:** descriptor dirs prune to the
  newest 50 (mirrors trace retention) with parked dirs exempt — prevents
  unbounded accumulation from daemon restarts; parked exemption is
  forward-compatibility for 4B-1 (nothing writes parked.json yet).
- **Type consistency check:** `SessionDescriptor` field set identical in
  T1 impl, T1/T2/T5 tests, T5/T6 construction. `sessions_root` return
  `Option<PathBuf>` used with `let Some(...)` everywhere. `build_trace`
  arity matches across T4/T5/T6.
- **Plan review (2026-07-10, single opus reviewer per SDLC):
  APPROVE-WITH-FIXES — all folded.** BLOCKER: CLI has no `--config` flag /
  `config_path` variable → CLI descriptor `config_path: None` by
  construction; smoke rewritten to `HOME=$TMPD` redirection (also no
  `--trace-dir` flag). MINOR: session.rs test must use
  `crate::setup::local_params` + non-existent `config_path` (the
  `load_over` overlay would wipe an in-memory `trace_dir`); Task 5's
  `config_path.is_some()` assertion is server-only, never copy to CLI.
  ACCEPTED RESIDUAL: `sessions_root` default-branch test assertion is
  HOME-guarded (silently passes if HOME unset — CI always sets it).
  Reviewer confirmed at live source: exactly two `build_trace` callers;
  `RuntimeState.config` is `Mutex<RuntimeConfig>`; 9-param `new` arity;
  `crate::daemon::SYSTEM_PROMPT` reachable from tests; sha2 + tempfile
  deps present; no compile-order trap in the Task-4 bridge.
- **Known test-env caveat:** existing agent-server tests using
  `make_with_tools()` construct RuntimeState with `trace_dir: None` and
  already touch `$HOME/.rusty-agent/sessions` (pre-existing behavior);
  the new descriptor write follows the same path there. New tests use
  tempdir `trace_dir` overrides. Cleaning up the pre-existing pollution is
  out of scope (would be a test-hygiene sweep, not 4B-0).
