use crate::wire::messages_to_json;
use crate::{Chunk, CompletionRequest, ModelError, RawToolCall, StopReason};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde_json::{json, Value};

/// Splits a streamed `content` channel into answer text and `<think>…</think>`
/// reasoning, buffering a partial tag that straddles a chunk boundary.
#[derive(Default)]
pub(crate) struct ThinkingSplitter {
    in_think: bool,
    buf: String,
}

impl ThinkingSplitter {
    fn emit(out: &mut Vec<Chunk>, in_think: bool, s: &str) {
        if s.is_empty() { return; }
        out.push(if in_think { Chunk::Reasoning(s.to_string()) } else { Chunk::Text(s.to_string()) });
    }

    pub(crate) fn push(&mut self, content: &str) -> Vec<Chunk> {
        let mut out = Vec::new();
        self.buf.push_str(content);
        loop {
            let tag: &str = if self.in_think { "</think>" } else { "<think>" };
            if let Some(idx) = self.buf.find(tag) {
                let before = self.buf[..idx].to_string();
                Self::emit(&mut out, self.in_think, &before);
                self.buf.drain(..idx + tag.len());
                self.in_think = !self.in_think;
                continue;
            }
            let keep = partial_prefix_len(tag, &self.buf);
            let flush_to = self.buf.len() - keep;
            let flush = self.buf[..flush_to].to_string();
            Self::emit(&mut out, self.in_think, &flush);
            self.buf.drain(..flush_to);
            break;
        }
        out
    }

    pub(crate) fn flush(&mut self) -> Vec<Chunk> {
        let mut out = Vec::new();
        let rest = std::mem::take(&mut self.buf);
        Self::emit(&mut out, self.in_think, &rest);
        out
    }
}

/// Length of the longest suffix of `buf` that is a proper prefix of `tag`.
fn partial_prefix_len(tag: &str, buf: &str) -> usize {
    let max = tag.len().saturating_sub(1).min(buf.len());
    for k in (1..=max).rev() {
        let start = buf.len() - k;
        if buf.is_char_boundary(start) && buf[start..] == tag[..k] {
            return k;
        }
    }
    0
}

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
            "chat_template_kwargs": {
                "enable_thinking": req.enable_thinking,
                "preserve_thinking": req.preserve_thinking,
            },
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

/// Accumulates raw response bytes and hands back complete SSE lines. Buffering at
/// the byte level (not as a `String`) is what keeps a multi-byte UTF-8 character
/// intact when it straddles two network chunks: a line is only decoded once its
/// terminating `\n` arrives, and a `\n` can never split a UTF-8 char — so the
/// per-line lossy decode is exact. Only the trailing partial line stays buffered.
#[derive(Default)]
struct SseLineBuffer {
    buf: Vec<u8>,
}

impl SseLineBuffer {
    fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Pop the next complete line (newline stripped, trimmed), or `None` if no
    /// full line is buffered yet.
    fn next_line(&mut self) -> Option<String> {
        let idx = self.buf.iter().position(|&c| c == b'\n')?;
        let line: Vec<u8> = self.buf.drain(..=idx).collect();
        Some(String::from_utf8_lossy(&line[..line.len() - 1]).trim().to_string())
    }
}

fn parse_sse_line(line: &str, splitter: &mut ThinkingSplitter) -> Option<Result<Vec<Chunk>, ModelError>> {
    let data = line.strip_prefix("data:")?.trim();
    if data == "[DONE]" {
        return Some(Ok(vec![]));
    }
    let v: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => return Some(Err(ModelError::Decode(e.to_string()))),
    };
    // A 200-status stream can still carry an error object instead of choices
    // (e.g. llama.cpp slot limits). Surface it instead of parsing empty deltas.
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| err.to_string());
        return Some(Err(ModelError::Stream(format!("server error in stream: {msg}"))));
    }
    let choice = &v["choices"][0];
    let mut out = Vec::new();
    if let Some(reasoning) = choice["delta"]["reasoning_content"].as_str() {
        if !reasoning.is_empty() {
            out.push(Chunk::Reasoning(reasoning.to_string()));
        }
    }
    if let Some(content) = choice["delta"]["content"].as_str() {
        if !content.is_empty() {
            out.extend(splitter.push(content));
        }
    }
    if let Some(calls) = choice["delta"]["tool_calls"].as_array() {
        for c in calls {
            out.push(Chunk::ToolCallDelta(RawToolCall {
                index: c["index"].as_u64().map(|i| i as usize),
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
            let code = resp.status().as_u16();
            // Capture the body — backends put the actionable error here (bad
            // request shape, slot limits, etc.). Truncate so a stray HTML page
            // can't flood logs.
            let mut body = resp.text().await.unwrap_or_default();
            body = body.trim().chars().take(1000).collect();
            return Err(ModelError::Status { code, body });
        }

        let mut byte_stream = resp.bytes_stream();
        let stream = async_stream::stream! {
            let mut lines = SseLineBuffer::default();
            let mut splitter = ThinkingSplitter::default();
            loop {
                // Drain any complete lines before fetching more bytes.
                if let Some(line) = lines.next_line() {
                    if line.is_empty() {
                        continue;
                    }
                    match parse_sse_line(&line, &mut splitter) {
                        None => continue,
                        // A malformed `data:` line (Decode) is transient corruption —
                        // skip it and keep streaming. Any other error (e.g. an in-band
                        // server error) is terminal.
                        Some(Err(ModelError::Decode(e))) => {
                            tracing::warn!(error = %e, "skipping malformed SSE data line");
                            continue;
                        }
                        Some(Err(e)) => {
                            yield Err(e);
                            return;
                        }
                        Some(Ok(chunks)) => {
                            // Only the bare `data: [DONE]` sentinel ends the stream —
                            // NOT a content delta that merely contains "[DONE]".
                            let is_done = line.strip_prefix("data:").map(str::trim) == Some("[DONE]");
                            for chunk in chunks {
                                yield Ok(chunk);
                            }
                            if is_done {
                                for chunk in splitter.flush() { yield Ok(chunk); }
                                return;
                            }
                            continue;
                        }
                    }
                }
                // Need more bytes.
                match byte_stream.next().await {
                    Some(Ok(b)) => lines.push(&b),
                    Some(Err(e)) => {
                        yield Err(ModelError::Stream(e.to_string()));
                        return;
                    }
                    None => {
                        for chunk in splitter.flush() { yield Ok(chunk); }
                        return;
                    }
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

    fn collect(s: &mut ThinkingSplitter, parts: &[&str]) -> (String, String) {
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut chunks: Vec<Chunk> = Vec::new();
        for p in parts { chunks.extend(s.push(p)); }
        chunks.extend(s.flush());
        for c in chunks {
            match c {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Reasoning(r) => reasoning.push_str(&r),
                _ => {}
            }
        }
        (text, reasoning)
    }

    #[test]
    fn sse_line_buffer_preserves_multibyte_char_split_across_chunks() {
        let mut b = SseLineBuffer::default();
        // "5°C" — '°' is 0xC2 0xB0; split the two bytes across separate pushes,
        // mid-line. A per-chunk lossy decode would corrupt it to replacement chars.
        b.push(b"data: {\"t\":\"5\xc2");
        assert!(b.next_line().is_none(), "no newline yet -> hold the partial char");
        b.push(b"\xb0C\"}\n");
        assert_eq!(b.next_line().as_deref(), Some("data: {\"t\":\"5°C\"}"));
        assert!(b.next_line().is_none());
    }

    #[test]
    fn sse_line_buffer_splits_multiple_lines_and_keeps_trailing_partial() {
        let mut b = SseLineBuffer::default();
        b.push(b"line one\nline two\npartial");
        assert_eq!(b.next_line().as_deref(), Some("line one"));
        assert_eq!(b.next_line().as_deref(), Some("line two"));
        assert!(b.next_line().is_none()); // "partial" has no '\n' yet
        b.push(b" done\n");
        assert_eq!(b.next_line().as_deref(), Some("partial done"));
    }

    #[test]
    fn splitter_routes_think_block() {
        let mut s = ThinkingSplitter::default();
        let (text, reasoning) = collect(&mut s, &["<think>plan</think>answer"]);
        assert_eq!(reasoning, "plan");
        assert_eq!(text, "answer");
    }

    #[test]
    fn splitter_handles_tag_split_across_chunks() {
        let mut s = ThinkingSplitter::default();
        let (text, reasoning) = collect(&mut s, &["<thi", "nk>deep", " thought</thi", "nk>done"]);
        assert_eq!(reasoning, "deep thought");
        assert_eq!(text, "done");
    }

    #[test]
    fn splitter_passes_through_plain_text() {
        let mut s = ThinkingSplitter::default();
        let (text, reasoning) = collect(&mut s, &["hello ", "world"]);
        assert_eq!(text, "hello world");
        assert!(reasoning.is_empty());
    }

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
                Chunk::Reasoning(_) => {}
            }
        }
        assert_eq!(text, "Hello");
        assert_eq!(done, Some(StopReason::Stop));
    }

    #[tokio::test]
    async fn literal_done_in_content_does_not_truncate_stream() {
        let server = MockServer::start().await;
        // A content delta legitimately contains the text "[DONE]"; only the bare
        // `data: [DONE]` sentinel line should end the stream.
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"see [DONE] here\"}}]}\n\n\
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
        let client = OpenAiCompatClient::new(server.uri(), "m".into(), None);
        let req = CompletionRequest { messages: vec![Message::user("hi")], ..Default::default() };
        let mut stream = client.stream(req).await.unwrap();
        let mut text = String::new();
        let mut done = None;
        while let Some(item) = stream.next().await {
            match item.unwrap() {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Done(r) => done = Some(r),
                _ => {}
            }
        }
        assert_eq!(text, "see [DONE] here");
        assert_eq!(done, Some(StopReason::Stop)); // finish_reason was reached, not skipped
    }

    #[tokio::test]
    async fn surfaces_http_error_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500)
                .set_body_string("{\"error\":\"n_cmpl cannot be greater than slots\"}"))
            .mount(&server)
            .await;
        let client = OpenAiCompatClient::new(server.uri(), "m".into(), None);
        let req = CompletionRequest {
            messages: vec![],
            ..Default::default()
        };
        let err = client.stream(req).await.err().unwrap();
        match err {
            ModelError::Status { code, body } => {
                assert_eq!(code, 500);
                assert!(body.contains("cannot be greater than slots"), "body was: {body}");
            }
            other => panic!("expected Status, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn streams_reasoning_content_separately() {
        let server = MockServer::start().await;
        let body = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking hard\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"the answer\"}}]}\n\n\
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
        let mut reasoning = String::new();
        let mut done = None;
        while let Some(item) = stream.next().await {
            match item.unwrap() {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Reasoning(r) => reasoning.push_str(&r),
                Chunk::Done(r) => done = Some(r),
                Chunk::ToolCallDelta(_) => {}
            }
        }
        assert_eq!(reasoning, "thinking hard");
        assert_eq!(text, "the answer");
        assert_eq!(done, Some(StopReason::Stop));
    }

    #[tokio::test]
    async fn skips_malformed_sse_line_and_keeps_streaming() {
        let server = MockServer::start().await;
        // A good delta, then a malformed data line, then another good delta + terminal.
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\n\n\
                    data: {bad json\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"B\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                    data: [DONE]\n\n";
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body))
            .mount(&server).await;
        let client = OpenAiCompatClient::new(server.uri(), "m".into(), None);
        let req = CompletionRequest { messages: vec![Message::user("hi")], ..Default::default() };
        let mut stream = client.stream(req).await.unwrap();

        let mut text = String::new();
        let mut done = None;
        while let Some(item) = stream.next().await {
            // unwrap() would panic if the bad line aborted the stream with an Err.
            match item.unwrap() {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Done(r) => done = Some(r),
                _ => {}
            }
        }
        assert_eq!(text, "AB", "the malformed line is skipped, both good deltas survive");
        assert_eq!(done, Some(StopReason::Stop));
    }

    #[tokio::test]
    async fn surfaces_in_band_error_object_in_200_body() {
        let server = MockServer::start().await;
        let body = "data: {\"error\":{\"message\":\"boom\"}}\n\n";
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body))
            .mount(&server).await;
        let client = OpenAiCompatClient::new(server.uri(), "m".into(), None);
        let req = CompletionRequest { messages: vec![Message::user("hi")], ..Default::default() };
        let mut stream = client.stream(req).await.unwrap();

        let mut err = None;
        while let Some(item) = stream.next().await {
            if let Err(e) = item { err = Some(e); break; }
        }
        match err {
            Some(ModelError::Stream(m)) => assert!(m.contains("boom"), "message was: {m}"),
            other => panic!("expected Stream error carrying the in-band message, got {other:?}"),
        }
    }

    #[test]
    fn splitter_flushes_unterminated_think() {
        let mut s = ThinkingSplitter::default();
        // Push an opening tag with content but no closing tag.
        let chunks_from_push = s.push("<think>partial reasoning");
        // The splitter may buffer the partial tag prefix; flush forces it out.
        let mut chunks: Vec<Chunk> = chunks_from_push;
        chunks.extend(s.flush());

        let mut text = String::new();
        let mut reasoning = String::new();
        for c in chunks {
            match c {
                Chunk::Text(t) => text.push_str(&t),
                Chunk::Reasoning(r) => reasoning.push_str(&r),
                _ => {}
            }
        }
        assert_eq!(reasoning, "partial reasoning");
        assert!(text.is_empty());
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
        // preserve_thinking rides alongside enable_thinking (Qwen3.6 keeps prior
        // reasoning only when this is true); default is false here.
        assert_eq!(b["chat_template_kwargs"]["preserve_thinking"], serde_json::json!(false));
        // Unset params are omitted entirely.
        assert!(b.get("min_p").is_none());
        assert!(b.get("presence_penalty").is_none());
        assert!(b.get("repeat_penalty").is_none());
    }
}
