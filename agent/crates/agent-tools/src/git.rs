use crate::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

async fn git(ctx: &ToolCtx, args: &[&str]) -> Result<String, ToolError> {
    use tokio::io::AsyncReadExt;
    let spec = crate::CommandSpec {
        program: "git".into(),
        args: args.iter().map(|s| s.to_string()).collect(),
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
        _ = ctx.cancel.cancelled() => return Err(ToolError::Denied("cancelled".into())),
        r = tokio::time::timeout(ctx.timeout, run) => match r {
            Err(_elapsed) => return Err(ToolError::Timeout),
            Ok((status, stdout, stderr)) => (
                status.map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })?,
                stdout, stderr),
        }
    };
    if status.success() {
        Ok(stdout)
    } else {
        Err(ToolError::Failed {
            message: format!("git {} failed", args.join(" ")),
            stderr: Some(stderr),
        })
    }
}

fn empty_schema(name: &str, desc: &str) -> ToolSchema {
    ToolSchema {
        name: name.into(),
        description: desc.into(),
        parameters: json!({"type":"object","properties":{}}),
    }
}

pub struct GitStatus;
#[async_trait]
impl Tool for GitStatus {
    fn name(&self) -> &str {
        "git_status"
    }
    fn description(&self) -> &str {
        "Show working-tree status."
    }
    fn schema(&self) -> ToolSchema {
        empty_schema(self.name(), self.description())
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "git_status".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "git status".into(),
        })
    }
    async fn execute(&self, _a: serde_json::Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            content: git(ctx, &["status", "--short", "--branch"]).await?,
            display: None,
        })
    }
}

pub struct GitDiff;
#[async_trait]
impl Tool for GitDiff {
    fn name(&self) -> &str {
        "git_diff"
    }
    fn description(&self) -> &str {
        "Show unstaged changes."
    }
    fn schema(&self) -> ToolSchema {
        empty_schema(self.name(), self.description())
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "git_diff".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "git diff".into(),
        })
    }
    async fn execute(&self, _a: serde_json::Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            content: git(ctx, &["diff"]).await?,
            display: None,
        })
    }
}

pub struct GitCommit;
#[async_trait]
impl Tool for GitCommit {
    fn name(&self) -> &str {
        "git_commit"
    }
    fn description(&self) -> &str {
        "Stage all changes and commit with a message."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({"type":"object","properties":{
                "message":{"type":"string","description":"The commit message."}},
                "required":["message"]}),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let msg = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing `message`".into()))?;
        Ok(ToolIntent {
            tool: "git_commit".into(),
            access: Access::Write,
            paths: vec![],
            command: None,
            summary: format!("git commit -m {msg:?}"),
        })
    }
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<ToolOutput, ToolError> {
        let msg = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing `message`".into()))?;
        git(ctx, &["add", "-A"]).await?;
        let out = git(ctx, &["commit", "-m", msg]).await?;
        Ok(ToolOutput {
            content: out,
            display: None,
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

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap();
        };
        run(&["init"]);
        run(&["config", "user.email", "t@t.com"]);
        run(&["config", "user.name", "t"]);
        dir
    }
    fn ctx(ws: std::path::PathBuf) -> ToolCtx {
        use std::sync::Arc;
        ToolCtx {
            workspace: ws,
            timeout: Duration::from_secs(10),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(crate::HostExecutor),
            call_id: "test".into(),
        }
    }

    #[tokio::test]
    async fn git_status_reports_untracked() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let out = GitStatus
            .execute(json!({}), &ctx(dir.path().into()))
            .await
            .unwrap();
        assert!(out.content.contains("a.txt"));
    }

    #[test]
    fn git_commit_intent_is_write() {
        assert_eq!(
            GitCommit.intent(&json!({"message":"m"})).unwrap().access,
            Access::Write
        );
        assert_eq!(GitStatus.intent(&json!({})).unwrap().access, Access::Read);
    }

    use std::sync::{Arc, Mutex};

    /// A SandboxStrategy that records the CommandSpec it received, then delegates to the
    /// host so the real git command still runs.
    struct RecordingExecutor {
        calls: Arc<Mutex<Vec<crate::CommandSpec>>>,
    }
    impl crate::SandboxStrategy for RecordingExecutor {
        fn launch(
            &self,
            spec: crate::CommandSpec,
        ) -> Result<crate::SandboxedChild, crate::SandboxError> {
            self.calls.lock().unwrap().push(spec.clone());
            crate::HostExecutor.launch(spec)
        }
        fn describe(&self) -> crate::SandboxDescriptor {
            crate::HostExecutor.describe()
        }
    }

    fn recording_ctx(ws: std::path::PathBuf) -> (ToolCtx, Arc<Mutex<Vec<crate::CommandSpec>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let ctx = ToolCtx {
            workspace: ws,
            timeout: Duration::from_secs(10),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(RecordingExecutor {
                calls: calls.clone(),
            }),
            call_id: "test".into(),
        };
        (ctx, calls)
    }

    #[tokio::test]
    async fn git_dispatches_through_sandbox() {
        let dir = init_repo();
        let (ctx, calls) = recording_ctx(dir.path().into());
        let _ = GitStatus.execute(json!({}), &ctx).await.unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            1,
            "git_status should launch exactly one sandbox process"
        );
        assert_eq!(calls[0].program, "git");
        assert_eq!(calls[0].args.first().map(String::as_str), Some("status"));
        assert_eq!(calls[0].cwd, dir.path());
    }

    #[tokio::test]
    async fn git_commit_commits_staged_changes() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let out = GitCommit
            .execute(json!({"message":"init"}), &ctx(dir.path().into()))
            .await
            .unwrap();
        assert!(
            out.content.to_lowercase().contains("init")
                || out.content.contains("master")
                || out.content.contains("main")
        );
    }
}
