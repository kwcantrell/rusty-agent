# Post-execution validator hook

**Date:** 2026-07-02
**Status:** Approved (2026-07-02 product-decision round, item 6)
**Branch:** `feat/post-tool-validator`

## Problem

After a mutating tool runs (write_file, edit_file, git_commit, a writing
execute_command), nothing checks that the edit left the workspace in a good
state. Tool results flow `execute_isolated` → `ToolResult` emit → `ctx.append`
(`agent-core/src/loop_.rs:772-849`) with **no post-execution validation step**.
This is the last unbuilt Guardrails/Hooks capability from the harness audit — the
classic "deterministic feedback loop": run `cargo check` / a linter / `npm run
typecheck` after an edit and hand any failure straight back to the model so it
fixes the break on the next turn instead of drifting.

## Design

A config-driven, **once-per-turn** validator pass that runs after a turn's tool
batch if any mutating tool call succeeded, executes configured shell commands
through the sandbox, and appends failures to the context as a user message.

### Configuration

`RuntimeConfig.post_tool_validators: Vec<String>` (serde-default **empty** —
disabled, zero behavior change and zero cost when unset; fully opt-in). Each
entry is a shell command line (e.g. `"cargo check --quiet"`), run via `sh -c`.
Plumbed to `LoopConfig.post_tool_validators: Vec<String>`, wired in
`assemble.rs::loop_config_from` following the existing field pattern. Serde-
default + `PartialRuntimeConfig` mirror + merge arm (additive, old files parse).

### Trigger

After Phase 2 (execute) and Phase 3 (append tool messages), once per turn, run
the validators **iff**:
1. `post_tool_validators` is non-empty, **and**
2. at least one tool call this turn had `Access::Write` or `Access::Destroy`
   **and** resolved `Executed::Ok` (a successful mutation).

Read-only turns, and turns where every mutation failed, skip validation. To know
per-call access, `ReadyCall` gains an `access: Access` field (populated from the
`intent.access` already computed in `gate_tool`), threaded into the `executed`
tuple; `turn_mutated` is a simple `.any()` over successful Write/Destroy calls.

Accepted over-trigger: `execute_command` is always `Access::Write` even for a
read-only command (`git status`), so a read-only shell call still triggers
validation. The validator is idempotent and only appends on failure; the user
opted in. Documented, not gated further.

### Execution

Each configured command runs sequentially via a new sink-free helper
`run_validator(sandbox, workspace, command, timeout, cancel) -> ValidatorOutcome`,
mirroring `shell.rs`'s launch + concurrent-pipe-drain pattern:
- `CommandSpec { program: "sh", args: ["-c", command], cwd: workspace,
  env: default, kind: OneShot }` through `self.config.sandbox` (the same executor
  tools use — so a degraded sandbox **refuses** it, exactly like tool exec).
- Timeout = `self.config.tool_timeout`; honors `cancel` (skip on cancel).
- Captures exit code + combined stdout/stderr, truncated to the first 4 KiB
  (char-boundary safe, `…(truncated)` marker) per command.
- `ValidatorOutcome` variants: `Passed`, `Failed { code, output }`,
  `Skipped { reason }` (sandbox refused / spawn error / cancelled). Validation is
  **best-effort**: a runner failure NEVER fails the run.

Trusted config (user-set, not model-set) → validators are NOT policy/approval-
gated, but ARE sandboxed for isolation.

### Feedback to the model

If any validator returns `Failed`, append **one** user message after the turn's
tool results (before the stuck-nudge append and `maintain` — same OpenAI-compat
siting as the stuck nudge):

```
Post-edit validation reported problems. Fix these before continuing:

$ cargo check --quiet  (exit 101)
<captured output>

$ <next failed validator> …
```

All-pass → append nothing (silent; no token cost). `Skipped` outcomes are noted
in a trailing line only if at least one validator also `Failed` (a fully-skipped
pass appends nothing, matching the all-pass silence, but emits observability).

### Observability

Each validator run emits a synthetic `ToolStart`/`ToolResult` pair riding the
**existing** event frames (old-SPA safe — no new event kinds):
- `ToolStart { id: "validate:{n}", name: "post_tool_validate", args: {"command": …} }`
- `ToolResult { id, name: "post_tool_validate", status: Ok (Passed) | Error
  (Failed/Skipped), output: <captured or skip reason>, duration_ms }`

Synthetic ids are turn-unique (`validate:{turn}:{n}`) so the web reducer's
id-correlation pairs them into their own rows. In a dispatch child, the
`SubagentSink` stamps `parent_id` on these exactly as it does other child tool
events — attribution is free.

### Interactions

- **max_turns wrap-up (cluster A):** validation runs inside the turn loop; the
  budget wrap-up is untouched (a wrap-up turn issues no tools → no mutation → no
  validation).
- **Dispatch children:** inherit via the `LoopConfig` clone. A child that edits
  files gets validated; its synthetic events attribute to the parent. Accepted
  cost (opt-in list; once-per-turn, not per-edit). A `subagent`-scoped disable is
  a recorded follow-up, not built.
- **Calibration/stats:** the synthetic `ToolResult` folds into `SessionStats`
  tool counts (a validator run counts as a tool call). Documented; acceptable
  (it IS a tool execution). No `ServerUsage`, no turn-index change.

## Tests

Loop tests (in `loop_.rs`, using `ScriptedModel`/`CollectingSink`/`DetailSink`
and a real temp workspace) with a trivial validator command (`sh -c` friendly,
e.g. `"true"` / `"false"` / `"echo boom >&2; exit 1"`):
- Write-tier tool + failing validator → a user message with the validator output
  is appended, and a `post_tool_validate` ToolResult with Error status is emitted.
- Write-tier tool + passing validator → NO appended user message; a
  `post_tool_validate` ToolResult with Ok status IS emitted.
- Read-only turn (a `read_file` call only) → validators do NOT run (no
  `post_tool_validate` event).
- Empty `post_tool_validators` → no validation events, no behavior change
  (regression pin).
- Mutation that FAILED (tool returned Err) → validators do NOT run.
- Multiple validators, first passes second fails → one appended message naming
  only the failed one; two `post_tool_validate` events.
- Cancelled run → validators skipped (no hang).

Config tests (`runtime_config.rs`): serde default empty; round-trip; partial-file
merge; `assemble.rs` passthrough into `LoopConfig`.

`ValidatorOutcome` truncation: a validator emitting >4 KiB is capped with the
marker (unit test on the helper).

## Out of scope (recorded)

- Per-validator trigger filters (run validator X only after tool Y) — the trigger
  is the whole Write/Destroy-succeeded set; finer routing is a follow-up.
- A `subagent`-scoped validator disable / separate child validator list.
- Blocking semantics (a validator that *vetoes* the edit / rolls it back) — this
  hook is advisory feedback, never a gate. The write already happened.
- Auto-fix (running a formatter that mutates) — validators are read-only checks
  by convention; a mutating validator's writes are not re-validated or surfaced
  beyond its exit code.
