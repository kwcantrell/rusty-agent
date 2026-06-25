# B2 — OpenAI Stream Robustness — Design

**Date:** 2026-06-25
**Status:** Approved (brainstorming) → ready for plan
**Source:** Cluster B (loop robustness) of the security/robustness audit backlog
(`2026-06-24-security-audit-backlog.md`). Second of three sub-specs; B1
(tool-call id contract) is merged (`6383a7b`); B3 (live cancellation wiring)
follows. Findings re-verified against current `main` on 2026-06-25.

## Principle

The streaming client is the only layer that knows whether a stream actually
completed or was cut off, and whether a 200-status body carried an error.
Surface both conditions as proper `ModelError`s — which `completion_with_retry`
already retries — while tolerating a single corrupt SSE line rather than
aborting an entire generation over it.

All changes live in `agent/crates/agent-model/src/openai.rs`
(`parse_sse_line` and the `stream!` loop in `OpenAiCompatClient::stream`).

## Data-flow context (why the fix belongs in the client)

The loop consumer `one_completion` (`agent/crates/agent-core/src/loop_.rs:91-108`)
defaults `let mut stop = StopReason::Stop;` and only overrides it on
`Chunk::Done` (`:104`). So a truncated stream that never yields a `Chunk::Done`
is indistinguishable downstream from a clean stop. Only `openai.rs` knows whether
a terminal marker (`[DONE]` or a `finish_reason` delta) was seen before the bytes
ran out — so truncation detection must happen there. Errors raised in `openai.rs`
propagate `one_completion` → `completion_with_retry`
(`loop_.rs:112-131`, retried up to `max_retries`, then emitted as
`AgentEvent::Error`).

## Finding 1 — Truncated stream looks like clean completion (MED)

**Where:** `openai.rs:262-265`, the byte-stream `None` (end-of-stream) branch.

**Bug:** When the underlying byte stream ends, the loop flushes the
`ThinkingSplitter` and returns cleanly — regardless of whether any terminal
marker was seen. A mid-response TCP truncation (no `finish_reason`, no `[DONE]`)
therefore yields no error and no `Chunk::Done`; the consumer keeps the default
`StopReason::Stop` and treats a cut-off response as a complete one.

### Fix

Track whether a terminal marker was seen, and treat a premature end as an error:

```rust
let mut saw_terminal = false;
// ... inside the Some(Ok(chunks)) arm:
    let is_done = line.strip_prefix("data:").map(str::trim) == Some("[DONE]");
    for chunk in chunks {
        if matches!(chunk, Chunk::Done(_)) { saw_terminal = true; } // finish_reason terminal
        yield Ok(chunk);
    }
    if is_done {
        saw_terminal = true;
        for chunk in splitter.flush() { yield Ok(chunk); }
        return;
    }
    continue;
// ... the byte-stream None branch becomes:
    None => {
        if !saw_terminal {
            yield Err(ModelError::Stream(
                "stream ended before a completion marker (truncated response)".into()));
            return;
        }
        for chunk in splitter.flush() { yield Ok(chunk); }
        return;
    }
```

A terminal marker is **either** a `finish_reason` delta (which `parse_sse_line`
turns into `Chunk::Done`) **or** the `[DONE]` sentinel. Well-behaved servers
(OpenAI, llama.cpp) always send one, so well-formed streams are unaffected; only
a genuinely cut-off stream now errors. `ModelError::Stream` is retryable via
`completion_with_retry`, which is the right default — truncation is usually
transient.

## Findings 2 + 3 — Malformed line aborts the stream; in-band 200 error swallowed (MED)

**Where:** `openai.rs:159-162` (malformed-JSON path), `:236-239` (consumer aborts
on any `Err`), `:214-221` (only non-2xx status produces `ModelError::Status`; a
200 body that is an error object parses to empty deltas at `:163-187` and is
silently ignored).

**Bugs:**
- **Finding 2:** a single `data:` line with malformed JSON returns
  `Some(Err(ModelError::Decode(_)))`, and the consumer's `Some(Err(e))` arm
  yields the error and `return`s — one bad line kills the whole stream, forcing a
  full re-generation on retry.
- **Finding 3:** some backends (llama.cpp slot limits, etc.) send
  `data: {"error":{...}}` mid-stream with HTTP 200. `parse_sse_line` parses it as
  JSON, finds no `choices[0].delta`, produces no chunks, and the error is
  swallowed — the turn looks empty/successful.

### Fix — distinguish the two error kinds by variant

`parse_sse_line` keeps returning `ModelError::Decode` for malformed JSON
(**skip**), and newly detects an in-band error object, returning
`ModelError::Stream` (**abort**):

```rust
let v: Value = match serde_json::from_str(data) {
    Ok(v) => v,
    Err(e) => return Some(Err(ModelError::Decode(e.to_string()))), // skip (transient corruption)
};
if let Some(err) = v.get("error") {                                 // in-band 200-body error
    let msg = err.get("message").and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| err.to_string());
    return Some(Err(ModelError::Stream(format!("server error in stream: {msg}"))));
}
// ... existing delta/finish_reason parsing unchanged ...
```

The consumer distinguishes by variant — `Decode` skips with a warning, anything
else is terminal:

```rust
match parse_sse_line(&line, &mut splitter) {
    None => continue,
    Some(Err(ModelError::Decode(e))) => {
        tracing::warn!(error = %e, "skipping malformed SSE data line");
        continue;
    }
    Some(Err(e)) => { yield Err(e); return; } // terminal: in-band server error
    Some(Ok(chunks)) => { /* unchanged + saw_terminal handling from Finding 1 */ }
}
```

**Chosen behavior for finding 2:** skip-and-continue (a single corrupt chunk no
longer nukes a whole generation), with a `tracing::warn`. The residual risk — a
dropped content delta making output subtly incomplete — is mitigated by the warn
and by the fact that Finding 1's terminal-marker detection still catches genuine
cut-offs.

## Error handling / data flow

All three new error paths produce a `ModelError`:
- **Truncation** → `ModelError::Stream` (retryable).
- **In-band server error** → `ModelError::Stream` (retryable; if the underlying
  cause is persistent, it re-fails and surfaces after `max_retries`).
- **Malformed line** → no error; a `tracing::warn` and the stream continues.

No new `ModelError` variants: `Decode` is the skip signal, `Stream` is the
terminal signal. The error-kind contract (`Decode` = skip-able, everything else =
terminal) is documented at both the `parse_sse_line` return site and the consumer
match.

## Testing (TDD — write the failing test first)

Use the existing `wiremock` SSE harness in `openai.rs` tests (`MockServer`,
`ResponseTemplate::set_body_string` with an SSE payload, `OpenAiCompatClient`,
collect the stream with `StreamExt`).

- **Truncation (Finding 1):** a 200 body with a content delta but **no**
  `finish_reason` and **no** `data: [DONE]`, then close → the stream yields the
  text chunk(s), then a terminal `Err(ModelError::Stream)` whose message mentions
  truncation. (Fails on current code: stream ends cleanly with no error.)
- **Malformed-line skip (Finding 2):** good delta → `data: {bad json` → good
  delta → `data: [DONE]` → the stream completes `Ok` (no `Err`) and the collected
  text is both good deltas concatenated, proving the bad line was skipped, not
  aborted on. (Fails on current code: the stream aborts at the bad line.)
- **In-band error (Finding 3):** a 200 body `data: {"error":{"message":"boom"}}`
  → the stream yields `Err` whose message contains `boom`. (Fails on current
  code: the error is swallowed and the stream ends empty.)
- **Regression:** all existing `openai.rs` stream tests stay green — they end with
  a proper terminal (`finish_reason` and/or `[DONE]`), so neither truncation
  detection nor the variant split changes their behavior.

## Scope

**In scope:** `agent-model/src/openai.rs` only — `parse_sse_line` (in-band error
detection) and the `stream!` loop (`saw_terminal` tracking, the `Decode`-skip /
other-abort split). Tests in the same file.

**Out of scope (explicit non-goals):**
- The loop consumer's `StopReason` defaulting in `loop_.rs` — unchanged; it
  already reacts correctly once the client emits an error or a `Chunk::Done`.
- New `ModelError` variants — reuse `Decode`/`Stream`.
- **B3** — live cancellation wiring (Ctrl-C/SIGINT into `ToolCtx`). Separate spec.
- **Cluster A** — server concurrency & leaks. Separate specs.

## Alternatives considered (malformed-line handling)

1. **Skip-and-continue with a warn — CHOSEN.** One corrupt line no longer aborts
   the whole generation; the retry cost of a full re-generation is avoided.
2. **Keep aborting the whole stream (current).** Safer against structurally
   broken framing, but one transient bad line forces a full re-generation.
3. **Skip up to a threshold, then abort.** Most robust but adds a counter +
   threshold constant — more moving parts for a rare case (YAGNI).
