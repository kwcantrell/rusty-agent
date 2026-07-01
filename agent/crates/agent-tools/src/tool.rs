use crate::{ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> ToolSchema;
    /// Guidance on when the model should NOT call this tool (and which sibling to
    /// prefer). `None` for tools whose name/purpose already disambiguate. The
    /// registry folds this into the model-facing schema description; it is not a
    /// separate wire field.
    fn when_not_to_call(&self) -> Option<&str> {
        None
    }
    /// Declare what this call will do, for the policy engine to judge before execution.
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError>;
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError>;
}
