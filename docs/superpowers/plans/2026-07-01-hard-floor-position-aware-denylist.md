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
