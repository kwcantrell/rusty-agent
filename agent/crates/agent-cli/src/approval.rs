use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use async_trait::async_trait;
use std::io::Write;
use std::time::Duration;

/// Default interactive approval window. Matches the server's `APPROVAL_TIMEOUT`
/// (`agent-server/src/session.rs`) so both front-ends share the same human-friendly
/// timeout before auto-denying.
const DEFAULT_TERMINAL_APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

type BlockingPrompt = std::sync::Arc<dyn Fn(String) -> ApprovalResponse + Send + Sync>;

pub struct TerminalApproval {
    timeout: Duration,
    /// Serializes concurrent requesters (parallel sub-agents both hitting Ask)
    /// so prompts never interleave on stdin.
    gate: tokio::sync::Mutex<()>,
    prompt: BlockingPrompt,
}

fn stdin_prompt(summary: String) -> ApprovalResponse {
    print!("\n\x1b[35mAllow:\x1b[0m {summary} ? [y]es / [n]o / [a]lways: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return ApprovalResponse::Deny;
    }
    match line.trim().to_lowercase().as_str() {
        "y" | "yes" => ApprovalResponse::Approve,
        "a" | "always" => ApprovalResponse::ApproveAlways,
        _ => ApprovalResponse::Deny,
    }
}

impl TerminalApproval {
    #[allow(dead_code)]
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            gate: tokio::sync::Mutex::new(()),
            prompt: std::sync::Arc::new(stdin_prompt),
        }
    }
    #[cfg(test)]
    fn with_prompt(timeout: Duration, prompt: BlockingPrompt) -> Self {
        Self {
            timeout,
            gate: tokio::sync::Mutex::new(()),
            prompt,
        }
    }
}

impl Default for TerminalApproval {
    fn default() -> Self {
        Self::new(DEFAULT_TERMINAL_APPROVAL_TIMEOUT)
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
        // blocked. The approval loop prompts one at a time, so this does not accumulate
        // in practice. A clean cancel would need raw-fd polling, not worth the
        // complexity for a CLI approval prompt.
        let _serialized = self.gate.lock().await;
        let summary = req.intent.summary.clone();
        let prompt = self.prompt.clone();
        let handle = tokio::task::spawn_blocking(move || prompt(summary));
        match tokio::time::timeout(self.timeout, handle).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(_join_err)) => ApprovalResponse::Deny,
            Err(_elapsed) => {
                eprintln!("\nApproval timed out; denying.");
                ApprovalResponse::Deny
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
        }
    }

    #[tokio::test]
    async fn denies_when_timeout_elapses() {
        // A ~1ms timeout returns Deny promptly. Which arm fires is env-dependent: with a
        // blocking terminal stdin the read parks and the `Err(_elapsed)` timeout arm fires;
        // under a closed/EOF stdin (e.g. `cargo test` in CI) `read_line` returns Ok(0) and
        // the `_ => Deny` arm fires first. Both resolve to Deny — which is all we assert.
        let ch = TerminalApproval::new(Duration::from_millis(1));
        let resp = ch.request(req()).await;
        assert!(matches!(resp, ApprovalResponse::Deny));
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
}
