//! App-lifetime session state for the Tauri IPC transport. Replaces the
//! per-connection state that `daemon::serve()` used to own.
use crate::approval::IpcApprovalChannel;
use crate::daemon::DaemonParams;
use crate::runtime::RuntimeState;
use crate::sink::{ChannelEventSink, EventSlot};
use crate::wire::{ArchitectureSnapshot, Decision, EventOut, SettingsState};
use agent_core::checkpoint;
use agent_core::{ContextManager, CuratedContext};
use agent_model::Message;
use agent_runtime_config::RuntimeConfig;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

/// DTO returned by `skill_get` / sent over Tauri IPC to the frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillDto {
    pub name: String,
    pub description: String,
    pub body: String,
    pub files: Vec<String>,
}

pub enum SendOutcome {
    Started,
    Busy,
}

pub struct Session {
    runtime: Arc<RuntimeState>,
    ctx: Arc<AsyncMutex<CuratedContext>>,
    slot: EventSlot,
    approval: Arc<IpcApprovalChannel>,
    active: Arc<Mutex<Option<CancellationToken>>>,
    memory_index_budget: usize,
    /// The live workspace for this session. `set_workspace` updates only this copy
    /// (so memory scope + skills follow the current workspace); it intentionally does
    /// NOT touch `RuntimeState`'s own workspace, which the run loop owns. Do not "sync"
    /// them — the divergence is by design.
    workspace: Mutex<PathBuf>,
    /// Session ids whose parked tree already has a resume in flight or done
    /// this daemon lifetime (first answer wins; spec §2.4).
    resuming: Arc<Mutex<HashSet<String>>>,
}

impl Session {
    pub fn from_params(params: DaemonParams) -> Arc<Self> {
        let slot: EventSlot = Arc::new(Mutex::new(None));
        let sink = Arc::new(ChannelEventSink::new(slot.clone()));
        let config = RuntimeConfig::load_over(params.config.clone(), &params.config_path);
        // E5: no configured window ⇒ an unanswered ask parks indefinitely; the
        // `approval_auto_deny_secs` knob opts headless/eval callers into auto-deny.
        let approval = Arc::new(IpcApprovalChannel::new(
            slot.clone(),
            config.approval_auto_deny_secs.map(Duration::from_secs),
        ));
        let max_tool_result_bytes = config.max_tool_result_bytes;
        let runtime = Arc::new(RuntimeState::new(
            config,
            sink,
            approval.clone(),
            params.workspace.clone(),
            params.api_key.clone(),
            params.claude_binary.clone(),
            params.config_path.clone(),
            params.mcp_tools.clone(),
            params.system_prompt.clone(),
        ));
        let ctx = Arc::new(AsyncMutex::new(
            CuratedContext::new(
                Message::system(params.system_prompt.clone()),
                runtime.artifacts(),
                runtime.compact_flag(),
            )
            .with_offload_config(agent_core::OffloadConfig {
                max_result_bytes: max_tool_result_bytes,
                ..Default::default()
            })
            .with_todos(runtime.todos()),
        ));
        Arc::new(Self {
            runtime,
            ctx,
            slot,
            approval,
            active: Arc::new(Mutex::new(None)),
            memory_index_budget: agent_core::DEFAULT_MEMORY_INDEX_BUDGET,
            workspace: Mutex::new(params.workspace),
            resuming: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    /// Register/replace the outbound channel (the `subscribe` command body).
    pub fn set_event_out(self: &Arc<Self>, out: Arc<dyn EventOut>) {
        *self.slot.lock().unwrap() = Some(out.clone());
        // Daemon-alive reattach (spec §2.4 step 5): pending asks re-emit under
        // their LIVE ids — no duplicate is minted.
        self.approval.reemit_pending(&out);
        // Restart-path reattach (spec §2.4 steps 1–5): index PRIOR sessions'
        // parked trees from disk and re-emit their asks with re-derived
        // displays + attribution.
        self.spawn_parked_reemit();
    }

    /// Emit a wire `Error` frame on the current outbound channel (no-op if
    /// none is attached — the resume driver's failures are surfaced here).
    fn emit_error(&self, message: String) {
        if let Some(out) = self.slot.lock().unwrap().clone() {
            out.send(crate::wire::ServerEvent::Error { message });
        }
    }

    /// Re-emit every parked ask from PRIOR sessions (spec §2.4 steps 1–5).
    ///
    /// Defense in depth: this must never abort a caller that has no Tokio
    /// reactor in scope (e.g. a sync Tauri command running on the glib main
    /// thread — `tokio::spawn` there panics non-unwinding across the C-FFI
    /// boundary and takes the whole app down). The `subscribe` command is
    /// `async` in production, which makes this branch unreachable there; the
    /// guard turns a future regression from an abort into a logged
    /// degradation instead.
    fn spawn_parked_reemit(self: &Arc<Self>) {
        if tokio::runtime::Handle::try_current().is_err() {
            tracing::warn!(
                target: "session",
                "no async runtime at attach; parked-run re-emit skipped"
            );
            return;
        }
        let sess = self.clone();
        tokio::spawn(async move {
            let cfg = sess.runtime.settings_state().settings;
            let Some(root) = agent_runtime_config::sessions_root(&cfg) else {
                return;
            };
            let Some(meta) = agent_runtime_config::metadata_root() else {
                return;
            };
            let Ok(key) = agent_runtime_config::load_or_create_secret(&meta) else {
                return;
            };
            for parked in crate::resume::scan_parked(&root, &key, sess.runtime.session_id()) {
                for err in &parked.errors {
                    sess.emit_error(format!(
                        "session {}: checkpoint unreadable; run cannot be resumed ({err})",
                        parked.descriptor.session_id
                    ));
                }
                if !parked.errors.is_empty() {
                    continue;
                }
                if !parked.descriptor.workspace.is_dir() {
                    sess.emit_error(format!(
                        "session {} is parked but its workspace {} is missing; cannot resume",
                        parked.descriptor.session_id,
                        parked.descriptor.workspace.display()
                    ));
                    continue;
                }
                sess.clone().wire_parked_session(parked, key);
            }
        });
    }

    /// Re-derive displays via a resume-built loop, register externals, and race
    /// their answers (spec §2.4 steps 2–4).
    fn wire_parked_session(self: Arc<Self>, parked: crate::resume::ParkedSession, key: [u8; 32]) {
        let Some(root_chk) = parked.root.clone() else {
            self.emit_error(format!(
                "session {}: parked tree has no root checkpoint; cannot resume",
                parked.descriptor.session_id
            ));
            return;
        };
        // One assembled loop serves BOTH display re-derivation and the actual
        // resume — built against the descriptor workspace + current config
        // (spec §3.3).
        let artifacts = Arc::new(agent_core::SessionArtifacts::new());
        let todos: agent_core::TodoHandle = Arc::new(Mutex::new(Vec::new()));
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ck = agent_core::Checkpointer::new(
            parked.root_dir.clone(),
            key,
            parked.descriptor.session_id.clone(),
        );
        let built = self.runtime.build_resume_loop(
            &parked.descriptor.workspace,
            ck,
            &artifacts,
            &todos,
            &flag,
        );
        let sid = parked.descriptor.session_id.clone();
        if self.resuming.lock().unwrap().contains(&sid) {
            // Already driving (or already attempted and failed) this tree this
            // daemon lifetime — branch review I1b: registering externals here
            // would show the user a prompt whose answer can never reach a
            // resume, since `start_resume`'s sticky guard would just bounce it.
            tracing::debug!(
                target: "session",
                sid = %sid,
                "skipping parked re-emit: session already in resuming set"
            );
            return;
        }
        for ask in parked.asks {
            if ask.answered {
                // Crash-after-commit window: resume directly, no re-prompt.
                self.clone().start_resume(
                    &sid,
                    built.loop_.clone(),
                    built.system_prompt.clone(),
                    root_chk.clone(),
                    artifacts.clone(),
                    todos.clone(),
                    flag.clone(),
                    key,
                    parked.root_dir.clone(),
                );
                continue;
            }
            let idx = ask.checkpoint.parked.parked_index.expect("gate-kind");
            let call = &ask.checkpoint.parked.tool_calls[idx];
            // Display integrity (spec §2.4 step 4 / §3.4): re-derive from stored
            // args via tool.intent; NEVER emit persisted text.
            let Some(intent) = built.loop_.derive_intent(&call.name, &call.args) else {
                self.emit_error(format!(
                    "session {sid}: parked tool {} unavailable under current config; \
                     answer it after restoring the tool or start a new run",
                    call.name
                ));
                continue;
            };
            let (_, rx) = self.approval.register_external(
                &sid,
                crate::approval::ExternalAsk {
                    summary: intent.summary.clone(),
                    command: intent.command.clone(),
                    display: None,
                    origin: ask.origin.as_ref().map(|o| crate::wire::ApprovalOriginDto {
                        delegation_id: o.delegation_id.clone(),
                        subagent: o.subagent_name.clone(),
                        depth: o.depth,
                    }),
                },
            );
            let sess = self.clone();
            let (loop_, sp) = (built.loop_.clone(), built.system_prompt.clone());
            let (arts, tds, flg, rc, rd) = (
                artifacts.clone(),
                todos.clone(),
                flag.clone(),
                root_chk.clone(),
                parked.root_dir.clone(),
            );
            let sid2 = sid.clone();
            let ask_dir = ask.dir.clone();
            tokio::spawn(async move {
                let Ok(resp) = rx.await else {
                    return;
                };
                // Durable answer commit (header note 3). E2: ApproveAlways is
                // committed as a plain one-shot approve; a Deny's feedback
                // text threads through so the resumed loop can render it.
                let decision = match resp {
                    agent_policy::ApprovalResponse::Approve
                    | agent_policy::ApprovalResponse::ApproveAlways => checkpoint::ParkedAnswer {
                        approve: true,
                        feedback: None,
                    },
                    agent_policy::ApprovalResponse::Deny { feedback } => checkpoint::ParkedAnswer {
                        approve: false,
                        feedback,
                    },
                };
                if let Err(e) = checkpoint::write_answer(
                    &ask_dir,
                    &key,
                    decision.approve,
                    decision.feedback.as_deref(),
                ) {
                    sess.emit_error(format!("cannot commit answer: {e}"));
                    return;
                }
                sess.start_resume(&sid2, loop_, sp, rc, arts, tds, flg, key, rd);
            });
        }
    }

    /// Restore artifacts + context from the ROOT checkpoint and resume the
    /// tree, once per session id (`resuming` guard; spec §2.4). Busy-guarded via
    /// the session's `active` slot (busy rule A1 applies to resumes too).
    #[allow(clippy::too_many_arguments)]
    fn start_resume(
        self: Arc<Self>,
        sid: &str,
        loop_: Arc<agent_core::AgentLoop>,
        system_prompt: String,
        root_chk: agent_core::Checkpoint,
        artifacts: Arc<agent_core::SessionArtifacts>,
        todos: agent_core::TodoHandle,
        flag: Arc<std::sync::atomic::AtomicBool>,
        key: [u8; 32],
        root_dir: PathBuf,
    ) {
        if !self.resuming.lock().unwrap().insert(sid.to_string()) {
            // First answer already driving this tree (or a prior resume attempt
            // already failed and left the sticky guard set — branch review I1a):
            // surface it instead of bouncing silently, since a caller retrying
            // after a failed resume would otherwise see nothing happen.
            self.emit_error(format!(
                "session {sid}: resume already in progress or attempted; restart the daemon to retry a failed resume"
            ));
            return;
        }
        {
            let mut active = self.active.lock().unwrap();
            if active.is_some() {
                self.emit_error(format!(
                    "session {sid} answered but a run is active; reattach after it finishes"
                ));
                self.resuming.lock().unwrap().remove(sid);
                return;
            }
            *active = Some(CancellationToken::new());
        }
        let cancel = self.active.lock().unwrap().as_ref().unwrap().clone();
        // First answer wins for THIS tree: retract our other re-emitted
        // externals so a stale prompt cannot mint a second resume or an
        // orphaned answer.json (plan review finding 12). The resumed tree
        // re-asks any still-parked child live under a fresh id.
        self.approval.retract_external_for(sid);
        let sess = self;
        let sid = sid.to_string();
        tokio::spawn(async move {
            if let Ok(dump) = checkpoint::load_artifact_dump(&root_dir, &key) {
                checkpoint::restore_artifacts(&artifacts, &dump).await;
            }
            let root_answer = checkpoint::take_answer(&root_dir, &key);
            let mut ctx = CuratedContext::restore(
                Message::system(system_prompt),
                artifacts,
                flag,
                todos,
                root_chk.context.clone(),
            );
            let resume = root_chk.resume_turn(root_answer);
            match loop_.resume_with_cancel(&mut ctx, resume, cancel).await {
                Ok(()) => {
                    // Completed tree: delete-on-completion (spec §2.3; parked
                    // children were consumed en route).
                    let _ = std::fs::remove_dir_all(&root_dir);
                }
                Err(e) => {
                    // Spec §4: surface as a normal run error on the attached
                    // frontend; the PARK IS RETAINED so a later attach can retry
                    // (plan review BLOCKER 1b — never destroy the checkpoint on a
                    // failed resume).
                    sess.emit_error(format!("session {sid}: resumed run failed: {e}"));
                }
            }
            *sess.active.lock().unwrap() = None;
        });
    }

    /// Start a run unless one is active (A1 guard).
    pub fn send_input(self: &Arc<Self>, text: String) -> SendOutcome {
        let mut active = self.active.lock().unwrap();
        if active.is_some() {
            return SendOutcome::Busy;
        }
        let cancel = CancellationToken::new();
        *active = Some(cancel.clone());
        drop(active);

        let agent = self.runtime.current_loop();
        let system_prompt = self.runtime.current_system_prompt();
        let ctx = self.ctx.clone();
        let active_slot = self.active.clone();
        let runtime = self.runtime.clone();
        let slot = self.slot.clone();
        tokio::spawn(async move {
            {
                let mut guard = ctx.lock().await;
                guard.set_system(Message::system(system_prompt));
                if let Err(e) = agent.run_with_cancel(&mut *guard, text, cancel).await {
                    tracing::error!(error=%e, "run failed");
                }
            }
            // Push a final stats snapshot so an attached client needs no poll.
            let snapshot = runtime
                .stats()
                .read()
                .map(|s| s.clone())
                .unwrap_or_default();
            if let Some(out) = slot.lock().unwrap().clone() {
                out.send(crate::wire::ServerEvent::SessionStats { stats: snapshot });
            }
            *active_slot.lock().unwrap() = None;
        });
        SendOutcome::Started
    }

    /// Trip the active run's token (the B3 interactive cancel). No-op when idle.
    pub fn cancel(&self) {
        if let Some(tok) = self.active.lock().unwrap().as_ref() {
            tok.cancel();
        }
    }

    pub fn approve(&self, id: &str, decision: Decision) {
        self.approval.resolve(id, decision.into());
    }

    pub fn settings_get(&self) -> SettingsState {
        self.runtime.settings_state()
    }

    /// Read-only architecture self-portrait (the `architecture_get` command).
    pub fn architecture(&self) -> ArchitectureSnapshot {
        self.runtime.architecture(self.memory_index_budget)
    }

    /// Snapshot of the cumulative per-session counters (the `session_stats` query).
    pub fn session_stats(&self) -> agent_core::SessionStats {
        self.runtime
            .stats()
            .read()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    pub async fn context_get(&self) -> agent_core::ContextSnapshot {
        let model_limit = self.runtime.settings_state().settings.context_limit;
        let guard = self.ctx.lock().await;
        guard.snapshot(model_limit, 0)
    }

    pub fn settings_update(&self, cfg: RuntimeConfig) -> Result<SettingsState, String> {
        self.runtime.apply(cfg)?;
        Ok(self.runtime.settings_state())
    }

    /// Build a fresh `SkillRegistry` from the current config + workspace.
    /// The workspace lock is released before returning.
    fn skill_registry(&self) -> agent_skills::SkillRegistry {
        let workspace = self.workspace.lock().unwrap().clone();
        let cfg = self.runtime.settings_state().settings;
        agent_skills::SkillRegistry::from_config(&cfg.skills_dirs, &workspace)
    }

    pub async fn skill_get(&self, name: String) -> Result<SkillDto, String> {
        let reg = self.skill_registry();
        // Normalize to a slug for lookup (ignore errors), then fall through to the raw
        // name so non-slug callers still resolve.
        let slug = agent_skills::sanitize_slug(&name).ok();
        let s = slug
            .as_deref()
            .and_then(|sl| reg.find(sl))
            .or_else(|| reg.find(&name))
            .ok_or_else(|| format!("skill not found: {name}"))?;
        Ok(SkillDto {
            name: s.name,
            description: s.description,
            body: s.body,
            files: s
                .files
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        })
    }

    pub async fn skill_save(&self, name: String, body: String) -> Result<(), String> {
        let slug = agent_skills::sanitize_slug(&name)?;
        let reg = self.skill_registry();
        let dir = reg.writable_root().join(&slug);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // Preserve an existing description: try the original name first (consistent
        // with skill_get), then the slug, then fall back to a generated default.
        let desc = reg
            .find(&name)
            .or_else(|| reg.find(&slug))
            .map(|s| s.description)
            .unwrap_or_else(|| format!("{slug} skill"));
        let desc = desc.replace('\n', " "); // frontmatter is single-line; harden interpolation
        let md = format!("---\nname: {slug}\ndescription: {desc}\n---\n{body}\n");
        std::fs::write(dir.join("SKILL.md"), md).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Cancel any run, then reset the conversation context (workspace switch).
    pub async fn set_workspace(self: &Arc<Self>, dir: PathBuf) {
        self.cancel();
        self.runtime.rewrite_descriptor_workspace(&dir);
        *self.workspace.lock().unwrap() = dir;
        let mut guard = self.ctx.lock().await;
        *guard = CuratedContext::new(
            Message::system(self.runtime.current_system_prompt()),
            self.runtime.artifacts(),
            self.runtime.compact_flag(),
        )
        .with_offload_config(agent_core::OffloadConfig {
            max_result_bytes: self.runtime.settings_state().settings.max_tool_result_bytes,
            ..Default::default()
        })
        .with_todos(self.runtime.todos());
    }

    #[cfg(test)]
    fn mark_active_for_test(&self) {
        *self.active.lock().unwrap() = Some(CancellationToken::new());
    }

    /// Durable session identity (4B-0), delegated from `RuntimeState`. Exposed
    /// `pub(crate)` for tests that need to locate this session's descriptor.
    #[cfg(test)]
    pub(crate) fn session_id(&self) -> &str {
        self.runtime.session_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{EventOut, ServerEvent};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Captured(Mutex<Vec<ServerEvent>>);
    impl EventOut for Captured {
        fn send(&self, ev: ServerEvent) {
            self.0.lock().unwrap().push(ev);
        }
    }

    fn session_with_scripted() -> (Arc<Session>, Arc<Captured>) {
        let dir = tempfile::tempdir().unwrap();
        let params = crate::setup::local_params(
            dir.path().to_path_buf(),
            dir.path().join("rt.json"),
            "http://localhost:8080".into(),
            "m".into(),
        );
        let sess = Session::from_params(params);
        let cap = Arc::new(Captured::default());
        sess.set_event_out(cap.clone());
        std::mem::forget(dir); // keep temp dir alive for the test process
        (sess, cap)
    }

    #[tokio::test]
    async fn second_input_during_run_is_busy() {
        let (sess, _cap) = session_with_scripted();
        sess.mark_active_for_test();
        assert!(matches!(sess.send_input("hi".into()), SendOutcome::Busy));
    }

    #[tokio::test]
    async fn cancel_when_idle_is_noop() {
        let (sess, _cap) = session_with_scripted();
        sess.cancel(); // must not panic
    }

    #[tokio::test]
    async fn settings_get_returns_state() {
        let (sess, _cap) = session_with_scripted();
        let st = sess.settings_get();
        assert!(!st.api_key_set);
    }

    #[tokio::test]
    async fn session_stats_starts_at_default() {
        let (sess, _cap) = session_with_scripted();
        assert_eq!(sess.session_stats(), agent_core::SessionStats::default());
    }

    #[tokio::test]
    async fn approve_unknown_id_is_noop() {
        let (sess, _cap) = session_with_scripted();
        sess.approve("nope", Decision::Approve); // must not panic
    }

    #[tokio::test]
    async fn context_get_returns_snapshot_with_system_segment() {
        let (sess, _cap) = session_with_scripted();
        let snap = sess.context_get().await;
        assert!(snap.segments.iter().any(|s| s.category == "system"));
        assert!(snap.model_limit > 0);
    }

    #[tokio::test]
    async fn set_workspace_rewrites_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        // Point config_path at a NON-EXISTENT file: Session::from_params calls
        // RuntimeConfig::load_over(params.config, &params.config_path), which
        // overlays the on-disk file over our in-memory config — an existing
        // file here would wipe the trace_dir override set below.
        let mut params = crate::setup::local_params(
            dir.path().to_path_buf(),
            dir.path().join("rt.json"),
            "http://localhost:8080".into(),
            "m".into(),
        );
        params.config.trace_dir = Some(sessions.path().to_string_lossy().into_owned());

        let sess = Session::from_params(params);
        let id = sess.session_id().to_string();
        let descriptor_dir = agent_runtime_config::session_dir(sessions.path(), &id);
        let before = agent_runtime_config::load_descriptor(&descriptor_dir)
            .expect("descriptor written at construction");
        assert_eq!(before.workspace, dir.path());

        sess.set_workspace(PathBuf::from("/elsewhere")).await;

        let after = agent_runtime_config::load_descriptor(&descriptor_dir)
            .expect("descriptor still present after rewrite");
        assert_eq!(after.workspace, PathBuf::from("/elsewhere"));
        assert_eq!(after.session_id, before.session_id);
        assert_eq!(after.created_ms, before.created_ms);
    }

    #[tokio::test]
    async fn skill_save_then_get_roundtrips() {
        let (sess, _cap) = session_with_scripted();
        sess.skill_save("greeter".into(), "Say hello to the user.".into())
            .await
            .unwrap();
        let got = sess.skill_get("greeter".into()).await.unwrap();
        assert_eq!(got.name, "greeter");
        assert!(got.body.contains("hello"));
    }

    /// On frontend attach, a PRIOR session's gate-kind park is re-emitted as an
    /// `ApprovalRequest` whose display is RE-DERIVED from the stored tool args via
    /// `tool.intent` — the checkpoint stores no display text, so equality with
    /// `intent(planted args)` pins §3.4 — and whose origin carries the checkpointed
    /// attribution.
    #[tokio::test]
    async fn attach_reemits_parked_ask_with_rederived_display() {
        use agent_core::checkpoint::{Checkpoint, Checkpointer, Guardrails, ParkedTurn};
        use agent_policy::ApprovalOrigin;

        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();

        // The session will read/create ~/.rusty-agent/secret via the REAL $HOME
        // (metadata_root()) — the accepted precedent on this branch (see runtime.rs
        // checkpointer tests). Plant the checkpoint with THAT key so the MAC verifies.
        let meta = agent_runtime_config::metadata_root().expect("HOME set");
        let key = agent_runtime_config::load_or_create_secret(&meta).expect("secret");

        // Plant PRIOR session "100-aaaaaaaa": descriptor (workspace must be a real
        // dir so reemit proceeds) + a root gate-kind park on execute_command.
        let prior_id = "100-aaaaaaaa";
        agent_runtime_config::write_descriptor(
            sessions.path(),
            &agent_runtime_config::SessionDescriptor {
                schema: agent_runtime_config::DESCRIPTOR_SCHEMA,
                session_id: prior_id.into(),
                workspace: ws.path().to_path_buf(),
                created_ms: 1,
                config_path: None,
            },
        )
        .unwrap();
        let prior_ck =
            agent_runtime_config::session_dir(sessions.path(), prior_id).join("checkpoint");
        let ckr = Checkpointer::new(prior_ck.clone(), key, prior_id.into());
        let planted_args = serde_json::json!({"command": "echo real"});
        let origin = ApprovalOrigin {
            delegation_id: "c9".into(),
            subagent_name: "explore".into(),
            depth: 1,
        };
        let chk = Checkpoint {
            version: agent_core::checkpoint::CHECKPOINT_VERSION,
            session_id: prior_id.into(),
            subagent_path: vec![],
            turn: 0,
            context: agent_core::CuratedContextState {
                goal: None,
                history: vec![Message::user("hi")],
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
                tool_calls: vec![agent_tools::ToolCall {
                    id: "c9".into(),
                    name: "execute_command".into(),
                    args: planted_args.clone(),
                }],
                invalid: vec![],
                gate_records: vec![],
                parked_index: Some(0),
                origin: Some(origin.clone()),
            },
        };
        ckr.write_park(chk, &agent_core::SessionArtifacts::new())
            .await
            .unwrap();

        // Construct a fresh session whose sessions_root IS our tempdir (trace_dir
        // override). Its own minted id != prior_id, so the scan does not skip it.
        let mut params = crate::setup::local_params(
            ws.path().to_path_buf(),
            ws.path().join("rt.json"),
            "http://localhost:8080".into(),
            "m".into(),
        );
        params.config.trace_dir = Some(sessions.path().to_string_lossy().into_owned());
        let sess = Session::from_params(params);

        let cap = Arc::new(Captured::default());
        sess.set_event_out(cap.clone());

        // The reemit runs on a spawned task — yield until the ApprovalRequest lands
        // (bounded so a bug fails fast instead of hanging).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let ask = loop {
            let found = cap.0.lock().unwrap().iter().find_map(|ev| match ev {
                ServerEvent::ApprovalRequest {
                    summary,
                    command,
                    origin,
                    ..
                } => Some((summary.clone(), command.clone(), origin.clone())),
                _ => None,
            });
            if let Some(a) = found {
                break a;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no ApprovalRequest re-emitted from the parked prior session"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let (summary, command, got_origin) = ask;

        // §3.4: the emitted display EQUALS tool.intent(planted args) — the only
        // possible source, since the checkpoint stores no display text.
        let expected =
            agent_tools::Tool::intent(&agent_tools::shell::ExecuteCommand, &planted_args)
                .expect("intent");
        assert_eq!(summary, expected.summary);
        assert_eq!(command, expected.command);
        assert_eq!(command.as_deref(), Some("echo real"));

        // Origin carries the checkpointed attribution.
        assert_eq!(
            got_origin,
            Some(crate::wire::ApprovalOriginDto {
                delegation_id: "c9".into(),
                subagent: "explore".into(),
                depth: 1,
            })
        );

        std::mem::forget(ws); // keep temp dirs alive for the process
        std::mem::forget(sessions);
    }

    /// If a session id is already in the `resuming` set (a resume for its tree is
    /// in flight or already attempted and failed this daemon lifetime), attach
    /// must NOT re-register its parked ask as a fresh external prompt — branch
    /// review I1b: any answer to that prompt could never reach a resume, since
    /// `start_resume`'s sticky guard would just bounce it, silently stranding the
    /// user's decision. Mirrors `attach_reemits_parked_ask_with_rederived_display`'s
    /// rig but pre-seeds `resuming` with the prior session id before attaching.
    #[tokio::test]
    async fn attach_skips_reemit_for_already_resuming_session() {
        use agent_core::checkpoint::{Checkpoint, Checkpointer, Guardrails, ParkedTurn};
        use agent_policy::ApprovalOrigin;

        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();

        let meta = agent_runtime_config::metadata_root().expect("HOME set");
        let key = agent_runtime_config::load_or_create_secret(&meta).expect("secret");

        let prior_id = "100-bbbbbbbb";
        agent_runtime_config::write_descriptor(
            sessions.path(),
            &agent_runtime_config::SessionDescriptor {
                schema: agent_runtime_config::DESCRIPTOR_SCHEMA,
                session_id: prior_id.into(),
                workspace: ws.path().to_path_buf(),
                created_ms: 1,
                config_path: None,
            },
        )
        .unwrap();
        let prior_ck =
            agent_runtime_config::session_dir(sessions.path(), prior_id).join("checkpoint");
        let ckr = Checkpointer::new(prior_ck.clone(), key, prior_id.into());
        let planted_args = serde_json::json!({"command": "echo real"});
        let origin = ApprovalOrigin {
            delegation_id: "c9".into(),
            subagent_name: "explore".into(),
            depth: 1,
        };
        let chk = Checkpoint {
            version: agent_core::checkpoint::CHECKPOINT_VERSION,
            session_id: prior_id.into(),
            subagent_path: vec![],
            turn: 0,
            context: agent_core::CuratedContextState {
                goal: None,
                history: vec![Message::user("hi")],
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
                tool_calls: vec![agent_tools::ToolCall {
                    id: "c9".into(),
                    name: "execute_command".into(),
                    args: planted_args.clone(),
                }],
                invalid: vec![],
                gate_records: vec![],
                parked_index: Some(0),
                origin: Some(origin.clone()),
            },
        };
        ckr.write_park(chk, &agent_core::SessionArtifacts::new())
            .await
            .unwrap();

        let mut params = crate::setup::local_params(
            ws.path().to_path_buf(),
            ws.path().join("rt.json"),
            "http://localhost:8080".into(),
            "m".into(),
        );
        params.config.trace_dir = Some(sessions.path().to_string_lossy().into_owned());
        let sess = Session::from_params(params);

        // Simulate an in-flight/failed-and-stuck resume for the prior tree
        // BEFORE attaching, exactly the state `start_resume`'s sticky guard
        // leaves behind (branch review I1a).
        sess.resuming.lock().unwrap().insert(prior_id.to_string());

        let cap = Arc::new(Captured::default());
        sess.set_event_out(cap.clone());

        // Give the reemit task a bounded window to (incorrectly) emit, then
        // assert it never did. There is no positive event to await here — the
        // absence IS the assertion — so we sleep past where the sibling test
        // reliably observes its ApprovalRequest and check nothing landed.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let saw_request = cap
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|ev| matches!(ev, ServerEvent::ApprovalRequest { .. }));
        assert!(
            !saw_request,
            "attach re-emitted an ApprovalRequest for a session already in `resuming`"
        );

        std::mem::forget(ws);
        std::mem::forget(sessions);
    }

    /// When a skill's directory name is not already a lowercase slug (e.g. "Greeter"
    /// → slug "greeter"), `skill_save` must still look up the existing description by
    /// the ORIGINAL name, not the slug — otherwise the description is silently replaced
    /// by the "{slug} skill" default on every body edit.
    #[tokio::test]
    async fn skill_save_edit_preserves_description_on_name_slug_mismatch() {
        let (sess, _cap) = session_with_scripted();

        // Seed a skill whose directory name is "Greeter" (has uppercase → slug is "greeter").
        let ws = sess.settings_get().workspace;
        let skills_root = std::path::Path::new(&ws)
            .join(".rusty-agent")
            .join("skills");
        let skill_dir = skills_root.join("Greeter");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: Greeter\ndescription: Greets the user warmly\n---\nSay hello.\n",
        )
        .unwrap();

        // Simulate a body edit via skill_save using the same name "Greeter".
        sess.skill_save("Greeter".into(), "Say hi to everyone.".into())
            .await
            .unwrap();

        // The written file goes to writable_root/greeter (the slug).
        // The description must be preserved from "Greeter", not defaulted to "greeter skill".
        let got = sess.skill_get("greeter".into()).await.unwrap();
        assert_eq!(
            got.description, "Greets the user warmly",
            "description was lost on edit; got: {:?}",
            got.description
        );
    }

    /// `set_event_out` (the `subscribe` command body) must never abort a caller
    /// with no Tokio reactor in scope — e.g. a sync Tauri command running on the
    /// glib main thread, where `tokio::spawn` panics non-unwinding across the
    /// C-FFI boundary and takes the whole app down. Regression test for that
    /// live-drive bug: build + attach entirely on a plain `std::thread`, no
    /// `#[tokio::test]` runtime anywhere in scope.
    #[test]
    fn set_event_out_from_non_tokio_thread_does_not_panic() {
        let handle = std::thread::spawn(|| {
            let dir = tempfile::tempdir().unwrap();
            let params = crate::setup::local_params(
                dir.path().to_path_buf(),
                dir.path().join("rt.json"),
                "http://localhost:8080".into(),
                "m".into(),
            );
            let sess = Session::from_params(params);
            let cap = Arc::new(Captured::default());
            sess.set_event_out(cap.clone()); // must not panic/abort
            std::mem::forget(dir);
        });
        handle
            .join()
            .expect("set_event_out must not panic off a Tokio reactor");
    }
}
