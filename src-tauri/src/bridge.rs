//! Localhost WebSocket bridge: accepts the webview's connection and drives the
//! embedded agent runtime via `agent_server::daemon::serve`.
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

pub struct Bridge {
    pub port: u16,
    current: Mutex<Option<tokio::task::JoinHandle<()>>>,
    workspace: Arc<Mutex<PathBuf>>,
    config_path: PathBuf,
    base_url: String,
    model: String,
}

impl Bridge {
    pub fn ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}/agent", self.port)
    }

    /// The workspace the next/active connection runs against (defaults to the
    /// value passed at `start`, e.g. $HOME on first launch).
    pub async fn current_workspace(&self) -> PathBuf {
        self.workspace.lock().await.clone()
    }

    /// Point the runtime at a new workspace: drop the active connection so the
    /// webview auto-reconnects into a fresh `serve()` bound to `dir`.
    pub async fn set_workspace(&self, dir: PathBuf) {
        *self.workspace.lock().await = dir;
        if let Some(task) = self.current.lock().await.take() {
            task.abort();
        }
    }
}

pub async fn start(
    workspace: PathBuf,
    config_path: PathBuf,
    base_url: String,
    model: String,
) -> std::io::Result<Arc<Bridge>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let bridge = Arc::new(Bridge {
        port,
        current: Mutex::new(None),
        workspace: Arc::new(Mutex::new(workspace)),
        config_path,
        base_url,
        model,
    });

    let b = bridge.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let ws = match tokio_tungstenite::accept_async(stream).await {
                Ok(ws) => ws,
                Err(_) => continue,
            };
            let dir = b.workspace.lock().await.clone();
            let params = agent_server::setup::local_params(
                dir,
                b.config_path.clone(),
                b.base_url.clone(),
                b.model.clone(),
            );
            let task = tokio::spawn(async move {
                let _ = agent_server::daemon::serve(ws, params).await;
            });
            *b.current.lock().await = Some(task);
        }
    });

    Ok(bridge)
}
