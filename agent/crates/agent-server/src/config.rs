use serde::{Deserialize, Serialize};
use std::path::Path;

type DynErr = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub worker_url: String,
    pub agent_id: String,
    pub agent_token: String,
    pub pairing_code: String,
}

impl DaemonConfig {
    pub fn load(path: &Path) -> Result<Self, DynErr> {
        let text = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text)?)
    }
    pub fn save(&self, path: &Path) -> Result<(), DynErr> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Convert an http(s) worker URL into the daemon's ws(s) `/agent` endpoint.
pub fn ws_url(worker_url: &str) -> String {
    let base = worker_url.trim_end_matches('/');
    let base = base.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
    format!("{base}/agent")
}

#[derive(Serialize)]
struct EnrollReq<'a> { name: &'a str }
#[derive(Deserialize)]
struct EnrollResp { agent_id: String, agent_token: String, pairing_code: String }

/// Register this daemon with the Worker, returning persisted credentials.
pub async fn enroll(worker_url: &str, bootstrap_secret: &str, name: &str)
    -> Result<DaemonConfig, DynErr> {
    let url = format!("{}/enroll", worker_url.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(url)
        .header("X-Bootstrap-Secret", bootstrap_secret)
        .json(&EnrollReq { name })
        .send().await?
        .error_for_status()?
        .json::<EnrollResp>().await?;
    Ok(DaemonConfig {
        worker_url: worker_url.trim_end_matches('/').to_string(),
        agent_id: resp.agent_id,
        agent_token: resp.agent_token,
        pairing_code: resp.pairing_code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_swaps_scheme_and_appends_agent() {
        assert_eq!(ws_url("http://localhost:8787"), "ws://localhost:8787/agent");
        assert_eq!(ws_url("https://x.dev/"), "wss://x.dev/agent");
    }

    #[test]
    fn config_round_trips_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.json");
        let c = DaemonConfig { worker_url: "http://localhost:8787".into(),
            agent_id: "a1".into(), agent_token: "t".into(), pairing_code: "123456".into() };
        c.save(&p).unwrap();
        let back = DaemonConfig::load(&p).unwrap();
        assert_eq!(back.agent_id, "a1");
    }
}
