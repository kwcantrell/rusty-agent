//! Durable session metadata: restart-stable session identity, per-session
//! descriptor dirs, and the daemon-local secret (4B-0, spec 2026-07-10 §2.1).
//! Identity is trace-independent: this module, not TraceWriter, mints ids.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// Not yet called within this task (Task 2/3 construct `SessionDescriptor`
/// values and will stamp `created_ms` from this); kept here per the brief's
/// exact interface so those tasks don't need to add it later.
#[allow(dead_code)]
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

/// 32 bytes of key material at <metadata_root>/secret, created 0o600 on
/// first use. A wrong-length file is InvalidData — never silently
/// regenerated (that would invalidate every existing checkpoint MAC).
#[allow(dead_code)]
pub fn load_or_create_secret(metadata_root: &Path) -> std::io::Result<[u8; 32]> {
    let path = metadata_root.join("secret");
    match std::fs::read(&path) {
        Ok(bytes) => bytes.as_slice().try_into().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{}: expected 32 bytes, found {}",
                    path.display(),
                    bytes.len()
                ),
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

/// All readable descriptors under root (corrupt/missing skipped), newest
/// first by session-id name order.
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

/// Keep the newest `keep` session DIRS (name-sorted); never remove a dir
/// containing checkpoint/parked.json (a parked run outlives retention).
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
    {
        std::fs::create_dir_all(dir)
    }
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
    // A crash can leave a stale tmp with pre-fix (or foreign) modes;
    // OpenOptions::mode() applies only at creation, so clear it first.
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
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
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
            "openai".into(),
            "http://localhost:8080".into(),
            "m1".into(),
            "native".into(),
            8192,
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

    #[cfg(unix)]
    #[test]
    fn stale_wide_mode_tmp_does_not_leak_into_descriptor() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let d = SessionDescriptor {
            schema: DESCRIPTOR_SCHEMA,
            session_id: "1751-0badf00d".into(),
            workspace: PathBuf::from("/w"),
            created_ms: 1,
            config_path: None,
        };
        // seed crash residue: a world-readable stale tmp
        let dir = session_dir(root.path(), &d.session_id);
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("descriptor.json.tmp");
        std::fs::write(&tmp, b"stale").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_descriptor(root.path(), &d).unwrap();
        let mode = std::fs::metadata(dir.join("descriptor.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn scan_returns_descriptors_newest_first_and_skips_corrupt() {
        let root = tempfile::tempdir().unwrap();
        for (id, ms) in [("100-aaaaaaaa", 1u64), ("200-bbbbbbbb", 2)] {
            write_descriptor(
                root.path(),
                &SessionDescriptor {
                    schema: DESCRIPTOR_SCHEMA,
                    session_id: id.into(),
                    workspace: PathBuf::from("/w"),
                    created_ms: ms,
                    config_path: None,
                },
            )
            .unwrap();
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
            write_descriptor(
                root.path(),
                &SessionDescriptor {
                    schema: DESCRIPTOR_SCHEMA,
                    session_id: id.into(),
                    workspace: PathBuf::from("/w"),
                    created_ms: 1,
                    config_path: None,
                },
            )
            .unwrap();
        }
        // oldest is parked → protected even though it would be pruned
        let parked = session_dir(root.path(), "100-aaaaaaaa").join("checkpoint");
        std::fs::create_dir_all(&parked).unwrap();
        std::fs::write(parked.join("parked.json"), b"{}").unwrap();
        // a trace .jsonl at root must be untouched by DIR pruning
        std::fs::write(root.path().join("050-eeeeeeee.jsonl"), b"{}").unwrap();
        prune_session_dirs(root.path(), 1);
        assert!(
            session_dir(root.path(), "100-aaaaaaaa").exists(),
            "parked kept"
        );
        assert!(
            !session_dir(root.path(), "200-bbbbbbbb").exists(),
            "old pruned"
        );
        assert!(
            session_dir(root.path(), "300-cccccccc").exists(),
            "newest kept"
        );
        assert!(
            root.path().join("050-eeeeeeee.jsonl").exists(),
            "jsonl untouched"
        );
    }

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
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
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
}
