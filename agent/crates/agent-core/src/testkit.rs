//! Test doubles for driving the loop deterministically.
use crate::{AgentEvent, ContextEvent, EventSink};
use agent_model::{AssistantTurn, Chunk, CompletionRequest, ModelClient, ModelError,
                  ParsedTurn, ProtocolError, RawToolCall, StopReason, ToolCallProtocol};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use std::sync::Mutex;

/// One scripted assistant turn the mock will emit, in order.
#[derive(Clone)]
pub enum Scripted {
    /// Final assistant text, no tool calls.
    Text(String),
    /// A native tool call: (id, name, json-args-string).
    Call(String, String, String),
    /// One assistant turn emitting several native tool calls: each (id, name, json-args).
    Calls(Vec<(String, String, String)>),
    /// Force a transport error this turn.
    Error,
    /// `stream()` succeeds but the returned stream never yields (inter-chunk stall).
    Hang,
    /// The `stream()` call itself never resolves (stream-open stall).
    HangOpen,
    /// A native tool call truncated by `max_tokens`: partial json-args + a
    /// `finish_reason: "length"` stop. Models the real "write a large file"
    /// case where the args JSON is cut off mid-string. (name, partial-json-args).
    TruncatedCall(String, String),
    /// Emits a reasoning chunk then a final answer (no tool calls): (reasoning, answer).
    Reasoning(String, String),
    /// Final text plus a server usage chunk: (answer, prompt_tokens, completion_tokens).
    TextWithUsage(String, u32, u32),
}

pub struct ScriptedModel { turns: Mutex<std::collections::VecDeque<Scripted>> }
impl ScriptedModel {
    pub fn new(turns: Vec<Scripted>) -> Self {
        Self { turns: Mutex::new(turns.into()) }
    }
}

#[async_trait]
impl ModelClient for ScriptedModel {
    async fn stream(&self, _req: CompletionRequest)
        -> Result<BoxStream<'static, Result<Chunk, ModelError>>, ModelError> {
        let next = self.turns.lock().unwrap().pop_front()
            .unwrap_or(Scripted::Text(String::new()));
        match next {
            Scripted::Error => Err(ModelError::Http("scripted error".into())),
            Scripted::Text(t) => Ok(stream::iter(vec![
                Ok(Chunk::Text(t)), Ok(Chunk::Done(StopReason::Stop))]).boxed()),
            Scripted::Call(id, name, args) => Ok(stream::iter(vec![
                Ok(Chunk::ToolCallDelta(RawToolCall { index: None, id: Some(id), name: Some(name),
                    args_fragment: args })),
                Ok(Chunk::Done(StopReason::ToolCalls))]).boxed()),
            Scripted::Calls(calls) => {
                let mut chunks: Vec<Result<Chunk, ModelError>> = Vec::new();
                for (i, (id, name, args)) in calls.into_iter().enumerate() {
                    chunks.push(Ok(Chunk::ToolCallDelta(RawToolCall {
                        index: Some(i), id: Some(id), name: Some(name), args_fragment: args })));
                }
                chunks.push(Ok(Chunk::Done(StopReason::ToolCalls)));
                Ok(stream::iter(chunks).boxed())
            }
            Scripted::TruncatedCall(name, partial) => Ok(stream::iter(vec![
                Ok(Chunk::ToolCallDelta(RawToolCall { index: None, id: Some("c0".into()),
                    name: Some(name), args_fragment: partial })),
                Ok(Chunk::Done(StopReason::Length))]).boxed()),
            Scripted::Reasoning(reasoning, answer) => Ok(stream::iter(vec![
                Ok(Chunk::Reasoning(reasoning)), Ok(Chunk::Text(answer)),
                Ok(Chunk::Done(StopReason::Stop))]).boxed()),
            Scripted::TextWithUsage(answer, prompt_tokens, completion_tokens) => Ok(stream::iter(vec![
                Ok(Chunk::Text(answer)),
                Ok(Chunk::Usage { prompt_tokens, completion_tokens }),
                Ok(Chunk::Done(StopReason::Stop))]).boxed()),
            Scripted::Hang => Ok(stream::pending().boxed()),
            Scripted::HangOpen => {
                std::future::pending::<()>().await;
                unreachable!("HangOpen never resolves")
            }
        }
    }
}

/// Trivial protocol for tests: reads native deltas, no prompt injection.
pub struct PassthroughProtocol;
impl ToolCallProtocol for PassthroughProtocol {
    fn prepare(&self, _req: &mut CompletionRequest) {}
    fn parse(&self, raw: &AssistantTurn) -> Result<ParsedTurn, ProtocolError> {
        let mut calls = Vec::new();
        for rc in &raw.raw_tool_calls {
            let name = rc.name.clone().ok_or_else(|| ProtocolError("no name".into()))?;
            let args = if rc.args_fragment.is_empty() { serde_json::json!({}) }
                else { serde_json::from_str(&rc.args_fragment)
                    .map_err(|e| ProtocolError(e.to_string()))? };
            calls.push(agent_tools::ToolCall {
                id: rc.id.clone().unwrap_or_else(|| "c".into()), name, args });
        }
        Ok(ParsedTurn { text: raw.text.clone(), tool_calls: calls })
    }
}

#[derive(Default)]
pub struct CollectingSink { pub events: Mutex<Vec<String>> }
impl EventSink for CollectingSink {
    fn emit(&self, event: AgentEvent) {
        let label = match event {
            AgentEvent::Token(t) => format!("token:{t}"),
            AgentEvent::Reasoning(r) => format!("reasoning:{r}"),
            AgentEvent::Usage { prompt_tokens, .. } => format!("usage:{prompt_tokens}"),
            AgentEvent::ServerUsage { prompt_tokens, completion_tokens, .. } => {
                format!("server_usage:{prompt_tokens}:{completion_tokens}")
            }
            AgentEvent::ToolStart { name, .. } => format!("tool_start:{name}"),
            AgentEvent::ToolResult { name, status, .. } =>
                format!("tool_result:{name}:{}", status.as_str()),
            AgentEvent::Approval(_) => "approval".into(),
            AgentEvent::Error(e) => format!("error:{e}"),
            AgentEvent::Done(_) => "done".into(),
            AgentEvent::Context(ContextEvent::Offloaded { id, .. }) => format!("offloaded:{id}"),
            AgentEvent::Context(ContextEvent::Compacted { turns_replaced, .. }) => {
                format!("compacted:{turns_replaced}")
            }
            AgentEvent::Context(ContextEvent::CompactionFailed { .. }) => "compaction_failed".into(),
            AgentEvent::SandboxDegraded { mechanism, .. } => format!("sandbox_degraded:{mechanism}"),
        };
        self.events.lock().unwrap().push(label);
    }
}

pub struct AlwaysApprove;
#[async_trait]
impl ApprovalChannel for AlwaysApprove {
    async fn request(&self, _req: ApprovalRequest) -> ApprovalResponse { ApprovalResponse::Approve }
}
