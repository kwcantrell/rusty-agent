use std::path::{Component, Path, PathBuf};

/// Resolve `rel` against `base_dir`, rejecting absolute paths and any `..`
/// escape. Lexical only — does not resolve symlinks (same limitation as the
/// workspace guard in `agent-tools`).
pub fn resolve_in_dir(base_dir: &Path, rel: &str) -> Result<PathBuf, String> {
    if Path::new(rel).is_absolute() {
        return Err(format!(
            "path must be relative to the skill directory: {rel}"
        ));
    }
    let candidate = base_dir.join(rel);
    let normalized = normalize(&candidate);
    let base_norm = normalize(base_dir);
    if base_norm.as_os_str().is_empty() {
        return Err(format!("invalid base directory: {}", base_dir.display()));
    }
    if normalized.starts_with(&base_norm) {
        Ok(normalized)
    } else {
        Err(format!("path escapes the skill directory: {rel}"))
    }
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn resolves_relative_inside_dir() {
        let base = PathBuf::from("/skills/foo");
        let p = resolve_in_dir(&base, "scripts/run.py").unwrap();
        assert_eq!(p, PathBuf::from("/skills/foo/scripts/run.py"));
    }

    #[test]
    fn rejects_absolute_path() {
        let base = PathBuf::from("/skills/foo");
        assert!(resolve_in_dir(&base, "/etc/passwd").is_err());
    }

    #[test]
    fn rejects_dotdot_escape() {
        let base = PathBuf::from("/skills/foo");
        let err = resolve_in_dir(&base, "../bar/secret").unwrap_err();
        assert!(err.contains("escapes"));
    }

    #[test]
    fn rejects_empty_base_dir() {
        let err = resolve_in_dir(Path::new(""), "anything").unwrap_err();
        assert!(err.contains("invalid base"));
    }
}
