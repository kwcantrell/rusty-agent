use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// Start the bridge, connect to its advertised ws_url, send settings_get, and
/// expect a settings_state frame back — proving the bridge wires an accepted
/// connection into agent_server::serve() with a desktop runtime.
#[tokio::test]
async fn bridge_serves_local_runtime() {
    let ws_dir = tempfile::tempdir().unwrap();
    let cfg = ws_dir.path().join("agent-runtime.json");
    let bridge = rust_agent_runtime_desktop_lib::bridge::start(
        ws_dir.path().to_path_buf(),
        cfg,
        "http://127.0.0.1:1".into(), // closed port: agent.run fails fast, loop survives
        "default".into(),
    )
    .await
    .unwrap();

    let url = bridge.ws_url();
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(WsMessage::Text(
        serde_json::json!({"v":1,"session_id":"s1","kind":"settings_get"}).to_string(),
    ))
    .await
    .unwrap();

    let saw = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(msg)) = ws.next().await {
            if let WsMessage::Text(t) = msg {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v["kind"] == "settings_state" {
                    return true;
                }
            }
        }
        false
    })
    .await
    .unwrap();
    assert!(saw, "expected settings_state from the bridge");
}
