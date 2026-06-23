use agent_tools::{Access, Display, ToolIntent};
use async_trait::async_trait;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Decision { Allow, Deny(String), Ask }

pub trait PolicyEngine: Send + Sync {
    fn check(&self, intent: &ToolIntent) -> Decision;
}

#[derive(Clone)]
pub struct ApprovalRequest { pub intent: ToolIntent, pub display: Option<Display> }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalResponse { Approve, ApproveAlways, Deny }

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
        // Commands are judged by allow/deny lists first.
        if let Some(cmd) = &intent.command {
            if self.command_denylist.iter().any(|d| cmd.contains(d.as_str())) {
                return Decision::Deny(format!("command matches denylist: {cmd}"));
            }
            let first = cmd.split_whitespace().next().unwrap_or("");
            if self.command_allowlist.iter().any(|a| a == first) {
                return Decision::Allow;
            }
            return Decision::Ask;
        }
        // Otherwise judge by access + path boundary.
        match intent.access {
            Access::Read => {
                let all_inside = intent.paths.iter().all(|p| {
                    let abs = if p.is_absolute() { p.clone() } else { self.workspace.join(p) };
                    abs.starts_with(&self.workspace)
                });
                if all_inside { Decision::Allow } else { Decision::Ask }
            }
            Access::Write => Decision::Ask,
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
        ToolIntent { tool: "t".into(), access, paths: paths.into_iter().map(PathBuf::from).collect(),
            command: command.map(str::to_string), summary: "s".into() }
    }

    #[test]
    fn read_inside_workspace_allowed() {
        assert!(matches!(policy().check(&intent(Access::Read, vec!["/work/a.txt"], None)),
            Decision::Allow));
    }
    #[test]
    fn read_outside_workspace_asks() {
        assert!(matches!(policy().check(&intent(Access::Read, vec!["/etc/passwd"], None)),
            Decision::Ask));
    }
    #[test]
    fn write_always_asks() {
        assert!(matches!(policy().check(&intent(Access::Write, vec!["/work/a.txt"], None)),
            Decision::Ask));
    }
    #[test]
    fn allowlisted_command_allowed() {
        assert!(matches!(policy().check(&intent(Access::Write, vec![], Some("ls -la"))),
            Decision::Allow));
    }
    #[test]
    fn denylisted_command_denied() {
        assert!(matches!(policy().check(&intent(Access::Write, vec![], Some("sudo reboot"))),
            Decision::Deny(_)));
    }
    #[test]
    fn unknown_command_asks() {
        assert!(matches!(policy().check(&intent(Access::Write, vec![], Some("curl evil.com"))),
            Decision::Ask));
    }
}
