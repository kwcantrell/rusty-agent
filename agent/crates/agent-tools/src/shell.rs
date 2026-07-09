use crate::{Access, Display, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

pub struct ExecuteCommand;

fn cmd_arg(args: &serde_json::Value) -> Result<String, ToolError> {
    args.get("command")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field `command`".into()))
}

#[async_trait]
impl Tool for ExecuteCommand {
    fn name(&self) -> &str {
        "execute_command"
    }
    fn description(&self) -> &str {
        "Run a shell command in the workspace directory."
    }
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Not for operations a dedicated tool does directly: use read_file (not \
             `cat`), list_directory (not `ls`), git_status/git_diff (not `git \
             status`/`git diff`) — those are Read-tier and path-policy-aware, while \
             shell commands are Write-tier and may need approval. Use execute_command \
             for real shell work: builds, tests, pipes, scripts.",
        )
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "command":{"type":"string","description":"The shell command line to execute."}},
                "required":["command"]}),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let command = cmd_arg(args)?;
        Ok(ToolIntent {
            tool: "execute_command".into(),
            access: Access::Write,
            paths: vec![],
            command: Some(command.clone()),
            summary: format!("run `{command}`"),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        use tokio::io::AsyncReadExt;
        let command = cmd_arg(&args)?;
        let spec = crate::CommandSpec {
            program: "sh".into(),
            args: vec!["-c".into(), command.clone()],
            cwd: ctx.workspace.clone(),
            env: Default::default(),
            kind: crate::ProcKind::OneShot,
        };
        let mut child = ctx.sandbox.launch(spec).map_err(|e| match e {
            crate::SandboxError::Unavailable(m) => ToolError::Denied(m),
            other => ToolError::Failed {
                message: other.to_string(),
                stderr: None,
            },
        })?;

        let mut out_pipe = child.take_stdout();
        let mut err_pipe = child.take_stderr();
        let read_out = async {
            let mut s = String::new();
            if let Some(p) = out_pipe.as_mut() {
                let _ = p.read_to_string(&mut s).await;
            }
            s
        };
        let read_err = async {
            let mut s = String::new();
            if let Some(p) = err_pipe.as_mut() {
                let _ = p.read_to_string(&mut s).await;
            }
            s
        };

        // Drain both pipes CONCURRENTLY with wait() to avoid a pipe-buffer deadlock.
        let run = async {
            let (status, stdout, stderr) = tokio::join!(child.wait(), read_out, read_err);
            (status, stdout, stderr)
        };

        let (status, stdout, stderr) = tokio::select! {
            // On cancel/timeout we return; dropping `child` fires SandboxedChild::Drop
            // (docker kill / start_kill) + the inner child's kill_on_drop.
            _ = ctx.cancel.cancelled() => return Err(ToolError::Denied("cancelled".into())),
            r = tokio::time::timeout(ctx.timeout, run) => match r {
                Err(_elapsed) => return Err(ToolError::Timeout),
                Ok((status, stdout, stderr)) => (
                    status.map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })?,
                    stdout, stderr),
            }
        };
        let exit_code = status.code().unwrap_or(-1);
        let content =
            format!("exit={exit_code}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}");
        Ok(ToolOutput {
            content,
            display: Some(Display::Terminal {
                command,
                stdout,
                stderr,
                exit_code,
            }),
        })
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
        use std::sync::Arc;
        ToolCtx {
            workspace: std::env::temp_dir(),
            timeout,
            cancel: CancellationToken::new(),
            sandbox: Arc::new(crate::HostExecutor),
            backend: Arc::new(crate::backend::HostBackend::new(std::env::temp_dir())),
            call_id: "test".into(),
        }
    }

    #[tokio::test]
    async fn runs_command_and_captures_stdout() {
        let out = ExecuteCommand
            .execute(
                json!({"command":"echo hello"}),
                &ctx(Duration::from_secs(5)),
            )
            .await
            .unwrap();
        assert!(out.content.contains("hello"));
        assert!(matches!(
            out.display,
            Some(Display::Terminal { exit_code: 0, .. })
        ));
    }

    #[test]
    fn intent_carries_command_string() {
        let i = ExecuteCommand.intent(&json!({"command":"ls -la"})).unwrap();
        assert_eq!(i.command.as_deref(), Some("ls -la"));
        assert_eq!(i.access, Access::Write);
    }

    #[tokio::test]
    async fn times_out_long_command() {
        let err = ExecuteCommand
            .execute(
                json!({"command":"sleep 5"}),
                &ctx(Duration::from_millis(200)),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Timeout));
    }

    #[tokio::test]
    async fn captures_large_output_without_deadlock() {
        // ~200 KiB of stdout — well past the OS pipe buffer; would hang under the
        // wait-before-drain bug.
        let out = ExecuteCommand
            .execute(
                json!({"command": "for i in $(seq 1 20000); do echo 0123456789; done"}),
                &ctx(Duration::from_secs(20)),
            )
            .await
            .unwrap();
        assert!(out.content.len() > 100_000);
        assert!(matches!(
            out.display,
            Some(Display::Terminal { exit_code: 0, .. })
        ));
    }
}
