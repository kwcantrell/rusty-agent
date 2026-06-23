use crate::approval::WsApprovalChannel;
use agent_runtime_config::{build_registry, default_allowlist, default_denylist, pick_protocol};
use crate::sink::WsEventSink;
use crate::wire::{WireBody, WireEnvelope};
use agent_core::{AgentLoop, LoopConfig, WindowContext};
use agent_model::{Message, ModelClient};
use agent_policy::RulePolicy;
use futures::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;

type DynErr = Box<dyn std::error::Error + Send + Sync>;

pub struct DaemonParams {
    pub ws_url: String,        // ws://host/agent
    pub agent_token: String,
    pub model: Arc<dyn ModelClient>,
    pub protocol: String,
    pub workspace: std::path::PathBuf,
    pub context_limit: usize,
}

const SYSTEM_PROMPT: &str = "You are a local coding agent. Use the provided tools to inspect \
and modify the workspace. Think step by step. When the task is complete, reply with a summary \
and no tool call.";

pub async fn run(params: DaemonParams) -> Result<(), DynErr> {
    // Shared session id (MVP: one active session per agent). The read loop sets it
    // on each user_input; the sink and approval channel stamp outgoing frames with it.
    let session = Arc::new(Mutex::new(String::new()));
    let (tx, mut rx) = mpsc::unbounded_channel::<WireEnvelope>();

    let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
    let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(),
        Duration::from_secs(300)));

    let policy = Arc::new(RulePolicy {
        workspace: params.workspace.clone(),
        command_allowlist: default_allowlist(),
        command_denylist: default_denylist(),
    });
    let agent = Arc::new(AgentLoop::new(
        params.model,
        pick_protocol(&params.protocol),
        Arc::new(build_registry()),
        policy,
        approval.clone(),
        sink,
        LoopConfig {
            model_limit: params.context_limit, max_turns: 25, max_retries: 3,
            temperature: 0.2, max_tokens: Some(2048), workspace: params.workspace.clone(),
            tool_timeout: Duration::from_secs(120),
            stream_idle_timeout: agent_core::DEFAULT_STREAM_IDLE_TIMEOUT,
        },
    ));
    let ctx = Arc::new(tokio::sync::Mutex::new(
        WindowContext::new(Message::system(SYSTEM_PROMPT))));

    let mut req = params.ws_url.clone().into_client_request()?;
    req.headers_mut().insert("Authorization",
        format!("Bearer {}", params.agent_token).parse()?);
    let (ws, _resp) = tokio_tungstenite::connect_async(req).await?;
    let (mut write, mut read) = ws.split();

    // Writer task: drain the channel to the socket; ping periodically to stay alive.
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(25));
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(env) => {
                        let txt = serde_json::to_string(&env).unwrap_or_default();
                        if write.send(WsMessage::Text(txt)).await.is_err() { break; }
                    }
                    None => break,
                },
                _ = ping.tick() => {
                    if write.send(WsMessage::Ping(Vec::new())).await.is_err() { break; }
                }
            }
        }
    });

    // Read loop: dispatch inbound frames.
    while let Some(msg) = read.next().await {
        let msg = match msg { Ok(m) => m, Err(_) => break };
        let WsMessage::Text(t) = msg else { continue };
        let env: WireEnvelope = match serde_json::from_str(t.as_str()) {
            Ok(e) => e,
            Err(e) => { tracing::warn!(error=%e, "bad frame"); continue }
        };
        match env.body {
            WireBody::UserInput { text } => {
                *session.lock().unwrap() = env.session_id.clone();
                let agent = agent.clone();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let mut guard = ctx.lock().await;
                    if let Err(e) = agent.run(&mut *guard, text).await {
                        tracing::error!(error=%e, "run failed");
                    }
                });
            }
            WireBody::ApprovalResponse { decision } => {
                if let Some(id) = env.id {
                    approval.resolve(&id, decision.into());
                }
            }
            _ => {}
        }
    }
    writer.abort();
    Ok(())
}
