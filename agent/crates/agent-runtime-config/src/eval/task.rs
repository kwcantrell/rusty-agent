use serde::{Deserialize, Serialize};

/// A file to materialize in the eval workspace before the run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedFile {
    pub path: String,
    pub contents: String,
}

/// One session of the task. A fresh window (new `CuratedContext`) is built per
/// session; the memory store is shared across sessions, so a fact stored in an
/// earlier session must be recalled from memory in a later one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSpec {
    /// Ordered user turns to run in this session (each is one `agent.run`).
    pub prompts: Vec<String>,
}

/// A frozen, context-management-bound task with a hidden test oracle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    /// Failure mode this task stresses: "drift" | "offload" | "compaction" |
    /// "recall" | "memory-under-recall" | "memory-over-recall".
    pub mode: String,
    pub realistic_window: usize,
    pub favorable_window: usize,
    pub memory_enabled: bool,
    pub seed_files: Vec<SeedFile>,
    /// Command run AFTER the agent finishes, cwd = workspace, with the hidden tests
    /// copied in. Exit code 0 == pass.
    pub test_cmd: String,
    pub sessions: Vec<SessionSpec>,
}

impl TaskSpec {
    pub fn from_json(s: &str) -> serde_json::Result<TaskSpec> {
        serde_json::from_str(s)
    }
    /// More than one session => a fact must survive a fresh (empty) window via memory.
    pub fn is_cross_session(&self) -> bool {
        self.sessions.len() > 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const JSON: &str = r#"{
      "id": "drift-ledger",
      "mode": "drift",
      "realistic_window": 16000,
      "favorable_window": 196608,
      "memory_enabled": false,
      "seed_files": [{ "path": "ledger.txt", "contents": "start: 0\n" }],
      "test_cmd": "bash hidden_tests/check.sh",
      "sessions": [{ "prompts": ["step 1", "step 2"] }]
    }"#;
    #[test]
    fn parses_and_detects_single_session() {
        let t = TaskSpec::from_json(JSON).unwrap();
        assert_eq!(t.id, "drift-ledger");
        assert_eq!(t.realistic_window, 16000);
        assert_eq!(t.seed_files[0].path, "ledger.txt");
        assert!(!t.is_cross_session());
    }
    #[test]
    fn two_sessions_is_cross_session() {
        let mut t = TaskSpec::from_json(JSON).unwrap();
        t.sessions.push(SessionSpec {
            prompts: vec!["recall it".into()],
        });
        assert!(t.is_cross_session());
    }
}
