use agent_model::{NativeProtocol, PromptedJsonProtocol, ToolCallProtocol};
use agent_tools::fs::{EditFile, ListDirectory, ReadFile, WriteFile};
use agent_tools::{git::{GitCommit, GitDiff, GitStatus}, shell::ExecuteCommand, ToolRegistry};
use std::sync::Arc;

#[allow(dead_code)]
pub fn protocol_name_is_valid(name: &str) -> bool {
    matches!(name, "native" | "prompted")
}

pub fn pick_protocol(name: &str) -> Arc<dyn ToolCallProtocol> {
    match name {
        "prompted" => Arc::new(PromptedJsonProtocol),
        _ => Arc::new(NativeProtocol),
    }
}

pub fn build_registry() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(ReadFile));
    r.register(Arc::new(WriteFile));
    r.register(Arc::new(EditFile));
    r.register(Arc::new(ListDirectory));
    r.register(Arc::new(ExecuteCommand));
    r.register(Arc::new(GitStatus));
    r.register(Arc::new(GitDiff));
    r.register(Arc::new(GitCommit));
    r
}

pub fn default_allowlist() -> Vec<String> {
    ["ls","cat","pwd","echo","git","grep","find","rg","cargo","head","tail","wc"]
        .into_iter().map(String::from).collect()
}
pub fn default_denylist() -> Vec<String> {
    ["rm -rf /","sudo",":(){","mkfs","dd if="].into_iter().map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pick_protocol_selects_by_name() {
        assert!(protocol_name_is_valid("native"));
        assert!(protocol_name_is_valid("prompted"));
        assert!(!protocol_name_is_valid("bogus"));
    }
    #[test]
    fn registry_has_all_core_tools() {
        let r = build_registry();
        for name in ["read_file","write_file","edit_file","list_directory",
                     "execute_command","git_status","git_diff","git_commit"] {
            assert!(r.get(name).is_some(), "missing {name}");
        }
    }
}
