use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use async_trait::async_trait;
use std::io::Write;

#[allow(dead_code)]
pub struct TerminalApproval;

#[async_trait]
impl ApprovalChannel for TerminalApproval {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse {
        // Run the blocking stdin read off the async runtime.
        let summary = req.intent.summary.clone();
        tokio::task::spawn_blocking(move || {
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
        })
        .await
        .unwrap_or(ApprovalResponse::Deny)
    }
}
