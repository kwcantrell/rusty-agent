//! Real-disk backend rooted at the workspace: today's file-tool behavior
//! relocated (spec §5.2). Containment via resolve_in_workspace — symlink
//! chasing, dangling-link rejection — its test suite keeps passing unchanged.
use super::{Backend, Entry, FsError, GrepHit, GLOB_MAX_RESULTS, GREP_MAX_HITS};
use crate::fs::resolve_in_workspace;
use crate::ToolError;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct HostBackend {
    root: PathBuf,
}

impl HostBackend {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn resolve(&self, path: &str) -> Result<PathBuf, FsError> {
        resolve_in_workspace(&self.root, path).map_err(|e| match e {
            ToolError::Denied(msg) => FsError::Denied(msg),
            other => FsError::InvalidPath(other.to_string()),
        })
    }

    /// Walk skip-set is exactly `.git/` (spec §5.2); reserved-prefix artifacts
    /// never reach this walker (they live on MemBackend mounts).
    fn walk(&self) -> impl Iterator<Item = walkdir::DirEntry> {
        walkdir::WalkDir::new(&self.root)
            .into_iter()
            .filter_entry(|e| e.file_name() != ".git")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
    }

    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned()
    }
}

#[async_trait]
impl Backend for HostBackend {
    async fn ls(&self, path: &str) -> Result<Vec<Entry>, FsError> {
        let full = self.resolve(path)?;
        let mut rd = tokio::fs::read_dir(&full)
            .await
            .map_err(|e| FsError::NotFound(format!("{path}: {e}")))?;
        let mut out = Vec::new();
        while let Some(e) = rd
            .next_entry()
            .await
            .map_err(|e| FsError::Io(e.to_string()))?
        {
            let is_dir = e.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            out.push(Entry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir,
            });
        }
        out.sort();
        Ok(out)
    }

    async fn read(&self, path: &str) -> Result<String, FsError> {
        let full = self.resolve(path)?;
        let bytes = tokio::fs::read(&full)
            .await
            .map_err(|e| FsError::NotFound(format!("{path}: {e}")))?;
        String::from_utf8(bytes)
            .map_err(|_| FsError::NotUtf8(format!("{path}: is not valid UTF-8")))
    }

    async fn write(&self, path: &str, content: &str) -> Result<(), FsError> {
        let full = self.resolve(path)?;
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| FsError::Io(e.to_string()))?;
        }
        // Atomic replace: same-dir temp + rename, so readers see old or new
        // complete content, never a truncated file (spec §2.4 atomic writes).
        // The temp name APPENDS to (not replaces) the full original file
        // name, so distinct targets sharing a stem (notes.md vs notes.txt)
        // never collide on the same temp path — each embeds its own full
        // name. Concurrent writes to the *same* path still race on rename
        // order; readers see old-or-new complete content only (accepted
        // residual, not a torn read).
        let mut tmp_name = full
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        tmp_name.push(format!(
            ".tmp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let tmp = full.with_file_name(tmp_name);
        tokio::fs::write(&tmp, content)
            .await
            .map_err(|e| FsError::Io(e.to_string()))?;
        tokio::fs::rename(&tmp, &full)
            .await
            .map_err(|e| FsError::Io(e.to_string()))
    }

    async fn glob(&self, pattern: &str) -> Result<Vec<String>, FsError> {
        let matcher = globset::Glob::new(pattern)
            .map_err(|e| FsError::InvalidPath(format!("bad glob pattern: {e}")))?
            .compile_matcher();
        let mut out: Vec<String> = self
            .walk()
            .map(|e| self.rel(e.path()))
            .filter(|r| matcher.is_match(r))
            .take(GLOB_MAX_RESULTS)
            .collect();
        out.sort();
        Ok(out)
    }

    async fn grep(&self, pattern: &str, path: Option<&str>) -> Result<Vec<GrepHit>, FsError> {
        let re = regex::Regex::new(pattern)
            .map_err(|e| FsError::InvalidPath(format!("bad regex: {e}")))?;
        let files: Vec<PathBuf> = match path {
            Some(p) => {
                let full = self.resolve(p)?;
                if full.is_dir() {
                    walkdir::WalkDir::new(&full)
                        .into_iter()
                        .filter_entry(|e| e.file_name() != ".git")
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_type().is_file())
                        .map(|e| e.into_path())
                        .collect()
                } else {
                    vec![full]
                }
            }
            None => self.walk().map(|e| e.into_path()).collect(),
        };
        let mut hits = Vec::new();
        'outer: for f in files {
            // Binary files are silently skipped in search (not an error).
            let Ok(content) = std::fs::read_to_string(&f) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    hits.push(GrepHit {
                        path: self.rel(&f),
                        line: i + 1,
                        text: line.to_string(),
                    });
                    if hits.len() >= GREP_MAX_HITS {
                        break 'outer;
                    }
                }
            }
        }
        Ok(hits)
    }

    async fn delete(&self, path: &str) -> Result<(), FsError> {
        let full = self.resolve(path)?;
        tokio::fs::remove_file(&full)
            .await
            .map_err(|e| FsError::NotFound(format!("{path}: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[tokio::test]
    async fn host_backend_conformance() {
        // Leak tempdirs for the closure's 'static lifetime (test-only).
        crate::backend::conformance::assert_backend_conformance(|| {
            let dir = Box::leak(Box::new(tempdir().unwrap()));
            Arc::new(HostBackend::new(dir.path().to_path_buf())) as Arc<dyn Backend>
        })
        .await;
    }

    #[tokio::test]
    async fn read_of_binary_file_is_not_utf8() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("bin"), [0xFF, 0xFE, 0x00, 0x01]).unwrap();
        let b = HostBackend::new(dir.path().to_path_buf());
        assert!(matches!(b.read("bin").await, Err(FsError::NotUtf8(_))));
    }

    #[tokio::test]
    async fn escape_is_denied_with_todays_message() {
        let dir = tempdir().unwrap();
        let b = HostBackend::new(dir.path().to_path_buf());
        match b.read("../escape.txt").await {
            Err(FsError::Denied(msg)) => assert!(msg.contains("path escapes workspace")),
            other => panic!("expected Denied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn walker_skips_git_dir() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/secret"), "needle").unwrap();
        std::fs::write(dir.path().join("real.txt"), "needle").unwrap();
        let b = HostBackend::new(dir.path().to_path_buf());
        let hits = b.grep("needle", None).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "real.txt");
    }

    #[tokio::test]
    async fn write_is_atomic_no_torn_reads() {
        let tmp = tempfile::tempdir().unwrap();
        let be = std::sync::Arc::new(HostBackend::new(tmp.path().to_path_buf()));
        be.write("f.txt", &"A".repeat(1 << 20)).await.unwrap();
        let w = {
            let be = be.clone();
            tokio::spawn(async move {
                for _ in 0..50 {
                    be.write("f.txt", &"B".repeat(1 << 20)).await.unwrap();
                }
            })
        };
        for _ in 0..200 {
            let s = be.read("f.txt").await.unwrap();
            // Old or new complete content only — never empty/partial.
            assert!(s.len() == 1 << 20, "torn read: len {}", s.len());
        }
        w.await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_same_stem_writes_do_not_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let be = std::sync::Arc::new(HostBackend::new(tmp.path().to_path_buf()));
        let a = {
            let be = be.clone();
            tokio::spawn(async move {
                for _ in 0..50 {
                    be.write("notes.md", &"M".repeat(64 * 1024)).await.unwrap();
                }
            })
        };
        let b = {
            let be = be.clone();
            tokio::spawn(async move {
                for _ in 0..50 {
                    be.write("notes.txt", &"T".repeat(64 * 1024)).await.unwrap();
                }
            })
        };
        a.await.unwrap();
        b.await.unwrap();
        assert_eq!(be.read("notes.md").await.unwrap(), "M".repeat(64 * 1024));
        assert_eq!(be.read("notes.txt").await.unwrap(), "T".repeat(64 * 1024));
    }
}
