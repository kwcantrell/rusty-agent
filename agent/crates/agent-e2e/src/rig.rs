//! Isolation Rig: tempdir-rooted workspace/sessions/metadata roots + a
//! pre-created secret, so e2e tests never touch `~/.rusty-agent` and never
//! race `load_or_create_secret`'s first-touch write.
use agent_runtime_config::RuntimeConfig;
use agent_server::session::Session;
use agent_server::testkit::Captured;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

pub struct Rig {
    pub workspace: TempDir,
    pub sessions: TempDir,
    pub meta: TempDir,
    pub key: [u8; 32],
}

impl Rig {
    pub fn new() -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let meta = tempfile::tempdir().unwrap();
        // Pre-create the secret: kills load_or_create_secret's first-touch race
        // and gives tests the key for (test-local) MAC forging.
        let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(3));
        let secret = meta.path().join("secret");
        std::fs::write(&secret, key).unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        Rig {
            workspace,
            sessions,
            meta,
            key,
        }
    }

    pub fn runtime_config(&self, base_url: &str) -> RuntimeConfig {
        let mut c = RuntimeConfig::from_launch(
            "openai".into(),
            base_url.into(),
            "stub-model".into(),
            "native".into(),
            32_768,
        );
        c.trace_dir = Some(self.sessions.path().to_string_lossy().into_owned());
        c.metadata_dir = Some(self.meta.path().to_string_lossy().into_owned());
        c
    }

    /// The GUI leg: a Session constructed exactly the way src-tauri's bridge
    /// does, pointed at this rig's roots, with a capturing sink attached.
    pub fn session(&self, base_url: &str) -> (Arc<Session>, Arc<Captured>) {
        self.session_with_model(base_url, "stub-model")
    }

    /// Same as `session`, but with an explicit model name — for live-model
    /// legs (`live_smoke.rs`) where the requested name should match what's
    /// actually loaded rather than the stubbed scripts' fixed "stub-model".
    pub fn session_with_model(&self, base_url: &str, model: &str) -> (Arc<Session>, Arc<Captured>) {
        let cfg_path = self.meta.path().join("agent-runtime.json");
        // File overlay carries the same overrides so an overlay can't undo them.
        std::fs::write(
            &cfg_path,
            serde_json::json!({
                "trace_dir": self.sessions.path().to_string_lossy(),
                "metadata_dir": self.meta.path().to_string_lossy(),
            })
            .to_string(),
        )
        .unwrap();
        let mut params = agent_server::setup::local_params(
            self.workspace.path().to_path_buf(),
            cfg_path,
            base_url.into(),
            model.into(),
        );
        params.config = self.runtime_config(base_url);
        let session = Session::from_params(params);
        let cap = Arc::new(Captured::default());
        session.set_event_out(cap.clone());
        (session, cap)
    }

    pub fn session_dirs(&self) -> Vec<PathBuf> {
        let mut v: Vec<_> = std::fs::read_dir(self.sessions.path())
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    pub fn only_session_dir(&self) -> PathBuf {
        let dirs = self.session_dirs();
        assert_eq!(
            dirs.len(),
            1,
            "expected exactly one session dir, got {dirs:?}"
        );
        dirs.into_iter().next().unwrap()
    }

    pub fn assert_parked(&self, dir: &Path) {
        assert!(
            ckpt(dir).join("parked.json").exists(),
            "positive-artifact rule: parked.json missing in {}",
            ckpt(dir).display()
        );
    }
}

impl Default for Rig {
    fn default() -> Self {
        Self::new()
    }
}

/// Root-ask checkpoint dir: ALL park artifacts (parked.json, manifest.json,
/// answer.json, resume.lock) live here, never on the session dir itself (F2).
pub fn ckpt(session_dir: &Path) -> PathBuf {
    session_dir.join("checkpoint")
}

/// Bounded poll — the only generic waiter (spec §2.4 watchdog policy).
pub fn wait_until(cap: Duration, poll: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < cap {
        if poll() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Async twin of `wait_until` for use directly inside tokio tests without
/// blocking the executor thread.
pub async fn wait_until_async(cap: Duration, poll: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < cap {
        if poll() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn rig_creates_isolated_roots_and_secret() {
        let rig = Rig::new();
        assert_eq!(
            std::fs::read(rig.meta.path().join("secret")).unwrap().len(),
            32
        );
        let cfg = rig.runtime_config("http://127.0.0.1:1");
        assert_eq!(
            agent_runtime_config::metadata_root_for(&cfg).unwrap(),
            rig.meta.path()
        );
        let (session, _cap) = rig.session("http://127.0.0.1:1");
        // Constructing a Session must have created its descriptor under the rig,
        // not under ~/.rusty-agent.
        assert!(!rig.session_dirs().is_empty());
        drop(session);
    }
}
