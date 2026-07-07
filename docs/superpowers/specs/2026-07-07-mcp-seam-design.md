# MCP Seam Hardening — Design (Audit-Drain Cluster 2)

**Date:** 2026-07-07
**Parent:** `docs/superpowers/specs/2026-07-07-audit-drain-action-plan-design.md` (Cluster 2).
**Findings closed:** 2.1, 5.1, 3.1 of `docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md`
(all med). Three dimensions independently converged on this boundary — the audit's headline.
**Branch:** `feature/audit-mcp-seam`.
**Grounding:** all cited code re-read live at `899ac84` (2026-07-07); line numbers are from
that commit.

## Decisions (owner-adjudicated 2026-07-07)

- **5.1 encoding: new `Access::TrustedWrite` variant** (over a `mutating` flag on `ToolIntent`).
  Rationale: exactly one exhaustive `match intent.access` exists in the workspace
  (`agent-policy/src/engine.rs:60`) so the compiler finds every consumer; a `ToolIntent` field
  would churn ~27 files of struct literals and thread a second value through the loop's
  `executed` tuples. `Access` never crosses the wire (approval prompts serialize only
  `summary`/`command` strings — `agent-server/src/approval.rs:47-48`), so serde/SPA compat is
  a non-issue.
- **3.1 mechanism: name-only `-e KEY` + values on the docker client process env** (over a 0600
  `--env-file`). Mirrors HostExecutor's existing private-env posture
  (`agent-tools/src/sandbox.rs:145`) with no tempfile lifecycle.
- **2.1 surface: `ServerStatus.schema_warnings: usize` + summary-line mention** so the lint is
  user-discoverable, not log-only.

## Finding 5.1 — trusted-MCP mutations must reach the post-exec validator

**Today:** `McpTool::intent()` encodes `Trust::Allow → Access::Read`
(`agent-mcp/src/tool.rs:72-75`), so `turn_mutated` (`agent-core/src/loop_.rs:828-834`, which
counts only `Write | Destroy` on `Executed::Ok`) never fires for a trusted MCP tool's
mutations, and configured `post_tool_validators` silently skip them. The Trust→Access
"zero policy change" acceptance predates the validator.

**Design:**

1. `agent-tools/src/types.rs` — add the variant:

   ```rust
   /// Third-party mutation pre-approved by config (MCP `Trust::Allow`): the
   /// approval gate auto-allows it, but post-execution validation treats it
   /// as a mutation. Never Destroy-tier.
   TrustedWrite,
   ```

2. `agent-policy/src/engine.rs:60` — new match arm with Read-arm semantics (paths checked for
   workspace containment; empty paths vacuously Allow). Gate behavior for today's MCP intents
   (always empty paths) is therefore **identical**: auto-allowed. The Destroy floor
   (engine.rs:52 command branch and the `Destroy => Ask` arm) is untouched.
3. `agent-mcp/src/tool.rs` — `Trust::Allow => Access::TrustedWrite`; update the encoding
   comment (it currently claims "zero policy change" via the Read axis) and the
   `allow_trust_maps_to_policy_allow` test (asserts `TrustedWrite` + `Decision::Allow`).
4. `agent-core/src/loop_.rs:828-834` — `turn_mutated` counts
   `Write | Destroy | TrustedWrite`.

**Pins:**
- engine: `TrustedWrite` with empty paths → `Decision::Allow`; with an out-of-workspace path →
  `Ask` (inherits Read-arm containment).
- mcp: `Trust::Allow` intent is `TrustedWrite` and still auto-allowed by `RulePolicy`.
- loop: a `TrustedWrite` stub tool (mirroring the existing `WriteStub` validator tests at
  loop_.rs:5593+) whose call succeeds sets `turn_mutated` and runs configured validators; a
  read-only turn still does not.

## Finding 2.1 — MCP schemas get a contract lint at connect

**Today:** `McpTool::schema()` injects the server's raw `description`/`input_schema` verbatim
(`agent-mcp/src/tool.rs:61-67`); `McpManager::connect` (`manager.rs:44-57`) just extends the
tool list. No MCP schema is ever contract-checked — the assemble.rs ratchet runs with
`mcp_tools: vec![]`.

**Design:** lint in `McpManager::connect`'s Ok arm, per wrapped tool, using the tool's own
`schema()`:

- **Checks:** (a) `description` empty after trim; (b)
  `agent_tools::required_params_missing_description(&schema)` (the existing helper in
  `agent-tools/src/contract.rs:25` — the plan verifies its export path and re-exports it from
  the crate root if it is not already public there).
- **Surfacing:** one `tracing::warn!(target: "mcp", server, tool, ...)` per violation naming
  what is missing; `ServerStatus` gains `pub schema_warnings: usize` (sum over the server's
  tools); `summary_line` appends the count when nonzero:
  `github ✓ (3 tools, 2 schema warnings)`.
- **Posture:** warn-don't-reject — tools still register, matching the duplicate-name
  precedent. The count is not persisted anywhere else (YAGNI).

**Pins:**
- connect test over `MockTransport` with one bad tool (empty description, undescribed required
  param) → `schema_warnings == 2`, summary line carries `2 schema warnings`, tool still
  registered.
- clean-schema connect test → `schema_warnings == 0`, summary line unchanged from today's
  format.
- `from_parts` and existing `ServerStatus` constructions updated.

## Finding 3.1 — MCP env secrets leave docker argv

**Today:** `docker_run_args` emits `-e KEY=VALUE` into the docker **client** argv
(`agent-sandbox/src/docker.rs:73-77`), fed from `mcp.json` `env` (conventionally API keys) via
`transport.rs:42`. Service-kind MCP containers keep that client — and its world-readable
`/proc/<pid>/cmdline` — alive for the whole session; `docker inspect` persists it too.
HostExecutor by contrast sets env privately on the child.

**Design:**

1. `docker_run_args` env loop emits **name-only** `-e KEY` (no value in argv). The
   `HOME=/tmp` default stays a literal `-e HOME=/tmp` when `spec.env` lacks HOME — it is
   non-secret and docker would otherwise not see a value for it. A spec-set HOME becomes
   name-only like every other key.
2. `spawn_docker` (`agent-sandbox/src/strategy.rs:179-186`) adds `cmd.envs(&spec.env)` on the
   docker client process — `docker run -e KEY` forwards the value from the client environment
   into the container without argv exposure.

Determinism: the env iteration order is whatever `CommandSpec.env`'s map yields today — the
change drops values from argv but does not alter ordering (the plan confirms the map type and
keeps the existing "sorted for determinism" behavior intact).

**Pins:**
- `docker_run_args` test: for a spec with `API_KEY=sekret`, argv contains the adjacent pair
  `-e API_KEY` and the string `sekret` appears **nowhere** in the argv vector.
- `home_defaults_to_tmp_unless_spec_sets_it` updated: default case still shows literal
  `-e HOME=/tmp`; spec-set case shows name-only `-e HOME` and no `HOME=/workspace` in argv.
- `spawn_docker`'s `cmd.envs` line is one statement; covered by review plus the pure-fn argv
  tests (spawning real docker in unit tests stays out of scope, matching existing tests).

## Out of scope

- Parsing MCP tool annotations (`readOnlyHint`/`destructiveHint`) for per-tool mutation
  fidelity — today every `Trust::Allow` tool is conservatively `TrustedWrite`. Revisit trigger:
  annotated servers become common in this runtime's configs.
- A description-length lint (no established token budget for tool prose).
- Any change to `Trust::Ask` behavior (already `Write`: gate asks, validator counts it).
- Persisting `schema_warnings` beyond `ServerStatus`/summary line.

## Testing & process

- Crate suites: `cargo test -p agent-tools -p agent-policy -p agent-mcp -p agent-sandbox
  -p agent-core` (from `agent/`), then full `bash scripts/ci.sh` before merge (now includes
  the okf and conditional src-tauri legs).
- The compiler surfaces any `Access` match this spec missed — treat new non-exhaustive-match
  errors as design input, not noise: each new site must decide what `TrustedWrite` means
  there, defaulting to "like Write" for mutation semantics and "auto-allow" only at the
  policy gate.
- Branch `feature/audit-mcp-seam` off `main`; per-task subagent review + whole-branch review +
  `--no-ff` merge; ledger section in `.superpowers/sdd/progress.md` (untracked, main
  checkout); post-merge re-stamps for 2.1/5.1/3.1 in
  `.agents/skills/harness-engineering/audit.md`.
