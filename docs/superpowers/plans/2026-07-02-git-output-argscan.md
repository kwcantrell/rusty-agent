# git --output Arg-Scan Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `git {log,diff,show} --output/-o` reaches Ask instead of auto-Allow; the documented residual is closed.

**Architecture:** One guard in `is_auto_allowed` (agent-policy/src/command.rs) scoped to git's log/diff/show subcommands; doc-comment updates in both crates.

**Tech Stack:** Rust (workspace `agent/`).

**Spec:** `docs/superpowers/specs/2026-07-02-git-output-argscan-design.md` — behavior authority, including the precision contract (indicator flags, ls-files -o, grep -o must stay allowed).

## Global Constraints

- Run cargo from `agent/` (`source ~/.cargo/env` if missing); `cargo fmt -p agent-policy` before committing; conventional commits; `bash scripts/ci.sh` green at cluster end.
- A scan hit → Ask (return false), never Deny.

---

### Task 1: The guard + tests + doc updates

**Files:**
- Modify: `agent/crates/agent-policy/src/command.rs:244-253` (the allowlist match tail) + doc comment at 185-189 + tests
- Modify: `agent/crates/agent-runtime-config/src/lib.rs:216-225` (ACCEPTED RESIDUAL → CLOSED note)

- [ ] **Step 1: Failing tests** (command.rs tests mod, using `default_allowlist()`-style fixtures as the neighboring tests do — read them first):

```rust
#[test]
fn git_output_flag_is_not_auto_allowed() {
    let al = agent_runtime_config_free_fixture(); // use the file's existing allowlist fixture pattern; entries incl. "git log","git diff","git show","git ls-files","git status","grep"
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
    let al = /* same fixture */;
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
```

(If the tests mod has no allowlist fixture helper, build `let al: Vec<String> = ["git log","git diff","git show","git ls-files","git status","grep"].iter().map(|s| s.to_string()).collect();` inline — mirror neighboring tests' style.)

- [ ] **Step 2: Verify failing** — `cd agent && cargo test -p agent-policy git_output` → first test FAILS (all four auto-allow today), second passes.

- [ ] **Step 3: Implement.** Replace the final expression of `is_auto_allowed` (the `allowlist.iter().any(...)`) with:

```rust
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
    && matches!(tokens.get(1).map(String::as_str), Some("log" | "diff" | "show"))
    && tokens[2..]
        .iter()
        .any(|t| t == "-o" || t == "--output" || t.starts_with("--output="))
{
    return false;
}
true
```

Extend the fn doc comment (185-189) with one sentence: "Matched git `log`/`diff`/`show` invocations are additionally screened for `--output`/`-o` (an arbitrary-file write) and fall to Ask." Update the runtime-config `default_allowlist()` comment: replace the "ACCEPTED RESIDUAL: the git prefixes … not the policy gate)." paragraph with "CLOSED (2026-07-02): `git {log,diff,show} --output=<path>`/`-o` now falls to Ask via the arg-scan in `agent-policy::is_auto_allowed` (see 2026-07-02-git-output-argscan spec)."

- [ ] **Step 4: Verify** — `cargo test -p agent-policy && cargo test -p agent-runtime-config` → PASS.

- [ ] **Step 5:** `cargo fmt -p agent-policy -p agent-runtime-config`, commit — `fix(policy): git log/diff/show --output arg-scan — arbitrary-file write no longer auto-allowed`

---

### Task 2: Cluster gate

- [ ] `bash scripts/ci.sh` → green. No commit expected.
