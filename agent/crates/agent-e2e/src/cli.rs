//! CLI subprocess driver: spawns the real `agent` binary (agent-cli crate)
//! against a `Rig`-isolated workspace/sessions/metadata root, and drives it
//! over stdin/stdout/stderr like a human would.
use crate::rig::Rig;
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

pub struct CliCmd {
    pub(crate) cmd: Command,
}

impl CliCmd {
    pub fn new(rig: &Rig, base_url: &str) -> Self {
        let mut cmd = Command::new(agent_bin());
        cmd.args([
            "--base-url",
            base_url,
            "--model",
            "stub-model",
            "--workspace",
            rig.workspace.path().to_str().unwrap(),
            "--trace-dir",
            rig.sessions.path().to_str().unwrap(),
            "--metadata-dir",
            rig.meta.path().to_str().unwrap(),
            "--stream-timeout-secs",
            "10",
            "--approval-timeout-secs",
            "20",
        ]);
        cmd.env("HOME", rig.meta.path()) // belt-and-braces (spec §2.3)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0); // own group → group-kill can't touch the test
        CliCmd { cmd }
    }

    /// Subcommand form, e.g. `sessions_sub(&["sessions","reopen","<id>"])` —
    /// clap subcommands must come AFTER top-level flags (`Cli`'s `command`
    /// field is parsed alongside the flags; agent-cli's own doc comment on
    /// `Command::Sessions` says as much: "Top-level flags (--base-url,
    /// --workspace, etc.) go BEFORE the subcommand"). `CliCmd::new` already
    /// puts flags first, so `sessions_sub` only needs to append.
    pub fn sessions_sub(mut self, sub: &[&str]) -> Self {
        self.cmd.args(sub);
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
        let transcript = cli.wait_for_output(REPL_MARKER, Duration::from_secs(3));
        assert!(
            transcript.contains(REPL_MARKER),
            "transcript so far:\n{transcript}"
        );
        cli.close_stdin();
    }
}
