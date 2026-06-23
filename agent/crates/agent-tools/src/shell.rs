use crate::{Access, Display, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

pub struct ExecuteCommand;

fn cmd_arg(args: &serde_json::Value) -> Result<String, ToolError> {
    args.get("command").and_then(|v| v.as_str()).map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field `command`".into()))
}

#[async_trait]
impl Tool for ExecuteCommand {
    fn name(&self) -> &str { "execute_command" }
    fn description(&self) -> &str { "Run a shell command in the workspace directory." }
    fn schema(&self) -> ToolSchema {
        ToolSchema { name: self.name().into(), description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "command":{"type":"string"}},"required":["command"]}) }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let command = cmd_arg(args)?;
        Ok(ToolIntent { tool: "execute_command".into(), access: Access::Write, paths: vec![],
            command: Some(command.clone()), summary: format!("run `{command}`") })
    }
    async fn execute(&self, args: serde_json::Value, ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError> {
        let command = cmd_arg(&args)?;
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&command).current_dir(&ctx.workspace)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let run = async {
            cmd.output().await
                .map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })
        };

        let output = tokio::select! {
            _ = ctx.cancel.cancelled() => return Err(ToolError::Denied("cancelled".into())),
            r = tokio::time::timeout(ctx.timeout, run) => match r {
                Err(_elapsed) => return Err(ToolError::Timeout),
                Ok(inner) => inner?,
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let exit_code = output.status.code().unwrap_or(-1);
        let content = format!("exit={exit_code}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}");
        Ok(ToolOutput { content, display: Some(Display::Terminal {
            command, stdout, stderr, exit_code }) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use serde_json::json;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn ctx(timeout: Duration) -> ToolCtx {
        ToolCtx { workspace: std::env::temp_dir(), timeout, cancel: CancellationToken::new() }
    }

    #[tokio::test]
    async fn runs_command_and_captures_stdout() {
        let out = ExecuteCommand.execute(json!({"command":"echo hello"}),
            &ctx(Duration::from_secs(5))).await.unwrap();
        assert!(out.content.contains("hello"));
        assert!(matches!(out.display, Some(Display::Terminal { exit_code: 0, .. })));
    }

    #[test]
    fn intent_carries_command_string() {
        let i = ExecuteCommand.intent(&json!({"command":"ls -la"})).unwrap();
        assert_eq!(i.command.as_deref(), Some("ls -la"));
        assert_eq!(i.access, Access::Write);
    }

    #[tokio::test]
    async fn times_out_long_command() {
        let err = ExecuteCommand.execute(json!({"command":"sleep 5"}),
            &ctx(Duration::from_millis(200))).await.unwrap_err();
        assert!(matches!(err, ToolError::Timeout));
    }
}
