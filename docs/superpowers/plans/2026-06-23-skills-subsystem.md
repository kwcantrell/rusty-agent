# Skills Subsystem Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a new `agent-skills` crate that lets the agent discover, load-on-demand, author, and preload (as presets) markdown skill packages — attaching only through the `Tool` seam + binary wiring, leaving the four core crates untouched.

**Architecture:** A new workspace crate `agent-skills` (mirroring the `agent-http` precedent) defines a `Skill` model, a re-scannable `SkillRegistry`, and four tools (`list_skills`, `use_skill`, `read_skill_file`, `create_skill`) that implement `agent_tools::Tool`. The tools hold an `Arc<SkillRegistry>` and re-scan per call, so the tool *surface* is fixed while the *catalog* is dynamic (authored skills appear with no loop rebuild). Progressive disclosure falls out of the existing tool-result path. Presets are skill bodies composed into the system prompt at startup, in the binaries only. The subsystem never executes anything — bundled scripts run through the existing `execute_command` + policy + approval seam.

**Tech Stack:** Rust 2021, `async-trait`, `serde_json`, `tokio`, `tracing`; `tempfile` for tests. No new third-party runtime dependencies (frontmatter is parsed with a minimal line parser; no YAML crate).

## Global Constraints

- **Cargo is not on PATH:** every `cargo` command assumes you ran `source "$HOME/.cargo/env"` first. The run commands below include it.
- **Build/test from `agent/`:** `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings` must stay green (the standing bar).
- **Core crates frozen:** do NOT modify `agent-core`, `agent-model`, `agent-tools`, or `agent-policy`. New code lives in the new `agent-skills` crate plus wiring in `agent-runtime-config`, `agent-cli`, and `agent-server`.
- **No wire/web/cloud changes:** do NOT touch `RuntimeConfig`, the wire protocol, `cloud/`, or `web/`. `--skills-dir`/`--skill` are launch flags only; persisting them into `RuntimeConfig` is deferred to the Settings capability cycle (it would otherwise let a browser settings-save silently wipe skill config).
- **Execution model:** the skills subsystem never executes scripts. It surfaces script paths; running them goes through the existing `execute_command` tool + command allow/deny policy + approval gate. No new execution authority.
- **Guards are lexical:** path containment normalizes `.`/`..` without symlink resolution (same discipline as `agent-tools`' `resolve_in_workspace`). OS sandboxing stays deferred to #2.
- **Test idiom:** inline `#[cfg(test)] mod tests` per source file, matching the codebase.
- **Canonical skill name = directory name.** Frontmatter must provide a non-empty `description`; a frontmatter `name`, if present, must equal the directory name or the skill is skipped with a warning.

---

### Task 1: Scaffold `agent-skills` crate + `Skill` model + frontmatter parser

**Files:**
- Create: `agent/crates/agent-skills/Cargo.toml`
- Create: `agent/crates/agent-skills/src/lib.rs`
- Create: `agent/crates/agent-skills/src/skill.rs`

**Interfaces:**
- Produces: `agent_skills::skill::{Skill, ParsedSkill, parse_skill_md}`.
  - `pub struct Skill { pub name: String, pub description: String, pub body: String, pub dir: std::path::PathBuf, pub files: Vec<std::path::PathBuf> }`
  - `pub struct ParsedSkill { pub name: Option<String>, pub description: String, pub body: String }`
  - `pub fn parse_skill_md(text: &str) -> Result<ParsedSkill, String>`

- [ ] **Step 1: Create the crate manifest**

Create `agent/crates/agent-skills/Cargo.toml`:

```toml
[package]
name = "agent-skills"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
agent-tools = { path = "../agent-tools" }
async-trait = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
agent-policy = { path = "../agent-policy" }
tempfile = { workspace = true }
```

- [ ] **Step 2: Create the crate root with module declarations**

Create `agent/crates/agent-skills/src/lib.rs`:

```rust
//! Skills subsystem: discover, load-on-demand, author, and preload markdown
//! skill packages. Attaches to the agent core only through the `Tool` seam.

pub mod guard;
pub mod presets;
pub mod registry;
pub mod skill;
pub mod tools;

pub use presets::{compose_system_prompt, SKILLS_AWARENESS};
pub use registry::{sanitize_slug, SkillRegistry};
pub use skill::Skill;
pub use tools::{CreateSkill, ListSkills, ReadSkillFile, UseSkill};
```

Note: `guard`, `presets`, `registry`, and `tools` do not exist yet — this file will not compile until their tasks land. Create the empty module files now so the crate compiles after Task 1's `skill.rs`:

- Create `agent/crates/agent-skills/src/guard.rs` containing only `// implemented in Task 2`
- Create `agent/crates/agent-skills/src/presets.rs` containing only `// implemented in Task 7`
- Create `agent/crates/agent-skills/src/registry.rs` containing only `// implemented in Task 3`
- Create `agent/crates/agent-skills/src/tools.rs` containing only `// implemented in Task 4-6`

Then temporarily comment out the `pub use` lines and the `pub mod guard/presets/registry/tools;` lines in `lib.rs` EXCEPT `pub mod skill;`, so Task 1 compiles standalone:

```rust
//! Skills subsystem: discover, load-on-demand, author, and preload markdown
//! skill packages. Attaches to the agent core only through the `Tool` seam.

pub mod skill;
pub use skill::Skill;

// Uncommented as later tasks land:
// pub mod guard;
// pub mod presets;
// pub mod registry;
// pub mod tools;
// pub use presets::{compose_system_prompt, SKILLS_AWARENESS};
// pub use registry::{sanitize_slug, SkillRegistry};
// pub use tools::{CreateSkill, ListSkills, ReadSkillFile, UseSkill};
```

- [ ] **Step 3: Write the failing test for the frontmatter parser**

Create `agent/crates/agent-skills/src/skill.rs`:

```rust
use std::path::PathBuf;

/// A parsed skill: identity, markdown body, and bundled files (absolute paths).
#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub dir: PathBuf,
    pub files: Vec<PathBuf>,
}

/// The result of parsing a SKILL.md's text (before a directory/name is attached).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSkill {
    pub name: Option<String>,
    pub description: String,
    pub body: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let text = "---\nname: my-skill\ndescription: Does a thing\n---\n\n# Body\nDo the thing.\n";
        let p = parse_skill_md(text).unwrap();
        assert_eq!(p.name.as_deref(), Some("my-skill"));
        assert_eq!(p.description, "Does a thing");
        assert_eq!(p.body, "# Body\nDo the thing.");
    }

    #[test]
    fn strips_surrounding_quotes_from_values() {
        let text = "---\ndescription: \"Quoted desc\"\n---\nbody\n";
        let p = parse_skill_md(text).unwrap();
        assert_eq!(p.description, "Quoted desc");
    }

    #[test]
    fn missing_frontmatter_is_error() {
        let err = parse_skill_md("no front matter here").unwrap_err();
        assert!(err.contains("front matter"));
    }

    #[test]
    fn missing_description_is_error() {
        let err = parse_skill_md("---\nname: x\n---\nbody").unwrap_err();
        assert!(err.contains("description"));
    }

    #[test]
    fn unterminated_frontmatter_is_error() {
        let err = parse_skill_md("---\ndescription: x\nbody with no close").unwrap_err();
        assert!(err.contains("front matter"));
    }
}
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills skill 2>&1 | tail -20`
Expected: FAIL — `cannot find function parse_skill_md`.

- [ ] **Step 5: Implement the parser**

Add to `agent/crates/agent-skills/src/skill.rs` (above the `#[cfg(test)]` block):

```rust
/// Parse a SKILL.md into `(name?, description, body)`. The frontmatter is a
/// leading `---` ... `---` block of simple `key: value` lines (single-line
/// scalar values only; surrounding quotes are stripped). Returns `Err` if the
/// frontmatter block is missing/unterminated or lacks a non-empty `description`.
pub fn parse_skill_md(text: &str) -> Result<ParsedSkill, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text); // tolerate a BOM
    let mut lines = text.lines();
    // First non-empty line must be the opening fence.
    let opened = lines.by_ref().find(|l| !l.trim().is_empty());
    if opened.map(str::trim) != Some("---") {
        return Err("missing front matter (file must start with a `---` block)".into());
    }
    let mut name = None;
    let mut description = None;
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = unquote(v.trim());
            match key {
                "name" => name = Some(val.to_string()),
                "description" => description = Some(val.to_string()),
                _ => {} // ignore unknown keys for forward-compat
            }
        }
    }
    if !closed {
        return Err("unterminated front matter (no closing `---`)".into());
    }
    let description = description
        .filter(|d| !d.trim().is_empty())
        .ok_or("front matter is missing a non-empty `description`")?;
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    Ok(ParsedSkill { name, description, body })
}

fn unquote(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills skill 2>&1 | tail -20`
Expected: PASS (5 tests).

- [ ] **Step 7: Commit**

```bash
cd agent && git add crates/agent-skills/Cargo.toml crates/agent-skills/src
git commit -m "feat(skills): scaffold agent-skills crate + SKILL.md frontmatter parser"
```

---

### Task 2: Skill-directory path guard

**Files:**
- Modify: `agent/crates/agent-skills/src/guard.rs` (replace the placeholder)
- Modify: `agent/crates/agent-skills/src/lib.rs` (enable the `guard` module)

**Interfaces:**
- Produces: `agent_skills::guard::resolve_in_dir(base_dir: &std::path::Path, rel: &str) -> Result<std::path::PathBuf, String>` — lexical containment within `base_dir`, rejecting absolute paths and `..` escapes.

- [ ] **Step 1: Enable the module**

In `agent/crates/agent-skills/src/lib.rs`, uncomment `pub mod guard;`.

- [ ] **Step 2: Write the failing test**

Replace the contents of `agent/crates/agent-skills/src/guard.rs` with:

```rust
use std::path::{Component, Path, PathBuf};

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
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills guard 2>&1 | tail -20`
Expected: FAIL — `cannot find function resolve_in_dir`.

- [ ] **Step 4: Implement the guard**

Add to `agent/crates/agent-skills/src/guard.rs` (above the test block):

```rust
/// Resolve `rel` against `base_dir`, rejecting absolute paths and any `..`
/// escape. Lexical only — does not resolve symlinks (same limitation as the
/// workspace guard in `agent-tools`).
pub fn resolve_in_dir(base_dir: &Path, rel: &str) -> Result<PathBuf, String> {
    if Path::new(rel).is_absolute() {
        return Err(format!("path must be relative to the skill directory: {rel}"));
    }
    let candidate = base_dir.join(rel);
    let normalized = normalize(&candidate);
    let base_norm = normalize(base_dir);
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
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills guard 2>&1 | tail -20`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
cd agent && git add crates/agent-skills/src/guard.rs crates/agent-skills/src/lib.rs
git commit -m "feat(skills): lexical skill-directory path guard"
```

---

### Task 3: `SkillRegistry` — discovery, config defaults, slug sanitization

**Files:**
- Modify: `agent/crates/agent-skills/src/registry.rs` (replace the placeholder)
- Modify: `agent/crates/agent-skills/src/lib.rs` (enable the `registry` module + re-exports)

**Interfaces:**
- Consumes: `crate::skill::{Skill, parse_skill_md}`.
- Produces:
  - `pub struct SkillRegistry` with:
    - `pub fn new(roots: Vec<PathBuf>, writable_root: PathBuf) -> Self`
    - `pub fn from_config(skills_dirs: &[String], workspace: &std::path::Path) -> Self`
    - `pub fn writable_root(&self) -> &std::path::Path`
    - `pub fn scan(&self) -> Vec<Skill>` (earlier root wins on duplicate name; malformed skipped+warned; sorted by name)
    - `pub fn find(&self, name: &str) -> Option<Skill>`
  - `pub fn sanitize_slug(name: &str) -> Result<String, String>`

- [ ] **Step 1: Enable the module + re-exports**

In `agent/crates/agent-skills/src/lib.rs`, uncomment `pub mod registry;` and `pub use registry::{sanitize_slug, SkillRegistry};`.

- [ ] **Step 2: Write the failing tests**

Replace the contents of `agent/crates/agent-skills/src/registry.rs` with:

```rust
use crate::skill::{parse_skill_md, Skill};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_skill(root: &Path, name: &str, desc: &str, body: &str) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {desc}\n---\n{body}\n")).unwrap();
    }

    #[test]
    fn scan_discovers_skills_sorted() {
        let dir = tempdir().unwrap();
        write_skill(dir.path(), "beta", "B skill", "b body");
        write_skill(dir.path(), "alpha", "A skill", "a body");
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let found = reg.scan();
        assert_eq!(found.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["alpha", "beta"]);
        assert_eq!(found[0].description, "A skill");
    }

    #[test]
    fn earlier_root_wins_on_name_conflict() {
        let proj = tempdir().unwrap();
        let user = tempdir().unwrap();
        write_skill(proj.path(), "dup", "from project", "p");
        write_skill(user.path(), "dup", "from user", "u");
        let reg = SkillRegistry::new(
            vec![proj.path().to_path_buf(), user.path().to_path_buf()],
            proj.path().to_path_buf());
        let found = reg.scan();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].description, "from project");
    }

    #[test]
    fn malformed_skill_is_skipped_not_fatal() {
        let dir = tempdir().unwrap();
        write_skill(dir.path(), "good", "ok", "body");
        let bad = dir.path().join("bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("SKILL.md"), "no front matter").unwrap();
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let found = reg.scan();
        assert_eq!(found.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["good"]);
    }

    #[test]
    fn frontmatter_name_mismatch_is_skipped() {
        let dir = tempdir().unwrap();
        let d = dir.path().join("realdir");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("SKILL.md"), "---\nname: other\ndescription: x\n---\nbody").unwrap();
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        assert!(reg.scan().is_empty());
    }

    #[test]
    fn scan_collects_bundled_files() {
        let dir = tempdir().unwrap();
        write_skill(dir.path(), "withfiles", "x", "body");
        fs::write(dir.path().join("withfiles").join("run.sh"), "echo hi").unwrap();
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let s = reg.find("withfiles").unwrap();
        assert_eq!(s.files.len(), 1);
        assert!(s.files[0].ends_with("run.sh"));
    }

    #[test]
    fn missing_root_is_empty_not_error() {
        let reg = SkillRegistry::new(vec![PathBuf::from("/nonexistent/xyz")], PathBuf::from("/tmp"));
        assert!(reg.scan().is_empty());
    }

    #[test]
    fn from_config_uses_explicit_dirs() {
        let reg = SkillRegistry::from_config(&["/a".into(), "/b".into()], Path::new("/ws"));
        assert_eq!(reg.writable_root(), Path::new("/a"));
    }

    #[test]
    fn from_config_defaults_to_workspace_dotagent() {
        let reg = SkillRegistry::from_config(&[], Path::new("/ws"));
        assert_eq!(reg.writable_root(), Path::new("/ws/.agent/skills"));
    }

    #[test]
    fn sanitize_slug_normalizes_and_rejects_traversal() {
        assert_eq!(sanitize_slug("My Cool Skill").unwrap(), "my-cool-skill");
        assert_eq!(sanitize_slug("a__b").unwrap(), "a-b");
        assert!(sanitize_slug("../evil").is_err());
        assert!(sanitize_slug("a/b").is_err());
        assert!(sanitize_slug("   ").is_err());
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills registry 2>&1 | tail -20`
Expected: FAIL — `cannot find ... SkillRegistry`.

- [ ] **Step 4: Implement the registry**

Add to `agent/crates/agent-skills/src/registry.rs` (above the test block):

```rust
/// Discovers skills from one or more roots and is the authoring target. Cheap to
/// re-scan, so callers re-scan per tool invocation and freshly authored skills
/// appear immediately without rebuilding the agent loop.
pub struct SkillRegistry {
    roots: Vec<PathBuf>,
    writable_root: PathBuf,
}

impl SkillRegistry {
    pub fn new(roots: Vec<PathBuf>, writable_root: PathBuf) -> Self {
        Self { roots, writable_root }
    }

    /// Explicit `--skills-dir` roots if any (writable = first); otherwise the
    /// defaults: `<workspace>/.agent/skills` (writable) + `~/.agent/skills`.
    pub fn from_config(skills_dirs: &[String], workspace: &Path) -> Self {
        if let Some((first, rest)) = skills_dirs.split_first() {
            let mut roots = vec![PathBuf::from(first)];
            roots.extend(rest.iter().map(PathBuf::from));
            Self::new(roots, PathBuf::from(first))
        } else {
            let project = workspace.join(".agent").join("skills");
            let mut roots = vec![project.clone()];
            if let Some(home) = std::env::var_os("HOME") {
                roots.push(PathBuf::from(home).join(".agent").join("skills"));
            }
            Self::new(roots, project)
        }
    }

    pub fn writable_root(&self) -> &Path {
        &self.writable_root
    }

    /// Walk every root for `<dir>/SKILL.md`. Earlier roots win on duplicate
    /// names; malformed skills are skipped with a warning. Result is sorted by name.
    pub fn scan(&self) -> Vec<Skill> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<Skill> = Vec::new();
        for root in &self.roots {
            let read = match std::fs::read_dir(root) {
                Ok(r) => r,
                Err(_) => continue, // missing/unreadable root → no skills here
            };
            let mut dirs: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            dirs.sort();
            for dir in dirs {
                if !dir.join("SKILL.md").is_file() {
                    continue;
                }
                let name = match dir.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if seen.contains(&name) {
                    continue;
                }
                match load_skill(&dir, &name) {
                    Ok(skill) => {
                        seen.insert(name);
                        out.push(skill);
                    }
                    Err(e) => tracing::warn!(skill = %name, error = %e, "skipping malformed skill"),
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn find(&self, name: &str) -> Option<Skill> {
        self.scan().into_iter().find(|s| s.name == name)
    }
}

fn load_skill(dir: &Path, dir_name: &str) -> Result<Skill, String> {
    let text = std::fs::read_to_string(dir.join("SKILL.md")).map_err(|e| format!("read SKILL.md: {e}"))?;
    let parsed = parse_skill_md(&text)?;
    if let Some(fm_name) = &parsed.name {
        if fm_name != dir_name {
            return Err(format!("frontmatter name '{fm_name}' != directory '{dir_name}'"));
        }
    }
    Ok(Skill {
        name: dir_name.to_string(),
        description: parsed.description,
        body: parsed.body,
        dir: dir.to_path_buf(),
        files: list_bundled_files(dir),
    })
}

fn list_bundled_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for e in read.flatten() {
            let p = e.path();
            if p.is_file() && p.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
                files.push(p);
            }
        }
    }
    files.sort();
    files
}

/// Validate + slugify a skill name: ASCII alphanumerics kept, anything else
/// becomes a single hyphen, trimmed. Rejects empty/oversize names and explicit
/// traversal before slugging (defense in depth).
pub fn sanitize_slug(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("skill name is empty".into());
    }
    if trimmed.chars().count() > 64 {
        return Err("skill name too long (max 64 chars)".into());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(format!("invalid skill name (no path separators): {trimmed}"));
    }
    let mut slug = String::new();
    let mut prev_dash = false;
    for c in trimmed.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        return Err(format!("skill name has no usable characters: {name}"));
    }
    Ok(slug)
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills registry 2>&1 | tail -20`
Expected: PASS (9 tests).

- [ ] **Step 6: Commit**

```bash
cd agent && git add crates/agent-skills/src/registry.rs crates/agent-skills/src/lib.rs
git commit -m "feat(skills): SkillRegistry discovery + config defaults + slug sanitization"
```

---

### Task 4: `list_skills` + `use_skill` tools

**Files:**
- Modify: `agent/crates/agent-skills/src/tools.rs` (replace the placeholder; this task adds `tools/mod`-style content directly in `tools.rs`)
- Modify: `agent/crates/agent-skills/src/lib.rs` (enable the `tools` module + re-exports)

**Interfaces:**
- Consumes: `crate::registry::SkillRegistry`, `agent_tools::{Tool, ToolSchema, ToolIntent, ToolOutput, ToolError, ToolCtx, Access}`.
- Produces:
  - `pub struct ListSkills { ... }` with `pub fn new(registry: std::sync::Arc<SkillRegistry>) -> Self`; tool name `"list_skills"`.
  - `pub struct UseSkill { ... }` with `pub fn new(registry: std::sync::Arc<SkillRegistry>) -> Self`; tool name `"use_skill"`.

- [ ] **Step 1: Enable the module + re-exports**

In `agent/crates/agent-skills/src/lib.rs`, uncomment `pub mod tools;` and the `pub use tools::{CreateSkill, ListSkills, ReadSkillFile, UseSkill};` line.

- [ ] **Step 2: Write the failing tests**

Replace the contents of `agent/crates/agent-skills/src/tools.rs` with:

```rust
use crate::guard::resolve_in_dir;
use crate::registry::{sanitize_slug, SkillRegistry};
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_FILES: usize = 32;

/// List available skills (name + when-to-use).
pub struct ListSkills {
    registry: Arc<SkillRegistry>,
}

impl ListSkills {
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ListSkills {
    fn name(&self) -> &str {
        "list_skills"
    }
    fn description(&self) -> &str {
        "List available skills (name + when to use). Call this before a task to see if a skill applies."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }
    fn intent(&self, _args: &Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "list_skills".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "list available skills".into(),
        })
    }
    async fn execute(&self, _args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let skills = self.registry.scan();
        if skills.is_empty() {
            return Ok(ToolOutput {
                content: "No skills available.".into(),
                display: None,
            });
        }
        let mut content = String::from("Available skills (load one with use_skill):\n");
        for s in &skills {
            content.push_str(&format!("- {}: {}\n", s.name, s.description));
        }
        Ok(ToolOutput { content, display: None })
    }
}

/// Load a skill's full body + bundled-file manifest into context.
pub struct UseSkill {
    registry: Arc<SkillRegistry>,
}

impl UseSkill {
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
        Self { registry }
    }
}

fn name_arg(args: &Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing '{key}' string")))
}

#[async_trait]
impl Tool for UseSkill {
    fn name(&self) -> &str {
        "use_skill"
    }
    fn description(&self) -> &str {
        "Load a skill by name: returns its full instructions plus the paths of any bundled files."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": { "name": { "type": "string", "description": "Skill name from list_skills." } },
                "required": ["name"]
            }),
        }
    }
    fn intent(&self, args: &Value) -> Result<ToolIntent, ToolError> {
        let name = name_arg(args, "name")?;
        Ok(ToolIntent {
            tool: "use_skill".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: format!("load skill {name}"),
        })
    }
    async fn execute(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let name = name_arg(&args, "name")?;
        let skill = self.registry.find(&name).ok_or_else(|| {
            let avail: Vec<String> = self.registry.scan().into_iter().map(|s| s.name).collect();
            ToolError::NotFound(format!("skill '{name}' not found. Available: {}", avail.join(", ")))
        })?;
        let mut content = format!("# Skill: {}\n\n{}\n", skill.name, skill.body);
        if skill.files.is_empty() {
            content.push_str("\n(No bundled files.)\n");
        } else {
            content.push_str("\n## Bundled files\n");
            for f in &skill.files {
                content.push_str(&format!("- {}\n", f.display()));
            }
            content.push_str(
                "\nRead a bundled file with read_skill_file; run a bundled script with execute_command.\n",
            );
        }
        Ok(ToolOutput { content, display: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_tools::ToolCtx;
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;
    use tempfile::{tempdir, TempDir};
    use tokio_util::sync::CancellationToken;

    fn ctx() -> ToolCtx {
        ToolCtx {
            workspace: std::env::temp_dir(),
            timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(),
        }
    }

    fn reg_with_skill(name: &str, body: &str, files: &[(&str, &str)]) -> (Arc<SkillRegistry>, TempDir) {
        let dir = tempdir().unwrap();
        let sdir = dir.path().join(name);
        fs::create_dir_all(&sdir).unwrap();
        fs::write(sdir.join("SKILL.md"), format!("---\nname: {name}\ndescription: d\n---\n{body}")).unwrap();
        for (fname, content) in files {
            fs::write(sdir.join(fname), content).unwrap();
        }
        let reg = Arc::new(SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf()));
        (reg, dir)
    }

    #[tokio::test]
    async fn list_skills_reports_catalog() {
        let (reg, _d) = reg_with_skill("alpha", "body", &[]);
        let out = ListSkills::new(reg).execute(json!({}), &ctx()).await.unwrap();
        assert!(out.content.contains("alpha: d"));
    }

    #[tokio::test]
    async fn list_skills_empty_is_clean() {
        let dir = tempdir().unwrap();
        let reg = Arc::new(SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf()));
        let out = ListSkills::new(reg).execute(json!({}), &ctx()).await.unwrap();
        assert!(out.content.contains("No skills"));
    }

    #[tokio::test]
    async fn use_skill_returns_body_and_files() {
        let (reg, _d) = reg_with_skill("alpha", "Step one.", &[("run.sh", "echo hi")]);
        let out = UseSkill::new(reg).execute(json!({"name": "alpha"}), &ctx()).await.unwrap();
        assert!(out.content.contains("Step one."));
        assert!(out.content.contains("run.sh"));
        assert!(out.content.contains("execute_command"));
    }

    #[tokio::test]
    async fn use_skill_unknown_is_not_found() {
        let (reg, _d) = reg_with_skill("alpha", "body", &[]);
        let err = UseSkill::new(reg).execute(json!({"name": "missing"}), &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }
}
```

Note: the `use` statements at the top reference `resolve_in_dir`, `sanitize_slug`, and the size consts that Tasks 5–6 use. They are unused after Task 4 alone, which would trip `-D warnings`. To keep Task 4 self-contained and clippy-clean, add `#[allow(unused_imports)]` above the `use crate::guard::resolve_in_dir;` and `use crate::registry::{sanitize_slug, SkillRegistry};` lines AND `#[allow(dead_code)]` above the three `const MAX_*` lines for now; Task 6 removes these allows when it consumes them.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills tools 2>&1 | tail -20`
Expected: FAIL — the structs/methods don't resolve until the implementation above is in place. (If you pasted the full block, this step confirms the test names compile and run; if you staged the test block before the impl, expect a compile error naming `ListSkills`.)

- [ ] **Step 4: Confirm the implementation is present**

The impl is included in Step 2's block. Re-read `tools.rs` to confirm both `ListSkills` and `UseSkill` `impl Tool` blocks exist above the tests.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills tools 2>&1 | tail -20`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
cd agent && git add crates/agent-skills/src/tools.rs crates/agent-skills/src/lib.rs
git commit -m "feat(skills): list_skills + use_skill tools (progressive disclosure)"
```

---

### Task 5: `read_skill_file` tool

**Files:**
- Modify: `agent/crates/agent-skills/src/tools.rs` (add `ReadSkillFile`)

**Interfaces:**
- Consumes: `crate::guard::resolve_in_dir`, `crate::registry::SkillRegistry`.
- Produces: `pub struct ReadSkillFile { ... }` with `pub fn new(registry: std::sync::Arc<SkillRegistry>) -> Self`; tool name `"read_skill_file"`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `agent/crates/agent-skills/src/tools.rs`:

```rust
    #[tokio::test]
    async fn read_skill_file_returns_contents() {
        let (reg, _d) = reg_with_skill("alpha", "body", &[("notes.txt", "hello notes")]);
        let out = ReadSkillFile::new(reg)
            .execute(json!({"skill": "alpha", "path": "notes.txt"}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("hello notes"));
    }

    #[tokio::test]
    async fn read_skill_file_rejects_escape() {
        let (reg, _d) = reg_with_skill("alpha", "body", &[]);
        let err = ReadSkillFile::new(reg)
            .execute(json!({"skill": "alpha", "path": "../../etc/passwd"}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)));
    }

    #[tokio::test]
    async fn read_skill_file_unknown_skill_is_not_found() {
        let (reg, _d) = reg_with_skill("alpha", "body", &[]);
        let err = ReadSkillFile::new(reg)
            .execute(json!({"skill": "nope", "path": "x"}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills read_skill_file 2>&1 | tail -20`
Expected: FAIL — `cannot find ... ReadSkillFile`.

- [ ] **Step 3: Implement the tool**

Add to `agent/crates/agent-skills/src/tools.rs` (after the `UseSkill` impl, before the test block). Also remove the `#[allow(unused_imports)]` on the `resolve_in_dir` import added in Task 4 — it is now used.

```rust
/// Read one bundled file from a skill's own directory (read-only, dir-confined).
pub struct ReadSkillFile {
    registry: Arc<SkillRegistry>,
}

impl ReadSkillFile {
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ReadSkillFile {
    fn name(&self) -> &str {
        "read_skill_file"
    }
    fn description(&self) -> &str {
        "Read a bundled file belonging to a skill (e.g. a script or reference), so you can inspect it before acting."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "skill": { "type": "string", "description": "Skill name." },
                    "path": { "type": "string", "description": "File path relative to the skill directory." }
                },
                "required": ["skill", "path"]
            }),
        }
    }
    fn intent(&self, args: &Value) -> Result<ToolIntent, ToolError> {
        let skill = name_arg(args, "skill")?;
        let path = name_arg(args, "path")?;
        Ok(ToolIntent {
            tool: "read_skill_file".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: format!("read skill file {skill}/{path}"),
        })
    }
    async fn execute(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let skill_name = name_arg(&args, "skill")?;
        let rel = name_arg(&args, "path")?;
        let skill = self
            .registry
            .find(&skill_name)
            .ok_or_else(|| ToolError::NotFound(format!("skill '{skill_name}' not found")))?;
        let full = resolve_in_dir(&skill.dir, &rel).map_err(ToolError::Denied)?;
        let content = std::fs::read_to_string(&full)
            .map_err(|e| ToolError::NotFound(format!("{}: {e}", full.display())))?;
        Ok(ToolOutput { content, display: None })
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills read_skill_file 2>&1 | tail -20`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-skills/src/tools.rs
git commit -m "feat(skills): read_skill_file tool (dir-confined, read-only)"
```

---

### Task 6: `create_skill` tool (agent authoring)

**Files:**
- Modify: `agent/crates/agent-skills/src/tools.rs` (add `CreateSkill`)

**Interfaces:**
- Consumes: `crate::registry::{sanitize_slug, SkillRegistry}`, `crate::guard::resolve_in_dir`, the `MAX_BODY_BYTES`/`MAX_FILE_BYTES`/`MAX_FILES` consts.
- Produces: `pub struct CreateSkill { ... }` with `pub fn new(registry: std::sync::Arc<SkillRegistry>) -> Self`; tool name `"create_skill"`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `agent/crates/agent-skills/src/tools.rs`:

```rust
    fn writable_reg() -> (Arc<SkillRegistry>, TempDir) {
        let dir = tempdir().unwrap();
        let root = dir.path().join("skills");
        let reg = Arc::new(SkillRegistry::new(vec![root.clone()], root));
        (reg, dir)
    }

    #[tokio::test]
    async fn create_skill_round_trips_to_listing() {
        let (reg, _d) = writable_reg();
        let out = CreateSkill::new(reg.clone())
            .execute(
                json!({"name": "Greeter", "description": "Greets", "body": "Say hi."}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(out.content.contains("greeter"));
        let listed = reg.scan();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "greeter");
        assert_eq!(listed[0].description, "Greets");
        assert!(listed[0].body.contains("Say hi."));
    }

    #[tokio::test]
    async fn create_skill_writes_bundled_files() {
        let (reg, _d) = writable_reg();
        CreateSkill::new(reg.clone())
            .execute(
                json!({"name": "withscript", "description": "d", "body": "b",
                       "files": [{"path": "scripts/run.sh", "content": "echo hi"}]}),
                &ctx(),
            )
            .await
            .unwrap();
        let skill = reg.find("withscript").unwrap();
        let script = skill.dir.join("scripts").join("run.sh");
        assert!(script.is_file());
        assert_eq!(std::fs::read_to_string(script).unwrap(), "echo hi");
    }

    #[tokio::test]
    async fn create_skill_refuses_overwrite() {
        let (reg, _d) = writable_reg();
        let t = CreateSkill::new(reg.clone());
        t.execute(json!({"name": "dup", "description": "d", "body": "b"}), &ctx()).await.unwrap();
        let err = t.execute(json!({"name": "dup", "description": "d2", "body": "b2"}), &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::Failed { .. }));
    }

    #[tokio::test]
    async fn create_skill_rejects_bad_name() {
        let (reg, _d) = writable_reg();
        let err = CreateSkill::new(reg)
            .execute(json!({"name": "../evil", "description": "d", "body": "b"}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn create_skill_rejects_file_escape() {
        let (reg, _d) = writable_reg();
        let err = CreateSkill::new(reg)
            .execute(
                json!({"name": "x", "description": "d", "body": "b",
                       "files": [{"path": "../escape.txt", "content": "no"}]}),
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)));
    }

    #[test]
    fn create_skill_intent_is_write_on_target() {
        let (reg, _d) = writable_reg();
        let intent = CreateSkill::new(reg.clone())
            .intent(&json!({"name": "Greeter", "description": "d", "body": "b"}))
            .unwrap();
        assert!(matches!(intent.access, Access::Write));
        assert!(intent.paths[0].ends_with("greeter"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills create_skill 2>&1 | tail -20`
Expected: FAIL — `cannot find ... CreateSkill`.

- [ ] **Step 3: Implement the tool**

Add to `agent/crates/agent-skills/src/tools.rs` (after the `ReadSkillFile` impl, before the test block). Remove the `#[allow(unused_imports)]`/`#[allow(dead_code)]` attributes added in Task 4 — `sanitize_slug` and the `MAX_*` consts are now used.

```rust
/// Author a new skill under the writable root (project-local by default).
pub struct CreateSkill {
    registry: Arc<SkillRegistry>,
}

impl CreateSkill {
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for CreateSkill {
    fn name(&self) -> &str {
        "create_skill"
    }
    fn description(&self) -> &str {
        "Author a new reusable skill (SKILL.md + optional bundled files) under the writable skills directory."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Skill name (slugified)." },
                    "description": { "type": "string", "description": "One-line 'when to use' summary." },
                    "body": { "type": "string", "description": "Markdown instructions." },
                    "files": {
                        "type": "array",
                        "description": "Optional bundled files.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string" },
                                "content": { "type": "string" }
                            },
                            "required": ["path", "content"]
                        }
                    }
                },
                "required": ["name", "description", "body"]
            }),
        }
    }
    fn intent(&self, args: &Value) -> Result<ToolIntent, ToolError> {
        let name = name_arg(args, "name")?;
        let slug = sanitize_slug(&name).map_err(ToolError::InvalidArgs)?;
        let target = self.registry.writable_root().join(&slug);
        Ok(ToolIntent {
            tool: "create_skill".into(),
            access: Access::Write,
            paths: vec![target],
            command: None,
            summary: format!("create skill {slug}"),
        })
    }
    async fn execute(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let name = name_arg(&args, "name")?;
        let description = name_arg(&args, "description")?;
        let body = name_arg(&args, "body")?;
        let slug = sanitize_slug(&name).map_err(ToolError::InvalidArgs)?;
        if body.len() > MAX_BODY_BYTES {
            return Err(ToolError::InvalidArgs(format!("body exceeds {MAX_BODY_BYTES} bytes")));
        }
        let target = self.registry.writable_root().join(&slug);
        if target.exists() {
            return Err(ToolError::Failed {
                message: format!("skill '{slug}' already exists at {}", target.display()),
                stderr: None,
            });
        }
        // Validate bundled files BEFORE writing anything (no partial skill on error).
        let files: Vec<(std::path::PathBuf, String)> = match args.get("files") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(arr)) => {
                if arr.len() > MAX_FILES {
                    return Err(ToolError::InvalidArgs(format!("too many files (max {MAX_FILES})")));
                }
                let mut out = Vec::new();
                for f in arr {
                    let path = f.get("path").and_then(Value::as_str)
                        .ok_or_else(|| ToolError::InvalidArgs("file missing 'path'".into()))?;
                    let content = f.get("content").and_then(Value::as_str)
                        .ok_or_else(|| ToolError::InvalidArgs("file missing 'content'".into()))?;
                    if content.len() > MAX_FILE_BYTES {
                        return Err(ToolError::InvalidArgs(format!("file '{path}' exceeds {MAX_FILE_BYTES} bytes")));
                    }
                    let full = resolve_in_dir(&target, path).map_err(ToolError::Denied)?;
                    out.push((full, content.to_string()));
                }
                out
            }
            Some(_) => return Err(ToolError::InvalidArgs("'files' must be an array".into())),
        };

        std::fs::create_dir_all(&target)
            .map_err(|e| ToolError::Failed { message: format!("mkdir {}: {e}", target.display()), stderr: None })?;
        let md = format!("---\nname: {slug}\ndescription: {description}\n---\n\n{body}\n");
        std::fs::write(target.join("SKILL.md"), md)
            .map_err(|e| ToolError::Failed { message: format!("write SKILL.md: {e}"), stderr: None })?;
        for (full, content) in &files {
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ToolError::Failed { message: format!("mkdir {}: {e}", parent.display()), stderr: None })?;
            }
            std::fs::write(full, content)
                .map_err(|e| ToolError::Failed { message: format!("write {}: {e}", full.display()), stderr: None })?;
        }
        Ok(ToolOutput {
            content: format!("Created skill '{slug}' at {}. Load it with use_skill.", target.display()),
            display: None,
        })
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills create_skill 2>&1 | tail -20`
Expected: PASS (6 tests).

- [ ] **Step 5: Run the whole crate's tests + clippy**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills 2>&1 | tail -20 && cargo clippy -p agent-skills --all-targets -- -D warnings 2>&1 | tail -10`
Expected: all tests PASS; clippy clean (no warnings).

- [ ] **Step 6: Commit**

```bash
cd agent && git add crates/agent-skills/src/tools.rs
git commit -m "feat(skills): create_skill authoring tool (slug + guards + size caps)"
```

---

### Task 7: System-prompt composition + presets helper

**Files:**
- Modify: `agent/crates/agent-skills/src/presets.rs` (replace the placeholder)
- Modify: `agent/crates/agent-skills/src/lib.rs` (enable the `presets` module + re-exports)

**Interfaces:**
- Consumes: `crate::registry::SkillRegistry`.
- Produces:
  - `pub const SKILLS_AWARENESS: &str`
  - `pub fn compose_system_prompt(base: &str, registry: &SkillRegistry, presets: &[String]) -> Result<String, String>`

- [ ] **Step 1: Enable the module + re-exports**

In `agent/crates/agent-skills/src/lib.rs`, uncomment `pub mod presets;` and `pub use presets::{compose_system_prompt, SKILLS_AWARENESS};`. The `lib.rs` should now have every module + re-export enabled (no commented lines remain).

- [ ] **Step 2: Write the failing tests**

Replace the contents of `agent/crates/agent-skills/src/presets.rs` with:

```rust
use crate::registry::SkillRegistry;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn reg_with(name: &str, body: &str) -> (Arc<SkillRegistry>, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let sdir = dir.path().join(name);
        fs::create_dir_all(&sdir).unwrap();
        fs::write(sdir.join("SKILL.md"), format!("---\nname: {name}\ndescription: d\n---\n{body}")).unwrap();
        (Arc::new(SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf())), dir)
    }

    #[test]
    fn always_appends_awareness_line() {
        let dir = tempdir().unwrap();
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let out = compose_system_prompt("BASE", &reg, &[]).unwrap();
        assert!(out.starts_with("BASE"));
        assert!(out.contains(SKILLS_AWARENESS));
    }

    #[test]
    fn inlines_preset_bodies() {
        let (reg, _d) = reg_with("greeter", "Say hi politely.");
        let out = compose_system_prompt("BASE", &reg, &["greeter".to_string()]).unwrap();
        assert!(out.contains("Say hi politely."));
        assert!(out.contains("greeter"));
    }

    #[test]
    fn unknown_preset_is_error() {
        let dir = tempdir().unwrap();
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let err = compose_system_prompt("BASE", &reg, &["nope".to_string()]).unwrap_err();
        assert!(err.contains("nope"));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills presets 2>&1 | tail -20`
Expected: FAIL — `cannot find ... compose_system_prompt`.

- [ ] **Step 4: Implement the composer**

Add to `agent/crates/agent-skills/src/presets.rs` (above the test block):

```rust
/// Short note appended to every system prompt so the model knows skills exist.
pub const SKILLS_AWARENESS: &str = "You have skills available — reusable instruction packages. \
Call `list_skills` to see them and `use_skill` to load one before tackling a matching task.";

/// Build the full system prompt: the base prompt, then the skills-awareness note,
/// then the full body of each preset skill (preloaded so it is active from turn one).
/// Returns `Err` if a named preset is not found, so a typo fails fast at startup.
pub fn compose_system_prompt(
    base: &str,
    registry: &SkillRegistry,
    presets: &[String],
) -> Result<String, String> {
    let mut out = String::from(base);
    out.push_str("\n\n");
    out.push_str(SKILLS_AWARENESS);
    for name in presets {
        let skill = registry
            .find(name)
            .ok_or_else(|| format!("preset skill not found: {name}"))?;
        out.push_str(&format!("\n\n## Skill: {}\n{}", skill.name, skill.body));
    }
    Ok(out)
}
```

- [ ] **Step 5: Run the tests + whole-crate check to verify they pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-skills 2>&1 | tail -20 && cargo clippy -p agent-skills --all-targets -- -D warnings 2>&1 | tail -10`
Expected: all `agent-skills` tests PASS; clippy clean.

- [ ] **Step 6: Commit**

```bash
cd agent && git add crates/agent-skills/src/presets.rs crates/agent-skills/src/lib.rs
git commit -m "feat(skills): compose_system_prompt + skills-awareness note for presets"
```

---

### Task 8: Wire skills into `agent-runtime-config`

**Files:**
- Modify: `agent/crates/agent-runtime-config/Cargo.toml` (add the `agent-skills` dep)
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (add `build_skills`)

**Interfaces:**
- Consumes: `agent_skills::{SkillRegistry, ListSkills, UseSkill, ReadSkillFile, CreateSkill}`, `agent_tools::{Tool, ToolRegistry}`.
- Produces: `pub fn build_skills(skills_dirs: &[String], workspace: &Path) -> (Arc<agent_skills::SkillRegistry>, Vec<Arc<dyn Tool>>)` — the shared registry plus the four skill tools, ready to register.

- [ ] **Step 1: Add the dependency**

In `agent/crates/agent-runtime-config/Cargo.toml`, under `[dependencies]`, add:

```toml
agent-skills = { path = "../agent-skills" }
```

- [ ] **Step 2: Write the failing test**

In `agent/crates/agent-runtime-config/src/lib.rs`, add to the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn build_skills_returns_four_tools() {
        let (_reg, tools) = build_skills(&[], std::path::Path::new("/tmp/ws"));
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        for expected in ["list_skills", "use_skill", "read_skill_file", "create_skill"] {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-runtime-config build_skills 2>&1 | tail -20`
Expected: FAIL — `cannot find function build_skills`.

- [ ] **Step 4: Implement `build_skills`**

In `agent/crates/agent-runtime-config/src/lib.rs`, add the import near the other `agent_*` imports (top of file):

```rust
use agent_skills::{CreateSkill, ListSkills, ReadSkillFile, SkillRegistry, UseSkill};
use agent_tools::Tool;
```

(Note: `agent_tools::ToolRegistry` is already imported; add `Tool` alongside it or use the line above.)

Then add the function (e.g. directly after `build_registry`):

```rust
/// Build the shared skill registry (from `--skills-dir`, or defaults) and the four
/// skill tools that wrap it. Register the returned tools into the `ToolRegistry`,
/// and use the returned `SkillRegistry` to compose preset bodies into the system prompt.
pub fn build_skills(
    skills_dirs: &[String],
    workspace: &Path,
) -> (Arc<SkillRegistry>, Vec<Arc<dyn Tool>>) {
    let registry = Arc::new(SkillRegistry::from_config(skills_dirs, workspace));
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ListSkills::new(registry.clone())),
        Arc::new(UseSkill::new(registry.clone())),
        Arc::new(ReadSkillFile::new(registry.clone())),
        Arc::new(CreateSkill::new(registry.clone())),
    ];
    (registry, tools)
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test -p agent-runtime-config build_skills 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cd agent && git add crates/agent-runtime-config/Cargo.toml crates/agent-runtime-config/src/lib.rs
git commit -m "feat(skills): build_skills wiring helper in agent-runtime-config"
```

---

### Task 9: Wire skills into the CLI (`agent-cli`)

**Files:**
- Modify: `agent/crates/agent-cli/src/main.rs`

**Interfaces:**
- Consumes: `agent_runtime_config::build_skills`, `agent_skills::compose_system_prompt`.

- [ ] **Step 1: Add the flags + base-prompt constant**

In `agent/crates/agent-cli/src/main.rs`, add two fields to the `Cli` struct (after `allow_host`):

```rust
    /// Skill search directory (repeatable). Default: <workspace>/.agent/skills + ~/.agent/skills.
    #[arg(long = "skills-dir")]
    skills_dir: Vec<String>,
    /// Preload a skill as a preset by name (repeatable): its body is injected into the system prompt.
    #[arg(long = "skill")]
    skill: Vec<String>,
```

Add the `agent-skills` dependency to `agent/crates/agent-cli/Cargo.toml` under `[dependencies]`:

```toml
agent-skills = { path = "../agent-skills" }
```

Add a base-prompt constant near the top of `main.rs` (after the imports):

```rust
const BASE_SYSTEM_PROMPT: &str = "You are a local coding agent. Use the provided tools to \
inspect and modify the workspace. Think step by step. When the task is complete, reply with a \
summary and no tool call.";
```

Update the import line for `agent_runtime_config` to include `build_skills`:

```rust
use agent_runtime_config::{backend_name_is_valid, build_registry, build_model, build_skills,
    default_allowlist, default_denylist, pick_protocol};
```

- [ ] **Step 2: Register skill tools + compose the system prompt**

In `main()`, after the MCP registration block (`let mcp_manager = match &cli.mcp_config { ... };`) and BEFORE `let tools = Arc::new(registry);`, add:

```rust
    // Skills: register the four skill tools, then compose any presets into the system prompt.
    let (skill_registry, skill_tools) = build_skills(&cli.skills_dir, &workspace);
    for t in skill_tools {
        registry.register(t);
    }
    let system_prompt = match agent_skills::compose_system_prompt(
        BASE_SYSTEM_PROMPT, &skill_registry, &cli.skill) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("skills: {e}");
            std::process::exit(2);
        }
    };
```

Then replace the `WindowContext::new(Message::system("You are a local coding agent. ..."))` block (the multi-line literal at lines ~103-106) with:

```rust
    let mut ctx = WindowContext::new(Message::system(system_prompt));
```

- [ ] **Step 3: Build + verify the CLI compiles and the workspace still passes**

Run: `source "$HOME/.cargo/env" && cd agent && cargo build -p agent-cli 2>&1 | tail -20 && cargo clippy -p agent-cli --all-targets -- -D warnings 2>&1 | tail -10`
Expected: builds; clippy clean.

- [ ] **Step 4: Smoke-test discovery + authoring against the live model (manual)**

Per `agent/docs/RUNNING.md`, start the CLI pointed at a scratch workspace, then exercise the loop:

```bash
source "$HOME/.cargo/env" && cd agent
mkdir -p /tmp/skills-demo
cargo run -p agent-cli -- --base-url http://localhost:8080 --model qwen3.6-35b-a3b \
  --workspace /tmp/skills-demo --context-limit 32768
```

At the prompt, ask the agent to: "List your skills, then create a skill named 'greet' that prints a greeting, then load it." Confirm `list_skills` → `create_skill` → `list_skills` (now shows `greet`) → `use_skill` round-trips, and that `create_skill` triggers an approval prompt (it is a Write intent). This is a manual check; record the outcome in the task note.

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-cli/Cargo.toml crates/agent-cli/src/main.rs
git commit -m "feat(skills): wire skill tools + presets into agent-cli"
```

---

### Task 10: Wire skills into the daemon (`agent-server`)

**Files:**
- Modify: `agent/crates/agent-server/Cargo.toml` (add `agent-skills` dep)
- Modify: `agent/crates/agent-server/src/daemon.rs` (make `SYSTEM_PROMPT` public; add `system_prompt` to `DaemonParams`; use it)
- Modify: `agent/crates/agent-server/src/main.rs` (flags; build skills; fold tools into the slice; compose prompt; thread it through)

**Interfaces:**
- Consumes: `agent_runtime_config::build_skills`, `agent_skills::compose_system_prompt`, the now-public `daemon::SYSTEM_PROMPT`.
- Produces: `DaemonParams.system_prompt: String`; the daemon builds its `WindowContext` from this instead of the const. The existing `mcp_tools` slice now also carries the four skill tools (registered by `build_loop` unchanged).

- [ ] **Step 1: Add the dependency**

In `agent/crates/agent-server/Cargo.toml`, under `[dependencies]`, add:

```toml
agent-skills = { path = "../agent-skills" }
```

- [ ] **Step 2: Make the base prompt public + thread it through `DaemonParams`**

In `agent/crates/agent-server/src/daemon.rs`:

Change the const to public:

```rust
pub const SYSTEM_PROMPT: &str = "You are a local coding agent. Use the provided tools to inspect \
and modify the workspace. Think step by step. When the task is complete, reply with a summary \
and no tool call.";
```

Add a field to `DaemonParams` (after `workspace`):

```rust
    pub system_prompt: String,
```

Replace the `WindowContext::new(Message::system(SYSTEM_PROMPT))` call (line ~58-59) with:

```rust
    let ctx = Arc::new(tokio::sync::Mutex::new(
        WindowContext::new(Message::system(params.system_prompt.clone()))));
```

- [ ] **Step 3: Add flags, build skills, compose prompt, thread it in `main.rs`**

In `agent/crates/agent-server/src/main.rs`:

Add two flags to the `Run` subcommand (after `allow_host`):

```rust
        /// Skill search directory (repeatable). Default: <workspace>/.agent/skills + ~/.agent/skills.
        #[arg(long = "skills-dir")]
        skills_dir: Vec<String>,
        /// Preload a skill as a preset by name (repeatable).
        #[arg(long = "skill")]
        skill: Vec<String>,
```

Add `skills_dir` and `skill` to the `Cmd::Run { ... }` destructuring pattern in `main()`.

After the `mcp_tools` slice is built (the `let mcp_tools: Arc<[...]> = ...;` block) and BEFORE constructing `DaemonParams`, add:

```rust
            // Skills: build the shared registry + tools, fold the tools into the
            // same slice build_loop already registers, and compose presets into the prompt.
            let (skill_registry, skill_tools) =
                agent_runtime_config::build_skills(&skills_dir, &workspace);
            let mut all_tools: Vec<std::sync::Arc<dyn agent_tools::Tool>> = mcp_tools.to_vec();
            all_tools.extend(skill_tools);
            let extra_tools: std::sync::Arc<[std::sync::Arc<dyn agent_tools::Tool>]> =
                std::sync::Arc::from(all_tools);
            let system_prompt = match agent_skills::compose_system_prompt(
                daemon::SYSTEM_PROMPT, &skill_registry, &skill) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("skills: {e}");
                    std::process::exit(2);
                }
            };
```

Update the `DaemonParams` construction: change `mcp_tools,` to `mcp_tools: extra_tools,` and add `system_prompt,`:

```rust
            let params = daemon::DaemonParams {
                ws_url: ws_url(&cfg.worker_url),
                agent_token: cfg.agent_token,
                config: base,
                api_key,
                claude_binary,
                config_path: runtime_config,
                workspace,
                system_prompt,
                mcp_tools: extra_tools,
            };
```

- [ ] **Step 4: Update `params_clone`**

In `agent/crates/agent-server/src/main.rs`, add `system_prompt` to `params_clone`:

```rust
fn params_clone(p: &daemon::DaemonParams) -> daemon::DaemonParams {
    daemon::DaemonParams {
        ws_url: p.ws_url.clone(),
        agent_token: p.agent_token.clone(),
        config: p.config.clone(),
        api_key: p.api_key.clone(),
        claude_binary: p.claude_binary.clone(),
        config_path: p.config_path.clone(),
        workspace: p.workspace.clone(),
        system_prompt: p.system_prompt.clone(),
        mcp_tools: p.mcp_tools.clone(),
    }
}
```

- [ ] **Step 5: Build + verify the daemon compiles, clippy clean, full workspace tests pass**

Run: `source "$HOME/.cargo/env" && cd agent && cargo build -p agent-server 2>&1 | tail -20 && cargo test --workspace 2>&1 | tail -20 && cargo clippy --all-targets -- -D warnings 2>&1 | tail -10`
Expected: builds; ALL workspace tests PASS (existing `agent-server` runtime tests pass `mcp_tools` positionally, so they are unaffected); clippy clean.

- [ ] **Step 6: Commit**

```bash
cd agent && git add crates/agent-server/Cargo.toml crates/agent-server/src/daemon.rs crates/agent-server/src/main.rs
git commit -m "feat(skills): wire skill tools + presets into agent-server daemon"
```

---

### Task 11: Document the subsystem + final verification

**Files:**
- Modify: `agent/docs/RUNNING.md` (document `--skills-dir`/`--skill` + the skill tools)

**Interfaces:** none (docs + verification only).

- [ ] **Step 1: Document skills in RUNNING.md**

Add a section to `agent/docs/RUNNING.md` describing the skills subsystem. Include:

```markdown
## Skills

A *skill* is a directory containing a `SKILL.md` (YAML-style frontmatter with `name` + `description`, then a markdown body) and any bundled files. Skills are discovered from:

- `<workspace>/.agent/skills` (project-local, writable — where `create_skill` writes), and
- `~/.agent/skills` (user-global, read-only),

or from explicit `--skills-dir <path>` flags (repeatable; the first is the writable root). Earlier roots win on a name conflict.

The agent gets four tools:
- `list_skills` — show the catalog (name + when-to-use).
- `use_skill {name}` — load a skill's full body + the paths of its bundled files into context.
- `read_skill_file {skill, path}` — read one bundled file (read-only, confined to the skill's directory).
- `create_skill {name, description, body, files?}` — author a new skill under the writable root (a Write action → goes through approval).

Bundled scripts are run with the ordinary `execute_command` tool, gated by the command allow/deny policy + approval — the skills subsystem never executes anything itself.

Preload a skill as a **preset** (its body injected into the system prompt from the first turn):

    cargo run -p agent-cli -- ... --skill code-review --skill changelog

`--skills-dir` and `--skill` are accepted by both `agent-cli` and `agent-serverd run`.
```

- [ ] **Step 2: Final full-workspace verification**

Run: `source "$HOME/.cargo/env" && cd agent && cargo test --workspace 2>&1 | tail -25 && cargo clippy --all-targets -- -D warnings 2>&1 | tail -10`
Expected: ALL workspace tests PASS; clippy clean (no warnings). Confirm the new `agent-skills` tests are included in the count.

- [ ] **Step 3: Commit**

```bash
cd agent && git add docs/RUNNING.md
git commit -m "docs(skills): document --skills-dir/--skill + the skill tools in RUNNING.md"
```

---

## Post-implementation (handled by the finishing-a-development-branch flow, not a task here)

- Record every Minor finding from the final whole-branch review (plus any Important/accepted items) into `docs/superpowers/context/follow-ups.md` under a `## 2026-06-23 skills-subsystem` section (Open / Accepted / Resolved), and commit it as part of finishing the branch.
- After the whole subsystem is merged, refresh the knowledge graph ONCE: `/graphify . --update`.

## Deferred (explicitly out of scope this cycle)

- **Persisting `skills_dirs`/`active_skills` into `RuntimeConfig`** + the browser Settings UI to edit them — belongs to the Settings capability cycle (persisting now, without matching web round-trip support, would let a browser settings-save silently wipe skill config).
- **Sub-agent skills** (a skill that spawns a constrained sub-`AgentLoop`) — needs nested-agent machinery; composes later as a different execution strategy over this same registry.
- **A dedicated skill-script runner / OS sandboxing** — execution stays on the existing `execute_command` + policy + approval seam (os-sandboxing is subsystem #2).
```
