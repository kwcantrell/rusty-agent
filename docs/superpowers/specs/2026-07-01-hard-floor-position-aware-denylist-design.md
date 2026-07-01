# Hard-floor guardrail: position-aware catastrophe detection

**Date:** 2026-07-01
**Component:** Harness Anatomy #5 — Guardrails / Hooks
**Source:** Deferred follow-up from `2026-07-01-guardrails-denylist-approval-hardening` (final-review Minor: redundant bare `mkfs` denylist literal causes benign substring over-denials).
**Status:** design approved; ready for implementation plan

## Problem

The catastrophic-command hard floor has two layers (`agent/crates/agent-policy/src/command.rs`):
- **Layer A** — structural, position-aware: `split_simple_commands` (quote-aware `shell-words`
  tokenization) → `simple_command_is_catastrophic` checks each simple command's `argv[0]` basename.
- **Layer B** — an always-on normalized-substring backstop against the configured denylist,
  which includes the built-in `HARD_FLOOR_DENYLIST` (`runtime_config.rs:8`).

The Layer-B substring match is **position-blind**: a bare program name in the denylist (`"sudo"`,
`"mkfs"`) matches anywhere in the command string, including argument position. So benign commands
are hard-**denied** with no approval override:

- `man mkfs`, `grep mkfs /var/log`, `cat mkfs-notes.txt`
- `man sudo`, `which sudo`, and even `pseudocode` (`sudo` is a substring of `pseudo`)

The structural handler already classifies these correctly (the catastrophe name is not `argv[0]`),
but the bare substring literal overrides it. The literal cannot simply be removed, though: it is the
only mechanism covering two forms the structural layer misses —

1. **Glued operators**: `echo x&&sudo reboot` — `shell-words` yields `["echo","x&&sudo","reboot"]`,
   so `sudo` is never an `argv[0]`; only the `"sudo"` substring catches it today.
2. **Unparseable**: `sudo "oops` — unbalanced quote, tokenization fails, Layer A is skipped; only the
   `"sudo"` substring catches it today.

Both forms are exercised by existing tests (`floor_denies_no_space_operator_via_backstop`,
`floor_denies_unparseable_with_denylisted_literal`).

**Violated principle:** "hooks are fast, side-effect-free validators; block bad actions, not delay
good ones" — SKILL.md Spine A, component 5. A hard-floor false positive is exactly a validator
blocking a *good* action.

**Scope decision:** fix the whole bare-program-name class — both `mkfs` and `sudo` share the identical
position-blindness bug. `rm -rf /`, `dd if=`, and `:(){` are left as substring literals: the first
two are specific multi-token strings with negligible benign-substring risk, and `:(){` is a shell
function-definition construct with no structural handler.

## Design

Generalize Layer A to cover the glued-operator and unparseable forms **structurally** (position-aware),
then drop the bare names from the substring backstop. All code changes are in
`agent/crates/agent-policy/src/command.rs` except the denylist constant + its tests.

### 1. Extract `program_name_is_catastrophic`

Pull the bare-program-name checks out of `simple_command_is_catastrophic` into a shared helper:

```rust
/// Catastrophe check keyed on a program's basename alone (no arguments needed).
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

`simple_command_is_catastrophic` calls it first (on `argv[0]`'s basename), then keeps its multi-arg
checks (recursive-`rm`-of-root, `dd`-writing-to-a-block-device) which need the full argv vector.

### 2. Raw-string command-boundary scan

```rust
/// Leading program token at each shell command boundary in the RAW string: start-of-string
/// and immediately after a control-operator char. Operates on the raw text (not shell-words),
/// so it works on glued operators (`x&&sudo`) and unparseable input (unbalanced quotes) alike.
/// Surrounding quote chars are stripped from each token so a quoted program name (`"sudo"`)
/// is still caught.
///
/// NOTE: this scan is intentionally NOT quote-aware about *operators* — an operator inside a
/// quoted string (`echo "a; sudo b"`) is treated as a boundary, so such a command is
/// over-denied. This is fail-safe (a hard floor errs toward denial), rare, and consistent with
/// the SHELL_SIGNIFICANT over-approximation elsewhere in this file. A full quote-aware parser
/// is deliberately out of scope.
fn command_boundary_programs(cmd: &str) -> impl Iterator<Item = &str> {
    cmd.split(|c| matches!(c, '&' | '|' | ';' | '\n'))
        .filter_map(|seg| seg.split_whitespace().next())
        .map(|tok| tok.trim_matches(|c| c == '"' || c == '\''))
        .filter(|tok| !tok.is_empty())
}
```

(Exact iterator shape is the implementer's call; the contract is "leading, quote-stripped program
token per operator-delimited segment, raw string.")

### 3. New Layer-A2 pass in `hard_floor_violation`

After the existing `split_simple_commands` structural pass, add a boundary-scan pass that runs
**regardless of tokenization success**:

```rust
for prog in command_boundary_programs(cmd) {
    if let Some(reason) = program_name_is_catastrophic(basename(prog)) {
        return Some(reason);
    }
}
```

This catches `echo x&&sudo reboot`, `ls | mkfs /dev/sda`, and `sudo "oops` — position-aware, so
`man mkfs` / `man sudo` are untouched.

### 4. Shrink `HARD_FLOOR_DENYLIST`

`runtime_config.rs:8`:

```rust
pub const HARD_FLOOR_DENYLIST: &[&str] = &["rm -rf /", ":(){", "dd if="];
```

Remove `"sudo"` and `"mkfs"` (now fully structural). Update the doc comment to note that
program-name catastrophes are handled structurally (position-aware) rather than by substring.

The Layer-B backstop logic is **unchanged** — only its built-in input set shrinks. User-configured
denylist entries keep whole-string, match-anywhere semantics (users legitimately denylist argument
substrings like hostnames).

## Testing

`agent/crates/agent-policy/src/command.rs` (update the in-file `floor()` helper denylist to mirror
the new floor — drop `sudo`/`mkfs`, keep `["rm -rf /", "dd if=", ":(){"]`):

- **False-positive-gone (the win):** `floor("man mkfs")`, `floor("grep mkfs /var/log")`,
  `floor("man sudo")`, `floor("which sudo")` → all `.is_none()`.
- **Preserved coverage (now via boundary scan):** `floor("echo x&&sudo reboot")`,
  `floor("sudo \"oops")` (unparseable), `floor("ls | mkfs /dev/sda")`,
  `floor("echo x&&mkfs /dev/sda")`, `floor(r#""sudo" reboot"#)` (quote-stripped) → all `.is_some()`.
- **Retained structural tests:** `floor_denies_privilege_escalation_by_basename`,
  `floor_denies_mkfs_structurally`, `floor_denies_rm_flag_and_spacing_variants`,
  `floor_denies_dd_and_fork_bomb` still pass.
- Adjust the comments/intent of `floor_denies_no_space_operator_via_backstop` and
  `floor_denies_unparseable_with_denylisted_literal` — these now pass via the boundary scan, not the
  substring backstop (the denylist no longer contains `sudo`).
- **Over-approximation guard (documents the tradeoff):** `floor(r#"echo "a; sudo b""#)` → `.is_some()`
  (accepted fail-safe over-denial).

`agent/crates/agent-runtime-config/src/runtime_config.rs`:

- `effective_denylist_floor_survives_empty_user_list` asserts `"sudo"` is present — change the
  asserted entry to `"rm -rf /"` (a still-present floor entry).
- `effective_denylist_always_contains_the_hard_floor` iterates `HARD_FLOOR_DENYLIST` — passes unchanged.

Commands: `cargo test -p agent-policy -p agent-runtime-config`, then whole-workspace `cargo test`.

## Scope guards (YAGNI)

- No quote-aware operator parser — the boundary scan over-approximates toward denial by design.
- No change to `rm`/`dd`/`:(){` handling, to the Layer-B backstop algorithm, or to user-denylist
  semantics.
- Bare-program-name class only: `sudo`/`doas`/`su` and `mkfs`/`mkfs.*`.

## Files touched

| File | Change |
|---|---|
| `agent/crates/agent-policy/src/command.rs` | extract `program_name_is_catastrophic`; add `command_boundary_programs` + Layer-A2 pass; update `floor()` test helper + tests |
| `agent/crates/agent-runtime-config/src/runtime_config.rs` | shrink `HARD_FLOOR_DENYLIST` to 3 entries; update doc comment + one test assertion |
