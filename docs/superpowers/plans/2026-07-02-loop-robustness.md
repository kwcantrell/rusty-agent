# Loop Robustness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Four turn-loop robustness fixes: cancellable approval waits, stream-retry retraction, per-call malformed-tool-call isolation, repeated-identical-call detection.

**Architecture:** All changes live in `agent-core/src/loop_.rs` + `agent-model` (`protocol.rs`, `types.rs`), with one additive event flowing through `event.rs` → `agent-server/src/wire.rs` → CLI `render.rs` → web `state.ts`. No breaking wire changes (old-SPA rule: additive frames only).

**Tech Stack:** Rust (workspace `agent/`), React/TS (`web/`), vitest.

**Spec:** `docs/superpowers/specs/2026-07-02-loop-robustness-design.md` — read the matching section before each task; it is the authority on behavior.

## Global Constraints

- Two Cargo workspaces; run cargo from `agent/` (`source ~/.cargo/env` if missing).
- Conventional commits `type(scope): summary`.
- Wire changes must be ADDITIVE only (new frame kind `stream_retry`; no field/kind renames). Old SPA must keep parsing.
- Tests use the existing testkit `Scripted` model conventions in loop_.rs tests; async timing tests run on tokio paused clocks (`#[tokio::test(start_paused = true)]`) — no real sleeps.
- Error-content strings fed to the model start with `"ERROR:"` (existing convention).
- `bash scripts/ci.sh` green at cluster end.

---

### Task 1: `ParsedTurn.invalid` + per-call-tolerant `NativeProtocol::parse`

**Files:**
- Modify: `agent/crates/agent-model/src/types.rs` (ParsedTurn struct; find `pub struct ParsedTurn`)
- Modify: `agent/crates/agent-model/src/protocol.rs:18-38` (parse impl) and its tests
- Check-only: `agent/crates/agent-model/src/prompted.rs` — its two `Ok(ParsedTurn {…})` literals need `..Default::default()` or `invalid: vec![]` added to keep compiling; make the minimal edit that matches file style.

**Interfaces:**
- Produces: `pub struct InvalidToolCall { pub id: String, pub name: String, pub error: String }` (types.rs, next to ParsedTurn); `ParsedTurn { pub text, pub tool_calls, pub invalid: Vec<InvalidToolCall> }`. Task 2 consumes `parsed.invalid`.

- [ ] **Step 1: Write failing tests** in protocol.rs tests mod — replace `native_rejects_malformed_args` with:

```rust
#[test]
fn native_isolates_malformed_args_per_call() {
    let turn = AssistantTurn {
        text: "".into(),
        raw_tool_calls: vec![
            RawToolCall { index: None, id: Some("c1".into()), name: Some("good".into()),
                          args_fragment: r#"{"a":1}"#.into() },
            RawToolCall { index: None, id: Some("c2".into()), name: Some("bad".into()),
                          args_fragment: "{not json".into() },
        ],
        stop: StopReason::ToolCalls,
        reasoning: String::new(),
        ..Default::default()
    };
    let parsed = NativeProtocol.parse(&turn).unwrap();
    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(parsed.tool_calls[0].name, "good");
    assert_eq!(parsed.invalid.len(), 1);
    assert_eq!(parsed.invalid[0].id, "c2");
    assert_eq!(parsed.invalid[0].name, "bad");
    assert!(parsed.invalid[0].error.contains("bad args"));
}

#[test]
fn native_isolates_missing_name_per_call() {
    let turn = AssistantTurn {
        text: "".into(),
        raw_tool_calls: vec![RawToolCall {
            index: None, id: None, name: None, args_fragment: "{}".into() }],
        stop: StopReason::ToolCalls,
        reasoning: String::new(),
        ..Default::default()
    };
    let parsed = NativeProtocol.parse(&turn).unwrap();
    assert!(parsed.tool_calls.is_empty());
    assert_eq!(parsed.invalid.len(), 1);
    assert_eq!(parsed.invalid[0].id, "call_0");
    assert_eq!(parsed.invalid[0].name, "unknown");
    assert!(parsed.invalid[0].error.contains("missing name"));
}
```

- [ ] **Step 2: Run to verify failure** — `cd agent && cargo test -p agent-model native_isolates` → FAIL (no `invalid` field).

- [ ] **Step 3: Implement.** In types.rs add (doc-commented, `#[derive(Debug, Clone, Default)]` matching ParsedTurn's derives):

```rust
/// A tool call the protocol could not parse; the loop answers it with a
/// per-call error result instead of discarding the turn (spec 2026-07-02).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InvalidToolCall {
    pub id: String,
    pub name: String,
    pub error: String,
}
```

Add `pub invalid: Vec<InvalidToolCall>` to ParsedTurn (and to its `Default`/constructors if hand-written). Export `InvalidToolCall` from lib.rs alongside `ParsedTurn`.

Rewrite the native parse loop body (protocol.rs:20-33):

```rust
let mut tool_calls = Vec::new();
let mut invalid = Vec::new();
for (i, rc) in raw.raw_tool_calls.iter().enumerate() {
    let id = rc.id.clone().unwrap_or_else(|| format!("call_{i}"));
    let Some(name) = rc.name.clone() else {
        invalid.push(InvalidToolCall {
            id, name: "unknown".into(),
            error: format!("tool call {i} missing name"),
        });
        continue;
    };
    let args: serde_json::Value = if rc.args_fragment.trim().is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_str(&rc.args_fragment) {
            Ok(v) => v,
            Err(e) => {
                invalid.push(InvalidToolCall {
                    id, name,
                    error: format!("tool call {i} bad args: {e}"),
                });
                continue;
            }
        }
    };
    tool_calls.push(ToolCall { id, name, args });
}
Ok(ParsedTurn { text: raw.text.clone(), tool_calls, invalid })
```

Fix prompted.rs literals (`invalid: vec![]`). Grep for other `ParsedTurn {` literals across both crates (loop_.rs tests, testkit) and fix them the same way.

- [ ] **Step 4: Verify** — `cargo test -p agent-model && cargo build -p agent-core` → model tests PASS; core may still compile (field unused yet).

- [ ] **Step 5: Commit** — `feat(model): ParsedTurn.invalid — per-call malformed-tool-call isolation in NativeProtocol`

---

### Task 2: Loop consumes invalid calls

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` — parse consumption (~406-491, the `parsed` match through Phase 1) and `normalize_tool_call_ids` (find its def); tests at file bottom.

**Interfaces:**
- Consumes: `parsed.invalid: Vec<agent_model::InvalidToolCall>` from Task 1.

- [ ] **Step 1: Failing test** (loop_.rs tests, follow the existing Scripted-model test idiom — copy the setup of the nearest test that scripts tool_calls, e.g. the parallel-dispatch ones):

```rust
/// One malformed call must not discard the turn: good calls execute, the bad
/// one gets a per-call ERROR result, and the assistant message keeps all ids.
#[tokio::test]
async fn malformed_call_isolated_good_calls_execute() {
    // Scripted turn 1: raw_tool_calls = [good echo call (valid json),
    //   bad call (args_fragment "{not json", id "c_bad")], turn 2: plain text stop.
    // Assert: echo tool actually ran (its recorder); sink saw ToolStart for BOTH ids;
    // sink saw ToolResult{id:"c_bad", status: Error} with content containing
    // "could not be parsed" and "re-emit only this call";
    // ctx history's assistant message tool_calls contains both ids;
    // run ends Done(Stop) — NOT the protocol-repair user message
    //   (assert no ctx user message containing "Re-emit it correctly").
}
```

Also a Length-guard test: same bad call but scripted `stop: StopReason::Length` → sink sees the max_tokens Error + `Done(StopReason::Length)`, no tool executed.

- [ ] **Step 2: Run to verify failure** — `cargo test -p agent-core malformed_call` → FAIL.

- [ ] **Step 3: Implement** in run_with_cancel:

(a) The Length arm's guard (loop_.rs:416) becomes reachable for Ok-with-invalid too. Restructure right after the parse match: keep the existing `Err` arms untouched (prompted path), and add after `parsed` is obtained:

```rust
// A call truncated by max_tokens must take the truncation path, not a
// per-call "re-emit" error that would truncate again (spec §3).
if !parsed.invalid.is_empty() && assistant.stop == StopReason::Length {
    self.sink.emit(AgentEvent::Error(/* same message as the Err(_) Length arm */));
    self.sink.emit(AgentEvent::Done(StopReason::Length));
    return Ok(());
}
```

(deduplicate the message string with the existing arm via a local `const LENGTH_TRUNCATION_MSG: &str`).

(b) Extend `normalize_tool_call_ids` (or add a sibling pass) so invalid ids participate in uniqueness: simplest is to normalize after merging — build `let mut all_ids: HashSet<&str>` from valid calls, then for each invalid entry whose id collides or is empty, rewrite to `format!("{}_inv{}", id, k)`. Keep the existing function's contract for valid calls unchanged.

(c) Assistant message: where `Message::assistant(parsed.text, Some(tool_calls))` is built (~445), append the invalid calls first:

```rust
let mut all_calls = parsed.tool_calls.clone();
all_calls.extend(parsed.invalid.iter().map(|inv| ToolCall {
    id: inv.id.clone(), name: inv.name.clone(), args: serde_json::json!({}),
}));
```

and use `all_calls` for the message (None only when BOTH lists are empty). The `parsed.tool_calls.is_empty()` early-Done check (~461) must become `all_calls.is_empty()`.

(d) Phase 1: before the gating loop over `parsed.tool_calls`, seed `order`/`results` with the invalid entries:

```rust
for inv in &parsed.invalid {
    self.sink.emit(AgentEvent::ToolStart {
        id: inv.id.clone(), name: inv.name.clone(),
        args: serde_json::json!({}), parent_id: None,
    });
    order.push(inv.id.clone());
    results.insert(inv.id.clone(), (inv.name.clone(), Resolved::Err {
        status: ToolStatus::Error,
        content: format!(
            "ERROR: this tool call could not be parsed ({}); the other calls in \
             this turn ran normally — re-emit only this call, with valid JSON \
             arguments", inv.error),
        duration_ms: 0,
    }));
}
```

(Match the actual `Resolved::Err` field shape at loop_.rs:478-482.)

- [ ] **Step 4: Verify** — `cargo test -p agent-core` full crate → PASS (existing repair-path tests must still pass — they script the prompted protocol or Err-returning paths).

- [ ] **Step 5: Commit** — `feat(core): per-call tool-call parse isolation — one bad call no longer discards the turn`

---

### Task 3: Cancellable approval wait

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs:631-703` (gate_tool); tests at bottom.

- [ ] **Step 1: Failing test:**

```rust
/// Ctrl-C during a pending approval prompt must end the run promptly as
/// Cancelled — not wedge until the prompt is answered (audit Component 4).
#[tokio::test(start_paused = true)]
async fn cancel_during_pending_approval_ends_run() {
    // Approval channel: a stub whose request() never resolves (std::future::pending).
    // Policy: a stub returning Decision::Ask for everything.
    // Scripted turn 1: one valid tool call.
    // Spawn run_with_cancel; after the Approval event is observed on the sink,
    // cancel the token; await the run handle with a short tokio::time::timeout.
    // Assert: run returned Ok, sink ends with Done(Cancelled), and the tool's
    // ToolResult has status Denied with content containing "run cancelled".
}
```

- [ ] **Step 2: Run to verify failure** — it hangs/timeouts today; assert accordingly (`cargo test -p agent-core cancel_during_pending_approval`).

- [ ] **Step 3: Implement** per spec §1: gate-entry short-circuit at the top of gate_tool:

```rust
if cancel.is_cancelled() {
    return GateOutcome::Rejected {
        id: call.id, name: call.name,
        content: format!("ERROR: {}", ToolError::Denied("run cancelled".into())),
    };
}
```

(keep it AFTER the ToolStart emit so every call still gets its start/terminal pair — i.e. place the check immediately after the `ToolStart` emit at 632-637); and the Ask arm await becomes:

```rust
self.sink.emit(AgentEvent::Approval(req.clone()));
tokio::select! {
    _ = cancel.cancelled() => false,
    resp = self.approval.request(req) => matches!(
        resp, ApprovalResponse::Approve | ApprovalResponse::ApproveAlways),
}
```

The `!allowed` rejection content must distinguish: track `let cancelled_mid_prompt = cancel.is_cancelled();` after the select and use `"run cancelled"` vs `"user declined"` in the Denied message.

- [ ] **Step 4: Verify** — `cargo test -p agent-core cancel_ && cargo test -p agent-core gate` → PASS, plus full `cargo test -p agent-core`.

- [ ] **Step 5: Commit** — `fix(core): race approval waits against cancellation — Ctrl-C no longer wedges on a pending prompt`

---

### Task 4: StreamRetry retraction event, end to end

**Files:**
- Modify: `agent/crates/agent-core/src/event.rs` (AgentEvent enum, after `SandboxDegraded`-style variants)
- Modify: `agent/crates/agent-core/src/loop_.rs` (one_completion signature + emission sites, spec §2)
- Modify: `agent/crates/agent-server/src/wire.rs` (ServerEvent variant + `server_event_from` arm + unit test)
- Modify: `agent/crates/agent-cli/src/render.rs` (render line)
- Modify: `web/src/state.ts` (+ its wire type decl file if separate — grep `sandbox_degraded` in web/src to find where frame types live) and a vitest.
- Check: `agent/crates/agent-runtime-config/src/trace.rs` — ObservedSink records all AgentEvents generically or per-variant; if per-variant, add the arm.

**Interfaces:**
- Produces: `AgentEvent::StreamRetry { discarded_text_chars: usize, discarded_reasoning_chars: usize }`; wire frame `{"type":"stream_retry","discarded_text_chars":N,"discarded_reasoning_chars":M}` (match the file's existing serde tagging exactly — read a neighboring variant first).

- [ ] **Step 1: Failing core test:**

```rust
/// A mid-stream failure that already emitted chunks must retract them before
/// the retry re-streams (spec §2); a clean or chunk-less failure must not.
#[tokio::test(start_paused = true)]
async fn stream_retry_retracts_partial_output() {
    // Scripted: attempt 1 emits 2 text chunks ("ab","cd") then fails Retryable;
    // attempt 2 succeeds with text "xy".
    // Assert sink order contains: Token(ab), Token(cd),
    //   StreamRetry{discarded_text_chars:4, discarded_reasoning_chars:0},
    //   Token(xy), Done(..). And max_retries >= 1 in the LoopConfig.
}

#[tokio::test(start_paused = true)]
async fn no_stream_retry_when_nothing_emitted() { /* fail before first chunk → no StreamRetry event anywhere in sink */ }
```

(Extend testkit `Scripted` if it can't script "chunks then error" — look at how `Scripted::Fail(ModelError)` was added and mirror: e.g. `Scripted::ChunksThenFail(Vec<Chunk>, ModelError)`.)

- [ ] **Step 2: Run to verify failure** — `cargo test -p agent-core stream_retry` → FAIL (variant missing).

- [ ] **Step 3: Implement:**

event.rs (doc comment per spec §2):

```rust
/// A model stream died mid-answer and another attempt follows: the
/// in-flight partial text/reasoning of this turn is abandoned — frontends
/// should discard those trailing chars before the retry re-streams.
StreamRetry {
    discarded_text_chars: usize,
    discarded_reasoning_chars: usize,
},
```

loop_.rs: `one_completion` gains `emitted: &mut (usize, usize)`; inside, after each Token emit `emitted.0 += t.chars().count();`, after each Reasoning emit `emitted.1 += r.chars().count();`. `completion_with_retry` per attempt: `let mut emitted = (0usize, 0usize);` passed in; in the Retryable arm (after the budget check passes, before the backoff sleep) and in run_with_cancel's FIRST Overflow arm (before `request_compaction`) — one_completion's emitted tuple must reach there: change `RetryFailure::Overflow(String)` to carry it OR simpler: hoist the StreamRetry emission INTO completion_with_retry for both Retryable-with-another-attempt and Overflow (overflow always defers to a turn-level rebuild attempt — but the SECOND overflow is fatal; at completion_with_retry level you can't know. So: for Overflow, do NOT emit there; instead return the tuple in `RetryFailure::Overflow(String, (usize, usize))` and let run_with_cancel's first-overflow arm emit iff nonzero; the second-overflow arm emits nothing.) Emit only `if emitted != (0, 0)`:

```rust
self.sink.emit(AgentEvent::StreamRetry {
    discarded_text_chars: emitted.0,
    discarded_reasoning_chars: emitted.1,
});
```

wire.rs: additive `ServerEvent::StreamRetry { discarded_text_chars: usize, discarded_reasoning_chars: usize }` with serde tag `stream_retry` (copy the tagging style of `SandboxDegraded`), map in `server_event_from`, add a unit test asserting the JSON `type` is `"stream_retry"`.

render.rs: on StreamRetry print `\n[stream interrupted — retrying; partial output above is discarded]\n` (match the file's existing styling helpers).

web/src/state.ts: case `"stream_retry"`: trim the trailing `discarded_text_chars` chars from the last text item and `discarded_reasoning_chars` from the last reasoning item (tokens only ever extend the LAST item per the comment at state.ts:217 — trimming the tail is exact; remove the item if it becomes empty). Vitest: reduce [token "ab", token "cd", stream_retry{4,0}, token "xy"] → transcript text is exactly "xy".

trace.rs: confirm ObservedSink is variant-generic (it serializes AgentEvent) — if it matches per-variant, add the arm so traces record retractions.

- [ ] **Step 4: Verify** — `cargo test -p agent-core stream_retry && cargo test -p agent-server && cargo test -p agent-cli && cd ../web && npm test -- --run state && cd ../agent` → PASS.

- [ ] **Step 5: Commit** — `feat(core): StreamRetry retraction event — mid-stream retry no longer duplicates partial output (additive wire frame)`

---

### Task 5: Repeated-identical-call detection

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (run_with_cancel turn loop + two consts near DEFAULT_MAX_PARALLEL_TOOLS); tests.

- [ ] **Step 1: Failing tests:**

```rust
/// The 3rd consecutive identical call-set gets a nudge; the 5th aborts the
/// run without executing (spec §4) — a stuck model burns 4 turns, not 25.
#[tokio::test]
async fn stuck_identical_calls_nudged_then_aborted() {
    // Scripted: 6 turns, each the same single tool call (same name+args).
    // Tool: a counter recorder.
    // Assert: tool executed exactly 4 times (turns 1-4; turn 5 aborted);
    // ctx contains exactly one user message containing "identical tool call";
    // sink ends Error(..contains "5 turns in a row") then Done(StopReason::Error).
}

#[tokio::test]
async fn stuck_counter_resets_on_different_call() { /* A A B A A A A A… pattern: differing turn resets; no abort within max_turns=8; assert tool ran 8 times and run ended by budget */ }
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p agent-core stuck_` → FAIL.

- [ ] **Step 3: Implement** per spec §4. Consts:

```rust
/// Nudge after this many consecutive REPEATS of an identical call set
/// (i.e. on the 3rd identical turn); abort after STUCK_ABORT_AFTER (the 5th).
/// Not configurable until a real workload needs the knob (spec 2026-07-02 §4).
pub const STUCK_NUDGE_AFTER: usize = 2;
pub const STUCK_ABORT_AFTER: usize = 4;
```

In run_with_cancel before the turn loop: `let mut last_sig: Option<String> = None; let mut repeats = 0usize; let mut nudged = false;`. After parse + normalize, when `!all_calls.is_empty()`:

```rust
let mut parts: Vec<String> = parsed.tool_calls.iter()
    .map(|c| format!("{}\u{1}{}", c.name, c.args))
    .chain(parsed.invalid.iter().map(|i| format!("{}\u{1}{}", i.name, i.error)))
    .collect();
parts.sort();
let sig = parts.join("\u{2}");
if last_sig.as_deref() == Some(&sig) { repeats += 1; } else { repeats = 0; nudged = false; }
last_sig = Some(sig);
```

**Message-ordering constraints (both matter for OpenAI-compat history validity):**

(a) **Abort runs BEFORE the assistant message is appended.** An aborted turn must
not leave a dangling assistant `tool_calls` message with no tool results in
persistent history (contexts survive across runs — the next run would 400).
On abort, append the turn as text only, then terminate:

```rust
if repeats >= STUCK_ABORT_AFTER {
    ctx.append(Message::assistant(parsed.text.clone(), None)); // no tool_calls — nothing will answer them
    self.sink.emit(AgentEvent::Error(
        "model repeated the identical tool call(s) 5 turns in a row; aborting the run".into()));
    self.sink.emit(AgentEvent::Done(StopReason::Error));
    return Ok(());
}
```

So: compute the signature right after `normalize_tool_call_ids`, run the abort
check, and only then fall through to the existing assistant-message append.

(b) **The nudge is appended AFTER the turn's tool results land** — a user message
between an assistant `tool_calls` message and its `Role::Tool` results is invalid
for OpenAI-compat servers. Set `let mut nudge_pending = false;` at the check
site:

```rust
if repeats >= STUCK_NUDGE_AFTER && !nudged {
    nudged = true;
    nudge_pending = true;
}
```

and after the Phase-3 drain finishes appending the turn's tool-result messages
to `ctx`, add:

```rust
if nudge_pending {
    ctx.append(Message::user(
        "You have now issued the identical tool call(s) 3 turns in a row; \
         repeating them will not change the result. Change your approach, or \
         reply with a summary and no tool call if you are done.".into()));
}
```

Note `c.args` identity: use `serde_json::to_string` (same model output → same string).
Update the Step-1 test asserts to match: the nudge user message appears after the
turn-3 tool results; the aborted turn's assistant message carries no tool_calls.

- [ ] **Step 4: Verify** — `cargo test -p agent-core stuck_ && cargo test -p agent-core` → PASS.

- [ ] **Step 5: Commit** — `feat(core): repeated-identical-call detection — nudge at 3, abort at 5 consecutive identical turns`

---

### Task 6: Cluster gate

- [ ] **Step 1:** `bash scripts/ci.sh` → all green. No commit expected.
