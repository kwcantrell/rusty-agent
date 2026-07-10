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
}
