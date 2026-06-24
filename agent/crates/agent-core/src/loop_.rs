use crate::{AgentEvent, ContextManager, EventSink};
use agent_model::{AssistantTurn, Chunk, CompletionRequest, Message, ModelClient, ModelError,
                  RawToolCall, StopReason, ToolCallProtocol};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse, Decision, PolicyEngine};
use agent_tools::{ToolCall, ToolCtx, ToolError, ToolRegistry};
use futures::StreamExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("model error after retries: {0}")]
    Model(String),
}

/// Default idle timeout for model-stream consumption. Generous enough to cover
/// claude-cli cold-start + `thinking` blocks before the first token.
pub const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct LoopConfig {
    pub model_limit: usize,
    pub max_turns: usize,
    pub max_retries: usize,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub workspace: PathBuf,
    pub tool_timeout: Duration,
    /// Max time with no stream progress (stream-open or inter-chunk) before a turn
    /// is treated as a stalled-backend `ModelError::Timeout`.
    pub stream_idle_timeout: Duration,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub min_p: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub repeat_penalty: Option<f32>,
    pub enable_thinking: bool,
    pub preserve_thinking: bool,
}

pub struct AgentLoop {
    model: Arc<dyn ModelClient>,
    protocol: Arc<dyn ToolCallProtocol>,
    tools: Arc<ToolRegistry>,
    policy: Arc<dyn PolicyEngine>,
    approval: Arc<dyn ApprovalChannel>,
    sink: Arc<dyn EventSink>,
    config: LoopConfig,
}

impl AgentLoop {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: Arc<dyn ModelClient>,
        protocol: Arc<dyn ToolCallProtocol>,
        tools: Arc<ToolRegistry>,
        policy: Arc<dyn PolicyEngine>,
        approval: Arc<dyn ApprovalChannel>,
        sink: Arc<dyn EventSink>,
        config: LoopConfig,
    ) -> Self {
        Self { model, protocol, tools, policy, approval, sink, config }
    }

    /// Drive one streamed completion to an `AssistantTurn`, emitting tokens as they arrive.
    async fn one_completion(&self, req: CompletionRequest) -> Result<AssistantTurn, ModelError> {
        let idle = self.config.stream_idle_timeout;
        let mut stream = match tokio::time::timeout(idle, self.model.stream(req)).await {
            Err(_) => return Err(ModelError::Timeout(idle)),
            Ok(opened) => opened?,
        };
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut raw_tool_calls: Vec<RawToolCall> = Vec::new();
        let mut stop = StopReason::Stop;
        loop {
            match tokio::time::timeout(idle, stream.next()).await {
                // Stalled: dropping `stream` on return fires kill_on_drop / tears down the connection.
                Err(_) => return Err(ModelError::Timeout(idle)),
                Ok(None) => break,
                Ok(Some(item)) => match item? {
                    Chunk::Text(t) => { self.sink.emit(AgentEvent::Token(t.clone())); text.push_str(&t); }
                    Chunk::Reasoning(r) => { self.sink.emit(AgentEvent::Reasoning(r.clone())); reasoning.push_str(&r); }
                    Chunk::ToolCallDelta(rc) => merge_tool_call(&mut raw_tool_calls, rc),
                    Chunk::Done(r) => stop = r,
                },
            }
        }
        Ok(AssistantTurn { text, raw_tool_calls, stop, reasoning })
    }

    /// Stream with retry/backoff on transport errors.
    async fn completion_with_retry(&self, base: &CompletionRequest)
        -> Result<AssistantTurn, AgentError> {
        let mut attempt = 0;
        loop {
            let mut req = base.clone();
            self.protocol.prepare(&mut req);
            match self.one_completion(req).await {
                Ok(turn) => return Ok(turn),
                Err(e) => {
                    attempt += 1;
                    if attempt > self.config.max_retries {
                        self.sink.emit(AgentEvent::Error(e.to_string()));
                        return Err(AgentError::Model(e.to_string()));
                    }
                    tracing::warn!(attempt, error = %e, "model error, retrying");
                    tokio::time::sleep(Duration::from_millis(100 * attempt as u64)).await;
                }
            }
        }
    }

    pub async fn run(&self, ctx: &mut dyn ContextManager, user_input: String)
        -> Result<(), AgentError> {
        ctx.append(Message::user(user_input));
        let mut protocol_repairs = 0;

        for _turn in 0..self.config.max_turns {
            let base = CompletionRequest {
                messages: ctx.build(self.config.model_limit),
                tools: self.tools.schemas(),
                temperature: self.config.temperature,
                max_tokens: self.config.max_tokens,
                top_p: self.config.top_p,
                top_k: self.config.top_k,
                min_p: self.config.min_p,
                presence_penalty: self.config.presence_penalty,
                repeat_penalty: self.config.repeat_penalty,
                enable_thinking: self.config.enable_thinking,
            };
            let assistant = self.completion_with_retry(&base).await?;

            let parsed = match self.protocol.parse(&assistant) {
                Ok(p) => { protocol_repairs = 0; p }
                Err(e) if protocol_repairs < 1 => {
                    protocol_repairs += 1;
                    ctx.append(Message::assistant(assistant.text.clone(), None));
                    ctx.append(Message::user(format!(
                        "Your tool call could not be parsed: {e}. Re-emit it correctly.")));
                    continue;
                }
                Err(e) => {
                    self.sink.emit(AgentEvent::Error(e.to_string()));
                    return Ok(());
                }
            };

            let stored = if self.config.preserve_thinking && !assistant.reasoning.is_empty() {
                format!("<think>{}</think>\n{}", assistant.reasoning, parsed.text)
            } else {
                parsed.text.clone()
            };
            ctx.append(Message::assistant(stored,
                if parsed.tool_calls.is_empty() { None } else { Some(parsed.tool_calls.clone()) }));

            if parsed.tool_calls.is_empty() {
                self.sink.emit(AgentEvent::Done(assistant.stop));
                return Ok(());
            }

            for call in parsed.tool_calls {
                let result = self.run_tool(call.clone()).await;
                let content = match result {
                    Ok(output) => {
                        self.sink.emit(AgentEvent::ToolResult {
                            name: call.name.clone(), output: output.clone() });
                        output.content
                    }
                    Err(e) => format!("ERROR: {e}"),
                };
                ctx.append(Message::tool(call.id, call.name, content));
            }
        }
        self.sink.emit(AgentEvent::Done(StopReason::BudgetExhausted));
        Ok(())
    }

    async fn run_tool(&self, call: ToolCall) -> Result<agent_tools::ToolOutput, ToolError> {
        self.sink.emit(AgentEvent::ToolStart { name: call.name.clone(), args: call.args.clone() });
        let tool = self.tools.get(&call.name)
            .ok_or_else(|| ToolError::NotFound(format!("unknown tool {}", call.name)))?;
        let intent = tool.intent(&call.args)?;
        let allowed = match self.policy.check(&intent) {
            Decision::Allow => true,
            Decision::Deny(reason) => return Err(ToolError::Denied(reason)),
            Decision::Ask => {
                // diff preview is produced by execute(); the approval prompt shows the summary.
                let req = ApprovalRequest { intent, display: None };
                self.sink.emit(AgentEvent::Approval(req.clone()));
                matches!(self.approval.request(req).await,
                    ApprovalResponse::Approve | ApprovalResponse::ApproveAlways)
            }
        };
        if !allowed {
            return Err(ToolError::Denied("user declined".into()));
        }
        // NOTE: this token is currently inert — it is not wired to any external
        // cancel source (e.g. Ctrl-C / SIGINT). Live cancellation is not yet
        // functional; this is a stub for future wiring.
        let ctx = ToolCtx { workspace: self.config.workspace.clone(),
            timeout: self.config.tool_timeout, cancel: CancellationToken::new(),
            // stopgap; Task 3 replaces this with the config-driven strategy
            sandbox: std::sync::Arc::new(agent_tools::HostExecutor) };
        tool.execute(call.args, &ctx).await
    }
}

/// Merge a streamed tool-call delta into the accumulator (handles fragmented args).
fn merge_tool_call(acc: &mut Vec<RawToolCall>, delta: RawToolCall) {
    if delta.id.is_some() || acc.is_empty() {
        acc.push(delta);
    } else if let Some(last) = acc.last_mut() {
        if last.name.is_none() { last.name = delta.name; }
        last.args_fragment.push_str(&delta.args_fragment);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use crate::{WindowContext};
    use agent_model::Message;
    use agent_policy::RulePolicy;
    use agent_tools::{fs::ReadFile, ToolRegistry};
    use std::sync::Arc;

    fn registry() -> Arc<ToolRegistry> {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(ReadFile));
        Arc::new(r)
    }
    fn policy(ws: std::path::PathBuf) -> Arc<RulePolicy> {
        Arc::new(RulePolicy { workspace: ws, command_allowlist: vec![], command_denylist: vec![] })
    }

    #[tokio::test]
    async fn runs_tool_then_finishes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "FILEBODY").unwrap();
        let ws = dir.path().to_path_buf();

        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()),
            Scripted::Text("The file says FILEBODY".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2,
                temperature: 0.0, max_tokens: None, workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });

        let mut ctx = WindowContext::new(Message::system("you are a test agent"));
        agent.run(&mut ctx, "read a.txt".into()).await.unwrap();

        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e == "tool_start:read_file"));
        assert!(events.iter().any(|e| e == "tool_result:read_file"));
        assert!(events.last().unwrap() == "done");
    }

    use agent_policy::PolicyEngine;
    use agent_tools::ToolIntent;

    struct DenyAll;
    impl PolicyEngine for DenyAll {
        fn check(&self, _i: &ToolIntent) -> Decision { Decision::Deny("nope".into()) }
    }

    #[tokio::test]
    async fn denied_tool_feeds_error_back_and_continues() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "X").unwrap();
        let ws = dir.path().to_path_buf();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()),
            Scripted::Text("Understood, it was denied.".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), Arc::new(DenyAll),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        let events = sink.events.lock().unwrap().clone();
        // No tool_result (it was denied), but the loop still reached done.
        assert!(!events.iter().any(|e| e == "tool_result:read_file"));
        assert_eq!(events.last().unwrap(), "done");
    }

    #[tokio::test]
    async fn transport_error_then_success_via_retry() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Error,
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 3, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        assert_eq!(sink.events.lock().unwrap().last().unwrap(), "done");
    }

    #[tokio::test]
    async fn budget_exhaustion_stops_the_loop() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "X").unwrap();
        let ws = dir.path().to_path_buf();
        // Model always calls a tool, never finishes -> must hit max_turns.
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c".into(), "read_file".into(), r#"{"path":"a.txt"}"#.into()); 100
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 3, max_retries: 1, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "loop forever".into()).await.unwrap();
        // 3 turns, each a tool call, then done (BudgetExhausted).
        let events = sink.events.lock().unwrap().clone();
        assert_eq!(events.iter().filter(|e| *e == "tool_start:read_file").count(), 3);
        assert_eq!(events.last().unwrap(), "done");
    }

    #[tokio::test(start_paused = true)]
    async fn idle_stall_times_out_and_fails_after_retries() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::Hang, Scripted::Hang, Scripted::Hang]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 2, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        // Guard >> the loop's 10s idle timeout so the loop terminates first.
        let result = tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
            .await
            .expect("loop must terminate on a stalled stream, not hang");
        let err = result.unwrap_err();
        assert!(matches!(err, AgentError::Model(_)));
        assert!(err.to_string().contains("timeout"),
            "expected a timeout-caused failure, got: {err}");
        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e.starts_with("error:")));
    }

    #[tokio::test(start_paused = true)]
    async fn stream_open_stall_times_out() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![Scripted::HangOpen, Scripted::HangOpen]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 1, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        let result = tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
            .await
            .expect("loop must terminate when the stream never opens, not hang");
        let err = result.unwrap_err();
        assert!(matches!(err, AgentError::Model(_)));
        assert!(err.to_string().contains("timeout"),
            "expected a timeout-caused failure, got: {err}");
    }

    #[tokio::test(start_paused = true)]
    async fn stall_then_success_recovers_via_retry() {
        let ws = std::env::temp_dir();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Hang,
            Scripted::Text("recovered".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 3, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        let result = tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
            .await
            .expect("loop must terminate, not hang");
        assert!(result.is_ok());
        assert_eq!(sink.events.lock().unwrap().last().unwrap(), "done");
    }

    struct SlowModel { gap: Duration }
    #[async_trait::async_trait]
    impl agent_model::ModelClient for SlowModel {
        async fn stream(&self, _req: CompletionRequest)
            -> Result<futures::stream::BoxStream<'static, Result<Chunk, ModelError>>, ModelError> {
            let gap = self.gap;
            let chunks = vec![
                Ok(Chunk::Text("hel".into())),
                Ok(Chunk::Text("lo".into())),
                Ok(Chunk::Done(StopReason::Stop)),
            ];
            Ok(futures::stream::iter(chunks)
                .then(move |c| async move { tokio::time::sleep(gap).await; c })
                .boxed())
        }
    }

    async fn run_reasoning(preserve: bool) -> Vec<Message> {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Reasoning("secret plan".into(), "final answer".into()),
        ]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink,
            LoopConfig { model_limit: 100_000, max_turns: 5, max_retries: 1, temperature: 0.0,
                max_tokens: None, workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                preserve_thinking: preserve, ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();
        ctx.build(100_000)
    }

    #[tokio::test]
    async fn preserve_thinking_keeps_reasoning_in_history() {
        let msgs = run_reasoning(true).await;
        let a = msgs.iter().find(|m| matches!(m.role, agent_model::Role::Assistant)).unwrap();
        assert!(a.content.contains("<think>secret plan</think>"));
        assert!(a.content.contains("final answer"));
    }

    #[tokio::test]
    async fn default_strips_reasoning_from_history() {
        let msgs = run_reasoning(false).await;
        let a = msgs.iter().find(|m| matches!(m.role, agent_model::Role::Assistant)).unwrap();
        assert!(!a.content.contains("secret plan"));
        assert_eq!(a.content, "final answer");
    }

    #[tokio::test(start_paused = true)]
    async fn slow_but_progressing_stream_does_not_trip() {
        let ws = std::env::temp_dir();
        // gap (5s) < idle timeout (10s): healthy progress must NOT trip the timeout.
        let model = Arc::new(SlowModel { gap: Duration::from_secs(5) });
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model, Arc::new(PassthroughProtocol), registry(), policy(ws.clone()),
            Arc::new(AlwaysApprove), sink.clone(),
            LoopConfig { model_limit: 100_000, max_turns: 10, max_retries: 1, temperature: 0.0,
                max_tokens: None, workspace: ws, tool_timeout: Duration::from_secs(5),
                stream_idle_timeout: Duration::from_secs(10), ..Default::default() });
        let mut ctx = WindowContext::new(Message::system("sys"));
        let result = tokio::time::timeout(Duration::from_secs(600), agent.run(&mut ctx, "go".into()))
            .await
            .expect("loop must terminate, not hang");
        assert!(result.is_ok());
        let events = sink.events.lock().unwrap().clone();
        assert!(!events.iter().any(|e| e.starts_with("error:")));
        assert_eq!(events.last().unwrap(), "done");
    }
}
