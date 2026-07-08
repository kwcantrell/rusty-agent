# Audit Drain Cluster 5 — Context, Trace & Config Guards (Design)

**Date:** 2026-07-07
**Input:** findings 7.1, 7.2, 6.1, 6.2, 7.3, 3.3, 5.2, 5.3 of
`docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md`, as triaged by
`docs/superpowers/specs/2026-07-07-audit-drain-action-plan-design.md` (Cluster 5).
**Branch:** `feature/audit-context-config-guards` (worktree off `main`).
**Owner decisions (2026-07-07, batched round):** 6.1 → RunStart carries the **full
composed system prompt text every run** (no dedup); 7.1 → **512-token** goal cap.

Line numbers are from live source at design time and drift; the plan re-opens live
source before acting. One finding changed state since the audit ran: **3.3 is already
closed** (see §6).

## 1. Finding 7.1 — cap the pinned goal block (med)

`CuratedContext::set_goal` (curated.rs:206-210) pins the FULL first user input,
set-once for the session. Every other pinned block is budgeted (recall 512,
`FOLDED_FACTS_MAX_TOKENS` 512, system quarter-window warn); an over-window first
paste makes `pinned_tokens()` exceed the window forever — build() budget saturates
to 0, all shrink paths touch history only, and second-overflow-in-a-turn is fatal —
permanently wedging the persistent server/REPL context.

**Fix.**

- New const in curated.rs: `const GOAL_MAX_TOKENS: usize = 512;` (owner-picked; same
  scale as `DEFAULT_RECALL_TOKEN_BUDGET` and `FOLDED_FACTS_MAX_TOKENS`). A const, not
  a config knob: `set_goal` has no view of `model_limit`, and no tuning demand exists.
- `set_goal` truncates: if `estimate_tokens(&goal) > GOAL_MAX_TOKENS`, keep the first
  `GOAL_MAX_TOKENS * 4` chars (char-boundary safe — `chars().take(n)`, matching the
  `chars/4` estimator) and append the marker
  `… [goal truncated; the full input remains in the conversation history]`.
- The marker's own tokens ride on top of the cap (bounded, ~15 tokens) — simpler than
  reserving headroom, and the cap's purpose is order-of-magnitude, not exactness.
- The full input still goes to history via the existing `ctx.append` (loop_.rs:474);
  fold/offload/eviction manage it there. No loop change.

**Champion-spine guard (mandatory, post-merge):** the goal block is the champion-v4
attention spine (the folded-facts ledger merges INTO it, curated.rs:91-105). After
merge, re-run the context-evolve paired guard sweep with the champion config
(`champion_k10.json`); ceilings are validated as (config, rate) pairs per the
2026-07-03 protocol. Expected no-op — eval task prompts are well under 512 tokens —
but the sweep is the evidence, not the expectation.

**Pins.**

- Over-cap first input → goal block ≤ cap + marker tokens; marker present;
  under-cap input unchanged (no marker).
- Wedge regression pin: with a small `model_limit`, an over-window first paste still
  yields `pinned_tokens() < model_limit` and a non-empty build budget (the "first
  paste does not permanently wedge the session" property, asserted structurally).
- Set-once semantics unchanged (existing `set_goal_is_set_once` keeps passing).

## 2. Finding 7.2 — `max_tool_result_bytes` floor (med)

`validate()` (runtime_config.rs:355+) guards max_tokens/max_turns/max_parallel_tools/
context_limit but not `max_tool_result_bytes`. A user writing `0` ("disable the cap")
gets the opposite: `select_oversized` selects every non-empty result, `capped_preview`
degrades all output to marker-only stubs, and recall pages at ~1 char/call — the model
never sees tool output again.

**Fix.**

- `validate()` rejects `max_tool_result_bytes < 1024` with
  `"max_tool_result_bytes must be >= 1024"` — reject, not clamp, matching every
  sibling guard (and the `context_limit >= 1024` floor style). Reaches all frontends:
  CLI (main.rs:224 gate), server (runtime.rs validate), settings apply.
- The opposite direction (cap far above the window silently re-opening the
  single-oversized-result overflow path) becomes a **warn**, not a reject, at the
  existing compose-time warn site in assemble.rs (next to the quarter-window system
  warn): `tracing::warn!` when `max_tool_result_bytes / 4 > context_limit / 4`
  (i.e. the cap's estimated tokens exceed a quarter of the window).

**Pins.** validate() rejects 0 and 1023, accepts 1024 and the default; partial-file
merge of a bad value still fails validate at the consuming edge (existing apply path
test extended or sibling test added); warn fires for cap ≥ window/4 tokens (assert via
the assemble warn seam if cheaply reachable — else the warn is code-reviewed only,
matching the quarter-window warn's own posture).

## 3. Finding 6.1 — run inputs in the trace (med)

The user input goes straight to context (`ctx.append`, loop_.rs:474) and never through
the sink; TraceEvent has no input/system-prompt record. A failed top-level turn cannot
be replayed from the trace alone, and traces cannot be harvested into eval datasets
(the child gap was closed by the dispatch ToolStart args; the parent is the one
unreplayable actor). The server recomposes the system prompt per turn
(session.rs:116), so it is otherwise unrecoverable.

**Fix — one emit site, all frontends.**

- New variant `AgentEvent::RunStart { input: String, system: Option<String> }`,
  emitted once at the top of `run_with_cancel` (after the SandboxDegraded emit,
  before `set_goal`). `input` is the full, un-truncated user input (independent of
  the 7.1 pin cap). `system` is the full composed system prompt text — **every run,
  no dedup** (owner call: ~4-8KB/run against the 64MB cap; dedup state isn't worth
  it).
- The loop reads the system prompt via a new default-`None` getter on
  `ContextManager`: `fn system(&self) -> Option<&Message> { None }`.
  `CuratedContext` and `WindowContext` override it; test stubs compile untouched.
- Wire compat: `server_event_from` maps `RunStart` to `None` (the Approval(_)
  precedent) — the old SPA never sees a new frame.
- Trace: new `TraceEvent::RunStart { input: &'a str, system: Option<&'a str> }`
  serialized like every other record (type-tagged snake_case).
- `SubagentSink` (dispatch.rs): child runs also emit RunStart; forward it to the
  child trace tap like other non-captured events (parent_id join as usual). The child
  input duplicates the dispatch ToolStart args by design — the trace record makes
  child rows self-describing; do not capture it into the parent transcript.
- SessionStats: RunStart folds as a no-op (not a tool call, not tokens).

**Pins.**

- ObservedSink/TraceWriter test: a run writes a `run_start` record carrying the exact
  input and system text as its first non-degraded record.
- server_event_from(RunStart) is None (wire-compat pin).
- Child dispatch: the child's trace lines include its own run_start with `parent_id`
  set (extend an existing child-trace test).

## 4. Finding 6.2 — terminal `trace_disabled` marker (low)

On cap breach or write failure (trace.rs:106-123) the JSONL just stops — a replayer
cannot distinguish "trace capped/disabled" from "process died mid-turn".

**Fix.**

- Reserve fixed headroom in the cap check: breach when
  `written + line + 1 > max_bytes - TRACE_DISABLED_HEADROOM` (const, 256 bytes).
- Before `inner.w = None` on either path, best-effort write one marker record built
  directly in trace.rs (it is not an AgentEvent):
  `{"seq":N,"ts_ms":…,"event":{"type":"trace_disabled","reason":"cap"|"io_error"}}`
  followed by a flush. On the io_error path the marker write may itself fail —
  accepted (best-effort; the reason we can't guarantee it is the reason it exists).
- The marker write bypasses the cap check (its headroom is pre-reserved) and must not
  recurse.

**Pins.** Cap-breach test (tiny max_bytes): file ends with a `trace_disabled`
reason=cap record and no partial line; subsequent records are dropped silently.
Marker fits within reserved headroom (assert marker line length < headroom).

## 5. Finding 7.3 — snapshot gains the ledger segment (low)

`snapshot()` (curated.rs:178-189) passes only the bare goal into `build_snapshot`;
the folded-facts ledger — injected via `pinned()` and charged in `pinned_tokens()` —
is invisible to the Context Explorer: up to 512 tokens missing from `est_total` and
every segment.

**Fix.**

- `build_snapshot` gains a ledger parameter (the pre-built `Option<Message>` ledger
  block plus the fact lines for items); `CuratedContext::snapshot` passes
  `(!self.folded_facts.is_empty()).then(|| self.folded_block())` and the lines.
- New segment `category: "ledger"` emitted after `goal`, with
  `est_tokens = message_tokens(folded_block)`, `items` = the numbered fact lines
  (previewed at 100 chars, recall-style), `count` = fact count.
- A **separate segment**, not folded into goal: `pinned_tokens()` (curated.rs:153-168)
  already counts goal and ledger as separate messages, so separate segments make
  `est_total` match the budget math exactly (folding into goal would mismatch by the
  per-message overhead) and give the explorer fact-line visibility for free.
- Web explorer renders unknown categories generically (verify while testing; if the
  Context tab hard-codes categories, add "ledger" to its palette/order — display-only
  change).

**Pins.** Snapshot with folded facts: ledger segment present, est matches
`message_tokens(folded_block())`, est_total includes it (compare to `pinned_tokens()`
+ history); without facts: no ledger segment (existing shape tests keep passing).

## 6. Finding 3.3 — CLI sandbox_mode validation (low) — ALREADY CLOSED

Since the audit ran, the claude-cli-followups branch (merged 2026-07-07) added the
CLI validate gate: `rt.validate()` in agent-cli main (main.rs:224, exit 2), and
`validate()` rejects unknown `sandbox_mode` (runtime_config.rs:432-435). A typo'd
`--sandbox-mode enfore` now exits 2 before `build_sandbox` — exactly the finding's
first proposed option.

**Remaining work (test-only pin).** Add `cli_bad_sandbox_mode_fails_validate` beside
the existing `cli_bad_claude_effort_fails_validate` (main.rs tests): CLI-assembled
config with `sandbox_mode: "enfore"` fails validate. Record the independent closure
in the audit.md re-stamp.

## 7. Finding 5.2 — allowlist interpreter warn (low)

command.rs's KNOWN LIMITATION prose (:391-394) forbids adding shell interpreters /
exec vehicles to `command_allowlist`, but nothing enforces it: adding `"bash"`
silently auto-allows `bash -c "sudo …"`. Owner call at triage: **warn**, not reject.

**Fix.**

- New `pub fn warnings(&self) -> Vec<String>` on `RuntimeConfig` (validate() stays
  reject-only). For each `command_allowlist` entry, take the leading whitespace-token,
  basename it (so `/bin/bash` matches), and flag membership in
  `["bash","sh","zsh","dash","ksh","eval","xargs","env"]` with a message naming the
  entry and pointing at the KNOWN LIMITATION rationale (auto-allowed wrapper can run
  anything).
- Surfacing: agent-cli main prints each as `warning: …` to stderr right after the
  validate gate; the server logs `tracing::warn!` at its validate sites (runtime.rs
  startup and the settings apply path). One fn, three call sites, no wire frames.

**Pins.** Unit tests on `warnings()`: `"bash"`, `"/bin/bash"`, `"xargs -0"` warn;
`"git status"`, `"cargo build"`, default allowlist produce zero warnings (the default
config stays warning-free — guard against warn fatigue).

## 8. Finding 5.3 — corpus rows for documented /dev residuals (low)

The corpus pins the /dev deny classes but not the 2026-07-02 dev-redirect spec's
documented Ask-not-Deny residuals; an allowlist or classifier change could silently
flip them to Allow with no test failing.

**Fix.** Five `ask` rows in `policy_corpus.tsv`, in the /dev class block, each with a
comment naming the residual class:

```
ask	tee /dev/sda	# non-redirect write vehicle (documented residual: reaches Ask, not Deny)
ask	cp /tmp/x /dev/sda	# non-redirect write vehicle
ask	echo x > $DEV/sda	# variable-expansion target (SHELL_SIGNIFICANT `$`)
ask	dd of=$DEV	# variable-expansion target in dd
ask	cd /dev && echo x > sda	# cwd-relative redirect
```

Verify each against the live engine before committing the row (the corpus runner is
the test; a row that fails means the posture already drifted — investigate, don't
edit the expectation).

## Execution shape

Six tasks (7.1, 7.2, 6.1, 6.2, 7.3, then the small trio 3.3+5.2+5.3 as one task),
TDD each, per-task subagent review, whole-branch review, `bash scripts/ci.sh` green,
`--no-ff` merge. Policy items ride `policy_corpus.tsv`/corpus runner per triage
process rule 4. Post-merge: context-evolve paired guard sweep (§1), audit.md
re-stamps, ledger + memory updates.

**Sub-agent hygiene (recurring gotchas — bake into every brief):** worktree branches
from stale origin/main (reset --hard to local main tip first); require
`git rev-parse --show-toplevel` == worktree path before EVERY commit; git
reset/rebase forbidden; `cargo fmt --all --check` in every implementer verification.

## Out of scope

- Making the goal cap configurable; goal-block re-wording beyond the marker.
- Trace-record dedup/compression; persisted traces beyond the existing cap/retention.
- Rejecting (vs warning on) allowlist interpreters; any command.rs scanner change.
- The declined July items and refuted findings (triage spec Out of scope).
