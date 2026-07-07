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

/// Env keys the docker CLI itself interprets (client-control, not secrets):
/// live on the client process they would redirect daemon/auth/config
/// discovery, so they ride argv as `-e K=V` and are excluded from the
/// client env in spawn_docker. HOME is here because it moves the CLI's
/// ~/.docker/config.json discovery (and its value is not a secret).
pub const DOCKER_CLIENT_CONTROL_KEYS: &[&str] = &[
    "DOCKER_HOST",
    "DOCKER_CONFIG",
    "DOCKER_CERT_PATH",
    "DOCKER_TLS_VERIFY",
    "DOCKER_CONTEXT",
    "HOME",
];

/// Build the full `docker run …` argument vector (excluding the leading "docker").
pub fn docker_run_args(
    policy: &SandboxPolicy,
    spec: &CommandSpec,
    container_name: &str,
    uid_gid: &str,
) -> Vec<String> {
    let mut a: Vec<String> = vec!["run".into()];
    match spec.kind {
        ProcKind::OneShot => a.push("--rm".into()),
        ProcKind::Service => {
            a.push("-i".into());
            a.push("--rm".into());
        } // keep stdin open; --rm auto-removes on exit/kill
    }
    a.push("--name".into());
    a.push(container_name.into());
    a.push("--network".into());
    a.push(if policy.network {
        "bridge".into()
    } else {
        "none".into()
    });
    a.push("--memory".into());
    a.push(policy.limits.memory.clone());
    a.push("--cpus".into());
    a.push(policy.limits.cpus.clone());
    a.push("--pids-limit".into());
    a.push(policy.limits.pids.to_string());
    if let Some(f) = &policy.limits.fsize {
        a.push("--ulimit".into());
        a.push(format!("fsize={f}"));
    }
    a.push("--read-only".into());
    a.push("--tmpfs".into());
    a.push(format!("/tmp:rw,size={}", policy.limits.tmp_size));
    a.push("--cap-drop".into());
    a.push("ALL".into());
    a.push("--security-opt".into());
    a.push("no-new-privileges".into());
    a.push("--user".into());
    a.push(uid_gid.into());
    // Workspace mount (RW) at a fixed path.
    a.push("-v".into());
    a.push(format!("{}:{}", spec.cwd.display(), WORKDIR));
    a.push("-w".into());
    a.push(WORKDIR.into());
    // extra_rw mounts widen the writable boundary BEYOND /workspace to a host-visible path
    // (validated against /, $HOME root, and the docker socket, but still host-writable).
    for p in &policy.extra_rw {
        a.push("-v".into());
        a.push(format!("{}:{}:rw", p.display(), p.display()));
    }
    for p in &policy.extra_ro {
        a.push("-v".into());
        a.push(format!("{}:{}:ro", p.display(), p.display()));
    }
    // Env: name-only `-e KEY`, sorted for determinism (BTreeMap) — values
    // travel on the docker CLIENT process env (spawn_docker sets cmd.envs) so
    // secrets never appear in world-readable argv or `docker inspect`
    // (audit 3.1). Client-control keys are the exception: they stay in argv
    // `-e K=V` form (non-secret) and never reach the client env.
    for (k, v) in &spec.env {
        a.push("-e".into());
        if DOCKER_CLIENT_CONTROL_KEYS.contains(&k.as_str()) {
            a.push(format!("{k}={v}"));
        } else {
            a.push(k.clone());
        }
    }
    // --user with no passwd entry leaves HOME=/ (read-only) — node/npx tooling
    // needs a writable HOME for caches. Default to the tmpfs unless the spec set one.
    if !spec.env.contains_key("HOME") {
        a.push("-e".into());
        a.push("HOME=/tmp".into());
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
        SandboxPolicy {
            mode: Mode::Auto,
            image: "debian:stable-slim".into(),
            network,
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
    fn oneshot() -> CommandSpec {
        CommandSpec {
            program: "sh".into(),
            args: vec!["-c".into(), "echo hi".into()],
            cwd: PathBuf::from("/work"),
            env: Default::default(),
            kind: ProcKind::OneShot,
        }
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
        // Resource-limit flags (always emitted)
        assert!(s.contains("--memory 2g"));
        assert!(s.contains("--cpus 2"));
        assert!(s.contains("--pids-limit 512"));
        assert!(s.contains("--tmpfs /tmp:rw,size=256m"));
        assert!(s.contains("--name agent-sbx-1"));
        // Dangerous flags absent
        assert!(!s.contains("--cap-add"));
    }

    #[test]
    fn network_true_uses_bridge() {
        let v = docker_run_args(&policy(true), &oneshot(), "n", "1000:1000");
        assert!(v.join(" ").contains("--network bridge"));
    }

    #[test]
    fn service_keeps_stdin_open_and_rm() {
        let mut spec = oneshot();
        spec.kind = ProcKind::Service;
        let v = docker_run_args(&policy(false), &spec, "n", "1000:1000");
        assert!(v.contains(&"-i".to_string()), "Service must have -i");
        assert!(
            v.contains(&"--rm".to_string()),
            "Service must have --rm to prevent container leaks"
        );
        // OneShot must NOT get -i
        let v2 = docker_run_args(&policy(false), &oneshot(), "n", "1000:1000");
        assert!(!v2.contains(&"-i".to_string()), "OneShot must not have -i");
    }

    #[test]
    fn ulimit_fsize_emitted_only_when_set() {
        // None → no --ulimit
        let v = docker_run_args(&policy(false), &oneshot(), "n", "1000:1000");
        assert!(!v.join(" ").contains("--ulimit"));
        // Some → --ulimit fsize=<f>
        let mut p = policy(false);
        p.limits.fsize = Some("1g".into());
        let v = docker_run_args(&p, &oneshot(), "n", "1000:1000");
        assert!(v.join(" ").contains("--ulimit fsize=1g"));
    }

    #[test]
    fn env_values_stay_out_of_argv() {
        let mut spec = oneshot();
        spec.env.insert("API_KEY".into(), "sekret-value".into());
        let v = docker_run_args(&policy(false), &spec, "n", "1000:1000");
        let s = v.join(" ");
        assert!(s.contains("-e API_KEY"), "name-only -e for spec env: {s}");
        assert!(
            !s.contains("sekret-value"),
            "value must never reach argv: {s}"
        );
        assert!(!s.contains("API_KEY="), "no KEY=VALUE form in argv: {s}");
    }

    #[test]
    fn docker_client_control_keys_stay_in_argv() {
        let mut spec = oneshot();
        spec.env
            .insert("DOCKER_HOST".into(), "tcp://evil:2375".into());
        spec.env.insert("API_KEY".into(), "sekret-value".into());
        let v = docker_run_args(&policy(false), &spec, "n", "1000:1000");
        let s = v.join(" ");
        assert!(
            s.contains("-e DOCKER_HOST=tcp://evil:2375"),
            "client-control keys ride argv with their value: {s}"
        );
        assert!(
            s.contains("-e API_KEY") && !s.contains("sekret-value"),
            "secret keys stay name-only: {s}"
        );
    }

    #[test]
    fn home_defaults_to_tmp_unless_spec_sets_it() {
        let v = docker_run_args(&policy(false), &oneshot(), "n", "1000:1000");
        assert!(
            v.join(" ").contains("-e HOME=/tmp"),
            "default HOME on writable tmpfs"
        );
        let mut spec = oneshot();
        spec.env.insert("HOME".into(), "/workspace".into());
        let s = docker_run_args(&policy(false), &spec, "n", "1000:1000").join(" ");
        assert!(
            s.contains("-e HOME=/workspace") && !s.contains("-e HOME=/tmp"),
            "spec-set HOME rides argv (client-control, non-secret): {s}"
        );
    }
}
