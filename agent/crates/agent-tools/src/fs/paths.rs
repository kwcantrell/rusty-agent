use crate::ToolError;
use std::path::{Path, PathBuf};

/// Resolve `arg` against `workspace`, rejecting anything that escapes it.
/// Works for non-existent files (for writes) by normalizing lexically.
pub fn resolve_in_workspace(workspace: &Path, arg: &str) -> Result<PathBuf, ToolError> {
    let candidate = if Path::new(arg).is_absolute() {
        PathBuf::from(arg)
    } else {
        workspace.join(arg)
    };
    let normalized = normalize(&candidate);
    let ws_norm = normalize(workspace);
    if normalized.starts_with(&ws_norm) {
        Ok(normalized)
    } else {
        Err(ToolError::Denied(format!("path escapes workspace: {arg}")))
    }
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
        assert!(p.starts_with(ws));
    }

    #[test]
    fn rejects_escape_with_dotdot() {
        let dir = tempdir().unwrap();
        let ws = dir.path();
        let err = resolve_in_workspace(ws, "../escape.txt").unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)));
    }
}
