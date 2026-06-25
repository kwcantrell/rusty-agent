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
}
