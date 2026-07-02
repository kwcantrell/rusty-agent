# Small-residuals sweep — design

**Date:** 2026-07-02
**Status:** Approved (autonomous backlog-drain run)
**Cluster:** 6 of 6 in the 2026-07 residual-backlog drain — the accumulated small
accepted-residuals from prior clusters' final reviews, batched. Each item is
independently small; the spec fixes intent and edge decisions, the plan groups them
by crate.

## Items

### S1 — Trace file perms 0600 (`agent-runtime-config/src/trace.rs:~40`)
Session JSONL traces hold full conversation content but are created with umask
defaults. Open with `OpenOptions` + `std::os::unix::fs::OpenOptionsExt::mode(0o600)`
(unix-gated `#[cfg(unix)]`; non-unix keeps current behavior). Existing files keep
their perms (no chmod pass — new-file posture only). Test: create a trace, assert
`metadata.permissions().mode() & 0o777 == 0o600` (unix).

### S2 — Delete dead `web/src/components/ToolCall.tsx`
Zero imports (verified). Delete the file; `npm run typecheck` + vitest confirm
nothing referenced it.

### S3 — "…and N more" truncation for L2 skill listings (`agent-skills/src/tools.rs`)
`use_skill`'s bundled-files and examples sections list every file — unbounded for
filesystem-authored skills. Cap each section at `MAX_LISTED_FILES = 50` entries,
then append `- …and {n} more (read_skill_file lists any path under the skill dir)`.
`list_skills` stays uncapped (one line per skill is already bounded by skill count).
Tests: 51-file skill lists 50 + the marker; ≤50 lists all, no marker.

### S4 — Backoff jitter + Retry-After (`agent-core/src/loop_.rs`, `agent-model`)
- `ModelError::Status` gains `retry_after: Option<u64>` (seconds; parsed from the
  `Retry-After` response header in openai.rs's non-2xx path — integer-seconds form
  only; HTTP-date form is ignored). All construction/match sites updated (`..` where
  the field is irrelevant); `class()` unchanged (429/408 stay Retryable).
- `completion_with_retry`'s Retryable arm sleeps
  `max(jittered_backoff(attempt), Duration::from_secs(retry_after.min(30)))` when the
  error carried Retry-After; otherwise `jittered_backoff(attempt)`.
- `jittered_backoff(attempt) = backoff_delay(attempt) + uniform(0..=delay/4)`.
  Randomness source: `rand`-free — hash of `Instant::now()` nanos is fine (advisory).
- The paused-clock backoff-growth pin changes from exact 700 ms to the bound
  `[700 ms, 875 ms]` (three retries, each +≤25%). New test: a scripted
  `Status{429, retry_after: Some(3)}` sleeps ≥3 s virtual before the retry.

### S5 — Evicted dedup key → `(messages, est_tokens)` (`agent-core/src/curated.rs`)
`last_evicted: usize` becomes `last_evicted: (usize, usize)`; re-emit when either
count or token estimate changes. Extend the existing
`maintain_emits_evicted_once_per_change` test with a same-count/different-tokens case.

### S6 — claude_cli.rs: drop the runtime key, fold cache tokens
- `Command::new(&self.binary)…` gains `.env_remove("AGENT_API_KEY")` — the CLI uses
  its own subscription auth; the runtime's key is the one secret it must not inherit
  (closes the last child that saw it; full env otherwise stays — trusted backend).
- Usage parsing: `prompt_tokens = input_tokens + cache_read_input_tokens +
  cache_creation_input_tokens` (missing fields default 0) — makes prompt_tokens the
  effective context size, so cluster 3's calibration is no longer inert on this
  backend and the display is faithful. `cached_tokens` keeps reporting
  `cache_read_input_tokens` separately. Test: parse a fixture "result" event with
  cache fields and assert the fold.

### S7 — `enforce` refusal gets actionable copy (`agent-sandbox/src/strategy.rs:~151`)
Mirror the auto-mode message: `"docker unreachable ({reason}); command refused —
start Docker, or set sandbox_mode=\"off\" to accept unsandboxed execution
(sandbox_mode=\"enforce\" never degrades)"`. Test: assert the enforce error contains
"start Docker".

### S8 — `ToolRegistry::register` duplicate-name warn (`agent-tools/src/registry.rs`)
`insert` return value inspected; on overwrite emit
`tracing::warn!(tool = %name, "duplicate tool name registered — last wins")`.
Last-wins behavior is kept (rejecting could break startup on MCP collisions);
the warn makes silent shadowing visible. MCP cross-server duplicates surface via the
same warn at registration. Test: register two tools with one name; assert the second
wins (behavior pin) — the warn itself is best-effort observability (no log-capture
harness needed).

### S9 — Memory tool optional-param descriptions (`agent-memory/src/tools.rs`)
`forget`'s `id` and `query`, `recall`'s `k` (and any other bare `{"type": …}` optional
param in remember/recall/forget) gain one-sentence `description`s. Extend the existing
recall contract test to assert non-empty descriptions for these named params.

### S10 — Non-cloning `pinned_tokens()` helper (`agent-core/src/curated.rs`)
The two `self.pinned().iter().map(message_tokens).sum()` sites clone every pinned
message to count tokens. Add a private `fn pinned_tokens(&self) -> usize` that
computes over references (restructure `pinned()`'s parts without cloning — e.g. an
iterator over the would-be pinned messages), and use it at both sites. Pure
refactor: property tests must stay green unchanged.

### S11 — Snapshot memory segment uses the capped recall block (`agent-core/src/snapshot.rs`)
The "memory" segment sums all recall lines; the context actually injects
`recall_block(recall, budget)` (512-token default cap). Compute the segment's
`est_tokens`/`items`/`count` from the SAME capped block the context injects (expose
or reuse `recall_block`), so the dashboard never over-reports memory pressure.
Test: recall lines exceeding the budget → segment est ≤ budget-ish (the block's own
estimate), not the raw sum.

### S12 — Static system-prompt budget warn (`agent-runtime-config/src/assemble.rs`)
After `compose_system_prompt` succeeds, if `estimate_tokens(&prompt) >
context_limit / 4`, emit `tracing::warn!` naming the estimate, the limit, and the
preset list. No behavior change; test optional (log-free assert via a small pure
helper `prompt_over_budget(est, limit) -> bool` if wanted).

### S13 — Hermetic `denies_when_timeout_elapses` (`agent-cli/src/approval.rs`)
The test constructs `TerminalApproval::new(1ms)` whose blocking thread parks on REAL
process stdin; under an open-pipe stdin, tokio runtime teardown joins that thread and
wedges the whole test binary (observed 874 s this session). Fix in the TEST layer:
use the existing `with_prompt` injection (the same seam `concurrent_requests_serialize`
uses) with a prompt fn that sleeps past the timeout — preserving the assertion
(timeout ⇒ Deny) without touching process stdin. If the timeout arm can't be reached
via `with_prompt` (read the impl first), add a test-only constructor that injects the
blocking reader. Production stdin path untouched (its orphan-thread caveat remains
documented).

## Out of scope (recorded residuals)

- HTTP-date `Retry-After` form (integer seconds only).
- Rejecting duplicate tool names (warn + last-wins keeps startup robust).
- chmod-ing pre-existing trace files.
- The production `TerminalApproval` stdin orphan-thread design itself (documented;
  only the test's dependence on real stdin is removed).
- Windows perms for traces (unix-gated).

## Testing

Per-item tests named above; the whole-workspace `bash scripts/ci.sh` (run with
stdin closed until S13 lands) gates the cluster.
