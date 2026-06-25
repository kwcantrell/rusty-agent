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

    // A ':' in the path corrupts the `docker -v src:dst:mode` argument (src == dst here).
    if canon.to_string_lossy().contains(':') {
        return Err(SandboxError::InvalidMount(
            format!("path contains ':': {}", canon.display())));
    }

    // The filesystem root is rejected on exact match only (every absolute path is a
    // descendant of "/", so a prefix check there would reject everything).
    if canon == Path::new("/") {
        return Err(SandboxError::InvalidMount("refusing to mount /".into()));
    }
    // System roots whose mount (or any descendant) would breach the sandbox boundary.
    // NOTE: on modern Linux /var/run is a symlink to /run, so canonicalize() maps
    // /var/run* -> /run*; keeping both is belt-and-suspenders.
    const SYSTEM_ROOTS: &[&str] = &[
        "/etc", "/usr", "/bin", "/sbin", "/lib", "/lib64", "/boot",
        "/sys", "/proc", "/dev", "/var/lib/docker",
        "/run", "/var/run", "/var/run/docker.sock", "/run/docker.sock",
    ];
    for root in SYSTEM_ROOTS {
        let r = Path::new(root);
        if canon == r || canon.starts_with(r) {
            return Err(SandboxError::InvalidMount(format!("refusing to mount {}", canon.display())));
        }
    }

    if let Some(h) = home {
        // Canonicalize home so a symlinked $HOME prefix can't dodge the checks.
        let h_canon = h.canonicalize();
        let h_cmp = h_canon.as_deref().unwrap_or(h);
        if canon == h_cmp {
            return Err(SandboxError::InvalidMount("refusing to mount \\$HOME root".into()));
        }
        // Credential directories under HOME (and their descendants) stay off-limits,
        // while ordinary project dirs elsewhere under HOME remain allowed.
        const HOME_SECRETS: &[&str] = &[
            ".ssh", ".aws", ".gnupg", ".kube", ".docker",
            ".config/gcloud", ".netrc", ".git-credentials",
        ];
        for sub in HOME_SECRETS {
            let secret = h_cmp.join(sub);
            if canon == secret || canon.starts_with(&secret) {
                return Err(SandboxError::InvalidMount(
                    format!("refusing to mount credential path {}", canon.display())));
            }
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

    #[test]
    fn rejects_system_dirs_and_descendants() {
        assert!(validate_mount("/etc", None).is_err());
        assert!(validate_mount("/etc/ssl", None).is_err()); // descendant
        assert!(validate_mount("/usr/bin", None).is_err());
        assert!(validate_mount("/var/lib/docker", None).is_err());
    }

    #[test]
    fn rejects_home_credential_dirs() {
        let home = std::env::temp_dir().join("agent-sbx-home-creds");
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        assert!(validate_mount(ssh.to_str().unwrap(), Some(&home)).is_err());
        // a descendant of a credential dir is also rejected
        let key = ssh.join("id_rsa.d");
        std::fs::create_dir_all(&key).unwrap();
        assert!(validate_mount(key.to_str().unwrap(), Some(&home)).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn rejects_path_containing_colon() {
        let dir = std::env::temp_dir().join("agent-sbx-colon:dir");
        // canonicalize requires existence; create it if the FS allows ':' (Linux does).
        if std::fs::create_dir_all(&dir).is_ok() {
            assert!(validate_mount(dir.to_str().unwrap(), None).is_err());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn still_accepts_ordinary_project_dir_under_home() {
        let home = std::env::temp_dir().join("agent-sbx-home-ok");
        let proj = home.join("projects").join("app");
        std::fs::create_dir_all(&proj).unwrap();
        let got = validate_mount(proj.to_str().unwrap(), Some(&home)).unwrap();
        assert_eq!(got, proj.canonicalize().unwrap());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn rejects_symlinked_home_root() {
        // Create a real directory and a symlink pointing to it.
        let temp = std::env::temp_dir();
        let real_dir = temp.join("agent-sbx-real-home");
        let link_dir = temp.join("agent-sbx-symlink-home");
        let _ = std::fs::remove_dir_all(&real_dir);
        let _ = std::fs::remove_dir_all(&link_dir);
        std::fs::create_dir(&real_dir).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

        // Pass the symlink as home; validating it should be rejected as the home root.
        let err = validate_mount(link_dir.to_str().unwrap(), Some(&link_dir));
        assert!(err.is_err(), "should reject symlinked home root");

        let _ = std::fs::remove_dir_all(&real_dir);
        let _ = std::fs::remove_dir_all(&link_dir);
    }
}
