use crate::registry::SkillRegistry;

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
