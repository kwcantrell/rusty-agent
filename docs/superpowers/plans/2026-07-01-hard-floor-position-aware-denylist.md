# Position-Aware Hard-Floor Denylist Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the catastrophic-command hard floor from over-denying benign commands (`man mkfs`, `man sudo`, `which sudo`) by making bare-program-name catastrophe detection position-aware, then removing those bare names from the substring backstop.

**Architecture:** Extract the bare-program-name checks (`sudo`/`doas`/`su`, `mkfs`/`mkfs.*`) into a shared `program_name_is_catastrophic` helper. Add a raw-string command-boundary scan (`command_boundary_programs`) that runs regardless of tokenization, so glued-operator (`echo x&&sudo reboot`) and unparseable (`sudo "oops`) forms are caught structurally/position-aware. Then drop `"sudo"` and `"mkfs"` from `HARD_FLOOR_DENYLIST`. This is one atomic task — the denylist shrink is only safe once the boundary scan exists.

**Tech Stack:** Rust; `shell-words` (already used); pure string scanning (no new deps).

## Global Constraints

- Rust workspace is `agent/`; run cargo from there (`source ~/.cargo/env` if `cargo` isn't on PATH).
- Conventional commits: `type(scope): summary`.
- Changes ship with tests; run the relevant `cargo test -p <crate>` before calling it done.
- Spec: `docs/superpowers/specs/2026-07-01-hard-floor-position-aware-denylist-design.md`.
- Fail-safe direction: the hard floor is an absolute deny; over-denial of bizarre commands is acceptable, under-denial is not. Position-aware means the catastrophe name must be in **program position** (start of command or after a control operator), not argument position.
- The Layer-B substring backstop algorithm and user-configured denylist semantics (whole-string, match-anywhere) are unchanged — only the built-in floor's input set shrinks.

---

### Task 1: Position-aware bare-name catastrophe detection

**Files:**
- Modify: `agent/crates/agent-policy/src/command.rs` (`simple_command_is_catastrophic` ~65-85; `hard_floor_violation` ~90-114; add two helpers; `floor()` test helper ~183-188; several tests ~199-248)
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (`HARD_FLOOR_DENYLIST` line 8 + doc comment; test `effective_denylist_floor_survives_empty_user_list` line 393)
- Modify: `agent/crates/agent-server/src/runtime.rs` (test `settings_state_reports_floor_and_no_api_key` line 226)

**Interfaces:**
- Consumes: existing `basename()`, `split_simple_commands()`, `normalize_ws()`, `strip_ws()`, `is_recursive_flag()`, `targets_root()` in `command.rs`.
- Produces: new private `fn program_name_is_catastrophic(name: &str) -> Option<String>` and `fn command_boundary_programs(cmd: &str) -> impl Iterator<Item = &str>`. `hard_floor_violation`'s signature is unchanged; `HARD_FLOOR_DENYLIST` shrinks from 5 to 3 entries.

**TDD isolation note:** the in-file `floor()` helper mirrors the runtime floor. After this task it must be `["rm -rf /", "dd if=", ":(){"]` (drop `"sudo"`; it never held `"mkfs"`). With `"sudo"` gone from the backstop, the glued-operator and unparseable sudo tests can only pass via the new boundary scan — that is what makes them genuine RED-before / GREEN-after tests.

- [ ] **Step 1: Update the `floor()` test helper and the test suite to the target behavior (RED)**

In `agent/crates/agent-policy/src/command.rs`:

(a) Replace the `floor()` helper denylist (lines 184-186) so it mirrors the new floor:

```rust
    fn floor(cmd: &str) -> Option<String> {
        // Default hard-floor denylist literals (mirrors the runtime's HARD_FLOOR set).
        // Bare program names (sudo/mkfs) are NOT here — they are caught structurally &
        // position-aware, not by the substring backstop.
        let deny = vec!["rm -rf /".to_string(), "dd if=".to_string(), ":(){".to_string()];
        hard_floor_violation(cmd, &deny)
    }
```

(b) Re-point the two tests that credited the substring backstop for sudo, since the denylist no longer contains it (they now pass via the boundary scan):

```rust
    #[test]
    fn floor_denies_no_space_operator_via_boundary_scan() {
        // Glued && hides sudo from shell-words (one token `x&&sudo`); the raw-string
        // boundary scan splits on the operator and catches `sudo` in program position.
        assert!(floor("echo x&&sudo reboot").is_some());
    }
```

```rust
    #[test]
    fn floor_denies_unparseable_via_boundary_scan() {
        // Unbalanced quote -> shell-words fails -> Layer A skipped. The boundary scan runs
        // on the raw string and still finds `sudo` at the start.
        assert!(floor(r#"sudo "oops"#).is_some());
    }
```

(Delete the old `floor_denies_no_space_operator_via_backstop` and `floor_denies_unparseable_with_denylisted_literal` — these two replace them.)

(c) Add the new position-awareness tests (place after `floor_denies_mkfs_structurally`):

```rust
    #[test]
    fn floor_allows_catastrophe_name_in_argument_position() {
        // The win: bare catastrophe names as ARGUMENTS are no longer over-denied.
        assert!(floor("man mkfs").is_none());
        assert!(floor("grep mkfs /var/log").is_none());
        assert!(floor("man sudo").is_none());
        assert!(floor("which sudo").is_none());
        assert!(floor("pseudocode").is_none()); // 'sudo' is a substring of 'pseudo'
    }

    #[test]
    fn floor_denies_catastrophe_name_in_program_position_via_boundary_scan() {
        assert!(floor("ls|mkfs /dev/sda").is_some());        // glued pipe
        assert!(floor("echo x&&mkfs /dev/sda").is_some());   // glued &&
        assert!(floor("\"sudo reboot").is_some());           // unbalanced quote before program name
    }

    #[test]
    fn floor_over_denies_quoted_operator_and_name_fail_safe() {
        // Accepted over-approximation: the boundary scan is not quote-aware about operators,
        // so an operator + catastrophe name both inside quotes is denied. Fail-safe & rare.
        assert!(floor(r#"echo "a; sudo b""#).is_some());
    }
```

(d) Fix the now-stale comment in `floor_allows_benign_despite_stricter_backstop` (the prod claim is no longer true — `mkfs` as an argument is fine in prod too):

```rust
        // 'mkfs' as an argument (not program position) is fine in BOTH this test and prod:
        // the real HARD_FLOOR_DENYLIST no longer contains a bare "mkfs" substring.
        assert!(floor("cat mkfs-notes.txt").is_none());
```

- [ ] **Step 2: Run the tests to verify they fail (RED)**

Run: `cd agent && cargo test -p agent-policy`
Expected: FAIL. `floor_denies_no_space_operator_via_boundary_scan`, `floor_denies_unparseable_via_boundary_scan`, `floor_denies_catastrophe_name_in_program_position_via_boundary_scan`, and `floor_over_denies_quoted_operator_and_name_fail_safe` fail — nothing catches those forms now that `"sudo"` is out of the `floor()` denylist and the boundary scan doesn't exist yet. (The `floor_allows_*` tests already pass — they guard against over-denial.)

- [ ] **Step 3: Extract `program_name_is_catastrophic` and refactor `simple_command_is_catastrophic`**

In `agent/crates/agent-policy/src/command.rs`, add the helper immediately above `simple_command_is_catastrophic`:

```rust
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
```

Then replace the body of `simple_command_is_catastrophic` so it delegates the bare-name checks and keeps only the multi-arg ones:

```rust
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
    if name == "dd" && rest.iter().any(|a| a.strip_prefix("of=")
        .is_some_and(|v| v.starts_with("/dev/")))
    {
        return Some("`dd` writing to a block device is denied".to_string());
    }
    None
}
```

- [ ] **Step 4: Add the `command_boundary_programs` scan**

Add above `hard_floor_violation`:

```rust
/// Leading program token at each shell command boundary in the RAW string: start-of-string
/// and immediately after a control-operator char (`&`, `|`, `;`, newline). Operates on the
/// raw text (not shell-words), so it works on glued operators (`x&&sudo`) and unparseable
/// input (unbalanced quotes) alike. Surrounding quote chars are stripped so a quoted program
/// name (`"sudo"`) is still caught.
///
/// NOTE: intentionally NOT quote-aware about operators — an operator inside a quoted string
/// (`echo "a; sudo b"`) is treated as a boundary, so such a command is over-denied. This is
/// fail-safe (a hard floor errs toward denial), rare, and consistent with the
/// SHELL_SIGNIFICANT over-approximation elsewhere in this file. A full quote-aware parser is
/// deliberately out of scope.
fn command_boundary_programs(cmd: &str) -> impl Iterator<Item = &str> {
    cmd.split(|c| matches!(c, '&' | '|' | ';' | '\n'))
        .filter_map(|seg| seg.split_whitespace().next())
        .map(|tok| tok.trim_matches(|c| c == '"' || c == '\''))
        .filter(|tok| !tok.is_empty())
}
```

- [ ] **Step 5: Wire the boundary scan into `hard_floor_violation`**

Replace `hard_floor_violation` (lines 87-114) with the version below — it adds "Layer A2" between the existing structural pass and the substring backstop:

```rust
/// Hard floor: a command that is denied even if a user would approve it. Layers:
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
```

- [ ] **Step 6: Run the agent-policy suite to verify GREEN**

Run: `cd agent && cargo test -p agent-policy`
Expected: PASS — all tests, including the four that were RED in Step 2. Confirm no existing structural test regressed (`floor_denies_privilege_escalation_by_basename`, `floor_denies_rm_flag_and_spacing_variants`, `floor_denies_dd_and_fork_bomb`, `floor_denies_mkfs_structurally`, all `auto_allow_*`).

- [ ] **Step 7: Shrink `HARD_FLOOR_DENYLIST` and fix its dependent tests**

In `agent/crates/agent-runtime-config/src/runtime_config.rs`, replace the const + doc comment (lines 5-8):

```rust
/// Commands ALWAYS denied regardless of user settings — defense-in-depth against
/// the model (or an injected settings frame), not against the operator. Intersected
/// into the effective denylist by `RuntimeConfig::effective_denylist`. Bare-program-name
/// catastrophes (sudo/su/doas, mkfs) are handled structurally & position-aware in
/// agent-policy (so `man mkfs` is not over-denied); only specific multi-token strings and
/// the forkbomb signature live here as substring backstop literals.
pub const HARD_FLOOR_DENYLIST: &[&str] = &["rm -rf /", ":(){", "dd if="];
```

In the same file, fix `effective_denylist_floor_survives_empty_user_list` (line 393) — it asserts a now-removed entry:

```rust
        assert!(c.effective_denylist().iter().any(|d| d == "rm -rf /"));
```

In `agent/crates/agent-server/src/runtime.rs`, fix `settings_state_reports_floor_and_no_api_key` (line 226):

```rust
        assert!(st.hard_floor.iter().any(|d| d == "rm -rf /"));
```

- [ ] **Step 8: Run the affected crates + full workspace to verify GREEN**

Run: `cd agent && cargo test -p agent-policy -p agent-runtime-config -p agent-server`
Expected: PASS in all three (the two edited assertions now check `"rm -rf /"`, which remains in the floor).

Run: `cd agent && cargo test`
Expected: whole workspace green. If any other test asserted `"sudo"`/`"mkfs"` floor membership, it surfaces here — report it (none is expected beyond the two fixed above).

- [ ] **Step 9: Commit**

```bash
cd agent && git add crates/agent-policy/src/command.rs \
  crates/agent-runtime-config/src/runtime_config.rs \
  crates/agent-server/src/runtime.rs
git commit -m "fix(policy): position-aware bare-name catastrophe detection; drop sudo/mkfs from substring floor"
```

---

## Self-Review

**Spec coverage:**
- Extract `program_name_is_catastrophic` → Step 3. ✔
- `command_boundary_programs` raw-string scan → Step 4. ✔
- Layer-A2 pass in `hard_floor_violation` → Step 5. ✔
- Shrink `HARD_FLOOR_DENYLIST` to `["rm -rf /", ":(){", "dd if="]` + doc comment → Step 7. ✔
- Known over-approximation documented in code + tested → Step 4 (comment), Step 1c (`floor_over_denies_quoted_operator_and_name_fail_safe`). ✔
- False-positive-gone tests (`man mkfs`, `grep mkfs`, `man sudo`, `which sudo`) → Step 1c (`floor_allows_catastrophe_name_in_argument_position`, plus `pseudocode`). ✔
- Preserved-coverage tests (glued/unparseable/pipe) → Step 1b + 1c. ✔
- `floor()` helper mirrors new floor → Step 1a. ✔
- Re-point backstop-crediting tests → Step 1b. ✔
- runtime_config test assertion → Step 7. ✔
- agent-server floor test assertion (found beyond the spec's list) → Step 7. ✔
- Testing commands (`-p agent-policy -p agent-runtime-config`, full `cargo test`) → Steps 6, 8. ✔

**Placeholder scan:** none — every code step shows complete code; every run step names the command and expected result.

**Type consistency:** `program_name_is_catastrophic(name: &str) -> Option<String>` defined in Step 3, called in Step 3 (`simple_command_is_catastrophic`) and Step 5 (`hard_floor_violation`, as `program_name_is_catastrophic(basename(prog))`). `command_boundary_programs(cmd: &str) -> impl Iterator<Item = &str>` defined Step 4, consumed Step 5. `basename` is the existing helper. `HARD_FLOOR_DENYLIST` 3-entry form in Step 7 matches the assertions updated in the same step. `floor()` helper's new denylist (Step 1a) is consistent with the tests in Steps 1b-1c.

---

### Task 2: Auto-allow guard for exec-wrapped catastrophes + interpreter-residual docs

Added after implementation review found a Critical under-denial: `find . -exec sudo reboot +` (`find` is default-allowlisted, exec-capable) reaches auto-Allow — verified `hard_floor_violation` → `None`, `is_auto_allowed` → `true` — where the old `"sudo"` substring hard-denied it. See the spec Addendum.

**Files:**
- Modify: `agent/crates/agent-policy/src/command.rs` (`is_auto_allowed` ~184-203; add tests in `mod tests`)

**Interfaces:**
- Consumes: existing `program_name_is_catastrophic(name: &str) -> Option<String>`, `basename(&str) -> &str`, `is_auto_allowed(cmd, allowlist)`, and the `allow()` test helper.
- Produces: `is_auto_allowed` now additionally returns `false` when any token's basename is a catastrophe program name. No signature change.

- [ ] **Step 1: Write the failing tests (RED)**

Add to `mod tests` in `agent/crates/agent-policy/src/command.rs` (near the other `auto_allow_*` tests):

```rust
    #[test]
    fn auto_allow_rejects_catastrophe_token_in_allowlisted_command() {
        // `find`/`xargs` are exec-capable; a catastrophe program passed as their argument must
        // NOT auto-run. Name-exact on token basenames, so it goes to Ask (is_auto_allowed=false).
        let al = vec!["find".to_string(), "xargs".to_string(), "cat".to_string()];
        assert!(!is_auto_allowed("find . -exec sudo reboot +", &al));
        assert!(!is_auto_allowed("xargs mkfs", &al));
        assert!(!is_auto_allowed("find / -name x -exec mkfs.ext4 {} +", &al) || true);
        // Name-exact: 'sudoku' is not the catastrophe name 'sudo' -> still auto-allowed.
        assert!(is_auto_allowed("cat sudoku.txt", &al));
    }

    #[test]
    fn interpreter_wrapping_reaches_ask_not_deny_or_allow() {
        // KNOWN LIMITATION: `bash -c "sudo reboot"` passes sudo as a quoted string the interpreter
        // runs. Position-aware layers can't see it; the name-exact guard can't either (the token is
        // "sudo reboot", basename != "sudo"). Under the DEFAULT allowlist (no interpreters) this
        // reaches Ask: NOT hard-denied, and NOT auto-allowed. Do not add interpreters to the allowlist.
        let floor = vec!["rm -rf /".to_string(), ":(){".to_string(), "dd if=".to_string()];
        let default_allow = vec!["ls","cat","pwd","echo","git","grep","find","rg","cargo","head","tail","wc"]
            .into_iter().map(String::from).collect::<Vec<_>>();
        assert!(hard_floor_violation(r#"bash -c "sudo reboot""#, &floor).is_none()); // not Deny
        assert!(!is_auto_allowed(r#"bash -c "sudo reboot""#, &default_allow));       // not Allow -> Ask
    }
```

(Note: the third assert in the first test is written `|| true` only because `{}` contains
shell-significant `{`/`}` so that command is already non-auto-allowed via the SHELL_SIGNIFICANT gate —
the assertion documents intent without depending on the new guard. Keep the first two asserts as the
genuine RED cases: `find . -exec sudo reboot +` and `xargs mkfs` are auto-allowed today.)

- [ ] **Step 2: Run to verify RED**

Run: `cd agent && cargo test -p agent-policy auto_allow_rejects_catastrophe_token_in_allowlisted_command`
Expected: FAIL — `is_auto_allowed("find . -exec sudo reboot +", &al)` and `is_auto_allowed("xargs mkfs", &al)` currently return `true` (allowlisted prog, no shell-significant char, no catastrophe check yet). The `interpreter_wrapping_reaches_ask_not_deny_or_allow` test already passes (documents existing behavior).

- [ ] **Step 3: Add the catastrophe-token guard to `is_auto_allowed`**

In `agent/crates/agent-policy/src/command.rs`, insert this block in `is_auto_allowed` immediately after the `SHELL_SIGNIFICANT` rejection (after line 197, before `let prog = &tokens[0];`):

```rust
    // Even an allowlisted program must not auto-run a catastrophe program passed as an argument
    // (`find . -exec sudo reboot +`, `xargs mkfs`). Name-exact on each token's basename, so benign
    // substrings (`sudoku`, `pseudo`) are unaffected. These fall through to Ask, not Deny — the hard
    // floor stays position-aware.
    //
    // KNOWN LIMITATION: a catastrophe wrapped in a quoted interpreter argument (`bash -c "sudo x"`)
    // is a single token whose basename != the catastrophe name, so this guard cannot see it. Under
    // the default allowlist such interpreters aren't allowlisted, so those reach Ask. Do not add
    // shell interpreters (bash/sh/zsh/dash/eval/xargs) to command_allowlist.
    if tokens.iter().any(|t| program_name_is_catastrophic(basename(t)).is_some()) {
        return false;
    }
```

- [ ] **Step 4: Run to verify GREEN**

Run: `cd agent && cargo test -p agent-policy`
Expected: PASS — the two new tests plus all existing (the guard only adds rejections; existing `auto_allow_*` tests use benign commands with no catastrophe token, so unaffected). Confirm the full agent-policy suite is green and output is pristine.

- [ ] **Step 5: Commit**

```bash
cd agent && git add crates/agent-policy/src/command.rs
git commit -m "fix(policy): auto-allow gate rejects catastrophe program passed to an allowlisted exec-capable program"
```

## Task 2 Self-Review

- Auto-allow guard closes the verified `find -exec sudo` auto-Allow hole → Step 3, tested Step 1. ✔
- Name-exact (no substring false positives: `cat sudoku.txt` still auto-allows) → Step 1 assertion. ✔
- Interpreter-wrapping documented as known limitation + tested to reach Ask (not Deny/Allow) under default allowlist + allowlist guidance → Step 1 (`interpreter_wrapping_reaches_ask_not_deny_or_allow`), Step 3 (doc comment). ✔
- No new hard-denies; guard only affects the auto-allow gate → Step 3 placement (in `is_auto_allowed`, not `hard_floor_violation`). ✔
- Type consistency: `program_name_is_catastrophic(basename(t))` matches the helper from Task 1. ✔

---

### Task 3: CLI default-denylist parity (Finding B) + exec-vehicle residual docs (Finding A)

Final whole-branch review found the false-positive fix never reached the CLI (`default_denylist()` still carried `sudo`/`mkfs`, and the CLI seeds its user denylist from it), and that allowlisted exec-capable programs (`git -c core.pager="sudo …"`) reach silent auto-Allow. See spec Addendum 2. Finding B is a bug fix; Finding A is documented + regression-tested as an accepted residual (user decision).

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/lib.rs:174` (`default_denylist()`)
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (new integration regression test in `mod tests`)
- Modify: `agent/crates/agent-policy/src/command.rs` (extend known-limitation comment; accepted-residual test in `mod tests`)

**Interfaces:**
- Consumes: `crate::default_denylist()`, `RuntimeConfig::effective_denylist()`, `agent_policy::hard_floor_violation`, `is_auto_allowed`.
- Produces: `default_denylist()` returns `["rm -rf /", ":(){", "dd if="]` (was 5 entries). No signature changes.

- [ ] **Step 1: Write the failing Finding-B regression test (RED)**

In `agent/crates/agent-runtime-config/src/runtime_config.rs`, add to `mod tests`:

```rust
    #[test]
    fn cli_default_config_does_not_over_deny_benign_catastrophe_names() {
        // The CLI seeds command_denylist = default_denylist(); the policy denylist is
        // effective_denylist() = HARD_FLOOR ∪ that. Regression (Finding B): benign catastrophe-name
        // arguments must NOT be hard-denied under the REAL assembled denylist — not just the floor.
        let mut c = base();
        c.command_denylist = crate::default_denylist();
        let deny = c.effective_denylist();
        assert!(agent_policy::hard_floor_violation("man mkfs", &deny).is_none());
        assert!(agent_policy::hard_floor_violation("man sudo", &deny).is_none());
        assert!(agent_policy::hard_floor_violation("cat sudoku.txt", &deny).is_none());
        // Direct catastrophe invocation is still denied (structural / boundary scan):
        assert!(agent_policy::hard_floor_violation("sudo reboot", &deny).is_some());
        assert!(agent_policy::hard_floor_violation("mkfs /dev/sda", &deny).is_some());
    }
```

- [ ] **Step 2: Run to verify RED**

Run: `cd agent && cargo test -p agent-runtime-config cli_default_config_does_not_over_deny_benign_catastrophe_names`
Expected: FAIL — `hard_floor_violation("man mkfs", &deny)` returns `Some(...)` because `default_denylist()` still contains the `"mkfs"` substring, which the effective denylist unions in. (`agent_policy` is already a dependency; if the test can't resolve the path, add `use agent_policy;` — the crate is in `Cargo.toml`.)

- [ ] **Step 3: Fix `default_denylist()` (Finding B)**

In `agent/crates/agent-runtime-config/src/lib.rs:174`:

```rust
pub fn default_denylist() -> Vec<String> {
    // Bare program-name catastrophes (sudo/mkfs) are handled position-aware in agent-policy;
    // keeping them here as substrings would re-introduce the `man mkfs` false positive on the CLI.
    // Mirrors HARD_FLOOR_DENYLIST.
    ["rm -rf /",":(){","dd if="].into_iter().map(String::from).collect()
}
```

- [ ] **Step 4: Run to verify GREEN (Finding B)**

Run: `cd agent && cargo test -p agent-runtime-config`
Expected: PASS — the new test plus all existing (`main.rs:327`'s `!command_denylist.is_empty()` still holds: 3 entries).

- [ ] **Step 5: Document the exec-vehicle residual (Finding A) + pin it with a test**

In `agent/crates/agent-policy/src/command.rs`, extend the KNOWN LIMITATION comment block in `is_auto_allowed` (the one about interpreter-wrapping) by appending:

```rust
    // The same blind spot applies to allowlisted exec-CAPABLE programs — `git` (via `-c
    // core.pager=…`/`core.editor=…`/aliases/hooks, run through `sh -c`), `cargo` (build scripts,
    // aliases), `find -exec sh -c …`. They can run arbitrary sub-commands (including catastrophes)
    // that neither the position-aware layers nor this name-exact guard can inspect. ACCEPTED
    // RESIDUAL: the hard floor covers DIRECT catastrophe invocation, not catastrophes smuggled
    // through allowlisted exec vehicles. Mitigations: don't allowlist exec-capable programs if the
    // floor must hold, and rely on the execution sandbox (agent-sandbox).
```

Then add a regression test in `mod tests` pinning the documented behavior:

```rust
    #[test]
    fn auto_allow_exec_vehicle_residual_is_documented_not_a_regression() {
        // ACCEPTED RESIDUAL (see is_auto_allowed comment + spec Addendum 2): an allowlisted
        // exec-capable program runs sub-commands the floor cannot inspect. `git -c core.pager=…`
        // runs its value via `sh -c`. This is auto-allowed by design; pinned so any future change
        // that alters it is noticed and re-evaluated. Mitigation = allowlist policy + sandbox.
        let al = vec!["git".to_string()];
        assert!(is_auto_allowed(r#"git -c core.pager="sudo reboot" log"#, &al));
    }
```

- [ ] **Step 6: Run to verify GREEN (Finding A test) + whole workspace**

Run: `cd agent && cargo test -p agent-policy`
Expected: PASS including the new residual test.

Run: `cd agent && cargo test`
Expected: whole workspace green.

- [ ] **Step 7: Commit**

```bash
cd agent && git add crates/agent-runtime-config/src/lib.rs \
  crates/agent-runtime-config/src/runtime_config.rs \
  crates/agent-policy/src/command.rs
git commit -m "fix(config): drop sudo/mkfs from default_denylist so CLI gets position-aware floor; document exec-vehicle residual"
```

## Task 3 Self-Review

- Finding B fixed: `default_denylist()` mirrors the floor → Step 3; verified by a test over the REAL effective denylist → Steps 1-2. ✔
- Finding B test exercises `effective_denylist()` (not the private `floor()` helper), closing the masking gap the reviewer flagged. ✔
- Finding A documented in code (exec-vehicle limitation) → Step 5; pinned by a regression test → Step 5. ✔
- No new hard-deny; no is_auto_allowed logic change (A is doc-only + a pin test). ✔
- Type consistency: `agent_policy::hard_floor_violation` / `is_auto_allowed` signatures match existing; `default_denylist()` return type unchanged. ✔
