use agent_sandbox::{Availability, DockerSandbox, SandboxPolicy};
use agent_tools::{CommandSpec, Limits, Mode, ProcKind, SandboxStrategy};
use tokio::io::AsyncReadExt;

fn policy(network: bool) -> SandboxPolicy {
    SandboxPolicy {
        mode: Mode::Enforce,
        image: "debian:stable-slim".into(),
        network,
        limits: Limits {
            memory: "256m".into(),
            cpus: "1".into(),
            pids: 128,
            fsize: None,
            tmp_size: "64m".into(),
        },
        extra_rw: vec![],
        extra_ro: vec![],
    }
}

fn cmd(c: &str, ws: std::path::PathBuf) -> CommandSpec {
    CommandSpec {
        program: "sh".into(),
        args: vec!["-c".into(), c.into()],
        cwd: ws,
        env: Default::default(),
        kind: ProcKind::OneShot,
    }
}

async fn run(network: bool, ws: std::path::PathBuf, c: &str) -> (i32, String) {
    let sb = DockerSandbox::new(policy(network), "0:0".into(), Availability::Available);
    let mut child = sb.launch(cmd(c, ws)).unwrap();
    let mut out = child.take_stdout().unwrap();
    let mut s = String::new();
    out.read_to_string(&mut s).await.unwrap();
    let code = child.wait().await.unwrap().code().unwrap_or(-1);
    (code, s)
}

/// Helper to get the current user's uid by shelling out to `id -u`
fn id_of(flag: &str) -> u32 {
    let out = std::process::Command::new("id")
        .arg(flag)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

#[tokio::test]
#[ignore]
async fn host_filesystem_is_not_visible() {
    // Create a secret file on the HOST that is NOT mounted into the container.
    let secret_dir = tempfile::tempdir().unwrap();
    let secret = secret_dir.path().join("host_secret.txt");
    std::fs::write(&secret, "TOPSECRET").unwrap();

    // The workspace is a SEPARATE tempdir; secret_dir is never mounted.
    let ws = tempfile::tempdir().unwrap();
    let sb = DockerSandbox::new(policy(false), "0:0".into(), Availability::Available);
    let mut child = sb
        .launch(cmd(&format!("cat {}", secret.display()), ws.path().into()))
        .unwrap();
    let mut out = child.take_stdout().unwrap();
    let mut s = String::new();
    out.read_to_string(&mut s).await.unwrap();
    let code = child.wait().await.unwrap().code().unwrap_or(-1);

    assert_ne!(code, 0, "host secret must be unreachable inside the sandbox");
    assert!(!s.contains("TOPSECRET"), "host secret content must not leak");
}

#[tokio::test]
#[ignore]
async fn rootfs_is_read_only() {
    let (code, _) = run(false, std::env::temp_dir(), "echo x > /etc/escape").await;
    assert_ne!(code, 0);
}

#[tokio::test]
#[ignore]
async fn network_is_off_by_default() {
    let (code, _) = run(
        false,
        std::env::temp_dir(),
        "getent hosts example.com || exit 7",
    )
    .await;
    assert_ne!(code, 0);
}

#[tokio::test]
#[ignore]
async fn workspace_is_writable() {
    let ws = tempfile::tempdir().unwrap();
    let uid = id_of("-u");
    let gid = id_of("-g");
    let uid_gid = format!("{}:{}", uid, gid);
    let mut child = DockerSandbox::new(policy(false), uid_gid, Availability::Available)
        .launch(cmd(
            "touch /workspace/made_in_container",
            ws.path().into(),
        ))
        .unwrap();
    assert!(child.wait().await.unwrap().success());
    assert!(ws.path().join("made_in_container").exists());
}

#[tokio::test]
#[ignore]
async fn workspace_is_host_owned() {
    let ws = tempfile::tempdir().unwrap();
    let uid = id_of("-u");
    let gid = id_of("-g");
    let uid_gid = format!("{}:{}", uid, gid);
    let sb = DockerSandbox::new(policy(false), uid_gid, Availability::Available);
    let mut child = sb
        .launch(cmd(
            "touch /workspace/made_in_container",
            ws.path().into(),
        ))
        .unwrap();
    assert!(child.wait().await.unwrap().success());
    let meta = std::fs::metadata(ws.path().join("made_in_container")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(meta.uid(), uid);
        assert_eq!(meta.gid(), gid);
    }
}
