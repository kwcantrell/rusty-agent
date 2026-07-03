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

/// Remove ALL ASCII whitespace. A second backstop pass uses this so spacing variants
/// (`: ( ) { :|:& } ; :`) cannot dodge a denylist literal like `:(){`.
fn strip_ws(s: &str) -> String {
    s.split_whitespace().collect::<String>()
}

fn is_recursive_flag(arg: &str) -> bool {
    arg == "--recursive"
        // bundled short flags like -rf / -fr / -R (single dash, not a long option)
        || (arg.starts_with('-') && !arg.starts_with("--")
            && arg.chars().skip(1).any(|c| c == 'r' || c == 'R'))
}

fn targets_root(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "/" || a == "/*" || a == "--no-preserve-root")
}

/// Catastrophe check keyed on a program's basename alone (no arguments needed): privilege-
/// escalation shims and filesystem-format tools. Shared by the structural per-simple-command
/// check (on argv[0]) and the raw-string boundary scan.
fn program_name_is_catastrophic(name: &str) -> Option<String> {
    if matches!(name, "sudo" | "doas" | "su") {
        return Some(format!("privilege escalation via `{name}` is denied"));
    }
    if name == "mkfs" || name.starts_with("mkfs.") {
        return Some(format!("filesystem creation via `{name}` is denied"));
    }
    None
}

/// Structural catastrophe check for a single simple command (argv vector).
fn simple_command_is_catastrophic(argv: &[String]) -> Option<String> {
    let prog = argv.first()?;
    let name = basename(prog);
    let rest = &argv[1..];

    if let Some(reason) = program_name_is_catastrophic(name) {
        return Some(reason);
    }
    if name == "rm" && rest.iter().any(|a| is_recursive_flag(a)) && targets_root(rest) {
        return Some("recursive delete of a root path is denied".to_string());
    }
    if name == "dd"
        && rest.iter().any(|a| {
            a.strip_prefix("of=")
                .is_some_and(|v| v.starts_with("/dev/"))
        })
    {
        return Some("`dd` writing to a block device is denied".to_string());
    }
    None
}

/// A /dev path that redirection may safely write to. Everything else under
/// /dev/ is a device write sink and is denied. Deny-by-default with a small
/// allowlist is the same fail-safe posture as the `dd of=` handler (which is
/// stricter still: it denies ALL of=/dev/* including /dev/null).
///
/// The target's leading path structure is normalized before the /dev match so
/// redundant `/`-runs and `.` segments cannot dodge it: `//dev/sda`,
/// `/./dev/sda`, and `/dev/./sda` all resolve to a write UNDER /dev and deny.
/// `..` is deliberately NOT collapsed — doing so could turn a deny into an
/// allow — so `..` forms (`/dev/../dev/sda`) keep denying via over-
/// approximation. Only an absolute path can name a device node, so a relative
/// `dev/sda` (an ordinary cwd file) is not this handler's concern. `/dev/shm/…`
/// is a standard world-writable tmpfs (files, not devices) and is allowed,
/// alongside the `/dev/fd/…` fds.
fn dev_redirect_target_is_safe(target: &str) -> bool {
    // Only an absolute path can name a /dev device node; a relative target is a
    // normal file and not this handler's concern.
    if !target.starts_with('/') {
        return true;
    }
    // Normalize the leading path structure: drop empty (`//`-run) and `.`
    // segments while PRESERVING `..` (collapsing it could unsafe-ify a deny).
    let segments: Vec<&str> = target
        .split('/')
        .filter(|seg| !seg.is_empty() && *seg != ".")
        .collect();
    // Must be a path UNDER /dev — first real segment `dev` plus at least one
    // more segment. Bare `/dev` is a directory, not a device write sink.
    if segments.first() != Some(&"dev") || segments.len() < 2 {
        return true; // not a (sub-)/dev path: not this handler's concern
    }
    // A `..` segment escapes upward (`/dev/shm/../sda` and `/dev/fd/../sda` both
    // resolve to /dev/sda). We do NOT resolve it; instead any /dev-rooted target
    // carrying `..` is denied (over-approximation) — this keeps the brief's `..`
    // deny posture AND stops `..` slipping past the `fd/`/`shm/` prefixes.
    if segments.contains(&"..") {
        return false;
    }
    let suffix = segments[1..].join("/");
    matches!(
        suffix.as_str(),
        "null"
            | "zero"
            | "full"
            | "random"
            | "urandom"
            | "stdin"
            | "stdout"
            | "stderr"
            | "tty"
            | "ptmx"
    ) || suffix.starts_with("fd/")
        || suffix.starts_with("shm/")
}

/// Strip a redirect-operator prefix from a token: optional fd digit-run or `&`,
/// then `>`, then one optional `>`, `|`, or `&`. Returns the glued remainder
/// ("" if the token was purely an operator), or None if the token is not a
/// redirect. The trailing `&` also covers the csh-style both-streams form
/// `>&file` (equivalent to `&>file`); fd-duplication like `2>&1` yields a
/// non-/dev target (`1`) and is harmless.
fn strip_redirect_op(tok: &str) -> Option<&str> {
    let t = tok.strip_prefix('&').unwrap_or(tok);
    let t = t.trim_start_matches(|c: char| c.is_ascii_digit());
    let t = t.strip_prefix('>')?;
    Some(t.strip_prefix(['>', '|', '&']).unwrap_or(t))
}

/// Structural check: a redirect targeting an unsafe /dev path anywhere in the
/// simple command (the tokenizer strips quotes, so a quoted ">" followed by a
/// /dev path is indistinguishable from real redirection — accepted fail-safe
/// over-approximation, same class as the A2 quote-blindness NOTE above).
fn redirect_catastrophe_in_argv(argv: &[String]) -> Option<String> {
    let mut i = 0;
    while i < argv.len() {
        if let Some(rest) = strip_redirect_op(&argv[i]) {
            let target = if rest.is_empty() {
                argv.get(i + 1).map(String::as_str).unwrap_or("")
            } else {
                rest
            };
            // No literal `/dev/` pre-gate: the predicate normalizes the target
            // itself, so `//dev/sda`, `/./dev/sda`, `/dev/./sda` are all caught.
            if !dev_redirect_target_is_safe(target) {
                return Some("redirection writing to a device file is denied".to_string());
            }
        }
        i += 1;
    }
    None
}

/// Raw-string backstop for redirects the tokenizer never sees (unbalanced
/// quotes) or quote-glued targets (`>"/dev/sda"`). After each `>` run, skip
/// `>`/`|`, whitespace, and leading quote chars; an unsafe /dev target denies.
/// Over-denial of `/dev/…` mentioned in quoted prose after a `>` is accepted —
/// the hard floor errs toward denial.
fn raw_redirect_catastrophe(cmd: &str) -> Option<String> {
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'>' {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j] == b'>' || bytes[j] == b'|' || bytes[j] == b'&') {
                j += 1;
            }
            while j < bytes.len() && (bytes[j] as char).is_ascii_whitespace() {
                j += 1;
            }
            while j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                j += 1;
            }
            let rest = &cmd[j..];
            // Extract the target token to the next shell delimiter and let the
            // normalized predicate decide. No literal `/dev/` pre-gate, so the
            // `//dev`, `/./dev`, `///dev` spellings enter the same as `/dev`.
            let end = rest
                .find(|c: char| {
                    c.is_ascii_whitespace() || matches!(c, '"' | '\'' | '&' | '|' | ';' | ')' | '`')
                })
                .unwrap_or(rest.len());
            if !dev_redirect_target_is_safe(&rest[..end]) {
                return Some("redirection writing to a device file is denied".to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

/// A `NAME=value` shell env-assignment prefix token (`FOO=bar` in `FOO=bar cmd`).
/// Such prefixes precede the real program, so the boundary scan skips them.
fn is_env_assignment(tok: &str) -> bool {
    match tok.find('=') {
        Some(eq) if eq > 0 => {
            let name = &tok[..eq];
            !name.contains('/')
                && name.chars().enumerate().all(|(i, c)| {
                    c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
                })
        }
        _ => false,
    }
}

/// Leading program token at each shell command boundary in the RAW string: start-of-string
/// and immediately after any boundary char (`&`, `|`, `;`, newline, `(`, `)`, `{`, `}`,
/// backtick). Operates on the raw text (not shell-words), so it works on glued operators
/// (`x&&sudo`), command substitution (`$(sudo …)`), subshells/groups (`(sudo …)`,
/// `{ sudo; }`), and unparseable input (unbalanced quotes) alike. Surrounding quote chars
/// are stripped so a quoted program name (`"sudo"`) is still caught. Leading `VAR=val`
/// env-assignment tokens are skipped so `FOO=bar sudo …` yields `sudo`.
///
/// NOTE: intentionally NOT quote-aware about operators — an operator or grouping character
/// inside a quoted string (`echo "a; sudo b"`) is treated as a boundary, so such a command
/// is over-denied. This is fail-safe (a hard floor errs toward denial), rare, and consistent
/// with the SHELL_SIGNIFICANT over-approximation elsewhere in this file. A full quote-aware
/// parser is deliberately out of scope.
fn command_boundary_programs(cmd: &str) -> impl Iterator<Item = &str> {
    cmd.split(['&', '|', ';', '\n', '(', ')', '{', '}', '`'])
        .filter_map(|seg| seg.split_whitespace().find(|&tok| !is_env_assignment(tok)))
        .map(|tok| tok.trim_matches(['"', '\'']))
        .filter(|tok| !tok.is_empty())
}

/// Hard floor: a command that is denied even if a user would approve it. Three layers:
/// (A) structural per-simple-command checks over shell-words tokenization; (A2) a raw-string
/// command-boundary scan for bare-program-name catastrophes hidden by glued operators or
/// unparseable input; (B) an always-on normalized-substring backstop against the configured
/// denylist. Any layer firing means deny.
pub fn hard_floor_violation(cmd: &str, denylist: &[String]) -> Option<String> {
    // Layer A: structural (only when the string tokenizes).
    if let Some(simples) = split_simple_commands(cmd) {
        for argv in &simples {
            if let Some(reason) = simple_command_is_catastrophic(argv) {
                return Some(reason);
            }
            if let Some(reason) = redirect_catastrophe_in_argv(argv) {
                return Some(reason);
            }
        }
    }
    // Layer A2: raw-string boundary scan — position-aware, so a catastrophe name in argument
    // position (`man mkfs`) is NOT flagged, but glued (`x&&sudo`) / unparseable (`sudo "oops`)
    // program-position uses that Layer A misses are caught.
    for prog in command_boundary_programs(cmd) {
        if let Some(reason) = program_name_is_catastrophic(basename(prog)) {
            return Some(reason);
        }
    }
    if let Some(reason) = raw_redirect_catastrophe(cmd) {
        return Some(reason);
    }
    // Layer B: always-on substring backstop (catches configured denylist literals, including
    // specific multi-token strings and the forkbomb signature). Fail-safe.
    let norm = normalize_ws(cmd);
    let stripped = strip_ws(cmd);
    for pat in denylist {
        let pnorm = normalize_ws(pat);
        if !pnorm.is_empty() && norm.contains(&pnorm) {
            return Some(format!("command matches denylist: {pat}"));
        }
        let pstripped = strip_ws(pat);
        if !pstripped.is_empty() && stripped.contains(&pstripped) {
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
    '*', '?', '[', ']', '{', '}', '~', '$', '`', '<', '>', '(', ')', ';', '&', '|', '\\', '\n',
    '#', '!',
];

/// A command is auto-allowed only if it is a single simple command, free of shell-
/// significant characters, invokes an unqualified (no `/`) program name, and matches an
/// allowlist entry. Entries are whitespace-token prefixes: `"ls"` matches any `ls`
/// invocation, while `"git status"` matches only that subcommand — `git push` et al.
/// fall through to Ask. Unknown subcommands of exec-capable programs fail safe to Ask.
/// Matched git `log`/`diff`/`show` invocations are additionally screened for
/// `--output`/`-o` (an arbitrary-file write) and fall to Ask.
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
    if tokens
        .iter()
        .any(|t| t.contains(|c| SHELL_SIGNIFICANT.contains(&c)))
    {
        return false;
    }
    // Even an allowlisted program must not auto-run a catastrophe program passed as an argument
    // (`find . -exec sudo reboot +`, `xargs mkfs`). Name-exact on each token's basename, so benign
    // substrings (`sudoku`, `pseudo`) are unaffected. These fall through to Ask, not Deny — the hard
    // floor stays position-aware.
    //
    // KNOWN LIMITATION: a catastrophe wrapped in a quoted interpreter argument (`bash -c "sudo x"`)
    // is a single token whose basename != the catastrophe name, so this guard cannot see it. Under
    // the default allowlist such wrappers aren't allowlisted, so those reach Ask. Do not add shell
    // interpreters or exec-capable arg runners (bash/sh/zsh/dash/eval/xargs) to command_allowlist.
    //
    // The same blind spot applies to allowlisted exec-CAPABLE programs — `git` (via `-c
    // core.pager=…`/`core.editor=…`/aliases/hooks, run through `sh -c`), `cargo` (build scripts,
    // aliases), `find -exec sh -c …`. They can run arbitrary sub-commands (including catastrophes)
    // that neither the position-aware layers nor this name-exact guard can inspect. ACCEPTED
    // RESIDUAL: the hard floor covers DIRECT catastrophe invocation, not catastrophes smuggled
    // through allowlisted exec vehicles. Mitigations: don't allowlist exec-capable programs if the
    // floor must hold, and rely on the execution sandbox (agent-sandbox).
    //
    // With prefix entries the DEFAULT allowlist no longer exposes bare `git`/`cargo`
    // (see agent-runtime-config::default_allowlist); the residual narrows to the
    // enumerated subcommands (`cargo build` still runs build scripts) and re-widens
    // only if a user adds a bare exec-capable entry back.
    if tokens
        .iter()
        .any(|t| program_name_is_catastrophic(basename(t)).is_some())
    {
        return false;
    }
    let prog = &tokens[0];
    if prog.contains('/') {
        return false;
    }
    // Allowlist entries are whitespace-split token prefixes: a one-word entry matches
    // the program name alone (legacy behavior); a multi-word entry ("git status") also
    // pins the leading arguments, so exec-capable programs can expose only read-safe
    // subcommands. Unknown subcommands fail safe to Ask. Degenerate (empty) entries
    // never match.
    let matched = allowlist.iter().any(|entry| {
        let want: Vec<&str> = entry.split_whitespace().collect();
        !want.is_empty()
            && want.len() <= tokens.len()
            && want
                .iter()
                .zip(tokens.iter())
                .all(|(w, t)| *w == t.as_str())
    });
    if !matched {
        return false;
    }
    // `git {log,diff,show} --output[=]<path>` truncates an arbitrary file — a write
    // hiding under read-safe prefixes. Scoped to those subcommands so read flags
    // stay allowed elsewhere (`git ls-files -o` = --others). `-o` has no meaning on
    // log/diff/show; scanning it too is belt-and-braces. `--output-indicator-*`
    // must not trip (rendering flags). Hit → Ask, never Deny.
    if tokens[0] == "git"
        && matches!(
            tokens.get(1).map(String::as_str),
            Some("log" | "diff" | "show")
        )
        && tokens[2..]
            .iter()
            .any(|t| t == "-o" || t == "--output" || t.starts_with("--output="))
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_spaced_operators() {
        let got = split_simple_commands("echo x && sudo reboot").unwrap();
        assert_eq!(
            got,
            vec![
                vec!["echo".to_string(), "x".to_string()],
                vec!["sudo".to_string(), "reboot".to_string()],
            ]
        );
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
        // Bare program names (sudo/mkfs) are NOT here — they are caught structurally &
        // position-aware, not by the substring backstop.
        let deny = vec![
            "rm -rf /".to_string(),
            "dd if=".to_string(),
            ":(){".to_string(),
        ];
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
    fn floor_denies_no_space_operator_via_boundary_scan() {
        // Glued && hides sudo from shell-words (one token `x&&sudo`); the raw-string
        // boundary scan splits on the operator and catches `sudo` in program position.
        assert!(floor("echo x&&sudo reboot").is_some());
    }

    #[test]
    fn floor_denies_dd_and_fork_bomb() {
        assert!(floor("dd if=/dev/zero of=/dev/sda").is_some());
        assert!(floor(":(){ :|:& };:").is_some());
    }

    #[test]
    fn floor_denies_mkfs_structurally() {
        // "mkfs" is NOT in the floor() denylist, so only the structural handler catches these.
        assert!(floor("mkfs /dev/sda").is_some());
        assert!(floor("mkfs.ext4 /dev/sdb1").is_some());
        assert!(floor("/sbin/mkfs.xfs /dev/sdb1").is_some());
        assert!(floor("echo hi && mkfs /dev/sda").is_some());
    }

    #[test]
    fn floor_allows_catastrophe_name_in_argument_position() {
        // The win: bare catastrophe names as ARGUMENTS are no longer over-denied.
        assert!(floor("man mkfs").is_none());
        assert!(floor("grep mkfs /var/log").is_none());
        assert!(floor("man sudo").is_none());
        assert!(floor("which sudo").is_none());
        assert!(floor("cat sudoku.txt").is_none()); // 'sudo' is a substring of 'sudoku'
    }

    #[test]
    fn floor_denies_catastrophe_name_in_program_position_via_boundary_scan() {
        assert!(floor("ls|mkfs /dev/sda").is_some()); // glued pipe
        assert!(floor("echo x&&mkfs /dev/sda").is_some()); // glued &&
        assert!(floor("\"sudo reboot").is_some()); // unbalanced quote before program name
    }

    #[test]
    fn floor_over_denies_quoted_operator_and_name_fail_safe() {
        // Accepted over-approximation: the boundary scan is not quote-aware about operators,
        // so an operator + catastrophe name both inside quotes is denied. Fail-safe & rare.
        assert!(floor(r#"echo "a; sudo b""#).is_some());
    }

    #[test]
    fn floor_denies_spaced_fork_bomb_via_stripped_backstop() {
        // Spaced variant dodges normalize_ws (single spaces remain) but not the
        // all-whitespace-removed pass, which collapses it to ":(){:|:&};:".
        assert!(floor(": ( ) { :|:& } ; :").is_some());
    }

    #[test]
    fn floor_allows_benign_despite_stricter_backstop() {
        assert!(floor("ls -la").is_none());
        assert!(floor("git status").is_none());
        assert!(floor("make build").is_none()); // 'mk' prefix must not trip mkfs
                                                // 'mkfs' as an argument (not program position) is fine in BOTH this test and prod:
                                                // the real HARD_FLOOR_DENYLIST no longer contains a bare "mkfs" substring.
        assert!(floor("cat mkfs-notes.txt").is_none());
    }

    #[test]
    fn floor_denies_unparseable_via_boundary_scan() {
        // Unbalanced quote -> shell-words fails -> Layer A skipped. The boundary scan runs
        // on the raw string and still finds `sudo` at the start.
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
        assert!(!allow("cat {a,b}")); // brace expansion
        assert!(!allow("ls *")); // glob
        assert!(!allow("cat ~/x")); // tilde
        assert!(!allow("ls | sh")); // pipe
        assert!(!allow("cat x; curl evil")); // semicolon
        assert!(!allow("ls && curl evil")); // and-operator
        assert!(!allow("echo $(whoami)")); // command substitution
        assert!(!allow("cat <in")); // redirection
    }

    #[test]
    fn auto_allow_rejects_explicit_paths_and_unknowns() {
        assert!(!allow("./ls")); // explicit path program
        assert!(!allow("/bin/ls")); // absolute path program
        assert!(!allow("curl evil.com")); // not on allowlist
    }

    #[test]
    fn auto_allow_rejects_unparseable() {
        assert!(!allow(r#"ls "unterminated"#));
    }

    #[test]
    fn auto_allow_rejects_catastrophe_token_in_allowlisted_command() {
        // `find`/`xargs` are exec-capable; a catastrophe program passed as their argument must
        // NOT auto-run. Name-exact on token basenames, so it goes to Ask (is_auto_allowed=false).
        let al = vec!["find".to_string(), "xargs".to_string(), "cat".to_string()];
        assert!(!is_auto_allowed("find . -exec sudo reboot +", &al));
        assert!(!is_auto_allowed("xargs mkfs", &al));
        // Name-exact: 'sudoku' is not the catastrophe name 'sudo' -> still auto-allowed.
        assert!(is_auto_allowed("cat sudoku.txt", &al));
    }

    #[test]
    fn interpreter_wrapping_reaches_ask_not_deny_or_allow() {
        // KNOWN LIMITATION: `bash -c "sudo reboot"` passes sudo as a quoted string the interpreter
        // runs. Position-aware layers can't see it; the name-exact guard can't either (the token is
        // "sudo reboot", basename != "sudo"). Under the DEFAULT allowlist (no interpreters) this
        // reaches Ask: NOT hard-denied, and NOT auto-allowed. Do not add interpreters to the allowlist.
        let floor = vec![
            "rm -rf /".to_string(),
            ":(){".to_string(),
            "dd if=".to_string(),
        ];
        let default_allow = vec![
            "ls", "cat", "pwd", "echo", "git", "grep", "find", "rg", "cargo", "head", "tail", "wc",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        assert!(hard_floor_violation(r#"bash -c "sudo reboot""#, &floor).is_none()); // not Deny
        assert!(!is_auto_allowed(r#"bash -c "sudo reboot""#, &default_allow)); // not Allow -> Ask
    }

    // --- Regression tests: under-denial cases fixed by widened boundary split + env-skip ---

    #[test]
    fn floor_denies_catastrophe_in_substitution_and_grouping() {
        assert!(floor("echo $(sudo reboot)").is_some());
        assert!(floor("echo `sudo reboot`").is_some());
        assert!(floor("(sudo reboot)").is_some());
        assert!(floor("{ sudo reboot; }").is_some());
        assert!(floor("echo $(mkfs.ext4 /dev/sda)").is_some());
    }

    #[test]
    fn floor_denies_catastrophe_behind_env_assignment() {
        assert!(floor("FOO=bar sudo reboot").is_some());
        assert!(floor("FOO=bar mkfs /dev/sda").is_some());
    }

    #[test]
    fn floor_still_allows_benign_substitution_and_assignment() {
        // Widened boundary set must not over-deny these benign forms.
        assert!(floor("echo $(date)").is_none());
        assert!(floor("(ls -la)").is_none());
        assert!(floor("FOO=bar make build").is_none());
    }

    #[test]
    fn auto_allow_rejects_env_prefixed_program() {
        assert!(!allow("FOO=bar sudo reboot")); // program token is FOO=bar, not allowlisted
    }

    #[test]
    fn auto_allow_exec_vehicle_residual_is_documented_not_a_regression() {
        // ACCEPTED RESIDUAL (see is_auto_allowed comment + spec Addendum 2): an allowlisted
        // exec-capable program runs sub-commands the floor cannot inspect. `git -c core.pager=…`
        // runs its value via `sh -c`. This is auto-allowed by design; pinned so any future change
        // that alters it is noticed and re-evaluated. Mitigation = allowlist policy + sandbox.
        let al = vec!["git".to_string()];
        assert!(is_auto_allowed(
            r#"git -c core.pager="sudo reboot" log"#,
            &al
        ));
    }

    #[test]
    fn prefix_entries_gate_subcommands() {
        let al = vec![
            "git status".to_string(),
            "git log".to_string(),
            "cargo build".to_string(),
        ];
        assert!(is_auto_allowed("git status", &al));
        assert!(is_auto_allowed("git status --porcelain -b", &al));
        assert!(is_auto_allowed("git log --oneline -5", &al));
        assert!(is_auto_allowed("cargo build --release", &al));
        // Destructive / unlisted subcommands are not auto-allowed (audit Top-10 #9).
        assert!(!is_auto_allowed("git push --force", &al));
        assert!(!is_auto_allowed("git reset --hard", &al));
        assert!(!is_auto_allowed("git clean -fdx", &al));
        assert!(!is_auto_allowed("cargo publish", &al));
        // Bare program does not match when only prefix entries exist.
        assert!(!is_auto_allowed("git", &al));
        // A flag before the subcommand breaks the prefix — accepted over-ask.
        assert!(!is_auto_allowed("git -C /tmp status", &al));
    }

    #[test]
    fn prefix_entry_longer_than_command_does_not_match() {
        let al = vec!["git status --short".to_string()];
        assert!(!is_auto_allowed("git status", &al));
        assert!(is_auto_allowed("git status --short", &al));
    }

    #[test]
    fn one_word_entries_keep_legacy_program_match() {
        let al = vec!["ls".to_string()];
        assert!(is_auto_allowed("ls -la", &al));
        assert!(!is_auto_allowed("lsblk", &al)); // token equality, not substring
    }

    #[test]
    fn degenerate_entries_never_match() {
        let al = vec!["".to_string(), "   ".to_string()];
        assert!(!is_auto_allowed("ls", &al));
    }

    // Mirrors default_allowlist()'s read-safe git prefixes plus grep.
    fn git_scan_fixture() -> Vec<String> {
        [
            "git log",
            "git diff",
            "git show",
            "git ls-files",
            "git status",
            "grep",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    #[test]
    fn git_output_flag_is_not_auto_allowed() {
        let al = git_scan_fixture();
        for cmd in [
            "git log --output=/tmp/x",
            "git diff --output /tmp/x",
            "git show --output=x HEAD",
            "git log -o x",
        ] {
            assert!(!is_auto_allowed(cmd, &al), "{cmd} must fall to Ask");
        }
    }

    #[test]
    fn git_output_scan_has_no_false_positives() {
        let al = git_scan_fixture();
        for cmd in [
            "git diff --output-indicator-new=+",
            "git log --oneline",
            "git ls-files -o",
            "git status",
            "grep -o pat file",
        ] {
            assert!(is_auto_allowed(cmd, &al), "{cmd} must stay auto-allowed");
        }
    }

    // --- Redirect-to-/dev hard-floor handler (closes the `echo x > /dev/sda` Ask→Deny gap) ---

    #[test]
    fn redirect_to_block_device_is_denied() {
        for cmd in [
            "echo x > /dev/sda",
            "echo x >/dev/sda",
            "echo x >> /dev/nvme0n1",
            "cmd 2>>/dev/sda",
            "cmd &>/dev/sda",
            "cmd >|/dev/sda",
            "echo x >\"/dev/sda\"",
            "echo x > /dev/sda \"unbalanced", // raw backstop: unparseable
            "git log > /dev/mem",
            "cmd 2> /dev/sda",       // split operator/target pair
            "echo x > /dev/ttyUSB0", // deny-by-default (matches dd posture)
            "echo x > /dev/sda1",
        ] {
            assert!(
                hard_floor_violation(cmd, &[]).is_some(),
                "expected deny: {cmd}"
            );
        }
    }

    #[test]
    fn safe_dev_and_plain_file_redirects_are_not_denied() {
        for cmd in [
            "cmd 2>/dev/null",
            "echo x > /dev/stdout",
            "cmd > /dev/fd/3",
            "echo hi > out.txt",
            "echo hi >> notes.md",
            "grep pattern file", // no redirect at all
        ] {
            assert!(
                hard_floor_violation(cmd, &[]).is_none(),
                "must not deny: {cmd}"
            );
        }
    }

    #[test]
    fn dev_redirect_denial_reason_names_device_write() {
        let r = hard_floor_violation("echo x > /dev/sda", &[]).unwrap();
        assert!(
            r.contains("redirection writing to a device file is denied"),
            "{r}"
        );
    }

    #[test]
    fn redirect_both_streams_amp_gt_form_is_denied() {
        // Adversarial probe: `>&file` / `>& file` is bash's csh-style redirect of BOTH
        // stdout+stderr to a file (mirror of the `&>` form the brief covers). Verified
        // against real bash. These bypassed the initial handler (target parsed as `&…`);
        // the trailing-`&` strip in strip_redirect_op + the raw skip-run close them.
        for cmd in [
            "echo x >& /dev/sda",  // split-pair, both-streams
            "echo x >&/dev/sda",   // glued, both-streams
            "cmd 2>& /dev/sda",    // fd-prefixed both-streams, split
            "echo x >&\"/dev/sda", // unbalanced quote -> raw backstop only
        ] {
            assert!(
                hard_floor_violation(cmd, &[]).is_some(),
                "expected deny (>& device write): {cmd}"
            );
        }
    }

    #[test]
    fn redirect_path_normalization_bypasses_are_denied() {
        // Linux collapses `/`-runs and `.` segments, so these WRITE to /dev/sda
        // yet the old literal `/dev/` prefix compare missed them. Normalizing the
        // target's leading path structure closes the bypass.
        for cmd in [
            "echo x > //dev/sda",
            "echo x >//dev/sda", // glued, doubled slash
            "echo x > ///dev/sda",
            "echo x > /./dev/sda",
            "echo x > /dev/./sda", // `.` segment between dev and the device
            "echo x > /dev/../dev/sda", // `..` kept (over-approx) → still denies
            "echo x > /dev/shm/../sda", // `..` escapes /dev/shm back to a device
            "echo x > /dev/fd/../sda", // `..` escapes /dev/fd back to a device
        ] {
            assert!(
                hard_floor_violation(cmd, &[]).is_some(),
                "expected deny (normalized /dev write): {cmd}"
            );
        }
    }

    #[test]
    fn normalized_safe_dev_and_shm_are_not_denied() {
        // Normalization must not over-deny: safe /dev sinks reached via redundant
        // `/`/`.` segments stay safe, and /dev/shm is a world-writable tmpfs (a
        // file, not a device) so writes under it are allowed.
        for cmd in [
            "echo x > /dev/shm/f",     // tmpfs file, not a device
            "echo x > //dev/null",     // doubled slash, safe sink
            "cmd > /dev/./null",       // `.` segment, safe sink
            "echo x > /dev/fd/3",      // fd still safe after normalization
            "echo x > /dev/shm/sub/f", // nested tmpfs path
        ] {
            assert!(
                hard_floor_violation(cmd, &[]).is_none(),
                "must not deny (safe after normalization): {cmd}"
            );
        }
    }

    #[test]
    fn redirect_fd_duplication_and_common_forms_are_not_denied() {
        // No false positives from the trailing-`&` strip: fd duplication (`2>&1`) targets
        // an fd, not a device, and the ubiquitous `> /dev/null 2>&1` idiom must stay clean.
        for cmd in [
            "cmd 2>&1",
            "cmd > /dev/null 2>&1",
            "cmd >&2",
            "echo hi > out.txt 2>&1",
        ] {
            assert!(
                hard_floor_violation(cmd, &[]).is_none(),
                "must not deny (fd dup / null idiom): {cmd}"
            );
        }
    }
}
