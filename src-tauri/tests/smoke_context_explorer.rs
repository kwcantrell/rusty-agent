//! Live end-to-end smoke for the Context Explorer feature (L1, needs llama-server
//! on :8080). Drives the in-process bridge + Session exactly like the desktop app's
//! Tauri commands do, over a REAL model turn, and asserts the feature's data paths:
//!
//!   1. a live turn reaches `done` with no `error`;
//!   2. `ServerEvent::ServerUsage` is FORWARDED on the wire (the faithful token total
//!      the breakdown bar reconciles against — this was the Critical bug: it used to
//!      be dropped, so the UI only ever saw the estimate);
//!   3. `context_get` returns a populated snapshot (system + messages segments);
//!   4. `skill_save` → `skill_get` round-trips under the writable root.
//!
//! Run: `cd src-tauri && cargo test --test smoke_context_explorer -- --ignored --nocapture`
//! Requires `curl localhost:8080/health` == {"status":"ok"}.

use agent_server::session::SendOutcome;
use agent_server::wire::{EventOut, ServerEvent};
use rust_agent_runtime_desktop_lib::bridge;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Capture(Mutex<Vec<ServerEvent>>);

impl EventOut for Capture {
    fn send(&self, ev: ServerEvent) {
        self.0.lock().unwrap().push(ev);
    }
}

#[tokio::test]
#[ignore = "live: needs llama-server on :8080 (qwen3.6-35b-a3b)"]
async fn context_explorer_live_smoke() {
    let ws = tempfile::tempdir().unwrap();
    let cfg = ws.path().join("agent-runtime.json");

    let bridge = bridge::start(
        ws.path().to_path_buf(),
        cfg,
        "http://localhost:8080".into(),
        "qwen3.6-35b-a3b".into(),
    )
    .await
    .expect("bridge start");
    let session = bridge.session();

    // Capture the outbound event stream the way the webview's Channel does.
    let cap = Arc::new(Capture::default());
    session.set_event_out(cap.clone());

    // --- Drive one real turn (this is what the composer's Send does) ---
    assert!(
        matches!(
            session.send_input("Reply with exactly the single word: pong".into()),
            SendOutcome::Started
        ),
        "send_input should start a turn (got Busy)"
    );

    // Poll the event stream for `done` (success) or `error` (failure), up to 120s.
    let mut done = false;
    let mut saw_error: Option<String> = None;
    for _ in 0..240 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let evs = cap.0.lock().unwrap();
        for ev in evs.iter() {
            match ev {
                ServerEvent::Error { message } => saw_error = Some(message.clone()),
                ServerEvent::Done { .. } => done = true,
                _ => {}
            }
        }
        if done || saw_error.is_some() {
            break;
        }
    }
    if let Some(e) = saw_error {
        panic!("turn emitted an error event: {e}");
    }
    assert!(done, "turn did not reach `done` within 120s");

    // --- Assertion 2: ServerUsage (faithful total) was forwarded on the wire ---
    let (server_usage_seen, prompt_tokens, token_text) = {
        let evs = cap.0.lock().unwrap();
        let mut su = false;
        let mut pt = 0u32;
        let mut text = String::new();
        for ev in evs.iter() {
            match ev {
                ServerEvent::ServerUsage { prompt_tokens, .. } => {
                    su = true;
                    pt = *prompt_tokens;
                }
                ServerEvent::Token { text: t } => text.push_str(t),
                _ => {}
            }
        }
        (su, pt, text)
    };
    assert!(
        server_usage_seen,
        "no ServerUsage event on the wire — the faithful-total path is broken (Critical regression)"
    );
    println!("[smoke] live turn ok · server_usage.prompt_tokens={prompt_tokens} · reply={token_text:?}");

    // --- Assertion 3: context_get returns a populated snapshot ---
    let snap = session.context_get().await;
    let cats: Vec<&str> = snap.segments.iter().map(|s| s.category.as_str()).collect();
    assert!(cats.contains(&"system"), "snapshot missing system segment: {cats:?}");
    assert!(cats.contains(&"messages"), "snapshot missing messages segment: {cats:?}");
    assert!(snap.model_limit > 0, "model_limit should be positive");
    let msg_count = snap
        .segments
        .iter()
        .find(|s| s.category == "messages")
        .map(|s| s.count)
        .unwrap_or(0);
    assert!(msg_count >= 1, "expected >=1 message after a turn, got {msg_count}");
    println!(
        "[smoke] context_get ok · est_total={} model_limit={} segments={:?} messages={}",
        snap.est_total, snap.model_limit, cats, msg_count
    );

    // --- Assertion 4: skill save/get round-trips under the writable root ---
    session
        .skill_save("smoke-skill".into(), "Smoke body: explorer end-to-end.".into())
        .await
        .expect("skill_save");
    let got = session.skill_get("smoke-skill".into()).await.expect("skill_get");
    assert_eq!(got.name, "smoke-skill");
    assert!(got.body.contains("Smoke body"), "skill body not persisted: {:?}", got.body);
    println!("[smoke] skill round-trip ok · {} -> {:?}", got.name, got.body);

    println!("[smoke] ALL CONTEXT-EXPLORER SMOKE CHECKS PASSED");
}
