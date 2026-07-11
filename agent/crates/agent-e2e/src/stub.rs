//! Scripted OpenAI-compatible model stub (wiremock) + a raw mid-stream-drop
//! stub, both used by the e2e lifecycle/stress scenarios to drive a real
//! `Session` through a real `OpenAiCompatClient` without a live model.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub enum StubResponse {
    ToolCall {
        name: String,
        args: serde_json::Value,
    },
    Text(String),
    MalformedJson,
    BogusTool,
    DelayedText {
        text: String,
        delay_ms: u64,
    },
}

pub struct ScriptStep {
    /// Must appear in the raw request body (e.g. the task text, or deny feedback).
    pub expect_substring: Option<String>,
    pub respond: StubResponse,
}

#[derive(Default)]
struct StubState {
    cursor: usize,
    recorded: Vec<String>,
    /// First protocol violation (stray request / matcher miss); poisons the test.
    poison: Option<String>,
}

pub struct ScriptedStub {
    server: MockServer,
    steps_len: usize,
    state: Arc<Mutex<StubState>>,
    checked: AtomicBool,
}

fn sse_text(text: &str) -> String {
    let chunk = serde_json::json!({"choices":[{"delta":{"content": text}}]});
    format!(
        "data: {chunk}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
    )
}

fn sse_tool_call(name: &str, args: &serde_json::Value) -> String {
    // One-shot tool_call delta then finish_reason=tool_calls. Field layout
    // (index/id/type/function.name/function.arguments, arguments as a JSON
    // *string*) mirrors exactly what agent-model's
    // `openai.rs::parse_sse_line` reads: `c["index"]`, `c["id"]`,
    // `c["function"]["name"]`, `c["function"]["arguments"]` — verified
    // against that parser (and its `NativeProtocol::parse` consumer in
    // `protocol.rs`, which `serde_json::from_str`s `args_fragment`).
    let call = serde_json::json!({"choices":[{"delta":{"tool_calls":[{
        "index":0,"id":"call_e2e_1","type":"function",
        "function":{"name":name,"arguments":args.to_string()}
    }]}}]});
    format!(
        "data: {call}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n"
    )
}

struct ScriptResponder {
    steps: Vec<(Option<String>, StubResponse)>,
    state: Arc<Mutex<StubState>>,
}

impl Respond for ScriptResponder {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&req.body).into_owned();
        let mut st = self.state.lock().unwrap();
        st.recorded.push(body.clone());
        let i = st.cursor;
        let Some((expect, resp)) = self.steps.get(i) else {
            st.poison = Some(format!("stray request past script end: {body:.200}"));
            return ResponseTemplate::new(500).set_body_string("E2E-STUB-STRAY");
        };
        if let Some(needle) = expect {
            if !body.contains(needle.as_str()) {
                st.poison = Some(format!("step {i}: body missing {needle:?}"));
                return ResponseTemplate::new(500).set_body_string("E2E-STUB-MISMATCH");
            }
        }
        st.cursor += 1;
        let sse = |b: String| {
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(b)
        };
        match resp {
            StubResponse::Text(t) => sse(sse_text(t)),
            StubResponse::ToolCall { name, args } => sse(sse_tool_call(name, args)),
            StubResponse::BogusTool => {
                sse(sse_tool_call("no_such_tool_e2e", &serde_json::json!({})))
            }
            StubResponse::MalformedJson => sse("data: {not json}\n\ndata: [DONE]\n\n".into()),
            StubResponse::DelayedText { text, delay_ms } => {
                sse(sse_text(text)).set_delay(std::time::Duration::from_millis(*delay_ms))
            }
        }
    }
}

impl ScriptedStub {
    pub async fn start(steps: Vec<ScriptStep>) -> Self {
        let steps_len = steps.len();
        let state = Arc::new(Mutex::new(StubState::default()));
        let responder = ScriptResponder {
            steps: steps
                .into_iter()
                .map(|s| (s.expect_substring, s.respond))
                .collect(),
            state: state.clone(),
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(responder)
            .mount(&server)
            .await;
        ScriptedStub {
            server,
            steps_len,
            state,
            checked: AtomicBool::new(false),
        }
    }

    pub fn base_url(&self) -> String {
        self.server.uri()
    }

    pub fn recorded(&self) -> Vec<String> {
        self.state.lock().unwrap().recorded.clone()
    }

    /// Call at the end of every test that used the stub.
    pub fn assert_consumed(&self) {
        self.checked.store(true, Ordering::SeqCst);
        let st = self.state.lock().unwrap();
        assert!(
            st.poison.is_none(),
            "stub poisoned: {}",
            st.poison.as_deref().unwrap()
        );
        assert_eq!(st.cursor, self.steps_len, "script not fully consumed");
    }
}

impl Drop for ScriptedStub {
    fn drop(&mut self) {
        // Only panic if: (1) assert_consumed() was never called, (2) poison exists,
        // and (3) we're not already unwinding (don't panic during unwind).
        if !self.checked.load(Ordering::SeqCst) {
            let st = self.state.lock().unwrap();
            if let Some(ref poison_msg) = st.poison {
                if !std::thread::panicking() {
                    panic!(
                        "ScriptedStub dropped with poison but assert_consumed() was never called. \
                         Poison: {}. Call assert_consumed() at test end.",
                        poison_msg
                    );
                }
            }
        }
    }
}

/// The standard approval-gated step: write_file is Access::Write ⇒ Ask.
pub fn gated_write(expect: &str) -> ScriptStep {
    ScriptStep {
        expect_substring: Some(expect.into()),
        respond: StubResponse::ToolCall {
            name: "write_file".into(),
            args: serde_json::json!({"path": "out.txt", "content": "e2e"}),
        },
    }
}

pub fn text_step(expect: Option<&str>, reply: &str) -> ScriptStep {
    ScriptStep {
        expect_substring: expect.map(Into::into),
        respond: StubResponse::Text(reply.into()),
    }
}

pub struct RawDropStub {
    addr: std::net::SocketAddr,
}

impl RawDropStub {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut first = true;
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let drop_this = std::mem::replace(&mut first, false);
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 65536];
                    let _ = sock.read(&mut buf).await; // consume request head
                    let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
                    let _ = sock.write_all(head.as_bytes()).await;
                    if drop_this {
                        // one partial chunk, then hard close mid-stream
                        let _ = sock
                            .write_all(
                                b"data: {\"choices\":[{\"delta\":{\"content\":\"par\"}}]}\n\n",
                            )
                            .await;
                        let _ = sock.shutdown().await;
                    } else {
                        let _ = sock.write_all(sse_text("recovered").as_bytes()).await;
                        let _ = sock.shutdown().await;
                    }
                });
            }
        });
        RawDropStub { addr }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn scripted_stub_matches_and_records() {
        let stub = ScriptedStub::start(vec![text_step(Some("ping"), "pong")]).await;
        // Drive through a Rig session so the real OpenAiCompatClient parses our SSE.
        let rig = crate::rig::Rig::new();
        let (session, cap) = rig.session(&stub.base_url());
        assert!(matches!(
            session.send_input("ping".into()),
            agent_server::session::SendOutcome::Started
        ));
        assert!(
            crate::rig::wait_until_async(std::time::Duration::from_secs(30), || {
                cap.snapshot()
                    .iter()
                    .any(|e| matches!(e, agent_server::wire::ServerEvent::Done { .. }))
            })
            .await
        );
        assert!(stub.recorded()[0].contains("ping"));
        stub.assert_consumed();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gated_write_flows_through_approval_request_to_done() {
        use agent_server::wire::Decision;

        let stub =
            ScriptedStub::start(vec![gated_write("SQRL-GATE"), text_step(None, "done")]).await;
        let rig = crate::rig::Rig::new();
        let (session, cap) = rig.session(&stub.base_url());
        assert!(matches!(
            session.send_input("SQRL-GATE please write the file".into()),
            agent_server::session::SendOutcome::Started
        ));

        // write_file is Access::Write ⇒ Ask: the tool call must park behind an
        // ApprovalRequest instead of executing immediately.
        let ask =
            agent_server::testkit::wait_for_ask_id(&cap, std::time::Duration::from_secs(30)).await;

        // The park must also be durable on disk before we answer it.
        assert!(
            crate::rig::wait_until_async(std::time::Duration::from_secs(30), || {
                crate::rig::ckpt(&rig.only_session_dir())
                    .join("parked.json")
                    .exists()
            })
            .await,
            "parked.json did not appear under the session checkpoint dir"
        );

        session.approve(&ask, Decision::Approve);

        assert!(
            crate::rig::wait_until_async(std::time::Duration::from_secs(30), || {
                cap.snapshot()
                    .iter()
                    .any(|e| matches!(e, agent_server::wire::ServerEvent::Done { .. }))
            })
            .await,
            "run did not reach Done after approving the gated write"
        );
        stub.assert_consumed();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn poison_enforced_at_drop_when_assert_consumed_skipped() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Start a stub with empty script so any request poisons it.
        let stub = ScriptedStub::start(vec![]).await;
        let base_url = stub.base_url();

        // Send a raw HTTP request to poison the stub (no reqwest dep needed).
        tokio::spawn(async move {
            if let Ok(mut stream) =
                tokio::net::TcpStream::connect(base_url.replace("http://", "")).await
            {
                let req =
                    "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\n{}";
                let _ = stream.write_all(req.as_bytes()).await;
                let mut buf = [0u8; 256];
                let _ = stream.read(&mut buf).await;
            }
        });

        // Give the request time to arrive and poison the stub.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Now drop the stub WITHOUT calling assert_consumed() and verify the panic.
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(stub)));
        assert!(
            panic_result.is_err(),
            "expected panic when dropping poisoned stub without assert_consumed()"
        );

        // Verify the panic message mentions both "stray request" (the poison) and "assert_consumed".
        if let Err(e) = panic_result {
            if let Some(msg) = e.downcast_ref::<String>() {
                assert!(
                    msg.contains("stray request") || msg.contains("Poison"),
                    "panic message should contain poison details: {}",
                    msg
                );
                assert!(
                    msg.contains("assert_consumed"),
                    "panic message should mention assert_consumed(): {}",
                    msg
                );
            }
        }
    }
}
