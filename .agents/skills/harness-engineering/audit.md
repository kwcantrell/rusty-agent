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

Re-stamp note (2026-07-01, context cluster): the deep audit's "cluster 5" — the **Context
Engineering** HIGH (torn eviction/compaction orphaning `Role::Tool` messages → mid-session 400;
Top-10 fix #5) plus the folded silent-eviction MED — is now **fixed and merged to `main`**
(5 commits, `c6a34d8..12a7841`, fast-forward): shared turn-unit helpers in
`agent-core/src/context.rs` (`turn_unit_ranges` / `evict_start` / `snap_split_to_unit_boundary` /
`orphaned_tool_positions`); both `build()`s evict whole units newest-first (keep-≥1-unit floor)
under `debug_assert!` orphan guards; the compaction split snaps left to a unit boundary; and a
change-deduped `ContextEvent::Evicted {messages, est_tokens}` emits on every `maintain` exit
(compaction arm extracted into `compact_old_span` to kill its early return), surfaced on CLI,
wire, trace, and web markers. Budget-sweep property tests pin the invariant for every limit.
See `docs/superpowers/specs/2026-07-01-turn-atomic-context-curation-design.md`. Follow-up
backlog (non-blocking, final review): dedup reset-re-emit test (the one untested spec behavior),
dedup keyed on count only (consider `(messages, est_tokens)`), non-cloning `pinned_tokens()`
helper. Design notes: `Evicted` is observable only after tool turns (emitted from `maintain`,
same siting as offload/compaction); the `debug_assert` would panic debug builds on a corrupted
pre-existing-orphan history if session rehydration ever lands.

Re-stamp note (2026-07-01, sandbox cluster): the deep audit's "cluster 1" — **Sandboxes &
Execution** (Component 3, Top-10 fixes #1 and #2) — is now **fixed and merged to `main`**
(9 commits, `ffc8ac8..be67413`, fast-forward): a degraded sandbox **refuses** exec-capable
launches instead of silently degrading to the host (`auto` stays the default; the error names
`sandbox_mode: "off"` as the explicit opt-out), with a self-healing re-probe (2 s bounded,
single-flighted) so Docker coming up mid-session recovers without restart; `HostExecutor` does
`env_clear()` + a six-var allow-list (PATH/HOME/LANG/LC_ALL/TERM/TMPDIR, `spec.env` wins) closing
the `AGENT_API_KEY` leak on every host path; `LoopConfig.sandbox` is a required field (the
fail-open `unwrap_or_else(HostExecutor)` is gone at the type level); MCP servers spawn with
`cwd = workspace` and are skipped loudly under refusal; `current_uid_gid()` falls back to nobody
(`65534:65534`), never `0:0`. See `docs/superpowers/specs/2026-07-01-sandbox-fail-closed-design.md`
and its plan. Accepted residuals (from the final whole-branch review): the test-only
`Default for LoopConfig` constructs `HostExecutor` by documented contract, not by type (four
integration suites depend on it); `enforce` refusals return the bare probe reason without the
actionable copy; and — **new residual for the next audit pass** — `claude_cli.rs:41` spawns the
Claude CLI model backend with the full inherited env including `AGENT_API_KEY` (trusted backend,
outside the tool-execution threat model, but the last child process that still sees the secret).

Re-stamp note (2026-07-01, tools cluster): the deep audit's "cluster 7" — the **Tools** HIGH
(unbounded tool-result ingestion; Top-10 fix #7) plus two folded build opportunities
(`read_file` pagination, `context_recall` pagination) — is now **fixed and merged to `main`**
(8 commits, `9f9ed70..fbf8ad7`, merge commit): an eager size-based pass as step (0) of
`CuratedContext::maintain` offloads any tool result over `OffloadConfig.max_result_bytes`
(default `DEFAULT_MAX_TOOL_RESULT_BYTES = 16 KiB`) WHOLE into the offload store on the same
pass — before the next model call — leaving a char-boundary-safe preview + recall marker whose
total is ≤ cap (idempotent by arithmetic; marker-only degenerate case starts with
`[tool_result#` so selectors skip it). `context_recall` pages by byte offset with each page
≤ the same budget (so recall can never re-trip the cap); `read_file` gained line-based
`offset`/`limit` (saturating arithmetic, `limit: 0` rejected); `RuntimeConfig.max_tool_result_bytes`
(serde-default, partial-merge aware) wires the cap into both frontends and the recall page
budget. Eager offloads reuse `ContextEvent::Offloaded` — zero wire/web changes. This also
partially defuses the Spine-B "single oversized message → unfixable over-limit request" MED
(tool results can no longer create an over-cap turn-unit; the compact-and-rebuild-on-overflow
half stays open, folded into Top-10 #8 territory). The eval harness intentionally neutralizes
the cap (`max_result_bytes: usize::MAX` in `eval/config.rs::offload_config` — not part of the
candidate genome; the context-evolve champion was validated without it). See
`docs/superpowers/specs/2026-07-01-tool-result-ingestion-cap-design.md` and its plan. Accepted
residuals (final whole-branch review): server settings change updates the recall page budget
on loop rebuild but the live context's cap only on workspace switch (bounded, convergent
drift); `lift` callers double-clone content (peak-memory nicety: clone only ~cap head); sliced
`read_file` normalizes CRLF; `MaintReport.offloaded_bytes` counts store writes (documented
double-count when a preview is later age-offloaded); offload store growth is still the
pre-existing RAM-only backlog item.

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

Ranked by impact (severity × remediation cost). All prior HIGH findings, the observability
cluster (per-call terminal events + durations, JSONL session traces, usage/cost parsing,
SessionStats + web panel, ContextEvent forwarding, CI gate), the sandbox cluster
(fail-closed degraded exec, env scrub, required `LoopConfig.sandbox`, MCP workspace cwd,
nobody uid fallback), the context cluster (turn-atomic eviction/compaction + visible
eviction), and the tools cluster (16 KiB ingestion cap + eager offload, recall/read_file
pagination) are **done** — for the full current backlog see
`docs/superpowers/audits/2026-07-01-harness-deep-audit.md` (its Top-10 table; items
1, 2, 3, 4, 5, 6, and 7 are now complete). Of this file's inline findings, one remains:

1. **[Component 1 — Instructions] De-duplicate the system prompt + add negative constraints** (Finding 1)
   `agent/crates/agent-server/src/daemon.rs:23` + `agent/crates/agent-cli/src/main.rs:15`; `.agents/skills/`
   — hoist the byte-identical coding-agent system prompt to one shared const, and add a
   "Forbidden"/negative-constraint section to the skill files. `low` severity; polish.
