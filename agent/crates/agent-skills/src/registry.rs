use crate::skill::{parse_skill_md, Skill};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Discovers skills from one or more roots and is the authoring target. Cheap to
/// re-scan, so callers re-scan per tool invocation and freshly authored skills
/// appear immediately without rebuilding the agent loop.
pub struct SkillRegistry {
    roots: Vec<PathBuf>,
    writable_root: PathBuf,
}

impl SkillRegistry {
    pub fn new(roots: Vec<PathBuf>, writable_root: PathBuf) -> Self {
        Self {
            roots,
            writable_root,
        }
    }

    /// Explicit `--skills-dir` roots if any (writable = first); otherwise the
    /// defaults: `<workspace>/.agent/skills` (writable) + `~/.agent/skills`.
    pub fn from_config(skills_dirs: &[String], workspace: &Path) -> Self {
        let filtered: Vec<String> = skills_dirs
            .iter()
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .collect();
        if let Some((first, rest)) = filtered.split_first() {
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
            let mut dirs: Vec<PathBuf> = read
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
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
    let text =
        std::fs::read_to_string(dir.join("SKILL.md")).map_err(|e| format!("read SKILL.md: {e}"))?;
    let parsed = parse_skill_md(&text)?;
    if let Some(fm_name) = &parsed.name {
        if fm_name != dir_name {
            return Err(format!(
                "frontmatter name '{fm_name}' != directory '{dir_name}'"
            ));
        }
    }
    let files = list_bundled_files(dir);
    // Component-wise filter (separator-safe): first component under the
    // skill dir must be the DIRECTORY `examples`.
    let examples: Vec<PathBuf> = files
        .iter()
        .filter(|p| {
            p.strip_prefix(dir)
                .ok()
                .and_then(|rel| rel.components().next())
                .map(|c| c.as_os_str() == "examples" && p.parent() != Some(dir))
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    Ok(Skill {
        name: dir_name.to_string(),
        description: parsed.description,
        body: parsed.body,
        dir: dir.to_path_buf(),
        files,
        examples,
    })
}

/// Collect bundled files (absolute paths) beneath a skill dir, recursively, so
/// nested subtrees like `examples/` are part of the superset. Excludes the
/// top-level `SKILL.md`. Sorted for stable rendering and ordered `examples`.
fn list_bundled_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files(dir, dir, &mut files);
    files.sort();
    files
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for e in read.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_files(root, &p, out);
        } else if p.is_file() && p != root.join("SKILL.md") {
            out.push(p);
        }
    }
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
        return Err(format!(
            "invalid skill name (no path separators): {trimmed}"
        ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_skill(root: &Path, name: &str, desc: &str, body: &str) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {desc}\n---\n{body}\n"),
        )
        .unwrap();
    }

    #[test]
    fn scan_discovers_skills_sorted() {
        let dir = tempdir().unwrap();
        write_skill(dir.path(), "beta", "B skill", "b body");
        write_skill(dir.path(), "alpha", "A skill", "a body");
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let found = reg.scan();
        assert_eq!(
            found.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
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
            proj.path().to_path_buf(),
        );
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
        assert_eq!(
            found.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["good"]
        );
    }

    #[test]
    fn frontmatter_name_mismatch_is_skipped() {
        let dir = tempdir().unwrap();
        let d = dir.path().join("realdir");
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("SKILL.md"),
            "---\nname: other\ndescription: x\n---\nbody",
        )
        .unwrap();
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
        let reg = SkillRegistry::new(
            vec![PathBuf::from("/nonexistent/xyz")],
            PathBuf::from("/tmp"),
        );
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
    fn from_config_ignores_empty_skills_dir_entries() {
        // An explicit but empty --skills-dir entry must not become a relative root;
        // it falls through to the workspace default.
        let reg = SkillRegistry::from_config(&["".to_string()], Path::new("/ws"));
        assert_eq!(reg.writable_root(), Path::new("/ws/.agent/skills"));
    }

    #[test]
    fn examples_are_the_examples_dir_subset_of_files() {
        let dir = tempfile::tempdir().unwrap();
        let sd = dir.path().join("demo");
        std::fs::create_dir_all(sd.join("examples/nested")).unwrap();
        std::fs::create_dir_all(sd.join("references")).unwrap();
        std::fs::write(sd.join("SKILL.md"), "---\ndescription: d\n---\nbody").unwrap();
        std::fs::write(sd.join("examples/a.md"), "A").unwrap();
        std::fs::write(sd.join("examples/nested/b.md"), "B").unwrap();
        std::fs::write(sd.join("references/r.md"), "R").unwrap();
        // A plain FILE named "examples" elsewhere must not confuse the filter:
        std::fs::write(sd.join("notes.md"), "N").unwrap();

        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let skill = reg.find("demo").expect("skill loads");
        let rel: Vec<String> = skill
            .examples
            .iter()
            .map(|p| p.strip_prefix(&skill.dir).unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(rel, vec!["examples/a.md".to_string(), "examples/nested/b.md".to_string()]);
        // Superset pin: files still contains every bundled file incl. examples.
        assert!(skill.files.len() >= 4, "{:?}", skill.files);
        for e in &skill.examples {
            assert!(skill.files.contains(e));
        }
    }

    #[test]
    fn no_examples_dir_means_empty_examples_and_a_plain_file_named_examples_does_not_count() {
        let dir = tempfile::tempdir().unwrap();
        let sd = dir.path().join("plain");
        std::fs::create_dir_all(&sd).unwrap();
        std::fs::write(sd.join("SKILL.md"), "---\ndescription: d\n---\nbody").unwrap();
        std::fs::write(sd.join("examples"), "just a file named examples").unwrap();
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let skill = reg.find("plain").expect("skill loads");
        assert!(skill.examples.is_empty(), "{:?}", skill.examples);
        assert_eq!(skill.files.len(), 1); // the odd file is still bundled
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
