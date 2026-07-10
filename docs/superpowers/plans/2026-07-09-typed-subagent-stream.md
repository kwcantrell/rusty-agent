# Typed Sub-agent Stream (3B-2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A typed, per-delegation sub-agent event stream (`AgentEvent::Subagent`) — lifecycle + live nested child text/reasoning — rendered as a live card in the web SPA and start/end lifecycle lines in the CLI.

**Architecture:** New `AgentEvent::Subagent(SubagentEvent)` family emitted only by `dispatch.rs` (`execute()` emits `Start` + a drop-guard-guaranteed `End`; `SubagentSink` forwards child deltas while still capturing). Five additive wire frames; the web reducer enriches the existing `dispatch_agent` tool item into a card; trace layer records Start/End and skips deltas (raw child tap remains the single transcript source).

**Tech Stack:** Rust (two crates workspaces — all Rust work is in `agent/`), React 19 + TypeScript + Vitest (`web/`), WebDriver live drive via the `auto-drive-tauri` skill.

**Spec:** `docs/superpowers/specs/2026-07-09-typed-subagent-stream-design.md` (panel-reviewed, gate G1–G5 decided). Read §2–§5 before starting.

## Global Constraints

- **Byte-compat:** every pre-existing wire frame, the `sub:`/`sub{n}:` renaming, `parent_id` tagging, and None-omission serialization are untouched. New frames are additive only (spec §3.1).
- **No loop changes:** `loop_.rs` is not edited. No calibration / estimator / pinned-block / CuratedContext touch (spec §3.2, §3.3).
- **Trace exactly-once:** typed `Text`/`Reasoning`/`StreamRetry` never produce trace records; the raw `ChildTraceTap` path stays the only child-transcript source. `Start`/`End` DO get trace records (spec §2.6).
- **`CaptureSummary` is the single stats truth** for `End` (spec §3.6). `stats.rs` counters must not move on `Subagent` events.
- **`AgentEvent` is a bare, derive-free enum** — never assume `AgentEvent: Clone`; construct fresh values at each emission site.
- **`file:line` anchors below go stale** the moment earlier tasks insert code. Anchors are orientation only; **locate quoted code by content before editing** — say so in any sub-agent prompt.
- **Two Cargo workspaces** — all `cargo` commands here run in `agent/` (`cd agent`). `ci.sh` runs from repo root.
- Conventional commits, `type(scope): summary`.

---

### Task 0: Branch

**Files:** none

- [ ] **Step 1:** `git checkout -b feature/typed-subagent-stream` from `main` (base must contain spec commit `7b1a20d`).

---

### Task A1: `SubagentEvent` + every compile-forced consumer arm (trace, testkit, wire, CLI)

Adding a variant to the derive-free `AgentEvent` compile-breaks four exhaustive matches (spec §2.6 inventory). This task lands the enum and ALL consumer arms in one commit so the workspace never has a broken intermediate state. Emission comes later (A2/A3) — after this task the new events exist but nothing emits them in production code.

**Files:**
- Modify: `agent/crates/agent-core/src/event.rs` (enum + variant)
- Modify: `agent/crates/agent-core/src/testkit.rs` (`CollectingSink` label arm)
- Modify: `agent/crates/agent-runtime-config/src/trace.rs` (`trace_event` → `Option`, two new `TraceEvent` variants, `write_record` skip)
- Modify: `agent/crates/agent-server/src/wire.rs` (five `ServerEvent` variants + mapping arm)
- Modify: `agent/crates/agent-cli/src/render.rs` (pure format helpers + `TerminalSink` arm)
- Tests: inline `#[cfg(test)]` mods of `trace.rs`, `wire.rs`, `render.rs`

**Interfaces:**
- Produces (later tasks rely on these exact names):
  - `agent_core::SubagentEvent` (re-exported via `pub use event::*`) with variants `Start { id: String, subagent_type: String, role: Option<String> }`, `Text { id: String, text: String }`, `Reasoning { id: String, text: String }`, `StreamRetry { id: String, discarded_text_chars: usize, discarded_reasoning_chars: usize }`, `End { id: String, outcome: SubagentOutcome, stop: Option<StopReason>, detail: Option<String>, turns: usize, tool_calls: u64, duration_ms: u64 }`
  - `agent_core::SubagentOutcome { Completed, Timeout, Failed, Cancelled }` with `pub fn as_str(&self) -> &'static str` returning `"completed" | "timeout" | "failed" | "cancelled"`
  - `AgentEvent::Subagent(SubagentEvent)`
  - Wire frames (snake_case type tags): `subagent_start`, `subagent_text`, `subagent_reasoning`, `subagent_stream_retry`, `subagent_end` with the §2.3 payloads

- [ ] **Step 1: Add the enum to `event.rs`** (below `ContextEvent`, above `ToolStatus`):

```rust
/// Typed per-delegation sub-agent stream (spec 2026-07-09 3B-2). Every case
/// carries the delegation id = the dispatching call's on-wire id — the exact
/// string forwarded child rows carry as `parent_id` — so frontends join the
/// typed stream to the existing `dispatch_agent` tool row, at any depth.
#[derive(Debug, Clone)]
pub enum SubagentEvent {
    /// One dispatch began. Emitted after all dispatch validation (a rejected
    /// dispatch never emits Start) and after the loop's ToolStart for the
    /// dispatch_agent call, so frontends always see the host row first.
    Start {
        id: String,
        /// Registry name, or "general-purpose" for the ad-hoc path.
        subagent_type: String,
        /// Per-call role arg (general-purpose only; None for named types).
        role: Option<String>,
    },
    /// Child assistant text delta.
    Text { id: String, text: String },
    /// Child reasoning delta. NOTE: a genuinely NEW egress path — child
    /// reasoning was trace-file-only before 3B-2. Streaming it to the local
    /// UI was gate-approved (spec G5): same trust boundary as the parent's
    /// own reasoning stream.
    Reasoning { id: String, text: String },
    /// Child mid-stream retry retracted in-flight deltas: frontends trim the
    /// tail of THIS delegation's transcript (mirrors top-level StreamRetry).
    StreamRetry {
        id: String,
        discarded_text_chars: usize,
        discarded_reasoning_chars: usize,
    },
    /// The delegation finished, on any path (drop-guard guaranteed: exactly
    /// one End per Start).
    End {
        id: String,
        outcome: SubagentOutcome,
        /// The child's own stop reason from the capture; None when the child
        /// never emitted Done (e.g. timeout before its first turn completed).
        stop: Option<StopReason>,
        /// Human-readable failure/timeout detail; None on Completed.
        detail: Option<String>,
        turns: usize,
        tool_calls: u64,
        duration_ms: u64,
    },
}

/// How a delegation terminated. Deliberately NOT derivable from `stop` +
/// tool-result status (Timeout and Failed both surface non-ok; Cancelled
/// returns Err) — dispatch's execute() is the only place that knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentOutcome {
    Completed,
    Timeout,
    Failed,
    Cancelled,
}

impl SubagentOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}
```

And add the variant to `AgentEvent` (after `RunStart`):

```rust
    /// Typed per-delegation sub-agent stream (3B-2). Emitted ONLY by
    /// dispatch.rs (execute() lifecycle + SubagentSink delta forwarding).
    Subagent(SubagentEvent),
```

- [ ] **Step 2: `testkit.rs` — add the `CollectingSink` label arm** (the match is exhaustive; locate the `AgentEvent::RunStart` arm and add after it):

```rust
            AgentEvent::Subagent(se) => match se {
                SubagentEvent::Start {
                    id, subagent_type, ..
                } => format!("subagent_start:{id}:{subagent_type}"),
                SubagentEvent::Text { id, text } => format!("subagent_text:{id}:{text}"),
                SubagentEvent::Reasoning { id, text } => {
                    format!("subagent_reasoning:{id}:{text}")
                }
                SubagentEvent::StreamRetry {
                    id,
                    discarded_text_chars,
                    ..
                } => format!("subagent_stream_retry:{id}:{discarded_text_chars}"),
                SubagentEvent::End { id, outcome, .. } => {
                    format!("subagent_end:{id}:{}", outcome.as_str())
                }
            },
```

Add `SubagentEvent` to the existing `use` of crate event types in testkit.rs (it currently imports `AgentEvent`/`ContextEvent` — locate by content).

- [ ] **Step 3: `trace.rs` — record Start/End, skip deltas.** Three edits:

(a) Two new `TraceEvent` variants (after `RunStart` in the enum):

```rust
    SubagentStart {
        id: &'a str,
        subagent_type: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        role: Option<&'a str>,
    },
    SubagentEnd {
        id: &'a str,
        outcome: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        stop: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<&'a str>,
        turns: usize,
        tool_calls: u64,
        duration_ms: u64,
    },
```

(b) Change `fn trace_event(e: &AgentEvent) -> TraceEvent<'_>` to return
`Option<TraceEvent<'_>>`: wrap the existing match body in `Some(match e { … })`
and add (before the closing) the new arm. For `stop`, reuse whatever mapping
the existing `AgentEvent::Done(r) => TraceEvent::Done { reason: … }` arm uses
(locate by content — there is an existing StopReason→&'static str path):

```rust
        AgentEvent::Subagent(se) => {
            use agent_core::SubagentEvent as SE;
            match se {
                SE::Start {
                    id,
                    subagent_type,
                    role,
                } => TraceEvent::SubagentStart {
                    id,
                    subagent_type,
                    role: role.as_deref(),
                },
                SE::End {
                    id,
                    outcome,
                    stop,
                    detail,
                    turns,
                    tool_calls,
                    duration_ms,
                } => TraceEvent::SubagentEnd {
                    id,
                    outcome: outcome.as_str(),
                    stop: stop.as_ref().map(|r| /* same helper the Done arm uses — locate by content */ stop_reason(r)),
                    detail: detail.as_deref(),
                    turns: *turns,
                    tool_calls: *tool_calls,
                    duration_ms: *duration_ms,
                },
                // (Bindings here are references — `trace_event` takes
                // `&AgentEvent` — hence `as_deref`/`as_ref` above; write the
                // stop mapping as `stop.as_ref().map(|r| <done-arm helper>(r))`.)
                // Typed deltas are UI telemetry; the raw ChildTraceTap records
                // remain the single trace source of the child transcript
                // (spec §2.6 exactly-once invariant).
                SE::Text { .. } | SE::Reasoning { .. } | SE::StreamRetry { .. } => return None,
            }
        }
```

(c) In `write_record`, replace `event: trace_event(event),` with an early
return on `None`:

```rust
        let Some(ev) = trace_event(event) else {
            return;
        };
```
…and use `event: ev` in the `TraceRecord`.

- [ ] **Step 4: `wire.rs` — five `ServerEvent` variants** (after `SessionStats` in the enum; the `#[serde(tag = "type", rename_all = "snake_case")]` container gives the snake_case tags):

```rust
    SubagentStart {
        id: String,
        subagent_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
    },
    SubagentText {
        id: String,
        text: String,
    },
    SubagentReasoning {
        id: String,
        text: String,
    },
    SubagentStreamRetry {
        id: String,
        discarded_text_chars: usize,
        discarded_reasoning_chars: usize,
    },
    SubagentEnd {
        id: String,
        outcome: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        turns: usize,
        tool_calls: u64,
        duration_ms: u64,
    },
```

And the mapping arm in `server_event_from` (after the `StreamRetry` arm):

```rust
        AgentEvent::Subagent(se) => {
            use agent_core::SubagentEvent as SE;
            match se {
                SE::Start {
                    id,
                    subagent_type,
                    role,
                } => ServerEvent::SubagentStart {
                    id,
                    subagent_type,
                    role,
                },
                SE::Text { id, text } => ServerEvent::SubagentText { id, text },
                SE::Reasoning { id, text } => ServerEvent::SubagentReasoning { id, text },
                SE::StreamRetry {
                    id,
                    discarded_text_chars,
                    discarded_reasoning_chars,
                } => ServerEvent::SubagentStreamRetry {
                    id,
                    discarded_text_chars,
                    discarded_reasoning_chars,
                },
                SE::End {
                    id,
                    outcome,
                    stop,
                    detail,
                    turns,
                    tool_calls,
                    duration_ms,
                } => ServerEvent::SubagentEnd {
                    id,
                    outcome: outcome.as_str().into(),
                    stop: stop.map(|r| stop_reason_str(&r).to_string()),
                    detail,
                    turns,
                    tool_calls,
                    duration_ms,
                },
            }
        }
```

- [ ] **Step 5: `render.rs` — pure helpers + `TerminalSink` arm.** Add pure functions next to `format_tool_start` (same style, testable):

```rust
fn format_subagent_start(subagent_type: &str, role: Option<&str>) -> String {
    let role_note = role.map(|r| format!(" — {r}")).unwrap_or_default();
    format!("  ↳ \x1b[36magent[{subagent_type}]\x1b[0m started{role_note}")
}

fn format_subagent_end(
    outcome: agent_core::SubagentOutcome,
    stop: Option<&str>,
    detail: Option<&str>,
    turns: usize,
    tool_calls: u64,
    duration_ms: u64,
) -> String {
    use agent_core::SubagentOutcome as O;
    let word = match outcome {
        O::Completed => "done",
        O::Timeout => "timed out",
        O::Failed => "failed",
        O::Cancelled => "cancelled",
    };
    let stop_note = stop.map(|s| format!("{s}, ")).unwrap_or_default();
    let detail_note = detail.map(|d| format!(" — {d}")).unwrap_or_default();
    let secs = duration_ms as f64 / 1000.0;
    format!("  ↳ agent {word} — {stop_note}{turns} turns, {tool_calls} tools, {secs:.1}s{detail_note}")
}
```

`TerminalSink::emit` arm (the match is exhaustive; add after the `Done` arm).
For `End`'s `stop` string reuse the wire convention: map via
`format!("{r:?}").to_lowercase()` is WRONG (BudgetExhausted ≠ budget_exhausted)
— add a tiny local `fn stop_str(r: &StopReason) -> &'static str` mirroring
`wire.rs::stop_reason_str` (six variants), or match inline:

```rust
            AgentEvent::Subagent(se) => {
                use agent_core::SubagentEvent as SE;
                match se {
                    SE::Start {
                        subagent_type, role, ..
                    } => {
                        let _ = writeln!(
                            out,
                            "\n{}",
                            format_subagent_start(&subagent_type, role.as_deref())
                        );
                    }
                    SE::End {
                        outcome,
                        stop,
                        detail,
                        turns,
                        tool_calls,
                        duration_ms,
                        ..
                    } => {
                        let _ = writeln!(
                            out,
                            "{}",
                            format_subagent_end(
                                outcome,
                                stop.map(|r| stop_str(&r)),
                                detail.as_deref(),
                                turns,
                                tool_calls,
                                duration_ms
                            )
                        );
                    }
                    // Live child prose is terminal noise (spec §2.5, owner
                    // decision) — lifecycle lines only.
                    SE::Text { .. } | SE::Reasoning { .. } | SE::StreamRetry { .. } => {}
                }
            }
```

- [ ] **Step 6: Write the tests.**

`wire.rs` tests (same module as `token_serializes_with_type_tag`):

```rust
    #[test]
    fn subagent_frames_serialize_with_type_tags_and_omit_none_optionals() {
        use agent_core::{SubagentEvent, SubagentOutcome};
        let start = server_event_from(AgentEvent::Subagent(SubagentEvent::Start {
            id: "c7".into(),
            subagent_type: "general-purpose".into(),
            role: None,
        }))
        .unwrap();
        let j = serde_json::to_string(&start).unwrap();
        assert!(j.contains(r#""type":"subagent_start""#), "{j}");
        assert!(j.contains(r#""id":"c7""#), "{j}");
        assert!(!j.contains("role"), "None role must be omitted: {j}");

        let end = server_event_from(AgentEvent::Subagent(SubagentEvent::End {
            id: "c7".into(),
            outcome: SubagentOutcome::Timeout,
            stop: None,
            detail: Some("sub-agent timed out after 5s".into()),
            turns: 2,
            tool_calls: 3,
            duration_ms: 5000,
        }))
        .unwrap();
        let j = serde_json::to_string(&end).unwrap();
        assert!(j.contains(r#""type":"subagent_end""#), "{j}");
        assert!(j.contains(r#""outcome":"timeout""#), "{j}");
        assert!(!j.contains(r#""stop""#), "None stop must be omitted: {j}");
        assert!(j.contains(r#""detail":"sub-agent timed out after 5s""#), "{j}");
        assert!(j.contains(r#""turns":2"#) && j.contains(r#""tool_calls":3"#), "{j}");

        let text = server_event_from(AgentEvent::Subagent(SubagentEvent::Text {
            id: "c7".into(),
            text: "hi".into(),
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_string(&text).unwrap(),
            r#"{"type":"subagent_text","id":"c7","text":"hi"}"#
        );

        let stop_end = server_event_from(AgentEvent::Subagent(SubagentEvent::End {
            id: "c7".into(),
            outcome: SubagentOutcome::Completed,
            stop: Some(StopReason::BudgetExhausted),
            detail: None,
            turns: 4,
            tool_calls: 0,
            duration_ms: 10,
        }))
        .unwrap();
        let j = serde_json::to_string(&stop_end).unwrap();
        assert!(j.contains(r#""stop":"budget_exhausted""#), "{j}");
        assert!(!j.contains("detail"), "{j}");
    }
```

`trace.rs` tests (same module as the existing `record_child_lines_…` test — copy its TraceWriter/tempfile read pattern by content):

```rust
    #[test]
    fn subagent_lifecycle_traced_and_deltas_skipped() {
        // Build a TraceWriter into a temp dir exactly as the existing
        // record_child test does (locate by content), then:
        use agent_core::{AgentEvent, SubagentEvent, SubagentOutcome};
        writer.record(&AgentEvent::Subagent(SubagentEvent::Start {
            id: "c1".into(),
            subagent_type: "researcher".into(),
            role: None,
        }));
        writer.record(&AgentEvent::Subagent(SubagentEvent::Text {
            id: "c1".into(),
            text: "delta".into(),
        }));
        writer.record(&AgentEvent::Subagent(SubagentEvent::Reasoning {
            id: "c1".into(),
            text: "rdelta".into(),
        }));
        writer.record(&AgentEvent::Subagent(SubagentEvent::StreamRetry {
            id: "c1".into(),
            discarded_text_chars: 1,
            discarded_reasoning_chars: 0,
        }));
        writer.record(&AgentEvent::Subagent(SubagentEvent::End {
            id: "c1".into(),
            outcome: SubagentOutcome::Completed,
            stop: None,
            detail: None,
            turns: 1,
            tool_calls: 0,
            duration_ms: 5,
        }));
        // Force a flush the way the existing test does (e.g. record a Done).
        let contents = /* read the trace file as the existing test does */;
        assert!(contents.contains(r#""type":"subagent_start""#));
        assert!(contents.contains(r#""type":"subagent_end""#));
        assert!(!contents.contains("subagent_text"), "deltas must be skipped");
        assert!(!contents.contains("subagent_reasoning"));
        assert!(!contents.contains("subagent_stream_retry"));
        assert!(!contents.contains("delta"), "delta payloads must not land");
    }
```

`render.rs` tests (next to existing format fn tests):

```rust
    #[test]
    fn subagent_lifecycle_lines() {
        assert!(format_subagent_start("researcher", None).contains("agent[researcher]"));
        assert!(format_subagent_start("general-purpose", Some("be brief"))
            .contains("— be brief"));
        let end = format_subagent_end(
            agent_core::SubagentOutcome::Timeout,
            None,
            Some("sub-agent timed out after 5s"),
            2,
            3,
            5000,
        );
        assert!(end.contains("timed out"), "{end}");
        assert!(end.contains("2 turns, 3 tools, 5.0s"), "{end}");
        assert!(end.contains("— sub-agent timed out after 5s"), "{end}");
        let done = format_subagent_end(
            agent_core::SubagentOutcome::Completed,
            Some("stop"),
            None,
            4,
            7,
            12300,
        );
        assert!(done.contains("done — stop, 4 turns, 7 tools, 12.3s"), "{done}");
    }
```

- [ ] **Step 7: Run:** `cd agent && cargo test -p agent-core -p agent-runtime-config -p agent-server -p agent-cli`
Expected: PASS (workspace compiles; new tests green; no existing test touched yet — nothing emits the variant in production code).

- [ ] **Step 8: Commit:** `git add -A && git commit -m "feat(core): SubagentEvent typed family + trace/wire/CLI/testkit consumer arms (3B-2 A1)"`

---

### Task A2: dispatch `Start` + drop-guard `End` (all exit paths)

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs` (execute() ~L686 region + new `EndGuard` near `SubagentSink`; tests in the same file + `agent/crates/agent-core/tests/dispatch_tool.rs`)

**Interfaces:**
- Consumes: `SubagentEvent`/`SubagentOutcome` from A1.
- Produces: `Start` emitted once per accepted dispatch with `id` = the delegation id (the local currently named `parent_id`, `format!("{id_prefix}{call_id}")`); exactly one `End` per `Start` on every termination (drop-guard). Existing dispatch tests using the local `FullSink` will now ALSO receive `Subagent` events — extend `FullSink` (Step 3) rather than filtering.

- [ ] **Step 1: Add `EndGuard`** below the `SubagentSink` impl in dispatch.rs:

```rust
/// RAII guarantee of exactly one `SubagentEvent::End` per `Start` (spec §2.2).
/// The four ordinary exit paths call `finish` with a precise outcome; if the
/// dispatch future panics inside the child run (caught upstream by
/// execute_isolated's catch_unwind) or is dropped by the loop's backstop
/// timeout, `Drop` emits a Failed End so a frontend card can never spin
/// forever. Stats come from the sink's CaptureSummary — the same source the
/// tool-result footer uses (spec §3.6).
struct EndGuard {
    sink: Arc<SubagentSink>,
    out: Arc<dyn EventSink>,
    id: String,
    started: std::time::Instant,
    armed: bool,
}

impl EndGuard {
    fn new(sink: Arc<SubagentSink>, out: Arc<dyn EventSink>, id: String) -> Self {
        Self {
            sink,
            out,
            id,
            started: std::time::Instant::now(),
            armed: true,
        }
    }

    fn emit(&self, outcome: SubagentOutcome, detail: Option<String>) {
        let s = self.sink.summary();
        self.out.emit(AgentEvent::Subagent(SubagentEvent::End {
            id: self.id.clone(),
            outcome,
            stop: s.stop,
            detail,
            turns: s.turns,
            tool_calls: s.tool_calls,
            duration_ms: self.started.elapsed().as_millis() as u64,
        }));
    }

    fn finish(mut self, outcome: SubagentOutcome, detail: Option<String>) {
        self.armed = false;
        self.emit(outcome, detail);
    }
}

impl Drop for EndGuard {
    fn drop(&mut self) {
        if self.armed {
            self.emit(
                SubagentOutcome::Failed,
                Some("dispatch aborted (panic or executor drop)".into()),
            );
        }
    }
}
```

Add `SubagentEvent, SubagentOutcome` to dispatch.rs's `use crate::{…}` list (dispatch lives in agent-core — `use crate::`, not `use agent_core::`; 3A gotcha).

- [ ] **Step 2: Wire into `execute()`.** Locate by content:
`let parent_id = format!("{}{}", self.deps.id_prefix, ctx.call_id);` followed by
`let sink = Arc::new(SubagentSink::new(`. `parent_id` is MOVED into the sink
today — clone it (spec/panel: Assumptions MAJOR-2). Replace the region with:

```rust
        // Visible parent id: at top level this is the raw call id; nested, the
        // prefix makes it the child row's on-wire id (spec G8 attribution).
        // Also the 3B-2 delegation id: Start/End and forwarded deltas all
        // carry this exact string.
        let parent_id = format!("{}{}", self.deps.id_prefix, ctx.call_id);
        let sink = Arc::new(SubagentSink::new(
            self.deps.sink.clone(),
            n,
            parent_id.clone(),
            self.deps.child_trace.clone(),
        ));
        // Typed lifecycle (spec §2.2): Start fires only after every validation
        // Err-return above (verified: none between next_dispatch_n() and the
        // run), and after the loop's own ToolStart for this call.
        self.deps.sink.emit(AgentEvent::Subagent(SubagentEvent::Start {
            id: parent_id.clone(),
            subagent_type: subagent_type.clone(),
            role: if resolved.is_some() { None } else { role.clone() },
        }));
        let guard = EndGuard::new(sink.clone(), self.deps.sink.clone(), parent_id);
```

(`subagent_type: String` and `role: Option<String>` are already in scope —
locate `let subagent_type = args` and `let role: Option<String> =` above. If
`role` is consumed later by value, keep the `.clone()` here.)

Then thread the four exits (locate each by content):

- timeout arm (`Err(_elapsed) => {`): before the `return Ok(failure_output(…))`,
  build the message once and reuse:

```rust
            Err(_elapsed) => {
                child_cancel.cancel();
                let what = format!("sub-agent timed out after {}s", ctx.timeout.as_secs());
                guard.finish(SubagentOutcome::Timeout, Some(what.clone()));
                return Ok(failure_output(&sink, what, "timeout"));
            }
```

- fatal arm (`Ok(Err(e)) => {`):

```rust
            Ok(Err(e)) => {
                let what = format!("sub-agent failed: {e}");
                guard.finish(SubagentOutcome::Failed, Some(what.clone()));
                return Ok(failure_output(&sink, what, "failed"));
            }
```

- cancel check (`if ctx.cancel.is_cancelled() {`): insert before the `return Err`:

```rust
        if ctx.cancel.is_cancelled() {
            guard.finish(SubagentOutcome::Cancelled, None);
            return Err(ToolError::Failed {
                message: "sub-agent cancelled".into(),
                stderr: None,
            });
        }
```

- normal path: immediately after the cancel check:

```rust
        guard.finish(SubagentOutcome::Completed, None);
```

- [ ] **Step 3: Extend the test-local `FullSink`** (dispatch.rs tests; its
catch-all currently records `"unexpected"` — typed lifecycle events reaching
the parent are now EXPECTED). Add before the `_ =>` arm:

```rust
                AgentEvent::Subagent(se) => {
                    use crate::SubagentEvent as SE;
                    match se {
                        SE::Start {
                            id, subagent_type, ..
                        } => ("subagent_start".into(), id, subagent_type, String::new()),
                        SE::Text { id, text } => ("subagent_text".into(), id, text, String::new()),
                        SE::Reasoning { id, text } => {
                            ("subagent_reasoning".into(), id, text, String::new())
                        }
                        SE::StreamRetry { id, .. } => {
                            ("subagent_stream_retry".into(), id, String::new(), String::new())
                        }
                        SE::End {
                            id, outcome, detail, ..
                        } => (
                            "subagent_end".into(),
                            id,
                            outcome.as_str().into(),
                            // 4th slot = detail so the timeout/fatal/panic
                            // tests can assert it (Failed/Timeout non-empty).
                            detail.unwrap_or_default(),
                        ),
                    }
                }
```

Run `cargo test -p agent-core` and update every existing exact-equality
expectation that now includes `subagent_start`/`subagent_end` rows (they are
end-to-end execute() tests; insert the new rows at the positions the failure
output shows — Start right after the dispatch validations, End at run end).
Do NOT weaken any assertion to "contains"; keep exact lists.

- [ ] **Step 4: Write the new dispatch tests** (dispatch.rs `mod tests`, using the existing `exec_deps`/`exec_ctx`/`ScriptedModel`/`resolved_with` helpers — locate by content):

```rust
    #[tokio::test]
    async fn start_carries_registry_name_and_gp_role_and_end_completed() {
        // general-purpose with role: Start{subagent_type:"general-purpose", role:Some}
        // named type: Start{subagent_type:"researcher", role:None} even when a
        // role arg is passed. Both end Completed with stop from the capture.
        // Drive execute() exactly like the existing named-resolution tests
        // (ScriptedModel child that immediately answers), with FullSink as the
        // parent sink; assert the subagent_start/subagent_end quads.
    }

    #[tokio::test]
    async fn rejected_dispatch_emits_no_start() {
        // Call execute() with an unknown subagent_type; assert Err AND that
        // the parent sink saw ZERO Subagent quads.
    }

    #[tokio::test]
    async fn end_on_timeout_fatal_and_cancel_paths() {
        // (a) timeout: ctx.timeout = Duration::from_millis(1) + a child model
        //     that never resolves (mirror how the existing timeout test stalls
        //     — locate `failure_output` timeout test by content).
        //     Assert exactly one End quad with outcome "timeout".
        // (b) fatal: a child model that returns Err → End "failed", and the
        //     detail-bearing tool result still matches failure_output.
        // (c) cancel: cancel ctx.cancel before/during run → End "cancelled".
        // Each: assert EXACTLY ONE subagent_end in the parent sink.
    }

    #[tokio::test]
    async fn panicking_child_still_yields_exactly_one_failed_end() {
        // Child model whose request handler panics (clone ScriptedModel's impl
        // shape with a panicking body). Wrap the execute() future in
        // futures::FutureExt::catch_unwind(AssertUnwindSafe(…)) — the same
        // mechanism execute_isolated uses — assert it panicked AND the parent
        // sink saw exactly one subagent_end with outcome "failed" and detail
        // "dispatch aborted (panic or executor drop)" (the EndGuard Drop).
    }

    #[tokio::test]
    async fn nested_dispatch_start_id_is_fully_qualified() {
        // Reuse the existing grandchild/G8 harness (locate the nested-dispatch
        // test by content). Assert the grandchild's subagent_start id starts
        // with "sub" and contains ':' — i.e. `sub{n}:{call_id}`, equal to the
        // forwarded sub:dispatch_agent row's on-wire id.
    }
```

(The bodies above name the harnesses to copy; write real assertions against
`FullSink` quads — no pseudo-asserts.)

And in `agent/crates/agent-core/tests/dispatch_tool.rs` (integration — parent
loop drives the dispatch tool; reuse its scripted-parent harness):

```rust
    #[tokio::test]
    async fn subagent_start_id_matches_an_already_emitted_tool_start() {
        // Run a parent loop whose scripted model makes one dispatch_agent call.
        // Collect all events in order (the file's existing collecting sink).
        // Find the ToolStart{name:"dispatch_agent", id: X} index and the
        // Subagent(Start{id: X}) index; assert tool_start_index < start_index
        // and the ids are string-equal (spec §5 ordering pin).
    }
```

- [ ] **Step 5: Run:** `cargo test -p agent-core`
Expected: PASS (including the updated exact-equality lists).

- [ ] **Step 6: Commit:** `git commit -am "feat(core): dispatch emits typed Subagent Start + drop-guard End on every exit path (3B-2 A2)"`

---

### Task A3: `SubagentSink` delta forwarding + exactly-once trace pin + stats pin

**Files:**
- Modify: `agent/crates/agent-core/src/dispatch.rs` (SubagentSink `emit`, doc comment deprecation note)
- Modify: `agent/crates/agent-runtime-config/src/trace.rs` (integration test only)
- Modify: `agent/crates/agent-core/src/stats.rs` (test only)

**Interfaces:**
- Consumes: A1 enum; A2's delegation-id conventions.
- Produces: child `Token`/`Reasoning`/`StreamRetry` arriving at any `SubagentSink` are forwarded to its parent sink as `Subagent(Text/Reasoning/StreamRetry)` stamped with `parent_call_id`; capture and trace-tap behavior byte-identical to today.

- [ ] **Step 1: Restructure the `other` arm of `SubagentSink::emit`.** Today it
taps, then locks `cap` and matches. Keep the tap first; move the lock INSIDE
the match arms so forwards happen without holding it:

```rust
            // Everything else stays off the frontends' RAW stream (spec D9/E9)
            // but goes to the child-trace tap so a failed child turn is
            // replayable (E4). 3B-2: Token/Reasoning/StreamRetry additionally
            // forward as TYPED Subagent deltas (spec §2.2) — capture and tap
            // are unchanged.
            other => {
                if let Some(t) = &self.child_trace {
                    t.record(self.n, &self.parent_call_id, &other);
                }
                match other {
                    AgentEvent::Token(t) => {
                        {
                            let mut cap = self.cap.lock().unwrap();
                            cap.segments
                                .last_mut()
                                .expect("segments never empty")
                                .push_str(&t);
                        }
                        self.parent.emit(AgentEvent::Subagent(SubagentEvent::Text {
                            id: self.parent_call_id.clone(),
                            text: t,
                        }));
                    }
                    // Net-new arm: reasoning is not captured (never was) — it
                    // only forwards (spec §2.2; gate G5 on the egress).
                    AgentEvent::Reasoning(t) => {
                        self.parent.emit(AgentEvent::Subagent(SubagentEvent::Reasoning {
                            id: self.parent_call_id.clone(),
                            text: t,
                        }));
                    }
                    AgentEvent::Usage { turn, .. } => {
                        self.cap.lock().unwrap().turns =
                            self.cap.lock().unwrap().turns.max(turn);
                    }
                    AgentEvent::Done(reason) => {
                        self.cap.lock().unwrap().stop = Some(reason);
                    }
                    AgentEvent::StreamRetry {
                        discarded_text_chars,
                        discarded_reasoning_chars,
                    } => {
                        {
                            let mut cap = self.cap.lock().unwrap();
                            let seg = cap.segments.last_mut().expect("segments never empty");
                            let keep =
                                seg.chars().count().saturating_sub(discarded_text_chars);
                            *seg = seg.chars().take(keep).collect();
                        }
                        self.parent
                            .emit(AgentEvent::Subagent(SubagentEvent::StreamRetry {
                                id: self.parent_call_id.clone(),
                                discarded_text_chars,
                                discarded_reasoning_chars,
                            }));
                    }
                    _ => {}
                }
            }
```

CAREFUL with the `Usage` arm above — do NOT double-lock as written; keep the
original single-lock form: `{ let mut cap = self.cap.lock().unwrap(); cap.turns = cap.turns.max(turn); }`.
Preserve the original comments on the StreamRetry capture-trim (char-boundary,
never pop) — move them with the code.

Also update the `SubagentSink` doc comment: it currently says forwards "ONLY
ToolStart/ToolResult … plus ServerUsage"; extend with the typed-delta
forwarding and add the deprecation note (spec §3.1):

```rust
/// DEPRECATED (3B-2): the `sub:{name}`/`sub{n}:{id}` renaming is superseded by
/// the typed `AgentEvent::Subagent` stream for new consumers; it is kept for
/// wire byte-compat and will be removed in a future phase.
```

- [ ] **Step 2: Update the sink-forwarding tests.** The existing
`forwards_carry_parent_id_and_tap_gets_exactly_the_suppressed_kinds` test's
FullSink exact-equality list gains a `subagent_text` quad for the Token
emission (tap expectations are UNCHANGED — token/error/done still tapped).
Then add:

```rust
    #[test]
    fn one_token_emission_forwards_typed_and_captures() {
        let parent = Arc::new(FullSink::default());
        let sink = SubagentSink::new(parent.clone(), 3, "d9".into(), None);
        sink.emit(AgentEvent::Token("hello".into()));
        // Forwarded typed, stamped with the delegation id:
        assert_eq!(
            parent.events.lock().unwrap()[0],
            ("subagent_text".to_string(), "d9".to_string(), "hello".to_string(), String::new())
        );
        // AND captured (single emission feeds both — spec §5 dual-role pin):
        assert_eq!(sink.summary().final_text, "hello");
    }

    #[test]
    fn reasoning_forwards_but_is_never_captured() {
        let parent = Arc::new(FullSink::default());
        let sink = SubagentSink::new(parent.clone(), 3, "d9".into(), None);
        sink.emit(AgentEvent::Reasoning("thinking".into()));
        assert_eq!(parent.events.lock().unwrap()[0].0, "subagent_reasoning");
        assert_eq!(sink.summary().final_text, "");
    }

    #[test]
    fn stream_retry_forwards_typed_and_trims_capture() {
        let parent = Arc::new(FullSink::default());
        let sink = SubagentSink::new(parent.clone(), 3, "d9".into(), None);
        sink.emit(AgentEvent::Token("abcdef".into()));
        sink.emit(AgentEvent::StreamRetry {
            discarded_text_chars: 3,
            discarded_reasoning_chars: 0,
        });
        assert_eq!(sink.summary().final_text, "abc"); // capture trimmed
        let got = parent.events.lock().unwrap().clone();
        assert_eq!(got[1].0, "subagent_stream_retry"); // AND forwarded
        assert_eq!(got[1].1, "d9");
    }

    #[test]
    fn subagent_events_never_arrive_at_a_subagent_sink_but_fall_to_tap_if_they_did() {
        // Absence pin (spec §2.2): feed a Subagent event straight in; it must
        // not be re-forwarded as a tool row or captured — it falls to the
        // catch-all (tap-only).
        let parent = Arc::new(FullSink::default());
        let tap = Arc::new(TapSpy::default());
        let sink = SubagentSink::new(parent.clone(), 3, "d9".into(), Some(tap.clone()));
        sink.emit(AgentEvent::Subagent(SubagentEvent::Text {
            id: "x".into(),
            text: "t".into(),
        }));
        assert_eq!(sink.summary().final_text, "");
        // It forwards nothing itself; only the tap records it.
        assert!(parent.events.lock().unwrap().is_empty());
        assert_eq!(tap.seen.lock().unwrap().len(), 1);
    }
```

NOTE: the absence-pin test requires the `TapSpy` kind-name mapping to label the
new variant — extend `TapSpy`'s match (locate by content in the same test mod)
with `AgentEvent::Subagent(_) => "subagent"`.

- [ ] **Step 3: Exactly-once trace test** (`trace.rs` test mod; this is the
spec §2.6/§3.4 pin — SubagentSink over an ObservedSink with a real TraceWriter
plus the ChildTraceTap, the production topology):

```rust
    #[test]
    fn child_token_lands_in_the_jsonl_exactly_once() {
        // TraceWriter into a temp dir (existing pattern). Then:
        let observed = Arc::new(ObservedSink {
            inner: Arc::new(NullSink), // a no-op EventSink test double
            stats: Arc::new(RwLock::new(SessionStats::default())),
            trace: Some(writer.clone()),
        });
        let sink = agent_core::SubagentSink::new(
            observed,
            1,
            "d1".into(),
            Some(Arc::new(ChildTraceTap(writer.clone()))),
        );
        sink.emit(agent_core::AgentEvent::Token("UNIQ_TOKEN_PAYLOAD".into()));
        // flush via the pattern the existing tests use (record a Done).
        let contents = /* read trace file */;
        let hits = contents.matches("UNIQ_TOKEN_PAYLOAD").count();
        assert_eq!(hits, 1, "child token must be traced exactly once (raw tap), got {hits}:\n{contents}");
        // And the one hit is the raw child record (has "sub":1), not a typed frame:
        let line = contents.lines().find(|l| l.contains("UNIQ_TOKEN_PAYLOAD")).unwrap();
        assert!(line.contains(r#""sub":1"#), "{line}");
        assert!(!line.contains("subagent_text"), "{line}");
    }
```

(`NullSink`: 3-line local `struct NullSink; impl EventSink for NullSink { fn emit(&self, _: AgentEvent) {} }`.)

- [ ] **Step 4: Stats pin** (`stats.rs` test mod — spec §2.6 table):

```rust
    #[test]
    fn subagent_events_move_no_counters() {
        let mut s = SessionStats::default();
        let before = s.clone();
        s.fold(&AgentEvent::Subagent(SubagentEvent::Text {
            id: "d1".into(),
            text: "x".into(),
        }));
        s.fold(&AgentEvent::Subagent(SubagentEvent::End {
            id: "d1".into(),
            outcome: SubagentOutcome::Completed,
            stop: None,
            detail: None,
            turns: 9,
            tool_calls: 9,
            duration_ms: 9,
        }));
        assert_eq!(s, before, "Subagent events must not double-count stats");
    }
```

(If `SessionStats` lacks `PartialEq`/`Clone`, compare the individual
`subagent_turns`/`subagent_tool_calls`/`turns`/`tool_calls` fields instead —
do not add derives just for the test unless trivial.)

- [ ] **Step 5: Run:** `cargo test -p agent-core -p agent-runtime-config`
Expected: PASS.

- [ ] **Step 6: Commit:** `git commit -am "feat(core): SubagentSink forwards typed child deltas; exactly-once trace + stats pins (3B-2 A3)"`

---

### Task B1: web wire types + reducer card logic

**Files:**
- Modify: `web/src/wire.ts` (`WireEvent` union)
- Modify: `web/src/state.ts` (Item `subagent` field, helpers, reducer cases)
- Create: `web/src/state.subagent.test.ts`

**Interfaces:**
- Consumes: the five A1 frame shapes (exact field names above).
- Produces: `Item` tool variant gains `subagent?: SubagentCard`; exported type `SubagentCard` (B2 renders it); reducer behavior per spec §2.4.

- [ ] **Step 1: `wire.ts` — extend `WireEvent`** (after the `stream_retry` member):

```ts
  | { type: "subagent_start"; id: string; subagent_type: string; role?: string }
  | { type: "subagent_text"; id: string; text: string }
  | { type: "subagent_reasoning"; id: string; text: string }
  | { type: "subagent_stream_retry"; id: string;
      discarded_text_chars: number; discarded_reasoning_chars: number }
  | { type: "subagent_end"; id: string; outcome: string; stop?: string; detail?: string;
      turns: number; tool_calls: number; duration_ms: number };
```

- [ ] **Step 2: `state.ts` — card type + Item field.** Above `Item`:

```ts
/** Live per-delegation card state (spec 3B-2 §2.4). Attached to the
 *  dispatch_agent tool item whose id equals the delegation id. */
export interface SubagentCard {
  subagentType: string;
  role?: string;
  status: "running" | "done";
  text: string;
  reasoning: string;
  /** Code points head-trimmed off text/reasoning by the transcript cap. */
  textElided: number;
  reasoningElided: number;
  outcome?: string;
  stop?: string;
  detail?: string;
  /** Accumulated from child server_usage frames (parent_id === delegation id). */
  promptTokens: number;
  completionTokens: number;
  costUsd: number;
  turns?: number;
  toolCalls?: number;
  durationMs?: number;
}
```

Tool Item member gains `subagent?: SubagentCard`:

```ts
  | { kind: "tool"; name: string; args: unknown; status: "running" | "done";
      id?: string; parentId?: string; subagent?: SubagentCard;
      content?: string; display?: Display; resultStatus?: string; durationMs?: number }
```

- [ ] **Step 3: helpers** (below `trimTrailing`):

```ts
/** Per-card transcript budget, in code points (spec §2.4: a runaway child
 *  must not grow a single React string unboundedly; append is a full copy). */
export const SUBAGENT_TRANSCRIPT_CAP = 30000;

function freshCard(subagentType: string, role?: string): SubagentCard {
  return { subagentType, role, status: "running", text: "", reasoning: "",
    textElided: 0, reasoningElided: 0, promptTokens: 0, completionTokens: 0, costUsd: 0 };
}

/** Append with head-trim: keep the newest CAP code points, count what fell off. */
function appendCapped(cur: string, elided: number, delta: string): { s: string; elided: number } {
  const cps = Array.from(cur + delta);
  if (cps.length <= SUBAGENT_TRANSCRIPT_CAP) return { s: cur + delta, elided };
  const overflow = cps.length - SUBAGENT_TRANSCRIPT_CAP;
  return { s: cps.slice(overflow).join(""), elided: elided + overflow };
}

/** Trim `chars` code points off a card transcript tail (child stream retry). */
function trimTail(s: string, chars: number): string {
  if (chars <= 0) return s;
  const cps = Array.from(s);
  return cps.slice(0, Math.max(0, cps.length - chars)).join("");
}

/** Find the newest tool item with this delegation id whose card can still
 *  receive frames (running card, or bare running dispatch row). Returns -1
 *  when only done cards (or nothing) match — caller creates a new item
 *  (placeholder rule, spec §2.4 / gate G3). */
function findLiveCardIndex(items: Item[], id: string): number {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === "tool" && it.id === id &&
        (it.subagent ? it.subagent.status === "running" : it.status === "running")) {
      return i;
    }
  }
  return -1;
}

/** Placeholder item for frames that matched nothing (mid-run reload, or a
 *  reused call id whose old card is done). */
function placeholderCardItem(id: string, card: SubagentCard): Item {
  return { kind: "tool", name: "dispatch_agent", args: {}, status: "running", id,
    subagent: card };
}
```

- [ ] **Step 4: reducer cases** (inside `reduceFrame`'s switch, before the
forward-compat `default:`; each returns via `startTurn`'s `s` like siblings):

```ts
    case "subagent_start": {
      const items = [...s.items];
      const i = findLiveCardIndex(items, p.id);
      if (i >= 0 && items[i].kind === "tool" && !(items[i] as Extract<Item, {kind:"tool"}>).subagent) {
        items[i] = { ...items[i], subagent: freshCard(p.subagent_type, p.role) } as Item;
      } else {
        // Reused call id landing on a live card, or no match at all → new card.
        items.push(placeholderCardItem(p.id, freshCard(p.subagent_type, p.role)));
      }
      return { ...s, items };
    }
    case "subagent_text":
    case "subagent_reasoning": {
      const items = [...s.items];
      let i = findLiveCardIndex(items, p.id);
      if (i < 0 || items[i].kind !== "tool" || !(items[i] as Extract<Item, {kind:"tool"}>).subagent) {
        // Placeholder rule: a frame with no live card materializes one so a
        // mid-run reload doesn't silently drop the delegation (gate G3).
        items.push(placeholderCardItem(p.id, freshCard("sub-agent")));
        i = items.length - 1;
      }
      const it = items[i] as Extract<Item, { kind: "tool" }>;
      const card = { ...it.subagent! };
      if (p.type === "subagent_text") {
        const r = appendCapped(card.text, card.textElided, p.text);
        card.text = r.s; card.textElided = r.elided;
      } else {
        const r = appendCapped(card.reasoning, card.reasoningElided, p.text);
        card.reasoning = r.s; card.reasoningElided = r.elided;
      }
      items[i] = { ...it, subagent: card };
      return { ...s, items };
    }
    case "subagent_stream_retry": {
      const items = [...s.items];
      const i = findLiveCardIndex(items, p.id);
      if (i < 0) return s;
      const it = items[i] as Extract<Item, { kind: "tool" }>;
      if (!it.subagent) return s;
      items[i] = { ...it, subagent: { ...it.subagent,
        text: trimTail(it.subagent.text, p.discarded_text_chars),
        reasoning: trimTail(it.subagent.reasoning, p.discarded_reasoning_chars) } };
      return { ...s, items };
    }
    case "subagent_end": {
      const items = [...s.items];
      let i = findLiveCardIndex(items, p.id);
      if (i < 0 || items[i].kind !== "tool" || !(items[i] as Extract<Item, {kind:"tool"}>).subagent) {
        items.push(placeholderCardItem(p.id, freshCard("sub-agent")));
        i = items.length - 1;
      }
      const it = items[i] as Extract<Item, { kind: "tool" }>;
      items[i] = { ...it, subagent: { ...it.subagent!, status: "done",
        outcome: p.outcome, stop: p.stop, detail: p.detail,
        turns: p.turns, toolCalls: p.tool_calls, durationMs: p.duration_ms } };
      return { ...s, items };
    }
```

- [ ] **Step 5: cost accumulation + finalize-on-done.** In the existing
`server_usage` case, replace `if (p.parent_id) return s;` with:

```ts
    case "server_usage": {
      // A sub-agent's usage frame must not flicker the parent turn readout;
      // it instead accumulates into its delegation card (spec 3B-2 §2.4) —
      // its tokens still land in session_stats (spec E5/E6c).
      if (p.parent_id) {
        const items = [...s.items];
        const i = findLiveCardIndex(items, p.parent_id);
        if (i < 0) return s;
        const it = items[i] as Extract<Item, { kind: "tool" }>;
        if (!it.subagent) return s;
        items[i] = { ...it, subagent: { ...it.subagent,
          promptTokens: it.subagent.promptTokens + p.prompt_tokens,
          completionTokens: it.subagent.completionTokens + p.completion_tokens,
          costUsd: it.subagent.costUsd + (p.cost_usd ?? 0) } };
        return { ...s, items };
      }
      return { ...s, serverUsage: { promptTokens: p.prompt_tokens, turn: p.turn } };
    }
```

In the existing `done` case, before the `return`, finalize orphaned cards
(client-side safety net, spec §2.4):

```ts
      // Safety net: a card still running at run end lost its End somewhere —
      // finalize as "unknown" so nothing spins forever (spec §2.4).
      for (let i = 0; i < items.length; i++) {
        const it = items[i];
        if (it.kind === "tool" && it.subagent?.status === "running") {
          items[i] = { ...it, subagent: { ...it.subagent, status: "done", outcome: "unknown" } };
        }
      }
```

- [ ] **Step 6: Write `web/src/state.subagent.test.ts`** (mirror
`state.dispatch.test.ts`'s `frame`/`red` helpers verbatim). Cover, as separate
`it` blocks with real assertions:

1. card assembly: `tool_start(dispatch_agent, id:"c1")` → `subagent_start(id:"c1")` attaches a running card with `subagentType`;
2. text/reasoning append routed by id under two interleaved delegations ("c1"/"c2" — each card gets only its own deltas);
3. duplicate call id across turns: first delegation ends (`subagent_end` on "c1"), a second `subagent_start` for "c1" opens a NEW card item (old card untouched);
4. placeholder: `subagent_text` with no prior items creates a placeholder card holding the text;
5. cap: append `SUBAGENT_TRANSCRIPT_CAP + 100` code points (two frames) → `text.length` (code points) === CAP and `textElided === 100`;
6. retry trim: text "abcdef", `subagent_stream_retry(discarded_text_chars: 3)` → "abc";
7. cost: `server_usage{parent_id:"c1", prompt_tokens:10, completion_tokens:5, cost_usd:0.01}` twice → card accumulates 20/10/0.02 AND `serverUsage` turn readout unchanged; without `parent_id` → readout updates as before;
8. finalize-on-done: running card + `done` frame → `status:"done"`, `outcome:"unknown"`;
9. `subagent_end` sets outcome/stop/detail/stats fields;
10. no-typed-frames fallback: a `tool_start`/`tool_result` pair with no subagent frames yields an item with `subagent === undefined` (renders as today);
11. depth-2: `tool_start(name:"sub:dispatch_agent", id:"sub3:c2", parent_id:"c9")` then `subagent_start(id:"sub3:c2")` attaches to that forwarded row.

- [ ] **Step 7: Run:** `cd web && npm test`
Expected: PASS (new file green, no existing test broken — `tsc` also clean via the test script; if the repo separates them run `npm run typecheck` too).

- [ ] **Step 8: Commit:** `git commit -am "feat(web): typed subagent frames + delegation card reducer (3B-2 B1)"`

---

### Task B2: card rendering in `AnimatedToolCall`

**Files:**
- Modify: `web/src/components/AnimatedToolCall.tsx`
- Test: `web/src/components/AnimatedToolCall.test.tsx` (extend)

**Interfaces:**
- Consumes: `SubagentCard` on `item.subagent` (B1). `AnimatedItem = Item & {…}` so the field flows through `animatedItemsFrom` untouched.
- Produces: `data-testid="subagent-card"` root, `data-testid="subagent-transcript"` transcript region (Task C's live drive asserts these).

Minimal-v1 card (spec §2.4, gate G2): the card is the dispatch row grown
richer — child rows stay flat `↳` siblings; do NOT touch `MessageList`,
`turnGroupsFrom`, or item grouping.

- [ ] **Step 1: render the card block** inside the existing component, after
the header `<div className="flex items-baseline gap-2">…</div>` and before the
`{!isRunning && …}` result line. Complete block:

```tsx
      {item.subagent && (
        <div className="mt-1 pl-5" data-testid="subagent-card">
          <div className="flex items-baseline gap-2">
            <span className="rounded px-1 text-xs"
              style={{ background: "var(--cli-accent)", color: "var(--cli-bg, #000)" }}>
              agent[{item.subagent.subagentType}]
            </span>
            <span className="text-xs" style={{
              color: item.subagent.status === "running" ? "var(--cli-accent)"
                : item.subagent.outcome === "completed" || item.subagent.outcome === undefined
                  ? "var(--cli-ok)" : "var(--cli-err)" }}>
              {item.subagent.status === "running" ? "running" : (item.subagent.outcome ?? "done")}
            </span>
            {(item.subagent.text || item.subagent.reasoning) && (
              <button type="button" onClick={() => setCardOpen((o) => !o)}
                style={{ color: "var(--cli-dim)" }} className="text-xs">
                {cardOpen ? "hide ▴" : "transcript ▾"}
              </button>
            )}
          </div>
          {cardOpen && (
            <pre data-testid="subagent-transcript"
              className="mt-1 max-h-64 overflow-y-auto whitespace-pre-wrap text-xs">
              {item.subagent.textElided > 0 && (
                <span style={{ color: "var(--cli-dim)" }}>
                  …({item.subagent.textElided} chars elided){"\n"}
                </span>
              )}
              {item.subagent.reasoning && (
                <span style={{ color: "var(--cli-dim)" }}>{item.subagent.reasoning}{"\n"}</span>
              )}
              <span style={{ color: "var(--cli-text)" }}>{item.subagent.text}</span>
            </pre>
          )}
          {item.subagent.status === "done" && (
            <div className="text-xs" style={{ color: "var(--cli-dim)" }}>
              {item.subagent.detail && <span style={{ color: "var(--cli-err)" }}>{item.subagent.detail} · </span>}
              {item.subagent.stop && `${item.subagent.stop} · `}
              {item.subagent.turns !== undefined && `${item.subagent.turns} turns · `}
              {item.subagent.toolCalls !== undefined && `${item.subagent.toolCalls} tools · `}
              {item.subagent.durationMs !== undefined && `${(item.subagent.durationMs / 1000).toFixed(1)}s`}
              {item.subagent.promptTokens > 0 &&
                ` · ${item.subagent.promptTokens + item.subagent.completionTokens} tok`}
              {item.subagent.costUsd > 0 && ` · $${item.subagent.costUsd.toFixed(4)}`}
            </div>
          )}
        </div>
      )}
```

Add the state hook next to `expanded`: `const [cardOpen, setCardOpen] = useState(true);`
(default open — the live transcript IS the product; users collapse noise, not
discover signal).

- [ ] **Step 2: extend `AnimatedToolCall.test.tsx`** (mirror its existing
render helpers/mocks — the file already mocks framer-motion or renders it;
locate by content). New cases:

```tsx
it("renders a running subagent card with badge, status, and live transcript", () => {
  // item: kind tool, status running, subagent: { subagentType:"researcher",
  //   status:"running", text:"partial answer", reasoning:"", textElided:0, … }
  // assert: getByTestId("subagent-card"); text "agent[researcher]"; "running";
  //   getByTestId("subagent-transcript") contains "partial answer".
});
it("renders outcome footer with detail, stats, and elision marker when done", () => {
  // subagent: status done, outcome "timeout", detail "sub-agent timed out after 5s",
  //   stop "stop", turns 2, toolCalls 3, durationMs 5000, textElided 42,
  //   promptTokens 100, completionTokens 20, costUsd 0.0123.
  // assert footer contains "timed out"? NO — footer shows outcome via status
  //   pill ("timeout") + detail line; assert "sub-agent timed out after 5s",
  //   "2 turns", "3 tools", "5.0s", "120 tok", "$0.0123",
  //   "…(42 chars elided)".
});
it("renders no card block when item.subagent is undefined", () => {
  // today's plain tool row: queryByTestId("subagent-card") === null.
});
```

(Write the real render calls/assertions following the file's existing style —
the comments above are the required assertions, not placeholders to skip.)

- [ ] **Step 3: Run:** `cd web && npm test`
Expected: PASS.

- [ ] **Step 4: Commit:** `git commit -am "feat(web): subagent delegation card in AnimatedToolCall (3B-2 B2)"`

---

### Task C: full CI + live WebDriver acceptance drive

**Files:** none created (verification task; fixes go where failures point)

- [ ] **Step 1: Full gate from repo root:** `bash scripts/ci.sh`
Expected: ALL legs green (okf check, skills lint, fmt, clippy, `agent/` tests, conditional `src-tauri`, web typecheck + vitest). Fix anything red before proceeding (fmt note: `src-tauri` is excluded from cargo fmt — hand-format if touched; not touched by this plan).

- [ ] **Step 2: Live drive (spec §5 acceptance).** Load the `auto-drive-tauri`
skill and use its **L2 GUI driving (WebDriver)** rung:
  - Precondition: `curl -s -m 2 localhost:8080/health` → `{"status":"ok"}`
    (llama-server with `qwen3.6-35b-a3b`; if down, start it per the skill /
    `local-llama-server` memory).
  - Drive the desktop app per the skill's WebDriver recipe (tauri-driver +
    selenium, one-session-per-run gotcha applies).
  - Send a prompt that forces a dispatch, e.g.:
    `Use the dispatch_agent tool to have a sub-agent count the .rs files under agent/crates/agent-core/src and report the number. Do not do it yourself.`
  - Assert, via WebDriver DOM queries:
    1. an element `[data-testid="subagent-card"]` appears while the run is live;
    2. `[data-testid="subagent-transcript"]` becomes non-empty **before** the
       card status leaves "running" (streamed child text, not a post-hoc dump);
    3. after completion the card shows a done footer (turns/tools/duration
       text present).
  - Record the observed transcript/DOM evidence in the session notes.
Expected: all three assertions hold against the live model.

- [ ] **Step 3: Commit** any drive-exposed fixes (each with its own test where
feasible), re-run `bash scripts/ci.sh`, then report the branch ready for
whole-branch review + merge (do NOT merge or push in this task).

---

## Execution notes for the coordinator

- Task order: 0 → A1 → A2 → A3 → B1 → B2 → C. A2/A3 both edit `dispatch.rs` —
  strictly sequential. B1/B2 could interleave with A-wave but share nothing
  with it; simplest is the listed order.
- Sub-agent prompts MUST repeat the anchors-drift warning: locate quoted code
  by content, not line number.
- Reviewer parity: A2 (drop-guard semantics) and A3 (exactly-once trace) are
  the calibration-sensitive tasks — review on a heavyweight model.
