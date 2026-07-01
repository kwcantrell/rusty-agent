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

## Example findings (last audit: 2026-06-30; re-stamped 2026-06-30 after both HIGHs fixed)

*Illustrative snapshot from a 2026-06-30 six-component fan-out run — re-stamp or replace when you run the audit; these cite live line numbers that drift.*

Record one finding block per gap discovered. Multiple gaps per component → multiple blocks.
Update this section when the audit is re-run; stamp with the new date. On re-run: remove findings whose proposed fix has been applied, retain still-open ones, and add new ones.

Re-stamp note (2026-06-30): both prior HIGH findings are **fixed and merged to `main`** and have been
removed from the list below. The sandbox silent-degradation gap now emits a first-class
`AgentEvent::SandboxDegraded` at run start (loud CLI line + web/desktop banner fed from
`settings_state`); the lexical-only workspace path check now canonicalizes the existing path
components (and chases dangling symlinks), closing the symlink escape. See
`docs/superpowers/specs/2026-06-30-sandbox-degraded-signal-design.md` and commits `344a40c` (symlink)
plus `9cf9562`/`262e764`/`9cf68e7` (degraded signal). The five findings below — all `med`/`low` —
remain open and are renumbered 1–5.

---

**Finding 1 — Orchestration: parallel tool dispatch is not failure-isolated**

```
severity: med (high-impact reliability)
file:line: agent/crates/agent-core/src/loop_.rs:275-311
violated principle: "hand-off aggregates results explicitly; no silent discard of tool-call
  outputs" — SKILL.md Spine A component 4 (judged from first principles + runtime conventions)
concrete proposed fix: Three related gaps in the buffer_unordered path — (a) futures run inline with
  no catch_unwind, so ONE tool panic aborts the whole AgentLoop; (b) no tokio::time::timeout wraps
  tool.execute and fs/write/render ignore ctx.timeout, so one stalled tool hangs all parallel slots;
  (c) the `None => continue` at :298 silently drops a tool result, leaving a tool_call_id with no tool
  message (invalid conversation state). Wrap each execute in catch_unwind + timeout; replace the drop
  with an explicit error result so every tool_call_id always yields exactly one message.
```

---

**Finding 2 — Tools: no "when NOT to call" contract in the Tool trait**

```
severity: med
file:line: agent/crates/agent-tools/src/tool.rs:4-13 (+ types.rs:9-14, registry.rs:13)
violated principle: "each tool has a clear name, tight description, and explicit 'when NOT to
  call' guidance; no thin endpoint wrappers" — Anthropic: Writing Tools for Agents (tier: eng-blog)
concrete proposed fix: [CONFIRMED from prior run] Trait exposes name/description/schema/intent/execute
  with no exclusion-prose slot; ToolRegistry::register is a bare HashMap insert. Add
  `when_not_to_call() -> Option<&str>` + a registration test requiring it for name-overlapping tools
  (e.g. recall vs context_recall). Also: 7 of ~19 tools give required params `{"type":"string"}` with
  no description — add descriptions to every required field.
```

---

**Finding 3 — Observability: tool failures, durations, and context events are invisible**

```
severity: med
file:line: agent/crates/agent-core/src/loop_.rs:304-308; wire.rs:103; agent-model/src/openai.rs:205-209
violated principle: "every tool call and model turn is logged with enough context to replay and
  diagnose" — SKILL.md Spine A component 6; co-design harness with eval (arXiv 2503.16416)
concrete proposed fix: [REFINED] prompt/completion tokens ARE emitted per turn (ServerUsage). Still
  open: (a) Resolved::Err emits no event — tool denials/errors are invisible to observers/evals;
  (b) no Instant measures tool duration_ms; (c) reasoning_tokens streamed as text but never counted
  (undercounts spend); (d) ContextEvent (offload/compaction) dropped at wire.rs:103, so the web UI
  never learns the window was truncated. Add a tool-error/status event + duration_ms, parse
  reasoning_tokens, and forward ContextEvents to the wire.
```

---

**Finding 4 — Guardrails: catastrophic-command denylist has structural gaps; CLI approval can hang**

```
severity: med
file:line: agent/crates/agent-policy/src/command.rs:59-76; agent/crates/agent-cli/src/approval.rs:13-28
violated principle: "hooks are fast, side-effect-free validators; block bad actions, not delay good
  ones" — SKILL.md Spine A component 5
concrete proposed fix: (a) Structural Layer-A detection covers sudo/rm/dd but NOT `mkfs` or the
  `:(){` forkbomb (they rely solely on the substring backstop) and no test exercises mkfs through
  hard_floor_violation — add structural handlers + tests. (b) TerminalApproval's spawn_blocking
  stdin read has NO timeout (unlike IpcApprovalChannel) — a caller holding stdin open hangs the agent;
  add a timeout defaulting to Deny.
  NOTE: gate ordering (policy → approval → execute) and default-Deny-when-no-approver are both SOUND.
```

---

**Finding 5 — Instructions: duplicated system prompt + skill files lack negative constraints**

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

## Top 3 highest-leverage fixes

Ranked by impact (severity × remediation cost). Both prior HIGH-severity entries (silent sandbox
degradation, symlink escape) are **done** — the remaining `med` findings re-rank as follows:

1. **[Component 4 — Orchestration] Harden the parallel-tool dispatch** (Finding 1)
   `agent/crates/agent-core/src/loop_.rs:275-311` — one focused change closes three gaps at once:
   catch per-tool panics (so one tool can't kill the session), wrap execute in
   `tokio::time::timeout(ctx.timeout,…)` (so a stalled fs tool can't hang all slots), and replace the
   silent `None => continue` with an explicit error result (so every tool_call_id yields one message).
   Best reliability leverage per unit effort; complements the existing parallel-tool work.

2. **[Component 5 — Guardrails] Close denylist gaps + the CLI approval hang** (Finding 4)
   `agent/crates/agent-policy/src/command.rs:59-76`; `agent/crates/agent-cli/src/approval.rs:13-28` —
   add structural handlers (+ tests) for `mkfs` and the `:(){` forkbomb (currently substring-backstop
   only), and give `TerminalApproval`'s stdin read a timeout defaulting to Deny so a caller holding
   stdin open can't hang the agent. Safety-adjacent, small, well-scoped.

3. **[Component 6 — Observability] Make tool failures + durations visible** (Finding 3)
   `agent/crates/agent-core/src/loop_.rs:304-308`; `wire.rs:103`; `agent-model/src/openai.rs:205-209` —
   emit a tool-error/status event + `duration_ms`, count streamed `reasoning_tokens`, and forward
   `ContextEvent`s to the wire so the web UI learns the window was truncated. Highest
   diagnose-and-eval leverage; pairs with the eval-harness work.
