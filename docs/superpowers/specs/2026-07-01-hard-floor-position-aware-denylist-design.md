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

---

## Addendum (2026-07-01): auto-allow guard for exec-wrapped catastrophes

**Discovered during implementation review.** Removing the bare `sudo`/`mkfs` substring literals
from `HARD_FLOOR_DENYLIST` did not only relax hard-deny for benign argument-position uses — it also
removed the blunt guard that kept a catastrophe program name, when passed as an **argument to an
allowlisted, exec-capable program**, out of the **auto-allow** path.

**Verified regression:** `find . -exec sudo reboot +` — `find` is on the default allowlist
(`default_allowlist()` = `ls cat pwd echo git grep find rg cargo head tail wc`) and the argument
string carries no shell-significant character, so `hard_floor_violation` returns `None` and
`is_auto_allowed` returns `true` → **silent execution, no approval prompt**. Before the substring
removal this was hard-denied (`Some("command matches denylist: sudo")`). This is a Critical
under-denial (Deny → auto-Allow), strictly worse than the interpreter-wrapping residual below (which
only reaches Ask, since interpreters are not default-allowlisted).

### Fix — name-exact catastrophe guard in the auto-allow gate

Add to `is_auto_allowed` (`command.rs`), after the shell-significant check and before the allowlist
lookup: refuse to auto-allow if **any** token's basename is a catastrophe program name
(`program_name_is_catastrophic(basename(tok))`). This is name-**exact** on whole-token basenames, not
a substring test, so it does not reintroduce the false positives this spec set out to remove:

- `find . -exec sudo reboot +` → token `sudo` → not auto-allowed → **Ask** (approval required).
- `xargs mkfs` → token `mkfs` → **Ask**.
- `cat sudoku.txt` → basename `sudoku.txt` ≠ `sudo` → **still auto-allows** (no false positive).
- `man mkfs` → unchanged (not hard-denied; `man` not allowlisted → already Ask).
- Accepted mild cost: `grep mkfs /var/log` → Ask instead of auto-Allow (rare; a command naming a
  catastrophe program as an exact argument is worth a human glance). This is fail-safe (Ask, never a
  hard-deny), so it does not resurrect the "blocked benign command" bug.

Hard-deny stays position-aware (Layers A/A2 unchanged); this guard only affects the auto-allow gate,
downgrading exec-wrapped catastrophes to Ask, never to Deny.

### Known limitation — interpreter-wrapping (documented, not fixed)

`bash -c "sudo reboot"`, `sh -c "…"`, `eval "sudo x"`, `xargs sudo` pass the catastrophe program as a
string *interpreted* by another program. A position-aware scan fundamentally cannot see this: the
leading token is `bash`/`sh`/`eval`, and the name-exact guard does not fire because `"sudo reboot"` is
a single quoted token (basename `sudo reboot` ≠ `sudo`). Under the default allowlist these reach
**Ask** (interpreters are not allowlisted), which is the correct security boundary — hard-denying them
is a slippery slope (`env`/`nice`/`timeout`/`nohup`/`setsid`/`watch` wrappers chain without terminus).
Documented as a known limitation in the code, regression-tested to assert Ask (not Deny, not Allow)
under the default allowlist, with allowlist guidance: **do not add shell interpreters
(`bash`/`sh`/`zsh`/`dash`/`eval`/`xargs`) to `command_allowlist`** — the guard cannot inspect their
quoted command-string arguments.

### Additional files touched

| File | Change |
|---|---|
| `agent/crates/agent-policy/src/command.rs` | catastrophe-token guard in `is_auto_allowed`; interpreter-limitation doc comment; auto-allow + interpreter-residual tests |

---

## Addendum 2 (2026-07-01): CLI default-denylist parity + exec-vehicle residual

Final whole-branch review (driving the real assembled denylists through `RulePolicy::check`) found
two blocking issues:

**B — the win did not reach the CLI.** `default_denylist()` (`agent-runtime-config/src/lib.rs:174`),
which the CLI seeds as its user denylist (`agent-cli/src/main.rs:53`), still contained `"sudo"` and
`"mkfs"`. Since the CLI's policy denylist is `effective_denylist()` = `HARD_FLOOR_DENYLIST ∪
config.command_denylist`, `man mkfs`/`man sudo`/`cat sudoku.txt` stayed hard-denied on the CLI — the
false-positive fix only landed on the server surface (whose config denylist is empty). The unit tests
masked this: the in-file `floor()` helper mirrors only `HARD_FLOOR`, never the assembled
`effective_denylist()`.

**Fix B:** drop `"sudo"`/`"mkfs"` from `default_denylist()` → `["rm -rf /", ":(){", "dd if="]`
(mirror the floor). Add a regression test in `agent-runtime-config` that drives
`agent_policy::hard_floor_violation` with the **real** effective denylist (`default_denylist()`
unioned via `effective_denylist()`), asserting `man mkfs`/`man sudo`/`cat sudoku.txt` are not denied
while `sudo reboot`/`mkfs /dev/sda` still are.

**A — allowlisted exec-capable programs smuggle catastrophes to silent auto-Allow.** `git -c
core.pager="sudo reboot" -p log` reaches auto-Allow (git runs its pager via `sh -c`; the token
`core.pager=sudo reboot` has no shell-significant char and its basename ≠ a catastrophe name, so
neither the boundary scan nor the name-exact guard sees it). Same class for `cargo` and
`find -exec sh -c`. This is fundamentally a property of allowlisting programs that execute arbitrary
sub-commands — `git -c core.pager=poweroff`/`shutdown` were silently auto-allowed before and after
this branch; the removed substring only ever caught the literal `sudo`/`mkfs` spelling, never a
coherent defense.

**Resolution A (chosen): document + regression-test as an accepted residual.** For a coding-agent
runtime that runs `git`/`cargo` constantly, forcing them to Ask is too disruptive, and enumerating
git/cargo exec vectors (`-c core.pager`/`core.editor`/`core.sshCommand`/`core.hooksPath`/`alias.*`,
cargo build scripts/aliases, `find -exec`) is a slippery slope with no terminus. The honest boundary:
**the hard floor protects against direct catastrophe invocation, not against catastrophes smuggled
through allowlisted exec-capable programs.** Mitigations are the allowlist policy (do not allowlist
exec-vehicles if the floor must hold) and the execution sandbox (`agent-sandbox`). Documented in code
+ a regression test pinning the observed behavior so any future change is noticed.

### Additional files touched

| File | Change |
|---|---|
| `agent/crates/agent-runtime-config/src/lib.rs` | drop `"sudo"`/`"mkfs"` from `default_denylist()` |
| `agent/crates/agent-runtime-config/src/runtime_config.rs` | real-effective-denylist regression test (Finding B) |
| `agent/crates/agent-policy/src/command.rs` | extend known-limitation comment to exec-vehicles; accepted-residual regression test (Finding A) |
