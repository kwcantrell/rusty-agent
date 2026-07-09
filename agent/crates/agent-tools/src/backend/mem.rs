//! In-process backend: session-scoped, unbounded (parity with the old
//! InMemoryOffloadStore, spec J5). Keys are mount-relative paths (E6).
use super::{Backend, Entry, FsError, GrepHit, GLOB_MAX_RESULTS, GREP_MAX_HITS};
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

#[derive(Default)]
pub struct MemBackend {
    // std Mutex: every op releases the guard before returning; never held
    // across .await (spec §5.1 implementer note).
    inner: Mutex<BTreeMap<String, String>>,
}

impl MemBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

fn norm_dir(path: &str) -> String {
    let p = path.trim_matches('/');
    if p.is_empty() {
        String::new()
    } else {
        format!("{p}/")
    }
}

#[async_trait]
impl Backend for MemBackend {
    async fn ls(&self, path: &str) -> Result<Vec<Entry>, FsError> {
        let prefix = norm_dir(path);
        let g = self.inner.lock().unwrap();
        let mut out: BTreeSet<Entry> = BTreeSet::new();
        for key in g.keys() {
            if let Some(rest) = key.strip_prefix(&prefix) {
                match rest.split_once('/') {
                    Some((dir, _)) => out.insert(Entry {
                        name: dir.into(),
                        is_dir: true,
                    }),
                    None => out.insert(Entry {
                        name: rest.into(),
                        is_dir: false,
                    }),
                };
            }
        }
        Ok(out.into_iter().collect())
    }

    async fn read(&self, path: &str) -> Result<String, FsError> {
        self.inner
            .lock()
            .unwrap()
            .get(path.trim_start_matches('/'))
            .cloned()
            .ok_or_else(|| FsError::NotFound(path.to_string()))
    }

    async fn write(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.inner.lock().unwrap().insert(
            path.trim_start_matches('/').to_string(),
            content.to_string(),
        );
        Ok(())
    }

    async fn glob(&self, pattern: &str) -> Result<Vec<String>, FsError> {
        let matcher = globset::Glob::new(pattern)
            .map_err(|e| FsError::InvalidPath(format!("bad glob pattern: {e}")))?
            .compile_matcher();
        let g = self.inner.lock().unwrap();
        Ok(g.keys()
            .filter(|k| matcher.is_match(k))
            .take(GLOB_MAX_RESULTS)
            .cloned()
            .collect())
    }

    async fn grep(&self, pattern: &str, path: Option<&str>) -> Result<Vec<GrepHit>, FsError> {
        let re = regex::Regex::new(pattern)
            .map_err(|e| FsError::InvalidPath(format!("bad regex: {e}")))?;
        let scope = path.map(|p| p.trim_start_matches('/').to_string());
        let g = self.inner.lock().unwrap();
        let mut hits = Vec::new();
        'outer: for (key, content) in g.iter() {
            if let Some(s) = &scope {
                if key != s && !key.starts_with(&norm_dir(s)) {
                    continue;
                }
            }
            for (i, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    hits.push(GrepHit {
                        path: key.clone(),
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
        self.inner
            .lock()
            .unwrap()
            .remove(path.trim_start_matches('/'))
            .map(|_| ())
            .ok_or_else(|| FsError::NotFound(path.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn mem_backend_conformance() {
        crate::backend::conformance::assert_backend_conformance(|| {
            Arc::new(MemBackend::new()) as Arc<dyn Backend>
        })
        .await;
    }

    #[tokio::test]
    async fn grep_hits_are_capped() {
        let b = MemBackend::new();
        let many: String = (0..500).map(|i| format!("needle {i}\n")).collect();
        b.write("big.txt", &many).await.unwrap();
        let hits = b.grep("needle", None).await.unwrap();
        assert_eq!(hits.len(), GREP_MAX_HITS);
    }
}
