//! The virtual-filesystem seam (spec: docs/superpowers/specs/2026-07-08-backend-seam-design.md §5.1).
//! One trait behind every file tool; backends are mount-location-transparent:
//! they always receive paths relative to their own root (the composite strips
//! the mount prefix on the way in and re-prefixes results on the way out, E6).
// mod composite; // Task 3
// mod host; // Task 2
pub mod conformance;
mod mem;
// pub use composite::{CompositeBackend, ReadOnlyToTools, ARTIFACTS_READONLY_MSG}; // Task 3
// pub use host::HostBackend; // Task 2
pub use mem::MemBackend;

use async_trait::async_trait;

/// Grep result-set cap (spec §5.4: "result-capped").
pub const GREP_MAX_HITS: usize = 200;
/// Glob result-set cap.
pub const GLOB_MAX_RESULTS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone)]
pub struct Edited {
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrepHit {
    pub path: String,
    /// 1-based line number.
    pub line: usize,
    pub text: String,
}

/// Structured backend errors (spec §5.1). `Unsupported` was cut at the gate;
/// tools map these to `ToolError` in one place, preserving pinned strings.
#[derive(Debug, Clone, thiserror::Error)]
pub enum FsError {
    #[error("not found: {0}")]
    NotFound(String),
    /// Containment violation OR a model-originated mutation of the read-only
    /// artifacts namespace.
    #[error("denied: {0}")]
    Denied(String),
    /// Exists but is not valid UTF-8 (binary) — the honest error the old
    /// read_to_string path mislabeled as NotFound (spec J3).
    #[error("not utf-8: {0}")]
    NotUtf8(String),
    /// `edit` old-string matched 0 or >1 times (count carried in message).
    #[error("edit conflict: {0}")]
    EditConflict(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("io: {0}")]
    Io(String),
}

/// The backend seam. Paths are backend-root-relative virtual paths; only
/// `HostBackend` touches the OS path type, so an execute-derived sandbox
/// backend stays implementable (spec J2). This trait's conformance suite
/// (`conformance::assert_backend_conformance`) is the PUBLIC acceptance test
/// for custom backends (spec J2 gate amendment).
#[async_trait]
pub trait Backend: Send + Sync {
    /// Entries directly under `path`, name-sorted.
    async fn ls(&self, path: &str) -> Result<Vec<Entry>, FsError>;
    /// Whole-document read.
    async fn read(&self, path: &str) -> Result<String, FsError>;
    /// Create or overwrite, creating parents.
    async fn write(&self, path: &str, content: &str) -> Result<(), FsError>;
    /// Replace `old` (must occur exactly once) with `new`. Returns
    /// before/after so tools can render diffs. Default: read → uniqueness →
    /// replacen → write; backends may override.
    async fn edit(&self, path: &str, old: &str, new: &str) -> Result<Edited, FsError> {
        let before = self.read(path).await?;
        let count = before.matches(old).count();
        if count != 1 {
            return Err(FsError::EditConflict(format!(
                "`old` matched {count} times; must match exactly once"
            )));
        }
        let after = before.replacen(old, new, 1);
        self.write(path, &after).await?;
        Ok(Edited { before, after })
    }
    /// Paths matching a glob pattern, capped at `GLOB_MAX_RESULTS`.
    /// No agent-facing tool in Phase 2 (spec J8).
    async fn glob(&self, pattern: &str) -> Result<Vec<String>, FsError>;
    /// Regex search. `path` scopes to a file or prefix; None = everywhere.
    /// Capped at `GREP_MAX_HITS`.
    async fn grep(&self, pattern: &str, path: Option<&str>) -> Result<Vec<GrepHit>, FsError>;
    /// No agent-facing tool in Phase 2 (spec J8).
    async fn delete(&self, path: &str) -> Result<(), FsError>;
}

/// Path-component sanitizer for artifact names (spec §5.5): keep
/// `[A-Za-z0-9._-]`, replace everything else with '-'; empty → "result".
pub fn sanitize_component(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if out.is_empty() {
        "result".into()
    } else {
        out
    }
}
