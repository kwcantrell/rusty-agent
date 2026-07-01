use crate::ToolError;
use std::path::{Path, PathBuf};

/// Resolve `arg` against `workspace`, rejecting anything that escapes it.
/// Works for non-existent files (for writes) by canonicalizing the longest
/// existing ancestor and re-attaching the not-yet-created trailing components.
///
/// # Symlink handling
///
/// The containment check runs against **symlink-resolved** paths: the deepest
/// existing ancestor is passed through [`std::fs::canonicalize`], so an
/// in-workspace symlink whose target points outside the workspace resolves to
/// that outside target *before* the prefix check and is rejected. Dangling
/// symlinks (whose target does not exist yet) are chased via [`std::fs::read_link`]
/// so a write cannot be smuggled through the link either.
///
/// This is defense-in-depth, not a full sandbox: OS-level isolation (`seccomp`,
/// `landlock`, or a container) remains the authoritative boundary — see
/// `docs/superpowers/context/os-sandboxing.md`. But the lexical-only escape the
/// prior version left open is now closed.
pub fn resolve_in_workspace(workspace: &Path, arg: &str) -> Result<PathBuf, ToolError> {
    let candidate = if Path::new(arg).is_absolute() {
        PathBuf::from(arg)
    } else {
        workspace.join(arg)
    };
    // Compare against the real (symlink-resolved) workspace root so the prefix
    // check is consistent even when the workspace itself sits under a symlink
    // (e.g. macOS `/tmp` -> `/private/tmp`).
    let ws_real = std::fs::canonicalize(workspace).unwrap_or_else(|_| normalize(workspace));
    let resolved = resolve_symlinks(&normalize(&candidate));
    if resolved.starts_with(&ws_real) {
        Ok(resolved)
    } else {
        Err(ToolError::Denied(format!("path escapes workspace: {arg}")))
    }
}

/// Canonicalize the deepest existing ancestor of `p`, then re-append the
/// components that do not exist yet (needed so writes can create new files).
/// Symlinks in existing components are resolved by `canonicalize`; a dangling
/// symlink (lstat-present but canonicalize-fails) is chased via its link target
/// so its destination — not the in-workspace link location — is what the caller
/// checks for containment. Bounded by `MAX_HOPS` to defuse symlink cycles.
fn resolve_symlinks(p: &Path) -> PathBuf {
    use std::ffi::OsString;
    const MAX_HOPS: usize = 40;
    let mut tail: Vec<OsString> = Vec::new();
    let mut cur = p.to_path_buf();
    for _ in 0..MAX_HOPS {
        if let Ok(real) = std::fs::canonicalize(&cur) {
            let mut base = real;
            for name in tail.iter().rev() {
                base.push(name);
            }
            return base;
        }
        // canonicalize failed: `cur` is either genuinely absent or a dangling
        // symlink. If it's a dangling symlink, follow its target so a write
        // through it is checked against where it actually lands.
        if let Ok(meta) = std::fs::symlink_metadata(&cur) {
            if meta.file_type().is_symlink() {
                if let Ok(target) = std::fs::read_link(&cur) {
                    let parent = cur.parent().map(Path::to_path_buf).unwrap_or_default();
                    cur = if target.is_absolute() {
                        normalize(&target)
                    } else {
                        normalize(&parent.join(target))
                    };
                    continue;
                }
            }
        }
        match cur.file_name() {
            Some(name) => {
                tail.push(name.to_os_string());
                match cur.parent() {
                    Some(parent) => cur = parent.to_path_buf(),
                    None => break,
                }
            }
            None => break,
        }
    }
    // Nothing along the path could be canonicalized (or the hop budget ran out):
    // fall back to the lexical form so the containment check still runs.
    p.to_path_buf()
}

/// Lexical normalization that collapses `.` and `..` without touching the filesystem.
fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => { out.pop(); }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolves_relative_inside_workspace() {
        let dir = tempdir().unwrap();
        let ws = dir.path();
        std::fs::write(ws.join("a.txt"), "hi").unwrap();
        let p = resolve_in_workspace(ws, "a.txt").unwrap();
        // Returned path is now canonical; compare against the canonical root.
        let ws_real = std::fs::canonicalize(ws).unwrap();
        assert!(p.starts_with(&ws_real));
    }

    #[test]
    fn allows_new_file_for_write() {
        // A not-yet-existent target must still resolve (writes create it).
        let dir = tempdir().unwrap();
        let ws = dir.path();
        let p = resolve_in_workspace(ws, "sub/new.txt").unwrap();
        let ws_real = std::fs::canonicalize(ws).unwrap();
        assert!(p.starts_with(&ws_real));
        assert!(p.ends_with("sub/new.txt"));
    }

    #[test]
    fn rejects_escape_with_dotdot() {
        let dir = tempdir().unwrap();
        let ws = dir.path();
        let err = resolve_in_workspace(ws, "../escape.txt").unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_read_through_existing_symlink_escape() {
        // An in-workspace symlink pointing OUTSIDE the workspace must not let a
        // read reach the target — the lexical-only guard used to allow this.
        let outside = tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "top secret").unwrap();
        let dir = tempdir().unwrap();
        let ws = dir.path();
        std::os::unix::fs::symlink(outside.path(), ws.join("escape")).unwrap();

        let err = resolve_in_workspace(ws, "escape/secret.txt").unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)),
            "symlink to an outside dir must be rejected");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_write_through_dangling_symlink_escape() {
        // A symlink to a not-yet-existent OUTSIDE path is still a write escape:
        // std::fs::write would follow it. It must be rejected.
        let outside = tempdir().unwrap();
        let dir = tempdir().unwrap();
        let ws = dir.path();
        let target = outside.path().join("planted.txt"); // does not exist yet
        std::os::unix::fs::symlink(&target, ws.join("escape")).unwrap();

        let err = resolve_in_workspace(ws, "escape").unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)),
            "dangling symlink to an outside target must be rejected");
    }

    #[cfg(unix)]
    #[test]
    fn allows_symlink_that_stays_inside_workspace() {
        // An in-workspace symlink whose target is also inside must still work.
        let dir = tempdir().unwrap();
        let ws = dir.path();
        std::fs::create_dir(ws.join("real")).unwrap();
        std::fs::write(ws.join("real/data.txt"), "ok").unwrap();
        std::os::unix::fs::symlink(ws.join("real"), ws.join("link")).unwrap();

        let p = resolve_in_workspace(ws, "link/data.txt").unwrap();
        let ws_real = std::fs::canonicalize(ws).unwrap();
        assert!(p.starts_with(&ws_real));
    }
}
