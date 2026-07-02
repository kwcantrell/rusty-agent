use crate::{docker_run_args, SandboxPolicy};
use agent_tools::{
    CommandSpec, Mode, ProcKind, SandboxDescriptor, SandboxError, SandboxStrategy, SandboxedChild,
};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Hard deadline on the `docker version` probe: a wedged daemon (socket
/// accepts, never responds) must not pin a worker indefinitely.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

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
    /// Single-flights the auto-mode re-probe: concurrent launches during a
    /// degraded burst contend here instead of each probing independently.
    probe_lock: Mutex<()>,
    /// Injectable so tests never need a Docker daemon. Defaults to `Self::probe`.
    prober: Box<dyn Fn() -> Availability + Send + Sync>,
}

impl DockerSandbox {
    pub fn new(policy: SandboxPolicy, uid_gid: String, available: Availability) -> Self {
        Self {
            policy,
            uid_gid,
            available: RwLock::new(available),
            probe_lock: Mutex::new(()),
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
                // Single-flight: only one launch probes at a time. Losers of
                // the race block here briefly (bounded by PROBE_TIMEOUT), then
                // the double-check below reads the winner's fresh result.
                let _guard = self.probe_lock.lock().unwrap();
                let recheck = self.available.read().unwrap().clone();
                if recheck == Availability::Available {
                    return recheck;
                }
                let fresh = (self.prober)();
                *self.available.write().unwrap() = fresh.clone();
                fresh
            }
            _ => cached,
        }
    }

    /// Blocking availability probe; run once at startup before `new`.
    /// Bounded by [`PROBE_TIMEOUT`] so a wedged daemon cannot hang the caller.
    pub fn probe() -> Availability {
        let mut cmd = std::process::Command::new("docker");
        cmd.args(["version", "--format", "{{.Server.Version}}"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Self::wait_bounded(cmd, PROBE_TIMEOUT)
    }

    /// Run `cmd` to completion with a hard deadline: poll `try_wait` in a
    /// short sleep loop; on deadline, kill + reap the child and report the
    /// timeout as `Unavailable`. Std-only so the probe stays runnable outside
    /// a tokio context.
    fn wait_bounded(mut cmd: std::process::Command, timeout: Duration) -> Availability {
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Availability::Unavailable(e.to_string()),
        };
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(s)) if s.success() => return Availability::Available,
                Ok(Some(s)) => {
                    return Availability::Unavailable(format!("docker version exited {s}"))
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Availability::Unavailable(format!(
                            "docker probe timed out after {timeout:?}"
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Availability::Unavailable(e.to_string()),
            }
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
                Mode::Enforce => Err(SandboxError::Unavailable(format!(
                    "docker unreachable ({reason}); command refused — start Docker, \
                     or set sandbox_mode=\"off\" to accept unsandboxed execution \
                     (sandbox_mode=\"enforce\" never degrades)"
                ))),
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
        let Err(SandboxError::Unavailable(msg)) = result else {
            panic!("enforce + unavailable must refuse");
        };
        assert!(
            msg.contains("start Docker"),
            "enforce refusal must be actionable: {msg}"
        );
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
        assert!(
            msg.contains("sandbox_mode"),
            "refusal must name the opt-out: {msg}"
        );
        assert!(
            msg.contains("no daemon"),
            "refusal must carry the probe reason: {msg}"
        );
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
        assert!(
            msg.contains("still down"),
            "must carry the re-probed reason: {msg}"
        );
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
        assert!(
            sb.describe().degraded.is_none(),
            "recovery must clear the degraded posture"
        );
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
        assert!(matches!(
            sb.launch(spec()),
            Err(SandboxError::Unavailable(_))
        ));
        assert_eq!(
            count.load(AtomicOrdering::SeqCst),
            0,
            "enforce must not re-probe"
        );
    }

    #[test]
    fn wait_bounded_kills_a_wedged_probe_at_the_deadline() {
        let mut cmd = std::process::Command::new("sh");
        cmd.args(["-c", "sleep 10"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let start = Instant::now();
        let got = DockerSandbox::wait_bounded(cmd, Duration::from_millis(500));
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "bounded wait must not ride out the child; took {elapsed:?}"
        );
        let Availability::Unavailable(msg) = got else {
            panic!("a timed-out probe must report Unavailable");
        };
        assert!(msg.contains("timed out"), "must name the timeout: {msg}");
    }

    #[test]
    fn wait_bounded_reports_fast_exits() {
        let mut ok = std::process::Command::new("sh");
        ok.args(["-c", "true"]);
        assert_eq!(
            DockerSandbox::wait_bounded(ok, Duration::from_secs(2)),
            Availability::Available
        );
        let mut bad = std::process::Command::new("sh");
        bad.args(["-c", "exit 3"]);
        assert!(matches!(
            DockerSandbox::wait_bounded(bad, Duration::from_secs(2)),
            Availability::Unavailable(_)
        ));
    }

    #[test]
    fn concurrent_launches_single_flight_the_reprobe() {
        // Four threads race resolve_availability() against a cached-unavailable
        // state. The prober flips the cache to Available (after a short sleep so
        // the others are very likely in-flight). Whatever the interleaving, the
        // structure guarantees exactly one probe: latecomers see Available at the
        // top-of-function read, and race losers see it at the double-check under
        // the probe lock. count == 1 is therefore a deterministic assertion.
        let count = std::sync::Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        let sb = DockerSandbox::new(
            policy(Mode::Auto),
            "1000:1000".into(),
            Availability::Unavailable("no daemon".into()),
        )
        .with_prober(move || {
            c.fetch_add(1, AtomicOrdering::SeqCst);
            std::thread::sleep(Duration::from_millis(100));
            Availability::Available
        });
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..4)
                .map(|_| s.spawn(|| sb.resolve_availability()))
                .collect();
            for h in handles {
                assert_eq!(h.join().unwrap(), Availability::Available);
            }
        });
        assert_eq!(
            count.load(AtomicOrdering::SeqCst),
            1,
            "exactly one caller may probe; the rest must read the fresh cache"
        );
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
        assert_eq!(
            count.load(AtomicOrdering::SeqCst),
            0,
            "describe() must stay a cached read"
        );
    }
}
