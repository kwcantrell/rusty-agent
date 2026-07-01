# Harness Observability + CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every tool call visible (id + status + duration), every session replayable (JSONL trace on by default), token/cost accounting faithful (reasoning/cached/claude-cli usage), per-session stats queryable + rendered in the web UI, and add a CI gate.

**Architecture:** Enrich the existing `AgentEvent` spine in place (spec-approved Approach A). One composite `ObservedSink` (stats fold + trace write + forward) wraps the frontend sink once in `assemble_loop`, so CLI, server, and desktop all get stats + tracing with zero per-frontend code. `SessionStats` is a pure fold in agent-core; `TraceWriter` and the sink live in agent-runtime-config; wire/web changes forward what used to be dropped.

**Tech Stack:** Rust (tokio, serde, futures) in the `agent/` Cargo workspace; React 19 + TypeScript + vitest in `web/`; bash + GitHub Actions for CI.

**Spec:** `docs/superpowers/specs/2026-07-01-harness-observability-ci-design.md` — read it first.

## Global Constraints

- Two Cargo workspaces: `agent/` and `src-tauri/`. All Rust commands below run from `agent/` unless stated. `source ~/.cargo/env` first if `cargo` is not on PATH.
- Conventional commits: `type(scope): summary`.
- The trace sink must NEVER fail a run: every I/O error → one `tracing::warn!`, then writes disabled for the session. No `unwrap()` on file I/O in `trace.rs`.
- Trace defaults (from spec): `trace: bool` default **true**, `trace_max_mb` default **64**, retention keep newest **50** files, dir default `~/.agent/sessions/`.
- Session id: `{unix_epoch_secs}-{pid}` (no chrono dependency exists in the workspace; epoch-secs is sortable and unique enough — accepted simplification of the spec's `YYYYMMDD-HHMMSS-<pid>`).
- `ToolStatus` serialization is snake_case everywhere (Rust serde, wire JSON, TS types).
- After each task: `cargo test -p <touched crates>`; after the last Rust task: `cargo test --workspace`.
- Existing tests that assert old event shapes/labels WILL break on Tasks 1–2; fixing them is part of those tasks, not a regression.

---

### Task 1: Event spine — ToolStatus, ids, durations, per-call terminal emission

**Files:**
- Modify: `agent/crates/agent-core/src/event.rs` (AgentEvent enum, new ToolStatus)
- Modify: `agent/crates/agent-core/src/loop_.rs` (gate_tool ~377, Phase 2 ~296-328, Phase 3 drain ~331-352, ServerUsage emit ~224-228, `Resolved` enum ~447-451, existing tests)
- Modify: `agent/crates/agent-core/src/testkit.rs` (CollectingSink labels ~104-129)
- Modify: `agent/crates/agent-server/src/wire.rs` (ServerEvent + server_event_from + tests)
- Modify: `agent/crates/agent-cli/src/render.rs` (TerminalSink ToolStart/ToolResult arms)
- Modify: any `agent/crates/agent-runtime-config/tests/*.rs` assertions the compiler/tests flag
- Test: new tests in `agent/crates/agent-core/src/loop_.rs` `mod tests`

**Interfaces:**
- Produces (later tasks rely on these exact shapes):
  - `pub enum ToolStatus { Ok, Denied, Error, Timeout, Panic }` with `pub fn as_str(&self) -> &'static str` (lowercase names), exported from agent-core via the existing `pub use event::*`.
  - `AgentEvent::ToolStart { id: String, name: String, args: serde_json::Value }`
  - `AgentEvent::ToolResult { id: String, name: String, status: ToolStatus, output: ToolOutput, duration_ms: u64 }`
  - `AgentEvent::ServerUsage { prompt_tokens: u32, completion_tokens: u32, reasoning_tokens: Option<u32>, cached_tokens: Option<u32>, cost_usd: Option<f64>, turn_duration_ms: u64, turn: usize }`
  - CollectingSink labels: `tool_start:{name}` (unchanged), `tool_result:{name}:{status}` (status via `as_str()`).

- [ ] **Step 1: Write the failing tests** (append to the existing `mod tests` in `loop_.rs`; reuse the file's existing loop-construction helper — the parallel-dispatch tests around line 1389 show the pattern with `ScriptedModel`/`PassthroughProtocol`/`CollectingSink`/`AlwaysApprove`). Add a structured capture sink for fields the string labels can't carry:

```rust
#[derive(Default)]
struct ToolEventCapture { results: Mutex<Vec<(String, String, ToolStatus, u64)>>, starts: Mutex<Vec<String>> }
impl EventSink for ToolEventCapture {
    fn emit(&self, event: AgentEvent) {
        match event {
            AgentEvent::ToolStart { id, .. } => self.starts.lock().unwrap().push(id),
            AgentEvent::ToolResult { id, name, status, duration_ms, .. } =>
                self.results.lock().unwrap().push((id, name, status, duration_ms)),
            _ => {}
        }
    }
}

#[tokio::test]
async fn every_resolved_call_emits_tool_result() {
    // Script one turn with three calls: one ok (echo tool), one unknown tool
    // (gate-rejected -> Denied), one erroring tool (-> Error). Build the loop
    // with the module's existing construction helper (see the parallel-dispatch
    // tests ~1389) but pass a ToolEventCapture as the sink.
    let results = capture.results.lock().unwrap();
    assert_eq!(results.len(), 3, "one terminal event per call, got {results:?}");
    let statuses: std::collections::HashSet<_> = results.iter().map(|r| r.2).collect();
    assert!(statuses.contains(&ToolStatus::Ok));
    assert!(statuses.contains(&ToolStatus::Denied));
    assert!(statuses.contains(&ToolStatus::Error));
}

#[tokio::test]
async fn tool_result_ids_match_tool_start() {
    // Two parallel ok calls through the same helper + ToolEventCapture.
    let starts: std::collections::HashSet<_> =
        capture.starts.lock().unwrap().iter().cloned().collect();
    let result_ids: std::collections::HashSet<_> =
        capture.results.lock().unwrap().iter().map(|r| r.0.clone()).collect();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts, result_ids);
}

#[tokio::test]
async fn executed_calls_report_nonzero_duration_and_denied_zero() {
    // One ok call whose scripted tool sleeps ~10ms, one unknown tool.
    let results = capture.results.lock().unwrap();
    let ok = results.iter().find(|r| r.2 == ToolStatus::Ok).unwrap();
    let denied = results.iter().find(|r| r.2 == ToolStatus::Denied).unwrap();
    assert!(ok.3 >= 5, "executed duration_ms should reflect the ~10ms sleep, got {}", ok.3);
    assert_eq!(denied.3, 0, "gate-rejected calls never executed");
}

#[tokio::test]
async fn server_usage_carries_turn_duration() {
    // Any single scripted turn; capture ServerUsage in a small sink that stores
    // turn_duration_ms. Assert the field exists and is plausibly measured:
    assert!(captured_turn_duration_ms.lock().unwrap().is_some());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-core every_resolved_call_emits_tool_result`
Expected: FAIL — compile errors (`ToolStatus` not found; `ToolStart` has no field `id`). That is the expected failure mode for an enum-shape task.

- [ ] **Step 3: Implement the event model** in `event.rs`. Add above `AgentEvent`:

```rust
/// Terminal status of one tool call — carried on every ToolResult so
/// observers/evals can compute error and denial rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus { Ok, Denied, Error, Timeout, Panic }

impl ToolStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok", Self::Denied => "denied", Self::Error => "error",
            Self::Timeout => "timeout", Self::Panic => "panic",
        }
    }
}
```

Change the three variants to exactly the shapes in **Interfaces** above (ServerUsage keeps its doc comment; add `/// duration_ms is 0 for gate-rejected calls that never executed.` on ToolResult).

- [ ] **Step 4: Implement the loop changes** in `loop_.rs`:

(a) `Resolved` (~447):

```rust
/// Final per-call result feeding the tool-result message + terminal event.
enum Resolved {
    Ok(agent_tools::ToolOutput, u64),
    /// Terminal `ERROR: …` content (rejected, failed, timed out, or panicked).
    Err { status: ToolStatus, content: String, duration_ms: u64 },
}
```

(b) Phase-1 gate rejections (~282-285):

```rust
GateOutcome::Rejected { id, name, content } => {
    order.push(id.clone());
    results.insert(id, (name, Resolved::Err {
        status: ToolStatus::Denied, content, duration_ms: 0 }));
}
```

(c) Phase-2 closure (~299-308) — time the execution:

```rust
async move {
    let started = std::time::Instant::now();
    let ex = execute_isolated(tool, args, &name, &ctx).await;
    (id, name, ex, started.elapsed().as_millis() as u64)
}
```

and the mapping (~309-328) — keep the existing tracing/Error emissions for Panicked/TimedOut exactly as they are, only change the `Resolved` construction:

```rust
for (id, name, ex, duration_ms) in executed {
    let resolved = match ex {
        Executed::Ok(o) => Resolved::Ok(o, duration_ms),
        Executed::ToolErr(s) => Resolved::Err {
            status: ToolStatus::Error, content: s, duration_ms },
        Executed::Panicked(s) => { /* existing tracing::error! + sink Error emit */ Resolved::Err {
            status: ToolStatus::Panic, content: s, duration_ms } }
        Executed::TimedOut(s) => { /* existing tracing::warn! + sink Error emit */ Resolved::Err {
            status: ToolStatus::Timeout, content: s, duration_ms } }
    };
    results.insert(id, (name, resolved));
}
```

(d) Phase-3 drain (~331-352) — emit for EVERY arm; the internal-invariant fallback becomes `Resolved::Err { status: ToolStatus::Error, content: …, duration_ms: 0 }`:

```rust
let content = match resolved {
    Resolved::Ok(output, duration_ms) => {
        self.sink.emit(AgentEvent::ToolResult { id: id.clone(), name: name.clone(),
            status: ToolStatus::Ok, output: output.clone(), duration_ms });
        output.content
    }
    Resolved::Err { status, content, duration_ms } => {
        self.sink.emit(AgentEvent::ToolResult { id: id.clone(), name: name.clone(),
            status, output: agent_tools::ToolOutput { content: content.clone(), display: None },
            duration_ms });
        content
    }
};
ctx.append(Message::tool(id, name, content));
```

(e) `gate_tool` ToolStart (~377): `AgentEvent::ToolStart { id: call.id.clone(), name: call.name.clone(), args: call.args.clone() }`.

(f) Turn duration + widened ServerUsage (~216-228): wrap the completion in an `Instant`:

```rust
let turn_started = std::time::Instant::now();
let assistant = match self.completion_with_retry(&base, &cancel).await { /* unchanged arms */ };
self.sink.emit(AgentEvent::ServerUsage {
    prompt_tokens: assistant.prompt_tokens,
    completion_tokens: assistant.completion_tokens,
    reasoning_tokens: None,  // wired in Task 2
    cached_tokens: None,     // wired in Task 2
    cost_usd: None,          // wired in Task 3
    turn_duration_ms: turn_started.elapsed().as_millis() as u64,
    turn: turn + 1,
});
```

- [ ] **Step 5: Update the compile sites the compiler flags** (mechanical; the shapes are fixed by Interfaces):

(a) `testkit.rs` CollectingSink: `tool_result:{name}` label becomes

```rust
AgentEvent::ToolResult { name, status, .. } =>
    format!("tool_result:{name}:{}", status.as_str()),
```

(`tool_start:{name}` label unchanged — the new `id` field is just ignored with `..`.)

(b) `wire.rs` ServerEvent variants:

```rust
ToolStart { id: String, name: String, args: serde_json::Value },
ToolResult {
    id: String,
    name: String,
    status: String,
    duration_ms: u64,
    content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display: Option<Display>,
},
ServerUsage {
    prompt_tokens: u32, completion_tokens: u32, turn: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cached_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cost_usd: Option<f64>,
    #[serde(default)]
    turn_duration_ms: u64,
},
```

and `server_event_from`:

```rust
AgentEvent::ToolStart { id, name, args } => ServerEvent::ToolStart { id, name, args },
AgentEvent::ToolResult { id, name, status, output, duration_ms } => ServerEvent::ToolResult {
    id, name, status: status.as_str().into(), duration_ms,
    content: output.content, display: output.display },
AgentEvent::ServerUsage { prompt_tokens, completion_tokens, reasoning_tokens,
    cached_tokens, cost_usd, turn_duration_ms, turn } =>
    ServerEvent::ServerUsage { prompt_tokens, completion_tokens, turn,
        reasoning_tokens, cached_tokens, cost_usd, turn_duration_ms },
```

(c) `render.rs` TerminalSink: `ToolStart { name, args, .. }` (ignore id). `ToolResult` arm: destructure `{ name, status, output, duration_ms, .. }`; BEFORE the existing display matching, add the failure one-liner and keep success rendering unchanged:

```rust
AgentEvent::ToolResult { name, status, output, duration_ms, .. } => {
    if status != ToolStatus::Ok {
        let _ = writeln!(out, "\x1b[31m✗ {name} ({}, {duration_ms}ms)\x1b[0m {}",
            status.as_str(), output.content);
    } else if let Some(Display::Diff { .. }) = &output.display {
        /* existing diff arm unchanged */
    } else if let Some(Display::Terminal { .. }) = &output.display {
        /* existing terminal arm unchanged */
    } else {
        let _ = writeln!(out, "\x1b[32m✓ {name}\x1b[0m");
    }
}
```

(d) Fix any test assertions across `agent-core`, `agent-server`, and `agent-runtime-config/tests/` that construct or match the old shapes (compiler + `cargo test --workspace` enumerate them; update labels to the `tool_result:{name}:{status}` form).

- [ ] **Step 6: Run the new tests + touched crates**

Run: `cargo test -p agent-core && cargo test -p agent-server && cargo test -p agent-cli && cargo test -p agent-runtime-config`
Expected: PASS (including the three new tests).

- [ ] **Step 7: Commit**

```bash
git add -A agent/crates
git commit -m "feat(core): per-call terminal ToolResult events with id, status, and duration_ms"
```

---

### Task 2: Chunk::Usage extension + OpenAI reasoning/cached-token parsing

**Files:**
- Modify: `agent/crates/agent-model/src/types.rs:75` (Chunk enum), AssistantTurn (~80-88)
- Modify: `agent/crates/agent-model/src/openai.rs:205-210` (usage parsing)
- Modify: `agent/crates/agent-core/src/loop_.rs` (one_completion usage accumulation ~107, 122-124; ServerUsage emit from Task 1)
- Modify: `agent/crates/agent-model/src/claude_cli.rs` (any `Chunk::Usage` construction the compiler flags — real parsing lands in Task 3)
- Test: `agent/crates/agent-model/src/openai.rs` `mod tests`

**Interfaces:**
- Consumes: `AgentEvent::ServerUsage` shape from Task 1.
- Produces: `Chunk::Usage { prompt_tokens: u32, completion_tokens: u32, reasoning_tokens: Option<u32>, cached_tokens: Option<u32>, cost_usd: Option<f64> }`; `AssistantTurn` gains `pub reasoning_tokens: Option<u32>, pub cached_tokens: Option<u32>, pub cost_usd: Option<f64>` (all in `Default`). Task 3 constructs this exact Chunk shape.

- [ ] **Step 1: Write the failing tests** in `openai.rs`'s existing test module, next to the current usage-parsing tests (find them with `rg -n "usage" crates/agent-model/src/openai.rs`):

```rust
#[test]
fn parses_reasoning_and_cached_token_details() {
    let line = r#"{"choices":[{"delta":{}}],"usage":{"prompt_tokens":100,"completion_tokens":50,"completion_tokens_details":{"reasoning_tokens":30},"prompt_tokens_details":{"cached_tokens":80}}}"#;
    // drive it through the same parse fn the existing usage test uses
    // assert Chunk::Usage { prompt_tokens: 100, completion_tokens: 50,
    //   reasoning_tokens: Some(30), cached_tokens: Some(80), cost_usd: None }
}

#[test]
fn usage_details_absent_yields_none() {
    let line = r#"{"choices":[{"delta":{}}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
    // assert reasoning_tokens == None && cached_tokens == None
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-model parses_reasoning_and_cached`
Expected: FAIL — compile error (Chunk::Usage has no field `reasoning_tokens`).

- [ ] **Step 3: Implement.** `types.rs`: extend `Chunk::Usage` and `AssistantTurn` per Interfaces. `openai.rs` usage block becomes:

```rust
if let Some(u) = v.get("usage").and_then(Value::as_object) {
    out.push(Chunk::Usage {
        prompt_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        completion_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        reasoning_tokens: u.get("completion_tokens_details")
            .and_then(|d| d.get("reasoning_tokens")).and_then(Value::as_u64).map(|n| n as u32),
        cached_tokens: u.get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens")).and_then(Value::as_u64).map(|n| n as u32),
        cost_usd: None,
    });
}
```

`loop_.rs one_completion`: replace the `usage = (0u32, 0u32)` tuple with the richer capture and thread into `AssistantTurn`:

```rust
let mut usage = (0u32, 0u32);
let mut usage_details: (Option<u32>, Option<u32>, Option<f64>) = (None, None, None);
// …
Chunk::Usage { prompt_tokens, completion_tokens, reasoning_tokens, cached_tokens, cost_usd } => {
    usage = (prompt_tokens, completion_tokens);
    usage_details = (reasoning_tokens, cached_tokens, cost_usd);
}
// …
Ok(AssistantTurn { text, raw_tool_calls, stop, reasoning,
    prompt_tokens: usage.0, completion_tokens: usage.1,
    reasoning_tokens: usage_details.0, cached_tokens: usage_details.1,
    cost_usd: usage_details.2 })
```

and the ServerUsage emit (Task 1 step 4f) replaces the three `None`s with `assistant.reasoning_tokens`, `assistant.cached_tokens`, `assistant.cost_usd`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p agent-model && cargo test -p agent-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates
git commit -m "feat(model): parse reasoning/cached token details into Chunk::Usage and ServerUsage"
```

---

### Task 3: claude-cli usage + cost parsing

**Files:**
- Modify: `agent/crates/agent-model/src/claude_cli.rs` (`parse_event_line`, the `Some("result")` arm ~172-183)
- Test: same file's `proc_tests` module

**Interfaces:**
- Consumes: `Chunk::Usage` shape from Task 2.
- Produces: claude-cli sessions emit real token counts + `cost_usd` instead of 0/0.

- [ ] **Step 1: Write the failing test** next to the existing `parse_event_line` tests:

```rust
#[test]
fn result_event_carries_usage_and_cost() {
    let line = r#"{"type":"result","subtype":"success","total_cost_usd":0.0421,"usage":{"input_tokens":1200,"output_tokens":345}}"#;
    let chunks = parse_event_line(line).unwrap();
    assert!(chunks.iter().any(|c| matches!(c,
        Chunk::Usage { prompt_tokens: 1200, completion_tokens: 345,
                       cost_usd: Some(c), .. } if (*c - 0.0421).abs() < 1e-9)));
    // Done must still be emitted after the usage chunk
    assert!(matches!(chunks.last(), Some(Chunk::Done(StopReason::Stop))));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-model result_event_carries_usage_and_cost`
Expected: FAIL — no `Chunk::Usage` in output.

- [ ] **Step 3: Implement** — in the `Some("result")` arm, BEFORE the existing `Chunk::Done` push:

```rust
Some("result") => {
    if let Some(u) = v.get("usage").and_then(Value::as_object) {
        out.push(Chunk::Usage {
            prompt_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            completion_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            reasoning_tokens: None,
            cached_tokens: None,
            cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
        });
    }
    // existing truncated/Done logic unchanged below
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agent-model`
Expected: PASS (including existing result-event tests — a `result` line without `usage` emits no Usage chunk, only Done).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-model/src/claude_cli.rs
git commit -m "feat(model): parse claude-cli result usage + total_cost_usd (fixes 0/0 ServerUsage)"
```

---

### Task 4: SessionStats pure fold

**Files:**
- Create: `agent/crates/agent-core/src/stats.rs`
- Modify: `agent/crates/agent-core/src/lib.rs` (add `pub mod stats;` + `pub use stats::SessionStats;` beside the existing exports at lines 13-23)
- Test: `mod tests` inside `stats.rs`

**Interfaces:**
- Consumes: `AgentEvent`/`ToolStatus` from Task 1.
- Produces (Tasks 6-9 rely on this exact struct — it is serialized to the wire and mirrored in TS):

```rust
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionStats {
    pub turns: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    pub cached_tokens: u64,
    pub cost_usd: f64,
    pub tool_calls: u64,
    pub tools_ok: u64,
    pub tools_denied: u64,
    pub tools_error: u64,
    pub tools_timeout: u64,
    pub tools_panic: u64,
    pub tool_time_ms: u64,
    pub turn_time_ms: u64,
    pub context_events: u64,
    pub errors: u64,
}
impl SessionStats { pub fn fold(&mut self, event: &AgentEvent) { … } }
```

- [ ] **Step 1: Write the failing test** (in the new file, module skeleton + test first):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentEvent, ContextEvent, ToolStatus};
    use agent_tools::ToolOutput;

    #[test]
    fn fold_accumulates_usage_tools_and_context() {
        let mut s = SessionStats::default();
        s.fold(&AgentEvent::ServerUsage { prompt_tokens: 100, completion_tokens: 40,
            reasoning_tokens: Some(10), cached_tokens: Some(60), cost_usd: Some(0.02),
            turn_duration_ms: 500, turn: 1 });
        s.fold(&AgentEvent::ServerUsage { prompt_tokens: 200, completion_tokens: 50,
            reasoning_tokens: None, cached_tokens: None, cost_usd: Some(0.03),
            turn_duration_ms: 700, turn: 2 });
        s.fold(&AgentEvent::ToolStart { id: "a".into(), name: "t".into(),
            args: serde_json::json!({}) });
        s.fold(&AgentEvent::ToolResult { id: "a".into(), name: "t".into(),
            status: ToolStatus::Ok,
            output: ToolOutput { content: "x".into(), display: None }, duration_ms: 30 });
        s.fold(&AgentEvent::ToolResult { id: "b".into(), name: "t".into(),
            status: ToolStatus::Timeout,
            output: ToolOutput { content: "e".into(), display: None }, duration_ms: 60000 });
        s.fold(&AgentEvent::Context(ContextEvent::CompactionFailed { reason: "r".into() }));
        s.fold(&AgentEvent::Error("boom".into()));

        assert_eq!(s.turns, 2);
        assert_eq!(s.prompt_tokens, 300);
        assert_eq!(s.completion_tokens, 90);
        assert_eq!(s.reasoning_tokens, 10);
        assert_eq!(s.cached_tokens, 60);
        assert!((s.cost_usd - 0.05).abs() < 1e-9);
        assert_eq!(s.turn_time_ms, 1200);
        assert_eq!(s.tool_calls, 1);           // counted on ToolStart
        assert_eq!(s.tools_ok, 1);
        assert_eq!(s.tools_timeout, 1);
        assert_eq!(s.tool_time_ms, 60030);
        assert_eq!(s.context_events, 1);
        assert_eq!(s.errors, 1);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-core fold_accumulates`
Expected: FAIL — module/struct not found.

- [ ] **Step 3: Implement the fold** (complete):

```rust
impl SessionStats {
    /// Pure accumulator over the event stream. Token/cost fields SUM per-turn
    /// server usage (total billed volume); `turns` tracks the highest turn seen.
    pub fn fold(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::ServerUsage { prompt_tokens, completion_tokens, reasoning_tokens,
                cached_tokens, cost_usd, turn_duration_ms, turn } => {
                self.turns = self.turns.max(*turn);
                self.prompt_tokens += *prompt_tokens as u64;
                self.completion_tokens += *completion_tokens as u64;
                self.reasoning_tokens += reasoning_tokens.unwrap_or(0) as u64;
                self.cached_tokens += cached_tokens.unwrap_or(0) as u64;
                self.cost_usd += cost_usd.unwrap_or(0.0);
                self.turn_time_ms += turn_duration_ms;
            }
            AgentEvent::ToolStart { .. } => self.tool_calls += 1,
            AgentEvent::ToolResult { status, duration_ms, .. } => {
                self.tool_time_ms += duration_ms;
                match status {
                    crate::ToolStatus::Ok => self.tools_ok += 1,
                    crate::ToolStatus::Denied => self.tools_denied += 1,
                    crate::ToolStatus::Error => self.tools_error += 1,
                    crate::ToolStatus::Timeout => self.tools_timeout += 1,
                    crate::ToolStatus::Panic => self.tools_panic += 1,
                }
            }
            AgentEvent::Context(_) => self.context_events += 1,
            AgentEvent::Error(_) => self.errors += 1,
            _ => {}
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agent-core stats`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/stats.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(core): SessionStats pure fold over the event stream"
```

---

### Task 5: TraceWriter + RuntimeConfig trace settings

**Files:**
- Create: `agent/crates/agent-runtime-config/src/trace.rs`
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (export `pub mod trace;` + re-export `trace::{TraceWriter, build_trace, ObservedSink}` following the crate's existing export style)
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (RuntimeConfig + PartialRuntimeConfig + the merge in `load_over` + `normalized` if it enumerates fields)
- Test: `mod tests` inside `trace.rs` (uses `tempfile` — check `rg tempfile crates/agent-runtime-config/Cargo.toml`; it is already a dev-dependency in several crates, add `tempfile = "3"` to `[dev-dependencies]` if absent)

**Interfaces:**
- Consumes: `AgentEvent`, `ContextEvent`, `SessionStats`, `ToolStatus` (Tasks 1+4).
- Produces:
  - `TraceWriter::create(dir: &Path, max_mb: u64) -> Option<Arc<TraceWriter>>`
  - `TraceWriter::record(&self, event: &AgentEvent)` (infallible)
  - `TraceWriter::session_id(&self) -> &str`
  - `pub fn build_trace(cfg: &RuntimeConfig) -> Option<Arc<TraceWriter>>`
  - `ObservedSink { inner, stats, trace }` implementing `EventSink` (fold → trace → forward)
  - RuntimeConfig fields: `trace: bool` (default true), `trace_dir: Option<String>`, `trace_max_mb: u64` (default 64)

- [ ] **Step 1: Write the failing tests** (in `trace.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{AgentEvent, ToolStatus};
    use agent_tools::ToolOutput;

    fn ev_ok() -> AgentEvent {
        AgentEvent::ToolResult { id: "c1".into(), name: "read_file".into(),
            status: ToolStatus::Ok,
            output: ToolOutput { content: "hi".into(), display: None }, duration_ms: 7 }
    }

    #[test]
    fn trace_writes_parseable_jsonl_with_header() {
        let dir = tempfile::tempdir().unwrap();
        let w = TraceWriter::create(dir.path(), 64).unwrap();
        w.record(&ev_ok());
        w.record(&AgentEvent::Done(agent_model::StopReason::Stop)); // Done flushes
        let path = dir.path().join(format!("{}.jsonl", w.session_id()));
        let body = std::fs::read_to_string(path).unwrap();
        let lines: Vec<serde_json::Value> = body.lines()
            .map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["schema"], 1);
        assert_eq!(lines[0]["session"], w.session_id());
        assert_eq!(lines[1]["event"]["type"], "tool_result");
        assert_eq!(lines[1]["event"]["status"], "ok");
        assert_eq!(lines[1]["seq"], 0);
        assert_eq!(lines[2]["event"]["type"], "done");
    }

    #[test]
    fn trace_respects_size_cap() {
        let dir = tempfile::tempdir().unwrap();
        let w = TraceWriter::create(dir.path(), 0).unwrap(); // 0 MB => cap hit immediately
        w.record(&ev_ok());
        w.record(&AgentEvent::Done(agent_model::StopReason::Stop));
        let path = dir.path().join(format!("{}.jsonl", w.session_id()));
        let body = std::fs::read_to_string(path).unwrap();
        assert_eq!(body.lines().count(), 1); // header only; cap stopped event writes
    }

    #[test]
    fn trace_prunes_to_retention() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..60 {
            std::fs::write(dir.path().join(format!("{:010}-1.jsonl", i)), "x").unwrap();
        }
        let _w = TraceWriter::create(dir.path(), 64).unwrap();
        let count = std::fs::read_dir(dir.path()).unwrap().count();
        assert!(count <= 50, "expected <=50 files after prune, got {count}");
    }

    #[test]
    fn trace_survives_unwritable_dir() {
        // A path that cannot be created (a FILE where the dir should be).
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocked");
        std::fs::write(&blocker, "not a dir").unwrap();
        assert!(TraceWriter::create(&blocker, 64).is_none()); // None, no panic
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-runtime-config trace_`
Expected: FAIL — `TraceWriter` not found.

- [ ] **Step 3: Implement `trace.rs`** (complete skeleton — fill nothing in later, this is the whole shape):

```rust
//! JSONL session trace + composite observability sink. NEVER fails a run:
//! every I/O error warns once and disables further writes.
use agent_core::{AgentEvent, ContextEvent, EventSink, SessionStats};
use agent_model::StopReason;
use serde::Serialize;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const TRACE_SCHEMA: u32 = 1;
const RETAIN_FILES: usize = 50;

pub struct TraceWriter {
    session_id: String,
    max_bytes: u64,
    inner: Mutex<Inner>,
}
struct Inner { w: Option<BufWriter<fs::File>>, written: u64, seq: u64 }

impl TraceWriter {
    pub fn session_id(&self) -> &str { &self.session_id }

    pub fn create(dir: &Path, max_mb: u64) -> Option<Arc<TraceWriter>> {
        if let Err(e) = fs::create_dir_all(dir) {
            tracing::warn!(target: "trace", error = %e, dir = %dir.display(),
                "cannot create trace dir; session tracing disabled");
            return None;
        }
        prune_retention(dir, RETAIN_FILES - 1); // -1: our new file makes 50
        let session_id = mint_session_id();
        let path = dir.join(format!("{session_id}.jsonl"));
        let file = match fs::OpenOptions::new().create(true).append(true).open(&path) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(target: "trace", error = %e, path = %path.display(),
                    "cannot open trace file; session tracing disabled");
                return None;
            }
        };
        let mut w = BufWriter::new(file);
        let header = serde_json::json!({
            "schema": TRACE_SCHEMA, "session": session_id, "started_ms": epoch_ms() });
        let _ = writeln!(w, "{header}");
        let _ = w.flush();
        Some(Arc::new(TraceWriter { session_id, max_bytes: max_mb.saturating_mul(1024 * 1024),
            inner: Mutex::new(Inner { w: Some(w), written: 0, seq: 0 }) }))
    }

    /// Append one event. Infallible; disables itself on error or cap breach.
    /// (Borrow order matters: read `seq`/`written` before taking `w` mutably.)
    pub fn record(&self, event: &AgentEvent) {
        let Ok(mut inner) = self.inner.lock() else { return };
        if inner.w.is_none() { return; }
        let rec = TraceRecord { seq: inner.seq, ts_ms: epoch_ms(), event: trace_event(event) };
        let line = match serde_json::to_string(&rec) { Ok(l) => l, Err(_) => return };
        if inner.written + line.len() as u64 + 1 > self.max_bytes {
            tracing::warn!(target: "trace", cap_mb = self.max_bytes / (1024 * 1024),
                "trace size cap reached; tracing disabled for this session");
            if let Some(w) = inner.w.as_mut() { let _ = w.flush(); }
            inner.w = None;
            return;
        }
        let flush = matches!(event, AgentEvent::Done(_) | AgentEvent::Error(_));
        let failed = match inner.w.as_mut() {
            Some(w) => writeln!(w, "{line}").is_err() || (flush && w.flush().is_err()),
            None => return,
        };
        if failed {
            tracing::warn!(target: "trace", "trace write failed; tracing disabled for this session");
            inner.w = None;
            return;
        }
        inner.written += line.len() as u64 + 1;
        inner.seq += 1;
    }
}

fn mint_session_id() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    format!("{secs}-{}", std::process::id())
}
fn epoch_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// Keep only the newest `keep` *.jsonl files (name-sorted; epoch-prefixed names sort chronologically).
fn prune_retention(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut names: Vec<_> = entries.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .map(|e| e.path()).collect();
    names.sort();
    if names.len() > keep {
        let excess = names.len() - keep;
        for p in names.into_iter().take(excess) { let _ = fs::remove_file(p); }
    }
}

#[derive(Serialize)]
struct TraceRecord<'a> { seq: u64, ts_ms: u64, event: TraceEvent<'a> }

/// Serializable mirror of AgentEvent — a stable on-disk schema decoupled from
/// the in-process enum (same pattern as wire.rs's ServerEvent).
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TraceEvent<'a> {
    Token { text: &'a str },
    Reasoning { text: &'a str },
    Usage { prompt_tokens: usize, context_limit: usize, turn: usize, max_turns: usize },
    ServerUsage { prompt_tokens: u32, completion_tokens: u32,
        reasoning_tokens: Option<u32>, cached_tokens: Option<u32>,
        cost_usd: Option<f64>, turn_duration_ms: u64, turn: usize },
    ToolStart { id: &'a str, name: &'a str, args: &'a serde_json::Value },
    ToolResult { id: &'a str, name: &'a str, status: &'static str,
        duration_ms: u64, content: &'a str },
    Approval { summary: &'a str, command: Option<&'a str> },
    Error { message: &'a str },
    Done { reason: &'static str },
    Context { kind: &'static str, detail: serde_json::Value },
    SandboxDegraded { mechanism: &'a str, reason: &'a str },
}

fn trace_event(e: &AgentEvent) -> TraceEvent<'_> {
    match e {
        AgentEvent::Token(t) => TraceEvent::Token { text: t },
        AgentEvent::Reasoning(t) => TraceEvent::Reasoning { text: t },
        AgentEvent::Usage { prompt_tokens, context_limit, turn, max_turns } =>
            TraceEvent::Usage { prompt_tokens: *prompt_tokens, context_limit: *context_limit,
                turn: *turn, max_turns: *max_turns },
        AgentEvent::ServerUsage { prompt_tokens, completion_tokens, reasoning_tokens,
            cached_tokens, cost_usd, turn_duration_ms, turn } =>
            TraceEvent::ServerUsage { prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens, reasoning_tokens: *reasoning_tokens,
                cached_tokens: *cached_tokens, cost_usd: *cost_usd,
                turn_duration_ms: *turn_duration_ms, turn: *turn },
        AgentEvent::ToolStart { id, name, args } => TraceEvent::ToolStart { id, name, args },
        AgentEvent::ToolResult { id, name, status, output, duration_ms } =>
            TraceEvent::ToolResult { id, name, status: status.as_str(),
                duration_ms: *duration_ms, content: &output.content },
        AgentEvent::Approval(req) => TraceEvent::Approval {
            summary: &req.intent.summary, command: req.intent.command.as_deref() },
        AgentEvent::Error(m) => TraceEvent::Error { message: m },
        AgentEvent::Done(r) => TraceEvent::Done { reason: stop_reason_str(r) },
        AgentEvent::Context(c) => match c {
            ContextEvent::Offloaded { id, bytes, tool } => TraceEvent::Context {
                kind: "offloaded",
                detail: serde_json::json!({"id": id, "bytes": bytes, "tool": tool}) },
            ContextEvent::Compacted { turns_replaced, tokens_before, tokens_after } =>
                TraceEvent::Context { kind: "compacted",
                    detail: serde_json::json!({"turns_replaced": turns_replaced,
                        "tokens_before": tokens_before, "tokens_after": tokens_after}) },
            ContextEvent::CompactionFailed { reason } => TraceEvent::Context {
                kind: "compaction_failed", detail: serde_json::json!({"reason": reason}) },
        },
        AgentEvent::SandboxDegraded { mechanism, reason } =>
            TraceEvent::SandboxDegraded { mechanism, reason },
    }
}

fn stop_reason_str(r: &StopReason) -> &'static str {
    match r {
        StopReason::Stop => "stop", StopReason::ToolCalls => "tool_calls",
        StopReason::Length => "length", StopReason::BudgetExhausted => "budget_exhausted",
        StopReason::Cancelled => "cancelled",
    }
}

/// Composite sink: fold stats, write trace, forward to the frontend sink.
pub struct ObservedSink {
    pub inner: Arc<dyn EventSink>,
    pub stats: Arc<RwLock<SessionStats>>,
    pub trace: Option<Arc<TraceWriter>>,
}
impl EventSink for ObservedSink {
    fn emit(&self, event: AgentEvent) {
        if let Ok(mut s) = self.stats.write() { s.fold(&event); }
        if let Some(t) = &self.trace { t.record(&event); }
        self.inner.emit(event);
    }
}

/// Frontend helper: config → optional trace writer (None when disabled or dir unusable).
pub fn build_trace(cfg: &crate::RuntimeConfig) -> Option<Arc<TraceWriter>> {
    if !cfg.trace { return None; }
    let dir = match &cfg.trace_dir {
        Some(d) => std::path::PathBuf::from(d),
        None => std::path::PathBuf::from(std::env::var_os("HOME")?).join(".agent").join("sessions"),
    };
    TraceWriter::create(&dir, cfg.trace_max_mb)
}
```

Note: `Approval` mapping requires `req.intent.command: Option<String>` — confirm the field name against `agent_policy::ApprovalRequest`/`agent_tools::ToolIntent` (seen in sink.rs tests: `ToolIntent { tool, access, paths, command, summary }`).

- [ ] **Step 4: Add the config fields** in `runtime_config.rs`: to `RuntimeConfig` (after the sandbox fields):

```rust
#[serde(default = "default_true")]
pub trace: bool,
#[serde(default)]
pub trace_dir: Option<String>,
#[serde(default = "default_trace_max_mb")]
pub trace_max_mb: u64,
```

plus `fn default_trace_max_mb() -> u64 { 64 }`; mirror all three in `PartialRuntimeConfig` and the `load_over` merge, following exactly the pattern of `sandbox_tmp_size`. If a `RuntimeConfig` literal in tests/fixtures enumerates every field, the compiler will point at each — add the three fields with defaults.

- [ ] **Step 5: Run tests**

Run: `cargo test -p agent-runtime-config`
Expected: PASS (the four new trace tests + existing config round-trip tests).

- [ ] **Step 6: Commit**

```bash
git add -A agent/crates/agent-runtime-config
git commit -m "feat(runtime-config): JSONL TraceWriter + ObservedSink + trace config (on by default)"
```

---

### Task 6: Wire ObservedSink into assemble_loop + both frontends

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (LoopParts, assemble_loop ~79-139, the test LoopParts constructor ~197-211)
- Modify: `agent/crates/agent-cli/src/main.rs` (LoopParts construction ~213-226)
- Modify: `agent/crates/agent-server/src/runtime.rs` (RuntimeState::new + build_loop + new getter)
- Modify: any other LoopParts construction sites the compiler flags (`agent-runtime-config/tests/*.rs`)
- Test: new test in `assemble.rs`

**Interfaces:**
- Consumes: `SessionStats` (Task 4), `ObservedSink`/`TraceWriter`/`build_trace` (Task 5).
- Produces:
  - `LoopParts` gains `pub stats: Arc<std::sync::RwLock<SessionStats>>` and `pub trace: Option<Arc<TraceWriter>>` (both caller-owned so they survive server loop rebuilds, same pattern as `offload_store`).
  - `RuntimeState::stats(&self) -> Arc<RwLock<SessionStats>>` getter (Task 8 uses it).

- [ ] **Step 1: Write the failing test** in `assemble.rs`'s test module (reuse its existing `LoopParts` test constructor):

```rust
#[test]
fn assemble_wires_stats_through_observed_sink() {
    // Build a loop whose parts carry a fresh stats handle and a CollectingSink,
    // then emit through the loop's sink is not directly reachable — instead
    // assert at the unit level: construct ObservedSink over the parts and emit.
    let stats = Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default()));
    let inner = Arc::new(agent_core::testkit::CollectingSink::default());
    let sink = crate::trace::ObservedSink { inner: inner.clone(), stats: stats.clone(), trace: None };
    use agent_core::EventSink;
    sink.emit(agent_core::AgentEvent::Error("x".into()));
    assert_eq!(stats.read().unwrap().errors, 1);          // folded
    assert_eq!(inner.events.lock().unwrap().len(), 1);    // forwarded
}
```

(The end-to-end proof that `assemble_loop` installs the wrapper is the compile-enforced field + the CLI/server wiring below; the deterministic e2e suites exercise it in Task 8's step 6.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-runtime-config assemble_wires_stats`
Expected: FAIL — `LoopParts` has no field `stats` / ObservedSink import path errors.

- [ ] **Step 3: Implement.** `assemble.rs`:

(a) `LoopParts` — add after `compact_flag`:

```rust
/// Session-stable stats handle; caller-owned (survives server loop rebuilds).
pub stats: Arc<std::sync::RwLock<agent_core::SessionStats>>,
/// Session-stable trace writer; None = tracing disabled. Caller-owned.
pub trace: Option<Arc<crate::trace::TraceWriter>>,
```

(b) In `assemble_loop`, wrap the sink before `AgentLoop::new`:

```rust
let sink: Arc<dyn EventSink> = Arc::new(crate::trace::ObservedSink {
    inner: parts.sink.clone(),
    stats: parts.stats.clone(),
    trace: parts.trace.clone(),
});
```

and pass `sink` (instead of `parts.sink`) as the sink argument at line ~137.

(c) `agent-cli/src/main.rs` — before the `assemble_loop` call:

```rust
let stats = Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default()));
let trace = agent_runtime_config::build_trace(&rt);
if let Some(t) = &trace {
    eprintln!("\x1b[2mtrace: ~/.agent/sessions/{}.jsonl\x1b[0m", t.session_id());
}
```

and add `stats: stats.clone(), trace,` to the `LoopParts` literal. Keep the `stats` handle in scope — Task 7 prints from it.

(d) `agent-server/src/runtime.rs` — `RuntimeState` gains fields

```rust
stats: Arc<std::sync::RwLock<agent_core::SessionStats>>,
trace: Option<Arc<agent_runtime_config::TraceWriter>>,
```

created once in `RuntimeState::new` (`Arc::default()` for stats; `build_trace(&config)` for trace) and passed into every `build_loop` call (initial + rebuild), plus a getter:

```rust
pub fn stats(&self) -> Arc<std::sync::RwLock<agent_core::SessionStats>> { self.stats.clone() }
```

(e) Fix remaining `LoopParts` literals the compiler flags (tests): `stats: Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default())), trace: None`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p agent-runtime-config && cargo test -p agent-cli && cargo test -p agent-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A agent/crates
git commit -m "feat(runtime-config): install ObservedSink (stats+trace) in assemble_loop; wire CLI and server"
```

---

### Task 7: CLI rendering — context notices + session stats line

**Files:**
- Modify: `agent/crates/agent-cli/src/render.rs` (the `AgentEvent::Context(_) => {}` drop arm; new `print_stats_line` helper + tests)
- Modify: `agent/crates/agent-cli/src/main.rs` (REPL loop ~240-262: print stats after each run)
- Test: `mod tests` in `render.rs`

**Interfaces:**
- Consumes: `ContextEvent` (existing), `SessionStats` (Task 4), the CLI `stats` handle (Task 6c).
- Produces: `pub fn format_stats_line(s: &SessionStats) -> String` (pure, testable).

- [ ] **Step 1: Write the failing tests** in `render.rs`:

```rust
#[test]
fn stats_line_summarizes_session() {
    let mut s = agent_core::SessionStats::default();
    s.turns = 3; s.prompt_tokens = 12_400; s.completion_tokens = 2_100;
    s.tool_calls = 7; s.tools_error = 1; s.tools_timeout = 1;
    s.tool_time_ms = 4_200; s.cost_usd = 0.05;
    let line = format_stats_line(&s);
    assert!(line.contains("3 turns"));
    assert!(line.contains("12.4k in"));
    assert!(line.contains("2.1k out"));
    assert!(line.contains("7 tools"));
    assert!(line.contains("2 failed"));
    assert!(line.contains("$0.05"));
}

#[test]
fn stats_line_omits_zero_cost() {
    let s = agent_core::SessionStats::default();
    assert!(!format_stats_line(&s).contains('$'));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-cli stats_line`
Expected: FAIL — `format_stats_line` not found.

- [ ] **Step 3: Implement.** In `render.rs`:

```rust
fn fmt_k(n: u64) -> String {
    if n >= 1000 { format!("{:.1}k", n as f64 / 1000.0) } else { n.to_string() }
}

/// One dim summary line printed after each run (pure for testability).
pub fn format_stats_line(s: &agent_core::SessionStats) -> String {
    let failed = s.tools_denied + s.tools_error + s.tools_timeout + s.tools_panic;
    let mut line = format!(
        "— session: {} turns · {} in / {} out tokens · {} tools ({} failed) · {:.1}s in tools",
        s.turns, fmt_k(s.prompt_tokens), fmt_k(s.completion_tokens),
        s.tool_calls, failed, s.tool_time_ms as f64 / 1000.0);
    if s.cost_usd > 0.0 { line.push_str(&format!(" · ${:.2}", s.cost_usd)); }
    line
}
```

Replace the `AgentEvent::Context(_) => {}` arm in `TerminalSink::emit`:

```rust
AgentEvent::Context(c) => {
    use agent_core::ContextEvent as CE;
    let note = match c {
        CE::Offloaded { id, bytes, tool } =>
            format!("⟲ offloaded {tool} result #{id} ({} KB)", bytes / 1024),
        CE::Compacted { turns_replaced, tokens_before, tokens_after } =>
            format!("⟲ compacted {turns_replaced} turns: {tokens_before} → {tokens_after} tokens"),
        CE::CompactionFailed { reason } => format!("⚠ compaction failed: {reason}"),
    };
    let _ = writeln!(out, "\x1b[2m{note}\x1b[0m");
}
```

In `main.rs`, after each run result is handled (right after the `if let Err(e) = result` block inside the REPL loop):

```rust
if let Ok(s) = stats.read() {
    eprintln!("\x1b[2m{}\x1b[0m", render::format_stats_line(&s));
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agent-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-cli/src
git commit -m "feat(cli): render context-curation notices + per-session stats summary line"
```

---

### Task 8: Server — Context + SessionStats on the wire, push-on-Done, stats query

**Files:**
- Modify: `agent/crates/agent-server/src/wire.rs` (new ServerEvent variants + forwarding + tests)
- Modify: `agent/crates/agent-server/src/session.rs` (push-on-Done in `send_input`; `session_stats()` method)
- Modify: `/home/kalen/rust-agent-runtime/src-tauri/src/lib.rs` (new `session_stats` Tauri command — **separate workspace**: build with `cargo check` from `src-tauri/`)
- Test: `wire.rs` tests + `session.rs` tests

**Interfaces:**
- Consumes: `ContextEvent`, `SessionStats` (Serialize+Deserialize, Task 4), `RuntimeState::stats()` (Task 6d).
- Produces:
  - `ServerEvent::Context { kind: String, detail: serde_json::Value }`
  - `ServerEvent::SessionStats { stats: agent_core::SessionStats }`
  - `Session::session_stats(&self) -> agent_core::SessionStats`

- [ ] **Step 1: Write the failing tests** in `wire.rs`'s test module:

```rust
#[test]
fn context_events_are_forwarded() {
    use agent_core::ContextEvent;
    for (ev, kind) in [
        (ContextEvent::Offloaded { id: 4, bytes: 2048, tool: "read_file".into() }, "offloaded"),
        (ContextEvent::Compacted { turns_replaced: 3, tokens_before: 900, tokens_after: 200 }, "compacted"),
        (ContextEvent::CompactionFailed { reason: "model err".into() }, "compaction_failed"),
    ] {
        let out = server_event_from(AgentEvent::Context(ev)).expect("must forward");
        let j = serde_json::to_value(&out).unwrap();
        assert_eq!(j["type"], "context");
        assert_eq!(j["kind"], kind);
    }
}

#[test]
fn tool_result_wire_carries_status_and_duration() {
    let out = server_event_from(AgentEvent::ToolResult {
        id: "c1".into(), name: "t".into(), status: agent_core::ToolStatus::Timeout,
        output: agent_tools::ToolOutput { content: "e".into(), display: None },
        duration_ms: 60000 }).unwrap();
    let j = serde_json::to_value(&out).unwrap();
    assert_eq!(j["type"], "tool_result");
    assert_eq!(j["id"], "c1");
    assert_eq!(j["status"], "timeout");
    assert_eq!(j["duration_ms"], 60000);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-server context_events_are_forwarded`
Expected: FAIL — `Context(_) => return None` still drops the event. (`tool_result_wire_carries_status_and_duration` passes already from Task 1 — keep it as the pinning test.)

- [ ] **Step 3: Implement.** `wire.rs`: add variants

```rust
Context { kind: String, detail: serde_json::Value },
SessionStats { stats: agent_core::SessionStats },
```

and replace the drop arm in `server_event_from`:

```rust
AgentEvent::Context(c) => {
    use agent_core::ContextEvent as CE;
    let (kind, detail) = match c {
        CE::Offloaded { id, bytes, tool } =>
            ("offloaded", serde_json::json!({"id": id, "bytes": bytes, "tool": tool})),
        CE::Compacted { turns_replaced, tokens_before, tokens_after } =>
            ("compacted", serde_json::json!({"turns_replaced": turns_replaced,
                "tokens_before": tokens_before, "tokens_after": tokens_after})),
        CE::CompactionFailed { reason } =>
            ("compaction_failed", serde_json::json!({"reason": reason})),
    };
    ServerEvent::Context { kind: kind.into(), detail }
}
```

`session.rs`: add the method + the push. In `send_input`'s spawned task, after the run completes (before clearing `active_slot`):

```rust
// Push a final stats snapshot so an attached client needs no poll.
let snapshot = runtime.stats().read().map(|s| s.clone()).unwrap_or_default();
if let Some(out) = slot.lock().unwrap().clone() {
    out.send(crate::wire::ServerEvent::SessionStats { stats: snapshot });
}
```

(clone `self.slot` and `self.runtime` into the task alongside the existing captures), and:

```rust
pub fn session_stats(&self) -> agent_core::SessionStats {
    self.runtime.stats().read().map(|s| s.clone()).unwrap_or_default()
}
```

`src-tauri/src/lib.rs`: locate the existing `settings_get` command and add a sibling with the identical shape (adapt the state accessor to match its neighbors exactly):

```rust
#[tauri::command]
fn session_stats(state: tauri::State<'_, AppState>) -> agent_core::SessionStats {
    session(&state).session_stats()
}
```

and register `session_stats` in the `invoke_handler![…]` list beside `settings_get`.

- [ ] **Step 4: Add a session test** in `session.rs` tests (mirror the existing `settings_get_returns_state` construction):

```rust
#[tokio::test]
async fn session_stats_starts_at_default() {
    let sess = /* same constructor the existing tests use */;
    assert_eq!(sess.session_stats(), agent_core::SessionStats::default());
}
```

- [ ] **Step 5: Run tests + check both workspaces**

Run: `cargo test -p agent-server && (cd /home/kalen/rust-agent-runtime/src-tauri && cargo check)`
Expected: PASS / clean check.

- [ ] **Step 6: Extend the deterministic e2e suite** — in `agent/crates/agent-runtime-config/tests/e2e_robustness.rs`, add to an existing denied-tool scenario (they use `CollectingSink` labels):

```rust
// Cluster-2 pinning: a denied call must surface a terminal tool_result with its status.
assert!(events.iter().any(|e| e.starts_with("tool_result:") && e.ends_with(":denied")),
    "denied call must emit a terminal ToolResult event, got: {events:?}");
```

Run: `cargo test -p agent-runtime-config --test e2e_robustness` — PASS.

- [ ] **Step 7: Commit**

```bash
git add -A agent/crates /home/kalen/rust-agent-runtime/src-tauri/src
git commit -m "feat(server): forward Context events, push SessionStats on Done, add session_stats query"
```

---

### Task 9: Web — wire types, reducer, context markers, stats panel

**Files:**
- Modify: `web/src/wire.ts` (WireEvent union, new SessionStats interface, RuntimeSettings trace fields)
- Modify: `web/src/state.ts` (Item union, ConversationState, reducer cases)
- Create: `web/src/components/StatsPanel.tsx`
- Modify: `web/src/components/ContextDashboard.tsx` (mount StatsPanel below existing sections)
- Modify: `web/src/components/ToolCall.tsx` (failure badge — adapt to the component's existing markup)
- Test: `web/src/state.stats.test.ts` (new), extend component tests if patterns exist

**Interfaces:**
- Consumes: wire JSON from Task 8 (`context`, `session_stats`, enriched `tool_result`/`server_usage`).
- Produces TS types (exact):

```ts
export interface SessionStats {
  turns: number; prompt_tokens: number; completion_tokens: number;
  reasoning_tokens: number; cached_tokens: number; cost_usd: number;
  tool_calls: number; tools_ok: number; tools_denied: number; tools_error: number;
  tools_timeout: number; tools_panic: number; tool_time_ms: number;
  turn_time_ms: number; context_events: number; errors: number;
}
```

- [ ] **Step 1: Write the failing reducer test** `web/src/state.stats.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { initialState, reduce } from "./state";
import type { Inbound } from "./wire";

const frame = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);

describe("cluster-2 observability frames", () => {
  it("stores session_stats", () => {
    const stats = { turns: 2, prompt_tokens: 300, completion_tokens: 90,
      reasoning_tokens: 10, cached_tokens: 60, cost_usd: 0.05, tool_calls: 3,
      tools_ok: 2, tools_denied: 0, tools_error: 1, tools_timeout: 0, tools_panic: 0,
      tool_time_ms: 900, turn_time_ms: 1200, context_events: 1, errors: 1 };
    const s = reduce(initialState([]), { type: "frame",
      frame: frame({ type: "session_stats", stats }) });
    expect(s.stats).toEqual(stats);
  });

  it("appends a context marker item", () => {
    const s = reduce(initialState([]), { type: "frame",
      frame: frame({ type: "context", kind: "compacted",
        detail: { turns_replaced: 3, tokens_before: 900, tokens_after: 200 } }) });
    const last = s.items[s.items.length - 1];
    expect(last.kind).toBe("context");
    expect((last as { text: string }).text).toContain("compacted 3 turns");
  });

  it("marks a failed tool result with status and duration", () => {
    let s = reduce(initialState([]), { type: "frame",
      frame: frame({ type: "tool_start", id: "c1", name: "read_file", args: {} }) });
    s = reduce(s, { type: "frame",
      frame: frame({ type: "tool_result", id: "c1", name: "read_file",
        status: "timeout", duration_ms: 60000, content: "ERROR: …" }) });
    const tool = s.items.find((i) => i.kind === "tool") as
      { resultStatus?: string; durationMs?: number };
    expect(tool.resultStatus).toBe("timeout");
    expect(tool.durationMs).toBe(60000);
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd web && npx vitest run src/state.stats.test.ts`
Expected: FAIL — `stats` not on state; `context`/`session_stats` cases missing.

- [ ] **Step 3: Implement.**

`wire.ts` — extend the `WireEvent` union entries:

```ts
| { type: "server_usage"; prompt_tokens: number; completion_tokens: number; turn: number;
    reasoning_tokens?: number; cached_tokens?: number; cost_usd?: number; turn_duration_ms?: number }
| { type: "tool_start"; id: string; name: string; args: unknown }
| { type: "tool_result"; id: string; name: string; status: string; duration_ms: number;
    content: string; display?: Display }
| { type: "context"; kind: string; detail: Record<string, unknown> }
| { type: "session_stats"; stats: SessionStats }
```

add the `SessionStats` interface (above), and add to `RuntimeSettings` (pass-through so settings round-trips don't drop them):

```ts
trace: boolean;
trace_dir: string | null;
trace_max_mb: number;
```

`state.ts`:

```ts
// Item union additions:
| { kind: "tool"; name: string; args: unknown; status: "running" | "done";
    content?: string; display?: Display; resultStatus?: string; durationMs?: number }
| { kind: "context"; text: string }

// ConversationState addition:
stats: SessionStats | null;   // + `stats: null` in initialState

// helper:
function describeContext(kind: string, detail: Record<string, unknown>): string {
  switch (kind) {
    case "offloaded": return `offloaded ${detail.tool} result #${detail.id}`;
    case "compacted":
      return `compacted ${detail.turns_replaced} turns: ${detail.tokens_before} → ${detail.tokens_after} tokens`;
    case "compaction_failed": return `compaction failed: ${detail.reason}`;
    default: return kind;
  }
}

// reducer cases (inside the payload switch):
case "context":
  return { ...s, items: [...s.items, { kind: "context", text: describeContext(p.kind, p.detail) }] };
case "session_stats":
  return { ...s, stats: p.stats };
// case "tool_result": extend the existing item update with
//   resultStatus: p.status, durationMs: p.duration_ms
```

`StatsPanel.tsx` (complete):

```tsx
import type { SessionStats } from "../wire";

const k = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));

export function StatsPanel({ stats }: { stats: SessionStats | null }) {
  if (!stats) return null;
  const failed = stats.tools_denied + stats.tools_error + stats.tools_timeout + stats.tools_panic;
  const rows: Array<[string, string]> = [
    ["Turns", String(stats.turns)],
    ["Tokens in / out", `${k(stats.prompt_tokens)} / ${k(stats.completion_tokens)}`],
    ["Reasoning / cached", `${k(stats.reasoning_tokens)} / ${k(stats.cached_tokens)}`],
    ["Tool calls", `${stats.tool_calls} (${failed} failed)`],
    ["Time in tools", `${(stats.tool_time_ms / 1000).toFixed(1)}s`],
    ["Model time", `${(stats.turn_time_ms / 1000).toFixed(1)}s`],
    ["Context events", String(stats.context_events)],
  ];
  if (stats.cost_usd > 0) rows.push(["Cost", `$${stats.cost_usd.toFixed(4)}`]);
  return (
    <section aria-label="Session stats" className="space-y-1 text-sm">
      <h3 className="font-medium">Session stats</h3>
      <dl className="grid grid-cols-2 gap-x-3 gap-y-0.5">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="text-right tabular-nums">{value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}
```

Add a render test `web/src/components/StatsPanel.test.tsx` (mirror the setup of the existing `SandboxBanner.test.tsx`):

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StatsPanel } from "./StatsPanel";

describe("StatsPanel", () => {
  it("renders nothing without stats", () => {
    const { container } = render(<StatsPanel stats={null} />);
    expect(container.firstChild).toBeNull();
  });
  it("shows failure count and cost", () => {
    render(<StatsPanel stats={{ turns: 2, prompt_tokens: 300, completion_tokens: 90,
      reasoning_tokens: 0, cached_tokens: 0, cost_usd: 0.05, tool_calls: 3,
      tools_ok: 2, tools_denied: 0, tools_error: 1, tools_timeout: 0, tools_panic: 0,
      tool_time_ms: 900, turn_time_ms: 1200, context_events: 1, errors: 1 }} />);
    expect(screen.getByText("3 (1 failed)")).toBeInTheDocument();
    expect(screen.getByText("$0.0500")).toBeInTheDocument();
  });
});
```

Mount `<StatsPanel stats={state.stats} />` in `ContextDashboard.tsx` below its existing sections (match the file's section styling); render `items` of `kind: "context"` in `MessageList.tsx` as a dim one-line marker (follow how other item kinds are dispatched there); in `ToolCall.tsx`, when `resultStatus && resultStatus !== "ok"`, show a small `{resultStatus} · {durationMs}ms` badge styled like existing error affordances.

- [ ] **Step 4: Run web checks**

Run: `cd web && npm run typecheck && npx vitest run`
Expected: PASS (fix any existing tests that assert the old `tool_result` payload shape — add the new required fields to their fixtures).

- [ ] **Step 5: Commit**

```bash
git add web/src
git commit -m "feat(web): context markers, enriched tool results, and session stats panel"
```

---

### Task 10: CI gate — ci.sh, pre-push hook, GitHub Actions

**Files:**
- Create: `scripts/ci.sh` (repo root)
- Create: `.githooks/pre-push`
- Create: `.github/workflows/ci.yml`
- Modify: `CLAUDE.md` (Commands section: hook setup + trace location note)

**Interfaces:** none consumed; produces the gate every future change runs through.

- [ ] **Step 1: Create `scripts/ci.sh`:**

```bash
#!/usr/bin/env bash
# Single source of truth for the CI gate — run by .githooks/pre-push and
# .github/workflows/ci.yml. src-tauri is intentionally excluded (GTK deps).
set -euo pipefail
cd "$(dirname "$0")/.."

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

echo "==> cargo fmt --check"
(cd agent && cargo fmt --all --check)

echo "==> cargo clippy -D warnings"
(cd agent && cargo clippy --workspace --all-targets -- -D warnings)

echo "==> cargo test"
(cd agent && cargo test --workspace)

echo "==> web typecheck + tests"
(cd web && npm ci --no-audit --no-fund && npm run typecheck && npx vitest run)

echo "CI gate passed."
```

- [ ] **Step 2: Create `.githooks/pre-push`:**

```bash
#!/usr/bin/env bash
exec bash "$(git rev-parse --show-toplevel)/scripts/ci.sh"
```

Then: `chmod +x scripts/ci.sh .githooks/pre-push && git config core.hooksPath .githooks`

- [ ] **Step 3: Create `.github/workflows/ci.yml`:**

```yaml
name: CI
on:
  push:
    branches: [main]
  pull_request:
jobs:
  gate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: agent
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: npm
          cache-dependency-path: web/package-lock.json
      - run: bash scripts/ci.sh
```

- [ ] **Step 4: Document in `CLAUDE.md`** — add to the Commands section:

```markdown
CI gate (also runs as a pre-push hook — enable once per clone with
`git config core.hooksPath .githooks`):

    bash scripts/ci.sh   # fmt + clippy + cargo test (agent/) + web typecheck/vitest

Session traces land in `~/.agent/sessions/<id>.jsonl` (disable with `"trace": false`
in the runtime config).
```

- [ ] **Step 5: Run the gate for real**

Run: `bash scripts/ci.sh`
Expected: "CI gate passed." — if clippy `-D warnings` flags pre-existing lints in touched files, fix them; if it flags unrelated legacy code, add targeted `#[allow]`s with a comment rather than expanding scope.

- [ ] **Step 6: Commit**

```bash
git add scripts/ci.sh .githooks/pre-push .github/workflows/ci.yml CLAUDE.md
git commit -m "ci: add local pre-push gate + GitHub Actions workflow (fmt, clippy, tests, web)"
```

---

### Task 11: Full-suite verification + spec cross-check

**Files:** none new.

- [ ] **Step 1:** `cd agent && cargo test --workspace` — PASS.
- [ ] **Step 2:** `cd web && npm run typecheck && npx vitest run` — PASS.
- [ ] **Step 3:** `cd /home/kalen/rust-agent-runtime/src-tauri && cargo check` — clean.
- [ ] **Step 4:** Manual smoke: run the CLI against any configured backend for one short task; confirm (a) the trace file appears under `~/.agent/sessions/` and every line parses as JSON, (b) a denied command shows the red `✗ name (denied, 0ms)` line, (c) the dim stats line prints after the run.
- [ ] **Step 5:** Open the spec (`docs/superpowers/specs/2026-07-01-harness-observability-ci-design.md`) and tick each Testing-section test name against an implemented test; any miss is a gap to close before review.
- [ ] **Step 6: Commit** any smoke-test fixes: `fix(scope): <what the smoke test caught>`.
