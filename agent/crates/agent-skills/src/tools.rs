use crate::guard::resolve_in_dir;
use crate::registry::sanitize_slug;
use crate::registry::SkillRegistry;
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
            let mark = if s.examples.is_empty() {
                String::new()
            } else {
                format!(" [{} examples]", s.examples.len())
            };
            content.push_str(&format!("- {}: {}{mark}\n", s.name, s.description));
        }
        Ok(ToolOutput {
            content,
            display: None,
        })
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
            ToolError::NotFound(format!(
                "skill '{name}' not found. Available: {}",
                avail.join(", ")
            ))
        })?;
        let mut content = format!("# Skill: {}\n\n{}\n", skill.name, skill.body);
        let rel = |p: &std::path::Path| {
            p.strip_prefix(&skill.dir)
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_else(|_| p.to_string_lossy().into_owned())
        };
        if !skill.examples.is_empty() {
            content.push_str("\n## Examples (worked exemplars)\n");
            for p in &skill.examples {
                content.push_str(&format!("- {}\n", rel(p)));
            }
            content.push_str(
                "Read one with read_skill_file and imitate its shape and conventions; \
                 do not copy content verbatim.\n",
            );
        }
        let others: Vec<&std::path::PathBuf> = skill
            .files
            .iter()
            .filter(|p| !skill.examples.contains(p))
            .collect();
        if others.is_empty() {
            if skill.examples.is_empty() {
                content.push_str("\n(No bundled files.)\n");
            }
        } else {
            content.push_str(&format!(
                "\n## Bundled files (dir: {})\n",
                skill.dir.display()
            ));
            for p in others {
                content.push_str(&format!("- {}\n", rel(p)));
            }
            content.push_str(
                "Read a bundled file with read_skill_file (paths above are relative, as it \
                 expects); run a bundled script with execute_command using the dir above.\n",
            );
        }
        Ok(ToolOutput {
            content,
            display: None,
        })
    }
}

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
        "Author a new reusable skill (SKILL.md + optional bundled files) under the writable skills directory. Put worked exemplars under examples/ — they surface to consumers as a distinct Examples section with imitate-don't-copy guidance."
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
                        "description": "Optional bundled files. Files under examples/ are surfaced as worked exemplars.",
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
            return Err(ToolError::InvalidArgs(format!(
                "body exceeds {MAX_BODY_BYTES} bytes"
            )));
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
                    return Err(ToolError::InvalidArgs(format!(
                        "too many files (max {MAX_FILES})"
                    )));
                }
                let mut out = Vec::new();
                for f in arr {
                    let path = f
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| ToolError::InvalidArgs("file missing 'path'".into()))?;
                    let content = f
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or_else(|| ToolError::InvalidArgs("file missing 'content'".into()))?;
                    if content.len() > MAX_FILE_BYTES {
                        return Err(ToolError::InvalidArgs(format!(
                            "file '{path}' exceeds {MAX_FILE_BYTES} bytes"
                        )));
                    }
                    let full = resolve_in_dir(&target, path).map_err(ToolError::Denied)?;
                    out.push((full, content.to_string()));
                }
                out
            }
            Some(_) => return Err(ToolError::InvalidArgs("'files' must be an array".into())),
        };

        std::fs::create_dir_all(&target).map_err(|e| ToolError::Failed {
            message: format!("mkdir {}: {e}", target.display()),
            stderr: None,
        })?;
        let md = format!("---\nname: {slug}\ndescription: {description}\n---\n\n{body}\n");
        std::fs::write(target.join("SKILL.md"), md).map_err(|e| ToolError::Failed {
            message: format!("write SKILL.md: {e}"),
            stderr: None,
        })?;
        for (full, content) in &files {
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).map_err(|e| ToolError::Failed {
                    message: format!("mkdir {}: {e}", parent.display()),
                    stderr: None,
                })?;
            }
            std::fs::write(full, content).map_err(|e| ToolError::Failed {
                message: format!("write {}: {e}", full.display()),
                stderr: None,
            })?;
        }
        Ok(ToolOutput {
            content: format!(
                "Created skill '{slug}' at {}. Load it with use_skill.",
                target.display()
            ),
            display: None,
        })
    }
}

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
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Not for arbitrary workspace files — use read_file. Use read_skill_file \
              only for files bundled inside a loaded skill's directory.",
        )
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
        Ok(ToolOutput {
            content,
            display: None,
        })
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
            // stopgap; Task 3 replaces this with the config-driven strategy
            sandbox: std::sync::Arc::new(agent_tools::HostExecutor),
            call_id: "test".into(),
        }
    }

    fn reg_with_skill(
        name: &str,
        body: &str,
        files: &[(&str, &str)],
    ) -> (Arc<SkillRegistry>, TempDir) {
        let dir = tempdir().unwrap();
        let sdir = dir.path().join(name);
        fs::create_dir_all(&sdir).unwrap();
        fs::write(
            sdir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: d\n---\n{body}"),
        )
        .unwrap();
        for (fname, content) in files {
            fs::write(sdir.join(fname), content).unwrap();
        }
        let reg = Arc::new(SkillRegistry::new(
            vec![dir.path().to_path_buf()],
            dir.path().to_path_buf(),
        ));
        (reg, dir)
    }

    #[tokio::test]
    async fn list_skills_reports_catalog() {
        let (reg, _d) = reg_with_skill("alpha", "body", &[]);
        let out = ListSkills::new(reg)
            .execute(json!({}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("alpha: d"));
    }

    #[tokio::test]
    async fn list_skills_empty_is_clean() {
        let dir = tempdir().unwrap();
        let reg = Arc::new(SkillRegistry::new(
            vec![dir.path().to_path_buf()],
            dir.path().to_path_buf(),
        ));
        let out = ListSkills::new(reg)
            .execute(json!({}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("No skills"));
    }

    #[tokio::test]
    async fn use_skill_returns_body_and_files() {
        let (reg, _d) = reg_with_skill("alpha", "Step one.", &[("run.sh", "echo hi")]);
        let out = UseSkill::new(reg)
            .execute(json!({"name": "alpha"}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("Step one."));
        assert!(out.content.contains("run.sh"));
        assert!(out.content.contains("execute_command"));
    }

    #[tokio::test]
    async fn use_skill_unknown_is_not_found() {
        let (reg, _d) = reg_with_skill("alpha", "body", &[]);
        let err = UseSkill::new(reg)
            .execute(json!({"name": "missing"}), &ctx())
            .await
            .unwrap_err();
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
            .execute(
                json!({"skill": "alpha", "path": "../../etc/passwd"}),
                &ctx(),
            )
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

    #[tokio::test]
    async fn read_skill_file_missing_file_is_not_found() {
        let (reg, _d) = reg_with_skill("alpha", "body", &[]);
        let err = ReadSkillFile::new(reg)
            .execute(json!({"skill": "alpha", "path": "absent.txt"}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }

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
        t.execute(
            json!({"name": "dup", "description": "d", "body": "b"}),
            &ctx(),
        )
        .await
        .unwrap();
        let err = t
            .execute(
                json!({"name": "dup", "description": "d2", "body": "b2"}),
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed { .. }));
    }

    #[tokio::test]
    async fn create_skill_rejects_bad_name() {
        let (reg, _d) = writable_reg();
        let err = CreateSkill::new(reg)
            .execute(
                json!({"name": "../evil", "description": "d", "body": "b"}),
                &ctx(),
            )
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

    /// Build a registry whose single skill "demo" carries the given nested
    /// bundled files (paths relative to the skill dir). Mirrors `reg_with_skill`
    /// but creates intermediate directories so `examples/` subtrees exist.
    fn reg_with_nested_skill(name: &str, files: &[(&str, &str)]) -> (Arc<SkillRegistry>, TempDir) {
        let dir = tempdir().unwrap();
        let sdir = dir.path().join(name);
        fs::create_dir_all(&sdir).unwrap();
        fs::write(
            sdir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: d\n---\nbody"),
        )
        .unwrap();
        for (rel, content) in files {
            let full = sdir.join(rel);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(full, content).unwrap();
        }
        let reg = Arc::new(SkillRegistry::new(
            vec![dir.path().to_path_buf()],
            dir.path().to_path_buf(),
        ));
        (reg, dir)
    }

    #[tokio::test]
    async fn use_skill_renders_examples_section_and_relative_bundled_paths() {
        let (reg, _tmp) = reg_with_nested_skill(
            "demo",
            &[("examples/a.md", "EX"), ("references/r.md", "REF")],
        );
        let out = UseSkill::new(reg)
            .execute(json!({"name": "demo"}), &ctx())
            .await
            .unwrap();
        let c = &out.content;
        // Examples section, relative path, contract line:
        assert!(c.contains("## Examples (worked exemplars)"), "{c}");
        assert!(c.contains("- examples/a.md"), "{c}");
        assert!(c.contains("imitate its shape and conventions"), "{c}");
        assert!(c.contains("do not copy content verbatim"), "{c}");
        // Bundled section: relative path, dir in header, examples EXCLUDED:
        assert!(c.contains("## Bundled files (dir: "), "{c}");
        assert!(c.contains("- references/r.md"), "{c}");
        let bundled = c.split("## Bundled files").nth(1).unwrap();
        assert!(
            !bundled.contains("examples/a.md"),
            "examples double-listed: {c}"
        );
        // No absolute paths in the LISTS (the only absolute path is the dir header):
        let after_body = c.split("## Examples").nth(1).unwrap();
        for line in after_body.lines().filter(|l| l.starts_with("- ")) {
            assert!(
                !line.contains(_tmp.path().to_str().unwrap()),
                "absolute path leaked: {line}"
            );
        }
    }

    #[tokio::test]
    async fn use_skill_without_examples_has_no_examples_section() {
        let (reg, _tmp) = reg_with_nested_skill("demo", &[("references/r.md", "REF")]);
        let out = UseSkill::new(reg)
            .execute(json!({"name": "demo"}), &ctx())
            .await
            .unwrap();
        assert!(!out.content.contains("## Examples"), "{}", out.content);
        assert!(
            out.content.contains("## Bundled files (dir: "),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn list_skills_marks_example_bearing_skills() {
        let dir = tempdir().unwrap();
        for (name, has_ex) in [("withex", true), ("plain", false)] {
            let sdir = dir.path().join(name);
            fs::create_dir_all(&sdir).unwrap();
            fs::write(
                sdir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: d\n---\nbody"),
            )
            .unwrap();
            if has_ex {
                fs::create_dir_all(sdir.join("examples")).unwrap();
                fs::write(sdir.join("examples/a.md"), "EX").unwrap();
            }
        }
        let reg = Arc::new(SkillRegistry::new(
            vec![dir.path().to_path_buf()],
            dir.path().to_path_buf(),
        ));
        let out = ListSkills::new(reg)
            .execute(json!({}), &ctx())
            .await
            .unwrap();
        assert!(
            out.content.contains("withex:") && out.content.contains("[1 examples]"),
            "{}",
            out.content
        );
        let plain_line = out.content.lines().find(|l| l.contains("plain:")).unwrap();
        assert!(!plain_line.contains("examples"), "{plain_line}");
    }

    #[tokio::test]
    async fn create_skill_examples_round_trip_surfaces_the_section() {
        let (reg, _d) = writable_reg();
        CreateSkill::new(reg.clone())
            .execute(
                json!({"name": "authored", "description": "d", "body": "b",
                       "files": [{"path": "examples/sample.md", "content": "EX"}]}),
                &ctx(),
            )
            .await
            .unwrap();
        let out = UseSkill::new(reg)
            .execute(json!({"name": "authored"}), &ctx())
            .await
            .unwrap();
        assert!(
            out.content.contains("## Examples (worked exemplars)"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("- examples/sample.md"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn examples_flow_l1_marker_l2_section_l3_read() {
        // One temp skill "flow" with examples/sample.md containing "EXEMPLAR BODY".
        let (reg, _tmp) = reg_with_nested_skill("flow", &[("examples/sample.md", "EXEMPLAR BODY")]);
        // L1: list marker.
        let l1 = ListSkills::new(reg.clone())
            .execute(json!({}), &ctx())
            .await
            .unwrap();
        assert!(l1.content.contains("[1 examples]"), "{}", l1.content);
        // L2: use_skill Examples section with relative path.
        let l2 = UseSkill::new(reg.clone())
            .execute(json!({"name": "flow"}), &ctx())
            .await
            .unwrap();
        assert!(
            l2.content.contains("- examples/sample.md"),
            "{}",
            l2.content
        );
        // L3: read_skill_file returns the example body.
        let l3 = ReadSkillFile::new(reg.clone())
            .execute(
                json!({"skill": "flow", "path": "examples/sample.md"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(l3.content.contains("EXEMPLAR BODY"), "{}", l3.content);
        // Confinement unchanged: escape still rejected.
        let escape = ReadSkillFile::new(reg)
            .execute(json!({"skill": "flow", "path": "../escape"}), &ctx())
            .await;
        assert!(escape.is_err(), "escape not rejected");
    }

    #[test]
    fn create_skill_schema_teaches_the_examples_convention() {
        let (reg, _d) = writable_reg();
        let t = CreateSkill::new(reg);
        let s = t.schema();
        let text = serde_json::to_string(&s.parameters).unwrap();
        assert!(text.contains("examples/"), "{text}");
        assert!(t.description().contains("examples/"), "{}", t.description());
    }
}
