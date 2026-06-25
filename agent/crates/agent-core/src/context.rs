use agent_model::Message;

/// Cheap, tokenizer-agnostic estimate (~4 chars/token). Swap for a real
/// tokenizer later behind the same call site.
pub fn estimate_tokens(s: &str) -> usize {
    (s.chars().count() / 4).max(1)
}

fn message_tokens(m: &Message) -> usize {
    let mut t = estimate_tokens(&m.content) + 4; // per-message overhead
    if let Some(r) = &m.reasoning {
        t += estimate_tokens(r);
    }
    if let Some(calls) = &m.tool_calls {
        for c in calls {
            t += estimate_tokens(&c.name) + estimate_tokens(&c.args.to_string());
        }
    }
    t
}

/// Total estimated tokens for a built context (system + kept history),
/// using the same per-message estimate the window manager evicts against.
pub fn built_tokens(messages: &[Message]) -> usize {
    messages.iter().map(message_tokens).sum()
}

/// Default cap on the auto-retrieval recall block, in estimated tokens. Keeps
/// recall from crowding out conversation history. Override per-context with
/// `WindowContext::with_recall_budget`.
pub const DEFAULT_RECALL_TOKEN_BUDGET: usize = 512;

pub trait ContextManager: Send + Sync {
    fn append(&mut self, msg: Message);
    fn build(&self, model_limit: usize) -> Vec<Message>;
    fn set_system(&mut self, system: Message);
    /// Replace the auto-retrieved recall lines surfaced this turn. Default no-op
    /// so non-memory implementations are unaffected.
    fn set_recall(&mut self, _items: Vec<String>) {}
}

/// Sliding-window context: always keeps the system prompt; evicts oldest
/// history turns until the estimate fits `model_limit`. An optional recall block
/// (auto-retrieved memories) sits right after the system prompt, capped at
/// `recall_budget` tokens so it can never starve history.
pub struct WindowContext {
    system: Message,
    history: Vec<Message>,
    recall: Vec<String>,
    recall_budget: usize,
}

impl WindowContext {
    pub fn new(system: Message) -> Self {
        Self {
            system,
            history: Vec::new(),
            recall: Vec::new(),
            recall_budget: DEFAULT_RECALL_TOKEN_BUDGET,
        }
    }

    /// Override the recall-block token cap (default `DEFAULT_RECALL_TOKEN_BUDGET`).
    pub fn with_recall_budget(mut self, budget: usize) -> Self {
        self.recall_budget = budget;
        self
    }

    /// Build the recall block message, greedily keeping lines under `recall_budget`.
    /// Always includes at least the first line if any are present (soft cap).
    fn recall_message(&self) -> Option<Message> {
        if self.recall.is_empty() {
            return None;
        }
        const HEADER: &str = "Relevant memories from past sessions:";
        let mut body = String::from(HEADER);
        for line in &self.recall {
            let candidate = format!("{body}\n- {line}");
            if estimate_tokens(&candidate) > self.recall_budget && body != HEADER {
                break;
            }
            body = candidate;
        }
        Some(Message::system(body))
    }
}

impl ContextManager for WindowContext {
    fn append(&mut self, msg: Message) {
        self.history.push(msg);
    }

    fn set_system(&mut self, system: Message) {
        self.system = system;
    }

    fn set_recall(&mut self, items: Vec<String>) {
        self.recall = items;
    }

    fn build(&self, model_limit: usize) -> Vec<Message> {
        let sys_tokens = message_tokens(&self.system);
        let recall_msg = self.recall_message();
        let recall_tokens = recall_msg.as_ref().map(message_tokens).unwrap_or(0);
        let budget = model_limit
            .saturating_sub(sys_tokens)
            .saturating_sub(recall_tokens);
        // Walk history newest-first, keep while it fits.
        let mut kept_rev: Vec<Message> = Vec::new();
        let mut used = 0usize;
        for m in self.history.iter().rev() {
            let t = message_tokens(m);
            if used + t > budget && !kept_rev.is_empty() {
                break;
            }
            used += t;
            kept_rev.push(m.clone());
        }
        kept_rev.reverse();
        let mut out = Vec::with_capacity(kept_rev.len() + 2);
        out.push(self.system.clone());
        if let Some(m) = recall_msg {
            out.push(m);
        }
        out.extend(kept_rev);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_model::{Message, Role};

    #[test]
    fn built_tokens_sums_per_message_estimate() {
        let msgs = vec![Message::system("SYS"), Message::user("hello world")];
        let expected = message_tokens(&msgs[0]) + message_tokens(&msgs[1]);
        assert_eq!(built_tokens(&msgs), expected);
    }

    #[test]
    fn message_tokens_counts_tool_calls_and_reasoning() {
        let plain = Message::assistant("hi", None);
        let heavy = Message::assistant(
            "hi",
            Some(vec![agent_tools::ToolCall {
                id: "c1".into(),
                name: "read_file".into(),
                args: serde_json::json!({"path": "some/long/path/to/a/file/name.txt"}),
            }]),
        )
        .with_reasoning("a fairly long chain of reasoning that should add tokens");
        assert!(
            message_tokens(&heavy) > message_tokens(&plain),
            "tool_calls + reasoning must increase the estimate"
        );
    }

    #[test]
    fn estimate_tokens_is_roughly_quarter_of_chars() {
        assert!(estimate_tokens("abcd") >= 1);
        assert!(estimate_tokens(&"x".repeat(400)) >= 90);
    }

    #[test]
    fn build_keeps_system_and_drops_oldest_when_over_limit() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        for i in 0..50 {
            ctx.append(Message::user(format!("message number {i} with some padding text")));
        }
        // Tiny limit forces eviction.
        let built = ctx.build(40);
        assert!(matches!(built[0].role, Role::System)); // system always first
        assert!(built.len() < 51);                       // some history evicted
        // The most recent user message survives.
        let last = built.last().unwrap();
        assert!(last.content.contains("49"));
    }

    #[test]
    fn build_returns_all_when_under_limit() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("hello"));
        let built = ctx.build(100_000);
        assert_eq!(built.len(), 2);
    }

    #[test]
    fn set_recall_injects_block_after_system_before_history() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("hello"));
        ctx.set_recall(vec!["user likes rust".into(), "project uses tokio".into()]);
        let built = ctx.build(100_000);
        assert!(matches!(built[0].role, Role::System)); // system first
        assert_eq!(built[1].content,
            "Relevant memories from past sessions:\n- user likes rust\n- project uses tokio");
        assert!(matches!(built[1].role, Role::System));  // recall block is system-role
        assert_eq!(built.last().unwrap().content, "hello"); // history after recall
    }

    #[test]
    fn empty_recall_injects_no_block() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.append(Message::user("hello"));
        ctx.set_recall(vec![]);
        let built = ctx.build(100_000);
        assert_eq!(built.len(), 2); // system + history only
    }

    #[test]
    fn set_recall_replaces_previous_lines() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        ctx.set_recall(vec!["old".into()]);
        ctx.set_recall(vec!["new".into()]);
        let built = ctx.build(100_000);
        assert!(built[1].content.contains("new"));
        assert!(!built[1].content.contains("old"));
    }

    #[test]
    fn recall_block_is_capped_by_budget() {
        // 30 long lines vastly exceed a 64-token budget; the block must stay under it
        // (plus the soft floor of one line) — never inject all 30.
        let mut ctx = WindowContext::new(Message::system("SYS")).with_recall_budget(64);
        let lines: Vec<String> = (0..30)
            .map(|i| format!("memory fact number {i} with a fair amount of padding text"))
            .collect();
        ctx.set_recall(lines);
        let built = ctx.build(100_000);
        let block = &built[1].content;
        // Far fewer than 30 lines survived.
        assert!(block.matches("\n- ").count() < 30);
        assert!(block.starts_with("Relevant memories from past sessions:"));
    }

    #[test]
    fn history_is_evicted_before_recall_and_system() {
        let mut ctx = WindowContext::new(Message::system("SYS"));
        for i in 0..50 {
            ctx.append(Message::user(format!("message number {i} with some padding text")));
        }
        ctx.set_recall(vec!["pinned memory".into()]);
        let built = ctx.build(40); // tiny limit forces history eviction
        assert!(matches!(built[0].role, Role::System));
        assert!(built[1].content.contains("pinned memory")); // recall survives
        assert!(built.len() < 51);                            // history evicted
    }

    #[test]
    fn set_system_replaces_prompt_and_keeps_history() {
        let mut ctx = WindowContext::new(Message::system("OLD"));
        ctx.append(Message::user("u1"));
        ctx.set_system(Message::system("NEW"));
        let built = ctx.build(100_000);
        assert!(matches!(built[0].role, Role::System)); // system still first
        assert_eq!(built[0].content, "NEW");            // and replaced
        assert!(built.iter().any(|m| m.content == "u1")); // history intact
    }
}
