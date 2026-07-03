# Redirection-to-/dev Deny parity

**Date:** 2026-07-02
**Status:** Approved (2026-07-02 product-decision round, item 10)
**Branch:** `fix/dev-redirect-denial`

## Problem

The hard floor structurally Denies `dd of=/dev/sda`
(`agent-policy/src/command.rs` — `simple_command_is_catastrophic`, the `dd`/`of=`
arm), but the equally destructive `echo x > /dev/sda` only trips the generic
`SHELL_SIGNIFICANT` metacharacter gate and lands at **Ask**. The hard floor's own
philosophy is that direct catastrophic invocation Denies even if a user would
approve it. Documented asymmetry, adjudicated: build the handler.

## Design

One shared target predicate + two detection layers, matching the file's existing
layer structure (A structural / A2 raw / B substring).

### Target predicate

```rust
/// A /dev path that redirection may safely write to. Everything else under
/// /dev/ is treated as a device write sink and denied. Deny-by-default with a
/// small allowlist is the same fail-safe posture as the dd handler (which is
/// stricter still: it denies ALL of=/dev/* including /dev/null).
fn dev_redirect_target_is_safe(target: &str) -> bool
```

`target` must normalize to an absolute path UNDER `/dev`; the suffix is safe iff
it is one of
`null | zero | full | random | urandom | stdin | stdout | stderr | tty | ptmx`
or starts with `fd/` or `shm/`. Everything else (`sda`, `nvme0n1`, `mem`,
`kmem`, `port`, `mmcblk0`, `loop0`, `dm-0`, `mapper/…`, `disk/…`, `ttyUSB0`, …)
is unsafe. Case-sensitive (device names are lowercase). `/dev/shm/…` is a
standard world-writable tmpfs (files, not devices) so writes under it are safe.

**Path normalization (added by the /dev-redirect-denial hardening pass; `..`
resolution corrected by fix wave 2):** the target's full path is lexically
normalized before the /dev match — redundant `/`-runs and `.` segments are
discarded, and `..` is fully resolved by popping the previous kept segment (a
leading `..` at root drops, matching POSIX `/..` == `/`). So `//dev/sda`,
`///dev/sda`, `/./dev/sda`, `/dev/./sda`, `/dev/../dev/sda`, and — critically —
the leading/mid-path forms `/../dev/sda`, `/usr/../dev/sda`, `/tmp/../dev/sda`
all resolve to a write under /dev and deny, while `//dev/null` and `/dev/./null`
normalize to a safe sink and `/dev/foo/../null` resolves to `/dev/null` (safe).
Full lexical resolution replaces the earlier "do NOT collapse `..`" over-
approximation, which returned SAFE for any target whose first segment was `..`
(`/../dev/sda`) and thus missed device writes that navigate INTO /dev. Resolving
`..` tracks the real kernel target and is strictly more correct; deny-by-default
still applies to the resolved /dev-rooted result. Symlinks are NOT resolved
(symlink-indirection into /dev is out of scope for this static floor and reaches
Ask). Only an absolute path can name a device node, so a relative `dev/sda` (an
ordinary cwd file) is not this handler's concern.

Note: `/dev/ttyUSB0`-style serial targets are hard-denied. This matches the
existing dd posture exactly (`dd of=/dev/ttyUSB0` is denied today) and is the
accepted cost of deny-by-default.

### Layer A — structural (quote-aware, runs when the string tokenizes)

`shell_words::split` treats `>` as an ordinary character, so within each simple
command (same iteration as `simple_command_is_catastrophic`, applied to the
argv including the program position):

- **Glued token**: strip an optional leading fd digit-run or `&`; if the
  remainder starts with `>`, strip `>`, then one optional `>` or `|`; a
  non-empty remainder is the glued target (`>/dev/sda`, `2>>/dev/sda`,
  `&>/dev/sda`, `>|/dev/sda`).
- **Split pair**: if the remainder after operator-stripping is empty (the token
  was purely an operator like `>`, `>>`, `2>`, `&>`, `>|`), the NEXT token is
  the target.
- If the resolved target starts with `/dev/` and
  `!dev_redirect_target_is_safe(target)` → deny with reason
  `"redirection writing to a device file is denied"`.

Because the tokenizer strips quotes, `echo ">" /dev/sda` (a quoted literal `>`
followed by a path argument) is indistinguishable from real redirection and is
denied — an accepted fail-safe over-approximation, same class as the documented
quote-blindness of the A2 boundary scan.

### Raw backstop (runs always — covers unparseable strings)

Mirroring Layer A2's placement in `hard_floor_violation`: scan the RAW string
for `>` occurrences; after each run of `>`/`|` consume optional whitespace and
optional leading quote chars (`"`, `'`); if the following text starts with
`/dev/`, the target extends to the next whitespace/quote/boundary char; apply
the same predicate → same denial reason. Catches `echo x >"/dev/sda"`,
unbalanced-quote strings, and glued forms uniformly. Over-denial of `/dev/sda`
mentioned in quoted prose AFTER a `>` (e.g. `echo "watch out > /dev/sda"`) is
accepted — the hard floor errs toward denial, per the file's existing NOTE.

`>` inside the safe-target set (`2>/dev/null`, `>/dev/stderr`) does not deny;
those commands still reach Ask via `SHELL_SIGNIFICANT` exactly as today. Plain
file redirects (`> out.txt`) are untouched (Ask). Input redirection (`<`,
reading a device) is out of scope — reads are not destructive; recorded here as
the deliberate boundary.

### Wiring

Both layers live in `command.rs` and fire from `hard_floor_violation`, which
both frontends share (CLI + server, HARD_FLOOR + `effective_denylist` path) —
no config, no wire changes, no new tiers.

## Tests

Unit tests in `command.rs`:
- Deny: `echo x > /dev/sda`, `echo x >/dev/sda`, `echo x >> /dev/nvme0n1`,
  `2>>/dev/sda`, `&>/dev/sda`, `>|/dev/sda`, `echo x >"/dev/sda"`,
  `echo x > /dev/sda "unbalanced` (raw backstop on unparseable),
  `git log > /dev/mem`, split-pair `... 2> /dev/sda`.
- Not denied (still Ask via metachar gate, pinned as non-deny here):
  `cmd 2>/dev/null`, `echo x > /dev/stdout`, `x > /dev/fd/3`, `echo hi > out.txt`,
  `echo hi >> notes.md`.
- Predicate edge: `/dev/` bare and `/dev/sda1` unsafe; `/dev/fd/3` safe.

Policy corpus (`agent-runtime-config/tests/policy_corpus.tsv`): add rows through
the real engine path for the deny cases above plus `2>/dev/null` → ask and
`> out.txt` → ask, and one documented accepted-FP row
(`echo "> /dev/sda"` → deny).

## Out of scope (recorded)

- Input redirection (`< /dev/sda`) and non-redirect write vehicles
  (`tee /dev/sda`, `cp x /dev/sda`) — the hard floor covers direct catastrophic
  invocation forms; these reach Ask like any other metachar/unknown command.
- Variable-expansion targets (`>$D/sda`, `> ${DEV}`) and cwd-relative redirects
  (`cd /dev && echo x > sda`) — this static floor does not resolve shell
  variables or track the working directory, so these reach Ask (not Deny). Path
  normalization now covers redundant `/` runs, `.` segments, AND full lexical
  `..` resolution (fix wave 2); it does NOT resolve symlinks.
- The dd handler now shares the lexical /dev resolver (`resolved_dev_suffix`,
  extracted in fix wave 3), so `dd of=/../dev/sda`, `dd of=//dev/sda`, and
  `dd of=/dev/./sda` deny through the same normalization as redirects — the
  earlier literal `/dev/` prefix check no longer lets them fall to Ask. Only the
  /dev/null strictness difference remains intentional: dd denies ALL of=/dev/*
  (including the safe sinks like /dev/null) via resolver PRESENCE, whereas
  redirects consult the safe-set and allow /dev/null.
