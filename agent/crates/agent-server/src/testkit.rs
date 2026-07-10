//! Test doubles and rig-building helpers for driving `Session` deterministically —
//! lifted out of `session.rs`'s `#[cfg(test)] mod tests` so an external
//! tests-only crate can build the same "prior parked session" rigs this
//! crate's own unit tests use (see `session.rs`'s park/resume/attach tests).
use crate::session::Session;
use crate::wire::{EventOut, ServerEvent};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Captures every `ServerEvent` sent through an `EventOut` sink, in order.
#[derive(Default)]
pub struct Captured(pub Mutex<Vec<ServerEvent>>);
impl EventOut for Captured {
    fn send(&self, ev: ServerEvent) {
        self.0.lock().unwrap().push(ev);
    }
}
impl Captured {
    /// A point-in-time copy of everything captured so far — lets callers
    /// outside this crate assert on events without reaching into the
    /// `Mutex`'s field layout.
    pub fn snapshot(&self) -> Vec<ServerEvent> {
        self.0.lock().unwrap().clone()
    }
}

/// Plants a prior parked session (mirrors
/// `approving_a_parked_ask_emits_resumed_before_the_resumed_runs_first_event`'s
/// rig) and returns everything a caller needs to attach, answer, and poll.
/// The planted tool call (`execute_command echo real`) is pre-approved via
/// `parked_index: Some(0)`, so once answered the resumed run executes it
/// then continues into `turn_loop`, which calls the model at
/// `http://127.0.0.1:1` (port 1 is reserved/unlistenable — deterministic,
/// instant connection-refused, unlike :8080 which this dev machine may
/// have a real llama-server bound to), so the model call fails fast and
/// deterministically with `AgentError::Model`, driving `start_resume`'s
/// `Err` arm without any extra sabotage.
///
/// `meta` is the metadata root the planted checkpoint's secret is keyed
/// against, and is also set as the constructed `Session`'s `metadata_dir`
/// override (E1) — so rig-rooted verification (a tempdir standing in for
/// `~/.rusty-agent`) actually round-trips instead of silently keying
/// against the real developer home.
pub async fn plant_parked_session(
    ws: &Path,
    sessions: &Path,
    meta: &Path,
    prior_id: &str,
) -> (Arc<Session>, Arc<Captured>, [u8; 32]) {
    plant_parked_session_with_command(ws, sessions, meta, prior_id, "echo real").await
}

/// `plant_parked_session`, parameterized on the planted command — the
/// cancelled-resume test (C-1) needs a command with a real execution
/// window (`sleep N`) instead of an instant `echo`, so cancellation has
/// somewhere deterministic to land before the doomed model call.
pub async fn plant_parked_session_with_command(
    ws: &Path,
    sessions: &Path,
    meta: &Path,
    prior_id: &str,
    command: &str,
) -> (Arc<Session>, Arc<Captured>, [u8; 32]) {
    use agent_core::checkpoint::{Checkpoint, Checkpointer, Guardrails, ParkedTurn};
    use agent_model::Message;
    use agent_policy::ApprovalOrigin;

    let key = agent_runtime_config::load_or_create_secret(meta).expect("secret");

    agent_runtime_config::write_descriptor(
        sessions,
        &agent_runtime_config::SessionDescriptor {
            schema: agent_runtime_config::DESCRIPTOR_SCHEMA,
            session_id: prior_id.into(),
            workspace: ws.to_path_buf(),
            created_ms: 1,
            config_path: None,
        },
    )
    .unwrap();
    let prior_ck = agent_runtime_config::session_dir(sessions, prior_id).join("checkpoint");
    let ckr = Checkpointer::new(prior_ck.clone(), key, prior_id.into());
    let planted_args = serde_json::json!({"command": command});
    let origin = ApprovalOrigin {
        delegation_id: "c9".into(),
        subagent_name: "explore".into(),
        depth: 1,
    };
    let planted_call = agent_tools::ToolCall {
        id: "c9".into(),
        name: "execute_command".into(),
        args: planted_args.clone(),
    };
    let chk = Checkpoint {
        version: agent_core::checkpoint::CHECKPOINT_VERSION,
        session_id: prior_id.into(),
        subagent_path: vec![],
        turn: 0,
        context: agent_core::CuratedContextState {
            goal: None,
            // The assistant message carrying the pending tool_calls must
            // already be in history (tool_phase only appends the Role::Tool
            // result on execution) — otherwise CuratedContext::build's
            // orphaned-tool-message debug_assert trips once the resumed run
            // actually executes the planted call.
            history: vec![
                Message::user("hi"),
                Message::assistant("running", Some(vec![planted_call.clone()])),
            ],
            compaction_summary: None,
            folded_facts: vec![],
            folded_sections: vec![],
            seq: 0,
            history_has_spans: false,
            history_incomplete: false,
            artifact_prefix: String::new(),
            todos: vec![],
        },
        guardrails: Guardrails {
            tool_calls: 0,
            model_calls: 0,
        },
        parked: ParkedTurn {
            assistant_text: "running".into(),
            tool_calls: vec![planted_call],
            invalid: vec![],
            gate_records: vec![],
            parked_index: Some(0),
            origin: Some(origin.clone()),
        },
    };
    ckr.write_park(chk, &agent_core::SessionArtifacts::new())
        .await
        .unwrap();

    // Port 1 is reserved/unlistenable — a connection to it refuses
    // instantly and deterministically, unlike :8080 which this machine's
    // dev environment may have a real model server bound to (see
    // memory: local-llama-server). That refusal is what drives
    // `resume_with_cancel`'s Err(AgentError::Model) path once the
    // resumed run's tool_phase finishes and turn_loop calls the model.
    let mut params = crate::setup::local_params(
        ws.to_path_buf(),
        ws.join("rt.json"),
        "http://127.0.0.1:1".into(),
        "m".into(),
    );
    params.config.trace_dir = Some(sessions.to_string_lossy().into_owned());
    params.config.metadata_dir = Some(meta.to_string_lossy().into_owned());
    let sess = Session::from_params(params);

    let cap = Arc::new(Captured::default());
    sess.set_event_out(cap.clone());

    (sess, cap, key)
}

/// Polls `cap` until an `ApprovalRequest` event lands, returning its id.
pub async fn wait_for_ask_id(cap: &Captured, timeout: Duration) -> String {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let found = cap.0.lock().unwrap().iter().find_map(|ev| match ev {
            ServerEvent::ApprovalRequest { id, .. } => Some(id.clone()),
            _ => None,
        });
        if let Some(id) = found {
            return id;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no ApprovalRequest re-emitted from the parked prior session; captured: {:#?}",
            cap.0.lock().unwrap()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
