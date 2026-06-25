use agent_runtime_config::RuntimeConfig;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// The Tauri bridge's path: the agent ACCEPTS an inbound socket and runs
/// `serve()` on it (no outbound dial). A client connects, sends `settings_get`,
/// and must receive a `settings_state` frame — proving `serve()` drives the
/// runtime over an accepted connection.
#[tokio::test]
async fn serve_answers_settings_get_over_accepted_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let workspace = tempfile::tempdir().unwrap();
    let config_path = workspace.path().join("agent-runtime.json");
    let params = agent_server::daemon::DaemonParams {
        config: RuntimeConfig::from_launch(
            "openai".into(),
            "http://127.0.0.1:1".into(),
            "default".into(),
            "native".into(),
            8192,
        ),
        api_key: None,
        claude_binary: "claude".into(),
        config_path,
        workspace: workspace.path().to_path_buf(),
        system_prompt: agent_server::daemon::SYSTEM_PROMPT.to_string(),
        mcp_tools: Arc::from(Vec::<Arc<dyn agent_tools::Tool>>::new()),
        memory_tools: Arc::from(Vec::<Arc<dyn agent_tools::Tool>>::new()),
        memory_retriever: None,
        recall_token_budget: 512,
    };

    let agent = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            agent_server::daemon::serve(ws, params),
        )
        .await;
    });

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/agent"))
        .await
        .unwrap();
    ws.send(WsMessage::Text(
        serde_json::json!({"v":1,"session_id":"s1","kind":"settings_get"}).to_string(),
    ))
    .await
    .unwrap();

    let mut saw_state = false;
    while let Some(Ok(msg)) = ws.next().await {
        let WsMessage::Text(t) = msg else { continue };
        let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap();
        if v["kind"] == "settings_state" {
            saw_state = true;
            break;
        }
    }
    assert!(saw_state, "expected a settings_state response from serve()");
    agent.abort();
}
