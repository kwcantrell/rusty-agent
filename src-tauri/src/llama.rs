//! Read-only health probe for the fixed local llama-server. The webview never
//! contacts :8080 directly (CSP); it calls the `llama_health` command instead.
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct LlamaHealth {
    pub ok: bool,
    pub model: Option<String>,
}

pub async fn check_health(base_url: &str) -> LlamaHealth {
    let base = base_url.trim_end_matches('/');
    let client = reqwest::Client::new();

    let ok = matches!(
        client.get(format!("{base}/health")).send().await,
        Ok(r) if r.status().is_success()
    );

    let model = match client.get(format!("{base}/props")).send().await {
        Ok(r) => r.json::<serde_json::Value>().await.ok().and_then(|v| {
            // llama-server exposes the model under default_generation_settings.model;
            // fall back to a top-level "model" if present.
            v.get("default_generation_settings")
                .and_then(|g| g.get("model"))
                .or_else(|| v.get("model"))
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
        }),
        Err(_) => None,
    };

    LlamaHealth { ok, model }
}
