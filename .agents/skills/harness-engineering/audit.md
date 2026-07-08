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

Re-stamp note (2026-07-01, retry cluster): the deep audit's "cluster 8" — the **Orchestration**
MED (unclassified retries; Top-10 fix #8) plus three folds (the Spine-B "compact-and-rebuild
on context overflow" MED half deferred from cluster 7, the terminal-`Done`-parity MED, the
backoff LOW part) — is now **fixed and merged to `main`** (7 commits, `b0d2de5..c2abcc1`,
merge commit): `ModelError::class() -> ErrorClass {Retryable, Fatal, ContextOverflow}` lives
on the type in `agent-model` — Fatal (non-408/429 4xx, initial-response Decode) aborts on
first sight, Retryable (transport/stream/process/timeout/5xx/408/429) retries with
exponential backoff (100ms·2^(attempt−1), 5 s cap), and overflow (`Status{400|413|422}` or
`Stream` body matching five conservative signatures, checked before the 4xx rule) defers to
turn level: `ctx.request_compaction()` (new provided `ContextManager` method; `CuratedContext`
sets the same flag the `context_compact` tool uses) + mid-turn `maintain()` + rebuild via a
shared `completion_request` helper, retried ONCE per turn without consuming retry budget.
`ModelError::Cancelled` replaces the spoofable `Stream("cancelled")` encoding. All eight
`run_with_cancel` terminal paths now emit `Done` (new `StopReason::Error`, additive on
wire/trace; web reads reason opaquely). Testkit gained `Scripted::Fail(ModelError)`. See
`docs/superpowers/specs/2026-07-01-retry-classification-design.md` and its plan. Accepted
residuals (final whole-branch review — merge-clean, follow-up candidates): overflow recovery
does NOT fire on the claude-cli backend (`Process` bodies aren't signature-checked — one-arm
fix + spec edge-case correction, the highest-value follow-up); `AgentError::Cancelled` is now
dead code; the turn's `Usage` event is stale after a mid-turn rebuild; recovery is visible
only via tracing (no frontend event); retry tests use real ~400 ms sleeps and no in-situ
backoff-growth pin (spec's paused-clock test deviation); Retry-After/jitter deferred. Design
interaction worth remembering: overflow recovery is now the runtime safety net for the known
token-estimate undercount, raising the value of the deferred server-usage-calibrated
budgeting item.

Re-stamp note (2026-07-01, permissions cluster): the deep audit's final cluster — the two
**Guardrails** MEDs (subcommand-unaware allowlist, Top-10 fix #9; memory tools declared
Read, Top-10 fix #10) plus the folded third Guardrails MED (the 2-state engine fold) and
the Tools-component git_status/execute_command friction asymmetry — is now **fixed and
merged to `main`** (7 commits, `21e5fb0..366bf09` merge commit). `Access::Destroy` is a
third tier (`agent-tools/src/types.rs`, additive Serde variant): its floor is Ask, never
auto-allowed — enforced twice (engine command-branch guard `access != Destroy &&
is_auto_allowed`, plus an explicit non-command `Destroy => Ask` arm; hard floor still
Denies first). `is_auto_allowed` allowlist entries are now whitespace-token PREFIXES
(one-word = legacy program match; `"git status"` gates the subcommand; unknown
subcommands fail safe to Ask), and `default_allowlist()` swapped bare `git`/`cargo` for
read-safe prefixes (git: status/log/diff/show/blame/rev-parse/ls-files; cargo:
build/check/test/fmt/clippy/metadata/tree) — `git push --force`/`reset --hard`/`clean
-fdx` now reach Ask, not Allow. agent-memory re-tiered: remember=Write, recall=Read,
forget=Destroy (the deliberate "declare Read so RulePolicy auto-allows" bypass comment
removed); every remember/forget now prompts. Friction symmetry pinned by test:
`execute_command("git status …")` and the `git_status` tool both Allow. See
`docs/superpowers/specs/2026-07-01-destroy-tier-subcommand-allowlist-design.md` and its
plan. Accepted residuals (final whole-branch review — merge-clean): `git
{log,diff,show} --output=<path>` is an auto-allowed arbitrary-file write under the
default prefixes (pre-existing under bare `git`; documented at `default_allowlist()`;
`--output`/`-o` arg-scanning is the follow-up candidate); saved user configs that
persisted bare `git`/`cargo` keep legacy semantics (user-owned, per-field merge, no
migration); the plan's `git diff HEAD~1` test assertion was impossible (`~` ∈
SHELL_SIGNIFICANT — over-ask accepted, test pins `git diff HEAD`); MCP Trust→Access and
HTTP Host→Access posture encodings, Write-vs-Destroy FS granularity (tracked-file
overwrite vs scratch write), wire Access surfacing, and ApproveAlways persistence stay
out of scope. **This closes the deep audit's Top-10.**

Re-stamp note (2026-07-01, retry follow-up batch): the retry cluster's accepted-residual
batch is now **fixed and merged to `main`** (6 commits, `1400ede..8bab0d6` merge commit; spec
`docs/superpowers/specs/2026-07-01-retry-followup-batch-design.md`). (1) `ModelError::Process`
bodies are overflow-signature-checked (one guard arm mirroring `Stream` in `class()`,
`agent-model/src/types.rs`) — claude-cli overflow now triggers the once-per-turn
compact-and-rebuild; end-to-end pinned via `Scripted::Fail(Process(..))`; the original retry
spec's wrong edge-case claim ("only the Stream body check can catch it") carries a dated
correction. (2) Recovery is observable everywhere: payload-free `ContextEvent::OverflowRecovery`
emitted BEFORE `maintain()` (fires even when compaction no-ops), wire/trace kind
`"overflow_recovery"`, CLI render line, web `describeContext` case; and `AgentEvent::Usage` is
re-emitted after the rebuild so the turn's estimate reflects the rebuilt request (all consumers
verified replace-or-ignore: web reducer replaces, `SessionStats::fold` ignores `Usage`, CLI
ignores, trace appends). (3) Dead `AgentError::Cancelled` deleted (zero refs across both
workspaces). (4) Retry tests run on tokio paused clocks (3 conversions) plus an in-situ
backoff-growth pin: virtual elapsed exactly 700 ms (100+200+400) across three retries.
Accepted residuals: a second overflow-classified error in a turn is FATAL by design (spec
narrative corrected 74cd21b — bounded fail-fast, not Retryable fallback); overflow-recovery
Usage assertion is `>= 2` without turn-parity pin; signatures scan the whole Process stderr
(anchor to the CLI error prefix only if a real transient ever echoes an overflow phrase);
Retry-After/jitter and server-usage-calibrated budgeting stay deferred.

Re-stamp note (2026-07-01, sub-agent dispatch build): the deep audit's **missing capability #1**
— sub-agent / decompose-and-delegate orchestration (Component 4's top build opportunity) — is now
**BUILT and merged to `main`** (10 commits, `84050d9..0ab9b1c`, merge `af4dd14`; spec
`docs/superpowers/specs/2026-07-01-subagent-dispatch-core-design.md`, sub-spec #1 of 3). A
`dispatch_agent` tool (sub-agents-as-tools, per `build.md`'s patterns) in
`agent-core/src/dispatch.rs` runs a nested `AgentLoop` per call: fresh `CuratedContext` with its
own offload store/compact flag + child-bound context tools; child registry = a snapshot of the
parent's tools taken **before** context-tool/dispatch registration (structural depth-1 no-recursion,
plus an in-tool defense-in-depth skip); the child holds the parent's **exact**
policy/approval/sandbox Arcs — a sub-agent is never more privileged than its parent, Ask prompts
route to the parent's channel on both surfaces (pinned by tests). Guards: additive
`Tool::timeout_override()` (dispatch runs under `subagent_timeout_secs`, default 600 s; the
`execute_isolated` 2× backstop scales with it), child-token cancellation (parent cancel propagates;
child self-cancel never travels up), child `max_turns = subagent_max_turns` (default 10),
`subagents: false` disables registration. Observability v1 rides existing frames (old-SPA
compatible, zero wire/web changes): `SubagentSink` forwards child ToolStart/ToolResult with
`sub{n}:` ids + `sub:` name prefix and ServerUsage verbatim (cost truth), captures token text for
the tool result (final-turn tail + `[sub-agent: N turns, M tool calls, stop: …]` footer), suppresses
the rest. `TerminalApproval` prompts now serialize via an internal gate (parallel children).
Accepted residuals (final whole-branch review, merge-clean): child tool calls/turns fold into
session stats by design (D9-documented distortion); a dispatch wall-clock timeout drops a child
mid-approval → bounded stale IPC prompt (joins the pre-existing approval-wait-vs-cancel residual);
the `tools` allowlist arg rejects context-tool names the child implicitly has (model-facing error
clarity, sub-spec #2); a timed-out `TerminalApproval` orphan thread can still race the next
prompt's stdin read (documented in `approval.rs`). Sub-spec #2 (nested wire hierarchy, trace
linkage, UI attribution, `ToolCtx.call_id`) and #3 (per-child model routing — first customer
`run_compaction` — role prompts, depth>1, fan-out ergonomics) remain open backlog. **Missing
capability #1 is closed; #2 (Examples context type) is the remaining absent capability.**

Re-stamp note (2026-07-02, sub-agent observability): sub-spec **#2 of the sub-agent capability —
surfaces & observability — is BUILT and merged to `main`** (10 commits, `17eae71..2b86d13`, merge
`0224383`; spec `docs/superpowers/specs/2026-07-02-subagent-observability-design.md`). Attribution
rides existing frames as `skip_serializing_if` optional fields (old-SPA byte-compat verified —
duck-typed parse ignores unknown fields): `ToolCtx.call_id` (filled by `gate_tool`) is the lineage
root; `SubagentSink` stamps `parent_id` on forwarded ToolStart/ToolResult/ServerUsage across
event/wire/trace schemas; suppressed child events (Token/Done/etc.) tee to a `SubagentTrace` tap →
`TraceWriter::record_child` writes them into the same session JSONL with `sub` ordinal +
`parent_id` (same seq counter — full interleaved child transcripts are now replayable, closing the
"failed child turn can't be replayed" gap); `SessionStats` gained subset counters
(`subagent_tool_calls ⊆ tool_calls`, `subagent_turns`) and child turns no longer distort
`turns = max(turn)` (v1 D9 distortion killed); CLI indents attributed rows (`  ↳ `); the web
reducer switched to **id-first tool correlation** (closing the observability cluster's old
name-correlation backlog item), renders `parentId` rows nested with the `sub:` prefix stripped,
guards the turn readout against child usage flicker, and StatsPanel shows a conditional Sub-agent
row. The `tools` allowlist now accepts the always-available child context tools (the #1 residual
assigned here). Accepted residuals (final whole-branch review, merge-clean): `session_stats`
always carries the two zero-valued subagent fields (E5-sanctioned additive exception, spec has a
dated correction); parallel-dispatch child rows interleave ambiguously under flat+indent (accepted
design); `tool_time_ms` double-counts child tool time inside the dispatch call's duration
(pre-existing v1); child approval wire prompts remain unattributed (v1 residual); dead
`ToolCall.tsx` parity copy pending cleanup. **Sub-spec #3 (per-child model routing — first
customer `run_compaction` — role prompts, depth>1, fan-out ergonomics) is the remaining open
sub-agent work; Examples context type remains the last absent capability.**

Re-stamp note (2026-07-02, advanced dispatch — SUB-AGENT CAPABILITY COMPLETE): sub-spec **#3 is
BUILT and merged to `main`** (7 commits + fix wave, `fef6209..4b2e7c5`, merge `d19625b`; spec
`docs/superpowers/specs/2026-07-02-subagent-advanced-dispatch-design.md`), closing the sub-agent
capability's planned scope (#1 `af4dd14`, #2 `0224383`, #3 `d19625b`). **Model routing:**
`ModelRef` partial-override config (every None inherits the primary; `protocol` override for
child loops) with two slots — `subagent_model` (child loops, incl. their retries/overflow
recovery) and `compaction_model` (both `MaintCtx` sites via `AgentLoop::with_compaction_model`;
one Arc shared by the parent loop and the whole dispatch subtree — the audit's "first customer
`run_compaction`" is landed). Routed clients are built centrally in `assemble_loop` from
frontend-supplied `LoopParts.api_key`/`claude_binary`; a `ModelRef` that switches the child
backend to claude-cli defaults the child protocol to `"prompted"`. **Role:** bounded (≤2000
chars) `role` arg → `Role:` block in the child system prompt. **Depth:** `subagent_max_depth`
(default 1 = unchanged; clamped ≥1); nested dispatch tools carry `depth+1` and an
`id_prefix = "sub{n}:"` so a grandchild's `parent_id` is the child's VISIBLE row id —
attribution chains transitively across the #2 surfaces with zero web/wire changes (hand-verified
to depth 3 in review); the `tools` allowlist now gates AND transitively scopes nested dispatch
(monotonically narrowing — a descendant can never widen its inherited tool set). **Fan-out:**
description sentence only (parallel tool_calls already provide the mechanism). Accepted
residuals: routed endpoints are config-controlled and receive the api_key + conversation
content (same posture as the wire-editable primary `base_url`); non-claude-cli children
contribute $0 to `cost_usd` and mixed-backend token totals conflate tokenizers;
`compaction_model.protocol` ignored by design; grandchild web rows render at child indent
(flat+indent); depth-2 wall-clock nesting is structurally argued, not separately tested.
Deferred (future work, on demand): skill-defined agent types / role registry, models map,
per-call model override, live child token streaming. **The audit's remaining absent capability
is the Examples context type — the last open item from the 2026-07-01 deep audit.**

Re-stamp note (2026-07-02, Examples context type — DEEP AUDIT FULLY CLOSED): the audit's
**missing capability #2 — the Examples context type — is BUILT and merged to `main`** (7 commits
+ fix wave, `7d9e253..c899e7b`, merge `fdf37fd`; spec
`docs/superpowers/specs/2026-07-02-examples-context-type-design.md`). Per-skill worked exemplars
live under `<skill>/examples/` (`Skill.examples` = that subset of the bundled files) and surface
through the existing progressive-disclosure levels: L1 `list_skills` marks example-bearing skills
(`[N examples]`, pluralized); L2 `use_skill` renders a distinct `## Examples (worked exemplars,
dir: …)` section with skill-relative paths and the imitate-don't-copy contract line, and the
bundled-files section switched to relative paths with the absolute dir stated in the header
(the read/exec guidance is self-consistent for both consumption paths); L3 `read_skill_file` is
unchanged; `create_skill` teaches the convention in prose; `SKILLS_AWARENESS` grew by exactly one
sentence (the feature's whole prompt-side static cost — `create_skill`'s schema prose also grew
~30 words/request, H5-sanctioned). Model-initiative, pay-per-use — no injection, no new tools,
no new caps. **Session discovery worth remembering:** `list_bundled_files` was NON-recursive all
along — nested skill files (`references/…`) were L3-readable but never listed at L2; recursion
was adjudicated in (dated spec correction) with symlink-traversal hardening (`DirEntry::file_type`
gating — symlinked dirs/files never listed; loop-pinned). Accepted residuals: L3 reads follow
symlinks (pre-existing lexical guard.rs limitation — unlisted-but-readable asymmetry, no new
access); unbounded L2 listing for filesystem-authored skills (default ingestion cap degrades
gracefully; the eval harness pins that cap OFF, so example-aware eval tasks should watch it;
"…and N more" truncation is the cheap fix); auto-injection (H6b) deferred pending eval evidence
that model-initiative loading under-triggers — the context-evolve harness is the natural
measuring ground. **With this, every finding and both build opportunities from
`docs/superpowers/audits/2026-07-01-harness-deep-audit.md` are closed: Top-10 fixed, sub-agent
capability built (3 sub-specs), Examples built. Only this file's inline Finding 1 (low-severity
prompt de-dup/negative-constraints polish) and per-cluster accepted residuals remain as backlog.
The next audit run starts from a clean slate.**

Re-stamp note (2026-07-02, instructions cluster — backlog drain 1/6): this file's inline
**Finding 1** — the last open finding — is now **fixed and merged to `main`** (4 commits,
`db6fde6..2537f0f`, merge `bc8934e`; spec
`docs/superpowers/specs/2026-07-02-instructions-single-source-design.md`). The coding-agent
system prompt lives in exactly one place (`agent-runtime-config/src/prompts.rs::BASE_SYSTEM_PROMPT`,
re-exported as `daemon::SYSTEM_PROMPT`; agent-cli imports it) guarded by a re-duplication
ratchet test that scans both workspaces (incl. `src-tauri/tests`, repo-root self-check) for the
identity sentence; the prompt gained a short negative-constraints clause (workspace confinement,
no sandbox/policy bypass, no secrets in outputs — pinned by test); all 8 skills now carry a
"**Do not**" block (6 added, house style); the context-evolve↔auto-drive-tauri cargo-PATH
contradiction is eliminated both ways (both use CLAUDE.md's conditional form); the
wayland→auto-drive-tauri deflection makes that cross-ref bidirectional; and CLAUDE.md's Gotchas
distinguish `.agents/skills/` (Claude-facing) from the runtime's `.agent/skills` registry dirs.
Accepted residuals (final whole-branch review — merge-clean): ratchet is needle-based (a future
file quoting the full identity sentence in a doc comment fails intentionally); skill-lint script,
`_facts.md`, prompt-eval gate, and catalog inlining remain unbuilt (recorded spec residuals /
product decisions). **No inline finding remains open in this file.**

Re-stamp note (2026-07-02, loop-robustness cluster — backlog drain 2/6): the deep audit's
never-clustered **Orchestration** MEDs/LOWs are now **fixed and merged to `main`** (6 commits
+ fix wave, `4bb31a2..431416c`, merge `f419188`; spec
`docs/superpowers/specs/2026-07-02-loop-robustness-design.md`). (1) Approval waits race
cancellation (`tokio::select!` in `gate_tool`'s Ask arm + gate-entry short-circuit after the
ToolStart emit) — Ctrl-C no longer wedges on a pending prompt; deny-on-cancel, content
distinguishes "run cancelled" from "user declined"; sub-agent children unwedge via the shared
gate path. (2) Mid-stream retry no longer duplicates output: additive
`AgentEvent::StreamRetry {discarded_text_chars, discarded_reasoning_chars}` (wire kind
`"stream_retry"`, old-SPA-safe) emitted only when a failed attempt leaked chunks AND another
attempt follows (Retryable-with-budget + first-overflow arms; never Fatal/Cancelled/
second-overflow); CLI prints a dim retraction line, web trims the in-flight item tail
(code-point exact), traces record it, and the dispatch child-capture trims
`segments.last_mut()` so the parent model never reads abandoned partial child text.
(3) One malformed tool call no longer discards the turn: `ParsedTurn.invalid`
(`InvalidToolCall{id,name,error}`) — native protocol isolates per call; invalid calls become
per-call `ToolResult{Error}` "re-emit only this call" results while good calls execute;
assistant history carries all ids (invalid as `args {}`), `normalize_invalid_ids` keeps id
uniqueness, and the max_tokens Length guard is preserved (prompted-protocol repair path
untouched). (4) Stuck-model detection: identical call-set signature (sorted, id-independent,
valid+invalid) → one nudge user message on the 3rd consecutive identical turn (appended
AFTER the tool results — OpenAI-compat ordering), abort on the 5th with `Done(Error)` and a
text-only assistant append (no dangling `tool_calls` in persistent history);
`STUCK_NUDGE_AFTER=2`/`STUCK_ABORT_AFTER=4` consts by design. Accepted residuals (final
whole-branch review — merge-clean): stranded IPC pending-approval entry on the cancel path
(bounded, spec-sanctioned answerable-but-ignored posture); benign TOCTOU decline-labeled-
cancelled; nudge/abort strings hardcode "3"/"5" apart from the consts; stuck signature relies
on serde_json default key-sorting (false-negative-only if `preserve_order` ever lands);
coverage gaps only: no Fatal/second-overflow-with-partial absence tests, no abort-after-reset
composition test, no direct `normalize_invalid_ids` collision unit test; `Some(all_calls.clone())`
allocation nit. Retry-After/jitter remains deferred to the small-residuals sweep.

Re-stamp note (2026-07-02, calibrated-budgeting cluster — backlog drain 3/6): the deep audit's
**Spine B #2 HIGH** (token accounting vs ground truth) and **#4 MED** (stale recall blocks) are
now **fixed and merged to `main`** (2 commits, `d02b262..197378b`, merge `ae3750d`; spec
`docs/superpowers/specs/2026-07-02-server-usage-calibrated-budgeting-design.md`).
`AgentLoop.calib_ratio_micros` (AtomicU64) learns the (server `prompt_tokens` / chars-4
estimate) density ratio per completed request — EMA α=0.5, clamped [1.0, 4.0], shrink-only —
and `effective_model_limit()` applies it at the four budgeting sites (turn build, overflow
MaintCtx + rebuild build, end-of-turn MaintCtx); `Usage` events and snapshots keep the
CONFIGURED limit (display truth); backends reporting no usage behave exactly as before. The
final review verified the feedback loop is structurally stable (density ratio is
eviction-scale-invariant; non-destructive build; triple-bounded shrink) and that the ratio
persists across runs on both frontends. `ctx.set_recall` is now unconditional — an
empty retrieval clears the previous run's recall block. Overflow compact-and-rebuild is
hereby demoted from de-facto safety net back to actual last resort (the
`context-token-estimate-undercounts` memory item is addressed). Accepted residuals
(merge-clean): claude-cli `prompt_tokens` omits cache_read/cache_creation tokens →
calibration inert on that backend (queued in drain cluster 6); server shares one loop → the
ratio is cross-session there (a backend property, desirable — recorded); `debug!` inside
`fetch_update` may double-fire under CAS contention; calibration unit tests use a
constant-est recorder (mechanism-level pin; no-oscillation is argued structurally); ratio
resets on settings change / child loops / restart (converges in 1-2 turns).

Re-stamp note (2026-07-02, git --output arg-scan — backlog drain 4/6): the permissions
cluster's documented accepted residual — `git {log,diff,show} --output=<path>`/`-o` as an
auto-allowed arbitrary-file write under the default read-safe prefixes — is now **fixed and
merged to `main`** (1 commit, `789d514`, merge `669972d`; spec
`docs/superpowers/specs/2026-07-02-git-output-argscan-design.md`). `is_auto_allowed` screens
matched git `log`/`diff`/`show` invocations for exact `-o`/`--output` or `--output=…` in any
argument position → Ask, never Deny. Precision pinned by test: `--output-indicator-*`,
`git ls-files -o` (--others), `git log --oneline`, `grep -o` stay auto-allowed; abbreviation
attack ruled out by parse-opt ambiguity against the `--output-indicator-*` siblings;
adversarial review found no bypass (quoting, glued `=`, positional-arg placement, last-token
flag all covered). The `default_allowlist()` ACCEPTED RESIDUAL comment is now a CLOSED note.
Remaining out of scope: FS Write-vs-Destroy granularity (standing product decision);
non-allowlisted git write-sinks (`format-patch -o`) already fail the prefix match; user-added
bare `git` entries keep legacy semantics.

Re-stamp note (2026-07-02, eval-flywheel cluster — backlog drain 5/6): the deep audit's
**Eval-dimension** MEDs/LOW are now **fixed and merged to `main`** (5 commits, `7e8c118..3c4a88f`,
merge `7f7c601`; spec `docs/superpowers/specs/2026-07-02-eval-flywheel-design.md`).
**Process signal:** `RunResult` gained `#[serde(default)] trajectory: Vec<TrajectoryStep{tool,args}>`
(ToolStart order, captured by the eval sink across ALL sessions of a run), `denials` (SafeApproval
denials — the accumulated-never-read counter now emits to stderr as `eval-denied:` lines + a count
in the JSON), and `gold_matched: Option<bool>`; `TaskSpec.gold_trajectory` (ordered tool-name
subsequence) + `trajectory_matches_gold` comparator (empty gold vacuous; duplicate-name semantics
pinned). All additive: old JSON lines/frozen task files parse (pinned by test + verified through
the real `eval_gate` binary both directions); the promotion gate is UNTOUCHED — trajectory is
diagnostic until the campaign decides to gate on it. **Regression corpus:**
`agent-runtime-config/tests/policy_corpus.{rs,tsv}` — 86 rows through the REAL engine path
(`RulePolicy` + `ExecuteCommand::intent` + `default_allowlist()`/`effective_denylist()`, mirroring
assemble.rs wiring) covering every closed-bypass class; new cases are one-line TSV additions.
Discovery: the name-exact catastrophe-token guard drops `grep mkfs README` to Ask (fail-safe,
documented row). **Coverage:** a `continue-on-error` GitHub Actions job runs
`cargo llvm-cov --workspace --summary-only` (scripts/ci.sh untouched — pre-push stays fast; first
real validation on the first Actions run). Accepted residuals (merge-clean): trajectory records
full args (unbounded JSON line size on huge write_file args — truncate if campaign jsonl ever
bites); TSV whitespace-smuggling rows could be silently degraded by a space-collapsing editor (no
.gitattributes; in-test multi-space assertion is the cheap pin); three hand-built RunResult test
literals to touch on any future field. Session discovery queued to drain cluster 6:
`agent-cli approval::denies_when_timeout_elapses` parks a thread on real stdin — under an
open-pipe stdin, tokio teardown wedges the whole test binary (observed 874 s); make it hermetic.

Re-stamp note (2026-07-02, small-residuals sweep — **BACKLOG DRAIN COMPLETE, 6/6**): the
accumulated small accepted-residuals from all prior clusters are now **fixed and merged to
`main`** (4 commits, `9e44ed6..39932bf`, merge `92fbad6`; spec
`docs/superpowers/specs/2026-07-02-small-residuals-sweep-design.md`, items S1-S13). Trace files
created 0600 (unix); dead web `ToolCall.tsx` deleted; `use_skill` listings cap at 50/section with
an "…and N more" marker; retry backoff gained bounded jitter (+≤25%) and honors integer-seconds
`Retry-After` (capped 30 s, `ModelError::Status.retry_after`); `Evicted` dedup keys on
`(messages, est_tokens)`; claude-cli spawns without `AGENT_API_KEY` and folds
cache_read/cache_creation into `prompt_tokens` (cluster-3 calibration no longer inert on that
backend); `enforce` refusals carry the actionable copy; duplicate tool-name registration warns
(last-wins kept); memory tools' optional params are described; non-cloning `pinned_tokens()`;
the snapshot memory segment reports the capped recall block the context actually injects; a
compose-time warn fires when the static prompt exceeds a quarter of the window; and the
`denies_when_timeout_elapses` test is hermetic (open-stdin wedge: 874 s → 0.5 s, reproduced by
review). Accepted residuals (merge-clean, cosmetic): snapshot memory est is content-only
(±4 tokens vs siblings); the S12 warn's "composed" wording also covers the Err-fallback prompt;
`remember` tags/scope descriptions not contract-asserted; HTTP-date `Retry-After` ignored
(pinned); last-wins duplicate registration by design.

**2026-07 BACKLOG DRAIN — CLOSED.** All six clusters merged: 1 Instructions (`bc8934e`),
2 Loop robustness (`f419188`), 3 Calibrated budgeting (`ae3750d`), 4 git --output arg-scan
(`669972d`), 5 Eval flywheel (`7f7c601`), 6 Small-residuals sweep (`92fbad6`). Every finding,
build opportunity, and deferred-work residual from
`docs/superpowers/audits/2026-07-01-harness-deep-audit.md` is now either FIXED, explicitly
ACCEPTED-BY-DESIGN (triage list in the drain ledger / `harness-backlog-drain-2026-07` memory),
or parked as a named PRODUCT DECISION awaiting the human (git-tool consolidation, persisted
OffloadStore, example auto-injection, CandidateConfig widening, catalog inlining, post-exec
validator hook, graceful max_turns landing, max_parallel_tools→RuntimeConfig, live trace toggle,
dd-redirection parity, lexical/FS split of the read-boundary hook). The next audit run starts
from a genuinely clean slate.

Re-stamp note (2026-07-02, product-decision round — ADJUDICATED; runtime-knobs cluster 1/5
merged): the parked PRODUCT DECISIONS list was verified against live source (four Explore
agents), briefed, and adjudicated by the owner in one batched decision round. **APPROVED:**
/dev-redirection Deny parity, post-exec validator hook, max_parallel_tools→RuntimeConfig,
graceful max_turns landing, CandidateConfig widening, and the H6b measuring experiment
(experiment, not build). **DECLINED-BY-OWNER:** git-tool consolidation, persisted OffloadStore,
list_skills catalog inlining, live trace toggle, lexical/FS split of the read-boundary hook,
and the sub-agent extras (role registry / per-call model override / live child token streaming
— stay "on demand"). Ledger: `.superpowers/sdd/progress.md` (drain ledger archived alongside).
**Cluster A (runtime knobs) is MERGED to `main`** (4 commits + fix wave, `6037779..2301310`,
merge `7f05ebe`; spec `docs/superpowers/specs/2026-07-02-runtime-knobs-design.md`):
`RuntimeConfig.max_parallel_tools` (serde-defaulted to `DEFAULT_MAX_PARALLEL_TOOLS` = 8,
per-field merge, `validate()` rejects 0, `loop_config_from` passthrough replaces the hardcoded
8), and a graceful max_turns landing — ONE best-effort tools-disabled wrap-up completion at the
budget fall-through (`one_completion` direct: no retry/overflow/StreamRetry; Cancelled →
`Done(Cancelled)`; other errors swallowed; text-only append, stray calls discarded; no estimate
`Usage` event, `ServerUsage` gated on nonzero usage at `turn = max_turns`) before
`Done(BudgetExhausted)`. Dispatch children inherit it — a child hitting `subagent_max_turns`
now hands its parent a real summary (pinned by `budget_exhausted_child_wrap_up_summary_reaches_parent`).
Zero wire changes. Accepted residuals (final whole-branch review — merge-clean): wrap-up reply
bypasses `protocol.parse` (a prompted-backend fenced block would append verbatim; `prepare` skip
verified benign); wrap-up ServerUsage shares `turn = max_turns` with the last real turn (verified
harmless in stats/web/CLI consumers); a wrap-up that streams partial text then errors leaves
un-retracted display text (partial beats none); calibration skips the wrap-up sample (consistent
— no estimate exists). DISCOVERY (pre-existing, follow-up candidate): `memory: bool` has NO
`PartialRuntimeConfig` mirror/merge arm — a `"memory": false` in a partial on-disk file is
silently ignored; this cluster's Task-1 pattern is the exact fix.

Re-stamp note (2026-07-02, /dev-redirection Deny parity — decision-round cluster 2/5): item 10
(the documented `dd of=/dev/sda` Deny vs `echo x > /dev/sda` Ask asymmetry) is now **fixed and
merged to `main`** (impl + 3 adversarial fix waves, `41c30ea..ad5c2f4`, merge `8197934`; spec
`docs/superpowers/specs/2026-07-02-dev-redirect-denial-design.md`). A shared lexical `/dev` target
resolver (`resolved_dev_suffix` in `agent-policy/src/command.rs`: absolute-only, drops `/`-runs +
`.` segs, pops on `..` with leading-`..`-drops-at-root per POSIX `/..`==`/`, returns the suffix
only when strictly under `/dev`) backs two hard-floor layers — a structural per-simple-command
scan (`redirect_catastrophe_in_argv`) and a raw-string backstop (`raw_redirect_catastrophe`) for
unparseable/quote-glued forms — that Deny redirection (`>`, `>>`, `>|`, `&>`, `>&`, fd-prefixed,
split-pair) to any unsafe `/dev` target; safe sinks (null/zero/full/random/urandom/std*/tty/ptmx,
`fd/`, `shm/`) still reach Ask. The `dd of=` handler now shares the same resolver (stricter: denies
`/dev/null` too via presence). Fires from `hard_floor_violation` (CLI + server), no config/wire/tier
change. **The adversarial loop earned its keep:** each of three review passes caught a same-class
device-write bypass the prior fix missed — csh `>&` both-streams (impl), `//dev`//`/./dev`
`/`-run+`.`-seg (wave 1), leading/mid `..` into /dev (wave 2, → full lexical resolution), and the
literal-prefix dd sibling (wave 3). Regression-pinned in `command.rs` unit tests + `policy_corpus.tsv`
rows through the real engine. Accepted residuals (out-of-scope, reach Ask not Deny; documented in
the spec): variable-expansion (`>$D/sda`, `dd of=$DEV`) and cwd-relative (`cd /dev && … > sda`)
targets, non-redirect write vehicles (`tee`/`cp` to /dev), symlink-indirection into /dev, and input
redirection (`< /dev/sda` — reads are not destructive). One accepted fail-safe over-denial: `/dev/…`
named in quoted prose after a `>` (e.g. `echo "watch out > /dev/sda"`). DISCOVERY (pre-existing,
carried from Cluster A): `memory: bool` still lacks a PartialRuntimeConfig mirror.

Re-stamp note (2026-07-02, CandidateConfig widening — decision-round cluster 3/5): item 4 (widen
the eval genome beyond context knobs) is now **fixed and merged to `main`** (2 commits + fmt fix,
`b81b3a0..8ab27d5`, merge `87fdaf9`; spec
`docs/superpowers/specs/2026-07-02-candidateconfig-widening-design.md`). `CandidateConfig`
(`agent-runtime-config/src/eval/config.rs`) gains two additive serde-default `Option` axes —
`system_prompt` and `protocol` — each inherit-on-`None`, with `resolved_system_prompt(default)` /
`resolved_protocol(default)` resolvers so the logic is unit-tested outside the `#[ignore]` live
harness; `tests/eval_context.rs` consumes them (protocol default `"native"`, prompt lifted to a
byte-identical `EVAL_DEFAULT_PROMPT` const). Frozen champion-config JSON still parses (missing
fields → inherit); the promotion gate and admissibility are UNTOUCHED (prompt/protocol are new
optimizer axes, judged by outcome exactly as before). This is the enabler for the item-3 measuring
experiment (prompt/protocol variants become candidates instead of guesses). **Tool-description
variants deliberately deferred** — unlike prompt/protocol they have no override seam (descriptions
come from each `Tool::schema()` across agent-tools); a per-candidate description-override layer on
`ToolRegistry` is a tool-vocabulary build, recorded as follow-up. Accepted residual Minors
(optional): back-compat test derives the frozen shape from `favorable()` rather than a pinned JSON
literal (wouldn't catch existing-field-rename drift); no `skip_serializing_if` (emits explicit
nulls); `protocol` is a free string validated downstream at `RuntimeConfig::validate` (by design).

Re-stamp note (2026-07-02, post-execution validator hook — decision-round cluster 4/5): item 6
(the last unbuilt Guardrails/Hooks capability — no post-exec validation after edit/write/commit)
is now **built and merged to `main`** (2 tasks + test fix wave, `e536ee2..36374a6`, merge `5f41db5`;
spec `docs/superpowers/specs/2026-07-02-post-tool-validator-hook-design.md`). A config-driven
`RuntimeConfig.post_tool_validators: Vec<String>` (serde-default empty = disabled, plumbed to
`LoopConfig`) drives a **once-per-turn** validator pass in the agent loop: after Phase-3 tool-result
append and before the stuck-nudge, iff at least one `Access::Write|Destroy` call resolved
`Executed::Ok`, each configured command runs via `sh -c` through `self.config.sandbox` (a sink-free
`run_validator` mirroring `shell.rs`'s launch + concurrent-pipe-drain + timeout/cancel select;
4 KiB char-boundary-safe output cap; best-effort → a refusal/spawn-error/timeout/cancel is
`Skipped`, NEVER a run failure). Failures append ONE user message ("Post-edit validation reported
problems…") so the model fixes the break next turn; all-pass/all-skip append nothing. Each run
emits a synthetic `post_tool_validate` ToolStart/ToolResult pair on the EXISTING event frames
(old-SPA safe — no new kind; turn-unique ids; `SubagentSink` stamps `parent_id` for child runs, so
attribution is free). `ReadyCall` gained an `access` field threaded from `intent.access` (Copy).
Trusted config → NOT policy/approval-gated, but sandboxed. Whole-branch review adversarial probes
all clean (no child-leak/deadlock: Drop + `kill_on_drop` + docker-kill backstop; zero `?`/unwrap
escapes to run failure; read-only turns and failed mutations correctly excluded — the last pinned by
a `FailStub` regression test added in the fix wave). Accepted residuals (merge-clean): `Skipped`
folds into `SessionStats.tools_error` (spec-conformant, conflates skip/failure in the counter); a
validator counts as a `tool_call` (spec-intended); unbounded read buffer before the 4 KiB cap
(shell.rs parity); env allowlist means `cargo`/`npm` must be on the daemon PATH (same posture as
`execute_command`); synthetic id repeats across `run()` calls at the same turn index (reducer pairs
correctly). Deferred (spec out-of-scope): per-validator trigger filters, a subagent-scoped disable,
blocking/veto (rollback) semantics, auto-fix (mutating validators).

Re-stamp note (2026-07-02, H6b measuring experiment — decision-round cluster 5/5): item 3 was
adjudicated "run the measuring experiment first" (not build, not keep-deferred-blind). The
experiment is BUILT, RUN, and merged to `main` (2 commits, merge `25950a1`; package at
`docs/superpowers/experiments/h6b-example-triggering/`). It is a MEASUREMENT, not a feature: an
example-bearing `csv-report` skill (strict-format `examples/` exemplar), an example-NECESSARY task
(gold_trajectory `[list_skills,use_skill,read_skill_file]` + hidden grading that requires the
example's format), an example-INJECTED control task, a runner, and a decision rule. The one
mergeable code change is an additive `SKILLS_DIR` hook in `eval_context.rs` letting a run opt into
a skills catalog. **Smoke (N=3/arm, live qwen3.6-35b-a3b):** model-initiative arm — example-load
trigger 3/3 (1.00), pass 3/3 (real ~20k-token/6-turn `list_skills→use_skill→read_skill_file(example)
→write` trajectories); injected-control arm — pass 3/3 at ~13.8k tokens/4 turns with NO skill
tools. **FINDING (directional):** model-initiative example-loading does NOT under-trigger for this
model/task; H6b auto-injection is NOT warranted for correctness — its only observed value is a
~7k-token/2-turn cost optimization (load vs inject). **=> H6b stays DEFERRED**, now with evidence
on record (was "deferred pending eval evidence"). Caveats: favorable-biased smoke (single obvious
skill, short exemplar), N=3 directional, eval harness pins the ingestion cap OFF; full-N execution
belongs to the context-evolve campaign (package is ready-to-run there). Footgun surfaced: eval
`AGENT_E2E_URL` must be `http://localhost:8080` (NOT `/v1` — the client appends
`/v1/chat/completions`; `/v1` doubles → silent 404 → tokens:0/turns:0).

---

## 2026-07 PRODUCT-DECISION ROUND — CLOSED

The parked PRODUCT DECISIONS list left at the close of the 2026-07 backlog drain was verified
against live source (four Explore agents), briefed, adjudicated by the owner in one batched
decision round, and executed. Final disposition of all 12 items:

**SHIPPED (5):**
- Item 8 — `max_parallel_tools` → RuntimeConfig — merged `7f05ebe` (cluster A).
- Item 7 — graceful max_turns landing (tools-disabled wrap-up) — merged `7f05ebe` (cluster A).
- Item 10 — /dev redirection + dd Deny parity (shared lexical resolver) — merged `8197934` (cluster B).
- Item 4 — widen eval CandidateConfig (system_prompt + protocol axes) — merged `87fdaf9` (cluster C).
- Item 6 — post-execution validator hook — merged `5f41db5` (cluster D).
- Item 3 — H6b measuring experiment (run, not build) — merged `25950a1` (cluster E); FINDING:
  keep H6b deferred (model-initiative suffices; evidence on record).

**DECLINED-BY-OWNER (6) — recorded, do not re-propose without new cause:**
- Item 1 — git tool consolidation (Access is per-tool; re-does allowlist/Destroy-tier work).
- Item 2 — persisted OffloadStore (no session rehydration exists; revisit only as a durable-
  sessions feature).
- Item 5 — inline list_skills catalog into the system prompt (permanent static cost vs
  progressive-disclosure design; testable later as a prompt variant via the now-widened
  CandidateConfig).
- Item 9 — live trace toggle without restart (niche; restart cheap; RAM-only sessions).
- Item 11 — lexical/FS split of the read-boundary hook (the decision-time FS I/O is read-only and
  deliberately closes the symlink escape; split risks a security regression).
- Item 12 — sub-agent extras (role registry, per-call model override, live child token streaming) —
  stay "on demand".

Carried follow-up discoveries — **both closed 2026-07-02**:
- `memory: bool` missing from the `PartialRuntimeConfig` mirror/merge — FIXED (merged to main,
  `fix(runtime-config): mirror memory in PartialRuntimeConfig merge`). The fix also added a
  structural guard test (`full_saved_file_overrides_every_field_via_partial_merge`) that flips
  every `RuntimeConfig` field and round-trips it through the partial-merge path, so any future
  field added without a mirror fails CI (and the exhaustive struct literal makes forgetting to
  update the guard a compile error). Field-by-field audit confirmed `memory` was the only gap.
- Tool-description eval axis (ToolRegistry description-override seam) — DECLINED-FOR-NOW by
  owner (no consumer: H6b resolved without it, catalog-inlining declined, no scheduled
  experiment varies tool descriptions). Full investigated design sketch + revisit trigger
  recorded in `docs/superpowers/specs/2026-07-02-tool-description-override-seam-design.md`
  (~half-day build via a registry override map + additive RuntimeConfig thread when needed).

The next audit run starts from a clean slate. Ledger: `.superpowers/sdd/progress.md`.

Re-stamp note (2026-07-06, full harness+SDLC audit): a fresh 11-dimension audit was run — the
original seven components plus sub-agent orchestration (as a finished whole), the desktop/web
design-tab harness, the skills/OKF knowledge layer, and a process dimension (the SDLC as run by
agents, per docs/okf/agent-sdlc/comparisons/two-meanings.md). Every finding was adversarially
verified before inclusion (44 raised → 41 confirmed: 18 med, 23 low, zero high; no prior-fix
regression found anywhere). **The current findings snapshot is
`docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md`** — it supersedes both this section's
inline list and the 2026-07-01 report. Spec:
`docs/superpowers/specs/2026-07-06-full-harness-sdlc-audit-design.md`.

Re-stamp note (2026-07-07, audit-drain cluster 1/6 — CI & gates; merge `33cb6fa`; triage spec
`docs/superpowers/specs/2026-07-07-audit-drain-action-plan-design.md`, plan
`docs/superpowers/plans/2026-07-07-audit-ci-gates.md`): four findings of the 2026-07-06 report
closed. **Process 11.1** — ci.sh runs a conditional src-tauri leg (clippy -D warnings + test when
pkg-config finds gtk+-3.0 + webkit2gtk-4.1, explicit SKIPPED line otherwise; GitHub-runner
rationale preserved, fmt stays hand-format-excluded). **Skills 9.2 / eval 10.1** — the OKF bundle
is gated: `test_okf_check.py` + `okf_check.py docs/okf/agent-sdlc` run as ci.sh's first
(fail-fast) leg. **Eval 10.2** — the coverage job tees the llvm-cov summary and appends
Filename/TOTAL to `$GITHUB_STEP_SUMMARY` (still continue-on-error, never a gate; confirm the
Summary tab on first push). **Skills 9.3** — okf_check gained checks 5-8 (`resource:` on Source
nodes, type vocabulary, `[n]`-marker resolution, directory-index coverage; suite 9→13 tests),
live bundle conformant with zero fixes, and the semantic-drift human duty is documented in the
checker docstring + authoring.md. Whole-branch review: READY TO MERGE, zero Critical/Important;
accepted minors in the cluster ledger (`.superpowers/sdd/progress.md`).

Re-stamp note (2026-07-07, audit-drain cluster 2/6 — MCP seam; merge `0ac6f4a`; spec
`docs/superpowers/specs/2026-07-07-mcp-seam-design.md`, plan
`docs/superpowers/plans/2026-07-07-audit-mcp-seam.md`): three findings closed — the audit's
headline boundary. **Guardrails 5.1** — new `Access::TrustedWrite` tier: MCP `Trust::Allow`
intents are auto-allowed at the gate exactly as before (Read-arm semantics, pinned both
directions) but now count as mutations for the post-exec validator trigger, pinned end-to-end
by a `TrustedStub` loop test. **Tools 2.1** — MCP schemas get a connect-time contract lint
(empty descriptions + undescribed required params via the shared
`required_params_missing_description`), warn-don't-reject, surfaced as
`ServerStatus.schema_warnings` + the mcp summary line. **Sandbox 3.1** — MCP env secrets left
docker argv: name-only `-e KEY` with values on the docker client process env; client-control
keys (`DOCKER_HOST`/`DOCKER_CONFIG`/`DOCKER_CERT_PATH`/`DOCKER_TLS_VERIFY`/`DOCKER_CONTEXT`/
`HOME`/`PATH`) deliberately stay argv-literal (non-secret) so mcp.json env cannot redirect the
docker CLI's daemon/auth/binary discovery — a final-review finding fixed and re-verified
pre-merge; note the deliberate spec deviation: spec-set HOME rides argv, not name-only.
Whole-branch review: READY TO MERGE; accepted minors + two process incidents (subagent commits
on local main; disk-full ld SIGBUS) in the cluster ledger.

Re-stamp note (2026-07-07, audit-drain cluster 3/6 — design-tab hardening; merge `686d889`;
plan `docs/superpowers/plans/2026-07-07-audit-design-tab-hardening.md`, straight-to-plan per
the triage spec): six findings closed. **Design-tab 8.1** — `DevServerManager::start`
canonicalizes the SPA-sent dir and requires containment in the current workspace (root
threaded from `bridge.current_workspace()`), rejecting before any teardown or spawn;
validate-before-stop preserved and pinned (traversal, nonexistent dir, live-server-survives-
rejection). **8.4** — `parse_url`'s prefix match replaced by `is_local_authority` (authority
up to the first `/?#`, `@` refused, exact `localhost`/`127.0.0.1`, numeric-only port) —
strict Rust first-line parity restored; the JS WHATWG guard stays authoritative at render.
**8.6** — `stop()` SIGTERMs the group, waits up to `STOP_GRACE` (1500 ms) on the direct child
with the mutex already released, then group-SIGKILLs as a true backstop; graceful-marker and
SIGTERM-ignoring tests. **8.2** — `PR_SET_PDEATHSIG(SIGTERM)` via `pre_exec`
(`cfg(target_os = "linux")`) so the dev server dies with a SIGKILLed/aborted app;
PR_GET_PDEATHSIG probe test. **8.3** — desktop CSP gains
`frame-src 'self' http://localhost:* http://127.0.0.1:*` with a conf-parsing regression test,
and the production CSP was verified LIVE on a bundled-asset build (`tauri/custom-protocol`):
a remote iframe tripped a `frame-src` securitypolicyviolation while a localhost iframe loaded
clean — both clauses of the finding closed. **8.5** — ArtifactRenderer's Image branch blocks
non-local http(s) srcs (`isLocalUrl`; `data:`/localhost only) with a visible notice; 4-case
vitest. Whole-branch review: READY TO MERGE, zero Critical/Important; accepted residuals in
the cluster ledger — notable: desktop `img-src` deliberately NOT widened to match 8.5's SPA
policy (localhost http images stay CSP-blocked on desktop, fail-closed asymmetry), the
containment canonicalize→spawn TOCTOU is accepted in-threat-model (requires workspace write,
which already grants in-workspace script execution), and `start()`'s discovery-timeout path
keeps its graceless SIGKILL by design.

Re-stamp note (2026-07-07, audit-drain cluster 4/6 — sub-agent composition; merge `1c984f5`;
spec `docs/superpowers/specs/2026-07-07-audit-subagent-composition-design.md`, plan
`docs/superpowers/plans/2026-07-07-audit-subagent-composition.md`): five findings closed —
the seams between July's individually-reviewed sub-agent clusters. **4.1** —
`DispatchDeps.description_overrides` threads `cfg.tool_description_overrides` into every
child registry (`set_description_overrides` after all child registrations; nested deps clone
carries it to grandchildren), restoring the override seam's parent/child uniformity claim;
pinned by a schema-capturing child-request test. **4.2** — `ModelRef` gains additive
`context_limit`/`max_tokens` (inherit-on-None, ≥1024 validate floor on both routed refs);
a routed subagent model's limits override the child `LoopConfig` clone, and a routed
compaction model's window caps the maintenance target via
`maint_model_limit() = min(effective, compaction window)` at exactly the three `MaintCtx`
sites — build/request sizing untouched, inert unless configured. **4.3** — the budget
wrap-up prompt is injected into the request messages only; durable history keeps just the
assistant summary (two-run pin + failed-wrap-up pin + wire-level request-capture guard).
Final-review fix `db7de6a`: the build now reserves `built_tokens`-denominated headroom for
the injected prompt so the single no-recovery wrap-up request cannot overshoot the window
(RED 449>400 → GREEN pin). **4.4** — timeout and fatal-failure dispatch arms return the
child's captured partial transcript (`failure_output`: loud note + transcript + honest
footer; no `unwrap_or(Stop)` — recorded stop or the failure kind; owner-adjudicated Ok
posture, parent-cancel stays Err). Discovery: a fatally-failed child always records
`Done(Error)` first, so the footer says `stop: Error` and the `failed` fallback is defensive
dead code. **2.3≡4.5** — `dispatch_agent`'s description is depth-computed at construction
("minus dispatch_agent itself" only at the depth floor; nested variant matches the
tools-param prose), floor text byte-identical. Whole-branch review: READY TO MERGE — all five
cross-task seams verified clean (override uniformity via the schemas()-only prose path,
mirrored budget postures, sink still captures wrap-up tokens, clone-after-loop_config
ordering); accepted residuals in the cluster ledger — notable: per-dispatch warn noise when
an override names a tool absent from a child registry (e.g. a `dispatch_agent` override at
`max_depth=1`), `ModelRef.max_tokens` inert for `compaction_model` (doc-scope), duplicated
footer format string across success/failure paths, no end-to-end child-maintains-at-min()
test (indirect pin chain accepted).

---

## Top highest-leverage fixes

Ranked by impact (severity × remediation cost). All prior HIGH findings, the observability
cluster (per-call terminal events + durations, JSONL session traces, usage/cost parsing,
SessionStats + web panel, ContextEvent forwarding, CI gate), the sandbox cluster
(fail-closed degraded exec, env scrub, required `LoopConfig.sandbox`, MCP workspace cwd,
nobody uid fallback), the context cluster (turn-atomic eviction/compaction + visible
eviction), the tools cluster (16 KiB ingestion cap + eager offload, recall/read_file
pagination), the retry cluster (classified retries, overflow compact-and-rebuild,
full Done parity), the permissions cluster (Access::Destroy tier, token-prefix
subcommand-aware allowlist, memory re-tiering), and the instructions cluster
(single-source ratchet-guarded prompt + negative constraints) are **done** — for
the full current backlog see `docs/superpowers/audits/2026-07-01-harness-deep-audit.md`
(its Top-10 table; **all ten items are now complete — the Top-10 is closed**).
**No inline finding remains open.** The 2026-07 residual-backlog drain is in
progress (see `.superpowers/sdd/progress.md` triage: orchestration robustness,
context budgeting, git --output arg-scan, eval flywheel, small-residuals sweep).
