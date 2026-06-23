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
