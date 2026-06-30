//! App-lifetime session state for the Tauri IPC transport. Replaces the
//! per-connection state that `daemon::serve()` used to own.
use crate::approval::IpcApprovalChannel;
use crate::daemon::DaemonParams;
use crate::runtime::RuntimeState;
use crate::sink::{ChannelEventSink, EventSlot};
use crate::wire::{Decision, EventOut, SettingsState};
use agent_core::{ContextManager, CuratedContext};
use agent_model::Message;
use agent_runtime_config::RuntimeConfig;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

pub enum SendOutcome { Started, Busy }

pub struct Session {
    runtime: Arc<RuntimeState>,
    ctx: Arc<AsyncMutex<CuratedContext>>,
    slot: EventSlot,
    approval: Arc<IpcApprovalChannel>,
    active: Arc<Mutex<Option<CancellationToken>>>,
    recall_budget: usize,
    workspace: Mutex<PathBuf>,
    memory_parts: Option<agent_memory::MemoryParts>,
}

impl Session {
    pub fn from_params(params: DaemonParams) -> Arc<Self> {
        let slot: EventSlot = Arc::new(Mutex::new(None));
        let sink = Arc::new(ChannelEventSink::new(slot.clone()));
        let approval = Arc::new(IpcApprovalChannel::new(slot.clone(), APPROVAL_TIMEOUT));
        let config = RuntimeConfig::load_over(params.config.clone(), &params.config_path);
        let runtime = Arc::new(RuntimeState::new(
            config, sink, approval.clone(), params.workspace.clone(), params.api_key.clone(),
            params.claude_binary.clone(), params.config_path.clone(),
            params.mcp_tools.clone(), params.memory_tools.clone(),
            params.memory_retriever.clone(), params.system_prompt.clone()));
        let ctx = Arc::new(AsyncMutex::new(
            CuratedContext::new(
                Message::system(params.system_prompt.clone()),
                runtime.offload_store(),
                runtime.compact_flag(),
            )
            .with_recall_budget(params.recall_token_budget)));
        Arc::new(Self { runtime, ctx, slot, approval,
            active: Arc::new(Mutex::new(None)), recall_budget: params.recall_token_budget,
            workspace: Mutex::new(params.workspace), memory_parts: params.memory_parts })
    }

    /// Register/replace the outbound channel (the `subscribe` command body).
    pub fn set_event_out(&self, out: Arc<dyn EventOut>) {
        *self.slot.lock().unwrap() = Some(out);
    }

    /// Start a run unless one is active (A1 guard).
    pub fn send_input(self: &Arc<Self>, text: String) -> SendOutcome {
        let mut active = self.active.lock().unwrap();
        if active.is_some() { return SendOutcome::Busy; }
        let cancel = CancellationToken::new();
        *active = Some(cancel.clone());
        drop(active);

        let agent = self.runtime.current_loop();
        let system_prompt = self.runtime.current_system_prompt();
        let ctx = self.ctx.clone();
        let active_slot = self.active.clone();
        tokio::spawn(async move {
            {
                let mut guard = ctx.lock().await;
                guard.set_system(Message::system(system_prompt));
                if let Err(e) = agent.run_with_cancel(&mut *guard, text, cancel).await {
                    tracing::error!(error=%e, "run failed");
                }
            }
            *active_slot.lock().unwrap() = None;
        });
        SendOutcome::Started
    }

    /// Trip the active run's token (the B3 interactive cancel). No-op when idle.
    pub fn cancel(&self) {
        if let Some(tok) = self.active.lock().unwrap().as_ref() { tok.cancel(); }
    }

    pub fn approve(&self, id: &str, decision: Decision) {
        self.approval.resolve(id, decision.into());
    }

    pub fn settings_get(&self) -> SettingsState {
        self.runtime.settings_state()
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

    fn memory_admin(&self) -> Option<agent_memory::MemoryAdmin> {
        let parts = self.memory_parts.as_ref()?;
        let scope = agent_memory::project_scope(&*self.workspace.lock().unwrap());
        Some(agent_memory::MemoryAdmin::new(
            parts.embedder.clone(), parts.store.clone(), parts.cfg.clone(), scope))
    }

    pub async fn memory_list(&self, limit: usize, offset: usize)
        -> Result<Vec<agent_memory::MemoryRow>, String> {
        match self.memory_admin() {
            Some(a) => a.list(limit, offset).await.map_err(|e| e.to_string()),
            None => Ok(vec![]),
        }
    }

    pub async fn memory_update(&self, id: String, text: Option<String>, tags: Option<Vec<String>>)
        -> Result<agent_memory::MemoryRow, String> {
        self.memory_admin().ok_or_else(|| "memory disabled".to_string())?
            .update(&id, text, tags).await.map_err(|e| e.to_string())
    }

    pub async fn memory_delete(&self, id: String) -> Result<bool, String> {
        self.memory_admin().ok_or_else(|| "memory disabled".to_string())?
            .delete(&id).await.map_err(|e| e.to_string())
    }

    pub async fn memory_recall_preview(&self, query: String) -> Vec<agent_memory::ScoredRow> {
        match self.memory_admin() {
            Some(a) => a.recall_preview(&query).await,
            None => vec![],
        }
    }

    /// Cancel any run, then reset the conversation context (workspace switch).
    pub async fn set_workspace(self: &Arc<Self>, dir: PathBuf) {
        self.cancel();
        *self.workspace.lock().unwrap() = dir;
        let mut guard = self.ctx.lock().await;
        *guard = CuratedContext::new(
            Message::system(self.runtime.current_system_prompt()),
            self.runtime.offload_store(),
            self.runtime.compact_flag(),
        )
        .with_recall_budget(self.recall_budget);
    }

    #[cfg(test)]
    fn mark_active_for_test(&self) {
        *self.active.lock().unwrap() = Some(CancellationToken::new());
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
        fn send(&self, ev: ServerEvent) { self.0.lock().unwrap().push(ev); }
    }

    fn session_with_scripted() -> (Arc<Session>, Arc<Captured>) {
        let dir = tempfile::tempdir().unwrap();
        let params = crate::setup::local_params(
            dir.path().to_path_buf(), dir.path().join("rt.json"),
            "http://localhost:8080".into(), "m".into(), None);
        let sess = Session::from_params(params);
        let cap = Arc::new(Captured::default());
        sess.set_event_out(cap.clone());
        std::mem::forget(dir); // keep temp dir alive for the test process
        (sess, cap)
    }

    #[tokio::test]
    async fn memory_list_is_empty_on_fresh_store() {
        let (sess, _cap) = session_with_scripted(); // scripted setup passes memory_parts: None
        let rows = sess.memory_list(20, 0).await.unwrap_or_default();
        assert!(rows.is_empty());
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
}
