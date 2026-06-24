use rust_agent_runtime_desktop_lib::llama::check_health;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn reports_ok_and_model_from_props() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"ok"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/props"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "default_generation_settings": { "model": "qwen3.6-35b-a3b" }
        })))
        .mount(&server)
        .await;

    let h = check_health(&server.uri()).await;
    assert!(h.ok);
    assert_eq!(h.model.as_deref(), Some("qwen3.6-35b-a3b"));
}

#[tokio::test]
async fn reports_not_ok_when_server_down() {
    // Nothing listening on this port.
    let h = check_health("http://127.0.0.1:1").await;
    assert!(!h.ok);
    assert!(h.model.is_none());
}
