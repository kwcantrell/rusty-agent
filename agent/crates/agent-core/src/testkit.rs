//! Test doubles for driving the loop deterministically.
use crate::{AgentEvent, EventSink};
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
    /// Force a transport error this turn.
    Error,
    /// `stream()` succeeds but the returned stream never yields (inter-chunk stall).
    Hang,
    /// The `stream()` call itself never resolves (stream-open stall).
    HangOpen,
    /// Emits a reasoning chunk then a final answer (no tool calls): (reasoning, answer).
    Reasoning(String, String),
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
                Ok(Chunk::ToolCallDelta(RawToolCall { id: Some(id), name: Some(name),
                    args_fragment: args })),
                Ok(Chunk::Done(StopReason::ToolCalls))]).boxed()),
            Scripted::Reasoning(reasoning, answer) => Ok(stream::iter(vec![
                Ok(Chunk::Reasoning(reasoning)), Ok(Chunk::Text(answer)),
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
            AgentEvent::ToolStart { name, .. } => format!("tool_start:{name}"),
            AgentEvent::ToolResult { name, .. } => format!("tool_result:{name}"),
            AgentEvent::Approval(_) => "approval".into(),
            AgentEvent::Error(e) => format!("error:{e}"),
            AgentEvent::Done(_) => "done".into(),
        };
        self.events.lock().unwrap().push(label);
    }
}

pub struct AlwaysApprove;
#[async_trait]
impl ApprovalChannel for AlwaysApprove {
    async fn request(&self, _req: ApprovalRequest) -> ApprovalResponse { ApprovalResponse::Approve }
}
