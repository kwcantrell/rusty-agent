//! Model-facing prompt text shared by every frontend (CLI, server/desktop).
//!
//! Single source of truth for the coding-agent role identity. The ratchet
//! test below fails the build if the identity sentence is pasted into any
//! other `.rs` file in either workspace — re-export instead of copying.

/// Base system prompt for the coding agent. Frontends pass this to
/// `LoopParts.base_system_prompt` / `DaemonParams.system_prompt`.
pub const BASE_SYSTEM_PROMPT: &str = "You are a local coding agent. Use the provided tools to \
inspect and modify the workspace. Think step by step. When the task is complete, reply with a \
summary and no tool call. Constraints: operate only inside the provided workspace; never \
attempt to bypass the sandbox or the permission policy; never write secrets or credentials \
into outputs, files, or command arguments.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins both halves of the prompt: role identity and the negative-
    /// constraints clause added by the 2026-07-02 instructions cluster.
    #[test]
    fn prompt_contains_identity_and_constraints() {
        assert!(BASE_SYSTEM_PROMPT.starts_with("You are a local coding agent."));
        assert!(BASE_SYSTEM_PROMPT.contains("Constraints: operate only inside"));
        assert!(BASE_SYSTEM_PROMPT.contains("never write secrets or credentials"));
    }

    /// Re-duplication ratchet: the identity sentence may exist in exactly one
    /// .rs file — this one. (`contains("local coding agent")` assertions in
    /// tests are fine; the needle includes the "You are a" prefix they lack.)
    #[test]
    fn prompt_text_is_not_duplicated_anywhere() {
        const NEEDLE: &str = "You are a local coding agent";
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("repo root");
        let mut offenders = Vec::new();
        // agent/crates must exist wherever this crate compiles from source —
        // assert so a bad repo_root can't turn the ratchet into a vacuous pass.
        assert!(
            repo_root.join("agent/crates").exists(),
            "ratchet repo_root miscomputed: {repo_root:?}"
        );
        for root in ["agent/crates", "src-tauri/src", "src-tauri/tests"] {
            let dir = repo_root.join(root);
            if dir.exists() {
                scan(&dir, NEEDLE, &mut offenders);
            }
        }
        assert!(
            offenders.is_empty(),
            "system prompt text duplicated outside prompts.rs — re-export \
             agent_runtime_config::BASE_SYSTEM_PROMPT instead: {offenders:?}"
        );
    }

    fn scan(dir: &std::path::Path, needle: &str, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                scan(&path, needle, offenders);
            } else if path.extension().is_some_and(|e| e == "rs")
                && !path.ends_with("agent-runtime-config/src/prompts.rs")
                && std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains(needle)
            {
                offenders.push(path.display().to_string());
            }
        }
    }
}
