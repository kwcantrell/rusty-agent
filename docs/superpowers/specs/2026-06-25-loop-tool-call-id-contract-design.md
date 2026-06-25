# B1 — Collision-Proof Tool-Call Result Contract — Design

**Date:** 2026-06-25
**Status:** Approved (brainstorming) → ready for plan
**Source:** Cluster B (loop robustness) of the security/robustness audit backlog
(`2026-06-24-security-audit-backlog.md`). First of three sequenced sub-specs;
B2 (OpenAI stream robustness) and B3 (live cancellation wiring) follow in their
own cycles. Findings re-verified against current `main` on 2026-06-25.

## Principle

Enforce the tool-call id invariant — *every tool call in a turn has a unique,
non-empty id* — at the single point where it originates for **all** protocols
(right after `protocol.parse()`), so neither the panic nor a malformed
duplicate-id transcript is reachable. Then make the Phase-3 drain structurally
panic-proof as defense-in-depth, so no future protocol change can reintroduce a
model-controllable crash.

## Finding 1 — Panic on duplicate/empty tool-call ids (HIGH)

**Where:** `agent/crates/agent-core/src/loop_.rs:203-252`.

**Bug:** Phase 1 builds `order: Vec<String>` with one entry per call
(`loop_.rs:204`, pushed at `:210`/`:214`) and `results: HashMap<String, ...>`
keyed by id (`:205`, inserted at `:211`/`:236`). When two calls in a turn share
an id, `results.insert` collapses them to one map entry while `order` still has
two — so Phase 3's `results.remove(&id).expect("every gated call id has a
result")` (`:241-242`) returns `None` on the second occurrence and **panics**.

Ids are model-controllable:
- `agent/crates/agent-model/src/protocol.rs:29` (`NativeProtocol`, production):
  `let id = rc.id.clone().unwrap_or_else(|| format!("call_{i}"));` — the
  positional fallback is unique only when ids are *absent*; **model-supplied
  duplicate ids pass through verbatim with no dedup**.
- `agent/crates/agent-core/src/testkit.rs:83` (`PassthroughProtocol`, test):
  id-less calls default to the literal `"c"`, so 2+ id-less calls collide.

Even when the collision does not panic, two tool messages get appended with the
same `tool_call_id` (`loop_.rs:251`), which is a malformed transcript for the
model. So the id invariant is wrong at the source, not just at the drain.

**Severity:** High — a remote/untrusted model can crash the agent process by
emitting two tool calls with the same (or empty) id.

### Fix — Component 1: normalize ids at the loop chokepoint

The appended assistant message (`loop_.rs:188-189`,
`Some(parsed.tool_calls.clone())`) and the Phase-3 tool messages (`:251`) both
draw their ids from the same `parsed.tool_calls`. Normalizing ids once, right
after parse and before either consumer, keeps the whole transcript internally
consistent: the model only ever sees our normalized ids, never its originals,
so tool-call/result correlation stays correct.

Add a free function in `loop_.rs`:

```rust
fn normalize_tool_call_ids(calls: &mut [ToolCall]) {
    let mut seen = std::collections::HashSet::new();
    for (i, c) in calls.iter_mut().enumerate() {
        if c.id.is_empty() || !seen.insert(c.id.clone()) {
            // empty or duplicate -> deterministic, order-stable, collision-checked
            let mut candidate = format!("call_{i}");
            let mut n = 1;
            while !seen.insert(candidate.clone()) {
                candidate = format!("call_{i}_{n}");
                n += 1;
            }
            c.id = candidate;
        }
    }
}
```

Apply it in `run()` immediately after `parsed` is obtained (currently line 186,
before the assistant message is built at 188). `parsed` becomes `let mut parsed`:

```rust
let mut parsed = match self.protocol.parse(&assistant) { ... };
normalize_tool_call_ids(&mut parsed.tool_calls);
```

Properties:
- **Order-preserving** — Phase 3 still emits tool messages in the model's call
  order.
- **Deterministic** — same input → same ids (no clock/random).
- **Collision-checked** — handles the edge where a model legitimately sends
  `call_0` *and* an id-less call: the `while` loop bumps the synthetic id
  (`call_0_1`, …) until free.
- **Pre-existing unique ids are untouched** — only empty or duplicate ids are
  rewritten.
- **Protocol-agnostic** — one chokepoint covers `NativeProtocol`, the testkit
  `PassthroughProtocol`, the prompted protocol, and any future protocol, so
  `protocol.rs` needs no change.

### Fix — Component 2: panic-proof Phase-3 drain

Replace the panicking drain (`loop_.rs:240-242`) with a graceful one:

```rust
for id in order {
    let (name, resolved) = match results.remove(&id) {
        Some(v) => v,
        None => continue, // invariant already guarantees presence; never crash if it's ever violated
    };
    // ... unchanged: emit ToolResult, append Message::tool(id, name, content)
}
```

With Component 1 the `None` arm is unreachable, but this converts "model can
crash the agent process" into "worst case, one tool result is silently dropped"
— the correct failure posture for untrusted input, and a guard against future
regressions in the id contract.

## Finding 4 — `message_tokens` undercount (LOW) — folded in

**Where:** `agent/crates/agent-core/src/context.rs:9-11`.

**Bug:** `message_tokens` counts only `m.content` (plus a flat `+4`), ignoring
the `Message` fields `reasoning: Option<String>` and
`tool_calls: Option<Vec<ToolCall>>` (`agent/crates/agent-model/src/types.rs`).
Assistant turns with large tool-call argument JSON or preserved reasoning are
undercounted, so the window-eviction budget in `build()` and the `Usage` events
underestimate the real token load. Folded into this spec because it is the same
crate and pure correctness.

### Fix — Component 3

```rust
fn message_tokens(m: &Message) -> usize {
    let mut t = estimate_tokens(&m.content) + 4; // per-message overhead
    if let Some(r) = &m.reasoning {
        t += estimate_tokens(r);
    }
    if let Some(calls) = &m.tool_calls {
        for c in calls {
            t += estimate_tokens(&c.name) + estimate_tokens(&c.args.to_string());
        }
    }
    t
}
```

`ToolCall` is `{ id: String, name: String, args: serde_json::Value }`;
`c.args.to_string()` serializes the args JSON for estimation. `estimate_tokens`
already takes `&str`.

## Error handling

No new error types. Normalization is infallible; the drain degrades to a skip;
the token estimate is a heuristic. The only behavior changes are: id-less or
duplicate tool-call ids now get unique ids, and token estimates increase for
messages carrying tool-call args or reasoning.

## Testing (TDD — write the failing test first)

**`normalize_tool_call_ids` unit tests (`loop_.rs`):**
- Input ids `["", "", "x", "x"]` → all outputs unique and non-empty, order
  preserved, `len` unchanged, and the first `"x"` (already unique when reached)
  stays `"x"` while the duplicate second `"x"` is rewritten.
- Collision edge: input `["", "call_0"]` (an id-less first call **and** a model
  supplying the literal `"call_0"`) → still yields two distinct ids (the `while`
  loop bumps the synthetic id rather than producing two `call_0`s). The assertion
  is on uniqueness, not on preserving any particular literal.

**Loop regression — the crash proof (`loop_.rs` tests / testkit):** a scripted
assistant turn with two tool calls that collide to the same id (via the testkit
`PassthroughProtocol` `"c"` default, or a scripted model emitting two id-less
calls) → `run()` completes **without panicking** and appends two tool messages
with **distinct** ids. This test panics on current code (proves the bug).

**`message_tokens` test (`context.rs`):** a `Message` carrying `tool_calls`
**and** `reasoning` estimates strictly more tokens than the same message with
only `content`.

## Scope

**In scope:** `agent-core/src/loop_.rs` (normalization helper + chokepoint call +
panic-proof drain) and `agent-core/src/context.rs` (`message_tokens`), plus tests.

**Out of scope (explicit non-goals):**
- `agent-model/src/protocol.rs` — the chokepoint approach makes per-protocol
  changes unnecessary.
- **B2** — OpenAI stream robustness (truncated-stream signal, malformed-SSE-line
  handling, in-band 200-body error). Separate spec.
- **B3** — live cancellation wiring (Ctrl-C/SIGINT source threaded into
  `ToolCtx`). Separate spec.

## Alternatives considered (where to enforce uniqueness)

1. **Normalize at the loop chokepoint — CHOSEN.** One protocol-agnostic place,
   covers every current and future protocol, keeps the assistant message and
   tool results consistent because both consume the same normalized list.
2. **Uniquify inside each protocol's `parse()`.** Correct but must be
   re-implemented (and re-tested) in every protocol; a new protocol that forgets
   reintroduces the panic.
3. **Only make the Phase-3 drain positional/panic-proof, leave ids alone.**
   Fixes the crash but still appends two tool messages with a duplicate
   `tool_call_id` — a malformed transcript. Component 2 keeps this as
   defense-in-depth, but it is not sufficient as the sole fix.
