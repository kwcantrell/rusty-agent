# /dev Redirection Denial Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hard-Deny shell redirection writing to unsafe `/dev/*` targets, closing the documented `dd of=/dev/sda` (Deny) vs `echo x > /dev/sda` (Ask) asymmetry.

**Architecture:** One target predicate + two detection layers in `agent-policy/src/command.rs`, both fired from `hard_floor_violation` (shared by CLI and server). Structural layer scans tokenized simple commands (quote-aware); a raw backstop covers unparseable/quote-glued forms.

**Tech Stack:** Rust (`agent/` workspace), shell-words tokenizer (already used).

**Spec:** `docs/superpowers/specs/2026-07-02-dev-redirect-denial-design.md`

## Global Constraints

- `agent/` Cargo workspace (`cd agent`; `source ~/.cargo/env` if needed).
- No config, wire, or tier changes — pure hard-floor logic + tests.
- Denial reason string, verbatim: `redirection writing to a device file is denied`.
- Safe /dev suffixes, verbatim set: `null zero full random urandom stdin stdout stderr tty ptmx` plus any suffix starting `fd/`. Deny-by-default for every other `/dev/` target; bare `/dev/` unsafe.
- `2>/dev/null` and plain-file redirects must NOT be denied (they keep reaching Ask via SHELL_SIGNIFICANT — pin as non-deny).
- Conventional commits.

---

### Task 1: redirect-to-/dev hard-floor handler

**Files:**
- Modify: `agent/crates/agent-policy/src/command.rs` (helpers near `simple_command_is_catastrophic` ~line 78; wiring inside `hard_floor_violation` ~line 140; unit tests in the file's test module)
- Modify: `agent/crates/agent-runtime-config/tests/policy_corpus.tsv` (+ its driver `policy_corpus.rs` only if a new expectation keyword is needed — inspect the TSV header/driver first; it already expresses deny/ask/allow rows)

**Interfaces:**
- Consumes: `split_simple_commands`, `hard_floor_violation`, existing test-module patterns.
- Produces: `fn dev_redirect_target_is_safe(target: &str) -> bool`, `fn redirect_catastrophe_in_argv(argv: &[String]) -> Option<String>`, `fn raw_redirect_catastrophe(cmd: &str) -> Option<String>` (all private to command.rs).

- [ ] **Step 1: Write the failing unit tests**

In command.rs's test module, following its existing test style:

```rust
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
        "echo x > /dev/sda \"unbalanced",   // raw backstop: unparseable
        "git log > /dev/mem",
        "cmd 2> /dev/sda",                  // split operator/target pair
        "echo x > /dev/ttyUSB0",            // deny-by-default (matches dd posture)
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
        "grep pattern file",  // no redirect at all
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
    assert!(r.contains("redirection writing to a device file is denied"), "{r}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-policy redirect`
Expected: `redirect_to_block_device_is_denied` and the reason test FAIL (no handler yet); the not-denied test passes vacuously — fine.

- [ ] **Step 3: Implement**

Add near `simple_command_is_catastrophic`:

```rust
/// A /dev path that redirection may safely write to. Everything else under
/// /dev/ is a device write sink and is denied. Deny-by-default with a small
/// allowlist is the same fail-safe posture as the `dd of=` handler (which is
/// stricter still: it denies ALL of=/dev/* including /dev/null).
fn dev_redirect_target_is_safe(target: &str) -> bool {
    let Some(suffix) = target.strip_prefix("/dev/") else {
        return true; // not a /dev path: not this handler's concern
    };
    matches!(
        suffix,
        "null" | "zero" | "full" | "random" | "urandom" | "stdin" | "stdout"
            | "stderr" | "tty" | "ptmx"
    ) || suffix.starts_with("fd/")
}

/// Strip a redirect-operator prefix from a token: optional fd digit-run or `&`,
/// then `>`, then one optional `>` or `|`. Returns the glued remainder ("" if
/// the token was purely an operator), or None if the token is not a redirect.
fn strip_redirect_op(tok: &str) -> Option<&str> {
    let t = tok.strip_prefix('&').unwrap_or(tok);
    let t = t.trim_start_matches(|c: char| c.is_ascii_digit());
    let t = t.strip_prefix('>')?;
    Some(t.strip_prefix(['>', '|']).unwrap_or(t))
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
            if target.starts_with("/dev/") && !dev_redirect_target_is_safe(target) {
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
            while j < bytes.len() && (bytes[j] == b'>' || bytes[j] == b'|') {
                j += 1;
            }
            while j < bytes.len() && (bytes[j] as char).is_ascii_whitespace() {
                j += 1;
            }
            while j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                j += 1;
            }
            let rest = &cmd[j..];
            if rest.starts_with("/dev/") {
                let end = rest
                    .find(|c: char| {
                        c.is_ascii_whitespace()
                            || matches!(c, '"' | '\'' | '&' | '|' | ';' | ')' | '`')
                    })
                    .unwrap_or(rest.len());
                if !dev_redirect_target_is_safe(&rest[..end]) {
                    return Some(
                        "redirection writing to a device file is denied".to_string(),
                    );
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}
```

Wire both into `hard_floor_violation`: inside the existing Layer-A loop add
`if let Some(reason) = redirect_catastrophe_in_argv(argv) { return Some(reason); }`
after the `simple_command_is_catastrophic` check, and after the Layer-A2 loop add
`if let Some(reason) = raw_redirect_catastrophe(cmd) { return Some(reason); }`.

- [ ] **Step 4: Run unit tests**

Run: `cd agent && cargo test -p agent-policy`
Expected: PASS, including all pre-existing tests (watch especially any existing
test that uses `>` in a command — if one now denies, evaluate against the spec's
accepted-FP posture before touching it; a plain-file `>` must still pass).

- [ ] **Step 5: Add policy-corpus rows**

Inspect `agent/crates/agent-runtime-config/tests/policy_corpus.tsv` (header +
2-3 rows) for the exact column format, then append rows exercising the engine
path (expectations use the corpus's existing vocabulary):

- `echo x > /dev/sda` → deny
- `echo x >/dev/sda` → deny
- `dd of=/dev/sda if=/tmp/x` → deny (pre-existing behavior; add only if not
  already a row)
- `cmd 2>/dev/null` → ask
- `echo hi > out.txt` → ask
- `echo "> /dev/sda"` → deny, with the corpus's comment convention marking it
  the documented accepted false positive

Run: `cd agent && cargo test -p agent-runtime-config --test policy_corpus`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-policy/src/command.rs agent/crates/agent-runtime-config/tests/policy_corpus.tsv
git commit -m "fix(policy): hard-deny redirection writing to unsafe /dev targets"
```

---

### Task 2: CI gate

- [ ] Run: `bash scripts/ci.sh` (repo root). Expected: green. Fix anything red.
