//! Live end-to-end check against a running llama-server on :8080.
//! Ignored by default (needs the model server up). Run explicitly:
//!   cargo test --test e2e_live -- --ignored --nocapture
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[tokio::test]
#[ignore = "requires llama-server running on localhost:8080"]
async fn live_user_input_streams_tokens_from_the_model() {
    let ws_dir = tempfile::tempdir().unwrap();
    let cfg = ws_dir.path().join("agent-runtime.json");

    // Same wiring the desktop app uses: bridge -> serve() -> agent loop -> :8080.
    let bridge = rust_agent_runtime_desktop_lib::bridge::start(
        ws_dir.path().to_path_buf(),
        cfg,
        "http://localhost:8080".into(),
        "qwen3.6-35b-a3b".into(),
    )
    .await
    .unwrap();

    let (mut ws, _) = tokio_tungstenite::connect_async(bridge.ws_url())
        .await
        .unwrap();

    // Exactly the frame the React app sends on send().
    ws.send(WsMessage::Text(
        serde_json::json!({
            "v": 1, "session_id": "e2e", "kind": "user_input",
            "text": "Reply with exactly the single word: pong"
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let mut tokens = String::new();
    let mut saw_event = false;
    let mut done = false;

    let outcome = tokio::time::timeout(Duration::from_secs(120), async {
        while let Some(Ok(msg)) = ws.next().await {
            let WsMessage::Text(t) = msg else { continue };
            let v: serde_json::Value = serde_json::from_str(&t).unwrap();
            if v["kind"] != "event" {
                continue;
            }
            saw_event = true;
            let payload = &v["payload"];
            match payload["type"].as_str() {
                Some("token") => {
                    let s = payload["text"].as_str().unwrap_or("");
                    tokens.push_str(s);
                    print!("{s}");
                }
                Some("tool_start") => {
                    println!("\n[tool_start] {}", payload["name"]);
                }
                Some("error") => {
                    panic!("agent error: {}", payload["message"]);
                }
                Some("done") => {
                    println!("\n[done] {}", payload["reason"]);
                    done = true;
                    break;
                }
                _ => {}
            }
        }
    })
    .await;

    assert!(outcome.is_ok(), "timed out waiting for the model to respond");
    assert!(saw_event, "no event frames received from the runtime");
    assert!(done, "stream never reached a `done` event");
    println!("\n--- collected tokens: {tokens:?} ---");
}
