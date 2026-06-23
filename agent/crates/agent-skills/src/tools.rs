use crate::guard::resolve_in_dir;
#[allow(unused_imports)]
use crate::registry::sanitize_slug;
use crate::registry::SkillRegistry;
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

#[allow(dead_code)]
const MAX_BODY_BYTES: usize = 64 * 1024;
#[allow(dead_code)]
const MAX_FILE_BYTES: usize = 256 * 1024;
#[allow(dead_code)]
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

// Stub for Task 6 (re-exported from lib.rs; implemented in that task).
pub struct CreateSkill;

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

#[cfg(test)]
mod tests {
    use super::*;
    use agent_tools::ToolCtx;
    use std::fs;
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
}
