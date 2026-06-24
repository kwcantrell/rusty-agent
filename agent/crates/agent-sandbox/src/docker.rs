use agent_tools::{CommandSpec, Limits, Mode, ProcKind};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    pub mode: Mode,
    pub image: String,
    pub network: bool,
    pub limits: Limits,
    pub extra_rw: Vec<PathBuf>, // already validated (Task 5)
    pub extra_ro: Vec<PathBuf>,
}

pub const WORKDIR: &str = "/workspace";

/// Build the full `docker run …` argument vector (excluding the leading "docker").
pub fn docker_run_args(policy: &SandboxPolicy, spec: &CommandSpec,
    container_name: &str, uid_gid: &str) -> Vec<String> {
    let mut a: Vec<String> = vec!["run".into()];
    match spec.kind {
        ProcKind::OneShot => a.push("--rm".into()),
        ProcKind::Service => { a.push("-i".into()); } // keep stdin open; no --rm (we kill by name)
    }
    a.push("--name".into()); a.push(container_name.into());
    a.push("--network".into());
    a.push(if policy.network { "bridge".into() } else { "none".into() });
    a.push("--memory".into());      a.push(policy.limits.memory.clone());
    a.push("--cpus".into());        a.push(policy.limits.cpus.clone());
    a.push("--pids-limit".into());  a.push(policy.limits.pids.to_string());
    if let Some(f) = &policy.limits.fsize {
        a.push("--ulimit".into());  a.push(format!("fsize={f}"));
    }
    a.push("--read-only".into());
    a.push("--tmpfs".into());       a.push(format!("/tmp:rw,size={}", policy.limits.tmp_size));
    a.push("--cap-drop".into());    a.push("ALL".into());
    a.push("--security-opt".into()); a.push("no-new-privileges".into());
    a.push("--user".into());        a.push(uid_gid.into());
    // Workspace mount (RW) at a fixed path.
    a.push("-v".into());
    a.push(format!("{}:{}", spec.cwd.display(), WORKDIR));
    a.push("-w".into());            a.push(WORKDIR.into());
    for p in &policy.extra_rw {
        a.push("-v".into()); a.push(format!("{}:{}:rw", p.display(), p.display()));
    }
    for p in &policy.extra_ro {
        a.push("-v".into()); a.push(format!("{}:{}:ro", p.display(), p.display()));
    }
    // Env (-e KEY=VAL), sorted for determinism.
    for (k, v) in &spec.env {
        a.push("-e".into()); a.push(format!("{k}={v}"));
    }
    a.push(policy.image.clone());
    a.push(spec.program.clone());
    a.extend(spec.args.iter().cloned());
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_tools::{CommandSpec, Limits, Mode, ProcKind};
    use std::path::PathBuf;

    fn policy(network: bool) -> SandboxPolicy {
        SandboxPolicy { mode: Mode::Auto, image: "debian:stable-slim".into(), network,
            limits: Limits { memory: "2g".into(), cpus: "2".into(), pids: 512,
                fsize: None, tmp_size: "256m".into() },
            extra_rw: vec![], extra_ro: vec![] }
    }
    fn oneshot() -> CommandSpec {
        CommandSpec { program: "sh".into(), args: vec!["-c".into(), "echo hi".into()],
            cwd: PathBuf::from("/work"), env: Default::default(), kind: ProcKind::OneShot }
    }

    #[test]
    fn oneshot_has_hardening_flags_and_network_none() {
        let v = docker_run_args(&policy(false), &oneshot(), "agent-sbx-1", "1000:1000");
        let s = v.join(" ");
        assert!(s.contains("run --rm"));
        assert!(s.contains("--network none"));
        assert!(s.contains("--read-only"));
        assert!(s.contains("--cap-drop ALL"));
        assert!(s.contains("--security-opt no-new-privileges"));
        assert!(s.contains("--user 1000:1000"));
        assert!(s.contains("-v /work:/workspace"));
        assert!(s.contains("-w /workspace"));
        assert!(s.ends_with("debian:stable-slim sh -c echo hi"));
        assert!(!s.contains("--privileged"));
        assert!(!s.contains("seccomp=unconfined"));
    }

    #[test]
    fn network_true_uses_bridge() {
        let v = docker_run_args(&policy(true), &oneshot(), "n", "1000:1000");
        assert!(v.join(" ").contains("--network bridge"));
    }

    #[test]
    fn service_keeps_stdin_open_and_no_rm() {
        let mut spec = oneshot(); spec.kind = ProcKind::Service;
        let v = docker_run_args(&policy(false), &spec, "n", "1000:1000");
        assert!(v.contains(&"-i".to_string()));
        assert!(!v.contains(&"--rm".to_string()));
    }
}
