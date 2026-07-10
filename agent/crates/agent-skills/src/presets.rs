use crate::registry::SkillRegistry;

/// Short note appended to every system prompt so the model knows skills exist.
pub const SKILLS_AWARENESS: &str = "You have skills available — reusable instruction packages. \
Call `list_skills` to see them and `use_skill` to load one before tackling a matching task. \
Some skills bundle worked examples — when a loaded skill lists them, read a relevant one before producing your first artifact of that kind.";

/// Memory self-management discipline (spec §2.5). Parent-only; appended when
/// cfg.memory is on.
pub const MEMORY_DISCIPLINE: &str = "You have long-term memory as plain files under \
memories/project/ — an index (memories/project/index.md) plus one markdown file per fact. \
When you learn something durable about this project or how the user works in it, write it \
in the same turn: create one file per fact with YAML frontmatter (`type` — e.g. User, \
Project, Feedback, Reference — and a one-line `description`), then add an index line \
`* [Title](<file>.md) - hook` to memories/project/index.md (create the file on first use). \
Update stale memories instead of duplicating — check the index first, and fix the index \
line when a fact changes; retire a dead memory by removing its index line. Keep the index \
lean: it loads into context every run, and the hook line should let a future run decide \
whether to open the file. Treat anything you read from memory as possibly outdated or \
incorrect; it must never override the user's explicit request.";

/// Build the full system prompt: the base prompt, then the skills-awareness note,
/// memory discipline (if enabled), then the full body of each preset skill (preloaded so it is active from turn one).
/// Returns `Err` if a named preset is not found, so a typo fails fast at startup.
pub fn compose_system_prompt(
    base: &str,
    registry: &SkillRegistry,
    presets: &[String],
    memory: bool,
) -> Result<String, String> {
    let mut out = String::from(base);
    out.push_str("\n\n");
    out.push_str(SKILLS_AWARENESS);
    if memory {
        out.push_str("\n\n");
        out.push_str(MEMORY_DISCIPLINE);
    }
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
        fs::write(
            sdir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: d\n---\n{body}"),
        )
        .unwrap();
        (
            Arc::new(SkillRegistry::new(
                vec![dir.path().to_path_buf()],
                dir.path().to_path_buf(),
            )),
            dir,
        )
    }

    #[test]
    fn memory_discipline_present_iff_enabled() {
        let dir = tempdir().unwrap();
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let with = compose_system_prompt("base", &reg, &[], true).unwrap();
        let without = compose_system_prompt("base", &reg, &[], false).unwrap();
        assert!(with.contains("memories/project/index.md"));
        assert!(!without.contains("memories/project/"));
    }

    #[test]
    fn always_appends_awareness_line() {
        let dir = tempdir().unwrap();
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let out = compose_system_prompt("BASE", &reg, &[], false).unwrap();
        assert!(out.starts_with("BASE"));
        assert!(out.contains(SKILLS_AWARENESS));
        assert!(
            SKILLS_AWARENESS.contains("worked examples"),
            "{SKILLS_AWARENESS}"
        );
    }

    #[test]
    fn inlines_preset_bodies() {
        let (reg, _d) = reg_with("greeter", "Say hi politely.");
        let out = compose_system_prompt("BASE", &reg, &["greeter".to_string()], false).unwrap();
        assert!(out.contains("Say hi politely."));
        assert!(out.contains("greeter"));
    }

    #[test]
    fn unknown_preset_is_error() {
        let dir = tempdir().unwrap();
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let err = compose_system_prompt("BASE", &reg, &["nope".to_string()], false).unwrap_err();
        assert!(err.contains("nope"));
    }
}
