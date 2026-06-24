use agent_tools::SandboxError;
use std::path::{Path, PathBuf};

pub fn validate_mount(path: &str, home: Option<&Path>) -> Result<PathBuf, SandboxError> {
    let expanded: PathBuf = if let Some(rest) = path.strip_prefix("~/") {
        match home { Some(h) => h.join(rest),
            None => return Err(SandboxError::InvalidMount(format!("~ unsupported: {path}"))) }
    } else if path == "~" {
        match home { Some(h) => h.to_path_buf(),
            None => return Err(SandboxError::InvalidMount(format!("~ unsupported: {path}"))) }
    } else { PathBuf::from(path) };

    let canon = expanded.canonicalize()
        .map_err(|e| SandboxError::InvalidMount(format!("{}: {e}", expanded.display())))?;

    if canon == Path::new("/") {
        return Err(SandboxError::InvalidMount("refusing to mount /".into()));
    }
    if let Some(h) = home {
        if canon == h {
            return Err(SandboxError::InvalidMount("refusing to mount \\$HOME root".into()));
        }
    }
    for bad in ["/var/run/docker.sock", "/run/docker.sock", "/var/run", "/run"] {
        if canon == Path::new(bad) {
            return Err(SandboxError::InvalidMount(format!("refusing to mount {bad}")));
        }
    }
    Ok(canon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_root_and_home_root_and_socket() {
        assert!(validate_mount("/", None).is_err());
        let home = std::env::temp_dir();
        assert!(validate_mount(home.to_str().unwrap(), Some(&home)).is_err());
        assert!(validate_mount("/var/run/docker.sock", None).is_err());
    }

    #[test]
    fn accepts_a_real_subdir() {
        let dir = std::env::temp_dir();
        let sub = dir.join("agent-sbx-mount-test");
        std::fs::create_dir_all(&sub).unwrap();
        let got = validate_mount(sub.to_str().unwrap(), Some(&dir)).unwrap();
        assert_eq!(got, sub.canonicalize().unwrap());
    }

    #[test]
    fn expands_tilde_subdir() {
        let home = std::env::temp_dir();
        let sub = home.join("agent-sbx-tilde");
        std::fs::create_dir_all(&sub).unwrap();
        let got = validate_mount("~/agent-sbx-tilde", Some(&home)).unwrap();
        assert_eq!(got, sub.canonicalize().unwrap());
    }
}
