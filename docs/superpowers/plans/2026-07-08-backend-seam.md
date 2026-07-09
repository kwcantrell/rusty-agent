# Backend Seam / Virtual Filesystem (Phase 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the gate-approved backend seam: a `Backend` trait with Host/Mem/Composite implementations, file tools migrated onto it, and the offload/eviction substrate moved from the id-keyed `OffloadStore` to `large_tool_results/` + `conversation_history/` files, retiring `context_recall`.

**Architecture:** Two waves per spec §5.10. Wave 1 (Tasks 1–5) lands the trait + three backends + tool adapters with **zero behavior change** — the whole existing suite passes with construction-site-only diffs. Wave 2 (Tasks 6–12) carries every declared change: `read_file` paging contract, `grep` tool, curation-writes-files, retirement + consumer sweep, wire change, eval-harness migration.

**Tech Stack:** Rust (async-trait, tokio), new deps `regex`, `globset`, `walkdir` in `agent-tools`; TypeScript (web explorer render line).

**Spec:** `docs/superpowers/specs/2026-07-08-backend-seam-design.md` (APPROVED at gate 2026-07-08, E1–E6 resolved). Section references below (§N) point there.

## Global Constraints

- **Anchor drift rule:** every `file:line` in this plan is orientation only — locate the quoted code by content before editing (AGENTS.md).
- **Wave-1 parity bar:** after each of Tasks 1–5, `cargo test` in `agent/` is green with **assertion bodies unchanged** (construction sites only may be touched).
- **Wave-2 superseded pins:** only the tests enumerated in spec §7 "Superseded pins" may change assertion bodies, and only as this plan directs.
- **Two Cargo workspaces:** all work is in `agent/`; `cargo -p <crate>` from `agent/`. Never touch `src-tauri/`.
- **Conventional commits** (`type(scope): summary`); commit after every task step marked Commit; never push.
- **Preserved literal strings** (tests pin them): `"path escapes workspace: {arg}"`, ``"`old` matched {count} times; must match exactly to once"`` — exactly as they appear today in `fs/paths.rs` and `fs/write.rs` (copy from live source, not from this plan).
- **Read-only artifacts guard message** (spec §5.2, verbatim): `large_tool_results/ and conversation_history/ are read-only records of offloaded context`.
- **Placeholder skip literals** (spec §5.5, exactly two): `[tool_result offloaded` and `[tool_result truncated`.
- **Reserved mount prefixes:** `large_tool_results/` and `conversation_history/` (trailing slash in mount table).
- Model-facing strings always carry **full virtual paths**; backend keys are **mount-relative** (spec E6).
- Final gate: `bash scripts/ci.sh` green (Task 12).

## File Structure (what exists where when done)

```
agent/crates/agent-tools/src/backend/mod.rs        # trait Backend, Entry/Edited/GrepHit, FsError, caps, sanitize helper
agent/crates/agent-tools/src/backend/mem.rs        # MemBackend
agent/crates/agent-tools/src/backend/host.rs       # HostBackend (resolve_in_workspace containment)
agent/crates/agent-tools/src/backend/composite.rs  # CompositeBackend (strip+remap) + ReadOnlyToTools
agent/crates/agent-tools/src/backend/conformance.rs# public conformance suite (spec J2)
agent/crates/agent-tools/src/fs/read.rs            # ReadFile (adapter + paging), ListDirectory (adapter)
agent/crates/agent-tools/src/fs/write.rs           # WriteFile/EditFile (adapters)
agent/crates/agent-tools/src/fs/search.rs          # GrepTool (new)
agent/crates/agent-core/src/artifacts.rs           # SessionArtifacts (new)
agent/crates/agent-core/src/offload_policy.rs      # path-grammar placeholders, OffloadEntry w/o id, selectors
agent/crates/agent-core/src/curated.rs             # curation writes files; pointer; ledger sections
agent/crates/agent-core/src/context_tools.rs       # ContextCompactTool only
agent/crates/agent-core/src/offload.rs             # DELETED (Task 8)
```

---

## WAVE 1 — the seam, zero behavior change

### Task 1: Backend trait, FsError, MemBackend, conformance suite

**Files:**
- Create: `agent/crates/agent-tools/src/backend/mod.rs`
- Create: `agent/crates/agent-tools/src/backend/mem.rs`
- Create: `agent/crates/agent-tools/src/backend/conformance.rs`
- Modify: `agent/crates/agent-tools/src/lib.rs` (add `pub mod backend;` after `pub mod fs;`)
- Modify: `agent/Cargo.toml` (workspace deps) + `agent/crates/agent-tools/Cargo.toml`

**Interfaces:**
- Produces: `agent_tools::backend::{Backend, Entry, Edited, GrepHit, FsError, MemBackend, GREP_MAX_HITS, GLOB_MAX_RESULTS}` and `agent_tools::backend::conformance::assert_backend_conformance`. Every later task consumes these exact names.

- [ ] **Step 1: Add dependencies**

In `agent/Cargo.toml` under `[workspace.dependencies]` add (alphabetical placement):

```toml
globset = "0.4"
regex = "1"
walkdir = "2"
```

In `agent/crates/agent-tools/Cargo.toml` under `[dependencies]` add:

```toml
globset.workspace = true
regex.workspace = true
walkdir.workspace = true
```

- [ ] **Step 2: Write the trait module with failing-to-compile consumers deferred**

Create `agent/crates/agent-tools/src/backend/mod.rs`:

```rust
//! The virtual-filesystem seam (spec: docs/superpowers/specs/2026-07-08-backend-seam-design.md §5.1).
//! One trait behind every file tool; backends are mount-location-transparent:
//! they always receive paths relative to their own root (the composite strips
//! the mount prefix on the way in and re-prefixes results on the way out, E6).
mod composite;
mod host;
mod mem;
pub mod conformance;
pub use composite::{CompositeBackend, ReadOnlyToTools, ARTIFACTS_READONLY_MSG};
pub use host::HostBackend;
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
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' })
        .collect();
    if out.is_empty() { "result".into() } else { out }
}
```

For this task to compile alone, create empty stub files `host.rs` and `composite.rs` containing only `// Task 2 / Task 3` plus the items `mod.rs` re-exports as `todo!()`-free stubs — **do not do this**. Instead: comment out the `mod composite; mod host;` lines and their `pub use` lines in this task, and uncomment them in Tasks 2/3 respectively. (Keeps every task compiling.)

- [ ] **Step 3: Write MemBackend + its failing test run**

Create `agent/crates/agent-tools/src/backend/mem.rs`:

```rust
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
    if p.is_empty() { String::new() } else { format!("{p}/") }
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
                    Some((dir, _)) => out.insert(Entry { name: dir.into(), is_dir: true }),
                    None => out.insert(Entry { name: rest.into(), is_dir: false }),
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
        self.inner
            .lock()
            .unwrap()
            .insert(path.trim_start_matches('/').to_string(), content.to_string());
        Ok(())
    }

    async fn glob(&self, pattern: &str) -> Result<Vec<String>, FsError> {
        let matcher = globset::Glob::new(pattern)
            .map_err(|e| FsError::InvalidPath(format!("bad glob pattern: {e}")))?
            .compile_matcher();
        let g = self.inner.lock().unwrap();
        Ok(g.keys().filter(|k| matcher.is_match(k)).take(GLOB_MAX_RESULTS).cloned().collect())
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
                    hits.push(GrepHit { path: key.clone(), line: i + 1, text: line.to_string() });
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
```

(`Entry` already derives `Ord` in Task 1's `mod.rs`, which the `BTreeSet` here requires.)

- [ ] **Step 4: Write the public conformance suite**

Create `agent/crates/agent-tools/src/backend/conformance.rs`:

```rust
//! PUBLIC conformance surface (spec J2 gate amendment): the acceptance test a
//! custom-backend author runs against their implementation, generic over
//! `Arc<dyn Backend>`. Kept dependency-light so external crates can call it
//! from their own test harnesses.
use super::{Backend, FsError};
use std::sync::Arc;

/// Run the full behavioral contract against a fresh backend. `fresh` must
/// return an empty backend each call.
pub async fn assert_backend_conformance<F>(fresh: F)
where
    F: Fn() -> Arc<dyn Backend>,
{
    // write → read round trip, parents auto-created
    let b = fresh();
    b.write("dir/sub/a.txt", "alpha").await.expect("write");
    assert_eq!(b.read("dir/sub/a.txt").await.expect("read"), "alpha");

    // read missing → NotFound
    let b = fresh();
    assert!(matches!(b.read("nope.txt").await, Err(FsError::NotFound(_))));

    // edit unique replaces once and reports before/after
    let b = fresh();
    b.write("e.txt", "foo bar baz").await.unwrap();
    let ed = b.edit("e.txt", "bar", "QUX").await.expect("edit");
    assert_eq!(ed.before, "foo bar baz");
    assert_eq!(ed.after, "foo QUX baz");
    assert_eq!(b.read("e.txt").await.unwrap(), "foo QUX baz");

    // edit ambiguous → EditConflict naming the count
    let b = fresh();
    b.write("e.txt", "x x").await.unwrap();
    match b.edit("e.txt", "x", "y").await {
        Err(FsError::EditConflict(msg)) => assert!(msg.contains("2 times"), "{msg}"),
        other => panic!("expected EditConflict, got {other:?}"),
    }

    // ls: name-sorted, dirs flagged
    let b = fresh();
    b.write("d/inner.txt", "1").await.unwrap();
    b.write("b.txt", "2").await.unwrap();
    let entries = b.ls("").await.expect("ls");
    let names: Vec<(String, bool)> = entries.into_iter().map(|e| (e.name, e.is_dir)).collect();
    assert_eq!(names, vec![("b.txt".to_string(), false), ("d".to_string(), true)]);

    // glob
    let b = fresh();
    b.write("a.rs", "").await.unwrap();
    b.write("a.txt", "").await.unwrap();
    let hits = b.glob("*.rs").await.expect("glob");
    assert_eq!(hits, vec!["a.rs".to_string()]);

    // grep: 1-based line numbers, scoped and unscoped
    let b = fresh();
    b.write("g.txt", "one\nneedle here\nthree").await.unwrap();
    let hits = b.grep("needle", None).await.expect("grep");
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].path.as_str(), hits[0].line), ("g.txt", 2));
    assert!(b.grep("needle", Some("elsewhere")).await.unwrap().is_empty());

    // delete then read → NotFound; delete missing → NotFound
    let b = fresh();
    b.write("del.txt", "x").await.unwrap();
    b.delete("del.txt").await.expect("delete");
    assert!(matches!(b.read("del.txt").await, Err(FsError::NotFound(_))));
    assert!(matches!(b.delete("del.txt").await, Err(FsError::NotFound(_))));
}
```

- [ ] **Step 5: Wire MemBackend to the suite and run**

Append to `mem.rs`:

```rust
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
```

Run: `cd agent && cargo test -p agent-tools backend::`
Expected: PASS (all new tests).

- [ ] **Step 6: Full wave-1 parity check + commit**

Run: `cd agent && cargo test -p agent-tools`
Expected: PASS, zero pre-existing test changes.

```bash
git checkout -b feature/backend-seam
git add agent/Cargo.toml agent/crates/agent-tools
git commit -m "feat(tools): Backend trait, FsError, MemBackend + public conformance suite (Phase 2 wave 1)"
```

---

### Task 2: HostBackend

**Files:**
- Create: `agent/crates/agent-tools/src/backend/host.rs`
- Modify: `agent/crates/agent-tools/src/backend/mod.rs` (uncomment `mod host;` + `pub use host::HostBackend;`)

**Interfaces:**
- Consumes: `crate::fs::resolve_in_workspace` (existing, unchanged), Task-1 types.
- Produces: `HostBackend::new(root: PathBuf)`; containment errors use the exact live string `"path escapes workspace: {arg}"` (already produced by `resolve_in_workspace` as `ToolError::Denied` — map to `FsError::Denied` preserving the inner text).

- [ ] **Step 1: Write HostBackend**

Create `agent/crates/agent-tools/src/backend/host.rs`:

```rust
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
        while let Some(e) = rd.next_entry().await.map_err(|e| FsError::Io(e.to_string()))? {
            let is_dir = e.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            out.push(Entry { name: e.file_name().to_string_lossy().into_owned(), is_dir });
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
            .map_err(|_| FsError::NotUtf8(format!("{path}: stream did not contain valid UTF-8")))
    }

    async fn write(&self, path: &str, content: &str) -> Result<(), FsError> {
        let full = self.resolve(path)?;
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| FsError::Io(e.to_string()))?;
        }
        tokio::fs::write(&full, content)
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
            let Ok(content) = std::fs::read_to_string(&f) else { continue };
            for (i, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    hits.push(GrepHit { path: self.rel(&f), line: i + 1, text: line.to_string() });
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
```

- [ ] **Step 2: Conformance + host-specific tests**

Append to `host.rs`:

```rust
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
}
```

Run: `cd agent && cargo test -p agent-tools backend::host`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add agent/crates/agent-tools
git commit -m "feat(tools): HostBackend — disk backend with resolve_in_workspace containment"
```

---

### Task 3: CompositeBackend (strip+remap) + ReadOnlyToTools guard

**Files:**
- Create: `agent/crates/agent-tools/src/backend/composite.rs`
- Modify: `agent/crates/agent-tools/src/backend/mod.rs` (uncomment `mod composite;` + its `pub use`)

**Interfaces:**
- Produces: `CompositeBackend::new(mounts: Vec<(String, Arc<dyn Backend>)>, default: Arc<dyn Backend>)`, `ReadOnlyToTools(pub Arc<dyn Backend>)`, `ARTIFACTS_READONLY_MSG`.
- Contract (spec §5.2/E6): longest-prefix routing; mounts see stripped (root-relative) paths; results re-prefixed; `ls`/`glob`/`grep` aggregate; a grep scoped to one prefix never surfaces another mount's files; default-mount results falling under a mount prefix are filtered out (shadowing).

- [ ] **Step 1: Write CompositeBackend + guard**

Create `agent/crates/agent-tools/src/backend/composite.rs`:

```rust
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
        self.mounts.iter().any(|(prefix, _)| rel.starts_with(prefix.as_str()))
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
        let at = if inner.is_empty() { String::new() } else { format!("{}/", inner.trim_end_matches('/')) };
        for (prefix, _) in &self.mounts {
            if let Some(rest) = prefix.strip_prefix(at.as_str()) {
                let first = rest.trim_end_matches('/').split('/').next().unwrap_or_default();
                if !first.is_empty() && !out.iter().any(|e| e.name == first) {
                    out.push(Entry { name: first.into(), is_dir: true });
                }
            }
        }
        out.retain(|e| {
            let rel = format!("{at}{}", e.name);
            e.is_dir || !self.shadowed(&format!("{rel}/")) || true // files can't be shadowed; dirs handled below
        });
        out.sort();
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
            let inner_scope = if inner.is_empty() { None } else { Some(inner.as_str()) };
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
```

Note: the `ls` retain-closure above contains a stub condition (`|| true`) — replace it before committing with the real rule: drop a **directory** entry from the default's results when `self.shadowed(&format!("{rel}/"))` is true and a mount has already inserted the synthetic entry (the mount wins). Simplest correct form:

```rust
        // A real default-mount dir with a reserved name is shadowed: the
        // synthetic mount entry already covers it, so dedup by name.
        out.sort();
        out.dedup_by(|a, b| a.name == b.name);
```

(Replace the whole `out.retain(...)` block with this dedup after the synthetic-insert loop.)

- [ ] **Step 2: Composite + guard tests**

Append to `composite.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemBackend;

    fn two_mounts() -> CompositeBackend {
        CompositeBackend::new(
            vec![
                ("large_tool_results/".into(), Arc::new(MemBackend::new()) as Arc<dyn Backend>),
                ("conversation_history/".into(), Arc::new(MemBackend::new()) as Arc<dyn Backend>),
            ],
            Arc::new(MemBackend::new()),
        )
    }

    #[tokio::test]
    async fn routes_by_longest_prefix_and_strips() {
        let c = two_mounts();
        c.write("large_tool_results/1-call", "payload").await.unwrap();
        // The mount saw the stripped key: reading through the mount directly
        // (transparency) — reach it via the composite's own route.
        assert_eq!(c.read("large_tool_results/1-call").await.unwrap(), "payload");
        // Default mount is untouched.
        assert!(matches!(c.read("1-call").await, Err(FsError::NotFound(_))));
    }

    #[tokio::test]
    async fn mount_location_transparency() {
        // The SAME backend mounted at two different prefixes behaves identically.
        let shared: Arc<dyn Backend> = Arc::new(MemBackend::new());
        let a = CompositeBackend::new(vec![("x/".into(), shared.clone())], Arc::new(MemBackend::new()));
        let b = CompositeBackend::new(vec![("y/z/".into(), shared.clone())], Arc::new(MemBackend::new()));
        a.write("x/f.txt", "one").await.unwrap();
        assert_eq!(b.read("y/z/f.txt").await.unwrap(), "one");
    }

    #[tokio::test]
    async fn grep_scoped_to_one_prefix_never_leaks_the_other() {
        let c = two_mounts();
        c.write("large_tool_results/a", "needle in results").await.unwrap();
        c.write("conversation_history/history.md", "needle in history").await.unwrap();
        let hits = c.grep("needle", Some("large_tool_results/")).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.starts_with("large_tool_results/"), "{}", hits[0].path);
    }

    #[tokio::test]
    async fn unscoped_grep_aggregates_and_reprefixes() {
        let c = two_mounts();
        c.write("large_tool_results/a", "needle").await.unwrap();
        c.write("workspace.txt", "needle").await.unwrap();
        let mut paths: Vec<String> = c.grep("needle", None).await.unwrap().into_iter().map(|h| h.path).collect();
        paths.sort();
        assert_eq!(paths, vec!["large_tool_results/a".to_string(), "workspace.txt".to_string()]);
    }

    #[tokio::test]
    async fn root_ls_shows_mounts_as_dirs() {
        let c = two_mounts();
        c.write("large_tool_results/a", "x").await.unwrap();
        let names: Vec<String> = c.ls("").await.unwrap().into_iter().filter(|e| e.is_dir).map(|e| e.name).collect();
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
            g.edit("a", "original", "forged").await.err().map(|e| e),
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
```

Run: `cd agent && cargo test -p agent-tools backend::composite`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add agent/crates/agent-tools
git commit -m "feat(tools): CompositeBackend (strip+remap, E6) + ReadOnlyToTools artifacts guard"
```

---

### Task 4: Plumb the backend through ToolCtx and AgentLoop

**Files:**
- Modify: `agent/crates/agent-tools/src/types.rs` (ToolCtx field)
- Modify: `agent/crates/agent-core/src/loop_.rs` (AgentLoop field + `with_backend` + ToolCtx construction + test helper)
- Modify (mechanical — every `ToolCtx {` literal gains one field): `agent-core/src/context_tools.rs`, `agent-core/src/dispatch.rs`, `agent-core/tests/dispatch_tool.rs`, `agent-http/src/tool.rs`, `agent-mcp/src/tool.rs`, `agent-memory/src/tools.rs`, `agent-memory/tests/live_embed.rs`, `agent-skills/src/tools.rs`, `agent-tools/src/fs/read.rs`, `agent-tools/src/fs/write.rs`, `agent-tools/src/git.rs`, `agent-tools/src/render.rs`, `agent-tools/src/shell.rs`

**Interfaces:**
- Produces: `ToolCtx.backend: Arc<dyn Backend>`; `AgentLoop::with_backend(self, Arc<dyn Backend>) -> Self`. Default backend (no `with_backend` call) = `HostBackend::new(config.workspace.clone())` — today's behavior exactly, so **no assemble/dispatch changes are needed in wave 1**.

- [ ] **Step 1: Add the ToolCtx field**

In `agent/crates/agent-tools/src/types.rs`, extend `ToolCtx` (locate `pub struct ToolCtx`):

```rust
pub struct ToolCtx {
    pub workspace: PathBuf,
    pub timeout: Duration,
    pub cancel: CancellationToken,
    pub sandbox: Arc<dyn SandboxStrategy>,
    /// The virtual filesystem this call's file tools operate on (spec §5.3).
    /// Mirrors `sandbox`: loop-scoped, set from LoopConfig-derived state.
    pub backend: Arc<dyn crate::backend::Backend>,
    pub call_id: String,
}
```

- [ ] **Step 2: Mechanical sweep of ToolCtx literals**

Every `ToolCtx {` literal in the files listed above gains exactly one line. In test helpers that already bind a workspace variable `ws`/`dir`, add:

```rust
            backend: Arc::new(agent_tools::backend::HostBackend::new(ws.clone())),
```

(inside `agent-tools` itself use `crate::backend::HostBackend`). Where the helper uses `std::env::temp_dir()` for `workspace`, use the same expression for the backend root. Find every site with:

Run: `grep -rn "ToolCtx {" agent/crates --include=*.rs`
Expected: the grep is the source of truth — update **every** line it returns (~30 literals plus the struct definition). Note `agent-mcp/src/tool.rs`, `agent-tools/src/git.rs`, and `loop_.rs` each contain TWO sites; a file-by-file pass that stops at the first hit will miss the second.

- [ ] **Step 3: AgentLoop carries and forwards the backend**

In `agent/crates/agent-core/src/loop_.rs`:

1. Add a field to `AgentLoop` (locate `pub struct AgentLoop`): `backend: Arc<dyn agent_tools::backend::Backend>,`
2. In `AgentLoop::new`, before `config` is moved: `let backend: Arc<dyn agent_tools::backend::Backend> = Arc::new(agent_tools::backend::HostBackend::new(config.workspace.clone()));` and store it.
3. Add the builder (next to `with_compaction_model`):

```rust
    /// Replace the virtual filesystem the loop hands to tools (spec §5.3).
    /// Default: a HostBackend rooted at `config.workspace` — bare-loop parity.
    pub fn with_backend(mut self, backend: Arc<dyn agent_tools::backend::Backend>) -> Self {
        self.backend = backend;
        self
    }
```

4. In the `ToolCtx` construction inside the gate path (locate `let ctx = ToolCtx {` near `gate_tool`), add: `backend: self.backend.clone(),`. Update the loop's own `test_ctx` helper the same way.

- [ ] **Step 4: Run the full agent workspace suite**

Run: `cd agent && cargo test`
Expected: PASS — zero assertion changes anywhere (construction sites only).

- [ ] **Step 5: Commit**

```bash
git add agent/crates
git commit -m "feat(core): backend handle rides ToolCtx/AgentLoop (default HostBackend at workspace) — wave-1 parity"
```

---

### Task 5: File tools become backend adapters

**Files:**
- Modify: `agent/crates/agent-tools/src/fs/read.rs` (ReadFile, ListDirectory execute bodies)
- Modify: `agent/crates/agent-tools/src/fs/write.rs` (WriteFile, EditFile execute bodies)

**Interfaces:**
- Consumes: `ctx.backend` (Task 4).
- Produces: byte-identical tool outputs for every case that succeeds today. Wave-1 parity mapping (spec §5.10): `FsError::NotUtf8(msg)` maps to `ToolError::NotFound(msg)` — the message text is already identical to today's; the honest-error flip is Task 6.
- Error mapping helper produced here, used by Tasks 6–7: `pub(crate) fn fs_err(e: FsError) -> ToolError`.

- [ ] **Step 1: Add the shared error mapping**

In `agent/crates/agent-tools/src/fs/mod.rs` add:

```rust
use crate::backend::FsError;
use crate::ToolError;

/// FsError → ToolError in one place (spec §5.1). Wave-1 parity: NotUtf8 maps
/// to NotFound (same message today's read_to_string path produced); Task 6
/// flips it to the honest error alongside the paging contract.
pub(crate) fn fs_err(e: FsError) -> ToolError {
    match e {
        FsError::NotFound(m) => ToolError::NotFound(m),
        FsError::Denied(m) => ToolError::Denied(m),
        FsError::NotUtf8(m) => ToolError::NotFound(m),
        FsError::EditConflict(m) | FsError::Io(m) => ToolError::Failed { message: m, stderr: None },
        FsError::InvalidPath(m) => ToolError::InvalidArgs(m),
    }
}
```

- [ ] **Step 2: Rewrite the four execute bodies**

`read.rs` — `ReadFile::execute`: replace the `resolve_in_workspace` + `read_to_string` block with:

```rust
        let path = arg_path(&args)?;
        let content = ctx.backend.read(&path).await.map_err(crate::fs::fs_err)?;
```

(the offset/limit slicing below it is unchanged). `ListDirectory::execute`:

```rust
        let path = arg_path(&args)?;
        let entries = ctx.backend.ls(&path).await.map_err(crate::fs::fs_err)?;
        let names: Vec<String> = entries.into_iter().map(|e| e.name).collect();
        Ok(ToolOutput { content: names.join("\n"), display: None })
```

`write.rs` — `WriteFile::execute`:

```rust
        let path = str_arg(&args, "path")?;
        let content = str_arg(&args, "content")?;
        let before = ctx.backend.read(&path).await.unwrap_or_default();
        ctx.backend.write(&path, &content).await.map_err(crate::fs::fs_err)?;
        Ok(ToolOutput {
            content: format!("wrote {} bytes to {path}", content.len()),
            display: Some(diff(&path, &before, &content)),
        })
```

`EditFile::execute`:

```rust
        let path = str_arg(&args, "path")?;
        let old = str_arg(&args, "old")?;
        let new = str_arg(&args, "new")?;
        let edited = ctx.backend.edit(&path, &old, &new).await.map_err(crate::fs::fs_err)?;
        Ok(ToolOutput {
            content: format!("edited {path}"),
            display: Some(diff(&path, &edited.before, &edited.after)),
        })
```

Remove the now-unused `resolve_in_workspace` imports from both files (it stays exported from `fs/paths.rs` for `HostBackend` and `agent-policy`).

- [ ] **Step 3: Run the wave-1 parity gate**

Run: `cd agent && cargo test` then `cd .. && bash scripts/ci.sh`
Expected: everything green; **no assertion body changed in this task** — the existing fs-tool tests (including `edit_file_errors_when_old_not_unique`, `read_file_default_is_whole_file_unchanged`, the symlink suite via HostBackend) pass as-is.

- [ ] **Step 4: Commit (wave 1 complete)**

```bash
git add agent/crates/agent-tools
git commit -m "refactor(tools): file tools are Backend adapters — wave 1 complete, zero behavior change"
```

---

## WAVE 2 — the substrate migration

### Task 6: `read_file` paging contract (byte mode + source cap)

**Files:**
- Modify: `agent/crates/agent-tools/src/fs/read.rs` (ReadFile gains `max_bytes`; byte mode; markers)
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (`build_registry` signature)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (pass `cfg.max_tool_result_bytes`)

**Interfaces:**
- Produces: `ReadFile { pub max_bytes: usize }`; new optional arg `byte_offset` (mutually exclusive with `offset`/`limit`); `build_registry(http_allow_hosts: &[String], max_read_bytes: usize) -> ToolRegistry`.
- Normative contract (spec §5.4): under-cap output byte-identical to today; `offset` is ALWAYS a 1-based line number; `byte_offset` returns a raw char-boundary-snapped slice with **no header** and the marker `\n[bytes {start}–{end} of {total} — continue with read_file(path: "{path}", byte_offset: {end})]`; following byte-mode markers from 0 reassembles any file exactly; over-cap line output truncates to whole lines with marker `\n[lines {first}–{last} of {n} — continue with read_file(path: "{path}", offset: {last+1})]`; a monster first line falls to byte mode. This task also flips `NotUtf8` to the honest error (spec J3).

- [ ] **Step 1: Write the failing byte-mode tests**

Add to `read.rs` tests (the helper `ctx()` already exists; construct the tool as `ReadFile { max_bytes: 4096 }` — this won't compile yet, which is the failing state):

```rust
    /// Extract the continuation byte offset from a page's trailing marker.
    fn byte_continuation(page: &str) -> Option<usize> {
        let tail = page.rsplit("byte_offset: ").next()?;
        tail.split(')').next()?.trim().parse().ok()
    }

    #[tokio::test]
    async fn byte_mode_pages_reassemble_exact_bytes() {
        // Ports recall_pages_a_large_entry_to_completion (spec §7).
        let dir = tempdir().unwrap();
        let content: String = (0..10_000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        std::fs::write(dir.path().join("blob"), &content).unwrap();
        let tool = ReadFile { max_bytes: 4096 };
        let mut reassembled = String::new();
        let mut offset = 0usize;
        loop {
            let out = tool
                .execute(json!({"path": "blob", "byte_offset": offset}), &ctx(dir.path().into()))
                .await
                .unwrap();
            assert!(out.content.len() <= 4096, "page exceeds cap");
            match byte_continuation(&out.content) {
                Some(next) if out.content.contains("continue with read_file") => {
                    let body = out.content.rsplit_once("\n[bytes ").unwrap().0;
                    assert!(next > offset, "no forward progress");
                    reassembled.push_str(body);
                    offset = next;
                }
                _ => {
                    reassembled.push_str(&out.content);
                    break;
                }
            }
        }
        assert_eq!(reassembled, content, "byte pages must reassemble the original exactly");
    }

    #[tokio::test]
    async fn byte_mode_slices_on_char_boundaries() {
        // Ports recall_slices_on_char_boundaries (spec §7).
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("crab"), "🦀".repeat(3000)).unwrap();
        let tool = ReadFile { max_bytes: 4096 };
        let out = tool
            .execute(json!({"path": "crab", "byte_offset": 0}), &ctx(dir.path().into()))
            .await
            .unwrap();
        assert!(out.content.starts_with('🦀'));
    }

    #[tokio::test]
    async fn byte_offset_past_end_is_invalid_args() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("s"), "short").unwrap();
        let tool = ReadFile { max_bytes: 4096 };
        let err = tool
            .execute(json!({"path": "s", "byte_offset": 999}), &ctx(dir.path().into()))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn byte_offset_and_offset_are_mutually_exclusive() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("s"), "short").unwrap();
        let tool = ReadFile { max_bytes: 4096 };
        let err = tool
            .execute(json!({"path": "s", "offset": 1, "byte_offset": 0}), &ctx(dir.path().into()))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn over_cap_multiline_read_truncates_to_whole_lines_with_marker() {
        let dir = tempdir().unwrap();
        let content: String = (0..500).map(|i| format!("line number {i} with padding\n")).collect();
        std::fs::write(dir.path().join("big"), &content).unwrap();
        let tool = ReadFile { max_bytes: 2048 };
        let out = tool.execute(json!({"path": "big"}), &ctx(dir.path().into())).await.unwrap();
        assert!(out.content.len() <= 2048);
        assert!(out.content.starts_with("[lines 1–"), "{}", &out.content[..40]);
        assert!(out.content.contains("continue with read_file(path: \"big\", offset: "));
    }

    #[tokio::test]
    async fn monster_single_line_falls_to_byte_mode() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("mono"), "x".repeat(50_000)).unwrap();
        let tool = ReadFile { max_bytes: 2048 };
        let out = tool.execute(json!({"path": "mono"}), &ctx(dir.path().into())).await.unwrap();
        assert!(out.content.len() <= 2048);
        assert!(out.content.contains("byte_offset: "), "{}", out.content);
    }

    #[tokio::test]
    async fn binary_file_is_an_honest_error() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("bin"), [0xFF, 0xFE, 0x00]).unwrap();
        let tool = ReadFile { max_bytes: 4096 };
        let err = tool.execute(json!({"path": "bin"}), &ctx(dir.path().into())).await.unwrap_err();
        match err {
            ToolError::Failed { message, .. } => assert!(message.contains("not valid UTF-8")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
```

Also update every existing `ReadFile` construction in this file's tests to `ReadFile { max_bytes: 16 * 1024 }` — the existing under-cap assertions (whole-file default, offset/limit slicing, clamps) must pass **unchanged** under the default-scale cap.

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-tools fs::read`
Expected: FAIL — `ReadFile` has no field `max_bytes`, no `byte_offset` handling.

- [ ] **Step 3: Implement**

In `read.rs`, change `pub struct ReadFile;` to `pub struct ReadFile { pub max_bytes: usize }`, extend the schema with:

```rust
                "byte_offset":{"type":"integer","description":
                    "Raw byte offset to continue a large read from (from a previous \
                     page's continuation marker). Returns raw bytes with no line header. \
                     Mutually exclusive with offset/limit."},
```

and replace the tail of `execute` (after `content` is read; the honest-error flip changes the read line too):

```rust
        let content = ctx.backend.read(&path).await.map_err(|e| match e {
            crate::backend::FsError::NotUtf8(m) => ToolError::Failed {
                message: format!("{m} (binary file) — file tools are text-only"),
                stderr: None,
            },
            other => crate::fs::fs_err(other),
        })?;
        let byte_offset = args.get("byte_offset").and_then(|v| v.as_u64()).map(|v| v as usize);
        let offset = args.get("offset").and_then(|v| v.as_u64()).map(|v| (v as usize).max(1));
        let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
        if byte_offset.is_some() && (offset.is_some() || limit.is_some()) {
            return Err(ToolError::InvalidArgs(
                "byte_offset is mutually exclusive with offset/limit".into(),
            ));
        }
        if limit == Some(0) {
            return Err(ToolError::InvalidArgs("limit must be >= 1".into()));
        }
        if let Some(start) = byte_offset {
            return byte_page(&path, &content, start, self.max_bytes).map(|content| ToolOutput { content, display: None });
        }
        let rendered = render_lines(&path, &content, offset, limit)?; // today's logic, extracted
        if rendered.len() <= self.max_bytes {
            return Ok(ToolOutput { content: rendered, display: None });
        }
        // Over-cap: whole lines that fit + line-mode marker; monster line → byte mode.
        Ok(ToolOutput { content: capped_lines(&path, &content, offset.unwrap_or(1), self.max_bytes), display: None })
```

with these free functions (ported from `context_tools.rs::recall_marker` + `ContextRecallTool::execute` — copy the char-boundary discipline exactly):

```rust
fn byte_marker(path: &str, start: usize, end: usize, total: usize) -> String {
    format!(
        "\n[bytes {start}–{end} of {total} — continue with read_file(path: \"{path}\", byte_offset: {end})]"
    )
}

/// Raw byte page, char-boundary-snapped on both ends (spec §5.4 byte mode).
fn byte_page(path: &str, content: &str, offset: usize, cap: usize) -> Result<String, ToolError> {
    let total = content.len();
    if offset > 0 && offset >= total {
        return Err(ToolError::InvalidArgs(format!(
            "byte_offset {offset} is past the end of {path} ({total} bytes)"
        )));
    }
    let mut start = offset;
    while !content.is_char_boundary(start) {
        start -= 1;
    }
    let rest = &content[start..];
    if rest.len() <= cap {
        return Ok(rest.to_string());
    }
    let worst = byte_marker(path, start, total, total);
    let budget = cap.saturating_sub(worst.len()).max(1);
    let mut cut = start + budget;
    while !content.is_char_boundary(cut) {
        cut -= 1;
    }
    if cut <= start {
        cut = start + rest.chars().next().map_or(1, |c| c.len_utf8());
    }
    Ok(format!("{}{}", &content[start..cut], byte_marker(path, start, cut, total)))
}

/// Over-cap line mode: greedily keep whole lines under the cap; if not even
/// one line fits, fall to a byte page starting at the first requested line.
fn capped_lines(path: &str, content: &str, first: usize, cap: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len();
    let worst = format!(
        "\n[lines {first}–{n} of {n} — continue with read_file(path: \"{path}\", offset: {})]",
        n + 1
    );
    let header_worst = format!("[lines {first}–{n} of {n}]\n");
    let budget = cap.saturating_sub(worst.len() + header_worst.len());
    let mut kept = 0usize;
    let mut used = 0usize;
    for l in &lines[first - 1..] {
        let add = l.len() + 1;
        if used + add > budget {
            break;
        }
        used += add;
        kept += 1;
    }
    if kept == 0 {
        // Monster line: byte page from the first requested line's byte offset.
        let start: usize = lines[..first - 1].iter().map(|l| l.len() + 1).sum();
        return byte_page(path, content, start, cap).expect("start < total by construction");
    }
    let last = first + kept - 1;
    format!(
        "[lines {first}–{last} of {n}]\n{}\n[lines {first}–{last} of {n} — continue with read_file(path: \"{path}\", offset: {})]",
        lines[first - 1..last].join("\n"),
        last + 1
    )
}
```

`render_lines` is today's `(offset, limit)` match block extracted verbatim into a function returning `Result<String, ToolError>` — move, don't rewrite (preserves the existing pins byte-for-byte).

- [ ] **Step 4: build_registry threads the cap**

In `agent/crates/agent-runtime-config/src/lib.rs` change the signature and the registration:

```rust
pub fn build_registry(http_allow_hosts: &[String], max_read_bytes: usize) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(ReadFile { max_bytes: max_read_bytes }));
```

In `assemble.rs`: `let mut registry = build_registry(&cfg.http_allow_hosts, cfg.max_tool_result_bytes);`. Update the `registry_has_all_core_tools` test call site with `agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES`-equivalent literal `16 * 1024`.

- [ ] **Step 5: Run and commit**

Run: `cd agent && cargo test -p agent-tools -p agent-runtime-config`
Expected: PASS (new byte-mode tests + all under-cap pins unchanged).

```bash
git add agent/crates
git commit -m "feat(tools): read_file paging contract — byte mode ports context_recall's exact-bytes machinery, source cap, honest NotUtf8 error"
```

---

### Task 7: `grep` tool

**Files:**
- Create: `agent/crates/agent-tools/src/fs/search.rs`
- Modify: `agent/crates/agent-tools/src/fs/mod.rs` (add `mod search; pub use search::GrepTool;`)
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (register in `build_registry`)

**Interfaces:**
- Produces: tool name `grep`, args `pattern` (required), `path` (optional scope). Output lines `path:line: text`; cap note appended when the backend cap is hit. `Access::Read` intent with the scope path.

- [ ] **Step 1: Failing test**

Create `search.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn ctx(ws: std::path::PathBuf) -> ToolCtx {
        ToolCtx {
            workspace: ws.clone(),
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(crate::HostExecutor),
            backend: Arc::new(crate::backend::HostBackend::new(ws)),
            call_id: "test".into(),
        }
    }

    #[tokio::test]
    async fn grep_reports_path_line_and_text() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "alpha\nthe needle line\n").unwrap();
        let out = GrepTool.execute(json!({"pattern": "needle"}), &ctx(dir.path().into())).await.unwrap();
        assert_eq!(out.content, "f.txt:2: the needle line");
    }

    #[tokio::test]
    async fn grep_no_hits_says_so() {
        let dir = tempdir().unwrap();
        let out = GrepTool.execute(json!({"pattern": "absent"}), &ctx(dir.path().into())).await.unwrap();
        assert_eq!(out.content, "no matches");
    }

    #[test]
    fn grep_intent_is_read_with_scope() {
        let i = GrepTool.intent(&json!({"pattern": "x", "path": "src/"})).unwrap();
        assert_eq!(i.access, Access::Read);
        assert_eq!(i.paths, vec![std::path::PathBuf::from("src/")]);
    }
}
```

Run: `cd agent && cargo test -p agent-tools fs::search`
Expected: FAIL — `GrepTool` not defined.

- [ ] **Step 2: Implement**

Above the tests in `search.rs`:

```rust
//! Regex search over the loop's virtual filesystem — the search half of the
//! offload-recovery surface that replaces context_recall (spec §5.4).
use crate::backend::GREP_MAX_HITS;
use crate::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search file contents by regex. Returns path:line: text hits. Searches \
         the workspace AND the read-only offload records under large_tool_results/ \
         and conversation_history/ (shell commands cannot see those two prefixes)."
    }
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Not for semantic search of saved memories — use recall. Use grep to \
             search current file contents, including offloaded tool results.",
        )
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "grep".into(),
            description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "pattern":{"type":"string","description":"Rust-flavored regex matched per line."},
                "path":{"type":"string","description":"Optional file or directory prefix to scope the search (e.g. large_tool_results/)."}},
                "required":["pattern"]}),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing string field `pattern`".into()))?;
        let scope = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        Ok(ToolIntent {
            tool: "grep".into(),
            access: Access::Read,
            paths: vec![scope.into()],
            command: None,
            summary: format!("grep {pattern:?} in {scope}"),
        })
    }
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing string field `pattern`".into()))?;
        let scope = args.get("path").and_then(|v| v.as_str());
        let hits = ctx.backend.grep(pattern, scope).await.map_err(crate::fs::fs_err)?;
        if hits.is_empty() {
            return Ok(ToolOutput { content: "no matches".into(), display: None });
        }
        let capped = hits.len() >= GREP_MAX_HITS;
        let mut lines: Vec<String> =
            hits.into_iter().map(|h| format!("{}:{}: {}", h.path, h.line, h.text)).collect();
        if capped {
            lines.push(format!("[hit cap reached: {GREP_MAX_HITS} — narrow the pattern or scope]"));
        }
        Ok(ToolOutput { content: lines.join("\n"), display: None })
    }
}
```

Register in `build_registry` (after `ListDirectory`): `r.register(Arc::new(GrepTool));`

- [ ] **Step 3: Run, then commit**

Run: `cd agent && cargo test -p agent-tools -p agent-runtime-config`
Expected: PASS. (If `registry_has_all_core_tools` asserts an exact tool list, add `"grep"` to it — that is an intentional wave-2 surface change, spec §5.6.)

```bash
git add agent/crates
git commit -m "feat(tools): grep tool — regex recovery surface over the virtual filesystem"
```

---

### Task 8: Curation writes files (substrate migration core)

This is the largest task; it must land as one compiling unit because `CuratedContext::new`'s signature changes. Internal step-commits keep review tractable.

**Files:**
- Create: `agent/crates/agent-core/src/artifacts.rs`
- Modify: `agent/crates/agent-core/src/offload_policy.rs` (path grammar; `OffloadEntry` loses `id`)
- Delete: `agent/crates/agent-core/src/offload.rs`
- Modify: `agent/crates/agent-core/src/curated.rs` (sinks + ledger + pointer + tests)
- Modify: `agent/crates/agent-core/src/context_tools.rs` (delete recall; `context_tools(flag)`)
- Modify: `agent/crates/agent-core/src/middleware.rs` (`ContextCurationMiddleware::new(flag)`)
- Modify: `agent/crates/agent-core/src/event.rs` (`Offloaded { path }`)
- Modify: `agent/crates/agent-core/src/lib.rs` (module swaps: `mod artifacts;` in, `mod offload;` out; re-exports)
- Modify: `agent/crates/agent-core/src/dispatch.rs`, `agent/crates/agent-core/src/loop_.rs` (construction sites)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (LoopParts + composite + shadow warning)
- Modify: `agent/crates/agent-cli/src/main.rs`, `agent/crates/agent-cli/src/render.rs`
- Modify: `agent/crates/agent-server/src/runtime.rs`, `session.rs`, `wire.rs`
- Modify: `web/src/state.ts`
- Modify (tests): `agent-core/tests/{compaction_routing.rs,timeout_override.rs}`, `agent-runtime-config/tests/{e2e_context_management.rs,e2e_auto_retrieval.rs,e2e_robustness.rs,stress_context_management.rs,soak_live.rs}` — the last two are integration-test **compile targets** referencing the deleted symbols (`#[ignore]` skips execution, not compilation), so they MUST migrate inside this task for the Step-8 gate to compile

**Interfaces:**
- Produces: `agent_core::SessionArtifacts { pub results: Arc<dyn Backend>, pub history: Arc<dyn Backend> }` with `SessionArtifacts::new()` building two `MemBackend`s (plan-time refinement of spec §5.3's concrete-type note: `dyn` fields keep the handles privileged AND let Task 10 inject a failing backend for the INCOMPLETE pin — record in the spec log, Task 12).
- `CuratedContext::new(system: Message, artifacts: Arc<SessionArtifacts>, compact_flag: Arc<AtomicBool>)` + `with_artifact_prefix(String)`.
- `ContextCurationMiddleware::new(flag: Arc<AtomicBool>)` — plan-time correction: with recall retired the middleware needs only the flag (record in spec log, Task 12).
- `ContextEvent::Offloaded { path: String, bytes: usize, tool: String }`.
- `LoopParts.artifacts: Arc<SessionArtifacts>` replaces `offload_store`.
- Placeholder grammar (verbatim, spec §5.5):
  - lift: `[tool_result offloaded to {vpath}: {bytes}B {kind} from "{tool}" — read_file the path, or grep large_tool_results/ to search]`
  - truncation: `\n[tool_result truncated: showing first {shown}B of {total}B from "{tool}" — full content at {vpath}; continue with read_file(path: "{vpath}", byte_offset: {shown})]`
- History file: mount-relative key `history.md`, virtual path `conversation_history/history.md`, sections `## folded-{seq}` / `## compacted-{seq}`.
- Pointer suffix (rendered by `pinned()`, never stored in `compaction_summary`):
  - complete: `Evicted transcripts: conversation_history/history.md — grep it for "## folded-" / "## compacted-" section headers, then read_file from the hit's line offset.`
  - incomplete: same with prefix `Evicted transcripts (INCOMPLETE — at least one span failed to record): …`

- [ ] **Step 1: SessionArtifacts**

Create `agent/crates/agent-core/src/artifacts.rs`:

```rust
//! Caller-owned artifact stores (spec §5.3, E6). The caller owns this handle
//! and passes the SAME one across loop rebuilds (server settings change) so
//! the conversation's offloaded artifacts survive — the successor of the
//! offload_store survival contract. Two stores because the composite strips
//! mount prefixes (E6): a single backend mounted twice would merge namespaces.
use agent_tools::backend::{Backend, MemBackend};
use std::sync::Arc;

pub struct SessionArtifacts {
    /// Backing store for the `large_tool_results/` mount (privileged handle).
    pub results: Arc<dyn Backend>,
    /// Backing store for the `conversation_history/` mount (privileged handle).
    pub history: Arc<dyn Backend>,
}

impl SessionArtifacts {
    pub fn new() -> Self {
        Self { results: Arc::new(MemBackend::new()), history: Arc::new(MemBackend::new()) }
    }
}

impl Default for SessionArtifacts {
    fn default() -> Self {
        Self::new()
    }
}
```

In `agent-core/src/lib.rs`: add `mod artifacts; pub use artifacts::SessionArtifacts;`, remove `mod offload;` and its re-exports (`OffloadStore`, `InMemoryOffloadStore`, `OffloadId`); keep `OffloadEntry`/`OffloadKind` re-exported from `offload_policy` (they move there in Step 2). Delete `agent/crates/agent-core/src/offload.rs`.

- [ ] **Step 2: offload_policy.rs path grammar**

Move `OffloadKind` + `OffloadEntry` (minus `id`) into `offload_policy.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OffloadKind {
    Error,
    Output,
}

#[derive(Debug, Clone)]
pub struct OffloadEntry {
    pub tool_call_id: String,
    pub tool_name: String,
    pub kind: OffloadKind,
    pub content: String,
    pub bytes: usize,
    pub turn: usize,
}
```

Replace `PLACEHOLDER_PREFIX` and the marker fns:

```rust
/// The two skip literals (spec §5.5) — as narrow as the old "[tool_result#":
/// selectors and the durable-unit detector all gate on exactly these.
pub const PLACEHOLDER_PREFIXES: [&str; 2] = ["[tool_result offloaded", "[tool_result truncated"];

pub fn is_placeholder(content: &str) -> bool {
    PLACEHOLDER_PREFIXES.iter().any(|p| content.starts_with(p))
}

/// The compact stub left in the live window (spec §5.5 grammar, verbatim).
pub fn placeholder_for(vpath: &str, tool_name: &str, kind: &OffloadKind, bytes: usize) -> String {
    let kind_str = match kind {
        OffloadKind::Error => "error",
        OffloadKind::Output => "output",
    };
    format!(
        "[tool_result offloaded to {vpath}: {bytes}B {kind_str} from \"{tool_name}\" \
         — read_file the path, or grep large_tool_results/ to search]"
    )
}

/// Marker appended to an ingestion-capped preview; continuation is read_file
/// byte mode against the artifact path (spec §5.4/§5.5).
pub fn truncation_marker(vpath: &str, tool_name: &str, shown: usize, total: usize) -> String {
    format!(
        "\n[tool_result truncated: showing first {shown}B of {total}B from \"{tool_name}\" \
         — full content at {vpath}; continue with read_file(path: \"{vpath}\", byte_offset: {shown})]"
    )
}
```

`capped_preview(content, cap, vpath, tool_name)` keeps its exact budgeting/char-boundary body, calling the new `truncation_marker`; its degenerate (marker-only, `trim_start`) output starts with `[tool_result truncated` → still a skip literal. In `select_offloads`/`select_oversized`, replace `m.content.starts_with(PLACEHOLDER_PREFIX)` with `is_placeholder(&m.content)` and drop `id: 0` from the `OffloadEntry` literals. Update this file's tests mechanically: `placeholder_for(7, …)` → `placeholder_for("large_tool_results/7-c1", …)`; `capped_preview(&content, 1024, 42, "shell")` → `capped_preview(&content, 1024, "large_tool_results/42-c1", "shell")`; assertions swap `tool_result#42 truncated` → `full content at large_tool_results/42-c1` and `context_recall(id: 42, offset: ` → `byte_offset: `. The idempotency pins (`capped_preview_is_idempotent_under_reselection`, `pathological_small_cap_degrades_to_placeholder_prefix`, `already_offloaded_placeholder_is_skipped`) keep their **property assertions** unchanged — only the constructed strings change. Add the accepted-residual pin (spec §7: "a future change is a conscious one"):

```rust
    #[test]
    fn result_echoing_a_placeholder_line_is_skipped_accepted_residual() {
        // A large tool result whose content STARTS with a full placeholder
        // line is skipped by the selectors — the same theoretical false
        // positive today's "[tool_result#" prefix had, accepted at the panel
        // (spec §5.5). This pin makes any future change to that behavior a
        // conscious decision.
        let echoed = format!(
            "{}\n{}",
            placeholder_for("large_tool_results/9-cX", "shell", &OffloadKind::Output, 9000),
            "y".repeat(5000)
        );
        let history = vec![Message::tool("c1", "shell", &echoed)];
        assert!(select_oversized(&history, &cap_cfg(1024)).is_empty());
    }
```

- [ ] **Step 3: event + wire + renderers (compile fallout of the enum change)**

`event.rs`: `Offloaded { path: String, bytes: usize, tool: String }`.
`agent-server/src/wire.rs` (locate the `CE::Offloaded` arm ~line 320): emit `("offloaded", json!({"path": path, "bytes": bytes, "tool": tool}))`; update the round-trip pin (~line 641) to construct/assert `path: "large_tool_results/1-c1"`.
`agent-cli/src/render.rs` (locate `CE::Offloaded`): `format!("⟲ offloaded {tool} result → {path} ({} KB)", bytes / 1024)`.
`web/src/state.ts` (locate `case "offloaded"`): `` return `offloaded ${detail.tool} result → ${detail.path ?? `#${detail.id}`}`; `` (the `?? #id` fallback keeps pre-Phase-2 trace replays rendering, spec §5.8).

- [ ] **Step 4: curated.rs sink migration**

Field/signature changes in `CuratedContext`:

```rust
    pub(crate) artifacts: Arc<crate::SessionArtifacts>,
    /// Prepended to artifact names; children get "sub{n}-" (spec §5.7).
    artifact_prefix: String,
    seq: u64,
    /// History-file state for the summary pointer (spec §5.5, E4).
    history_has_spans: bool,
    history_incomplete: bool,
    /// `## folded-{seq}` sections cited by the ledger (per-batch granularity).
    folded_sections: Vec<u64>,
```

(`store` and `folded_ids` are removed.) Constructor:

```rust
    pub fn new(system: Message, artifacts: Arc<crate::SessionArtifacts>, compact_flag: Arc<AtomicBool>) -> Self { /* fields as before, new ones defaulted */ }

    pub fn with_artifact_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.artifact_prefix = prefix.into();
        self
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }
```

`lift` becomes async and writes the store; on write failure it skips (message intact, retried next maintain — spec §5.5):

```rust
    async fn lift(
        &mut self,
        hit: crate::offload_policy::OffloadHit,
        report: &mut MaintReport,
        deps: &MaintCtx<'_>,
        replacement: impl FnOnce(&str) -> String,
    ) {
        let idx = hit.history_index;
        let tool = hit.entry.tool_name.clone();
        let bytes = hit.entry.bytes;
        let key = format!(
            "{}{}-{}",
            self.artifact_prefix,
            self.next_seq(),
            agent_tools::backend::sanitize_component(&hit.entry.tool_call_id)
        );
        let vpath = format!("large_tool_results/{key}");
        if let Err(e) = self.artifacts.results.write(&key, &hit.entry.content).await {
            tracing::warn!(error = %e, "artifact write failed; offload skipped this pass");
            return;
        }
        self.history[idx].content = replacement(&vpath);
        report.offloaded += 1;
        report.offloaded_bytes += bytes;
        deps.sink.emit(AgentEvent::Context(ContextEvent::Offloaded { path: vpath, bytes, tool }));
    }
```

Call sites in `maintain` become `self.lift(hit, &mut report, deps, |vpath| capped_preview(&content, cap, vpath, &tool)).await;` and `self.lift(hit, &mut report, deps, |vpath| placeholder_for(vpath, &tool, &kind, bytes)).await;` — same two passes, and the compaction-boundary pass in `compact_old_span` likewise.

History append helper:

```rust
    async fn append_history(&self, section: &str, body: &str) -> Result<(), agent_tools::backend::FsError> {
        use agent_tools::backend::FsError;
        let existing = match self.artifacts.history.read("history.md").await {
            Ok(s) => s,
            Err(FsError::NotFound(_)) => String::new(),
            Err(e) => return Err(e),
        };
        let updated = format!("{existing}\n\n## {section}\n\n{body}");
        self.artifacts.history.write("history.md", updated.trim_start()).await
    }
```

**Fifth lockstep site (spec §5.5 names five):** in `is_durable_placeholder_unit` (curated.rs, locate `m.content.starts_with("[tool_result#")`), replace the literal prefix check with `crate::offload_policy::is_placeholder(&m.content)` — without this, new-grammar placeholders lose their durable-anchor protection and get summarized away (the exact regression `maintain_keeps_offload_placeholders_verbatim_through_compaction` pins).

`fold_evicted_users`: replace the `store.put` block with — write FIRST, abort-on-failure (all-or-nothing keeps its exact shape, spec §5.5):

```rust
        let content = folded
            .iter()
            .map(|m| format!("[user] {}", m.content))
            .collect::<Vec<_>>()
            .join("\n");
        let bytes = content.len();
        let seq = self.next_seq();
        if let Err(e) = self.append_history(&format!("folded-{seq}"), &content).await {
            tracing::warn!(error = %e, "history write failed; leaving fold for the next maintain");
            return;
        }
        self.folded_sections.push(seq);
        self.folded_facts.extend(lines);
```

(the unit-removal/cap/report/emit tail is unchanged except the event: `ContextEvent::Offloaded { path: "conversation_history/history.md".into(), bytes, tool: "user_history".into() }`). Ledger body (`folded_block_body`) — replace the `ids` construction with:

```rust
        let sections = self
            .folded_sections
            .iter()
            .map(|s| format!("## folded-{s}"))
            .collect::<Vec<_>>()
            .join(", ");
        // header text becomes:
        // "…(facts extracted verbatim; the full original messages are preserved in \
        //  conversation_history/history.md, sections {sections} — read_file or grep it \
        //  if ever needed). When a task needs ALL earlier instructions…"
```

`compact_old_span`, inside the `Ok(summary) if compaction_is_worthwhile…` commit arm, after the reassembly + before the event emit:

```rust
                let seq = self.next_seq();
                let rendered: String = to_summarize
                    .iter()
                    .map(|m| {
                        let role = match m.role {
                            Role::System => "system",
                            Role::User => "user",
                            Role::Assistant => "assistant",
                            Role::Tool => "tool",
                        };
                        format!("[{role}] {}\n", m.content)
                    })
                    .collect();
                match self.append_history(&format!("compacted-{seq}"), &rendered).await {
                    Ok(()) => self.history_has_spans = true,
                    Err(e) => {
                        // E4 gate decision: commit anyway; the pointer goes
                        // permanently honest-incomplete.
                        tracing::warn!(error = %e, "history write failed; compaction commits without this span");
                        self.history_incomplete = true;
                    }
                }
```

Pointer rendering — in `pinned()`, replace the plain `compaction_summary` push:

```rust
        if let Some(c) = &self.compaction_summary {
            let mut msg = c.clone();
            if let Some(p) = self.history_pointer() {
                msg.content = format!("{}\n\n{p}", msg.content);
            }
            out.push(msg);
        }
```

with the helper (and mirror the same construction in `pinned_tokens`, which counts `message_tokens` of the identical composed message — lockstep, spec §5.5):

```rust
    /// The transcript pointer (spec §5.5): tracked as flags, NEVER stored in
    /// compaction_summary — the summarizer must never see or paraphrase it.
    fn history_pointer(&self) -> Option<String> {
        if !self.history_has_spans && !self.history_incomplete {
            return None;
        }
        let prefix = if self.history_incomplete {
            "Evicted transcripts (INCOMPLETE — at least one span failed to record)"
        } else {
            "Evicted transcripts"
        };
        Some(format!(
            "{prefix}: conversation_history/history.md — grep it for \"## folded-\" / \
             \"## compacted-\" section headers, then read_file from the hit's line offset."
        ))
    }
```

- [ ] **Step 5: context_tools + middleware**

`context_tools.rs`: delete `ContextRecallTool`, `recall_marker`, and all recall tests (`compact_sets_the_flag` survives). New assembly fn:

```rust
/// The context-management toolset. Since Phase 2 (spec G5) this is compact
/// only: offload recovery goes through the ordinary file tools.
pub fn context_tools(flag: Arc<AtomicBool>) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(ContextCompactTool::new(flag))]
}
```

`middleware.rs` `ContextCurationMiddleware`: fields become `{ flag: Arc<AtomicBool> }`; `new(flag)`; `tools()` → `crate::context_tools(self.flag.clone())` mapped to `child_visible: false` as today. (`maintain`/hooks are untouched — Phase-1 invariant.)

- [ ] **Step 6: construction-site rewire**

- `assemble.rs`: `LoopParts.offload_store: Arc<dyn OffloadStore>` → `artifacts: Arc<agent_core::SessionArtifacts>` (carry the survival doc comment over verbatim, re-worded for artifacts — spec §5.3). Stack push becomes `ContextCurationMiddleware::new(parts.compact_flag.clone())`. After `AgentLoop::new(...)`, build and attach the composite + shadow warning:

```rust
    use agent_tools::backend::{Backend, CompositeBackend, HostBackend, ReadOnlyToTools};
    for name in ["large_tool_results", "conversation_history"] {
        if parts.workspace.join(name).exists() {
            tracing::warn!(dir = name, "workspace entry is shadowed by a reserved artifact mount (spec §5.2)");
        }
    }
    let composite: Arc<dyn Backend> = Arc::new(CompositeBackend::new(
        vec![
            ("large_tool_results/".into(), Arc::new(ReadOnlyToTools(parts.artifacts.results.clone())) as Arc<dyn Backend>),
            ("conversation_history/".into(), Arc::new(ReadOnlyToTools(parts.artifacts.history.clone())) as Arc<dyn Backend>),
        ],
        Arc::new(HostBackend::new(parts.workspace.clone())),
    ));
    let agent = agent.with_backend(composite);
```

  Also update `assemble.rs`'s own test `LoopParts` fixture (`offload_store: Arc::new(...)` → `artifacts: Arc::new(agent_core::SessionArtifacts::new())`).
- `dispatch.rs` (locate the per-child store/flag block): `let artifacts = Arc::new(crate::SessionArtifacts::new()); let flag = Arc::new(AtomicBool::new(false));`; curation middleware `ContextCurationMiddleware::new(flag.clone())`; child context `CuratedContext::new(Message::system(system), artifacts.clone(), flag).with_offload_config(...).with_artifact_prefix(format!("sub{n}-"))`; child composite (parent workspace root comes from `self.deps.loop_config.workspace`):

```rust
        let child_backend: Arc<dyn agent_tools::backend::Backend> =
            Arc::new(agent_tools::backend::CompositeBackend::new(
                vec![
                    ("large_tool_results/".into(),
                     Arc::new(agent_tools::backend::ReadOnlyToTools(artifacts.results.clone())) as Arc<dyn agent_tools::backend::Backend>),
                    ("conversation_history/".into(),
                     Arc::new(agent_tools::backend::ReadOnlyToTools(artifacts.history.clone())) as Arc<dyn agent_tools::backend::Backend>),
                ],
                Arc::new(agent_tools::backend::HostBackend::new(self.deps.loop_config.workspace.clone())),
            ));
        let child = child.with_middleware(vec![curation, Arc::new(StuckDetectionMiddleware)]).with_backend(child_backend);
```

- `agent-cli/src/main.rs`: `offload_store`/store construction → `let artifacts = Arc::new(agent_core::SessionArtifacts::new());`, pass `artifacts` (both to `LoopParts` and any local uses).
- `agent-server/src/runtime.rs`: field + accessor rename (`offload_store` → `artifacts: Arc<SessionArtifacts>`, `pub fn artifacts(&self)`); `session.rs` both `CuratedContext::new(system, self.runtime.artifacts(), self.runtime.compact_flag())` sites.
- Test construction sites: `curated.rs` `ctx()` helper → `CuratedContext::new(Message::system("SYS"), Arc::new(crate::SessionArtifacts::new()), Arc::new(AtomicBool::new(false)))`; same pattern in `compaction_routing.rs`, `timeout_override.rs`, `loop_.rs` tests, and the runtime-config e2e fixtures.

- [ ] **Step 7: migrate curated.rs + e2e assertions (superseded pins, spec §7)**

Mechanical mapping, applied to every affected assertion:
- `c.store.len()` → `count of keys via c.artifacts.results.ls("").await.unwrap().len()` (add a small test helper `async fn results_count(c: &CuratedContext) -> usize`).
- `c.store.get(1).unwrap().content == X` → `c.artifacts.results.read("1-<sanitized-id>").await.unwrap() == X` (the test fixtures use `tool_call_id: "c1"`/`"call-1"` → keys `1-c1` / `1-call-1`).
- `content.starts_with("[tool_result#1 offloaded")` → `content.starts_with("[tool_result offloaded to large_tool_results/1-")`.
- ledger pins: `block.content.contains("context_recall(1)")` → `block.content.contains("conversation_history/history.md")` and `block.content.contains("## folded-1")`; add `assert!(c.artifacts.history.read("history.md").await.unwrap().contains("entry number 0"))` where the old pin read the store.
- `fold_extraction_failure_leaves_history_intact`: store-empty assertion → `history.read("history.md")` is `NotFound`.
- durable-placeholder-unit fixtures (`maintain_keeps_offload_placeholders_verbatim_through_compaction`): construct `ph` via the NEW `placeholder_for("large_tool_results/7-c1", "read_file", &OffloadKind::Output, 5000)`.
- `e2e_context_management.rs`: rename the round trip to `offload_then_read_file_round_trips_through_the_loop`; the scripted turn-2 call becomes `Scripted::Call("c2".into(), "read_file".into(), r#"{"path":"large_tool_results/1-c1"}"#.into())` (match the actual key from turn 1's fixture call id) and the sink filter matches `read_file`; the exact-bytes assertion is unchanged.
- `e2e_auto_retrieval.rs` / `e2e_robustness.rs`: construction sites only.
- `eval_context.rs` (same compile-target logic): **construction sites only** in this task — swap the `InMemoryOffloadStore` build + imports for `Arc::new(agent_core::SessionArtifacts::new())` so the target compiles. The semantic migration (recall-driving prompts/calls → file grammar) stays in Task 11 (E5); recall-driving *strings* compile fine untouched.
- `stress_context_management.rs` + `soak_live.rs` (compile targets — must migrate here, not later):
  - Delete the `ContextRecallTool` / `InMemoryOffloadStore` / `OffloadStore` imports and every `reg.register(Arc::new(ContextRecallTool::new(...)))` site; construct `Arc::new(agent_core::SessionArtifacts::new())` where the store was built; `OffloadEntry { id: 0, ... }` literals drop the `id` field.
  - **Registry surgery:** these files hand-build their `ToolRegistry` (they never call `build_registry`), so register the recovery tools explicitly: `reg.register(Arc::new(agent_tools::fs::ReadFile { max_bytes: 16 * 1024 }));` and `reg.register(Arc::new(agent_tools::fs::GrepTool));`.
  - **Backend for the test ToolCtx/loop:** the loop (or hand-built `ToolCtx`) must carry a composite mounting the test's `SessionArtifacts` — a bare HostBackend cannot see mem-backed artifacts, so migrated `read_file large_tool_results/…` calls would be NotFound. Build it exactly as assemble does (two `ReadOnlyToTools` mounts over `artifacts.results`/`artifacts.history`, `HostBackend` default) and attach via `.with_backend(...)`.
  - Scripted `context_recall {"id":1}` → `read_file {"path":"large_tool_results/<key>"}` where `<key>` = `{seq}-{sanitized call id}` from the fixture; recall `offset` paging → `byte_offset`; soak's injected prompt ("call context_recall with id 1", ~line 590) → "read_file the large_tool_results/ path named in the placeholder"; sink counters filtering `name == "context_recall"` → `name == "read_file"` with an args-path `large_tool_results/` prefix check.

- [ ] **Step 8: full suite + commit**

Run: `cd agent && cargo test && cd ../web && npm test && npx tsc --noEmit`
Expected: PASS across the workspace + web.

```bash
git add agent/crates web/src
git commit -m "feat(core): curation writes large_tool_results/ + conversation_history/ files; retire context_recall; Offloaded event carries path (spec §5.5-5.6, E2/E4/E6)"
```

---

### Task 9: Consumer sweep (periphery of the retirement)

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs` (`IMPLICIT_CHILD_TOOLS`, schema prose)
- Modify: `agent/crates/agent-core/tests/dispatch_tool.rs` (allowlist pin)
- Modify: `agent/crates/agent-memory/src/tools.rs` (recall disambiguation prose + its pin)
- Modify: `agent/crates/agent-tools/src/contract.rs` (`CONFUSABLE_TOOLS`)
- Modify: `agent/crates/agent-server/src/runtime.rs` (`CONTEXT_TOOLS` + architecture pin)
- Modify: `web/src/components/design/archFixture.ts`
- Modify: `agent/crates/agent-runtime-config/tests/stress_context_management.rs`, `agent/crates/agent-runtime-config/tests/soak_live.rs`

**Interfaces:**
- Consumes: Task 8's grammar and tool surface. Everything here is §5.6-table execution; no new API.

- [ ] **Step 1: dispatch**

Locate `const IMPLICIT_CHILD_TOOLS` (~dispatch.rs:403): `["context_compact"]` (drop `context_recall`; adjust the array length type). Locate the schema prose (~line 325) `"The child's context tools (context_recall, context_compact) are always available"` → `"The child's context tool (context_compact) is always available; offloaded content is recovered with the ordinary file tools."` In `dispatch_tool.rs::allowlist_accepts_always_available_context_tools`, drive `"tools": ["context_compact"]` and update the rejection-message assertion accordingly.

- [ ] **Step 2: memory prose + pin**

In `agent-memory/src/tools.rs`, locate the `recall` tool's `when_not_to_call` (`"…use context_recall"`) →

```rust
        Some(
            "Not for rehydrating offloaded conversation context — offloaded tool \
             results live as read-only files under large_tool_results/; use \
             read_file or grep there. Use recall for semantic search of saved memories.",
        )
```

Its pin (locate `wntc.contains("context_recall")`) → `assert!(wntc.contains("large_tool_results/"), "recall must disambiguate vs file-based offload recovery");`

- [ ] **Step 3: contract.rs + server classification + fixture**

- `contract.rs` `CONFUSABLE_TOOLS`: remove `"context_recall"`, add `"grep"` (the `recall`↔`grep` cluster replaces `recall`↔`context_recall`; both tools ship `when_not_to_call` so the ratchet passes). Update the cluster doc comment.
- `agent-server/src/runtime.rs`: `const CONTEXT_TOOLS: [&str; 1] = ["context_compact"];`; the architecture pin (locate `"context_recall/context_compact must be classified context"`) asserts only `context_compact` is `context` and adds `grep` classified with the file tools (match whatever bucket `read_file` uses in `architecture()`).
- `web/src/components/design/archFixture.ts`: replace the `context_recall` fixture entry with `{ name: "grep", … }` mirroring the neighboring file-tool entries' shape.
- **Web vitest assertion for the render fallback** (spec §7 gate note: typecheck alone cannot catch this — `detail.id` type-checks as `undefined`). In the web test file that covers `state.ts` (locate the existing `describeContext`/reducer tests; if none covers context events, add to the nearest `state` test file):

```typescript
it("renders offloaded events by path, falling back to legacy id", () => {
  expect(describeContext("offloaded", { tool: "shell", path: "large_tool_results/1-c1" }))
    .toBe("offloaded shell result → large_tool_results/1-c1");
  // Pre-Phase-2 trace replay: old events carry a numeric id, no path.
  expect(describeContext("offloaded", { tool: "shell", id: 4 }))
    .toBe("offloaded shell result → #4");
});
```

(Export `describeContext` from `state.ts` if it is module-private — a one-line `export` is acceptable test surface.)

- [ ] **Step 4: verify no stragglers**

The stress/soak migration happened in Task 8 (they are compile targets of the symbols Task 8 deletes). Here, confirm the retired surface has zero references left outside the spec/plan docs:

Run: `grep -rn "context_recall\|OffloadStore\|InMemoryOffloadStore" agent/crates web/src --include=*.rs --include=*.ts`
Expected: no hits (eval_context.rs is Task 11's scope — if it appears, that is the only acceptable remainder at this point).

Run: `cd agent && cargo test -p agent-core -p agent-memory -p agent-tools -p agent-server -p agent-runtime-config` and `cd ../web && npx tsc --noEmit && npm test`
Expected: PASS (soak/stress live tests compile; `#[ignore]`d ones checked via `cargo test --no-run`).

- [ ] **Step 5: Commit**

```bash
git add agent/crates web/src
git commit -m "refactor(core): retire context_recall across the consumer surface (spec §5.6 table)"
```

---

### Task 10: New integration pins (guard, round trips, children, recovery)

**Files:**
- Modify: `agent/crates/agent-runtime-config/tests/e2e_context_management.rs` (guard + deep-recovery pins)
- Modify: `agent/crates/agent-core/src/curated.rs` (INCOMPLETE-pointer unit tests)
- Modify: `agent/crates/agent-core/tests/dispatch_tool.rs` (child isolation pins)

**Interfaces:**
- Consumes: everything shipped in Tasks 1–9. Produces only tests (spec §7 "New tests" — the loop-level subset not already added in Tasks 1–9).

- [ ] **Step 1: Guard pin (loop-level)**

In `e2e_context_management.rs` add:

```rust
#[tokio::test]
async fn model_write_into_artifacts_is_denied_and_bytes_survive() {
    // Turn 1 offloads a big result; turn 2 the model tries to overwrite the
    // artifact; turn 3 reads it back — original bytes, not the forgery.
    // (spec §5.2 ReadOnlyToTools; §7 guard pin)
    // Build the loop exactly as offload_then_read_file_round_trips_through_the_loop
    // does, then script:
    //   Scripted::Call("c2", "write_file", r#"{"path":"large_tool_results/1-c1","content":"forged"}"#)
    //   Scripted::Call("c3", "read_file",  r#"{"path":"large_tool_results/1-c1"}"#)
    // Assert: the c2 tool result content starts with "ERROR: denied:" and
    // contains "read-only records of offloaded context"; the c3 result equals
    // the original oversized payload byte-for-byte.
}
```

Write the body by cloning the round-trip test's fixture from Task 8 Step 7 (same `build_loop`, same oversized first result) with the two extra scripted calls and the two assertions stated in the comment. No new helpers needed.

- [ ] **Step 2: Deep-recovery pin**

```rust
#[tokio::test]
async fn deep_recovery_is_grep_then_read_file_in_two_calls() {
    // Seed a CuratedContext-owned history.md with three sections via real
    // folds/compactions (drive maintain with a scripted extraction model),
    // then: grep("## folded-2", Some("conversation_history/")) → exactly one
    // hit carrying a line number; read_file(path, offset: hit.line) returns
    // content containing the span's marker fact. Two tool calls total
    // (spec §5.5 deep-recovery recipe).
}
```

Implement against the composite directly (construct `SessionArtifacts`, a `CuratedContext` with three folds using the Task-8 fold fixture pattern, then a `CompositeBackend` mounting the same artifacts) — tool-level, not loop-level, is sufficient and fast: call `GrepTool` then `ReadFile { max_bytes: 16*1024 }` with a `ToolCtx` whose `backend` is that composite.

- [ ] **Step 3: INCOMPLETE pointer unit tests (curated.rs)**

```rust
    /// A history backend whose writes always fail (E4 pin).
    struct FailingHistory;
    #[async_trait::async_trait]
    impl agent_tools::backend::Backend for FailingHistory {
        async fn ls(&self, _: &str) -> Result<Vec<agent_tools::backend::Entry>, agent_tools::backend::FsError> { Ok(vec![]) }
        async fn read(&self, p: &str) -> Result<String, agent_tools::backend::FsError> {
            Err(agent_tools::backend::FsError::NotFound(p.into()))
        }
        async fn write(&self, _: &str, _: &str) -> Result<(), agent_tools::backend::FsError> {
            Err(agent_tools::backend::FsError::Io("disk on fire".into()))
        }
        async fn glob(&self, _: &str) -> Result<Vec<String>, agent_tools::backend::FsError> { Ok(vec![]) }
        async fn grep(&self, _: &str, _: Option<&str>) -> Result<Vec<agent_tools::backend::GrepHit>, agent_tools::backend::FsError> { Ok(vec![]) }
        async fn delete(&self, p: &str) -> Result<(), agent_tools::backend::FsError> {
            Err(agent_tools::backend::FsError::NotFound(p.into()))
        }
    }

    #[tokio::test]
    async fn failed_history_write_still_commits_compaction_with_incomplete_pointer() {
        // Same fixture as maintain_compacts_old_span_when_over_high_water, but
        // artifacts.history = FailingHistory. Assert: report.compacted_turns > 0
        // (E4: commit), and build() contains "INCOMPLETE — at least one span
        // failed to record".
    }

    #[tokio::test]
    async fn successful_compaction_renders_the_complete_pointer_and_it_survives_recompaction() {
        // Two forced compactions; after each, build() contains
        // "Evicted transcripts: conversation_history/history.md" and never the
        // INCOMPLETE marker; history.md contains "## compacted-" twice.
    }

    #[tokio::test]
    async fn failed_history_write_aborts_the_fold_atomically() {
        // fold fixture + FailingHistory: store nothing, history intact, no
        // ledger block (mirrors fold_extraction_failure_leaves_history_intact).
    }
```

Fill the bodies from the named existing fixtures (they are in the same file; copy their setup verbatim and swap the artifacts handle: `SessionArtifacts { results: Arc::new(MemBackend::new()), history: Arc::new(FailingHistory) }`).

- [ ] **Step 4: Child isolation pins (dispatch_tool.rs)**

```rust
#[tokio::test]
async fn parent_read_of_child_artifact_path_is_not_found_never_cross_tenant() {
    // Parent offloads its own artifact (key "1-p"); child artifact keys are
    // prefixed "sub1-" (spec §5.7), so a parent read_file of
    // "large_tool_results/sub1-1-c" must be NotFound even when the parent's
    // results store has entries. Drive: seed parent SessionArtifacts.results
    // with "1-p"; execute ReadFile against the PARENT composite for the
    // child-shaped path; assert ToolError::NotFound.
}

#[tokio::test]
async fn child_reads_its_own_artifacts_and_shares_workspace() {
    // Dispatch a scripted child whose turn-1 tool result is oversized (child
    // curation offloads it to the CHILD store), turn-2 read_file of the
    // child-cited path succeeds inside the child; and a workspace file written
    // by the test is readable by the child's read_file. Assert both from the
    // child's captured tool results (CollectingSink on the child stream).
}
```

Model the scripted-child machinery on the existing `child_stack_is_exactly_curation_and_stuck_detection_never_memory_recall` test in the same file (it already builds a dispatch with a scripted child model — reuse its harness).

- [ ] **Step 5: Run + commit**

Run: `cd agent && cargo test`
Expected: PASS.

```bash
git add agent/crates
git commit -m "test(core): guard, deep-recovery, INCOMPLETE-pointer, and child-isolation pins (spec §7 new tests)"
```

---

### Task 11: Eval-harness migration (E5)

**Files:**
- Modify: `agent/crates/agent-runtime-config/tests/eval_context.rs`

**Interfaces:**
- Consumes: Tasks 6–9 surface. Deliverable: the harness **compiles and its non-live tests run** against the new grammar. Ceiling re-measurement is explicitly NOT this task (memory note `context-evolve-needs-backend-migration`).

- [ ] **Step 1: Apply the migration mapping**

Read `eval_context.rs` end-to-end first (Task 8 already swapped its construction sites so it compiles; everything semantic was deliberately left). Apply the same mapping as Task 8's stress/soak bullet:
- Any task-driver prompt or scripted call using `context_recall` → `read_file` with the placeholder-cited `large_tool_results/…` path (`byte_offset` for paging).
- Assertions on placeholder strings → the two new skip literals / path grammar.
- Window/config knobs (drift-ledger window 4000, abs paths — see memory `context-evolve-harness`) are untouched.

- [ ] **Step 2: Compile-verify the live tests, run the rest**

Run: `cd agent && cargo test -p agent-runtime-config --test eval_context --no-run && cargo test -p agent-runtime-config --test eval_context`
Expected: compiles; non-`#[ignore]` tests PASS; live/ignored tests compile only.

- [ ] **Step 3: Commit**

```bash
git add agent/crates/agent-runtime-config/tests/eval_context.rs
git commit -m "test(eval): migrate context-evolve harness to file-based offload recovery (E5 gate decision; ceilings re-measurement stays parked)"
```

---

### Task 12: Docs, spec log, full CI

**Files:**
- Modify: `docs/okf/deepagents-refactor/comparisons/capability-gap-analysis.md` (multimodal row)
- Modify: `docs/superpowers/specs/2026-07-08-backend-seam-design.md` (implementation-notes log entries)

**Interfaces:** none (docs + gate).

- [ ] **Step 1: Gap-analysis row**

Locate the multimodal row (`| Multimodal files | … | **unassessed** |`) and change the current-runtime cell + verdict to: `Text-only end-to-end (read_to_string; String content in ToolOutput/Message); assessed 2026-07-08 (Phase-2 spec J3)` / `**absent** (deliberately deferred)`. Run `python3 scripts/okf_check.py` (via `bash scripts/ci.sh` in Step 3) to confirm the bundle still lints.

- [ ] **Step 2: Spec implementation notes**

Append to the spec's Panel & review log (dated implementation-note bullets, mirroring Phase 1's practice):

```markdown
- 2026-07-08 — implementation notes (plan/build): (1) `SessionArtifacts`
  fields shipped as `Arc<dyn Backend>` rather than concrete `Arc<MemBackend>`
  — still the privileged unwrapped handles §5.3 requires, and it lets the E4
  INCOMPLETE pin inject a failing history backend. (2) With `context_recall`
  retired, `ContextCurationMiddleware` needs only the compact flag —
  `new(flag)`, not §5.3's sketched `new(artifacts, flag, cap)`; the artifacts
  handle is consumed by `CuratedContext` alone. (3) `build_registry` gained a
  `max_read_bytes` parameter to thread the §5.4 read cap to `ReadFile`.
  (4) `LoopConfig` was NOT extended with a `backend` field (§5.3 sketched
  one): the default HostBackend is derived in `AgentLoop::new` from
  `config.workspace` and overridden via `with_backend`, keeping the wave-1
  config surface and its `Default` untouched.
```

- [ ] **Step 3: Full gate**

Run: `bash scripts/ci.sh`
Expected: green end-to-end (okf check, skills lint, fmt, clippy, cargo test, conditional src-tauri, web typecheck + vitest).

- [ ] **Step 4: Commit**

```bash
git add docs
git commit -m "docs(spec): Phase-2 implementation notes + gap-analysis multimodal row assessed (J3)"
```

---

## Review log

- 2026-07-08 — single plan review (opus, per AGENTS.md: spec coverage /
  decomposition / buildability): **APPROVE-WITH-FIXES**, all applied —
  (1 BLOCKER) stress/soak are compile targets of Task-8-deleted symbols;
  their migration moved from Task 9 into Task 8 (+ the same logic applied to
  eval_context.rs: construction-site fix in Task 8, semantics stay Task 11);
  (2) stress/soak registry surgery + composite-backed test ToolCtx added;
  (3) `is_durable_placeholder_unit` — the fifth lockstep placeholder site —
  given its explicit edit; (4) ToolCtx grep-exhaustiveness note (double
  sites); (5) `LoopConfig`-backend deviation logged for the spec (Task 12).
  Verified clean: LoopConfig::Default derive pattern, lift borrow shape,
  MemBackend lock discipline, middleware `new(flag)` reduction, every §5.6
  consumer row, Task-10 fixture names, Cargo deps, spec exclusions.

## Post-plan checklist (for the executor)

- After all tasks: whole-branch review per repo SDLC, then `superpowers:finishing-a-development-branch` (merge decision is the owner's; never push).
- `graphify . --update` after merge needs an LLM API key for the doc deltas (known env gap — flag to the owner if absent).
