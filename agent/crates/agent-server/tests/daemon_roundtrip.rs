use agent_runtime_config::RuntimeConfig;
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// The daemon connects to a fake Worker, exercises the `user_input` read-loop
/// arm (spawns an `agent.run` that fails fast against a closed port — the daemon
/// loop must survive), then honours a `settings_get` query and returns a
/// `settings_state` frame, confirming the daemon is still alive and responsive.
#[tokio::test]
async fn settings_get_round_trips_over_websocket() {
    // 1. Fake Worker: accept one daemon connection.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

        // Send a user_input frame first. The spawned agent.run() will fail fast
        // because the model base_url points at a closed port (127.0.0.1:1). The
        // daemon must not crash — the read loop should keep running.
        let user_input_frame = serde_json::json!({
            "v": 1, "session_id": "s1", "kind": "user_input", "text": "hello"
        });
        ws.send(WsMessage::Text(user_input_frame.to_string())).await.unwrap();

        // Now send a settings_get frame to verify the daemon loop is still alive.
        let get_frame = serde_json::json!({
            "v": 1, "session_id": "s1", "kind": "settings_get"
        });
        ws.send(WsMessage::Text(get_frame.to_string())).await.unwrap();

        // Expect a settings_state response, proving the read loop survived the
        // user_input arm's background spawn erroring out.
        let mut saw_state = false;
        while let Some(Ok(msg)) = ws.next().await {
            let WsMessage::Text(t) = msg else { continue };
            let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap();
            if v["kind"] == "settings_state" {
                saw_state = true;
                break;
            }
        }
        assert!(saw_state, "expected a settings_state response");
    });

    // 2. Daemon with a valid config pointing at a non-existent model endpoint.
    let workspace = tempfile::tempdir().unwrap();
    let config_path = workspace.path().join("agent-runtime.json");
    let config = RuntimeConfig::from_launch(
        "openai".into(),
        "http://127.0.0.1:1".into(), // port 1 — connection will be refused
        "default".into(),
        "native".into(),
        8192,
    );
    let params = agent_server::daemon::DaemonParams {
        ws_url: format!("ws://{addr}/agent"),
        agent_token: "test-token".into(),
        config,
        api_key: None,
        claude_binary: "claude".into(),
        config_path,
        workspace: workspace.path().to_path_buf(),
        system_prompt: agent_server::daemon::SYSTEM_PROMPT.to_string(),
        mcp_tools: std::sync::Arc::from(Vec::<std::sync::Arc<dyn agent_tools::Tool>>::new()),
    };

    let daemon = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(10),
            agent_server::daemon::run(params)).await;
    });

    server.await.unwrap();
    daemon.abort();
}
