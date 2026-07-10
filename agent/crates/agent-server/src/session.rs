//! App-lifetime session state for the Tauri IPC transport. Replaces the
//! per-connection state that `daemon::serve()` used to own.
use crate::approval::IpcApprovalChannel;
use crate::daemon::DaemonParams;
use crate::runtime::RuntimeState;
use crate::sink::{ChannelEventSink, EventSlot};
use crate::wire::{ArchitectureSnapshot, Decision, EventOut, SettingsState};
use agent_core::{ContextManager, CuratedContext};
use agent_model::Message;
use agent_runtime_config::RuntimeConfig;
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
        })
    }

    /// Register/replace the outbound channel (the `subscribe` command body).
    pub fn set_event_out(&self, out: Arc<dyn EventOut>) {
        *self.slot.lock().unwrap() = Some(out.clone());
        // Daemon-alive reattach (spec §2.4 step 5): pending asks re-emit under
        // their LIVE ids — no duplicate is minted.
        self.approval.reemit_pending(&out);
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
}
