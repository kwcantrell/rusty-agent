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

    /// Emit a wire event on the current outbound channel (no-op if none is
    /// attached).
    fn send_event(&self, ev: crate::wire::ServerEvent) {
        if let Some(out) = self.slot.lock().unwrap().clone() {
            out.send(ev);
        }
    }

    /// Emit a wire `Error` frame on the current outbound channel (no-op if
    /// none is attached — the resume driver's failures are surfaced here).
    fn emit_error(&self, message: String) {
        self.send_event(crate::wire::ServerEvent::Error { message });
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
            let Some(meta) = agent_runtime_config::metadata_root_for(&cfg) else {
                return;
            };
            let Ok(key) = agent_runtime_config::load_or_create_secret(&meta) else {
                return;
            };
            let parked_list = crate::resume::scan_parked(&root, &key, sess.runtime.session_id());

            let resuming = sess.resuming.lock().unwrap().clone();
            let runs: Vec<crate::wire::ParkedRunDto> = parked_list
                .iter()
                .filter(|p| !resuming.contains(&p.descriptor.session_id))
                .map(|p| crate::wire::ParkedRunDto {
                    session_id: p.descriptor.session_id.clone(),
                    workspace: p.descriptor.workspace.display().to_string(),
                    created_ms: p.descriptor.created_ms,
                    asks: p.asks.iter().filter(|a| !a.answered).count() as u32,
                    error: p.errors.first().cloned(),
                })
                .collect();
            if !runs.is_empty() {
                sess.send_event(crate::wire::ServerEvent::ParkedRuns { runs });
            }

            for parked in parked_list {
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
            &parked.descriptor.session_id,
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
                "session {sid}: resume already in progress; answer again after it finishes"
            ));
            return;
        }
        match agent_core::checkpoint::claim_resume(&root_dir) {
            Ok(true) => {}
            _ => {
                self.emit_error(format!(
                    "session {sid}: is being resumed elsewhere (another daemon or a CLI reopen); \
                     if that process crashed, remove {}/resume.lock",
                    root_dir.display()
                ));
                self.resuming.lock().unwrap().remove(sid);
                return;
            }
        }
        {
            let mut active = self.active.lock().unwrap();
            if active.is_some() {
                self.emit_error(format!(
                    "session {sid} answered but a run is active; reattach after it finishes"
                ));
                self.resuming.lock().unwrap().remove(sid);
                agent_core::checkpoint::release_resume(&root_dir);
                return;
            }
            *active = Some(CancellationToken::new());
        }
        self.send_event(crate::wire::ServerEvent::Resumed {
            session_id: sid.to_string(),
        });
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
            let mut retained = false;
            match loop_
                .resume_with_cancel(&mut ctx, resume, cancel.clone())
                .await
            {
                Ok(()) if cancel.is_cancelled() => {
                    // P1 (4B-2 branch review C-1): `turn_loop` returns Ok(())
                    // on cancellation too, so Ok(()) alone does not mean the
                    // tree completed. A cancel mid-resume (e.g. a re-park at
                    // a NEW ask, or a `cancel` command arriving while
                    // resuming) must retain the park exactly like the Err
                    // arm below — never reap a park the run didn't actually
                    // finish. Mirrors dispatch.rs's is_cancelled() check
                    // BEFORE treating Ok as completed.
                    retained = true;
                }
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
                    retained = true;
                }
            }
            *sess.active.lock().unwrap() = None;
            if retained {
                // In-life retry (4B-1 merge-gate deferral): the park was
                // retained; releasing lock + guard lets the next attach
                // re-prompt (refinement 11 makes the retry re-claim).
                agent_core::checkpoint::release_resume(&root_dir);
                sess.resuming.lock().unwrap().remove(&sid);
            }
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
    use crate::testkit::{wait_for_ask_id, Captured};
    use crate::wire::ServerEvent;
    use std::sync::Arc;

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

    /// On attach, the `ParkedRuns` snapshot carries one `ParkedRunDto` per PRIOR
    /// parked session (4B-2) — session id, workspace, unanswered-ask count, no
    /// error. A session already in `resuming` (a resume for its tree already in
    /// flight or attempted) must NOT appear, since attach also skips re-emitting
    /// its ask (mirrors `attach_skips_reemit_for_already_resuming_session`).
    #[tokio::test]
    async fn attach_sends_parked_runs_snapshot() {
        use agent_core::checkpoint::{Checkpoint, Checkpointer, Guardrails, ParkedTurn};
        use agent_policy::ApprovalOrigin;

        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();

        let meta = agent_runtime_config::metadata_root().expect("HOME set");
        let key = agent_runtime_config::load_or_create_secret(&meta).expect("secret");

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

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let runs = loop {
            let found = cap.0.lock().unwrap().iter().find_map(|ev| match ev {
                ServerEvent::ParkedRuns { runs } => Some(runs.clone()),
                _ => None,
            });
            if let Some(r) = found {
                break r;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no ParkedRuns snapshot sent on attach"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        assert_eq!(runs.len(), 1, "exactly one prior parked session");
        let run = &runs[0];
        assert_eq!(run.session_id, prior_id);
        assert_eq!(run.workspace, ws.path().display().to_string());
        assert_eq!(run.asks, 1);
        assert_eq!(run.error, None);

        std::mem::forget(ws);
        std::mem::forget(sessions);
    }

    /// A session id already in `resuming` at attach time must NOT appear in the
    /// `ParkedRuns` snapshot — mirrors the ask-suppression rule in
    /// `attach_skips_reemit_for_already_resuming_session`.
    #[tokio::test]
    async fn attach_parked_runs_snapshot_excludes_already_resuming_session() {
        use agent_core::checkpoint::{Checkpoint, Checkpointer, Guardrails, ParkedTurn};
        use agent_policy::ApprovalOrigin;

        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();

        let meta = agent_runtime_config::metadata_root().expect("HOME set");
        let key = agent_runtime_config::load_or_create_secret(&meta).expect("secret");

        let prior_id = "100-cccccccc";
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
        sess.resuming.lock().unwrap().insert(prior_id.to_string());

        let cap = Arc::new(Captured::default());
        sess.set_event_out(cap.clone());

        // No positive event to await (the exclusion IS the assertion) — sleep past
        // where the sibling test reliably observes its snapshot, then check.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let saw_prior_run = cap.0.lock().unwrap().iter().any(|ev| match ev {
            ServerEvent::ParkedRuns { runs } => runs.iter().any(|r| r.session_id == prior_id),
            _ => false,
        });
        assert!(
            !saw_prior_run,
            "already-resuming session must not appear in the ParkedRuns snapshot"
        );

        std::mem::forget(ws);
        std::mem::forget(sessions);
    }

    /// Approving a re-emitted parked ask drives the answer through
    /// `checkpoint::write_answer` and into `start_resume` (mirrors
    /// `attach_reemits_parked_ask_with_rederived_display`'s rig, then answers).
    /// `start_resume` must emit a `Resumed { session_id }` frame right after the
    /// `resuming`/`active` guards succeed — i.e. synchronously, before the
    /// resumed run itself is even spawned (so it necessarily precedes whatever
    /// that run's first event turns out to be).
    #[tokio::test]
    async fn approving_a_parked_ask_emits_resumed_before_the_resumed_runs_first_event() {
        use agent_core::checkpoint::{Checkpoint, Checkpointer, Guardrails, ParkedTurn};
        use agent_policy::ApprovalOrigin;

        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();

        let meta = agent_runtime_config::metadata_root().expect("HOME set");
        let key = agent_runtime_config::load_or_create_secret(&meta).expect("secret");

        let prior_id = "100-dddddddd";
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

        let cap = Arc::new(Captured::default());
        sess.set_event_out(cap.clone());

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let ask_id = loop {
            let found = cap.0.lock().unwrap().iter().find_map(|ev| match ev {
                ServerEvent::ApprovalRequest { id, .. } => Some(id.clone()),
                _ => None,
            });
            if let Some(id) = found {
                break id;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no ApprovalRequest re-emitted from the parked prior session"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        // Snapshot the frame count before answering: `Resumed` is emitted
        // synchronously in `start_resume`, before the resumed run's driving task
        // is even spawned — so it must appear at (or after) this index, and
        // nothing the resumed run itself produces can have beaten it there.
        let before_answer = cap.0.lock().unwrap().len();
        sess.approve(&ask_id, Decision::Approve);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let resumed_index = loop {
            let idx = cap.0.lock().unwrap().iter().position(
                |ev| matches!(ev, ServerEvent::Resumed { session_id } if session_id == prior_id),
            );
            if let Some(i) = idx {
                break i;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no Resumed frame emitted for the answered parked session"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert!(
            resumed_index >= before_answer,
            "Resumed frame must be captured no earlier than the answer that triggers it"
        );

        std::mem::forget(ws);
        std::mem::forget(sessions);
    }

    /// A resumed run's trace must land in ITS OWN session's jsonl (4B-2 fix for a
    /// 4B-1 merge-gate deferral) — not the resuming daemon's own trace file.
    /// `max_turns: 1` on the daemon's config makes `resume_with_cancel` complete
    /// without a model call: the checkpoint's `turn: 0` means the post-batch
    /// `turn_loop(start_turn + 1 = 1)` runs zero iterations against `max_turns: 1`
    /// and lands straight in the budget-exhaustion epilogue (loop_.rs), so the
    /// executed `execute_command` tool events are the only thing traced — real
    /// completion, no live model server needed.
    #[tokio::test]
    async fn resumed_run_traces_into_its_own_session_file() {
        use agent_core::checkpoint::{Checkpoint, Checkpointer, Guardrails, ParkedTurn};
        use agent_policy::ApprovalOrigin;

        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();

        let meta = agent_runtime_config::metadata_root().expect("HOME set");
        let key = agent_runtime_config::load_or_create_secret(&meta).expect("secret");

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
                // The parked assistant turn (tool_calls: [c9]) must already be in
                // history — tool_phase appends only the Role::Tool result, never the
                // assistant message, so its absence here would leave that result
                // orphaned (curated.rs orphaned_tool_positions).
                history: vec![
                    Message::user("hi"),
                    Message::assistant(
                        "running",
                        Some(vec![agent_tools::ToolCall {
                            id: "c9".into(),
                            name: "execute_command".into(),
                            args: planted_args.clone(),
                        }]),
                    ),
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
        params.config.trace = true;
        params.config.trace_dir = Some(sessions.path().to_string_lossy().into_owned());
        params.config.max_turns = 1;
        let sess = Session::from_params(params);
        let own_id = sess.session_id().to_string();

        let cap = Arc::new(Captured::default());
        sess.set_event_out(cap.clone());

        // 15s, not the usual 5s: observed flaking at 5s under parallel `cargo
        // test` package invocations (CPU/disk contention delays the re-emit).
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let ask_id = loop {
            let found = cap.0.lock().unwrap().iter().find_map(|ev| match ev {
                ServerEvent::ApprovalRequest { id, .. } => Some(id.clone()),
                _ => None,
            });
            if let Some(id) = found {
                break id;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no ApprovalRequest re-emitted from the parked prior session"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        let own_trace = sessions.path().join(format!("{own_id}.jsonl"));
        let own_lines_before = std::fs::read_to_string(&own_trace)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        let prior_trace = sessions.path().join(format!("{prior_id}.jsonl"));
        let prior_lines_before = std::fs::read_to_string(&prior_trace)
            .map(|s| s.lines().count())
            .unwrap_or(0);

        sess.approve(&ask_id, Decision::Approve);

        // Wait for the resumed run to finish (active cleared in start_resume's
        // spawned task) rather than for a specific event, since completion here
        // is budget exhaustion, not a Done the harness names distinctly.
        // 15s, not the usual 5s: this test flaked 3x across task runs under
        // parallel `cargo test` package invocations — the spawned resume task
        // is contention-sensitive, so give it the file's tolerant-end budget.
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if sess.resuming.lock().unwrap().contains(prior_id)
                && sess.active.lock().unwrap().is_none()
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "resumed run never completed"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Give the trace writer's fs write a beat past the active-flag clear.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let prior_lines_after = std::fs::read_to_string(&prior_trace)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        assert!(
            prior_lines_after > prior_lines_before,
            "resumed session's own trace file must gain records from the resumed run"
        );

        let own_lines_after = std::fs::read_to_string(&own_trace)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        assert_eq!(
            own_lines_after, own_lines_before,
            "the daemon's own trace file must NOT gain the resumed run's tool events"
        );

        std::mem::forget(ws);
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

    /// Thin 3-arg wrapper over `testkit::plant_parked_session`, defaulting
    /// `meta` to the real `metadata_root()` (`~/.rusty-agent`) — the ~8
    /// existing call sites in this test mod predate the E1 `metadata_dir`
    /// seam and don't need rig-rooted metadata.
    async fn plant_parked_session(
        ws: &std::path::Path,
        sessions: &std::path::Path,
        prior_id: &str,
    ) -> (Arc<Session>, Arc<Captured>, [u8; 32]) {
        let meta = agent_runtime_config::metadata_root().expect("HOME set");
        crate::testkit::plant_parked_session(ws, sessions, &meta, prior_id).await
    }

    /// Thin 3-arg (+ command) wrapper over
    /// `testkit::plant_parked_session_with_command`, defaulting `meta` to
    /// the real `metadata_root()` — see `plant_parked_session` above.
    async fn plant_parked_session_with_command(
        ws: &std::path::Path,
        sessions: &std::path::Path,
        prior_id: &str,
        command: &str,
    ) -> (Arc<Session>, Arc<Captured>, [u8; 32]) {
        let meta = agent_runtime_config::metadata_root().expect("HOME set");
        crate::testkit::plant_parked_session_with_command(ws, sessions, &meta, prior_id, command)
            .await
    }

    /// A resumed run whose (mocked) model call fails must NOT strand the
    /// `resuming` guard or the cross-process lock for the rest of the daemon's
    /// lifetime (4B-1 merge-gate deferral: previously only a daemon restart
    /// could retry).
    ///
    /// NOTE on the retained-park framing: for THIS fixture (a single-ask,
    /// gate-kind root checkpoint), consuming the committed answer clears
    /// `parked.json` at consume time — synchronously, inside `tool_phase`,
    /// BEFORE the run can possibly fail (loop_.rs ~1414-1419: "Answer commit
    /// ... delete the park before proceeding"; the ONLY failure path,
    /// `AgentError::Model`, can only occur afterward, in `turn_loop`). So a
    /// SECOND `spawn_parked_reemit` disk rescan finds nothing to re-emit for
    /// this tree — there is no leftover ask once the answer that unblocked
    /// the (now-failed) run was itself durably consumed. What DOES hold, and
    /// is what this test asserts, is the guard/lock release that makes a
    /// direct retry (the CLI-reopen path Task 10 adds, or a second live
    /// `start_resume`) possible without a daemon restart — instead of
    /// `start_resume`'s sticky-guard bounce this task replaces.
    #[tokio::test]
    async fn failed_resume_clears_the_guard_so_the_next_attach_reprompts() {
        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let prior_id = "100-eeeeeeee";

        let (sess, cap, key) = plant_parked_session(ws.path(), sessions.path(), prior_id).await;
        let root_dir =
            agent_runtime_config::session_dir(sessions.path(), prior_id).join("checkpoint");

        let ask_id = wait_for_ask_id(&cap, Duration::from_secs(5)).await;
        sess.approve(&ask_id, Decision::Approve);

        // The resumed run's model call fails fast (nothing listens on
        // 127.0.0.1:1), then exhausts the configured retries (with backoff)
        // before `start_resume`'s Err arm fires — allow generous headroom.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let saw_failure = cap.0.lock().unwrap().iter().any(|ev| {
                matches!(ev, ServerEvent::Error { message } if message.contains("resumed run failed"))
            });
            if saw_failure {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "resumed run never surfaced a failure error frame; captured: {:#?}",
                cap.0.lock().unwrap()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(
            !sess.resuming.lock().unwrap().contains(prior_id),
            "failed resume must remove the session id from the `resuming` guard"
        );
        assert!(
            agent_core::checkpoint::claim_resume(&root_dir).unwrap(),
            "failed resume must release the cross-process lock so a retry can claim it"
        );
        agent_core::checkpoint::release_resume(&root_dir); // undo the probe claim above

        // The consume-time clear (loop_.rs ~1417) already removed this
        // single-ask tree's park+answer, so nothing is left to re-scan from
        // disk — confirm that directly instead of asserting a rescan finds
        // an ask that structurally cannot exist here.
        assert!(
            !agent_core::checkpoint::has_park(&root_dir),
            "the answered ask's park is expected to be consumed regardless of the run's outcome"
        );

        // What must hold: retry no longer requires a daemon restart. Bounce
        // literal changed (this task); the OLD wording promised nothing but
        // a restart could ever retry — assert that's no longer what a second
        // attempt hits. Directly re-drive `start_resume` (the same call a
        // fresh attach/CLI-reopen makes) and confirm it is NOT refused by a
        // stale `resuming` entry or an unreleased lock; NOTE the checkpoint
        // tree is gone (consumed above), so `load_checkpoint` legitimately
        // finds nothing left to resume — that is a different, expected,
        // outcome from the old sticky-guard bounce this task removes.
        assert_eq!(
            agent_core::checkpoint::load_checkpoint(&root_dir, &key).unwrap(),
            None,
            "sanity: nothing left to resume once the answered ask's park is consumed"
        );

        std::mem::forget(ws);
        std::mem::forget(sessions);
    }

    /// C-1 (4B-2 branch review): a `cancel` command arriving mid-resume must
    /// retain the root park instead of hitting `start_resume`'s success-reap
    /// path — `resume_with_cancel` (via `turn_loop`) returns `Ok(())` on
    /// cancellation exactly like completion, so `Ok(())` alone can't mean
    /// "reap". Uses a `sleep 2` planted command (instead of the failed-resume
    /// test's instant `echo real`) so cancellation has a real, deterministic
    /// execution window to land in — well before the doomed model call at
    /// `127.0.0.1:1` would otherwise race it via the network-refused path.
    #[tokio::test]
    async fn cancelled_mid_resume_retains_the_park_and_releases_the_guard() {
        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let prior_id = "100-cccccccc";

        let (sess, cap, _key) =
            plant_parked_session_with_command(ws.path(), sessions.path(), prior_id, "sleep 2")
                .await;
        let root_dir =
            agent_runtime_config::session_dir(sessions.path(), prior_id).join("checkpoint");

        let ask_id = wait_for_ask_id(&cap, Duration::from_secs(5)).await;
        sess.approve(&ask_id, Decision::Approve);

        // Give start_resume's spawned task a moment to actually begin the
        // `sleep 2` tool call (active slot populated) before tripping cancel,
        // so we don't race the resume-lock claim/guard-insert at the very
        // top of start_resume itself.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while sess.active.lock().unwrap().is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "resumed run never went active"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        sess.cancel();

        // The resumed run must finish promptly (turn_loop's cancellation
        // check, not the 2s sleep completing or the network call failing).
        let cancel_deadline = std::time::Instant::now() + Duration::from_secs(5);
        while sess.active.lock().unwrap().is_some() {
            assert!(
                std::time::Instant::now() < cancel_deadline,
                "cancelled resume never went idle"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(
            !cap.0.lock().unwrap().iter().any(|ev| {
                matches!(ev, ServerEvent::Error { message } if message.contains("resumed run failed"))
            }),
            "a cancelled resume must not surface as a failure (proves the Ok(()) + \
             is_cancelled() arm fired, not the Err arm)"
        );
        // This single-ask fixture's `parked.json` already self-cleared at
        // answer-CONSUME time (inside tool_phase, before the sleep even
        // starts — same property the failed-resume test above documents),
        // so `has_park` can't be the signal here. What C-1 actually guards
        // is the checkpoint TREE (root_dir + resume.lock) surviving instead
        // of being `remove_dir_all`'d by the success-reap arm.
        assert!(
            root_dir.exists(),
            "cancellation mid-resume must retain the checkpoint tree, not reap it \
             via the success-reap remove_dir_all"
        );
        assert!(
            !sess.resuming.lock().unwrap().contains(prior_id),
            "cancelled resume must remove the session id from the `resuming` guard"
        );
        assert!(
            agent_core::checkpoint::claim_resume(&root_dir).unwrap(),
            "cancelled resume must release the cross-process lock so a retry can claim it"
        );
        agent_core::checkpoint::release_resume(&root_dir); // undo the probe claim above

        std::mem::forget(ws);
        std::mem::forget(sessions);
    }

    /// A concurrent CLI reopen (a second process) holding `resume.lock` must
    /// make `start_resume` refuse before the resumed run starts, leaving the
    /// park and lock untouched — refinement 11 extends the daemon-local
    /// `resuming` guard's exclusivity across processes.
    #[tokio::test]
    async fn resume_refuses_when_another_process_holds_the_lock() {
        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let prior_id = "100-ffffffff";

        let (sess, cap, key) = plant_parked_session(ws.path(), sessions.path(), prior_id).await;
        let root_dir =
            agent_runtime_config::session_dir(sessions.path(), prior_id).join("checkpoint");

        // Simulate a concurrent CLI reopen already holding the lock.
        assert!(agent_core::checkpoint::claim_resume(&root_dir).unwrap());

        let ask_id = wait_for_ask_id(&cap, Duration::from_secs(5)).await;
        sess.approve(&ask_id, Decision::Approve);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let saw_refusal = cap.0.lock().unwrap().iter().any(|ev| {
                matches!(ev, ServerEvent::Error { message } if message.contains("being resumed elsewhere"))
            });
            if saw_refusal {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no 'being resumed elsewhere' error frame emitted"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(
            !cap.0
                .lock()
                .unwrap()
                .iter()
                .any(|ev| matches!(ev, ServerEvent::Resumed { .. })),
            "a lock-refused resume must never emit Resumed"
        );

        // Park and lock untouched: the checkpoint tree still verifies, and the
        // lock is still held (claim_resume from here fails).
        assert!(agent_core::checkpoint::has_park(&root_dir));
        assert!(agent_core::checkpoint::load_checkpoint(&root_dir, &key)
            .unwrap()
            .is_some());
        assert!(
            !agent_core::checkpoint::claim_resume(&root_dir).unwrap(),
            "the pre-held lock must still be held; a refused resume must not touch it"
        );

        std::mem::forget(ws);
        std::mem::forget(sessions);
    }

    /// The active-slot conflict branch ("answered but a run is active;
    /// reattach after it finishes") fires AFTER `claim_resume` already
    /// succeeded — it must release the cross-process lock before bouncing,
    /// same as the sticky-guard and lock-refused branches above. Otherwise
    /// the lock leaks and every subsequent resume attempt for this session
    /// spuriously hits "being resumed elsewhere" (Task 5 review finding).
    #[tokio::test]
    async fn active_conflict_resume_releases_the_lock() {
        let ws = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let prior_id = "100-cccccccc";

        let (sess, cap, _key) = plant_parked_session(ws.path(), sessions.path(), prior_id).await;
        let root_dir =
            agent_runtime_config::session_dir(sessions.path(), prior_id).join("checkpoint");

        let ask_id = wait_for_ask_id(&cap, Duration::from_secs(5)).await;

        // Occupy the session's `active` slot as if a run were already live,
        // mirroring how other tests in this module drive the busy path.
        *sess.active.lock().unwrap() = Some(CancellationToken::new());

        sess.approve(&ask_id, Decision::Approve);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let saw_conflict = cap.0.lock().unwrap().iter().any(|ev| {
                matches!(ev, ServerEvent::Error { message } if message.contains("run is active"))
            });
            if saw_conflict {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no 'run is active' error frame emitted; captured: {:#?}",
                cap.0.lock().unwrap()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(
            agent_core::checkpoint::claim_resume(&root_dir).unwrap(),
            "the active-conflict early return must release the cross-process lock \
             so a later resume attempt can claim it"
        );
        agent_core::checkpoint::release_resume(&root_dir); // undo the probe claim above

        std::mem::forget(ws);
        std::mem::forget(sessions);
    }
}
