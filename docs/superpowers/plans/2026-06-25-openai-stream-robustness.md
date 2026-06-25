# B2 — OpenAI Stream Robustness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the OpenAI-compatible streaming client surface truncated streams and in-band 200-body errors as proper `ModelError`s, and tolerate a single malformed SSE line instead of aborting the whole generation.

**Architecture:** All changes are in `agent-model/src/openai.rs` — `parse_sse_line` (detect an in-band `error` object) and the `stream!` loop in `OpenAiCompatClient::stream` (split the consumer error arm into `Decode`=skip / other=terminal, and track `saw_terminal` to flag a premature byte-stream end). No new `ModelError` variants.

**Tech Stack:** Rust, `async_stream`, `futures`, `serde_json`, `reqwest`; tests use `wiremock` (SSE bodies via `ResponseTemplate::set_body_string`) and `tracing`.

## Global Constraints

- TDD: write the failing test first, watch it fail, then the minimal fix.
- Run tests from the Rust workspace root: `cd agent` first. `source ~/.cargo/env` if cargo is not on PATH.
- Single crate/file: `agent-model/src/openai.rs`. Do not change `agent-core` or add `ModelError` variants.
- Error-kind contract: `parse_sse_line` returns `ModelError::Decode` **only** for malformed JSON (the skip signal); any other `Err` from it (e.g. an in-band server error) is terminal.
- Preserve existing behavior: every existing `openai.rs` test stays green (they all end with `finish_reason` + `data: [DONE]`).

## Reference — confirmed facts (do not re-derive)

- `parse_sse_line(line: &str, splitter: &mut ThinkingSplitter) -> Option<Result<Vec<Chunk>, ModelError>>`. Malformed JSON path is at ~159-162; `let choice = &v["choices"][0];` follows at ~163.
- The `stream!` loop is at ~225-268: `let mut splitter = ThinkingSplitter::default();` (~227); consumer match (~234-253) with `Some(Err(e)) => { yield Err(e); return; }` (~236-239); the `Some(Ok(chunks))` arm (~240-252); the byte-stream `None` branch (~262-265).
- `ModelError::{Decode(String), Stream(String), Status{code,body}, ...}` (from `types.rs`).
- Test harness pattern (existing): `OpenAiCompatClient::new(server.uri(), "m".into(), None)`, `CompletionRequest { messages: vec![Message::user("hi")], ..Default::default() }`, `client.stream(req).await`, consume with `while let Some(item) = stream.next().await`.

---

### Task 1: Skip malformed lines; surface in-band 200-body errors (findings 2 + 3)

**Files:**
- Modify: `agent/crates/agent-model/src/openai.rs` — `parse_sse_line` (in-band error detection) and the consumer `Some(Err(_))` arm (variant split).
- Test: `agent/crates/agent-model/src/openai.rs` (the existing `#[cfg(test)] mod tests`).

**Interfaces:**
- Produces: `parse_sse_line` now returns `Some(Err(ModelError::Stream(...)))` when a parsed data object has a top-level `error` key; `ModelError::Decode` remains malformed-JSON only. The stream consumer skips `Decode` (with a `tracing::warn`) and aborts on any other `Err`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `openai.rs`:

```rust
#[tokio::test]
async fn skips_malformed_sse_line_and_keeps_streaming() {
    let server = MockServer::start().await;
    // A good delta, then a malformed data line, then another good delta + terminal.
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\n\n\
                data: {bad json\n\n\
                data: {\"choices\":[{\"delta\":{\"content\":\"B\"}}]}\n\n\
                data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                data: [DONE]\n\n";
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body))
        .mount(&server).await;
    let client = OpenAiCompatClient::new(server.uri(), "m".into(), None);
    let req = CompletionRequest { messages: vec![Message::user("hi")], ..Default::default() };
    let mut stream = client.stream(req).await.unwrap();

    let mut text = String::new();
    let mut done = None;
    while let Some(item) = stream.next().await {
        // unwrap() would panic if the bad line aborted the stream with an Err.
        match item.unwrap() {
            Chunk::Text(t) => text.push_str(&t),
            Chunk::Done(r) => done = Some(r),
            _ => {}
        }
    }
    assert_eq!(text, "AB", "the malformed line is skipped, both good deltas survive");
    assert_eq!(done, Some(StopReason::Stop));
}

#[tokio::test]
async fn surfaces_in_band_error_object_in_200_body() {
    let server = MockServer::start().await;
    let body = "data: {\"error\":{\"message\":\"boom\"}}\n\n";
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body))
        .mount(&server).await;
    let client = OpenAiCompatClient::new(server.uri(), "m".into(), None);
    let req = CompletionRequest { messages: vec![Message::user("hi")], ..Default::default() };
    let mut stream = client.stream(req).await.unwrap();

    let mut err = None;
    while let Some(item) = stream.next().await {
        if let Err(e) = item { err = Some(e); break; }
    }
    match err {
        Some(ModelError::Stream(m)) => assert!(m.contains("boom"), "message was: {m}"),
        other => panic!("expected Stream error carrying the in-band message, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd agent && cargo test -p agent-model --lib skips_malformed_sse_line_and_keeps_streaming surfaces_in_band` (run each name separately — `cargo test` takes one filter; use `cargo test -p agent-model --lib malformed` then `... in_band`, or run the whole file).
Expected: `skips_malformed_sse_line...` FAILS — `item.unwrap()` panics because the bad line aborts the stream with `ModelError::Decode`. `surfaces_in_band...` FAILS — the error object is swallowed, the stream ends with no `Err`, so `err` is `None` and the `panic!` arm fires.

- [ ] **Step 3: Detect the in-band error in `parse_sse_line`**

In `openai.rs`, immediately after the JSON-parse block and before `let choice = &v["choices"][0];`, insert:

```rust
    // A 200-status stream can still carry an error object instead of choices
    // (e.g. llama.cpp slot limits). Surface it instead of parsing empty deltas.
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| err.to_string());
        return Some(Err(ModelError::Stream(format!("server error in stream: {msg}"))));
    }
```

The malformed-JSON arm above it is unchanged (`Err(e) => return Some(Err(ModelError::Decode(e.to_string())))`).

- [ ] **Step 4: Split the consumer error arm (skip `Decode`, abort otherwise)**

In the `stream!` loop, replace:

```rust
                        Some(Err(e)) => {
                            yield Err(e);
                            return;
                        }
```

with:

```rust
                        // A malformed `data:` line (Decode) is transient corruption —
                        // skip it and keep streaming. Any other error (e.g. an in-band
                        // server error) is terminal.
                        Some(Err(ModelError::Decode(e))) => {
                            tracing::warn!(error = %e, "skipping malformed SSE data line");
                            continue;
                        }
                        Some(Err(e)) => {
                            yield Err(e);
                            return;
                        }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd agent && cargo test -p agent-model --lib` (runs the whole file).
Expected: both new tests PASS; all existing `openai.rs` tests stay green.

- [ ] **Step 6: Commit**

```bash
cd agent && git add crates/agent-model/src/openai.rs
git commit -m "fix(openai): skip malformed SSE lines, surface in-band 200 errors

A single malformed data: line no longer aborts the whole stream (skip + warn);
a {\"error\":...} object in a 200 body is surfaced as ModelError::Stream instead
of being swallowed. Decode=skip / other=terminal variant contract."
```

---

### Task 2: Detect truncated streams (finding 1)

**Files:**
- Modify: `agent/crates/agent-model/src/openai.rs` — the `stream!` loop (`saw_terminal` tracking + the `None` branch).
- Test: `agent/crates/agent-model/src/openai.rs` (`mod tests`).

**Interfaces:**
- Produces: when the byte stream ends without ever yielding a terminal marker (a `finish_reason` `Chunk::Done` or the `[DONE]` sentinel), the stream yields a terminal `Err(ModelError::Stream(...))` mentioning truncation, instead of ending cleanly.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `openai.rs`:

```rust
#[tokio::test]
async fn truncated_stream_without_terminal_marker_errors() {
    let server = MockServer::start().await;
    // A content delta, then the body just ends — no finish_reason, no [DONE].
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body))
        .mount(&server).await;
    let client = OpenAiCompatClient::new(server.uri(), "m".into(), None);
    let req = CompletionRequest { messages: vec![Message::user("hi")], ..Default::default() };
    let mut stream = client.stream(req).await.unwrap();

    let mut text = String::new();
    let mut err = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(Chunk::Text(t)) => text.push_str(&t),
            Ok(_) => {}
            Err(e) => { err = Some(e); break; }
        }
    }
    assert_eq!(text, "partial", "deltas before the cut-off are still delivered");
    match err {
        Some(ModelError::Stream(m)) => assert!(m.contains("truncat"), "message was: {m}"),
        other => panic!("expected a truncation Stream error, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd agent && cargo test -p agent-model --lib truncated_stream_without_terminal_marker_errors`
Expected: FAIL — current code hits the `None` branch, flushes, and returns cleanly with no `Err`, so `err` is `None` and the `panic!` arm fires.

- [ ] **Step 3: Track `saw_terminal`**

Add the flag after the splitter is created. Replace:

```rust
            let mut splitter = ThinkingSplitter::default();
```

with:

```rust
            let mut splitter = ThinkingSplitter::default();
            let mut saw_terminal = false;
```

- [ ] **Step 4: Mark the terminal in the `Some(Ok(chunks))` arm**

Replace:

```rust
                        Some(Ok(chunks)) => {
                            // Only the bare `data: [DONE]` sentinel ends the stream —
                            // NOT a content delta that merely contains "[DONE]".
                            let is_done = line.strip_prefix("data:").map(str::trim) == Some("[DONE]");
                            for chunk in chunks {
                                yield Ok(chunk);
                            }
                            if is_done {
                                for chunk in splitter.flush() { yield Ok(chunk); }
                                return;
                            }
                            continue;
                        }
```

with:

```rust
                        Some(Ok(chunks)) => {
                            // Only the bare `data: [DONE]` sentinel ends the stream —
                            // NOT a content delta that merely contains "[DONE]".
                            let is_done = line.strip_prefix("data:").map(str::trim) == Some("[DONE]");
                            for chunk in chunks {
                                if matches!(chunk, Chunk::Done(_)) { saw_terminal = true; }
                                yield Ok(chunk);
                            }
                            if is_done {
                                saw_terminal = true;
                                for chunk in splitter.flush() { yield Ok(chunk); }
                                return;
                            }
                            continue;
                        }
```

- [ ] **Step 5: Error on a premature byte-stream end**

Replace the `None` branch:

```rust
                    None => {
                        for chunk in splitter.flush() { yield Ok(chunk); }
                        return;
                    }
```

with:

```rust
                    None => {
                        // Byte stream ended. If no terminal marker (finish_reason or
                        // [DONE]) was ever seen, the response was truncated — surface it
                        // as a retryable error instead of a silent clean finish.
                        if !saw_terminal {
                            yield Err(ModelError::Stream(
                                "stream ended before a completion marker (truncated response)".into()));
                            return;
                        }
                        for chunk in splitter.flush() { yield Ok(chunk); }
                        return;
                    }
```

- [ ] **Step 6: Run the test + full file suite**

Run: `cd agent && cargo test -p agent-model --lib`
Expected: PASS — `truncated_stream_...` passes; all existing tests stay green (each ends with `finish_reason` + `[DONE]`, so `saw_terminal` is `true` before the byte end); Task 1's two tests still pass.

- [ ] **Step 7: Commit**

```bash
cd agent && git add crates/agent-model/src/openai.rs
git commit -m "fix(openai): detect truncated streams via terminal-marker tracking

A byte stream that ends with no finish_reason/[DONE] previously looked like a
clean stop. Track saw_terminal and surface a premature end as a retryable
ModelError::Stream so the loop can retry instead of accepting a cut-off turn."
```

---

### Task 3: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Build and test the crate**

Run: `cd agent && cargo test -p agent-model --lib`
Expected: PASS, no warnings about unused imports/variables.

- [ ] **Step 2: Downstream sweep (the loop consumer)**

Run: `cd agent && cargo build && cargo test -p agent-core --lib`
Expected: PASS — the only new behavior is additional `ModelError`s on previously-silent paths; `completion_with_retry` already handles `ModelError` uniformly, and no signature changed.

- [ ] **Step 3: Confirm the spec's testing checklist is satisfied**

Cross-check against `docs/superpowers/specs/2026-06-25-openai-stream-robustness-design.md` → "Testing": truncation error, malformed-line skip, in-band error, and existing-tests-green. All present and passing. If any is missing, add it before finishing.
