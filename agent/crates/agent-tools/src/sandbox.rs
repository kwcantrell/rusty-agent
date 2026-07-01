use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcKind {
    OneShot,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Auto,
    Enforce,
}

#[derive(Debug, Clone)]
pub struct Limits {
    pub memory: String,        // "2g"
    pub cpus: String,          // "2"
    pub pids: u32,             // 512
    pub fsize: Option<String>, // ulimit fsize, e.g. "1g"
    pub tmp_size: String,      // "256m"
}

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub kind: ProcKind,
}

#[derive(Debug, Clone)]
pub struct SandboxDescriptor {
    pub mode: Mode,
    pub mechanism: &'static str, // "host" | "docker"
    pub image: Option<String>,
    pub network: bool,
    pub degraded: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("sandbox unavailable: {0}")]
    Unavailable(String),
    #[error("sandbox launch failed: {0}")]
    LaunchFailed(String),
    #[error("invalid mount: {0}")]
    InvalidMount(String),
}

/// A launched process plus optional container id for teardown.
pub struct SandboxedChild {
    child: Child,
    container: Option<String>, // docker container name; None for host
}

impl SandboxedChild {
    pub fn new_host(child: Child) -> Self {
        Self {
            child,
            container: None,
        }
    }
    pub fn new_container(child: Child, name: String) -> Self {
        Self {
            child,
            container: Some(name),
        }
    }
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }
    /// Kill the container (docker kill) or the local child, then reap it; idempotent best-effort.
    pub async fn kill(&mut self) {
        if let Some(name) = &self.container {
            let _ = tokio::process::Command::new("docker")
                .args(["kill", name])
                .output()
                .await;
        }
        // Intentional dual-kill: docker kill stops the container; start_kill reaps
        // the local foreground `docker run` client process.
        let _ = self.child.start_kill();
        // Reap now instead of relying on the kill_on_drop orphan reaper.
        let _ = self.child.wait().await;
    }
}

impl Drop for SandboxedChild {
    fn drop(&mut self) {
        // Backstop: Drop cannot await. Fire-and-forget a detached docker kill,
        // and start_kill the local child so nothing leaks on panic/early-return.
        if let Some(name) = self.container.take() {
            let _ = std::process::Command::new("docker")
                .args(["kill", &name])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        let _ = self.child.start_kill();
    }
}

pub trait SandboxStrategy: Send + Sync {
    fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError>;
    fn describe(&self) -> SandboxDescriptor;
}

/// Default strategy: run on the host exactly as the core did pre-sandbox.
pub struct HostExecutor;

/// Env vars forwarded from the parent to host-executed children. Everything
/// else is scrubbed (`env_clear`) so secrets like AGENT_API_KEY never reach
/// tool subprocesses; `spec.env` is applied afterwards and always wins.
const HOST_ENV_ALLOWLIST: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "TERM", "TMPDIR"];

impl SandboxStrategy for HostExecutor {
    fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError> {
        let mut cmd = tokio::process::Command::new(&spec.program);
        cmd.args(&spec.args)
            .current_dir(&spec.cwd)
            .env_clear()
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in HOST_ENV_ALLOWLIST {
            if let Ok(v) = std::env::var(key) {
                cmd.env(key, v);
            }
        }
        cmd.envs(&spec.env);
        // Service (mcp) needs an open stdin pipe; OneShot does not read stdin.
        match spec.kind {
            ProcKind::Service => {
                cmd.stdin(Stdio::piped());
            }
            ProcKind::OneShot => {
                cmd.stdin(Stdio::null());
            }
        }
        let child = cmd
            .spawn()
            .map_err(|e| SandboxError::LaunchFailed(e.to_string()))?;
        Ok(SandboxedChild::new_host(child))
    }
    fn describe(&self) -> SandboxDescriptor {
        SandboxDescriptor {
            mode: Mode::Off,
            mechanism: "host",
            image: None,
            network: true,
            degraded: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spec(program: &str, args: &[&str]) -> CommandSpec {
        CommandSpec {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir(),
            env: Default::default(),
            kind: ProcKind::OneShot,
        }
    }

    fn spec_with_env(program: &str, args: &[&str], env: &[(&str, &str)]) -> CommandSpec {
        CommandSpec {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            kind: ProcKind::OneShot,
        }
    }

    async fn run_and_capture(spec: CommandSpec) -> String {
        let mut sb = HostExecutor.launch(spec).unwrap();
        let mut out = sb.take_stdout().unwrap();
        let mut buf = String::new();
        use tokio::io::AsyncReadExt;
        out.read_to_string(&mut buf).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), sb.wait())
            .await
            .unwrap()
            .unwrap();
        buf
    }

    #[tokio::test]
    async fn host_executor_does_not_leak_parent_env() {
        // Plant a secret in the parent env (edition 2021: set_var is safe).
        // Unique name so no other test can collide with it.
        std::env::set_var("AGENT_TEST_SECRET_XYZQ", "leaked");
        let out = run_and_capture(spec(
            "sh",
            &["-c", "printenv AGENT_TEST_SECRET_XYZQ || echo ABSENT"],
        ))
        .await;
        assert!(
            out.contains("ABSENT"),
            "parent env must not leak into host children, got: {out:?}"
        );
    }

    #[tokio::test]
    async fn host_executor_passes_allowlisted_path() {
        let out = run_and_capture(spec("sh", &["-c", "printenv PATH"])).await;
        assert!(!out.trim().is_empty(), "PATH must be forwarded to children");
    }

    #[tokio::test]
    async fn host_executor_spec_env_wins() {
        // spec.env entries survive the scrub and override allow-listed values.
        let out = run_and_capture(spec_with_env(
            "sh",
            &["-c", "printenv AGENT_TEST_EXPLICIT"],
            &[("AGENT_TEST_EXPLICIT", "explicit-value")],
        ))
        .await;
        assert!(
            out.contains("explicit-value"),
            "spec.env must be applied, got: {out:?}"
        );
    }

    fn service_spec(program: &str, args: &[&str]) -> CommandSpec {
        CommandSpec {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: std::env::temp_dir(),
            env: Default::default(),
            kind: ProcKind::Service,
        }
    }

    #[tokio::test]
    async fn kill_reaps_a_long_running_child() {
        // A 30s sleeper: kill() must return almost immediately (kill + reap), not
        // block until the process would naturally exit.
        let mut sb = HostExecutor
            .launch(service_spec("sh", &["-c", "sleep 30"]))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), sb.kill())
            .await
            .expect("kill() must return promptly, not wait out the sleep");
    }

    #[tokio::test]
    async fn kill_is_idempotent() {
        let mut sb = HostExecutor
            .launch(service_spec("sh", &["-c", "sleep 30"]))
            .unwrap();
        sb.kill().await;
        // A second kill on an already-reaped child returns promptly and does not panic.
        tokio::time::timeout(Duration::from_secs(5), sb.kill())
            .await
            .expect("second kill() must return promptly");
    }

    #[tokio::test]
    async fn host_executor_runs_and_captures_stdout() {
        let mut sb = HostExecutor.launch(spec("sh", &["-c", "echo hi"])).unwrap();
        let mut out = sb.take_stdout().unwrap();
        let mut buf = String::new();
        use tokio::io::AsyncReadExt;
        out.read_to_string(&mut buf).await.unwrap();
        let status = tokio::time::timeout(Duration::from_secs(5), sb.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success());
        assert!(buf.contains("hi"));
    }

    #[test]
    fn host_descriptor_is_host_mechanism() {
        assert_eq!(HostExecutor.describe().mechanism, "host");
    }
}
