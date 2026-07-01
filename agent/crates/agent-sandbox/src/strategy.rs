use crate::{docker_run_args, SandboxPolicy};
use agent_tools::{
    CommandSpec, Mode, ProcKind, SandboxDescriptor, SandboxError, SandboxStrategy, SandboxedChild,
};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq)]
pub enum Availability {
    Available,
    Unavailable(String),
}

pub struct DockerSandbox {
    policy: SandboxPolicy,
    uid_gid: String,
    /// Cached probe result. Written only by `resolve_availability` (auto-mode
    /// re-probe); read by `describe()` and `launch()`.
    available: RwLock<Availability>,
    /// Injectable so tests never need a Docker daemon. Defaults to `Self::probe`.
    prober: Box<dyn Fn() -> Availability + Send + Sync>,
}

impl DockerSandbox {
    pub fn new(policy: SandboxPolicy, uid_gid: String, available: Availability) -> Self {
        Self {
            policy,
            uid_gid,
            available: RwLock::new(available),
            prober: Box::new(Self::probe),
        }
    }

    #[cfg(test)]
    fn with_prober(mut self, p: impl Fn() -> Availability + Send + Sync + 'static) -> Self {
        self.prober = Box::new(p);
        self
    }

    /// Current availability, re-probing once in `auto` mode when the cache says
    /// unavailable — Docker may have come up since startup, and "start Docker
    /// and retry" should work without restarting the session. `Enforce` never
    /// re-probes: probe once at startup, refuse thereafter.
    fn resolve_availability(&self) -> Availability {
        let cached = self.available.read().unwrap().clone();
        match (&cached, self.policy.mode) {
            (Availability::Unavailable(_), Mode::Auto) => {
                let fresh = (self.prober)();
                *self.available.write().unwrap() = fresh.clone();
                fresh
            }
            _ => cached,
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
        match self.resolve_availability() {
            Availability::Available => self.spawn_docker(&spec, &name),
            Availability::Unavailable(reason) => match self.policy.mode {
                Mode::Enforce => Err(SandboxError::Unavailable(reason)),
                _ => {
                    // auto: fail closed. The old degrade-to-host arm is gone —
                    // unsandboxed execution is an explicit config choice only.
                    tracing::warn!(target: "sandbox", reason=%reason,
                        "docker unavailable; refusing exec (fail-closed)");
                    Err(SandboxError::Unavailable(format!(
                        "docker unreachable ({reason}); command refused — start Docker, \
                         or set sandbox_mode=\"off\" to accept unsandboxed execution"
                    )))
                }
            },
        }
    }

    fn describe(&self) -> SandboxDescriptor {
        let degraded = match &*self.available.read().unwrap() {
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
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

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

    #[test]
    fn auto_refuses_when_unavailable() {
        // Pin the re-probe to "still unavailable" so the test is hermetic — auto
        // always re-probes a cached-unavailable state, and the default prober
        // would shell out to the host's real Docker.
        let sb = DockerSandbox::new(
            policy(Mode::Auto),
            "1000:1000".into(),
            Availability::Unavailable("no daemon".into()),
        )
        .with_prober(|| Availability::Unavailable("no daemon".into()));
        let result = sb.launch(spec());
        let Err(SandboxError::Unavailable(msg)) = result else {
            panic!("auto + degraded must refuse, got a launch");
        };
        assert!(msg.contains("sandbox_mode"), "refusal must name the opt-out: {msg}");
        assert!(msg.contains("no daemon"), "refusal must carry the probe reason: {msg}");
    }

    #[test]
    fn auto_reprobe_updates_cache_and_message() {
        // Prober says "still down" — launch must re-probe (auto), refuse with the
        // FRESH reason, and update the cached availability describe() reads.
        let sb = DockerSandbox::new(
            policy(Mode::Auto),
            "1000:1000".into(),
            Availability::Unavailable("boot reason".into()),
        )
        .with_prober(|| Availability::Unavailable("still down".into()));
        let Err(SandboxError::Unavailable(msg)) = sb.launch(spec()) else {
            panic!("must refuse");
        };
        assert!(msg.contains("still down"), "must carry the re-probed reason: {msg}");
        assert_eq!(sb.describe().degraded.as_deref(), Some("still down"));
    }

    #[test]
    fn auto_reprobe_recovers_when_docker_comes_up() {
        let sb = DockerSandbox::new(
            policy(Mode::Auto),
            "1000:1000".into(),
            Availability::Unavailable("no daemon".into()),
        )
        .with_prober(|| Availability::Available);
        // Don't spawn a real container: assert on the resolved availability + posture.
        assert_eq!(sb.resolve_availability(), Availability::Available);
        assert!(sb.describe().degraded.is_none(), "recovery must clear the degraded posture");
    }

    #[test]
    fn enforce_never_reprobes() {
        let count = std::sync::Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let sb = DockerSandbox::new(
            policy(Mode::Enforce),
            "1000:1000".into(),
            Availability::Unavailable("no daemon".into()),
        )
        .with_prober(move || {
            c.fetch_add(1, AtomicOrdering::SeqCst);
            Availability::Available
        });
        assert!(matches!(sb.launch(spec()), Err(SandboxError::Unavailable(_))));
        assert_eq!(count.load(AtomicOrdering::SeqCst), 0, "enforce must not re-probe");
    }

    #[test]
    fn describe_never_probes() {
        let count = std::sync::Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let sb = DockerSandbox::new(
            policy(Mode::Auto),
            "1000:1000".into(),
            Availability::Unavailable("no daemon".into()),
        )
        .with_prober(move || {
            c.fetch_add(1, AtomicOrdering::SeqCst);
            Availability::Available
        });
        let _ = sb.describe();
        let _ = sb.describe();
        assert_eq!(count.load(AtomicOrdering::SeqCst), 0, "describe() must stay a cached read");
    }
}
