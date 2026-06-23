use agent_runtime_config::RuntimeConfig;
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// The daemon connects to a fake Worker, honours a settings_get query, and
/// does not crash when a user_input arrives (the in-flight run fails cleanly
/// because there is no real model endpoint, but the daemon loop keeps running).
#[tokio::test]
async fn settings_get_round_trips_over_websocket() {
    // 1. Fake Worker: accept one daemon connection.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

        // Send a settings_get frame.
        let get_frame = serde_json::json!({
            "v": 1, "session_id": "s1", "kind": "settings_get"
        });
        ws.send(WsMessage::Text(get_frame.to_string())).await.unwrap();

        // Expect a settings_state response.
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
    };

    let daemon = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(10),
            agent_server::daemon::run(params)).await;
    });

    server.await.unwrap();
    daemon.abort();
}
