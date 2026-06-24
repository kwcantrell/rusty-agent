use agent_model::Message;

/// Cheap, tokenizer-agnostic estimate (~4 chars/token). Swap for a real
/// tokenizer later behind the same call site.
pub fn estimate_tokens(s: &str) -> usize {
    (s.chars().count() / 4).max(1)
}

fn message_tokens(m: &Message) -> usize {
    estimate_tokens(&m.content) + 4 // per-message overhead
}

/// Total estimated tokens for a built context (system + kept history),
/// using the same per-message estimate the window manager evicts against.
pub fn built_tokens(messages: &[Message]) -> usize {
    messages.iter().map(message_tokens).sum()
}

pub trait ContextManager: Send + Sync {
    fn append(&mut self, msg: Message);
    fn build(&self, model_limit: usize) -> Vec<Message>;
    fn set_system(&mut self, system: Message);
}

/// Sliding-window context: always keeps the system prompt; evicts oldest
/// history turns until the estimate fits `model_limit`.
pub struct WindowContext {
    system: Message,
    history: Vec<Message>,
}

impl WindowContext {
    pub fn new(system: Message) -> Self {
        Self { system, history: Vec::new() }
    }
}

impl ContextManager for WindowContext {
    fn append(&mut self, msg: Message) {
        self.history.push(msg);
    }

    fn set_system(&mut self, system: Message) {
        self.system = system;
    }

    fn build(&self, model_limit: usize) -> Vec<Message> {
        let sys_tokens = message_tokens(&self.system);
        let budget = model_limit.saturating_sub(sys_tokens);
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
        let mut out = Vec::with_capacity(kept_rev.len() + 1);
        out.push(self.system.clone());
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
