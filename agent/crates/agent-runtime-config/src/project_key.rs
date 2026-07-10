use sha2::{Digest, Sha256};
use std::path::Path;

/// Stable per-project key: SHA256 hex of the git toplevel (stable across
/// subdirs), else the canonicalized workspace root (spec §3 invariant 7).
pub fn project_key(workspace: &Path) -> String {
    let canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let root = git_toplevel(&canonical).unwrap_or(canonical);
    let mut h = Sha256::new();
    h.update(root.to_string_lossy().as_bytes());
    format!("{:x}", h.finalize())
}

fn git_toplevel(dir: &Path) -> Option<std::path::PathBuf> {
    let out = std::process::Command::new("git")
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
    #[test]
    fn stable_across_subdirs_of_a_git_repo() {
        // This repo is a git workspace: key(root) == key(root/agent).
        // CARGO_MANIFEST_DIR is agent/crates/agent-runtime-config, so three
        // levels up reaches the repo root.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let sub = root.join("agent");
        assert_eq!(project_key(&root), project_key(&sub));
        assert_eq!(project_key(&root).len(), 64);
    }
}
