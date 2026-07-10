use agent_tools::fs::resolve_in_workspace;
use agent_tools::{Access, Display, ToolIntent};
use async_trait::async_trait;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Decision {
    Allow,
    Deny(String),
    Ask,
}

pub trait PolicyEngine: Send + Sync {
    fn check(&self, intent: &ToolIntent) -> Decision;
}

#[derive(Clone)]
pub struct ApprovalRequest {
    pub intent: ToolIntent,
    pub display: Option<Display>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalResponse {
    Approve,
    ApproveAlways,
    Deny,
}

#[async_trait]
pub trait ApprovalChannel: Send + Sync {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse;
}

pub struct RulePolicy {
    pub workspace: PathBuf,
    pub command_allowlist: Vec<String>,
    pub command_denylist: Vec<String>,
}

impl PolicyEngine for RulePolicy {
    fn check(&self, intent: &ToolIntent) -> Decision {
        // Commands are judged by the parse-then-classify policy (see `command.rs`):
        // a two-layer hard floor first, then a deny-by-default auto-allow gate.
        if let Some(cmd) = &intent.command {
            if let Some(reason) = crate::command::hard_floor_violation(cmd, &self.command_denylist)
            {
                return Decision::Deny(reason);
            }
            // Destroy-declared intents are never auto-allowed, even when the command
            // itself is allowlisted — the tier's floor is Ask.
            if intent.access != Access::Destroy
                && crate::command::is_auto_allowed(cmd, &self.command_allowlist)
            {
                return Decision::Allow;
            }
            return Decision::Ask;
        }
        // Otherwise judge by access + path boundary.
        match intent.access {
            // TrustedWrite (pre-approved third-party mutation, e.g. MCP Trust::Allow)
            // shares Read's gate semantics: auto-allow inside the workspace boundary.
            // Post-exec validation — not this gate — is where its mutations surface.
            Access::Read | Access::TrustedWrite => {
                // Decide "inside workspace?" with the SAME resolver execute() uses, so the
                // approval gate and the execution guard can never disagree (resolve_in_workspace
                // collapses `.`/`..` before the boundary check). An escaping read -> Ask.
                let all_inside = intent
                    .paths
                    .iter()
                    .all(|p| resolve_in_workspace(&self.workspace, &p.to_string_lossy()).is_ok());
                if all_inside {
                    Decision::Allow
                } else {
                    Decision::Ask
                }
            }
            Access::Write => Decision::Ask,
            // Destroy never participates in any auto-allow; its floor is Ask.
            Access::Destroy => Decision::Ask,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_tools::{Access, ToolIntent};
    use std::path::PathBuf;

    fn policy() -> RulePolicy {
        RulePolicy {
            workspace: PathBuf::from("/work"),
            command_allowlist: vec!["ls".into(), "cat".into(), "git".into()],
            command_denylist: vec!["rm -rf /".into(), "sudo".into()],
        }
    }
    fn intent(access: Access, paths: Vec<&str>, command: Option<&str>) -> ToolIntent {
        ToolIntent {
            tool: "t".into(),
            access,
            paths: paths.into_iter().map(PathBuf::from).collect(),
            command: command.map(str::to_string),
            summary: "s".into(),
        }
    }

    #[test]
    fn read_inside_workspace_allowed() {
        assert!(matches!(
            policy().check(&intent(Access::Read, vec!["/work/a.txt"], None)),
            Decision::Allow
        ));
    }
    #[test]
    fn read_outside_workspace_asks() {
        assert!(matches!(
            policy().check(&intent(Access::Read, vec!["/etc/passwd"], None)),
            Decision::Ask
        ));
    }
    #[test]
    fn read_relative_dotdot_escape_asks() {
        // ../../etc/passwd joined to /work escapes; must require approval, not Allow.
        assert!(matches!(
            policy().check(&intent(Access::Read, vec!["../../etc/passwd"], None)),
            Decision::Ask
        ));
    }

    #[test]
    fn read_absolute_dotdot_escape_asks() {
        // /work/../etc normalizes to /etc — outside the workspace.
        assert!(matches!(
            policy().check(&intent(Access::Read, vec!["/work/../etc/x"], None)),
            Decision::Ask
        ));
    }

    #[test]
    fn read_dotdot_staying_inside_allows() {
        // sub/../a.txt normalizes to /work/a.txt — still inside.
        assert!(matches!(
            policy().check(&intent(Access::Read, vec!["sub/../a.txt"], None)),
            Decision::Allow
        ));
    }

    #[test]
    fn write_always_asks() {
        assert!(matches!(
            policy().check(&intent(Access::Write, vec!["/work/a.txt"], None)),
            Decision::Ask
        ));
    }

    // --- Task A5: memories/project/ mount policy pins (spec §2.6) ------------
    // No engine.rs code change: `memories/project/...` resolves as an ordinary
    // workspace-relative path, so the existing Read-Allow/Write-Ask gates
    // already yield these decisions. These tests only pin them.

    #[test]
    fn memory_mount_read_auto_allows() {
        assert!(matches!(
            policy().check(&intent(
                Access::Read,
                vec!["memories/project/index.md"],
                None
            )),
            Decision::Allow
        ));
    }
    #[test]
    fn memory_mount_write_asks() {
        assert!(matches!(
            policy().check(&intent(
                Access::Write,
                vec!["memories/project/index.md"],
                None
            )),
            Decision::Ask
        ));
    }
    #[test]
    fn memory_absolute_form_asks() {
        // Auto-allow depends on the workspace-relative rendering — a leading
        // slash must NOT be silently allowed (regression guard: every rendered
        // memory path in headers/pointers/discipline is slash-less on purpose;
        // reintroducing a leading slash would cause per-read approval fatigue).
        assert!(matches!(
            policy().check(&intent(
                Access::Read,
                vec!["/memories/project/index.md"],
                None
            )),
            Decision::Ask
        ));
    }

    #[test]
    fn allowlisted_command_allowed() {
        assert!(matches!(
            policy().check(&intent(Access::Write, vec![], Some("ls -la"))),
            Decision::Allow
        ));
    }
    #[test]
    fn denylisted_command_denied() {
        assert!(matches!(
            policy().check(&intent(Access::Write, vec![], Some("sudo reboot"))),
            Decision::Deny(_)
        ));
    }
    #[test]
    fn unknown_command_asks() {
        assert!(matches!(
            policy().check(&intent(Access::Write, vec![], Some("curl evil.com"))),
            Decision::Ask
        ));
    }
    #[test]
    fn allowlisted_command_with_shell_operator_asks() {
        assert!(matches!(
            policy().check(&intent(Access::Write, vec![], Some("ls && curl evil.com"))),
            Decision::Ask
        ));
    }
    #[test]
    fn allowlisted_command_with_pipe_asks() {
        assert!(matches!(
            policy().check(&intent(Access::Write, vec![], Some("ls | sh"))),
            Decision::Ask
        ));
    }
    #[test]
    fn allowlisted_command_with_semicolon_asks() {
        assert!(matches!(
            policy().check(&intent(Access::Write, vec![], Some("cat x; curl evil.com"))),
            Decision::Ask
        ));
    }

    #[test]
    fn floor_denies_rm_variants_through_check() {
        for cmd in ["rm -fr /", "rm --recursive --force /", "rm -rf  /"] {
            assert!(
                matches!(
                    policy().check(&intent(Access::Write, vec![], Some(cmd))),
                    Decision::Deny(_)
                ),
                "expected Deny for {cmd}"
            );
        }
    }

    #[test]
    fn metachar_commands_ask_through_check() {
        for cmd in ["cat {a,b}", "ls *", "cat ~/x"] {
            assert!(
                matches!(
                    policy().check(&intent(Access::Write, vec![], Some(cmd))),
                    Decision::Ask
                ),
                "expected Ask for {cmd}"
            );
        }
    }

    #[test]
    fn clean_allowlisted_still_allows_through_check() {
        assert!(matches!(
            policy().check(&intent(Access::Write, vec![], Some("ls -la"))),
            Decision::Allow
        ));
        assert!(matches!(
            policy().check(&intent(Access::Write, vec![], Some("git status"))),
            Decision::Allow
        ));
    }

    #[test]
    fn trusted_write_auto_allows_with_empty_paths() {
        assert!(matches!(
            policy().check(&intent(Access::TrustedWrite, vec![], None)),
            Decision::Allow
        ));
    }

    #[test]
    fn trusted_write_escaping_path_asks() {
        assert!(matches!(
            policy().check(&intent(Access::TrustedWrite, vec!["/etc/passwd"], None)),
            Decision::Ask
        ));
    }

    #[test]
    fn destroy_inside_workspace_still_asks() {
        // Destroy never participates in the Read-style inside-workspace auto-allow.
        assert!(matches!(
            policy().check(&intent(Access::Destroy, vec!["/work/a.txt"], None)),
            Decision::Ask
        ));
    }

    #[test]
    fn destroy_pathless_commandless_asks() {
        // Memory-shaped intent: a path-less, command-less `forget`.
        assert!(matches!(
            policy().check(&intent(Access::Destroy, vec![], None)),
            Decision::Ask
        ));
    }

    #[test]
    fn destroy_command_never_auto_allowed() {
        // "ls -la" is Allow for a Write intent (allowlisted); a Destroy intent skips the gate.
        assert!(matches!(
            policy().check(&intent(Access::Destroy, vec![], Some("ls -la"))),
            Decision::Ask
        ));
    }

    #[test]
    fn destroy_command_still_hits_hard_floor() {
        // Deny still beats Ask for Destroy-declared commands.
        assert!(matches!(
            policy().check(&intent(Access::Destroy, vec![], Some("sudo reboot"))),
            Decision::Deny(_)
        ));
    }
}
