use crate::record::MemoryScope;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

/// Derive the project scope for a workspace: prefer the git top-level (stable across
/// subdirs), else the canonicalized workspace root; hash the path so raw filesystem
/// paths are never stored.
pub fn project_scope(workspace: &Path) -> MemoryScope {
    let canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let root = git_toplevel(&canonical).unwrap_or(canonical);
    let mut h = Sha256::new();
    h.update(root.to_string_lossy().as_bytes());
    MemoryScope::Project(format!("{:x}", h.finalize()))
}

fn git_toplevel(dir: &Path) -> Option<std::path::PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn same_key_from_repo_root_and_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert!(Command::new("git")
            .arg("-C")
            .arg(root)
            .arg("init")
            .output()
            .unwrap()
            .status
            .success());
        let sub = root.join("crates/inner");
        std::fs::create_dir_all(&sub).unwrap();
        let a = project_scope(root);
        let b = project_scope(&sub);
        assert_eq!(
            a, b,
            "subdir must map to the same project scope as the repo root"
        );
        assert!(matches!(a, MemoryScope::Project(ref k) if k.len() == 64));
    }

    #[test]
    fn non_git_dir_uses_canonical_path() {
        let tmp = tempfile::tempdir().unwrap();
        let s = project_scope(tmp.path());
        assert!(matches!(s, MemoryScope::Project(_)));
    }
}
