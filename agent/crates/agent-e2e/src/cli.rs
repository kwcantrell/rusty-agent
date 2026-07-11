//! CLI subprocess driver: spawns the real `agent` binary (agent-cli crate)
//! against a `Rig`-isolated workspace/sessions/metadata root, and drives it
//! over stdin/stdout/stderr like a human would.
use crate::rig::Rig;
use agent_server::wire::ServerEvent;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The REPL's line prompt, printed by agent-cli's `main.rs` (~line 713) as
/// `print!("\n\x1b[1m›\x1b[0m ")` before it blocks on stdin for a task line.
/// This is the stable, ANSI-code-free substring of that prompt.
pub const REPL_MARKER: &str = "›";

static AGENT_BIN: OnceLock<PathBuf> = OnceLock::new();

/// Freshness rule (spec §2.2 item 5): build once per test process, then use.
pub fn agent_bin() -> PathBuf {
    AGENT_BIN
        .get_or_init(|| {
            let ws = workspace_root();
            let status = Command::new("cargo")
                .args(["build", "-p", "agent-cli", "--quiet"])
                .current_dir(&ws)
                .status()
                .expect("cargo build -p agent-cli");
            assert!(status.success(), "agent-cli build failed");
            target_dir(&ws).join("debug/agent")
        })
        .clone()
}

/// Honor CARGO_TARGET_DIR when set (plan-review F7).
fn target_dir(ws: &std::path::Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| ws.join("target"))
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = .../agent/crates/agent-e2e → workspace = two up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Own crate's bin: cargo sets `CARGO_BIN_EXE_e2e-daemon` when the test binary
/// is built via `cargo test` in this crate (hyphens in the bin name are
/// literal in that env var — this is NOT `CARGO_BIN_EXE_e2e_daemon`). Fall
/// back to an explicit build for callers that don't get that env var (e.g. a
/// doctest harness or a differently-invoked test runner).
pub fn e2e_daemon_bin() -> PathBuf {
    option_env!("CARGO_BIN_EXE_e2e-daemon")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let ws = workspace_root();
            let status = Command::new("cargo")
                .args(["build", "-p", "agent-e2e", "--bin", "e2e-daemon", "--quiet"])
                .current_dir(&ws)
                .status()
                .expect("build e2e-daemon");
            assert!(status.success(), "e2e-daemon build failed");
            target_dir(&ws).join("debug/e2e-daemon")
        })
}

pub struct CliCmd {
    pub(crate) cmd: Command,
    /// Deferred: pushed onto `cmd` only in `spawn()` (before `subcommand`, see
    /// below) so `approval_timeout_secs` can override the default without clap
    /// rejecting a duplicate single-value `--approval-timeout-secs` flag.
    /// `None` for non-`agent-cli` commands (`DaemonCmd`, via `from_command`)
    /// that don't take this flag.
    approval_timeout_secs: Option<u64>,
    /// Deferred the same way as `approval_timeout_secs`: `new()` seeds this
    /// with `"stub-model"`, and `.model()` overwrites it before `spawn()`
    /// pushes a single `--model` flag — avoids ever emitting the flag twice.
    /// `None` for non-`agent-cli` commands (`from_command`).
    model: Option<String>,
    /// Same deferral shape as `model`, for `--stream-timeout-secs` (default
    /// 10s, overridden by `.stream_timeout_secs()` for live-model runs).
    stream_timeout_secs: Option<u64>,
    /// Deferred: a `sessions_sub` call is stashed here rather than appended
    /// immediately, so it always lands AFTER `approval_timeout_secs`'s flag
    /// pair regardless of builder call order — clap subcommands must come
    /// after top-level flags.
    subcommand: Vec<String>,
}

impl CliCmd {
    /// Bypasses the `agent-cli`-specific defaults below — used by `DaemonCmd`,
    /// which drives the unrelated `e2e-daemon` binary and has its own arg set
    /// (no `--approval-timeout-secs`), and by tests that need to build a
    /// fully owned (`'static`) `Command` themselves (e.g. to move it across a
    /// `spawn_blocking` boundary without borrowing a `Rig`).
    pub fn from_command(cmd: Command) -> Self {
        CliCmd {
            cmd,
            approval_timeout_secs: None,
            model: None,
            stream_timeout_secs: None,
            subcommand: Vec::new(),
        }
    }

    pub fn new(rig: &Rig, base_url: &str) -> Self {
        let mut cmd = Command::new(agent_bin());
        cmd.args([
            "--base-url",
            base_url,
            "--workspace",
            rig.workspace.path().to_str().unwrap(),
            "--trace-dir",
            rig.sessions.path().to_str().unwrap(),
            "--metadata-dir",
            rig.meta.path().to_str().unwrap(),
        ]);
        cmd.env("HOME", rig.meta.path()) // belt-and-braces (spec §2.3)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0); // own group → group-kill can't touch the test
        CliCmd {
            cmd,
            approval_timeout_secs: Some(20),
            model: Some("stub-model".to_string()),
            stream_timeout_secs: Some(10),
            subcommand: Vec::new(),
        }
    }

    /// Override the interactive approval window (default 20s). Use a short
    /// window (e.g. 1s) to deterministically drive a timeout park-and-exit.
    pub fn approval_timeout_secs(mut self, secs: u64) -> Self {
        self.approval_timeout_secs = Some(secs);
        self
    }

    /// Override the model name (default "stub-model") — for live-model runs
    /// where the CLI should request the name actually loaded server-side.
    pub fn model(mut self, model: &str) -> Self {
        self.model = Some(model.to_string());
        self
    }

    /// Override the model-stream idle timeout (default 10s) — live models
    /// need a generous window (e.g. 120s) instead of the stub-server default.
    pub fn stream_timeout_secs(mut self, secs: u64) -> Self {
        self.stream_timeout_secs = Some(secs);
        self
    }

    /// Subcommand form, e.g. `sessions_sub(&["sessions","reopen","<id>"])` —
    /// clap subcommands must come AFTER top-level flags (`Cli`'s `command`
    /// field is parsed alongside the flags; agent-cli's own doc comment on
    /// `Command::Sessions` says as much: "Top-level flags (--base-url,
    /// --workspace, etc.) go BEFORE the subcommand"). Stashed rather than
    /// appended immediately so it lands after `spawn()`'s deferred
    /// `--approval-timeout-secs` push regardless of call order.
    pub fn sessions_sub(mut self, sub: &[&str]) -> Self {
        self.subcommand = sub.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn arg(mut self, a: &str) -> Self {
        self.cmd.arg(a);
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: AsRef<std::ffi::OsStr>>(mut self, a: I) -> Self {
        self.cmd.args(a);
        self
    }

    pub fn spawn(mut self) -> Cli {
        if let Some(secs) = self.approval_timeout_secs {
            self.cmd
                .args(["--approval-timeout-secs", &secs.to_string()]);
        }
        if let Some(model) = &self.model {
            self.cmd.args(["--model", model]);
        }
        if let Some(secs) = self.stream_timeout_secs {
            self.cmd.args(["--stream-timeout-secs", &secs.to_string()]);
        }
        if !self.subcommand.is_empty() {
            self.cmd.args(&self.subcommand);
        }
        let mut child = self.cmd.spawn().expect("spawn agent");
        let stdin = child.stdin.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        // F1: prompts are print!-ed with NO trailing newline — read raw byte
        // chunks, never BufReader::lines() (a line reader never yields the
        // approval/feedback/REPL prompt and every waiter deadlines).
        fn pump(mut r: impl std::io::Read + Send + 'static, tx: std::sync::mpsc::Sender<String>) {
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match r.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if tx
                                .send(String::from_utf8_lossy(&buf[..n]).into_owned())
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                }
            });
        }
        pump(child.stdout.take().unwrap(), tx.clone());
        pump(child.stderr.take().unwrap(), tx);
        Cli {
            child,
            stdin: Some(stdin),
            rx,
            transcript: String::new(),
        }
    }
}

/// Driver for `e2e-daemon`'s `run`/`hold-lock` modes — same builder/waiter
/// reuse as `CliCmd`, but against the daemon binary instead of `agent-cli`.
pub struct DaemonCmd;

impl DaemonCmd {
    /// Spawn `e2e-daemon run` against `rig`'s isolated roots, optionally
    /// sending `--task` up front.
    pub fn run(rig: &Rig, base_url: &str, task: Option<&str>) -> Cli {
        let mut cmd = Command::new(e2e_daemon_bin());
        cmd.args([
            "run",
            "--workspace",
            rig.workspace.path().to_str().unwrap(),
            "--sessions",
            rig.sessions.path().to_str().unwrap(),
            "--meta",
            rig.meta.path().to_str().unwrap(),
            "--base-url",
            base_url,
        ]);
        if let Some(t) = task {
            cmd.args(["--task", t]);
        }
        cmd.env("HOME", rig.meta.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        CliCmd::from_command(cmd).spawn()
    }

    /// Spawn `e2e-daemon hold-lock --dir <dir>` — `dir` must be the
    /// CHECKPOINT dir (`rig::ckpt(&session_dir)`), not the session dir.
    pub fn hold_lock(rig: &Rig, dir: &std::path::Path) -> Cli {
        let mut cmd = Command::new(e2e_daemon_bin());
        cmd.args(["hold-lock", "--dir", dir.to_str().unwrap()]);
        cmd.env("HOME", rig.meta.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        CliCmd::from_command(cmd).spawn()
    }
}

pub struct Cli {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<String>,
    transcript: String,
}

impl Cli {
    pub fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    pub fn write_line(&mut self, s: &str) {
        let stdin = self.stdin.as_mut().expect("stdin already closed");
        writeln!(stdin, "{s}").unwrap();
        stdin.flush().unwrap();
    }

    pub fn close_stdin(&mut self) {
        self.stdin.take();
    }

    /// Non-blocking peek: has the child already exited? Used by callers that
    /// must tolerate EITHER a still-live prompt OR an already-occurred exit
    /// (e.g. a genuine race where this process may lose before ever
    /// prompting) without burning a full deadline on a `wait_for_output`
    /// that will never see its needle.
    pub fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    fn drain(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(chunk) => self.transcript.push_str(&chunk), // raw chunks (F1)
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return,
            }
        }
    }

    pub fn transcript(&mut self) -> String {
        self.drain();
        self.transcript.clone()
    }

    pub fn wait_for_output(&mut self, needle: &str, cap: Duration) -> String {
        let start = Instant::now();
        loop {
            self.drain();
            if self.transcript.contains(needle) {
                return self.transcript.clone();
            }
            assert!(
                start.elapsed() < cap,
                "deadline waiting for {needle:?}; transcript so far:\n{}",
                self.transcript
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Wait for a `ServerEvent` (from `EV `-prefixed lines emitted by
    /// `e2e-daemon run`) matching `pred`. EV lines are newline-terminated
    /// (`out()` in the daemon uses `println!`), so scanning `transcript.lines()`
    /// on the raw-chunk transcript is safe even though `Cli` otherwise never
    /// assumes line framing.
    pub fn wait_for_event(
        &mut self,
        cap: Duration,
        pred: impl Fn(&ServerEvent) -> bool,
    ) -> ServerEvent {
        let start = Instant::now();
        loop {
            self.drain();
            for line in self.transcript.lines() {
                if let Some(json) = line.strip_prefix("EV ") {
                    if let Ok(ev) = serde_json::from_str::<ServerEvent>(json) {
                        if pred(&ev) {
                            return ev;
                        }
                    }
                }
            }
            assert!(
                start.elapsed() < cap,
                "deadline waiting for event; transcript:\n{}",
                self.transcript
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn sigint(&self) {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(self.pid()),
            nix::sys::signal::Signal::SIGINT,
        )
        .unwrap();
    }

    pub fn sigkill(&self) {
        // group kill: negative pid (we set process_group(0) at spawn)
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(-self.pid()),
            nix::sys::signal::Signal::SIGKILL,
        );
    }

    pub fn wait_exit(&mut self, cap: Duration) -> std::process::ExitStatus {
        let start = Instant::now();
        loop {
            if let Some(st) = self.child.try_wait().unwrap() {
                self.drain();
                return st;
            }
            if start.elapsed() >= cap {
                self.sigkill();
                panic!(
                    "deadline waiting for exit; transcript:\n{}",
                    self.transcript()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        self.sigkill(); // KillOnDrop: held-Child group kill, never pattern-match
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_list_on_empty_root_exits_clean() {
        let rig = Rig::new();
        let mut cli = CliCmd::new(&rig, "http://127.0.0.1:1")
            .sessions_sub(&["sessions", "list"])
            .spawn();
        let st = cli.wait_exit(Duration::from_secs(30));
        assert!(st.success(), "transcript:\n{}", cli.transcript());
    }

    #[test]
    fn repl_prints_marker_before_reading_a_task_line() {
        let rig = Rig::new();
        // Unreachable base_url: the REPL still starts and prints its prompt
        // before it ever needs a model — no live server required.
        let mut cli = CliCmd::new(&rig, "http://127.0.0.1:1").spawn();
        let transcript = cli.wait_for_output(REPL_MARKER, Duration::from_secs(30));
        assert!(
            transcript.contains(REPL_MARKER),
            "transcript so far:\n{transcript}"
        );
        cli.close_stdin();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn daemon_runs_one_turn_and_streams_events() {
        let stub =
            crate::stub::ScriptedStub::start(vec![crate::stub::text_step(Some("hi"), "yo")]).await;
        let rig = Rig::new();
        let mut d = DaemonCmd::run(&rig, &stub.base_url(), Some("hi"));
        d.wait_for_output("READY", Duration::from_secs(30));
        d.wait_for_event(Duration::from_secs(30), |e| {
            matches!(e, ServerEvent::Done { .. })
        });
        stub.assert_consumed();
    }
}
