use crate::wire::messages_to_json;
use crate::{Chunk, CompletionRequest, ModelError, RawToolCall, StopReason};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::{json, Value};

/// Trait for streaming chat-completion clients.
#[async_trait]
pub trait ModelClient: Send + Sync {
    async fn stream(
        &self,
        req: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<Chunk, ModelError>>, ModelError>;
}

/// OpenAI-compatible streaming client.
pub struct OpenAiCompatClient {
    base_url: String,
    model: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl OpenAiCompatClient {
    pub fn new(base_url: String, model: String, api_key: Option<String>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            api_key,
            http: reqwest::Client::new(),
        }
    }

    fn body(&self, req: &CompletionRequest) -> Value {
        let mut b = json!({
            "model": self.model,
            "messages": messages_to_json(&req.messages),
            "stream": true,
            "temperature": req.temperature,
            "chat_template_kwargs": { "enable_thinking": req.enable_thinking },
        });
        if let Some(mt) = req.max_tokens {
            b["max_tokens"] = json!(mt);
        }
        if let Some(v) = req.top_p { b["top_p"] = json!(v); }
        if let Some(v) = req.top_k { b["top_k"] = json!(v); }
        if let Some(v) = req.min_p { b["min_p"] = json!(v); }
        if let Some(v) = req.presence_penalty { b["presence_penalty"] = json!(v); }
        if let Some(v) = req.repeat_penalty { b["repeat_penalty"] = json!(v); }
        if !req.tools.is_empty() {
            b["tools"] = json!(req
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters
                        }
                    })
                })
                .collect::<Vec<_>>());
        }
        b
    }
}

fn parse_sse_line(line: &str) -> Option<Result<Vec<Chunk>, ModelError>> {
    let data = line.strip_prefix("data:")?.trim();
    if data == "[DONE]" {
        return Some(Ok(vec![]));
    }
    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => return Some(Err(ModelError::Decode(e.to_string()))),
    };
    let choice = &v["choices"][0];
    let mut out = Vec::new();
    if let Some(content) = choice["delta"]["content"].as_str() {
        if !content.is_empty() {
            out.push(Chunk::Text(content.to_string()));
        }
    }
    if let Some(calls) = choice["delta"]["tool_calls"].as_array() {
        for c in calls {
            out.push(Chunk::ToolCallDelta(RawToolCall {
                id: c["id"].as_str().map(str::to_string),
                name: c["function"]["name"].as_str().map(str::to_string),
                args_fragment: c["function"]["arguments"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
            }));
        }
    }
    if let Some(reason) = choice["finish_reason"].as_str() {
        let stop = match reason {
            "tool_calls" => StopReason::ToolCalls,
            "length" => StopReason::Length,
            _ => StopReason::Stop,
        };
        out.push(Chunk::Done(stop));
    }
    Some(Ok(out))
}

#[async_trait]
impl ModelClient for OpenAiCompatClient {
    async fn stream(
        &self,
        req: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<Chunk, ModelError>>, ModelError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let mut builder = self.http.post(&url).json(&self.body(&req));
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| ModelError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ModelError::Status(resp.status().as_u16()));
        }

        let mut byte_stream = resp.bytes_stream();
        let stream = async_stream::stream! {
            let mut buf = String::new();
            loop {
                // Drain any complete lines from buf before fetching more bytes.
                if let Some(idx) = buf.find('\n') {
                    let line = buf[..idx].trim().to_string();
                    buf.drain(..=idx);
                    if line.is_empty() {
                        continue;
                    }
                    match parse_sse_line(&line) {
                        None => continue,
                        Some(Err(e)) => {
                            yield Err(e);
                            return;
                        }
                        Some(Ok(chunks)) => {
                            let is_done = line.contains("[DONE]");
                            for chunk in chunks {
                                yield Ok(chunk);
                            }
                            if is_done {
                                return;
                            }
                            continue;
                        }
                    }
                }
                // Need more bytes.
                match byte_stream.next().await {
                    Some(Ok(b)) => buf.push_str(&String::from_utf8_lossy(&b)),
                    Some(Err(e)) => {
                        yield Err(ModelError::Stream(e.to_string()));
                        return;
                    }
                    None => return,
                }
            }
        };
        Ok(stream.boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use futures::StreamExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn streams_text_chunks_then_done() {
        let server = MockServer::start().await;
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                    data: [DONE]\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let client = OpenAiCompatClient::new(server.uri(), "test-model".into(), None);
        let req = CompletionRequest {
            messages: vec![Message::user("hi")],
            ..Default::default()
        };
        let mut stream = client.stream(req).await.unwrap();

        let mut text = String::new();
        let mut done = None;
        while let Some(item) = stream.next().await {
            match item.unwrap() {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Done(r) => done = Some(r),
                Chunk::ToolCallDelta(_) => {}
            }
        }
        assert_eq!(text, "Hello");
        assert_eq!(done, Some(StopReason::Stop));
    }

    #[tokio::test]
    async fn surfaces_http_error_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let client = OpenAiCompatClient::new(server.uri(), "m".into(), None);
        let req = CompletionRequest {
            messages: vec![],
            ..Default::default()
        };
        let err = client.stream(req).await.err().unwrap();
        assert!(matches!(err, ModelError::Status(500)));
    }

    #[test]
    fn body_serializes_sampling_and_thinking() {
        let client = OpenAiCompatClient::new("http://x".into(), "m".into(), None);
        let req = CompletionRequest {
            messages: vec![Message::user("hi")],
            top_p: Some(0.8),
            top_k: Some(30),
            enable_thinking: false,
            ..Default::default()
        };
        let b = client.body(&req);
        // f32 0.8 serialises as an f64 approximation; compare via as_f64.
        assert!((b["top_p"].as_f64().unwrap() - 0.8_f32 as f64).abs() < 1e-6);
        assert_eq!(b["top_k"], serde_json::json!(30));
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], serde_json::json!(false));
        // Unset params are omitted entirely.
        assert!(b.get("min_p").is_none());
        assert!(b.get("presence_penalty").is_none());
        assert!(b.get("repeat_penalty").is_none());
    }
}
