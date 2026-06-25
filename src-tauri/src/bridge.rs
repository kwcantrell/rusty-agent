//! Holds the app-lifetime agent Session. Workspace switches reset the live
//! Session's context rather than dropping a socket.
use agent_server::session::Session;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct Bridge {
    session: Arc<Session>,
    workspace: Mutex<PathBuf>,
    // Retained for a future per-workspace Session rebuild (not needed for ctx reset).
    #[allow(dead_code)]
    config_path: PathBuf,
    #[allow(dead_code)]
    base_url: String,
    #[allow(dead_code)]
    model: String,
    #[allow(dead_code)]
    memory_parts: Option<agent_memory::MemoryParts>,
}

impl Bridge {
    pub fn session(&self) -> Arc<Session> {
        self.session.clone()
    }

    pub async fn current_workspace(&self) -> PathBuf {
        self.workspace.lock().await.clone()
    }

    /// Switch workspace: caller persists; reset the live Session's context bound
    /// to `dir`.
    pub async fn set_workspace(&self, dir: PathBuf) {
        *self.workspace.lock().await = dir.clone();
        self.session.set_workspace(dir).await;
    }
}

pub async fn start(
    workspace: PathBuf,
    config_path: PathBuf,
    base_url: String,
    model: String,
) -> std::io::Result<Arc<Bridge>> {
    // Memory is loaded once (model + DB), gated on the effective (persisted) flag.
    let eff = agent_runtime_config::RuntimeConfig::load_over(
        agent_runtime_config::RuntimeConfig::from_launch(
            "openai".into(), base_url.clone(), model.clone(), "native".into(), 262_144),
        &config_path);
    let memory_parts = if eff.memory {
        match agent_memory::open_memory_parts(agent_memory::MemoryConfig::default()) {
            Ok(parts) => Some(parts),
            Err(e) => { eprintln!("warning: desktop memory disabled: {e}"); None }
        }
    } else {
        None
    };

    let params = agent_server::setup::local_params(
        workspace.clone(), config_path.clone(), base_url.clone(), model.clone(),
        memory_parts.as_ref());
    let session = Session::from_params(params);

    Ok(Arc::new(Bridge {
        session,
        workspace: Mutex::new(workspace),
        config_path,
        base_url,
        model,
        memory_parts,
    }))
}
