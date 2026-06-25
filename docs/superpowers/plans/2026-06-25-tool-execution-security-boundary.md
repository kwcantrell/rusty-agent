# Tool-Execution Security Boundary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the command policy, sandbox mount guard, and git tool enforce the security boundary at the point where the action actually executes, closing the audit's verified bypasses.

**Architecture:** Three independent changes sharing one theme. (1) `agent-policy` replaces raw-string substring/first-token matching with a parse-then-classify model in a new `command.rs` module. (2) `agent-sandbox` replaces its tiny mount blocklist with a deny-by-default sensitive-path guard. (3) `agent-tools` routes the git helper through `ctx.sandbox` instead of spawning on the host.

**Tech Stack:** Rust (workspace, edition 2021), `tokio`, `async-trait`, `thiserror`, and the `shell-words` crate (new) for quote-aware tokenization.

## Global Constraints

- Edition: `2021` (workspace `[workspace.package] edition = "2021"`).
- Dependencies are declared in the workspace root `agent/Cargo.toml` `[workspace.dependencies]` and referenced from crates with `name.workspace = true`. Follow this pattern for any new dependency.
- Tests are dense, colocated `#[cfg(test)] mod tests` blocks in the same file as the code. Match this style — do not create separate test files.
- Run all commands from the `agent/` directory (the Cargo workspace root). `cargo` may not be on PATH — run `source ~/.cargo/env` first in each shell if `cargo` is not found.
- No changes to `agent-core`, `agent-memory`, `agent-server`, or the memory auto-retrieval feature.
- `RulePolicy`'s public fields (`workspace`, `command_allowlist`, `command_denylist`) and the `PolicyEngine::check` signature do not change.

---

## File Structure

- `agent/Cargo.toml` — add `shell-words` to `[workspace.dependencies]`.
- `agent/crates/agent-policy/Cargo.toml` — add `shell-words.workspace = true`.
- `agent/crates/agent-policy/src/command.rs` — **new.** Pure command-classification logic: tokenization, hard-floor (two-layer), auto-allow gate. Self-contained with its own tests.
- `agent/crates/agent-policy/src/lib.rs` — declare `mod command;`.
- `agent/crates/agent-policy/src/engine.rs` — `RulePolicy::check` calls `command.rs` helpers for the command path; path-boundary logic unchanged.
- `agent/crates/agent-sandbox/src/mounts.rs` — rewrite `validate_mount`'s blocklist into a deny-by-default sensitive-path guard + `:` rejection.
- `agent/crates/agent-tools/src/git.rs` — route `git()` through `ctx.sandbox`; add a recording fake executor in tests.

---

## Task 1: Add `shell-words` and the command tokenizer

**Files:**
- Modify: `agent/Cargo.toml` (`[workspace.dependencies]`)
- Modify: `agent/crates/agent-policy/Cargo.toml` (`[dependencies]`)
- Create: `agent/crates/agent-policy/src/command.rs`
- Modify: `agent/crates/agent-policy/src/lib.rs` (add `mod command;`)

**Interfaces:**
- Produces: `pub fn split_simple_commands(cmd: &str) -> Option<Vec<Vec<String>>>` — quote-aware tokenization split into simple commands across the control operators `&&`, `||`, `;`, `|`, `&`; returns `None` if the string cannot be tokenized.
- Produces: `pub(crate) fn is_control_op(tok: &str) -> bool`.

- [ ] **Step 1: Add the workspace dependency**

In `agent/Cargo.toml`, under `[workspace.dependencies]`, add after the `similar = "2"` line:

```toml
shell-words = "1"
```

- [ ] **Step 2: Reference it from agent-policy**

In `agent/crates/agent-policy/Cargo.toml`, under `[dependencies]`, add:

```toml
shell-words.workspace = true
```

- [ ] **Step 3: Declare the new module**

`agent/crates/agent-policy/src/lib.rs` currently reads:

```rust
//! Permission policy engine and approval channel abstraction.
mod engine;
pub use engine::*;
```

Add `mod command;` so it becomes:

```rust
//! Permission policy engine and approval channel abstraction.
mod engine;
mod command;
pub use engine::*;
```

`command` stays an internal module; `engine.rs` reaches its functions via `crate::command::...` (as written in Task 4).

- [ ] **Step 4: Write the failing test**

Create `agent/crates/agent-policy/src/command.rs` with only the test module and a stub:

```rust
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
    todo!()
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
```

- [ ] **Step 5: Run the test to verify it fails**

Run: `cd agent && cargo test -p agent-policy command::tests 2>&1 | tail -20`
Expected: compiles, then panics with `not yet implemented` (the `todo!()`), tests FAIL.

- [ ] **Step 6: Implement `split_simple_commands`**

Replace the `todo!()` body:

```rust
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
```

- [ ] **Step 7: Run the test to verify it passes**

Run: `cd agent && cargo test -p agent-policy command::tests 2>&1 | tail -20`
Expected: all 4 tests PASS.

- [ ] **Step 8: Commit**

```bash
cd agent && git add Cargo.toml Cargo.lock crates/agent-policy/Cargo.toml crates/agent-policy/src/command.rs crates/agent-policy/src/lib.rs
git commit -m "feat(agent-policy): quote-aware command tokenizer (shell-words)"
```

---

## Task 2: Hard-floor structural deny + substring backstop

**Files:**
- Modify: `agent/crates/agent-policy/src/command.rs`

**Interfaces:**
- Consumes: `split_simple_commands` (Task 1).
- Produces: `pub fn hard_floor_violation(cmd: &str, denylist: &[String]) -> Option<String>` — returns `Some(reason)` if the command must be denied, else `None`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `command.rs`:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd agent && cargo test -p agent-policy command::tests 2>&1 | tail -20`
Expected: compile error — `hard_floor_violation` not found.

- [ ] **Step 3: Implement the hard floor**

Add to `command.rs` (above the `tests` module):

```rust
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
    let prog = match argv.first() {
        Some(p) => p,
        None => return None,
    };
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd agent && cargo test -p agent-policy command::tests 2>&1 | tail -25`
Expected: all `command::tests` (Task 1 + Task 2) PASS.

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-policy/src/command.rs
git commit -m "feat(agent-policy): two-layer hard-floor (structural + substring backstop)"
```

---

## Task 3: Auto-allow gate (deny-by-default)

**Files:**
- Modify: `agent/crates/agent-policy/src/command.rs`

**Interfaces:**
- Consumes: `is_control_op` (Task 1).
- Produces: `pub fn is_auto_allowed(cmd: &str, allowlist: &[String]) -> bool` — `true` only for a single simple command with no operators/expansions/redirections, an unqualified program name in `allowlist`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `command.rs`:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd agent && cargo test -p agent-policy command::tests 2>&1 | tail -20`
Expected: compile error — `is_auto_allowed` not found.

- [ ] **Step 3: Implement the auto-allow gate**

Add to `command.rs` (above the `tests` module):

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd agent && cargo test -p agent-policy command::tests 2>&1 | tail -25`
Expected: all `command::tests` PASS.

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-policy/src/command.rs
git commit -m "feat(agent-policy): deny-by-default auto-allow gate"
```

---

## Task 4: Wire the new model into `RulePolicy::check`

**Files:**
- Modify: `agent/crates/agent-policy/src/engine.rs:30-46` (the command branch of `check`)

**Interfaces:**
- Consumes: `hard_floor_violation` (Task 2), `is_auto_allowed` (Task 3).
- Produces: unchanged `PolicyEngine::check` behavior for non-command intents.

- [ ] **Step 1: Write the failing tests**

Add to the existing `tests` module in `engine.rs` (it already has `policy()` and `intent()` helpers):

```rust
    #[test]
    fn floor_denies_rm_variants_through_check() {
        for cmd in ["rm -fr /", "rm --recursive --force /", "rm -rf  /"] {
            assert!(matches!(policy().check(&intent(Access::Write, vec![], Some(cmd))),
                Decision::Deny(_)), "expected Deny for {cmd}");
        }
    }

    #[test]
    fn metachar_commands_ask_through_check() {
        for cmd in ["cat {a,b}", "ls *", "cat ~/x"] {
            assert!(matches!(policy().check(&intent(Access::Write, vec![], Some(cmd))),
                Decision::Ask), "expected Ask for {cmd}");
        }
    }

    #[test]
    fn clean_allowlisted_still_allows_through_check() {
        assert!(matches!(policy().check(&intent(Access::Write, vec![], Some("ls -la"))),
            Decision::Allow));
        assert!(matches!(policy().check(&intent(Access::Write, vec![], Some("git status"))),
            Decision::Allow));
    }
```

The `policy()` helper's denylist is `vec!["rm -rf /".into(), "sudo".into()]`; the structural layer is what makes `rm -fr /` and `rm --recursive --force /` deny, so these tests fail against the current substring-only code.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd agent && cargo test -p agent-policy engine::tests 2>&1 | tail -25`
Expected: `floor_denies_rm_variants_through_check` FAILS (current code returns `Ask` for `rm -fr /`).

- [ ] **Step 3: Replace the command branch of `check`**

In `engine.rs`, replace the command block (currently lines ~32-46, from `if let Some(cmd) = &intent.command {` through its closing `}` that ends with `return Decision::Ask;`) with:

```rust
        if let Some(cmd) = &intent.command {
            if let Some(reason) = crate::command::hard_floor_violation(cmd, &self.command_denylist) {
                return Decision::Deny(reason);
            }
            if crate::command::is_auto_allowed(cmd, &self.command_allowlist) {
                return Decision::Allow;
            }
            return Decision::Ask;
        }
```

Leave the `match intent.access { ... }` path-boundary block below it untouched.

- [ ] **Step 4: Run the full policy test suite to verify it passes**

Run: `cd agent && cargo test -p agent-policy 2>&1 | tail -30`
Expected: all tests PASS, including the pre-existing `denylisted_command_denied`, `allowlisted_command_with_pipe_asks`, etc., and the three new tests.

Note: the pre-existing `denylisted_command_denied` test (`sudo reboot`) still passes via both layers; `allowlisted_command_with_shell_operator_asks` / `_with_pipe_asks` / `_with_semicolon_asks` pass via the auto-allow gate's operator rejection.

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-policy/src/engine.rs
git commit -m "feat(agent-policy): parse-then-classify in RulePolicy::check"
```

---

## Task 5: Deny-by-default mount guard

**Files:**
- Modify: `agent/crates/agent-sandbox/src/mounts.rs` (`validate_mount` body + tests)

**Interfaces:**
- Produces: unchanged `pub fn validate_mount(path: &str, home: Option<&Path>) -> Result<PathBuf, SandboxError>` signature; stricter behavior.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `mounts.rs`:

```rust
    #[test]
    fn rejects_system_dirs_and_descendants() {
        assert!(validate_mount("/etc", None).is_err());
        assert!(validate_mount("/etc/ssl", None).is_err()); // descendant
        assert!(validate_mount("/usr/bin", None).is_err());
        assert!(validate_mount("/var/lib/docker", None).is_err());
    }

    #[test]
    fn rejects_home_credential_dirs() {
        let home = std::env::temp_dir().join("agent-sbx-home-creds");
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        assert!(validate_mount(ssh.to_str().unwrap(), Some(&home)).is_err());
        // a descendant of a credential dir is also rejected
        let key = ssh.join("id_rsa.d");
        std::fs::create_dir_all(&key).unwrap();
        assert!(validate_mount(key.to_str().unwrap(), Some(&home)).is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn rejects_path_containing_colon() {
        let dir = std::env::temp_dir().join("agent-sbx-colon:dir");
        // canonicalize requires existence; create it if the FS allows ':' (Linux does).
        if std::fs::create_dir_all(&dir).is_ok() {
            assert!(validate_mount(dir.to_str().unwrap(), None).is_err());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn still_accepts_ordinary_project_dir_under_home() {
        let home = std::env::temp_dir().join("agent-sbx-home-ok");
        let proj = home.join("projects").join("app");
        std::fs::create_dir_all(&proj).unwrap();
        let got = validate_mount(proj.to_str().unwrap(), Some(&home)).unwrap();
        assert_eq!(got, proj.canonicalize().unwrap());
        let _ = std::fs::remove_dir_all(&home);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd agent && cargo test -p agent-sandbox mounts 2>&1 | tail -25`
Expected: `rejects_system_dirs_and_descendants` and `rejects_home_credential_dirs` FAIL (current guard allows them).

- [ ] **Step 3: Rewrite the guard**

Replace the body of `validate_mount` in `mounts.rs` (keep the `~` expansion block at the top unchanged; replace everything from the `canonicalize()` call to the final `Ok(canon)`):

```rust
    let canon = expanded.canonicalize()
        .map_err(|e| SandboxError::InvalidMount(format!("{}: {e}", expanded.display())))?;

    // A ':' in the path corrupts the `docker -v src:dst:mode` argument (src == dst here).
    if canon.to_string_lossy().contains(':') {
        return Err(SandboxError::InvalidMount(
            format!("path contains ':': {}", canon.display())));
    }

    // The filesystem root is rejected on exact match only (every absolute path is a
    // descendant of "/", so a prefix check there would reject everything).
    if canon == Path::new("/") {
        return Err(SandboxError::InvalidMount("refusing to mount /".into()));
    }
    // System roots whose mount (or any descendant) would breach the sandbox boundary.
    const SYSTEM_ROOTS: &[&str] = &[
        "/etc", "/usr", "/bin", "/sbin", "/lib", "/lib64", "/boot",
        "/sys", "/proc", "/dev", "/var/lib/docker",
        "/run", "/var/run", "/var/run/docker.sock", "/run/docker.sock",
    ];
    for root in SYSTEM_ROOTS {
        let r = Path::new(root);
        if canon == r || canon.starts_with(r) {
            return Err(SandboxError::InvalidMount(format!("refusing to mount {}", canon.display())));
        }
    }

    if let Some(h) = home {
        // Canonicalize home so a symlinked $HOME prefix can't dodge the checks.
        let h_canon = h.canonicalize();
        let h_cmp = h_canon.as_deref().unwrap_or(h);
        if canon == h_cmp {
            return Err(SandboxError::InvalidMount("refusing to mount \\$HOME root".into()));
        }
        // Credential directories under HOME (and their descendants) stay off-limits,
        // while ordinary project dirs elsewhere under HOME remain allowed.
        const HOME_SECRETS: &[&str] = &[
            ".ssh", ".aws", ".gnupg", ".kube", ".docker",
            ".config/gcloud", ".netrc", ".git-credentials",
        ];
        for sub in HOME_SECRETS {
            let secret = h_cmp.join(sub);
            if canon == secret || canon.starts_with(&secret) {
                return Err(SandboxError::InvalidMount(
                    format!("refusing to mount credential path {}", canon.display())));
            }
        }
    }

    Ok(canon)
```

(The `/` root and docker-socket cases are now covered by `SYSTEM_ROOTS`, so the old dedicated `if canon == Path::new("/")` and the `for bad in [...docker.sock...]` loop are removed by this replacement.)

- [ ] **Step 4: Run the full sandbox test suite to verify it passes**

Run: `cd agent && cargo test -p agent-sandbox mounts 2>&1 | tail -30`
Expected: all `mounts` tests PASS, including the pre-existing `rejects_root_and_home_root_and_socket`, `accepts_a_real_subdir`, `expands_tilde_subdir`, `rejects_symlinked_home_root`.

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-sandbox/src/mounts.rs
git commit -m "feat(agent-sandbox): deny-by-default mount guard + colon rejection"
```

---

## Task 6: Route the git tool through the sandbox

**Files:**
- Modify: `agent/crates/agent-tools/src/git.rs` (the `git()` helper + tests)

**Interfaces:**
- Consumes: `ctx.sandbox: Arc<dyn SandboxStrategy>`, `CommandSpec`, `ProcKind`, `SandboxError`, `SandboxedChild` (all from `agent_tools::sandbox`, re-exported at crate root).
- Produces: unchanged `git()` signature `async fn git(ctx: &ToolCtx, args: &[&str]) -> Result<String, ToolError>`.

- [ ] **Step 1: Write the failing test (recording executor)**

Add to the `tests` module in `git.rs`:

```rust
    use std::sync::{Arc, Mutex};

    /// A SandboxStrategy that records the CommandSpec it received, then delegates to the
    /// host so the real git command still runs.
    struct RecordingExecutor {
        calls: Arc<Mutex<Vec<crate::CommandSpec>>>,
    }
    impl crate::SandboxStrategy for RecordingExecutor {
        fn launch(&self, spec: crate::CommandSpec)
            -> Result<crate::SandboxedChild, crate::SandboxError> {
            self.calls.lock().unwrap().push(spec.clone());
            crate::HostExecutor.launch(spec)
        }
        fn describe(&self) -> crate::SandboxDescriptor { crate::HostExecutor.describe() }
    }

    fn recording_ctx(ws: std::path::PathBuf)
        -> (ToolCtx, Arc<Mutex<Vec<crate::CommandSpec>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let ctx = ToolCtx {
            workspace: ws,
            timeout: Duration::from_secs(10),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(RecordingExecutor { calls: calls.clone() }),
        };
        (ctx, calls)
    }

    #[tokio::test]
    async fn git_dispatches_through_sandbox() {
        let dir = init_repo();
        let (ctx, calls) = recording_ctx(dir.path().into());
        let _ = GitStatus.execute(json!({}), &ctx).await.unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "git_status should launch exactly one sandbox process");
        assert_eq!(calls[0].program, "git");
        assert_eq!(calls[0].args.first().map(String::as_str), Some("status"));
        assert_eq!(calls[0].cwd, dir.path());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd agent && cargo test -p agent-tools git:: 2>&1 | tail -25`
Expected: `git_dispatches_through_sandbox` FAILS — `calls` is empty because `git()` still spawns directly via `tokio::process::Command`.

- [ ] **Step 3: Rewrite the `git()` helper to use the sandbox**

Replace the `git()` function at the top of `git.rs` (lines ~5-16) with:

```rust
async fn git(ctx: &ToolCtx, args: &[&str]) -> Result<String, ToolError> {
    use tokio::io::AsyncReadExt;
    let spec = crate::CommandSpec {
        program: "git".into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        cwd: ctx.workspace.clone(),
        env: Default::default(),
        kind: crate::ProcKind::OneShot,
    };
    let mut child = ctx.sandbox.launch(spec).map_err(|e| match e {
        crate::SandboxError::Unavailable(m) => ToolError::Denied(m),
        other => ToolError::Failed { message: other.to_string(), stderr: None },
    })?;

    let mut out_pipe = child.take_stdout();
    let mut err_pipe = child.take_stderr();
    let read_out = async {
        let mut s = String::new();
        if let Some(p) = out_pipe.as_mut() { let _ = p.read_to_string(&mut s).await; }
        s
    };
    let read_err = async {
        let mut s = String::new();
        if let Some(p) = err_pipe.as_mut() { let _ = p.read_to_string(&mut s).await; }
        s
    };
    // Drain both pipes CONCURRENTLY with wait() to avoid a pipe-buffer deadlock.
    let run = async {
        let (status, stdout, stderr) = tokio::join!(child.wait(), read_out, read_err);
        (status, stdout, stderr)
    };
    let (status, stdout, stderr) = tokio::select! {
        _ = ctx.cancel.cancelled() => return Err(ToolError::Denied("cancelled".into())),
        r = tokio::time::timeout(ctx.timeout, run) => match r {
            Err(_elapsed) => return Err(ToolError::Timeout),
            Ok((status, stdout, stderr)) => (
                status.map_err(|e| ToolError::Failed { message: e.to_string(), stderr: None })?,
                stdout, stderr),
        }
    };
    if status.success() {
        Ok(stdout)
    } else {
        Err(ToolError::Failed {
            message: format!("git {} failed", args.join(" ")),
            stderr: Some(stderr) })
    }
}
```

- [ ] **Step 4: Run the full git test suite to verify it passes**

Run: `cd agent && cargo test -p agent-tools git:: 2>&1 | tail -30`
Expected: all `git` tests PASS — the new `git_dispatches_through_sandbox` plus the pre-existing `git_status_reports_untracked` and `git_commit_commits_staged_changes` (which use the default `HostExecutor` ctx and still run git on the host).

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-tools/src/git.rs
git commit -m "feat(agent-tools): route git tool through ctx.sandbox"
```

---

## Final verification

- [ ] **Step 1: Build and test the whole workspace**

Run: `cd agent && cargo test 2>&1 | tail -40`
Expected: all crates compile and all tests PASS.

- [ ] **Step 2: Clippy clean on the touched crates**

Run: `cd agent && cargo clippy -p agent-policy -p agent-sandbox -p agent-tools 2>&1 | tail -30`
Expected: no warnings introduced by the new code.

---

## Self-Review (completed during authoring)

**Spec coverage:**
- Component 1 (parse-then-classify policy): Tasks 1–4. Two-layer floor → Task 2; deny-by-default auto-allow → Task 3; wiring → Task 4. ✓
- Component 2 (deny-by-default mounts + `:` rejection): Task 5. ✓
- Component 3 (git through sandbox): Task 6. ✓
- Out-of-scope HTTP redirect: intentionally excluded (backlog). ✓
- Testing section bypass cases: rm variants (T2/T4), glob/brace/tilde (T3/T4), credential-dir + `:` mounts (T5), git-through-sandbox recording test (T6). ✓

**Type consistency:** `split_simple_commands`, `hard_floor_violation`, `is_auto_allowed`, `is_control_op` are defined in Task 1–3 and consumed by the same names in Task 4. `CommandSpec`/`ProcKind`/`SandboxError`/`SandboxedChild`/`SandboxStrategy`/`SandboxDescriptor`/`HostExecutor` names match the `agent-tools` exports verified in the source. `validate_mount` signature unchanged.

**Known residual (documented in spec, not a gap):** allowlisting a program trusts all its flags (`git -c …`); per-program arg rules are a future extension.
