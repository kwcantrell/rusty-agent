//! Prefix-routed composite (spec §5.2, E6: strip on entry, re-prefix on exit —
//! mounted backends are mount-location-transparent) + the read-only guard for
//! the artifact mounts (spec §5.2: placeholders vouch for provenance; only
//! curation's privileged handle may write).
use super::{Backend, Edited, Entry, FsError, GrepHit};
use async_trait::async_trait;
use std::sync::Arc;

pub const ARTIFACTS_READONLY_MSG: &str =
    "large_tool_results/ and conversation_history/ are read-only records of offloaded context";

pub struct CompositeBackend {
    /// (prefix with trailing '/', backend), sorted longest-prefix-first.
    mounts: Vec<(String, Arc<dyn Backend>)>,
    default: Arc<dyn Backend>,
}

impl CompositeBackend {
    pub fn new(mut mounts: Vec<(String, Arc<dyn Backend>)>, default: Arc<dyn Backend>) -> Self {
        for (p, _) in &mut mounts {
            if !p.ends_with('/') {
                p.push('/');
            }
        }
        mounts.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));
        Self { mounts, default }
    }

    /// Longest-prefix route: (backend, mount prefix, stripped inner path).
    /// A path equal to a prefix minus the slash routes into that mount ("").
    fn route(&self, path: &str) -> (&Arc<dyn Backend>, Option<&str>, String) {
        let p = path.trim_start_matches('/');
        for (prefix, backend) in &self.mounts {
            if let Some(inner) = p.strip_prefix(prefix.as_str()) {
                return (backend, Some(prefix), inner.to_string());
            }
            if p == prefix.trim_end_matches('/') {
                return (backend, Some(prefix), String::new());
            }
        }
        (&self.default, None, p.to_string())
    }

    fn shadowed(&self, rel: &str) -> bool {
        self.mounts
            .iter()
            .any(|(prefix, _)| rel.starts_with(prefix.as_str()))
    }
}

#[async_trait]
impl Backend for CompositeBackend {
    async fn ls(&self, path: &str) -> Result<Vec<Entry>, FsError> {
        let (backend, prefix, inner) = self.route(path);
        if prefix.is_some() {
            return backend.ls(&inner).await;
        }
        // Default territory: real entries (minus shadowed) + synthetic mount dirs.
        let mut out = match self.default.ls(&inner).await {
            Ok(v) => v,
            // Root ls must still show mounts even if the default root is empty/odd.
            Err(FsError::NotFound(_)) if inner.is_empty() => Vec::new(),
            Err(e) => return Err(e),
        };
        let at = if inner.is_empty() {
            String::new()
        } else {
            format!("{}/", inner.trim_end_matches('/'))
        };
        for (prefix, _) in &self.mounts {
            if let Some(rest) = prefix.strip_prefix(at.as_str()) {
                let first = rest
                    .trim_end_matches('/')
                    .split('/')
                    .next()
                    .unwrap_or_default();
                if !first.is_empty() && !out.iter().any(|e| e.name == first) {
                    out.push(Entry {
                        name: first.into(),
                        is_dir: true,
                    });
                }
            }
        }
        // A real default-mount dir with a reserved name is shadowed: the
        // synthetic mount entry already covers it, so dedup by name.
        out.sort();
        out.dedup_by(|a, b| a.name == b.name);
        Ok(out)
    }

    async fn read(&self, path: &str) -> Result<String, FsError> {
        let (b, _, inner) = self.route(path);
        b.read(&inner).await
    }

    async fn write(&self, path: &str, content: &str) -> Result<(), FsError> {
        let (b, _, inner) = self.route(path);
        b.write(&inner, content).await
    }

    async fn edit(&self, path: &str, old: &str, new: &str) -> Result<Edited, FsError> {
        let (b, _, inner) = self.route(path);
        b.edit(&inner, old, new).await
    }

    async fn glob(&self, pattern: &str) -> Result<Vec<String>, FsError> {
        // Union: default results (minus shadowed) + each mount's, re-prefixed.
        let mut out: Vec<String> = self
            .default
            .glob(pattern)
            .await?
            .into_iter()
            .filter(|r| !self.shadowed(r))
            .collect();
        for (prefix, backend) in &self.mounts {
            // The pattern is workspace-scoped; strip the prefix if the pattern
            // targets this mount, else run it as-is inside the mount.
            let inner_pat = pattern.strip_prefix(prefix.as_str()).unwrap_or(pattern);
            for hit in backend.glob(inner_pat).await? {
                out.push(format!("{prefix}{hit}"));
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    async fn grep(&self, pattern: &str, path: Option<&str>) -> Result<Vec<GrepHit>, FsError> {
        // Scoped into one mount → that mount only (no cross-namespace leak).
        if let Some(scope) = path {
            let (backend, prefix, inner) = self.route(scope);
            let inner_scope = if inner.is_empty() {
                None
            } else {
                Some(inner.as_str())
            };
            let mut hits = backend.grep(pattern, inner_scope).await?;
            if let Some(prefix) = prefix {
                for h in &mut hits {
                    h.path = format!("{prefix}{}", h.path);
                }
            } else {
                hits.retain(|h| !self.shadowed(&h.path));
            }
            return Ok(hits);
        }
        // Unscoped: union default (minus shadowed) + all mounts re-prefixed.
        let mut hits: Vec<GrepHit> = self
            .default
            .grep(pattern, None)
            .await?
            .into_iter()
            .filter(|h| !self.shadowed(&h.path))
            .collect();
        for (prefix, backend) in &self.mounts {
            for mut h in backend.grep(pattern, None).await? {
                h.path = format!("{prefix}{}", h.path);
                hits.push(h);
            }
        }
        Ok(hits)
    }

    async fn delete(&self, path: &str) -> Result<(), FsError> {
        let (b, _, inner) = self.route(path);
        b.delete(&inner).await
    }
}

/// Rejects model-originated mutations of an artifact mount (spec §5.2).
/// Curation writes through the UNWRAPPED handle it owns; tools only ever see
/// this guard via the composite.
pub struct ReadOnlyToTools(pub Arc<dyn Backend>);

#[async_trait]
impl Backend for ReadOnlyToTools {
    async fn ls(&self, path: &str) -> Result<Vec<Entry>, FsError> {
        self.0.ls(path).await
    }
    async fn read(&self, path: &str) -> Result<String, FsError> {
        self.0.read(path).await
    }
    async fn write(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Err(FsError::Denied(ARTIFACTS_READONLY_MSG.into()))
    }
    async fn edit(&self, _path: &str, _old: &str, _new: &str) -> Result<Edited, FsError> {
        Err(FsError::Denied(ARTIFACTS_READONLY_MSG.into()))
    }
    async fn glob(&self, pattern: &str) -> Result<Vec<String>, FsError> {
        self.0.glob(pattern).await
    }
    async fn grep(&self, pattern: &str, path: Option<&str>) -> Result<Vec<GrepHit>, FsError> {
        self.0.grep(pattern, path).await
    }
    async fn delete(&self, _path: &str) -> Result<(), FsError> {
        Err(FsError::Denied(ARTIFACTS_READONLY_MSG.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemBackend;

    fn two_mounts() -> CompositeBackend {
        CompositeBackend::new(
            vec![
                (
                    "large_tool_results/".into(),
                    Arc::new(MemBackend::new()) as Arc<dyn Backend>,
                ),
                (
                    "conversation_history/".into(),
                    Arc::new(MemBackend::new()) as Arc<dyn Backend>,
                ),
            ],
            Arc::new(MemBackend::new()),
        )
    }

    #[tokio::test]
    async fn routes_by_longest_prefix_and_strips() {
        let c = two_mounts();
        c.write("large_tool_results/1-call", "payload")
            .await
            .unwrap();
        // The mount saw the stripped key: reading through the mount directly
        // (transparency) — reach it via the composite's own route.
        assert_eq!(
            c.read("large_tool_results/1-call").await.unwrap(),
            "payload"
        );
        // Default mount is untouched.
        assert!(matches!(c.read("1-call").await, Err(FsError::NotFound(_))));
    }

    #[tokio::test]
    async fn mount_location_transparency() {
        // The SAME backend mounted at two different prefixes behaves identically.
        let shared: Arc<dyn Backend> = Arc::new(MemBackend::new());
        let a = CompositeBackend::new(
            vec![("x/".into(), shared.clone())],
            Arc::new(MemBackend::new()),
        );
        let b = CompositeBackend::new(
            vec![("y/z/".into(), shared.clone())],
            Arc::new(MemBackend::new()),
        );
        a.write("x/f.txt", "one").await.unwrap();
        assert_eq!(b.read("y/z/f.txt").await.unwrap(), "one");
    }

    #[tokio::test]
    async fn grep_scoped_to_one_prefix_never_leaks_the_other() {
        let c = two_mounts();
        c.write("large_tool_results/a", "needle in results")
            .await
            .unwrap();
        c.write("conversation_history/history.md", "needle in history")
            .await
            .unwrap();
        let hits = c.grep("needle", Some("large_tool_results/")).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].path.starts_with("large_tool_results/"),
            "{}",
            hits[0].path
        );
    }

    #[tokio::test]
    async fn unscoped_grep_aggregates_and_reprefixes() {
        let c = two_mounts();
        c.write("large_tool_results/a", "needle").await.unwrap();
        c.write("workspace.txt", "needle").await.unwrap();
        let mut paths: Vec<String> = c
            .grep("needle", None)
            .await
            .unwrap()
            .into_iter()
            .map(|h| h.path)
            .collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "large_tool_results/a".to_string(),
                "workspace.txt".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn root_ls_shows_mounts_as_dirs() {
        let c = two_mounts();
        c.write("large_tool_results/a", "x").await.unwrap();
        let names: Vec<String> = c
            .ls("")
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.is_dir)
            .map(|e| e.name)
            .collect();
        assert!(names.contains(&"large_tool_results".to_string()));
        assert!(names.contains(&"conversation_history".to_string()));
    }

    #[tokio::test]
    async fn guard_denies_mutations_and_passes_reads() {
        let inner: Arc<dyn Backend> = Arc::new(MemBackend::new());
        inner.write("a", "original").await.unwrap();
        let g = ReadOnlyToTools(inner.clone());
        for result in [
            g.write("a", "forged").await.err(),
            g.edit("a", "original", "forged").await.err(),
            g.delete("a").await.err(),
        ] {
            match result {
                Some(FsError::Denied(msg)) => assert_eq!(msg, ARTIFACTS_READONLY_MSG),
                other => panic!("expected Denied, got {other:?}"),
            }
        }
        // Bytes intact after denied overwrite attempts (spec §7 guard pin).
        assert_eq!(g.read("a").await.unwrap(), "original");
    }
}
