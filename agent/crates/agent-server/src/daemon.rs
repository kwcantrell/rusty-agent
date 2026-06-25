use crate::approval::WsApprovalChannel;
use crate::runtime::RuntimeState;
use crate::sink::WsEventSink;
use crate::wire::{WireBody, WireEnvelope};
use agent_core::{ContextManager, WindowContext};
use agent_model::Message;
use agent_runtime_config::RuntimeConfig;
use agent_tools::Tool;
use futures::{SinkExt, StreamExt};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::WebSocketStream;

type DynErr = Box<dyn std::error::Error + Send + Sync>;

pub struct DaemonParams {
    pub config: RuntimeConfig, // flag-derived base; the file at config_path overlays it
    pub api_key: Option<String>,
    pub claude_binary: String,
    pub config_path: PathBuf,
    pub workspace: PathBuf,
    pub system_prompt: String,
    pub mcp_tools: Arc<[Arc<dyn Tool>]>,
    pub memory_tools: Arc<[Arc<dyn Tool>]>,
}

pub const SYSTEM_PROMPT: &str = "You are a local coding agent. Use the provided tools to inspect \
and modify the workspace. Think step by step. When the task is complete, reply with a summary \
and no tool call.";

/// Drive the runtime over an already-established WebSocket. The desktop bridge
/// (`src-tauri/src/bridge.rs`) accepts a local connection and hands the socket here.
pub async fn serve<S>(ws: WebSocketStream<S>, params: DaemonParams) -> Result<(), DynErr>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Shared session id (MVP: one active session per agent). The read loop sets it
    // on each user_input; the sink, approval channel, and settings replies stamp it.
    let session = Arc::new(Mutex::new(String::new()));
    let (tx, mut rx) = mpsc::unbounded_channel::<WireEnvelope>();

    let sink = Arc::new(WsEventSink::new(tx.clone(), session.clone()));
    let approval = Arc::new(WsApprovalChannel::new(tx.clone(), session.clone(),
        Duration::from_secs(300)));

    // Live settings survive reconnect/restart: overlay the persisted file on the flag base.
    let config = RuntimeConfig::load_over(params.config.clone(), &params.config_path);
    let runtime = Arc::new(RuntimeState::new(
        config,
        sink,
        approval.clone(),
        params.workspace.clone(),
        params.api_key.clone(),
        params.claude_binary.clone(),
        params.config_path.clone(),
        session.clone(),
        tx.clone(),
        params.mcp_tools.clone(),
        params.memory_tools.clone(),
        params.system_prompt.clone(),
    ));
    let ctx = Arc::new(tokio::sync::Mutex::new(
        WindowContext::new(Message::system(params.system_prompt.clone()))));

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
                let agent = runtime.current_loop();
                let system_prompt = runtime.current_system_prompt();
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let mut guard = ctx.lock().await;
                    guard.set_system(Message::system(system_prompt));
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
            other @ (WireBody::SettingsGet | WireBody::SettingsUpdate { .. }) => {
                *session.lock().unwrap() = env.session_id.clone();
                runtime.handle(&other);
            }
            _ => {}
        }
    }
    writer.abort();
    Ok(())
}
