//! Persisted desktop app config (currently just the chosen workspace dir).
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub workspace: Option<PathBuf>,
}

impl AppConfig {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_round_trips_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config/app.json");
        let cfg = AppConfig { workspace: Some(PathBuf::from("/home/u/proj")) };
        cfg.save(&p).unwrap();
        let back = AppConfig::load(&p);
        assert_eq!(back.workspace, Some(PathBuf::from("/home/u/proj")));
    }

    #[test]
    fn missing_file_loads_default() {
        let back = AppConfig::load(Path::new("/no/such/file.json"));
        assert!(back.workspace.is_none());
    }
}
