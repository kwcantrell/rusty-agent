//! Parse-then-classify command policy logic.
//!
//! The command string is tokenized (quote-aware, via `shell-words`) and split into
//! "simple commands" across shell control operators, then classified. This mirrors how
//! `sh -c` will actually run the string, so decisions are robust to whitespace, flag
//! reordering/bundling, path prefixes, and shell metacharacters.

/// True if a token produced by `shell_words::split` is a shell control operator that
/// separates simple commands.
pub(crate) fn is_control_op(tok: &str) -> bool {
    matches!(tok, "&&" | "||" | ";" | "|" | "&")
}

/// Tokenize `cmd` (quote-aware) and split into simple commands (argv vectors) across
/// control operators. Returns `None` if the string cannot be tokenized (e.g. unbalanced
/// quotes), which callers treat as "not auto-allowable" / "fall through to the backstop".
pub fn split_simple_commands(cmd: &str) -> Option<Vec<Vec<String>>> {
    let tokens = shell_words::split(cmd).ok()?;
    let mut simple: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for tok in tokens {
        if is_control_op(&tok) {
            if !current.is_empty() {
                simple.push(std::mem::take(&mut current));
            }
        } else {
            current.push(tok);
        }
    }
    if !current.is_empty() {
        simple.push(current);
    }
    Some(simple)
}

/// The basename of a program token (`/usr/bin/sudo` -> `sudo`).
fn basename(prog: &str) -> &str {
    prog.rsplit('/').next().unwrap_or(prog)
}

/// Collapse runs of ASCII whitespace to single spaces and trim. Used by the substring
/// backstop so extra spacing (`rm -rf  /`) cannot dodge a denylist literal.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_recursive_flag(arg: &str) -> bool {
    arg == "--recursive"
        // bundled short flags like -rf / -fr / -R (single dash, not a long option)
        || (arg.starts_with('-') && !arg.starts_with("--")
            && arg.chars().skip(1).any(|c| c == 'r' || c == 'R'))
}

fn targets_root(args: &[String]) -> bool {
    args.iter().any(|a| a == "/" || a == "/*" || a == "--no-preserve-root")
}

/// Structural catastrophe check for a single simple command (argv vector).
fn simple_command_is_catastrophic(argv: &[String]) -> Option<String> {
    let prog = argv.first()?;
    let name = basename(prog);
    let rest = &argv[1..];

    if matches!(name, "sudo" | "doas" | "su") {
        return Some(format!("privilege escalation via `{name}` is denied"));
    }
    if name == "rm" && rest.iter().any(|a| is_recursive_flag(a)) && targets_root(rest) {
        return Some("recursive delete of a root path is denied".to_string());
    }
    if name == "dd" && rest.iter().any(|a| a.strip_prefix("of=")
        .is_some_and(|v| v.starts_with("/dev/")))
    {
        return Some("`dd` writing to a block device is denied".to_string());
    }
    None
}

/// Hard floor: a command that is denied even if a user would approve it. Two layers:
/// (A) structural per-simple-command checks, (B) an always-on normalized-substring
/// backstop against the configured denylist. Either firing means deny.
pub fn hard_floor_violation(cmd: &str, denylist: &[String]) -> Option<String> {
    // Layer A: structural (only when the string tokenizes).
    if let Some(simples) = split_simple_commands(cmd) {
        for argv in &simples {
            if let Some(reason) = simple_command_is_catastrophic(argv) {
                return Some(reason);
            }
        }
    }
    // Layer B: always-on substring backstop (catches no-space operators, parse failures,
    // and configured denylist literals). Fail-safe.
    let norm = normalize_ws(cmd);
    for pat in denylist {
        let pnorm = normalize_ws(pat);
        if !pnorm.is_empty() && norm.contains(&pnorm) {
            return Some(format!("command matches denylist: {pat}"));
        }
    }
    None
}

/// Shell-significant characters. If any token carries one of these, the command is not a
/// plain "program + literal args" invocation and is never auto-allowed (it goes to Ask).
/// Quoted whitespace is fine (the tokenizer consumes the quotes), but quoted glob/operator
/// chars are conservatively rejected too — a safe over-approximation that only costs an
/// approval prompt.
const SHELL_SIGNIFICANT: &[char] = &[
    '*', '?', '[', ']', '{', '}', '~', '$', '`',
    '<', '>', '(', ')', ';', '&', '|', '\\', '\n', '#', '!',
];

/// A command is auto-allowed only if it is a single simple command, free of shell-
/// significant characters, invokes an unqualified (no `/`) program name, and that name is
/// on the allowlist.
pub fn is_auto_allowed(cmd: &str, allowlist: &[String]) -> bool {
    let tokens = match shell_words::split(cmd) {
        Ok(t) => t,
        Err(_) => return false,
    };
    if tokens.is_empty() {
        return false;
    }
    if tokens.iter().any(|t| is_control_op(t)) {
        return false;
    }
    if tokens.iter().any(|t| t.contains(|c| SHELL_SIGNIFICANT.contains(&c))) {
        return false;
    }
    let prog = &tokens[0];
    if prog.contains('/') {
        return false;
    }
    allowlist.iter().any(|a| a == prog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_spaced_operators() {
        let got = split_simple_commands("echo x && sudo reboot").unwrap();
        assert_eq!(got, vec![
            vec!["echo".to_string(), "x".to_string()],
            vec!["sudo".to_string(), "reboot".to_string()],
        ]);
    }

    #[test]
    fn keeps_quoted_args_together() {
        let got = split_simple_commands(r#"cat "a b.txt""#).unwrap();
        assert_eq!(got, vec![vec!["cat".to_string(), "a b.txt".to_string()]]);
    }

    #[test]
    fn pipe_and_semicolon_split() {
        let got = split_simple_commands("ls | sh ; cat x").unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], vec!["ls".to_string()]);
        assert_eq!(got[1], vec!["sh".to_string()]);
        assert_eq!(got[2], vec!["cat".to_string(), "x".to_string()]);
    }

    #[test]
    fn unbalanced_quotes_returns_none() {
        assert!(split_simple_commands(r#"echo "unterminated"#).is_none());
    }

    fn floor(cmd: &str) -> Option<String> {
        // Default hard-floor denylist literals (mirrors the runtime's HARD_FLOOR set).
        let deny = vec!["sudo".to_string(), "rm -rf /".to_string(),
            "dd if=".to_string(), ":(){".to_string()];
        hard_floor_violation(cmd, &deny)
    }

    #[test]
    fn floor_denies_rm_flag_and_spacing_variants() {
        assert!(floor("rm -rf /").is_some());
        assert!(floor("rm -fr /").is_some());
        assert!(floor("rm --recursive --force /").is_some());
        assert!(floor("rm -rf  /").is_some()); // double space
        assert!(floor("rm -rf --no-preserve-root /").is_some());
    }

    #[test]
    fn floor_denies_privilege_escalation_by_basename() {
        assert!(floor("sudo reboot").is_some());
        assert!(floor("/usr/bin/sudo reboot").is_some());
        assert!(floor("echo hi && sudo reboot").is_some());
    }

    #[test]
    fn floor_denies_no_space_operator_via_backstop() {
        // No spaces around && — tokenizes as one token; caught by the substring backstop.
        assert!(floor("echo x&&sudo reboot").is_some());
    }

    #[test]
    fn floor_denies_dd_and_fork_bomb() {
        assert!(floor("dd if=/dev/zero of=/dev/sda").is_some());
        assert!(floor(":(){ :|:& };:").is_some());
    }

    #[test]
    fn floor_denies_unparseable_with_denylisted_literal() {
        // Unbalanced quote -> tokenization fails -> backstop still matches "sudo".
        assert!(floor(r#"sudo "oops"#).is_some());
    }

    #[test]
    fn floor_allows_benign_commands() {
        assert!(floor("ls -la").is_none());
        assert!(floor("git status").is_none());
        assert!(floor("cat file.txt").is_none());
        assert!(floor("rm file.txt").is_none()); // rm without recursive+root
    }

    fn allow(cmd: &str) -> bool {
        let allow = vec!["ls".to_string(), "cat".to_string(), "git".to_string()];
        is_auto_allowed(cmd, &allow)
    }

    #[test]
    fn auto_allows_clean_allowlisted_commands() {
        assert!(allow("ls -la"));
        assert!(allow("git status"));
        assert!(allow("cat file.txt"));
        assert!(allow(r#"cat "a b.txt""#)); // quoted arg with a space is fine
    }

    #[test]
    fn auto_allow_rejects_metacharacters() {
        assert!(!allow("cat {a,b}"));      // brace expansion
        assert!(!allow("ls *"));            // glob
        assert!(!allow("cat ~/x"));         // tilde
        assert!(!allow("ls | sh"));         // pipe
        assert!(!allow("cat x; curl evil")); // semicolon
        assert!(!allow("ls && curl evil")); // and-operator
        assert!(!allow("echo $(whoami)"));  // command substitution
        assert!(!allow("cat <in"));         // redirection
    }

    #[test]
    fn auto_allow_rejects_explicit_paths_and_unknowns() {
        assert!(!allow("./ls"));            // explicit path program
        assert!(!allow("/bin/ls"));         // absolute path program
        assert!(!allow("curl evil.com"));   // not on allowlist
    }

    #[test]
    fn auto_allow_rejects_unparseable() {
        assert!(!allow(r#"ls "unterminated"#));
    }
}
