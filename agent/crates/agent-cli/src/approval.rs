use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use async_trait::async_trait;
use std::io::Write;
use std::time::Duration;

/// Default interactive approval window for the terminal front-end. The CLI keeps
/// a fixed 300s auto-deny in 4B-1 (parks-and-exits is 4B-2); the *server* channel,
/// by contrast, now parks indefinitely unless its `approval_auto_deny_secs` knob
/// is set, so the two front-ends no longer share one constant.
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
