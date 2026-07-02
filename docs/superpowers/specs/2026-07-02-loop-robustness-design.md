# Loop robustness — approval-cancel race, stream-retry retraction, per-call parse isolation, stuck-call detection

**Date:** 2026-07-02
**Status:** Approved (autonomous backlog-drain run; brief fixed by the 2026-07-01 deep
audit's Component 4 MEDs/LOWs)
**Cluster:** 2 of 6 in the 2026-07 residual-backlog drain (see `.superpowers/sdd/progress.md`)

## Problem

Four verified-open robustness gaps in the turn loop (`agent-core/src/loop_.rs`,
`agent-model/src/protocol.rs`):

1. **Approval waits are uncancellable.** `gate_tool`'s Ask arm awaits
   `self.approval.request(req)` (loop_.rs:692) with no race against the cancellation
   token. Ctrl-C wedges until the prompt is answered. Sub-agent children share the gate
   path and the parent's approval channel, so they inherit the wedge (and a dispatch
   wall-clock timeout can strand a child mid-approval — the documented stale-IPC-prompt
   residual).
2. **Mid-stream failure + retry duplicates output.** `one_completion` emits
   `Token`/`Reasoning` chunks as they arrive; when the stream dies mid-way,
   `completion_with_retry` re-opens a fresh stream and re-emits from the start — the
   user sees partial text then a full duplicate. Same on the turn-level
   overflow-recovery rebuild.
3. **One malformed tool call discards the whole turn.** `NativeProtocol::parse` returns
   `Err` on the first call with a missing name or bad-JSON args (protocol.rs:24,29),
   so N−1 good calls are thrown away and a whole-turn "re-emit it correctly" repair
   runs (loop_.rs:426-432) — the good work is redone at full cost.
4. **No stuck-model detection.** A model emitting the identical call set every turn
   burns all `max_turns` (default 25 in prod configs); nothing nudges or aborts.

Non-goal ruled out during triage: `LoopConfig::Default` hand-impl — already done
(documented test-only contract, loop_.rs:72-98).

## Approaches considered

- **Fix 2 alternatives:** (a) buffer tokens until stream completion — kills streaming
  UX for every turn to serve a rare failure; rejected. (b) suppress emission on retry
  attempts — the partial from attempt 1 is already on screen, duplication merely moves;
  rejected. (c — **chosen**) a retraction event: tell frontends to discard the
  in-flight partial before the retry re-streams. Old SPAs ignore unknown frame kinds
  (verified precedent: `session_stats`, `context` frames were added additively), so the
  new frame degrades to today's behavior (duplicate text), never breaks parsing.
- **Fix 3 alternatives:** (a) make `parse` return per-call `Result`s — breaks every
  `ToolCallProtocol` impl and caller; rejected. (b — **chosen**) additive
  `ParsedTurn.invalid: Vec<InvalidToolCall>` (`#[derive(Default)]`-friendly, prompted
  protocol never fills it); `Err` remains for whole-turn structural failures (prompted
  fenced-block cases), preserving the existing repair path.
- **Fix 4 alternatives:** compare call sets only (chosen) vs call sets + results —
  result comparison adds surface for marginal precision; a model re-issuing the
  byte-identical call set for 5 consecutive turns is burn regardless of output.

## Design

### 1. Cancellable approval wait (`gate_tool`)

- At gate entry: if `cancel.is_cancelled()`, return `Rejected` immediately with
  `"ERROR: denied: run cancelled"` (short-circuits the rest of a Phase-1 batch after
  a cancel).
- The Ask arm's await becomes:

  ```rust
  let approved = tokio::select! {
      _ = cancel.cancelled() => false,
      resp = self.approval.request(req) => matches!(
          resp, ApprovalResponse::Approve | ApprovalResponse::ApproveAlways),
  };
  ```

  Cancel-during-prompt = deny. The rejected result's content distinguishes
  `"run cancelled"` from `"user declined"`.
- Downstream is already correct: Phase-2 tools receive the cancelled token (cancel-aware
  tools abort; the rest are fast), and the next turn's top-of-loop check emits
  `Done(Cancelled)`. No event/wire changes.
- Children: the fix lives in the shared gate path; a parent cancel propagates into the
  child token (dispatch already wires that), so child approval waits unwedge too. A
  frontend prompt already rendered stays answerable-but-ignored (same bounded posture
  as the dispatch-timeout residual; note, don't fix, the unattributed-prompt part).

### 2. Stream-retry retraction event

- New core event (additive): `AgentEvent::StreamRetry { discarded_text_chars: usize,
  discarded_reasoning_chars: usize }` — "the in-flight assistant text/reasoning of the
  current turn is abandoned; a fresh attempt follows."
- `one_completion` gains an out-param `emitted: &mut (usize, usize)` (chars of text /
  reasoning emitted this attempt; reset by the caller per attempt). On any error return
  the caller knows what leaked to the sink.
- Emission sites (exactly the paths where **another attempt follows a partial**):
  - `completion_with_retry`, Retryable arm, before the backoff sleep — iff
    `emitted != (0, 0)`.
  - `run_with_cancel`, first `Overflow` arm (the once-per-turn rebuild) — iff the
    failed attempt emitted chunks.
  - NOT on Fatal/Cancelled/second-overflow (no further attempt; partial stays, the
    terminal event explains).
- Wire (additive frame, old-SPA-safe): `ServerEvent::StreamRetry { discarded_text_chars,
  discarded_reasoning_chars }`, kind `"stream_retry"`; forwarded by `server_event_from`;
  trace records it via the existing ObservedSink path.
- Frontends: CLI prints `\n[stream interrupted — retrying; partial output above is
  discarded]`; web reducer clears the current in-flight assistant text/reasoning
  buffers (id-first correlation untouched). Old SPA: unknown kind ignored → behavior
  identical to today.

### 3. Per-call malformed-call isolation

- `ParsedTurn` (agent-model) gains `pub invalid: Vec<InvalidToolCall>` where
  `InvalidToolCall { id: String, name: String, error: String }` (`name` may be
  `"unknown"`). Prompted protocol never fills it; its structural failures keep
  returning `Err` (repair path unchanged).
- `NativeProtocol::parse` never fails per-call anymore: missing name → invalid entry
  (`name: "unknown"`, id from `rc.id` or `call_{i}`); bad-JSON args → invalid entry
  with the serde error. Good calls parse as before. `Err` is no longer reachable from
  the native impl (kept in the signature for the prompted path).
- Loop consumption (after `normalize_tool_call_ids`, which now also normalizes/dedups
  invalid ids against valid ones):
  - Each invalid call joins the assistant message's `tool_calls` as
    `ToolCall { id, name, args: json!({}) }` — history stays id-coherent (every tool
    message keeps a parent call).
  - Each invalid call emits `ToolStart` (args `{}`) + joins `order`/`results` as
    `Resolved::Err { status: ToolStatus::Error, content: format!("ERROR: this tool
    call could not be parsed ({error}); the other calls in this turn ran normally —
    re-emit only this call, with valid JSON arguments"), duration_ms: 0 }` → the
    Phase-3 drain emits its terminal `ToolResult` like any other failure.
  - **Length guard preserved:** if `assistant.stop == StopReason::Length` and
    `parsed.invalid` is non-empty, take the existing max_tokens-truncation path
    (loop_.rs:416-425) instead — a truncated call must not be answered with a
    "re-emit" tool error that re-truncates.
  - A turn with only invalid calls still round-trips them as tool errors (no repair
    message needed — the per-call errors are the repair prompt).
- `protocol_repairs` counter and the `Err` arms stay, now exercised only by the
  prompted protocol.

### 4. Repeated-identical-call detection

- In `run_with_cancel`: after a successful parse with non-empty `tool_calls`, compute
  the turn's signature = sorted `Vec<(name, canonical_json(args))>` (invalid calls
  included by id-less signature `(name, error)`). Track
  `(last_signature, consecutive_repeats)`.
- Identical to previous turn → `consecutive_repeats += 1`; else reset to 0.
- At `consecutive_repeats == STUCK_NUDGE_AFTER (= 2`, i.e. the 3rd identical turn`)`:
  append one user message before executing: `"You have now issued the identical tool
  call(s) 3 turns in a row; repeating them will not change the result. Change your
  approach, or reply with a summary and no tool call if you are done."` (appended
  once per stuck episode; the calls still execute).
- At `consecutive_repeats == STUCK_ABORT_AFTER (= 4`, i.e. the 5th identical turn`)`:
  emit `AgentEvent::Error("model repeated the identical tool call(s) 5 turns in a
  row; aborting the run")` + `Done(StopReason::Error)` and return `Ok(())` — same
  terminal shape as protocol-repair exhaustion. The calls of the aborting turn are
  NOT executed.
- Constants `pub const STUCK_NUDGE_AFTER: usize = 2` / `STUCK_ABORT_AFTER: usize = 4`
  in loop_.rs (not config — YAGNI; documented in the doc comment). Sub-agent children
  inherit (same loop). No event/wire additions.

## Error handling

- StreamRetry emission must never fire without a following attempt (checked by
  emission-site placement, pinned by tests).
- Invalid-call content strings start with `"ERROR:"` matching the existing gate/exec
  error convention (selectors and offload previews already handle that shape).
- Cancel-during-approval must resolve the pending `ApprovalRequest` on the CLI channel
  gracefully: `TerminalApproval`'s stdin read is already timeout-bounded and its
  orphan-thread caveat is documented; no change there (the select! simply stops
  waiting).

## Testing

(Testkit `Scripted` model + existing unit-test conventions in loop_.rs / protocol.rs.)

1. Approval race: a `Scripted` turn hits Ask with an approval channel that never
   answers; cancel the token after dispatch → run ends `Done(Cancelled)` promptly
   (paused-clock, no real waits); rejected result content says cancelled. Child-loop
   variant covered by the shared path (no separate dispatch test needed).
2. StreamRetry: scripted stream that emits N text chunks then a Retryable error, then
   a clean attempt → exactly one `StreamRetry` with the right char counts, tokens of
   the clean attempt not duplicated after it; no `StreamRetry` when the failed attempt
   emitted nothing, on Fatal, or on the second overflow.
3. Native per-call isolation: turn with [good, bad-args, good] → 3 ToolStart, bad one
   gets `ToolResult{status: Error}` + both good ones execute; assistant message carries
   all 3 ids; `parse` returns `Ok`. Missing-name variant. Length-stop variant takes the
   max_tokens path. Prompted-protocol repair path still works (existing tests).
4. Stuck detection: scripted model repeating one call → nudge message appended at turn
   3 (assert via ctx messages), abort at turn 5 with `Done(Error)` and no execution of
   turn-5 calls; a differing turn resets the counter; zero-call turns don't count.
5. Wire: `server_event_from(StreamRetry{..})` maps to kind `"stream_retry"` (unit,
   wire.rs); web reducer clears in-flight buffers on `stream_retry` (vitest).

## Out of scope (recorded residuals)

- Retry-After / backoff jitter (cluster 6).
- Unattributed child approval prompts on the wire (sub-agent v1 residual, unchanged).
- A `StopReason::Stuck` wire variant — reusing `Error` + message keeps the wire
  surface fixed; revisit only if a frontend needs to distinguish.
- Configurable stuck thresholds (constants until someone needs the knob).
- Buffered/resumable streaming (server-side dedup of re-streamed prefixes).
