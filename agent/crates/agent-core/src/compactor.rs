use crate::context::message_tokens;
use agent_model::{Chunk, CompletionRequest, Message, ModelClient, Role};
use futures::StreamExt;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub enum CompactError {
    Model(String),
    Cancelled,
}

impl std::fmt::Display for CompactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompactError::Model(m) => write!(f, "compaction model error: {m}"),
            CompactError::Cancelled => write!(f, "compaction cancelled"),
        }
    }
}

const COMPACTION_SYSTEM: &str = "You are a context compaction engine. You are given the prior \
RUNNING SUMMARY (may be empty) and the NEW conversation turns since it. Output an updated \
running summary that strictly contains all information from the prior summary PLUS the new \
turns. Rules: (1) Carry forward every fact, decision, unresolved problem, and file/identifier \
name from the prior summary — and especially every number, count, and running total. Never \
drop or paraphrase them away; a later summary must never lose information an earlier one held. \
(2) Preserve enumerated or step-wise items individually (e.g. 'step 1 adds 5; step 2 adds 12'); \
never collapse them into a vague phrase like 'several steps'. (3) Drop only redundant tool \
output and chatter. (4) Output ONLY the updated summary text — do NOT repeat these \
instructions or the section labels. Be terse.";

const PRIOR_PREFIX: &str = "Summary of earlier conversation:\n";

fn render_span(span: &[Message], prior: Option<&Message>) -> String {
    let mut s = String::new();
    match prior {
        Some(p) => {
            let body = p.content.strip_prefix(PRIOR_PREFIX).unwrap_or(&p.content);
            s.push_str("PRIOR RUNNING SUMMARY:\n");
            s.push_str(body);
            s.push_str("\n\n");
        }
        None => s.push_str("PRIOR RUNNING SUMMARY: (none)\n\n"),
    }
    s.push_str("NEW TURNS:\n");
    for m in span {
        let role = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        s.push_str(&format!("[{role}] {}\n", m.content));
    }
    s
}

/// Drive one completion to a single collected string.
async fn collect_stream(
    model: &Arc<dyn ModelClient>,
    req: CompletionRequest,
    cancel: &CancellationToken,
) -> Result<String, CompactError> {
    let mut stream = tokio::select! {
        _ = cancel.cancelled() => return Err(CompactError::Cancelled),
        opened = model.stream(req) => opened.map_err(|e| CompactError::Model(e.to_string()))?,
    };
    let mut text = String::new();
    loop {
        let step = tokio::select! {
            _ = cancel.cancelled() => return Err(CompactError::Cancelled),
            s = stream.next() => s,
        };
        match step {
            None => break,
            Some(item) => match item.map_err(|e| CompactError::Model(e.to_string()))? {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Done(_) => break,
                _ => {}
            },
        }
    }
    Ok(text)
}

/// Summarize `span` into a single high-fidelity system message. Read-only: the
/// caller decides whether to commit the result.
pub async fn run_compaction(
    span: &[Message],
    prior: Option<&Message>,
    model: &Arc<dyn ModelClient>,
    cancel: &CancellationToken,
) -> Result<Message, CompactError> {
    let req = CompletionRequest {
        messages: vec![
            Message::system(COMPACTION_SYSTEM),
            Message::user(render_span(span, prior)),
        ],
        temperature: 0.0,
        ..Default::default()
    };
    let summary = collect_stream(model, req, cancel).await?;
    let body = format!("{PRIOR_PREFIX}{}", summary.trim());
    Ok(Message::system(body))
}

/// True when `summary` is a net token win over `span` (and non-empty).
pub(crate) fn compaction_is_worthwhile(summary: &Message, span: &[Message]) -> bool {
    let summary_body = summary
        .content
        .strip_prefix("Summary of earlier conversation:\n")
        .unwrap_or(&summary.content);
    if summary_body.trim().is_empty() {
        return false;
    }
    let span_tokens: usize = span.iter().map(message_tokens).sum();
    message_tokens(summary) < span_tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{Scripted, ScriptedModel};

    #[tokio::test]
    async fn run_compaction_returns_summary_message() {
        let span = vec![Message::user("a".repeat(50)), Message::assistant("b".repeat(50), None)];
        let model: Arc<dyn ModelClient> =
            Arc::new(ScriptedModel::new(vec![Scripted::Text("decided X; bug Y open".into())]));
        let cancel = CancellationToken::new();
        let msg = run_compaction(&span, None, &model, &cancel).await.unwrap();
        assert!(matches!(msg.role, Role::System));
        assert!(msg.content.contains("decided X; bug Y open"));
    }

    #[tokio::test]
    async fn worthwhile_rejects_empty_or_larger_summary() {
        let span = vec![Message::user("tiny")];
        let empty = Message::system("Summary of earlier conversation:\n   ");
        assert!(!compaction_is_worthwhile(&empty, &span));
        let huge = Message::system(format!("Summary of earlier conversation:\n{}", "x".repeat(9999)));
        assert!(!compaction_is_worthwhile(&huge, &span));
    }
}
