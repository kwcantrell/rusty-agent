use crate::{Tool, ToolSchema};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
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
        fn name(&self) -> &str { "echo" }
        fn description(&self) -> &str { "echoes" }
        fn schema(&self) -> ToolSchema {
            ToolSchema { name: "echo".into(), description: "echoes".into(),
                         parameters: json!({"type":"object"}) }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent { tool: "echo".into(), access: Access::Read, paths: vec![],
                            command: None, summary: "echo".into() })
        }
        async fn execute(&self, args: serde_json::Value, _ctx: &ToolCtx)
            -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput { content: args.to_string(), display: None })
        }
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
