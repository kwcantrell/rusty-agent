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

pub struct LoopConfig {
    pub model_limit: usize,
    pub max_turns: usize,
    pub max_retries: usize,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub workspace: PathBuf,
    pub tool_timeout: Duration,
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
        let mut stream = self.model.stream(req).await?;
        let mut text = String::new();
        let mut raw_tool_calls: Vec<RawToolCall> = Vec::new();
        let mut stop = StopReason::Stop;
        while let Some(item) = stream.next().await {
            match item? {
                Chunk::Text(t) => { self.sink.emit(AgentEvent::Token(t.clone())); text.push_str(&t); }
                Chunk::ToolCallDelta(rc) => merge_tool_call(&mut raw_tool_calls, rc),
                Chunk::Done(r) => stop = r,
            }
        }
        Ok(AssistantTurn { text, raw_tool_calls, stop })
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

            ctx.append(Message::assistant(parsed.text.clone(),
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
        let ctx = ToolCtx { workspace: self.config.workspace.clone(),
            timeout: self.config.tool_timeout, cancel: CancellationToken::new() };
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
                tool_timeout: std::time::Duration::from_secs(5) });

        let mut ctx = WindowContext::new(Message::system("you are a test agent"));
        agent.run(&mut ctx, "read a.txt".into()).await.unwrap();

        let events = sink.events.lock().unwrap().clone();
        assert!(events.iter().any(|e| e == "tool_start:read_file"));
        assert!(events.iter().any(|e| e == "tool_result:read_file"));
        assert!(events.last().unwrap() == "done");
    }
}
