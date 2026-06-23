use agent_core::testkit::{ScriptedModel, Scripted};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[tokio::test]
async fn user_input_streams_events_and_round_trips_approval() {
    // 1. Fake Worker: accept one daemon connection.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();

        // Tell the daemon to start a turn.
        let user = serde_json::json!({
            "v":1, "session_id":"s1", "kind":"user_input", "text":"please run it"
        });
        ws.send(WsMessage::Text(user.to_string())).await.unwrap();

        // Collect frames until we see the approval_request, approve it, then read to done.
        let mut saw_done = false;
        while let Some(Ok(msg)) = ws.next().await {
            let WsMessage::Text(t) = msg else { continue };
            let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap();
            match v["kind"].as_str() {
                Some("approval_request") => {
                    let id = v["id"].as_str().unwrap();
                    let resp = serde_json::json!({
                        "v":1, "session_id":"s1", "id":id,
                        "kind":"approval_response", "decision":"approve"
                    });
                    ws.send(WsMessage::Text(resp.to_string())).await.unwrap();
                }
                Some("event") if v["payload"]["type"] == "done" => { saw_done = true; break; }
                _ => {}
            }
        }
        assert!(saw_done, "expected a done event");
    });

    // 2. Daemon: scripted model emits a tool call needing approval, then finishes.
    let workspace = tempfile::tempdir().unwrap();
    let model = Arc::new(ScriptedModel::new(vec![
        Scripted::Call("c1".into(), "execute_command".into(),
            r#"{"command":"echo hi > out.txt"}"#.into()),
        Scripted::Text("all done".into()),
    ]));
    let params = agent_server::daemon::DaemonParams {
        ws_url: format!("ws://{addr}/agent"),
        agent_token: "test-token".into(),
        model,
        protocol: "native".into(),
        workspace: workspace.path().to_path_buf(),
        context_limit: 8192,
    };

    // The daemon read loop ends when the fake server closes after `done`.
    let daemon = tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(10),
            agent_server::daemon::run(params)).await;
    });

    server.await.unwrap();
    daemon.abort();
}
