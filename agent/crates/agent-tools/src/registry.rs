use crate::{Tool, ToolSchema, WHEN_NOT_TO_CALL_MARKER};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        // Last-wins is intentional (rejecting could wedge startup on MCP
        // cross-server name collisions); the warn makes silent shadowing visible.
        if self.tools.insert(name.clone(), tool).is_some() {
            tracing::warn!(tool = %name, "duplicate tool name registered — last wins");
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Every registered tool (arbitrary order). Cheap: Arc clones.
    pub fn all(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.values().cloned().collect()
    }

    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .values()
            .map(|t| {
                let mut s = t.schema();
                if let Some(excl) = t.when_not_to_call() {
                    s.description =
                        format!("{}\n\n{} {}", s.description, WHEN_NOT_TO_CALL_MARKER, excl);
                }
                s
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;

    struct Echo;
    #[async_trait]
    impl Tool for Echo {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echoes"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: "echo".into(),
                description: "echoes".into(),
                parameters: json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent {
                tool: "echo".into(),
                access: Access::Read,
                paths: vec![],
                command: None,
                summary: "echo".into(),
            })
        }
        async fn execute(
            &self,
            args: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput {
                content: args.to_string(),
                display: None,
            })
        }
    }

    struct Confusable;
    #[async_trait]
    impl Tool for Confusable {
        fn name(&self) -> &str {
            "confusable"
        }
        fn description(&self) -> &str {
            "does a thing"
        }
        fn when_not_to_call(&self) -> Option<&str> {
            Some("use echo instead for X")
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: "confusable".into(),
                description: "does a thing".into(),
                parameters: json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent {
                tool: "confusable".into(),
                access: Access::Read,
                paths: vec![],
                command: None,
                summary: "c".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput {
                content: "ok".into(),
                display: None,
            })
        }
    }

    #[test]
    fn schemas_fold_when_not_to_call_into_description() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(Echo)); // no override -> untouched
        r.register(Arc::new(Confusable)); // override -> folded
        let schemas = r.schemas();
        let echo = schemas.iter().find(|s| s.name == "echo").unwrap();
        let conf = schemas.iter().find(|s| s.name == "confusable").unwrap();
        assert_eq!(
            echo.description, "echoes",
            "None tools keep their description verbatim"
        );
        assert!(
            conf.description.contains(WHEN_NOT_TO_CALL_MARKER),
            "marker present: {}",
            conf.description
        );
        assert!(
            conf.description.contains("use echo instead for X"),
            "prose present: {}",
            conf.description
        );
        assert!(
            conf.description.starts_with("does a thing"),
            "original description preserved"
        );
    }

    #[test]
    fn agent_tools_confusable_prose_and_required_descs() {
        use crate::fs::{EditFile, ListDirectory, ReadFile, WriteFile};
        use crate::{git::GitCommit, shell::ExecuteCommand, RenderArtifact};
        // Confusable prose mentions the sibling tool.
        assert!(ReadFile
            .when_not_to_call()
            .unwrap()
            .contains("read_skill_file"));
        assert!(WriteFile.when_not_to_call().unwrap().contains("edit_file"));
        assert!(EditFile.when_not_to_call().unwrap().contains("write_file"));
        // Every required param on these tools now has a description.
        for s in [
            ReadFile.schema(),
            WriteFile.schema(),
            EditFile.schema(),
            ExecuteCommand.schema(),
            GitCommit.schema(),
            RenderArtifact.schema(),
            ListDirectory.schema(),
        ] {
            assert!(
                required_params_missing_description(&s).is_empty(),
                "{} has undescribed required params: {:?}",
                s.name,
                required_params_missing_description(&s)
            );
        }
    }

    fn fake(name: &'static str) -> Arc<dyn Tool> {
        use crate::{Access, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
        struct F(&'static str);
        #[async_trait::async_trait]
        impl Tool for F {
            fn name(&self) -> &str {
                self.0
            }
            fn description(&self) -> &str {
                "fake"
            }
            fn schema(&self) -> ToolSchema {
                ToolSchema {
                    name: self.0.into(),
                    description: "fake".into(),
                    parameters: serde_json::json!({"type":"object"}),
                }
            }
            fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
                Ok(ToolIntent {
                    tool: self.0.into(),
                    access: Access::Read,
                    paths: vec![],
                    command: None,
                    summary: "x".into(),
                })
            }
            async fn execute(
                &self,
                _a: serde_json::Value,
                _c: &ToolCtx,
            ) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput {
                    content: "ok".into(),
                    display: None,
                })
            }
        }
        Arc::new(F(name))
    }

    #[test]
    fn all_returns_every_registered_tool() {
        let mut r = ToolRegistry::new();
        assert!(r.all().is_empty());
        r.register(fake("a"));
        r.register(fake("b"));
        let mut names: Vec<String> = r.all().iter().map(|t| t.name().to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    struct Named(&'static str, &'static str);
    #[async_trait]
    impl Tool for Named {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            self.1
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema {
                name: self.0.into(),
                description: self.1.into(),
                parameters: json!({"type":"object"}),
            }
        }
        fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent {
                tool: self.0.into(),
                access: Access::Read,
                paths: vec![],
                command: None,
                summary: "x".into(),
            })
        }
        async fn execute(
            &self,
            _a: serde_json::Value,
            _c: &ToolCtx,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput {
                content: self.1.into(),
                display: None,
            })
        }
    }

    #[test]
    fn duplicate_name_registration_is_last_wins() {
        // Behavior pin for S8: registering a second tool under an existing name
        // overwrites (the warn is best-effort observability, not asserted here).
        let mut r = ToolRegistry::new();
        r.register(Arc::new(Named("dup", "first")));
        r.register(Arc::new(Named("dup", "second")));
        assert_eq!(r.all().len(), 1, "collision must not grow the registry");
        assert_eq!(r.get("dup").unwrap().description(), "second");
    }

    #[test]
    fn registry_registers_and_looks_up() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(Echo));
        assert!(r.get("echo").is_some());
        assert!(r.get("missing").is_none());
        let schemas = r.schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "echo");
    }
}
