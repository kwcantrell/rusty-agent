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

## Findings (last audit: 2026-06-30)

Record one finding block per gap discovered. Multiple gaps per component → multiple blocks.
Update this section when the audit is re-run; stamp with the new date. On re-run: remove findings whose proposed fix has been applied, retain still-open ones, and add new ones.

---

**Finding 1 — Sandboxes: sandbox is opt-in, not the default**

```
severity: high
file:line: agent/crates/agent-core/src/loop_.rs:48-51
violated principle: "execution is isolated by default; capabilities are explicitly granted,
  not ambient" — SKILL.md Spine A component 3 (Sandboxes & execution)
concrete proposed fix: Change `sandbox: Option<Arc<dyn SandboxStrategy>>` in LoopConfig to a
  required field with a safe default (e.g. HostExecutor restricted to workspace root).
  Require callers who want unrestricted execution to opt in explicitly, not opt into sandboxing.
  A `None` sandbox currently means tools run with full ambient filesystem/network access.
```

---

**Finding 2 — Tools: no "when NOT to call" contract in the Tool trait**

```
severity: med
file:line: agent/crates/agent-tools/src/tool.rs:5-13
violated principle: "each tool has a clear name, tight description, and explicit 'when NOT to
  call' guidance; no thin endpoint wrappers" — Anthropic: Writing Tools for Agents (tier: eng-blog)
concrete proposed fix: Add a `when_not_to_call() -> &str` method to the Tool trait (default → ""),
  include it in ToolSchema, and add a lint/test asserting it is non-empty for all registered tools.
  Alternatively, enforce a prose convention in description() ("Must include a 'Do NOT use when …'
  clause") with a test that pattern-matches the descriptions at registration time.
```

---

**Finding 3 — Observability: per-turn token and latency events not verified**

```
severity: med
file:line: agent/crates/agent-server/src/runtime.rs (RuntimeState, ChannelEventSink)
violated principle: "every tool call and model turn is logged with enough context to replay and
  diagnose" — SKILL.md Spine A component 6; co-design harness with eval (arXiv 2503.16416)
concrete proposed fix: Verify ChannelEventSink emits a TurnObserved (or equivalent) event per
  model turn carrying prompt_tokens, completion_tokens, reasoning_tokens (when preserved),
  stop_reason, and per-tool-call duration_ms. If absent, add these fields so cost/latency can
  be metered and drift can be detected without re-running the session.
```

---

**Finding 4 — Instructions: skill files lack negative constraints**

```
severity: low
file:line: .agents/skills/harness-engineering/SKILL.md (pattern: all skill files in .agents/skills/)
violated principle: "a single, versioned source of truth per agent role; no contradictory or stale
  rule files" — SKILL.md Spine A component 1
concrete proposed fix: Add a "NOT TO DO" or "Forbidden" section to each skill file explicitly
  stating what the agent operating under that skill is NOT permitted to do (e.g. "NEVER edits
  code", "NEVER pushes to remote"). Currently most skill files describe capabilities only and
  are silent on constraints, leaving the model to infer hard limits from tone and context.
```

---

## Top 3 highest-leverage fixes

Ranked by impact (severity × remediation cost):

1. **[Component 3 — Sandboxes] Make sandbox non-optional in LoopConfig**
   `agent/crates/agent-core/src/loop_.rs` lines 48–51 — Change `Option<Arc<dyn SandboxStrategy>>`
   to a required field with a safe workspace-restricted default. Any tool call on a `None` sandbox
   today runs with full ambient access; fixing this closes the highest-severity gap in the harness
   with a single struct field change + callers audit.

2. **[Component 2 — Tools] Add and enforce "when NOT to call" in Tool trait**
   `agent/crates/agent-tools/src/tool.rs` lines 5–13 — Add `when_not_to_call()` to the `Tool`
   trait (or enforce a prose convention + test). Without explicit call-boundary guidance the model
   must infer tool scope from names and brief descriptions, the documented root cause of tool
   over-calling (Anthropic: Writing Tools for Agents).

3. **[Component 6 — Observability] Emit per-turn token + latency telemetry**
   `agent/crates/agent-server/src/runtime.rs` (`ChannelEventSink`) — Wire a structured per-turn
   event (prompt_tokens, completion_tokens, reasoning_tokens, stop_reason, tool_call_duration_ms)
   so cost, latency, and reasoning overhead can be tracked externally without re-running. Without
   this the harness is opaque between invocation and final output, making drift detection and
   harness-vs-model attribution impossible (arXiv 2503.16416, SKILL.md Spine A component 6).
