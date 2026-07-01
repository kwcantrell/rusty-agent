# audit.md — harness auditor

**REPORTS ONLY. NEVER EDITS CODE.** Emit findings + ranked fixes; the human holds
the judgment gate (research principle: judgment over generation).

For each of the six Harness Anatomy components (Spine A in `SKILL.md`):
1. **Open and re-read** the anchored file(s) live — do not trust remembered internals; anchors drift.
2. Judge conformance against that component's checklist items + the corroborating principles.
3. Emit a finding: `{ severity, file:line, violated principle + source, concrete proposed fix }`.

Close with **"Top 3 highest-leverage fixes"** — ranked, each naming the component, the file:line, and the one-line fix.

---

## Finding schema

Each finding must carry all four fields:

```
severity:                  high | med | low
file:line:                 <repo-relative path>:<line or range>
violated principle:        <principle name> — <source URL or tier>
concrete proposed fix:     <one action the implementer should take>
```

## Severity rubric

| Level | Meaning |
|-------|---------|
| `high` | correctness / safety gap — could cause wrong outputs, data loss, or security boundary violation |
| `med` | leverage / efficiency gap — the harness works but misses significant performance or reliability upside |
| `low` | polish — inconsistency or missing guidance that does not affect correctness but adds friction |

Judge all findings from these three levels consistently across all six components.

---

## Thinly-sourced components — judge locally

The external research corpus thinly covers error-recovery/retry, guardrails/permission tiers, and
**parallel tool execution**. For these sub-topics, audit against the runtime's own existing patterns
as the local reference:

- **Parallel tool calls**: `agent/crates/agent-core/src/loop_.rs` already implements concurrent tool
  dispatch controlled by `DEFAULT_MAX_PARALLEL_TOOLS = 8`; audit correctness against its own
  contract and the `max_parallel_tools` field in `LoopConfig`.
- **Config-based limits**: `agent/crates/agent-runtime-config/src/lib.rs` (`HARD_FLOOR_DENYLIST`,
  `RuntimeConfig`) are the source-of-truth; audit against their own intended semantics.

Mark findings in these areas: "judge from first principles + this runtime's conventions," NOT
asserting external authority.

---

## Anchor table

Open these files fresh at audit time. Do not rely on remembered content — anchors drift.

| Harness component | Runtime anchor(s) to open |
|---|---|
| 1. Instructions & Rule Files | `.agents/skills/`, root `CLAUDE.md`, sub-agent prompts (no root AGENTS.md exists — audit whatever rule files are present) |
| 2. Tools | `agent/crates/agent-tools/src/tool.rs`, `agent/crates/agent-tools/src/types.rs`, `agent/crates/agent-core/src/context_tools.rs` |
| 3. Sandboxes & execution | tool-execution path in `agent/crates/agent-tools/`, `agent/crates/agent-server/src/runtime.rs` |
| 4. Orchestration logic | `agent/crates/agent-core/src/loop_.rs` (incl. parallel tool calls), `agent/crates/agent-model/src/protocol.rs` |
| 5. Guardrails / Hooks | permission/limit checks across agent-core/agent-server; `agent/crates/agent-runtime-config/src/lib.rs`; `agent/crates/agent-policy/src/engine.rs` |
| 6. Observability | logging/metering across agent-server/agent-core; offload/eval surfaces; `agent/crates/agent-server/src/runtime.rs` |
| (Context engineering — Spine B) | `agent/crates/agent-core/src/context.rs`, `agent/crates/agent-core/src/context_tools.rs`, `agent/crates/agent-core/src/offload.rs`, `agent/crates/agent-core/src/offload_policy.rs` |

---

## Per-component audit checklist

### 1. Instructions & Rule Files

Open `.agents/skills/` (list all skill files) and `CLAUDE.md`; note any sub-agent prompt strings
in the codebase.

Checklist:
- [ ] Each skill's frontmatter `description` is unambiguous and non-overlapping with other skills.
- [ ] Skill files include **negative constraints** — what the agent is forbidden from, not only what
      it can do.
- [ ] There is a single versioned source of truth per agent role; no duplicate or contradictory rule
      files.
- [ ] Sub-agent prompts, if any, are versioned and role-specific (not ad-hoc inline strings).
- [ ] `CLAUDE.md` project config is consistent with skill metadata; no contradictions.

Principle: "a single, versioned source of truth per agent role; no contradictory or stale rule
files" (SKILL.md Spine A, component 1).

### 2. Tools

Open `agent/crates/agent-tools/src/tool.rs`, `agent/crates/agent-tools/src/types.rs`, and
`agent/crates/agent-core/src/context_tools.rs`.

Checklist:
- [ ] `Tool` trait or `ToolSchema` enforces (or at least structurally encourages) "when NOT to call"
      prose — checked at registration or by test.
- [ ] Each tool `description()` is tight: what it does, when to use it, and when NOT to use it.
- [ ] No thin endpoint wrappers when a consolidated tool covers the same semantics.
- [ ] Tool names are unambiguous within the registry; no two tools share a plausible call pattern.
- [ ] `ToolSchema.parameters` property descriptions are non-empty for all required fields.

Principle: "each tool has a clear name, tight description, and explicit 'when NOT to call' guidance;
no thin endpoint wrappers when a consolidated tool will do" (SKILL.md Spine A, component 2;
corroborating: Anthropic — Writing Tools for Agents).

### 3. Sandboxes & execution environments

Open `agent/crates/agent-tools/src/tool.rs` (`ToolCtx`, `SandboxStrategy`), and
`agent/crates/agent-server/src/runtime.rs` (`LoopConfig.sandbox` field wiring).

Checklist:
- [ ] Sandbox default is safe — open `LoopConfig` and check whether the `sandbox` field is optional; if it is `Option<...>`, verify a safe default sandbox is always installed before any tool call runs (a `None` must not silently fall back to unrestricted host execution).
- [ ] Execution environment grants capabilities explicitly; ambient filesystem/network access is denied.
- [ ] Network egress is gated per-tool (`NetworkPolicy` in agent-http) — not globally open at the
      runtime level.
- [ ] Filesystem access outside the workspace root requires an explicit grant.

Principle: "execution is isolated by default; capabilities are explicitly granted, not ambient"
(SKILL.md Spine A, component 3).

### 4. Orchestration logic

Open `agent/crates/agent-core/src/loop_.rs` and `agent/crates/agent-model/src/protocol.rs`.

Checklist:
- [ ] `max_parallel_tools` is documented and has a safe, intentional default (not "0 meaning
      unlimited"); callers understand what they're wiring.
- [ ] Retry logic (`max_retries`) covers all distinct failure modes: model error, tool timeout, tool
      execution error, stream stall.
- [ ] Stop conditions (max turns, cancellation token) are reachable and correctly wired in
      `LoopConfig`.
- [ ] Any sub-agent spawning follows explicit routing rules — not "ask the model to decide" unless
      the decision is genuinely judgment-gated.
- [ ] Hand-off between orchestrator and worker (if used) aggregates results explicitly; no silent
      discard of tool-call outputs.

Principle: "explicit routing rules; no 'ask the model to decide' unless the decision is truly
judgment-gated" (SKILL.md Spine A, component 4; corroborating: Anthropic — Building Effective
Agents). Note: parallel tool calls are audited "from first principles + this runtime's conventions"
(see thinly-sourced clause above).

### 5. Guardrails / Hooks

Open `agent/crates/agent-runtime-config/src/lib.rs` (trace `HARD_FLOOR_DENYLIST`, `RuntimeConfig`)
and follow the permission/approval path through agent-core and agent-server.

Checklist:
- [ ] Deterministic pre-execution hook runs before every tool call (policy check → approval
      channel → execute, in that order).
- [ ] The approval channel is non-optional for write/destructive operations — not bypassed in
      non-interactive sessions.
- [ ] `HARD_FLOOR_DENYLIST` is reviewed, exhaustive, and tested.
- [ ] Hooks are fast and side-effect-free; no I/O between tool-call decision and execution that could
      introduce race conditions.
- [ ] Permission tiers are explicit (read vs. write vs. destroy) — not a flat allow/deny binary.

Principle: "hooks are fast, side-effect-free validators; they block bad actions, not delay good
ones" (SKILL.md Spine A, component 5).

### 6. Observability

Open the `EventSink` implementation in agent-server and trace what is emitted per tool call and per
model turn. Also inspect `agent/crates/agent-core/src/offload.rs` for eval/replay surfaces.

Checklist:
- [ ] Every tool call emits: tool name, args summary (or hash), result status, duration (ms).
- [ ] Every model turn emits: turn index, `prompt_tokens`, `completion_tokens` (and
      `reasoning_tokens` if preserved), `stop_reason`.
- [ ] Cost / latency metering is available per session, not only at session end or not at all.
- [ ] Enough context is logged to replay and diagnose a failed turn without re-running the session.
- [ ] Eval surfaces (offload store, eval harness) are wired to observability — results go somewhere
      actionable, not only to the model context.

Principle: "every tool call and model turn is logged with enough context to replay and diagnose;
without it, no way to tell if the agent is drifting" (SKILL.md Spine A, component 6; corroborating:
co-design harness with eval, arXiv 2503.16416).

---

## Example findings (last audit: 2026-06-30; re-stamped 2026-07-01 after the guardrails finding + its position-aware-denylist follow-up fixed)

*Illustrative snapshot from a 2026-06-30 six-component fan-out run — re-stamp or replace when you run the audit; these cite live line numbers that drift.*

Record one finding block per gap discovered. Multiple gaps per component → multiple blocks.
Update this section when the audit is re-run; stamp with the new date. On re-run: remove findings whose proposed fix has been applied, retain still-open ones, and add new ones.

Re-stamp note (2026-06-30): the two prior HIGH findings (sandbox silent-degradation, symlink escape)
**and** the two top `med` findings — parallel-dispatch failure-isolation and the tool "when NOT to
call" contract — are all now **fixed and merged to `main`**, and have been removed from the list
below. Parallel dispatch: each tool is panic- and timeout-isolated (`execute_isolated`, a grace-margin
backstop), see `docs/superpowers/specs/2026-06-30-parallel-tool-dispatch-hardening-design.md` and
commits `96ec134`/`7329bd1`. Tool contract: a `Tool::when_not_to_call()` folded into the model-facing
schema + required-param descriptions + a curated-confusable enforcement ratchet, see
`docs/superpowers/specs/2026-06-30-tool-when-not-to-call-contract-design.md` and commits
`955fc15`/`76fc4ae`/`0dc6cc2`.

Re-stamp note (2026-07-01): the prior top-ranked `med` **Guardrails** finding — the catastrophic-command
denylist's structural gaps (`mkfs`, the `:(){` forkbomb) and the un-timed `TerminalApproval` stdin read
— is now **fixed and merged to `main`**, and has been removed from the list below. A structural
`mkfs`/`mkfs.*` handler + an all-whitespace-removed backstop pass close the denylist gap; a configurable
`TerminalApproval` timeout (300s `Default`, matching the server) wraps the `spawn_blocking` stdin read in
`tokio::time::timeout` and denies on elapse. See
`docs/superpowers/specs/2026-07-01-guardrails-denylist-approval-hardening-design.md` and commits
`4408060`/`be58ca5`/`b147d7e`. The two findings below — one `med`, one `low` — remain open and are
renumbered 1–2.

Re-stamp note (2026-07-01, guardrails follow-up): the residual bare-`mkfs`/`sudo` **denylist
false-positive** flagged during the above finding's review — the position-blind substring backstop
hard-denied benign `man mkfs` / `man sudo` / `which sudo` — is now **fixed and merged to `main`**,
completing the Guardrails cluster. Catastrophe detection is made position-aware (a raw-string
command-boundary scan, "Layer A2", catches `sudo`/`mkfs` in program position incl. glued operators,
`$()`/backtick/subshell/group, env-prefix, and unparseable forms), `sudo`/`mkfs` were dropped from
both `HARD_FLOOR_DENYLIST` and `default_denylist()` (so the fix lands on the CLI **and** server), and
a name-exact catastrophe-token guard in `is_auto_allowed` keeps a catastrophe passed to an allowlisted
exec-capable program (`find . -exec sudo …`) out of auto-Allow (→ Ask). One **accepted, documented
residual**: catastrophes smuggled through allowlisted exec-vehicles that interpret their arguments
(`git -c core.pager="sudo …"`, `bash -c`, `cargo`, `find -exec sh -c`) reach Ask, not Deny — the hard
floor covers direct invocation, not arbitrary sub-command execution; mitigations are the allowlist
policy and the execution sandbox. See
`docs/superpowers/specs/2026-07-01-hard-floor-position-aware-denylist-design.md` (+ Addenda 1–2) and
commits `d1e0f21`/`1aadf3d`/`a616750`/`59ed233`. No open Guardrails finding remains.

Re-stamp note (2026-07-01, observability cluster): a full 7-dimension deep audit was run (report:
`docs/superpowers/audits/2026-07-01-harness-deep-audit.md` — treat it as the current findings
snapshot superseding this section's inline list). Its "cluster 2" — the prior **Observability**
Finding 1 plus the missing-CI eval finding — is now **fixed and merged to `main`** (16 commits,
tip `5009f71`): every resolved tool call emits a terminal `ToolResult{id,status,duration_ms}`
(denied/error/timeout/panic included), reasoning/cached tokens + claude-cli `total_cost_usd` are
parsed, a JSONL `TraceWriter` persists every session by default (`~/.agent/sessions/`, 64 MB cap,
keep-50), `SessionStats` feed a CLI summary line + web StatsPanel, `ContextEvent`s reach both UIs,
and a CI gate exists (`scripts/ci.sh` + `.githooks/pre-push` + GitHub Actions). See
`docs/superpowers/specs/2026-07-01-harness-observability-ci-design.md` and its plan. Follow-up
backlog (non-blocking, from the final whole-branch review) lives in the merge notes:
turns=max(turn) semantics, session_stats query has no client caller yet, trace file perms 0600,
live trace toggle needs restart, id-based tool correlation in the web reducer. Remaining open
finding below renumbered to 1.

---

**Finding 1 — Instructions: duplicated system prompt + skill files lack negative constraints**

```
severity: low
file:line: agent/crates/agent-server/src/daemon.rs:23 + agent/crates/agent-cli/src/main.rs:15;
  .agents/skills/ (pattern)
violated principle: "a single, versioned source of truth per agent role; no contradictory or stale
  rule files" — SKILL.md Spine A component 1
concrete proposed fix: The coding-agent system prompt is byte-identical but duplicated as two
  constants in two crates — hoist to agent-runtime-config as one shared const. Separately, most skill
  files describe capabilities only; add a "Forbidden"/negative-constraint section per skill (esp.
  wayland), and disambiguate the auto-drive-tauri↔wayland description overlap.
```

---

## Top highest-leverage fixes

Ranked by impact (severity × remediation cost). All prior HIGH findings and the observability
cluster (per-call terminal events + durations, JSONL session traces, usage/cost parsing,
SessionStats + web panel, ContextEvent forwarding, CI gate) are **done** — for the full current
backlog see `docs/superpowers/audits/2026-07-01-harness-deep-audit.md` (its Top-10 table; items
3, 4, and 6 are now complete). Of this file's inline findings, one remains:

1. **[Component 1 — Instructions] De-duplicate the system prompt + add negative constraints** (Finding 1)
   `agent/crates/agent-server/src/daemon.rs:23` + `agent/crates/agent-cli/src/main.rs:15`; `.agents/skills/`
   — hoist the byte-identical coding-agent system prompt to one shared const, and add a
   "Forbidden"/negative-constraint section to the skill files. `low` severity; polish.
