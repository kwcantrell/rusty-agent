# git --output arg-scan — close the auto-allowed arbitrary-file-write residual

**Date:** 2026-07-02
**Status:** Approved (autonomous backlog-drain run; the destroy-tier spec's recorded residual)
**Cluster:** 4 of 6 in the 2026-07 residual-backlog drain

## Problem

The default allowlist exposes `git {log,diff,show}` as read-safe token prefixes, but
those subcommands accept `--output=<path>` / `--output <path>`, which truncates and
overwrites an arbitrary file — an auto-Allowed write (documented ACCEPTED RESIDUAL at
`agent-runtime-config/src/lib.rs:222-225` and in the destroy-tier spec's out-of-scope).
`is_auto_allowed` (agent-policy/src/command.rs:190-253) matches token prefixes and never
inspects later arguments.

## Approaches considered

- **A (chosen): targeted post-match arg scan in `is_auto_allowed`.** After an allowlist
  entry matches, if the program is `git` and the subcommand is one of
  `log`/`diff`/`show`, scan the remaining tokens for `--output`, `--output=<…>`, or
  `-o`; any hit → not auto-allowed (falls to Ask, same posture as every other
  non-read-safe git form). Smallest change, zero config surface.
- **B: per-allowlist-entry denied-args configuration** — general mechanism nobody else
  needs yet (YAGNI).
- **C: tighten FS Write/Destroy granularity so the write itself is caught** — that's
  the recorded product-decision item (tracked-file overwrite vs scratch write), out of
  scope here; this gate-level fix is orthogonal and cheaper.

## Design

In `is_auto_allowed`, replace the final `allowlist.iter().any(...)` expression with a
match + guard:

- If no entry matches → `false` (unchanged).
- If matched and `tokens[0] == "git"` and `tokens.get(1)` is `log`, `diff`, or `show`:
  return `false` when any token in `tokens[2..]` is exactly `-o`, exactly `--output`,
  or starts with `--output=`.
- Everything else → matched result (unchanged).

Precision notes (part of the contract, pinned by tests):

- `--output-indicator-{new,old,context}[=…]` are harmless diff-rendering flags and must
  NOT trip the scan (`--output-indicator…` neither equals `--output` nor starts with
  `--output=`).
- `git ls-files -o` (`--others`, a pure read) must stay auto-allowed — hence the scan is
  scoped to `log`/`diff`/`show`, where `-o` has no legitimate meaning (belt-and-braces
  for format-patch-style muscle memory).
- Abbreviated long options (`--outp`) are not exploitable: git's parse-opt requires
  unambiguous abbreviations, and every proper prefix of `--output` is ambiguous against
  the `--output-indicator-*` siblings that log/diff/show all carry; exact `--output` is
  the only spelling that writes, and the scan catches it.
- `git -P log --output…` and other pre-subcommand flags already fail the prefix match
  (`tokens[1] != "log"`) → Ask. Values via `$(…)`/globs already fail SHELL_SIGNIFICANT.

Doc updates: the ACCEPTED RESIDUAL comment at `default_allowlist()`
(agent-runtime-config/src/lib.rs) becomes a CLOSED note pointing at the scan; the
`is_auto_allowed` doc comment gains one sentence.

## Error handling

None new — the function stays a pure predicate; a scan hit means Ask, never Deny.

## Testing

Unit tests in command.rs (extend the existing allowlist test group, default_allowlist
fixtures):

1. `git log --output=/tmp/x`, `git diff --output /tmp/x` (separate token), `git show
   --output=x`, `git log -o x` → NOT auto-allowed.
2. `git diff --output-indicator-new=+`, `git log --oneline`, `git ls-files -o`,
   `git status` → still auto-allowed.
3. Non-git entries unaffected: `grep -o pattern file` (grep's -o is only-matching, a
   read) → still auto-allowed.

## Out of scope (recorded residuals)

- FS Write-vs-Destroy granularity (the general fix for gate-level write sinks) — the
  standing product decision.
- Other git write-sink flags on *non-allowlisted* subcommands (`format-patch -o`,
  `bundle create`) — those subcommands already fail the prefix match.
- User-added bare `git` entries (user-owned config keeps legacy semantics, unchanged).
