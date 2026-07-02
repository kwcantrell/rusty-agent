# Destroy tier + subcommand-aware allowlist

**Date:** 2026-07-01
**Cluster:** harness deep audit Top-10 #9 + #10 (Guardrails) — the final Top-10 cluster.
**Audit:** `docs/superpowers/audits/2026-07-01-harness-deep-audit.md`, Component 5 (Guardrails).

## Invariant

No destructive operation reaches `Decision::Allow` without explicit user opt-in.
Concretely: (a) an allowlisted exec-capable program auto-allows only enumerated
read-safe subcommands — unknown subcommands fail safe to Ask; (b) a tool that
declares `Access::Destroy` can never be auto-allowed by any rule (allowlist or
path boundary) — its floor is Ask, and the hard floor can still Deny it.

## Findings addressed (verified live)

1. **MED #9 — `agent-policy/src/command.rs:188-233`.** `is_auto_allowed` matches
   only `tokens[0]` against the allowlist. `git` and `cargo` are in
   `default_allowlist()` (`agent-runtime-config/src/lib.rs:200-207`), so
   `git push --force`, `git reset --hard`, `git clean -fdx` reach
   `Decision::Allow` — destructive history loss without even an Ask.
2. **MED #10 — `agent-memory/src/tools.rs:14-17`.** All three memory tools share
   a `read_intent()` helper declaring `Access::Read`; the doc comment says this
   was chosen deliberately so `RulePolicy` auto-allows them. `forget` is an
   irreversible hard delete (`store.rs`: SQL `DELETE` / `HashMap::remove`);
   `remember` writes (`upsert`, plus capacity eviction). A prompt-injected
   `forget` silently destroys a long-term memory record with zero approval.
3. **MED (engine fold) — `agent-tools/src/types.rs:24-27`,
   `agent-policy/src/engine.rs:56-72`.** `Access` has two variants and the
   engine is a 2-state fold (Read→Allow-if-inside, Write→Ask). The audit's
   build note: a Destroy tier closes the three MED findings together. This spec
   adds the tier and applies it where destruction is unambiguous (`forget`);
   finer Write-vs-Destroy classification for file overwrites stays a residual.
4. **Asymmetry note (Tools component).** `git_status` declares `Access::Read`
   with `command: None` (git.rs:82-87) while `execute_command("git status")`
   goes through the command branch. After the allowlist rework both land on
   Allow via different judges (`git status` prefix entry vs Read-no-paths) —
   the friction asymmetry disappears; a test pins the parity.

## Decisions

- **Destroy is a third `Access` variant, not a `ToolIntent` flag.** The enum is
  the type the engine folds on; a parallel bool would let the two disagree.
  Behaviorally Destroy → Ask today (there is no stronger interactive state than
  Ask; Deny stays the hard floor's verdict). The tier's value now: it can never
  be auto-allowed (new command-branch guard + non-command arm), it is truthful
  in the audit log, and it is the hook for future wire surfacing.
- **No wire/web/Tauri changes.** The approval wire (`agent-server/src/wire.rs:63-70`)
  carries no Access field; approval stays a generic yes/no. Destroy mapping to
  Ask means both surfaces already handle it. Surfacing severity in the approval
  UI is out of scope.
- **#9 is fixed by allowlist generalization, not by the tier.** The engine's
  command branch never consults `intent.access` for the allow verdict, and
  `execute_command`'s Access is static. Making entries token-prefix-aware fixes
  the class (any exec-capable program), keeps policy in config rather than
  hardcoded per-program tables, and fails safe (unknown subcommand → Ask).
- **A bare one-word entry keeps program-only matching.** Users who put `git`
  back in `command_allowlist` restore the old behavior knowingly — documented
  escape hatch, no config schema change (`Vec<String>` unchanged, round-trip
  untouched). Existing saved configs that persisted the old default (bare
  `git`/`cargo`) keep their persisted semantics; only fresh defaults change.
- **Cluster-8 follow-up batch (claude-cli Process-body overflow arm,
  paused-clock backoff tests, `AgentError::Cancelled` dead code) is NOT folded
  in.** It shares no files or theme with this cluster; it stays a separate
  small cleanup.

## Section 1 — `Access::Destroy` (agent-tools, agent-policy)

`agent-tools/src/types.rs`: add `Destroy` to `Access` (additive; the enum
derives Serde — old traces/records still decode).

`agent-policy/src/engine.rs` (`RulePolicy::check`):

- Command branch: hard floor first (unchanged, Deny wins), then auto-allow is
  guarded: `intent.access != Access::Destroy && is_auto_allowed(...)`. A
  Destroy-declared command can therefore only reach Ask or Deny. (No current
  tool declares command+Destroy; the guard makes the invariant structural.)
- Non-command branch: `Access::Destroy => Decision::Ask` as its own arm with a
  comment — Destroy never participates in the inside-workspace auto-allow.

## Section 2 — Subcommand-aware allowlist (agent-policy, agent-runtime-config)

`command.rs::is_auto_allowed`: all existing guards run unchanged (tokenize,
control ops, `SHELL_SIGNIFICANT`, catastrophe-token name-exact, no-`/` program).
Only the final membership test changes, from `entry == tokens[0]` to
token-prefix matching:

- Split each allowlist entry on ASCII whitespace.
- An entry matches iff it is non-empty, `entry_tokens.len() <= tokens.len()`,
  and each entry token equals the corresponding command token exactly.
- One-word entries therefore behave exactly as today.

`agent-runtime-config/src/lib.rs::default_allowlist()`: drop bare `git` and
`cargo`; keep the other single-word entries; add:

- `git status`, `git log`, `git diff`, `git show`, `git blame`,
  `git rev-parse`, `git ls-files`
- `cargo build`, `cargo check`, `cargo test`, `cargo fmt`, `cargo clippy`,
  `cargo metadata`, `cargo tree`

Rationale: the git set is read-safe in the common case but NOT pure-read —
`git {log,diff,show} --output=<path>` truncates/overwrites an arbitrary file
and still auto-allows (accepted residual, recorded at `default_allowlist()`;
pre-existing under the old bare `git` entry and mitigated by the execution
sandbox). The cargo set preserves dev UX and is already covered by the
documented exec-vehicle residual (build scripts). Every
other subcommand (`git push`, `git reset`, `git clean`, `git commit`,
`git branch`, `cargo publish`, `cargo install`, `cargo run`, …) demotes to Ask.
Flag-before-subcommand forms (`git -C x status`) also reach Ask — accepted
over-ask, one keypress.

Update the module doc comment (command.rs:185-187 and the exec-vehicle
ACCEPTED RESIDUAL block at 205-221): the residual narrows — bare exec-capable
programs are no longer auto-allowed by default, but the enumerated subcommands
(`cargo build` et al.) can still run arbitrary code via build scripts/aliases,
and a user-added bare entry re-widens it.

## Section 3 — Memory tool re-tiering (agent-memory)

`agent-memory/src/tools.rs`: replace `read_intent()` with a helper taking the
access level (or three inline constructions):

- `remember` → `Access::Write` (upsert + eviction)
- `recall` → `Access::Read` (unchanged)
- `forget` → `Access::Destroy` (irreversible hard delete)

Rewrite the lines-11-13 doc comment: memory mutations are now approval-gated by
design; `summary` stays truthful. Intents keep `paths: []`, `command: None`, so
the engine judges purely by access: remember/forget → Ask, recall → Allow.

**Behavior change:** every `remember`/`forget` now prompts on both surfaces.
This is the audit's required fix; `ApproveAlways` (even as a session no-op) and
future policy config are the relief valves, out of scope here.

**Call-site sweep:** any agent-core/agent-cli/agent-server test or fixture that
drives `remember`/`forget` through the policy gate must gain an approval stub
(approval fails closed — such tests would otherwise deny, not hang). Direct
`execute()` unit tests in agent-memory are unaffected (no gate).

## Section 4 — Symmetry pin + docs

- Engine-level test pinning parity: an `execute_command`-shaped intent
  (`command: Some("git status")`, `Access::Write`) → Allow, and a
  `git_status`-shaped intent (`Access::Read`, no paths, no command) → Allow.
- `AGENTS`-facing docs: none required beyond code comments; runtime-config
  field docs mention multi-word entries.

## Error handling & edge cases

- Allowlist entry that is empty/whitespace → never matches (fail-safe).
- Entry containing shell-significant chars can never match (command tokens
  containing them are rejected earlier) — harmless dead entry, no validation
  needed.
- Entry longer than the command (`git status --short` vs `git status`) → no
  match → Ask (fail-safe).
- Quoted args: `git log "-S needle"`? Tokenizer yields `-S needle` as one
  token; prefix `git log` still matches — allowed, read-only. Catastrophe and
  metacharacter guards still precede matching.
- Hard floor ordering unchanged: Deny beats Allow beats Ask; a floored command
  never reaches the allowlist.
- Old saved configs: `command_allowlist` persisted with bare `git`/`cargo`
  keeps old behavior (user-owned config; per-field merge means the field, once
  present, wins). Documented, no migration.

## Testing

- `command.rs`: prefix matching — `git status`, `git status --porcelain -b`,
  `git log --oneline` auto-allowed under new defaults; `git push --force`,
  `git reset --hard`, `git clean -fdx`, bare `git`, `git -C /tmp status`,
  `cargo publish` NOT auto-allowed; single-word entries (`ls -la`) unchanged;
  entry-longer-than-command no-match; empty entry no-match.
- `engine.rs`: `Access::Destroy` with inside-workspace paths → Ask; intent with
  `command` + `Access::Destroy` + matching allowlist → Ask (guard); floored
  command + Destroy → Deny; symmetry pin (Section 4).
- `agent-memory/tools.rs`: intent access assertions (remember=Write,
  recall=Read, forget=Destroy); a `RulePolicy::check` integration assertion
  that a forget intent yields Ask.
- `runtime_config.rs`: `default_allowlist()` contents updated;
  `cli_default_config_does_not_over_deny_benign_catastrophe_names` still green;
  round-trip test untouched.
- Full gate: `bash scripts/ci.sh` (fmt + clippy + cargo test + web).

## Files touched

- `agent/crates/agent-tools/src/types.rs` — `Access::Destroy`
- `agent/crates/agent-policy/src/engine.rs` — Destroy arms + command-branch guard + tests
- `agent/crates/agent-policy/src/command.rs` — prefix matching + doc updates + tests
- `agent/crates/agent-runtime-config/src/lib.rs` — `default_allowlist()` rework
- `agent/crates/agent-memory/src/tools.rs` — re-tiering + comment + tests
- (sweep) any loop-level tests driving memory tools through the gate

## Out of scope (recorded residuals)

- MCP `Trust::Allow → Access::Read` mapping (`agent-mcp/src/tool.rs:72-75`)
  and HTTP `HostDecision → Access` mapping — approval-posture encodings, noted
  in the audit as accepted LOW.
- Write-vs-Destroy granularity for FS tools (tracked-file overwrite vs scratch
  write) and `git_commit`'s tier.
- Surfacing Access/severity over the approval wire; `ApproveAlways` persistence.
- `git {log,diff,show} --output=<path>`: auto-allowed arbitrary-file write under
  the default git prefixes (pre-existing under bare `git`; candidate for
  `--output`/`-o` arg-scanning if FS-write granularity is ever tightened).
- Cluster-8 follow-up batch (separate cleanup).
