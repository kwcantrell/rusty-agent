use crate::{docker_run_args, SandboxPolicy};
use agent_tools::{
    CommandSpec, HostExecutor, Mode, ProcKind, SandboxDescriptor, SandboxError, SandboxStrategy,
    SandboxedChild,
};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub enum Availability {
    Available,
    Unavailable(String),
}

pub struct DockerSandbox {
    policy: SandboxPolicy,
    uid_gid: String,
    available: Availability,
}

impl DockerSandbox {
    pub fn new(policy: SandboxPolicy, uid_gid: String, available: Availability) -> Self {
        Self {
            policy,
            uid_gid,
            available,
        }
    }

    /// Blocking availability probe; run once at startup before `new`.
    pub fn probe() -> Availability {
        match std::process::Command::new("docker")
            .args(["version", "--format", "{{.Server.Version}}"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(s) if s.success() => Availability::Available,
            Ok(s) => Availability::Unavailable(format!("docker version exited {s}")),
            Err(e) => Availability::Unavailable(e.to_string()),
        }
    }

    fn spawn_docker(&self, spec: &CommandSpec, name: &str) -> Result<SandboxedChild, SandboxError> {
        let args = docker_run_args(&self.policy, spec, name, &self.uid_gid);
        let mut cmd = tokio::process::Command::new("docker");
        cmd.args(&args)
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match spec.kind {
            ProcKind::Service => {
                cmd.stdin(Stdio::piped());
            }
            ProcKind::OneShot => {
                cmd.stdin(Stdio::null());
            }
        }
        tracing::info!(target: "sandbox", mechanism="docker", image=%self.policy.image,
            network=self.policy.network, container=%name, "launching sandboxed process");
        let child = cmd
            .spawn()
            .map_err(|e| SandboxError::LaunchFailed(e.to_string()))?;
        Ok(SandboxedChild::new_container(child, name.to_string()))
    }
}

impl SandboxStrategy for DockerSandbox {
    fn launch(&self, spec: CommandSpec) -> Result<SandboxedChild, SandboxError> {
        let name = format!(
            "agent-sbx-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        match &self.available {
            Availability::Available => self.spawn_docker(&spec, &name),
            Availability::Unavailable(reason) => match self.policy.mode {
                Mode::Enforce => Err(SandboxError::Unavailable(reason.clone())),
                _ => {
                    // auto (or off, though off never wires DockerSandbox): degrade to host.
                    tracing::warn!(target: "sandbox", reason=%reason,
                        "docker unavailable; degrading to host execution");
                    HostExecutor.launch(spec)
                }
            },
        }
    }

    fn describe(&self) -> SandboxDescriptor {
        let degraded = match &self.available {
            Availability::Unavailable(r) => Some(r.clone()),
            Availability::Available => None,
        };
        SandboxDescriptor {
            mode: self.policy.mode,
            mechanism: "docker",
            image: Some(self.policy.image.clone()),
            network: self.policy.network,
            degraded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_tools::{Limits, Mode};

    fn policy(mode: Mode) -> SandboxPolicy {
        SandboxPolicy {
            mode,
            image: "debian:stable-slim".into(),
            network: false,
            limits: Limits {
                memory: "2g".into(),
                cpus: "2".into(),
                pids: 512,
                fsize: None,
                tmp_size: "256m".into(),
            },
            extra_rw: vec![],
            extra_ro: vec![],
        }
    }
    fn spec() -> CommandSpec {
        CommandSpec {
            program: "sh".into(),
            args: vec!["-c".into(), "true".into()],
            cwd: std::env::temp_dir(),
            env: Default::default(),
            kind: ProcKind::OneShot,
        }
    }

    #[test]
    fn enforce_denies_when_unavailable() {
        let sb = DockerSandbox::new(
            policy(Mode::Enforce),
            "1000:1000".into(),
            Availability::Unavailable("no daemon".into()),
        );
        let result = sb.launch(spec());
        assert!(matches!(result, Err(SandboxError::Unavailable(_))));
        assert!(sb.describe().degraded.is_some());
    }

    #[tokio::test]
    async fn auto_degrades_to_host_when_unavailable() {
        let sb = DockerSandbox::new(
            policy(Mode::Auto),
            "1000:1000".into(),
            Availability::Unavailable("no daemon".into()),
        );
        // Host fallback actually runs `sh -c true`.
        let mut child = sb.launch(spec()).expect("auto degrades, does not error");
        let status = child.wait().await.unwrap();
        assert!(status.success());
    }
}
