use agent_core::checkpoint::ParkedAnswer;
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use async_trait::async_trait;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Default interactive approval window for the terminal front-end. On timeout the
/// CLI parks-and-exits when a durable-park capability is wired (`ParkExit`, 4B-2)
/// and denies otherwise (no `Checkpointer`, e.g. tests); the *server* channel, by
/// contrast, parks indefinitely unless its `approval_auto_deny_secs` knob is set,
/// so the two front-ends still don't share one constant.
const DEFAULT_TERMINAL_APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

type BlockingPrompt = std::sync::Arc<dyn Fn(String) -> ApprovalResponse + Send + Sync>;

/// Durable-park capability (4B-2): injected only when a `Checkpointer` was
/// built, so the CLI's timeout arm can park-and-exit instead of denying.
pub struct ParkExit {
    pub session_id: String,
    /// Flushed before exit — process::exit skips Drop, and buffered trace
    /// tail-loss on every park-exit would be a real audit gap.
    pub trace: Option<Arc<agent_runtime_config::TraceWriter>>,
    /// Set ONLY by the reopen driver (Task 10): the checkpoint dir whose
    /// resume.lock must be released before exit — process::exit skips
    /// Drop, and a leaked lock would refuse the NEXT reopen.
    pub release_lock: Option<PathBuf>,
    /// Test seam; production = Box::new(|code| std::process::exit(code)).
    pub exit: Box<dyn Fn(i32) + Send + Sync>,
}

pub struct TerminalApproval {
    timeout: Duration,
    /// Serializes concurrent requesters (parallel sub-agents both hitting Ask)
    /// so prompts never interleave on stdin.
    gate: tokio::sync::Mutex<()>,
    prompt: BlockingPrompt,
    park_exit: Option<ParkExit>,
}

/// The operator-facing hint printed on a durable park-and-exit — factored out
/// so the message content is assertable without capturing process stderr.
fn park_exit_message(timeout: Duration, session_id: &str) -> String {
    format!(
        "\nApproval unanswered for {}s — run parked; answer later with:\n  agent sessions reopen {session_id}",
        timeout.as_secs()
    )
}

fn stdin_prompt(summary: String) -> ApprovalResponse {
    print!("\n\x1b[35mAllow:\x1b[0m {summary} ? [y]es / [n]o / [a]lways: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return ApprovalResponse::Deny { feedback: None };
    }
    match line.trim().to_lowercase().as_str() {
        "y" | "yes" => ApprovalResponse::Approve,
        "a" | "always" => ApprovalResponse::ApproveAlways,
        _ => ApprovalResponse::Deny { feedback: None },
    }
}

impl TerminalApproval {
    #[allow(dead_code)]
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            gate: tokio::sync::Mutex::new(()),
            prompt: std::sync::Arc::new(stdin_prompt),
            park_exit: None,
        }
    }

    /// Production constructor: wires the durable-park capability (4B-2).
    /// `park_exit: None` degrades to timeout-denies. `timeout` is the interactive
    /// approval window (E2 knob; CLI default is DEFAULT_TERMINAL_APPROVAL_TIMEOUT).
    pub fn with_park_exit(park_exit: Option<ParkExit>, timeout: Duration) -> Self {
        Self {
            park_exit,
            ..Self::new(timeout)
        }
    }

    #[cfg(test)]
    fn with_prompt(timeout: Duration, prompt: BlockingPrompt) -> Self {
        Self {
            timeout,
            gate: tokio::sync::Mutex::new(()),
            prompt,
            park_exit: None,
        }
    }

    #[cfg(test)]
    fn with_park_exit_for_test(mut self, park_exit: ParkExit) -> Self {
        self.park_exit = Some(park_exit);
        self
    }
}

impl Default for TerminalApproval {
    fn default() -> Self {
        Self::new(DEFAULT_TERMINAL_APPROVAL_TIMEOUT)
    }
}

/// Reopen-path prompt: y/n/always, then an optional feedback line on deny.
pub fn prompt_for_answer_with_reader<R: std::io::BufRead>(
    summary: &str,
    who: &str,
    mut input: R,
) -> ParkedAnswer {
    print!("\n\x1b[35mAllow:\x1b[0m {who}{summary} ? [y]es / [n]o / [a]lways: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if input.read_line(&mut line).is_err() {
        return ParkedAnswer {
            approve: false,
            feedback: None,
        };
    }
    match line.trim().to_lowercase().as_str() {
        "y" | "yes" | "a" | "always" => ParkedAnswer {
            approve: true,
            feedback: None,
        }, // E2: always = plain approve
        _ => {
            print!("Feedback for the agent (optional, Enter to skip): ");
            let _ = std::io::stdout().flush();
            let mut fb = String::new();
            let _ = input.read_line(&mut fb);
            let fb = fb.trim();
            ParkedAnswer {
                approve: false,
                feedback: (!fb.is_empty()).then(|| fb.to_string()),
            }
        }
    }
}

#[async_trait]
impl ApprovalChannel for TerminalApproval {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
        // Run the blocking stdin read off the async runtime, bounded by a timeout.
        //
        // NOTE: std's blocking `read_line` cannot be cancelled, so on timeout the
        // spawned thread is orphaned — it stays parked on stdin until the next line or
        // EOF arrives, then its result is discarded. Harmless: one idle thread per
        // elapsed prompt (bounded by tokio's blocking pool), and the agent is no longer
        // blocked. The `gate` mutex below now enforces one-at-a-time for LIVE prompts
        // (parallel sub-agents can each hit Ask — spec D12), so at most one prompt reads
        // stdin at a time. But a timed-out orphan thread still holds its own `read_line`
        // on stdin, so it can race the next prompt's read for the operator's keystrokes
        // — an accepted residual: a clean cancel would need raw-fd polling, not worth
        // the complexity for a CLI approval prompt.
        let _serialized = self.gate.lock().await;
        // Attribute the prompt to its originating sub-agent (if any) so a
        // parallel-dispatch operator can tell whose Ask they are answering.
        let who = match &req.origin {
            Some(o) => format!("[sub-agent {} (depth {})] ", o.subagent_name, o.depth),
            None => String::new(),
        };
        let summary = format!("{who}{}", req.intent.summary);
        let prompt = self.prompt.clone();
        let handle = tokio::task::spawn_blocking(move || prompt(summary));
        match tokio::time::timeout(self.timeout, handle).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(_join_err)) => ApprovalResponse::Deny { feedback: None },
            Err(_elapsed) => {
                if let Some(park) = &self.park_exit {
                    eprintln!("{}", park_exit_message(self.timeout, &park.session_id));
                    if let Some(t) = &park.trace {
                        t.flush();
                    }
                    if let Some(dir) = &park.release_lock {
                        agent_core::checkpoint::release_resume(dir);
                    }
                    (park.exit)(0);
                }
                eprintln!("\nApproval timed out; denying.");
                ApprovalResponse::Deny { feedback: None }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_policy::{ApprovalRequest, ApprovalResponse};
    use agent_tools::{Access, ToolIntent};
    use std::time::Duration;

    fn req() -> ApprovalRequest {
        ApprovalRequest {
            intent: ToolIntent {
                tool: "bash".into(),
                access: Access::Write,
                paths: vec![],
                command: Some("echo hi".into()),
                summary: "run echo".into(),
            },
            display: None,
            origin: None,
        }
    }

    #[tokio::test]
    async fn denies_when_timeout_elapses() {
        // Exercise the timeout arm hermetically via `with_prompt` — the same injection
        // seam `concurrent_requests_serialize` uses — so the test never touches process
        // stdin. A prompt fn that sleeps well past the 1ms timeout guarantees the
        // `Err(_elapsed)` arm fires and resolves to Deny. (The old test called `new()`,
        // parking a spawn_blocking thread on REAL stdin; under an open, never-EOF stdin
        // tokio's runtime teardown joined that orphan and wedged the binary for ~874s.)
        let ch = TerminalApproval::with_prompt(
            Duration::from_millis(1),
            std::sync::Arc::new(|_summary: String| {
                std::thread::sleep(Duration::from_millis(500));
                ApprovalResponse::Approve
            }),
        );
        let resp = ch.request(req()).await;
        assert!(matches!(resp, ApprovalResponse::Deny { .. }));
    }

    #[tokio::test]
    async fn timeout_with_durable_park_prints_hint_and_exits() {
        // No `new_for_test`: build the rig via `with_prompt` (1ms timeout, a
        // prompt that sleeps 500ms so the timeout arm fires) plus the
        // `with_park_exit_for_test` setter this task adds. The exit hook is
        // injected for tests — production installs std::process::exit.
        let captured: Arc<std::sync::Mutex<Option<i32>>> = Arc::new(std::sync::Mutex::new(None));
        let c2 = captured.clone();
        let ch = TerminalApproval::with_prompt(
            Duration::from_millis(1),
            std::sync::Arc::new(|_summary: String| {
                std::thread::sleep(Duration::from_millis(500));
                ApprovalResponse::Approve
            }),
        )
        .with_park_exit_for_test(ParkExit {
            session_id: "100-aaaaaaaa".into(),
            trace: None,
            release_lock: None,
            exit: Box::new(move |code| {
                *c2.lock().unwrap() = Some(code);
            }),
        });
        let _resp = ch.request(req()).await;
        let code = captured.lock().unwrap().expect("exit hook must have fired");
        assert_eq!(code, 0);
        let msg = park_exit_message(Duration::from_millis(1), "100-aaaaaaaa");
        assert!(msg.contains("run parked"), "message: {msg}");
        assert!(
            msg.contains("agent sessions reopen 100-aaaaaaaa"),
            "message: {msg}"
        );
    }

    #[tokio::test]
    async fn timeout_without_durable_wiring_keeps_denying() {
        // park_exit: None (no checkpointer was built) -> today's behavior:
        // returns ApprovalResponse::Deny { feedback: None }.
        let ch = TerminalApproval::with_prompt(
            Duration::from_millis(1),
            std::sync::Arc::new(|_summary: String| {
                std::thread::sleep(Duration::from_millis(500));
                ApprovalResponse::Approve
            }),
        );
        let resp = ch.request(req()).await;
        assert!(matches!(resp, ApprovalResponse::Deny { feedback: None }));
    }

    #[test]
    fn deny_prompt_collects_optional_feedback() {
        let with_feedback = prompt_for_answer_with_reader(
            "run echo",
            "",
            std::io::Cursor::new(&b"n\nuse staging\n"[..]),
        );
        assert_eq!(
            with_feedback,
            ParkedAnswer {
                approve: false,
                feedback: Some("use staging".into())
            }
        );

        let no_feedback =
            prompt_for_answer_with_reader("run echo", "", std::io::Cursor::new(&b"n\n\n"[..]));
        assert_eq!(
            no_feedback,
            ParkedAnswer {
                approve: false,
                feedback: None
            }
        );

        let approved =
            prompt_for_answer_with_reader("run echo", "", std::io::Cursor::new(&b"y\n"[..]));
        assert_eq!(
            approved,
            ParkedAnswer {
                approve: true,
                feedback: None
            }
        );
    }

    #[tokio::test]
    async fn concurrent_requests_serialize() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let live = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        let (l, m) = (live.clone(), max.clone());
        let ch = Arc::new(TerminalApproval::with_prompt(
            Duration::from_secs(5),
            Arc::new(move |_summary: String| {
                let now = l.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(20));
                l.fetch_sub(1, Ordering::SeqCst);
                ApprovalResponse::Approve
            }),
        ));
        let (a, b) = tokio::join!(ch.request(req()), ch.request(req()));
        assert!(matches!(a, ApprovalResponse::Approve));
        assert!(matches!(b, ApprovalResponse::Approve));
        // Two children prompting at once must never overlap on stdin (spec D12).
        assert_eq!(max.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn with_park_exit_honors_custom_timeout() {
        // Same-module test may read the private field — no stdin involvement
        // (plan-review F6: the public request() path would park an orphan
        // blocking read_line on real process stdin).
        let ch = TerminalApproval::with_park_exit(None, Duration::from_secs(7));
        assert_eq!(ch.timeout, Duration::from_secs(7));
        let d = TerminalApproval::with_park_exit(None, DEFAULT_TERMINAL_APPROVAL_TIMEOUT);
        assert_eq!(d.timeout, Duration::from_secs(300));
    }
}
