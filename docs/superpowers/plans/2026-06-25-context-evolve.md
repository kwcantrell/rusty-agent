# context-evolve Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a self-improving harness (a skill + a Rust eval harness) that lets a Claude agent iteratively improve this runtime's context-management subsystem (in-window curation + long-term memory) so the running model solves hard, long-horizon tasks without drifting and in fewer tokens — correctness gated, tokens as tiebreaker.

**Architecture:** Three layers. (1) **Faithful token accounting** — surface server-reported `usage` through the model client and the agent loop (today's `Usage` event uses an internal estimate the spec forbids). (2) **Pure eval logic** — config/result/aggregate/gate/admissibility/task-spec modules in `agent-runtime-config`, fully unit-tested with no live model. (3) **Live harness + skill** — an `#[ignore]` test-binary that drives the real `assemble_loop` on a frozen task (single- or cross-session) and emits a `RunResult` JSON line, plus the `context-evolve` skill (`prepare.md`/`train.md`/`program.md`) that orchestrates the loop.

**Tech Stack:** Rust (workspace crates `agent-model`, `agent-core`, `agent-memory`, `agent-runtime-config`), `serde`/`serde_json`, `tokio`, local llama.cpp OpenAI-compatible server (Qwen3.6). Skill playbooks in Markdown under `.agents/skills/`.

## Global Constraints

- Cargo is not on PATH by default — every `cargo` command must be preceded by `source ~/.cargo/env`.
- All crates live under `agent/` (e.g. `agent/crates/agent-model`). Run cargo from `agent/`.
- Token metric = **sum over every model turn of (server `prompt_tokens` + server `completion_tokens`)**. Never the internal `built_tokens` estimate.
- Correctness is a hard gate; tokens are only a tiebreaker among passing runs. Never blend them.
- Live harness changes go in **`#[ignore]`** tests (no live server in CI). Pure logic is always non-ignored unit tests.
- Adding a field to `AssistantTurn` (derives `Default`) must keep existing literal constructions compiling — use `..Default::default()` at sites that don't set usage.
- Local server defaults: `AGENT_E2E_URL=http://localhost:8080`, `AGENT_E2E_MODEL=qwen3.6-35b-a3b`. Favorable window = `196608`.

---

## File Structure

**Modified (Milestone 1 — token accounting):**
- `agent/crates/agent-model/src/types.rs` — add `Chunk::Usage`; add usage fields to `AssistantTurn`.
- `agent/crates/agent-model/src/openai.rs` — request `stream_options.include_usage`; parse `usage` in `parse_sse_line`.
- `agent/crates/agent-core/src/event.rs` — add `AgentEvent::ServerUsage`.
- `agent/crates/agent-core/src/loop_.rs` — capture usage in the chunk fold; emit `ServerUsage` per turn.
- `agent/crates/agent-model/src/protocol.rs` — `..Default::default()` on `AssistantTurn` literals.

**Created (Milestone 2/3 — eval logic):**
- `agent/crates/agent-runtime-config/src/eval/mod.rs` — module root, re-exports.
- `agent/crates/agent-runtime-config/src/eval/config.rs` — `CandidateConfig`.
- `agent/crates/agent-runtime-config/src/eval/result.rs` — `RunResult`, `BatchResult`, aggregation.
- `agent/crates/agent-runtime-config/src/eval/gate.rs` — `gate`, `heldout_ok`.
- `agent/crates/agent-runtime-config/src/eval/admissibility.rs` — `admit`.
- `agent/crates/agent-runtime-config/src/eval/task.rs` — `TaskSpec`, `SessionSpec`, `SeedFile`.
- `agent/crates/agent-runtime-config/src/bin/eval_gate.rs` — thin CLI over gate/admit.
- `agent/crates/agent-runtime-config/src/lib.rs` — `pub mod eval;`.

**Created (Milestone 4 — live harness):**
- `agent/crates/agent-runtime-config/tests/eval_context.rs` — the live single-run driver.

**Created (Milestone 5 — skill + first task):**
- `.agents/skills/context-evolve/SKILL.md`, `prepare.md`, `train.md`, `program.md`.
- `.agents/skills/context-evolve/tasks/drift-ledger/task.json` + `seed/` + `hidden_tests/`.

---

## Milestone 1 — Faithful server-side token accounting

### Task 1: Surface server `usage` from the OpenAI streaming client

**Files:**
- Modify: `agent/crates/agent-model/src/types.rs:74-83` (`AssistantTurn`), `:74` (`Chunk` enum at line 75)
- Modify: `agent/crates/agent-model/src/openai.rs:91-127` (`body`), `:154-205` (`parse_sse_line`)
- Test: inline `#[cfg(test)]` in `agent/crates/agent-model/src/openai.rs`

**Interfaces:**
- Produces: `Chunk::Usage { prompt_tokens: u32, completion_tokens: u32 }`.

- [ ] **Step 1: Write the failing test** — append to the `mod tests` block at the bottom of `openai.rs`:

```rust
#[test]
fn parse_sse_line_extracts_server_usage() {
    let mut s = ThinkingSplitter::default();
    let line = r#"data: {"choices":[],"usage":{"prompt_tokens":1234,"completion_tokens":56}}"#;
    let out = parse_sse_line(line, &mut s).unwrap().unwrap();
    assert!(matches!(
        out.as_slice(),
        [Chunk::Usage { prompt_tokens: 1234, completion_tokens: 56 }]
    ));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-model parse_sse_line_extracts_server_usage`
Expected: FAIL — `no variant named Usage found for enum Chunk`.

- [ ] **Step 3: Add the `Chunk::Usage` variant** — in `types.rs`, change the `Chunk` enum (line 75) to:

```rust
#[derive(Debug, Clone)]
pub enum Chunk { Text(String), Reasoning(String), ToolCallDelta(RawToolCall), Done(StopReason), Usage { prompt_tokens: u32, completion_tokens: u32 } }
```

- [ ] **Step 4: Parse usage in `parse_sse_line`** — in `openai.rs`, immediately before the final `Some(Ok(out))` (line 204), insert:

```rust
    if let Some(u) = v.get("usage").and_then(Value::as_object) {
        out.push(Chunk::Usage {
            prompt_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
            completion_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        });
    }
```

- [ ] **Step 5: Request usage on the stream** — in `body()` (`openai.rs`), immediately before the final `b` (line 126), insert:

```rust
        b["stream_options"] = json!({ "include_usage": true });
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-model parse_sse_line_extracts_server_usage`
Expected: PASS.

- [ ] **Step 7: Build the whole crate to catch match-exhaustiveness breaks**

Run: `source ~/.cargo/env && cd agent && cargo build -p agent-model`
Expected: compile error(s) at any `match chunk { ... }` that is now non-exhaustive — fix in Task 2 (the loop fold is the only real consumer). If `openai.rs`'s own `collect` test helper (around line 314) matches `Chunk`, add `Chunk::Usage { .. } => {}` there.

- [ ] **Step 8: Commit**

```bash
git add agent/crates/agent-model/src/types.rs agent/crates/agent-model/src/openai.rs
git commit -m "feat(model): surface server-reported token usage as Chunk::Usage"
```

### Task 2: Carry usage on `AssistantTurn` and emit `ServerUsage` per turn

**Files:**
- Modify: `agent/crates/agent-model/src/types.rs:77-83` (`AssistantTurn`)
- Modify: `agent/crates/agent-model/src/protocol.rs:43,60` (`AssistantTurn` literals)
- Modify: `agent/crates/agent-core/src/event.rs:13-23` (`AgentEvent`)
- Modify: `agent/crates/agent-core/src/loop_.rs:108-118` (fold), `:196` (emit after completion)
- Test: inline `#[cfg(test)]` in `loop_.rs` (extend the existing fake-model test)

**Interfaces:**
- Consumes: `Chunk::Usage { prompt_tokens, completion_tokens }` (Task 1).
- Produces: `AssistantTurn { .., prompt_tokens: u32, completion_tokens: u32 }`; `AgentEvent::ServerUsage { prompt_tokens: u32, completion_tokens: u32, turn: usize }`.

- [ ] **Step 1: Write the failing test** — add to `loop_.rs` `mod tests`, modeled on the existing fake-model streaming test (which builds a `Vec<Result<Chunk, _>>`):

```rust
#[tokio::test]
async fn server_usage_event_carries_token_totals() {
    use std::sync::Mutex;
    #[derive(Default)]
    struct Caps(Mutex<Vec<(u32, u32)>>);
    impl EventSink for Caps {
        fn emit(&self, e: AgentEvent) {
            if let AgentEvent::ServerUsage { prompt_tokens, completion_tokens, .. } = e {
                self.0.lock().unwrap().push((prompt_tokens, completion_tokens));
            }
        }
    }
    let chunks = vec![
        Ok(Chunk::Text("done".into())),
        Ok(Chunk::Usage { prompt_tokens: 900, completion_tokens: 12 }),
        Ok(Chunk::Done(StopReason::Stop)),
    ];
    let caps = Arc::new(Caps::default());
    let agent = test_loop_with_chunks(chunks, caps.clone()); // helper: see Step 3
    let mut ctx = test_ctx();
    agent.run(&mut ctx, "hi".into()).await.unwrap();
    assert_eq!(caps.0.lock().unwrap().as_slice(), &[(900, 12)]);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core server_usage_event_carries_token_totals`
Expected: FAIL — `no variant named ServerUsage` and/or missing test helper.

- [ ] **Step 3: Add usage fields to `AssistantTurn`** — in `types.rs`:

```rust
#[derive(Debug, Clone, Default)]
pub struct AssistantTurn {
    pub text: String,
    pub raw_tool_calls: Vec<RawToolCall>,
    pub stop: StopReason,
    pub reasoning: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}
```

Then in `protocol.rs`, append `..Default::default()` to each `AssistantTurn { .. }` literal (lines 43, 60) so they still compile, e.g. `AssistantTurn { text, raw_tool_calls, stop, reasoning, ..Default::default() }`.

- [ ] **Step 4: Add the event variant** — in `event.rs`, add to `AgentEvent`:

```rust
    ServerUsage { prompt_tokens: u32, completion_tokens: u32, turn: usize },
```

- [ ] **Step 5: Capture usage in the fold** — in `loop_.rs`, in the chunk-consuming match (lines 111-114), add an arm and accumulate into locals declared alongside `text`/`reasoning`/`raw_tool_calls`:

```rust
                    Chunk::Usage { prompt_tokens, completion_tokens } => {
                        usage = (prompt_tokens, completion_tokens);
                    }
```

Declare `let mut usage = (0u32, 0u32);` next to the other accumulators, and change the final turn construction (line 118) to:

```rust
        Ok(AssistantTurn { text, raw_tool_calls, stop, reasoning, prompt_tokens: usage.0, completion_tokens: usage.1 })
```

- [ ] **Step 6: Emit `ServerUsage` after each completion** — in the turn loop, right after `let assistant = match self.completion_with_retry(&base, &cancel).await { Ok(t) => t, .. };` (around line 196), insert:

```rust
            self.sink.emit(AgentEvent::ServerUsage {
                prompt_tokens: assistant.prompt_tokens,
                completion_tokens: assistant.completion_tokens,
                turn: turn + 1,
            });
```

(If `assistant` is bound by a `match` whose arms `return`, place this emit on the success path immediately after the binding.) Add the `test_loop_with_chunks(chunks, sink)` helper next to the existing fake-model test, reusing its fake `ModelClient`/`ContextManager` but accepting an injected `sink`.

- [ ] **Step 7: Run the test to verify it passes**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core server_usage_event_carries_token_totals`
Expected: PASS.

- [ ] **Step 8: Full build + existing tests**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-model -p agent-core`
Expected: PASS (no regressions).

- [ ] **Step 9: Commit**

```bash
git add agent/crates/agent-model/src/types.rs agent/crates/agent-model/src/protocol.rs agent/crates/agent-core/src/event.rs agent/crates/agent-core/src/loop_.rs
git commit -m "feat(core): emit ServerUsage per turn from real server token counts"
```

---

## Milestone 2 — Pure eval logic (unit-tested, no live model)

### Task 3: `eval` module scaffold + `CandidateConfig`

**Files:**
- Create: `agent/crates/agent-runtime-config/src/eval/mod.rs`, `eval/config.rs`
- Modify: `agent/crates/agent-runtime-config/src/lib.rs` (add `pub mod eval;`)
- Test: inline in `eval/config.rs`

**Interfaces:**
- Produces: `CandidateConfig` with `fn favorable(window: usize) -> Self`, `fn offload_config(&self) -> agent_core::OffloadConfig`, serde `Serialize`/`Deserialize`.

- [ ] **Step 1: Add the module** — in `lib.rs`, add `pub mod eval;`. Create `eval/mod.rs`:

```rust
pub mod config;
pub mod result;
pub mod gate;
pub mod admissibility;
pub mod task;

pub use config::CandidateConfig;
pub use result::{BatchResult, RunResult};
pub use gate::{gate, heldout_ok, Verdict};
pub use admissibility::{admit, Admissibility};
pub use task::{SeedFile, SessionSpec, TaskSpec};
```

(Create the other four files as empty stubs with a `// filled in later task` line so the module compiles; each later task fills one in.)

- [ ] **Step 2: Write the failing test** — in `eval/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn favorable_disables_curation() {
        let f = CandidateConfig::favorable(196608);
        assert_eq!(f.context_limit, 196608);
        assert!(f.high_water_pct >= 1.0);
        assert_eq!(f.offload_config().output_min_bytes, usize::MAX);
        assert_eq!(f.offload_config().error_min_bytes, usize::MAX);
        assert!(f.auto_recall && f.relevance_threshold <= 0.001);
    }
    #[test]
    fn round_trips_through_json() {
        let c = CandidateConfig::favorable(32000);
        let s = serde_json::to_string(&c).unwrap();
        let back: CandidateConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.context_limit, 32000);
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config favorable_disables_curation`
Expected: FAIL — `CandidateConfig` not found.

- [ ] **Step 4: Implement `CandidateConfig`** — top of `eval/config.rs`:

```rust
use agent_core::OffloadConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateConfig {
    // in-window curation
    pub context_limit: usize,
    pub high_water_pct: f32,
    pub keep_recent: usize,
    pub error_min_bytes: usize,
    pub output_min_bytes: usize,
    pub recall_budget: usize,
    // long-term memory
    pub memory_enabled: bool,
    pub default_k: usize,
    pub relevance_threshold: f32,
    pub dedup_threshold: f32,
    pub forget_threshold: f32,
    pub max_recall_chars: usize,
    pub recall_token_budget: usize,
    pub auto_recall: bool,
}

impl CandidateConfig {
    /// The "context manager neutralized" reference: nothing offloads, nothing
    /// compacts, retrieval surfaces everything. Used as the favorable side of the
    /// two-sided admissibility test.
    pub fn favorable(window: usize) -> Self {
        Self {
            context_limit: window,
            high_water_pct: 1.0,
            keep_recent: usize::MAX,
            error_min_bytes: usize::MAX,
            output_min_bytes: usize::MAX,
            recall_budget: 4096,
            memory_enabled: true,
            default_k: 20,
            relevance_threshold: 0.0,
            dedup_threshold: 0.95,
            forget_threshold: 0.85,
            max_recall_chars: 64 * 1024,
            recall_token_budget: 8192,
            auto_recall: true,
        }
    }

    pub fn offload_config(&self) -> OffloadConfig {
        OffloadConfig {
            error_min_bytes: self.error_min_bytes,
            output_min_bytes: self.output_min_bytes,
            keep_recent: self.keep_recent,
            exclude_tools: Vec::new(),
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config eval::config`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-runtime-config/src/lib.rs agent/crates/agent-runtime-config/src/eval/
git commit -m "feat(eval): CandidateConfig with favorable preset + offload mapping"
```

### Task 4: `RunResult` / `BatchResult` + aggregation

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/eval/result.rs`
- Test: inline

**Interfaces:**
- Produces: `RunResult { passed: bool, tokens: u64, turns: usize }`; `BatchResult { runs: Vec<RunResult> }` with `passes()`, `pass_rate()`, `median_tokens_passing() -> Option<u64>`.

- [ ] **Step 1: Write the failing test** — in `eval/result.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn rr(passed: bool, tokens: u64) -> RunResult { RunResult { passed, tokens, turns: 1 } }
    #[test]
    fn median_uses_only_passing_runs() {
        let b = BatchResult { runs: vec![rr(true, 100), rr(false, 1), rr(true, 300), rr(true, 200)] };
        assert_eq!(b.passes(), 3);
        assert!((b.pass_rate() - 0.75).abs() < 1e-9);
        assert_eq!(b.median_tokens_passing(), Some(200)); // median of {100,200,300}
    }
    #[test]
    fn no_passing_runs_has_no_median() {
        let b = BatchResult { runs: vec![rr(false, 5), rr(false, 9)] };
        assert_eq!(b.median_tokens_passing(), None);
        assert_eq!(b.pass_rate(), 0.0);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config median_uses_only_passing_runs`
Expected: FAIL — types not found.

- [ ] **Step 3: Implement** — in `eval/result.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub passed: bool,
    pub tokens: u64,
    pub turns: usize,
}

#[derive(Debug, Clone, Default)]
pub struct BatchResult {
    pub runs: Vec<RunResult>,
}

impl BatchResult {
    pub fn passes(&self) -> usize { self.runs.iter().filter(|r| r.passed).count() }

    pub fn pass_rate(&self) -> f64 {
        if self.runs.is_empty() { return 0.0; }
        self.passes() as f64 / self.runs.len() as f64
    }

    /// Median token count over passing runs only (failed runs are not comparable).
    pub fn median_tokens_passing(&self) -> Option<u64> {
        let mut v: Vec<u64> = self.runs.iter().filter(|r| r.passed).map(|r| r.tokens).collect();
        if v.is_empty() { return None; }
        v.sort_unstable();
        Some(v[v.len() / 2])
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config eval::result`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/eval/result.rs
git commit -m "feat(eval): RunResult/BatchResult with pass-rate + passing-only median"
```

### Task 5: The gate (correctness gate + token tiebreak + held-out no-regression)

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/eval/gate.rs`
- Test: inline

**Interfaces:**
- Consumes: `BatchResult` (Task 4).
- Produces: `enum Verdict { Promote, Reject { reason: String } }`; `fn gate(champion: &BatchResult, candidate: &BatchResult) -> Verdict`; `fn heldout_ok(champion: &[BatchResult], candidate: &[BatchResult]) -> bool`.

- [ ] **Step 1: Write the failing test** — in `eval/gate.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::result::{BatchResult, RunResult};
    fn batch(spec: &[(bool, u64)]) -> BatchResult {
        BatchResult { runs: spec.iter().map(|&(passed, tokens)| RunResult { passed, tokens, turns: 1 }).collect() }
    }
    #[test]
    fn rejects_when_correctness_regresses() {
        let champ = batch(&[(true, 500), (true, 500), (true, 500)]);
        let cand = batch(&[(true, 100), (false, 1), (true, 100)]); // fewer passes
        assert!(matches!(gate(&champ, &cand), Verdict::Reject { .. }));
    }
    #[test]
    fn promotes_when_correctness_holds_and_tokens_drop() {
        let champ = batch(&[(true, 500), (true, 500), (true, 500)]);
        let cand = batch(&[(true, 300), (true, 300), (true, 300)]);
        assert!(matches!(gate(&champ, &cand), Verdict::Promote));
    }
    #[test]
    fn rejects_when_tokens_not_better() {
        let champ = batch(&[(true, 300), (true, 300), (true, 300)]);
        let cand = batch(&[(true, 300), (true, 300), (true, 300)]);
        assert!(matches!(gate(&champ, &cand), Verdict::Reject { .. }));
    }
    #[test]
    fn heldout_blocks_any_pass_rate_regression() {
        let champ = vec![batch(&[(true, 9), (true, 9)]), batch(&[(true, 9), (true, 9)])];
        let cand_ok = vec![batch(&[(true, 9), (true, 9)]), batch(&[(true, 9), (true, 9)])];
        let cand_bad = vec![batch(&[(true, 9), (true, 9)]), batch(&[(true, 9), (false, 9)])];
        assert!(heldout_ok(&champ, &cand_ok));
        assert!(!heldout_ok(&champ, &cand_bad));
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config eval::gate`
Expected: FAIL — `gate`/`Verdict` not found.

- [ ] **Step 3: Implement** — in `eval/gate.rs`:

```rust
use crate::eval::result::BatchResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict { Promote, Reject { reason: String } }

/// Lexicographic: correctness is a hard gate, tokens are the tiebreaker. A
/// candidate is promoted only if it does not lower the pass count AND strictly
/// reduces the median tokens among passing runs.
pub fn gate(champion: &BatchResult, candidate: &BatchResult) -> Verdict {
    if candidate.passes() < champion.passes() {
        return Verdict::Reject {
            reason: format!("correctness regressed: {} < {} passes", candidate.passes(), champion.passes()),
        };
    }
    match (candidate.median_tokens_passing(), champion.median_tokens_passing()) {
        (Some(cand), Some(champ)) if cand < champ => Verdict::Promote,
        (Some(cand), Some(champ)) => Verdict::Reject {
            reason: format!("tokens not improved: {cand} >= {champ}"),
        },
        _ => Verdict::Reject { reason: "no passing runs to compare tokens".into() },
    }
}

/// Held-out is a hard pass-rate gate: a promotion is rejected if it regresses
/// ANY individual held-out task's pass rate. Tokens on held-out are advisory and
/// not checked here. `champion`/`candidate` are aligned per-task.
pub fn heldout_ok(champion: &[BatchResult], candidate: &[BatchResult]) -> bool {
    champion.len() == candidate.len()
        && champion.iter().zip(candidate).all(|(c, n)| n.pass_rate() >= c.pass_rate())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config eval::gate`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/eval/gate.rs
git commit -m "feat(eval): promotion gate (correctness hard gate, tokens tiebreak, held-out no-regression)"
```

### Task 6: Two-sided admissibility

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/eval/admissibility.rs`
- Test: inline

**Interfaces:**
- Consumes: `BatchResult` (Task 4).
- Produces: `enum Admissibility { Admitted, IllSized, CapabilityBound, NoWeakness }`; `fn admit(favorable: &BatchResult, realistic: &BatchResult, favorable_overflowed: bool) -> Admissibility`.

- [ ] **Step 1: Write the failing test** — in `eval/admissibility.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::result::{BatchResult, RunResult};
    fn batch(passes: usize, n: usize) -> BatchResult {
        BatchResult { runs: (0..n).map(|i| RunResult { passed: i < passes, tokens: 1, turns: 1 }).collect() }
    }
    #[test]
    fn admits_when_favorable_passes_and_realistic_fails() {
        // favorable 5/5 green, realistic 1/5 red -> a real, capturable weakness
        assert_eq!(admit(&batch(5, 5), &batch(1, 5), false), Admissibility::Admitted);
    }
    #[test]
    fn ill_sized_when_favorable_overflowed() {
        assert_eq!(admit(&batch(5, 5), &batch(0, 5), true), Admissibility::IllSized);
    }
    #[test]
    fn capability_bound_when_favorable_also_fails() {
        assert_eq!(admit(&batch(1, 5), &batch(0, 5), false), Admissibility::CapabilityBound);
    }
    #[test]
    fn no_weakness_when_realistic_already_passes() {
        assert_eq!(admit(&batch(5, 5), &batch(4, 5), false), Admissibility::NoWeakness);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config eval::admissibility`
Expected: FAIL — `admit`/`Admissibility` not found.

- [ ] **Step 3: Implement** — in `eval/admissibility.rs`:

```rust
use crate::eval::result::BatchResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admissibility {
    /// Red under realistic config AND green under favorable: a capturable weakness.
    Admitted,
    /// Favorable run overflowed the window — the transcript doesn't fit; re-size.
    IllSized,
    /// Even favorable fails: the model can't do it regardless of context. Discard.
    CapabilityBound,
    /// Realistic already passes: there's nothing for the loop to capture. Discard.
    NoWeakness,
}

/// Favorable must reliably pass; realistic must reliably fail. Thresholds: favorable
/// pass-rate >= 0.8 ("the model can do it given ideal context"), realistic pass-rate
/// < 0.5 ("the weakness bites a majority of the time").
const FAVORABLE_MIN: f64 = 0.8;
const REALISTIC_MAX: f64 = 0.5;

pub fn admit(favorable: &BatchResult, realistic: &BatchResult, favorable_overflowed: bool) -> Admissibility {
    if favorable_overflowed { return Admissibility::IllSized; }
    if favorable.pass_rate() < FAVORABLE_MIN { return Admissibility::CapabilityBound; }
    if realistic.pass_rate() >= REALISTIC_MAX { return Admissibility::NoWeakness; }
    Admissibility::Admitted
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config eval::admissibility`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/eval/admissibility.rs
git commit -m "feat(eval): two-sided admissibility (red-realistic AND green-favorable)"
```

### Task 7: `TaskSpec` manifest

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/eval/task.rs`
- Test: inline

**Interfaces:**
- Produces: `TaskSpec`, `SessionSpec`, `SeedFile` (all serde); `TaskSpec::is_cross_session(&self) -> bool`; `TaskSpec::from_json(&str) -> serde_json::Result<TaskSpec>`.

- [ ] **Step 1: Write the failing test** — in `eval/task.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    const JSON: &str = r#"{
      "id": "drift-ledger",
      "mode": "drift",
      "realistic_window": 16000,
      "favorable_window": 196608,
      "memory_enabled": false,
      "seed_files": [{ "path": "ledger.txt", "contents": "start: 0\n" }],
      "test_cmd": "bash hidden_tests/check.sh",
      "sessions": [{ "prompts": ["step 1", "step 2"] }]
    }"#;
    #[test]
    fn parses_and_detects_single_session() {
        let t = TaskSpec::from_json(JSON).unwrap();
        assert_eq!(t.id, "drift-ledger");
        assert_eq!(t.realistic_window, 16000);
        assert_eq!(t.seed_files[0].path, "ledger.txt");
        assert!(!t.is_cross_session());
    }
    #[test]
    fn two_sessions_is_cross_session() {
        let mut t = TaskSpec::from_json(JSON).unwrap();
        t.sessions.push(SessionSpec { prompts: vec!["recall it".into()] });
        assert!(t.is_cross_session());
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config eval::task`
Expected: FAIL — types not found.

- [ ] **Step 3: Implement** — in `eval/task.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedFile {
    pub path: String,
    pub contents: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSpec {
    /// Ordered user turns to run in this session (each is one `agent.run`).
    pub prompts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    /// Failure mode this task stresses: "drift" | "offload" | "compaction" |
    /// "recall" | "memory-under-recall" | "memory-over-recall".
    pub mode: String,
    pub realistic_window: usize,
    pub favorable_window: usize,
    pub memory_enabled: bool,
    pub seed_files: Vec<SeedFile>,
    /// Command run AFTER the agent finishes, cwd = workspace, with hidden tests
    /// copied in. Exit code 0 == pass.
    pub test_cmd: String,
    pub sessions: Vec<SessionSpec>,
}

impl TaskSpec {
    pub fn from_json(s: &str) -> serde_json::Result<TaskSpec> { serde_json::from_str(s) }
    /// More than one session => a fact must survive a fresh (empty) window via memory.
    pub fn is_cross_session(&self) -> bool { self.sessions.len() > 1 }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config eval::task`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/eval/task.rs
git commit -m "feat(eval): TaskSpec manifest with cross-session detection"
```

---

## Milestone 3 — Decision CLI

### Task 8: `eval_gate` binary

**Files:**
- Create: `agent/crates/agent-runtime-config/src/bin/eval_gate.rs`
- Test: a non-ignored integration smoke test `agent/crates/agent-runtime-config/tests/eval_gate_cli.rs`

**Interfaces:**
- Consumes: `gate`, `admit`, `BatchResult`, `RunResult` (Tasks 4-6).
- CLI: `eval_gate gate <champion.jsonl> <candidate.jsonl>` and `eval_gate admit <favorable.jsonl> <realistic.jsonl> [--overflowed]`, where each file is newline-delimited `RunResult` JSON. Prints a one-line verdict and exits 0 (Promote/Admitted) or 1 (otherwise).

- [ ] **Step 1: Write the failing test** — in `tests/eval_gate_cli.rs`:

```rust
use std::io::Write;
use std::process::Command;

fn write_jsonl(dir: &std::path::Path, name: &str, lines: &[&str]) -> std::path::PathBuf {
    let p = dir.join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    for l in lines { writeln!(f, "{l}").unwrap(); }
    p
}

#[test]
fn admit_cli_reports_admitted_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let fav = write_jsonl(dir.path(), "fav.jsonl",
        &[r#"{"passed":true,"tokens":9,"turns":1}"#; 5]);
    let real = write_jsonl(dir.path(), "real.jsonl",
        &[r#"{"passed":false,"tokens":9,"turns":1}"#; 5]);
    let out = Command::new(env!("CARGO_BIN_EXE_eval_gate"))
        .args(["admit", fav.to_str().unwrap(), real.to_str().unwrap()])
        .output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Admitted"));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config --test eval_gate_cli`
Expected: FAIL — `CARGO_BIN_EXE_eval_gate` not defined (binary doesn't exist).

- [ ] **Step 3: Implement the binary** — `src/bin/eval_gate.rs`:

```rust
use agent_runtime_config::eval::{admit, gate, Admissibility, BatchResult, RunResult, Verdict};
use std::process::exit;

fn load(path: &str) -> BatchResult {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| { eprintln!("read {path}: {e}"); exit(2) });
    let runs = text.lines().filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<RunResult>(l).unwrap_or_else(|e| { eprintln!("parse: {e}"); exit(2) }))
        .collect();
    BatchResult { runs }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gate") => {
            let (champ, cand) = (load(&args[2]), load(&args[3]));
            match gate(&champ, &cand) {
                Verdict::Promote => { println!("Promote"); exit(0); }
                Verdict::Reject { reason } => { println!("Reject: {reason}"); exit(1); }
            }
        }
        Some("admit") => {
            let (fav, real) = (load(&args[2]), load(&args[3]));
            let overflowed = args.iter().any(|a| a == "--overflowed");
            let verdict = admit(&fav, &real, overflowed);
            println!("{verdict:?}");
            exit(if verdict == Admissibility::Admitted { 0 } else { 1 });
        }
        _ => { eprintln!("usage: eval_gate <gate|admit> <a.jsonl> <b.jsonl> [--overflowed]"); exit(2); }
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config --test eval_gate_cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/bin/eval_gate.rs agent/crates/agent-runtime-config/tests/eval_gate_cli.rs
git commit -m "feat(eval): eval_gate CLI over gate + admissibility"
```

---

## Milestone 4 — Live eval harness

### Task 9: `eval_context` single-run driver (live, `#[ignore]`)

**Files:**
- Create: `agent/crates/agent-runtime-config/tests/eval_context.rs`

**Interfaces:**
- Consumes: `CandidateConfig`, `TaskSpec` (Tasks 3,7); `assemble_loop`/`LoopParts`; `CuratedContext`; `agent_memory`; `AgentEvent::ServerUsage` (Task 2).
- Env: `AGENT_E2E_URL`, `AGENT_E2E_MODEL`, `TASK_JSON` (path), `CONFIG_JSON` (path), `HIDDEN_TESTS_DIR` (path), optional `AGENT_API_KEY`.
- Output: exactly one line of `RunResult` JSON on stdout.

- [ ] **Step 1: Write the harness** — `tests/eval_context.rs`. This is a live test (no CI run); its "test" is the documented invocation in Step 2. Key structure (mirrors `soak_live.rs` for the `SafeApproval` gate — copy that struct verbatim from `soak_live.rs:31-66`):

```rust
//! LIVE EVAL HARNESS — one run of one task under one CandidateConfig. Drives the
//! real assemble_loop (single- or cross-session), sums SERVER token usage, runs the
//! hidden tests, and prints one RunResult JSON line. Opt-in. Run via prepare.md/train.md.
//!   AGENT_E2E_URL=... AGENT_E2E_MODEL=... TASK_JSON=task.json CONFIG_JSON=cfg.json \
//!   HIDDEN_TESTS_DIR=hidden_tests cargo test -p agent-runtime-config --test eval_context \
//!     -- --ignored --nocapture
use agent_core::{AgentEvent, CuratedContext, EventSink, InMemoryOffloadStore, Message, OffloadStore};
use agent_memory::{build_tools_with, project_scope, MemoryConfig, MemoryRetriever, SqliteStore, StubEmbedder};
use agent_model::OpenAiCompatClient;
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use agent_runtime_config::eval::{CandidateConfig, RunResult, TaskSpec};
use agent_runtime_config::{assemble_loop, LoopParts, RuntimeConfig};
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// --- paste SafeApproval from soak_live.rs:31-66 here ---

#[derive(Default)]
struct TokenMeter { total: AtomicU64 }
impl EventSink for TokenMeter {
    fn emit(&self, e: AgentEvent) {
        if let AgentEvent::ServerUsage { prompt_tokens, completion_tokens, .. } = e {
            self.total.fetch_add(prompt_tokens as u64 + completion_tokens as u64, Ordering::Relaxed);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live eval: requires AGENT_E2E_URL/MODEL, TASK_JSON, CONFIG_JSON, HIDDEN_TESTS_DIR"]
async fn eval_context_run() {
    let url = std::env::var("AGENT_E2E_URL").expect("AGENT_E2E_URL");
    let model = std::env::var("AGENT_E2E_MODEL").expect("AGENT_E2E_MODEL");
    let task: TaskSpec = TaskSpec::from_json(
        &std::fs::read_to_string(std::env::var("TASK_JSON").expect("TASK_JSON")).unwrap()).unwrap();
    let cc: CandidateConfig = serde_json::from_str(
        &std::fs::read_to_string(std::env::var("CONFIG_JSON").expect("CONFIG_JSON")).unwrap()).unwrap();
    let hidden = std::env::var("HIDDEN_TESTS_DIR").expect("HIDDEN_TESTS_DIR");

    // Throwaway workspace + seed files. Memory store SHARED across sessions; each
    // session gets a FRESH window (new CuratedContext + new offload store).
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    for sf in &task.seed_files { std::fs::write(ws.join(&sf.path), &sf.contents).unwrap(); }

    let mem_db = ws.join("memory.db");
    let meter = Arc::new(TokenMeter::default());

    for session in &task.sessions {
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(), url.clone(), model.clone(), "native".into(), cc.context_limit);
        cfg.context_limit = cc.context_limit; // realistic (or favorable) window
        cfg.memory = task.memory_enabled && cc.memory_enabled;
        cfg.sandbox_mode = "off".into();
        cfg.max_turns = 12;

        // Memory: shared SqliteStore so facts persist across sessions; StubEmbedder
        // is deterministic (set the onnx feature for real embeddings if needed).
        let (mem_tools, retriever) = if cfg.memory {
            let store = Arc::new(SqliteStore::open(&mem_db).unwrap());
            let embedder = Arc::new(StubEmbedder::d384());
            let mut mcfg = MemoryConfig::default();
            mcfg.default_k = cc.default_k; mcfg.relevance_threshold = cc.relevance_threshold;
            mcfg.dedup_threshold = cc.dedup_threshold; mcfg.forget_threshold = cc.forget_threshold;
            mcfg.max_recall_chars = cc.max_recall_chars; mcfg.recall_token_budget = cc.recall_token_budget;
            mcfg.auto_recall = cc.auto_recall;
            let mcfg = Arc::new(mcfg);
            let scope = project_scope(&ws);
            let tools = build_tools_with(embedder.clone(), store.clone(), mcfg.clone(), scope.clone());
            let key = match &scope { agent_memory::MemoryScope::Project(k) => k.clone(), _ => String::new() };
            let r: Arc<dyn agent_core::Retriever> = Arc::new(MemoryRetriever { embedder, store, cfg: mcfg, project_key: key });
            (tools, Some(r))
        } else { (vec![], None) };

        let offload: Arc<dyn OffloadStore> = Arc::new(InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        let built = assemble_loop(&cfg, LoopParts {
            model: Arc::new(OpenAiCompatClient::new(url.clone(), model.clone(), std::env::var("AGENT_API_KEY").ok())),
            sink: meter.clone(),
            approval: Arc::new(SafeApproval { denied: Mutex::new(Vec::new()) }),
            workspace: ws.clone(),
            mcp_tools: vec![], memory_tools: mem_tools, memory_retriever: retriever,
            stream_idle_timeout: Duration::from_secs(120),
            base_system_prompt: "You are a coding agent in a sandboxed workspace. Use the provided tools to complete each task, then give a short final reply.".into(),
            offload_store: offload.clone(), compact_flag: flag.clone(),
        });
        let agent = built.loop_;
        let mut ctx = CuratedContext::new(Message::system(built.system_prompt), offload.clone(), flag)
            .with_recall_budget(cc.recall_budget)
            .with_offload_config(cc.offload_config())
            .with_high_water_pct(cc.high_water_pct);

        for prompt in &session.prompts {
            let cancel = tokio_util::sync::CancellationToken::new();
            let run = agent.run_with_cancel(&mut ctx, prompt.clone(), cancel.clone());
            let _ = tokio::time::timeout(Duration::from_secs(120), run).await;
        }
    }

    // Sealed grading step: copy hidden tests in, run test_cmd, capture exit code.
    let dest = ws.join("hidden_tests");
    std::fs::create_dir_all(&dest).unwrap();
    for entry in std::fs::read_dir(&hidden).unwrap() {
        let e = entry.unwrap();
        std::fs::copy(e.path(), dest.join(e.file_name())).unwrap();
    }
    let status = std::process::Command::new("bash").arg("-c").arg(&task.test_cmd)
        .current_dir(&ws).status().unwrap();

    let result = RunResult {
        passed: status.success(),
        tokens: meter.total.load(Ordering::Relaxed),
        turns: 0,
    };
    println!("{}", serde_json::to_string(&result).unwrap());
}
```

(If `agent_core` does not re-export `Message`, import it from `agent_model::Message` as `soak_live.rs` does. Confirm `StubEmbedder`/`SqliteStore`/`project_scope`/`MemoryScope` are the public names from `agent_memory::lib.rs` — they are, per its `pub use`.)

- [ ] **Step 2: Verify it compiles and runs one live pass**

Run (compile only, no server needed):
`source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config --test eval_context --no-run`
Expected: builds clean.

Then a smoke run against the live server using the first task (created in Task 11):
```bash
source ~/.cargo/env && cd agent
AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
TASK_JSON=../.agents/skills/context-evolve/tasks/drift-ledger/task.json \
CONFIG_JSON=/tmp/champion.json \
HIDDEN_TESTS_DIR=../.agents/skills/context-evolve/tasks/drift-ledger/hidden_tests \
cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture
```
Expected: a final stdout line like `{"passed":false,"tokens":12873,"turns":0}`.

- [ ] **Step 3: Commit**

```bash
git add agent/crates/agent-runtime-config/tests/eval_context.rs
git commit -m "feat(eval): live single-run eval_context harness (cross-session, server-usage tokens)"
```

---

## Milestone 5 — The skill and the first task

### Task 10: First drift task fixture + hidden tests

**Files:**
- Create: `.agents/skills/context-evolve/tasks/drift-ledger/task.json`
- Create: `.agents/skills/context-evolve/tasks/drift-ledger/seed/` (none needed; seed via task.json)
- Create: `.agents/skills/context-evolve/tasks/drift-ledger/hidden_tests/check.sh`

**Design (drift mode, single session, scaled 16K window):** The agent maintains a running ledger in `ledger.txt`. Across ~10 sequential sub-steps it is told small append/update operations (each step also dumps a large file to read, to fill the window). The *original goal* — "the final line of ledger.txt must be the running total of every `+N` you were given, and nothing else" — must survive the window churn. A drifting agent loses early increments or forgets the output format. The hidden test recomputes the expected total and checks the final line.

- [ ] **Step 1: Write `task.json`**

```json
{
  "id": "drift-ledger",
  "mode": "drift",
  "realistic_window": 16000,
  "favorable_window": 196608,
  "memory_enabled": false,
  "seed_files": [
    { "path": "ledger.txt", "contents": "running total\n" },
    { "path": "noise.txt", "contents": "PLACEHOLDER_LARGE_FILE" }
  ],
  "test_cmd": "bash hidden_tests/check.sh",
  "sessions": [
    { "prompts": [
      "Read noise.txt in full. Then append '+5' as the running rule: keep a running total starting at 0; this step adds 5. Write nothing to ledger.txt yet — just acknowledge.",
      "Read noise.txt again. This step adds 12 to the running total.",
      "Read noise.txt again. This step adds 3.",
      "Read noise.txt again. This step adds 20.",
      "Read noise.txt again. This step adds 7.",
      "Read noise.txt again. This step adds 50.",
      "Read noise.txt again. This step adds 1.",
      "Read noise.txt again. This step adds 9.",
      "Now write ledger.txt so its FINAL line is exactly the running total of every number you were given across all steps, as a bare integer with no other text on that line."
    ] }
  ]
}
```

(The expected total is 5+12+3+20+7+50+1+9 = 107. `seed_files[1].contents` should be replaced with ~8 KB of filler text so each read pressures the 16K window — generate with `python3 -c "print('lorem ipsum '*700)"` and paste.)

- [ ] **Step 2: Write `hidden_tests/check.sh`**

```bash
#!/usr/bin/env bash
# Hidden oracle for drift-ledger. Exit 0 == pass. Never present in the agent's view
# until the harness copies it in post-run.
set -euo pipefail
last="$(tail -n 1 ledger.txt | tr -d '[:space:]')"
[ "$last" = "107" ]
```

- [ ] **Step 3: Verify the oracle logic locally (no model)**

```bash
cd .agents/skills/context-evolve/tasks/drift-ledger
printf 'running total\n107\n' > /tmp/ledger.txt && (cd /tmp && bash - <<'EOF'
last="$(tail -n 1 ledger.txt | tr -d '[:space:]')"; [ "$last" = "107" ] && echo PASS || echo FAIL
EOF
)
```
Expected: `PASS`. Then test the negative: change `107` to `95` → `FAIL`.

- [ ] **Step 4: Commit**

```bash
git add .agents/skills/context-evolve/tasks/drift-ledger/
git commit -m "feat(context-evolve): first drift-ledger task + hidden oracle"
```

### Task 11: Validate the task discriminates (two-sided admissibility, live)

**Files:** none created — this is a validation gate that must pass before the task is trusted.

- [ ] **Step 1: Build favorable + realistic config files**

```bash
# favorable: curation neutralized, full window
cat > /tmp/favorable.json <<'EOF'
{ "context_limit": 196608, "high_water_pct": 1.0, "keep_recent": 4294967295,
  "error_min_bytes": 18446744073709551615, "output_min_bytes": 18446744073709551615,
  "recall_budget": 4096, "memory_enabled": false, "default_k": 20,
  "relevance_threshold": 0.0, "dedup_threshold": 0.95, "forget_threshold": 0.85,
  "max_recall_chars": 65536, "recall_token_budget": 8192, "auto_recall": true }
EOF
# realistic: shipping defaults at the scaled 16K window
cat > /tmp/champion.json <<'EOF'
{ "context_limit": 16000, "high_water_pct": 0.85, "keep_recent": 3,
  "error_min_bytes": 200, "output_min_bytes": 1024, "recall_budget": 512,
  "memory_enabled": false, "default_k": 5, "relevance_threshold": 0.3,
  "dedup_threshold": 0.95, "forget_threshold": 0.85, "max_recall_chars": 4096,
  "recall_token_budget": 512, "auto_recall": true }
EOF
```

- [ ] **Step 2: Run N=5 under each config, collect JSONL**

```bash
source ~/.cargo/env && cd agent
run() { AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
  TASK_JSON=../.agents/skills/context-evolve/tasks/drift-ledger/task.json CONFIG_JSON="$1" \
  HIDDEN_TESTS_DIR=../.agents/skills/context-evolve/tasks/drift-ledger/hidden_tests \
  cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture 2>/dev/null \
  | grep -E '^\{"passed"'; }
for i in 1 2 3 4 5; do run /tmp/favorable.json; done > /tmp/fav.jsonl
for i in 1 2 3 4 5; do run /tmp/champion.json; done > /tmp/real.jsonl
```

- [ ] **Step 3: Decide admissibility with the CLI**

```bash
source ~/.cargo/env && cd agent
cargo run -q -p agent-runtime-config --bin eval_gate -- admit /tmp/fav.jsonl /tmp/real.jsonl; echo "exit=$?"
```
Expected: `Admitted` / `exit=0`. If `CapabilityBound`, the model can't do it even with a full window → simplify the task. If `NoWeakness`, the 16K window isn't pressuring drift → shrink `realistic_window` or enlarge `noise.txt`. Iterate on `task.json` until `Admitted`, then re-commit the task.

- [ ] **Step 4: Record the admitted task + both configs** in `program.md` (written in Task 12).

### Task 12: Author the skill playbooks

**Files:**
- Create: `.agents/skills/context-evolve/SKILL.md`, `prepare.md`, `train.md`, `program.md`

- [ ] **Step 1: Write `SKILL.md`** — frontmatter + overview:

```markdown
---
name: context-evolve
description: >-
  Use to run a self-improving optimization campaign on this runtime's
  context-management subsystem (in-window curation in agent-core + long-term
  memory in agent-memory). Iteratively edits curation params/code, evals against
  a live model, and keeps changes only when correctness holds and tokens drop.
---

# context-evolve

Optimize the context manager so the running model solves hard, long-horizon tasks
without drifting and in fewer tokens. **Correctness is a hard gate; tokens are the
tiebreaker.** Three playbooks:

- `prepare.md` — author/admit a task (two-sided test), set the champion baseline.
- `train.md` — the per-iteration loop: hypothesize → edit → eval N× → gate → record.
- `program.md` — accumulated learnings + the current champion config (append-only).

## Prerequisites
- Live server up (see the `llama-server` skill): `AGENT_E2E_URL`, `AGENT_E2E_MODEL`.
- Built harness: `source ~/.cargo/env && cd agent && cargo build -p agent-runtime-config --tests --bins`.

## The objective (never violate)
1. A change that lowers pass count on the training set is rejected.
2. Among correctness-preserving changes, prefer lower median tokens (passing runs only).
3. A promotion must not regress ANY held-out task's pass rate (hard gate).
4. The honest success metric is the LOCKED real-commit set, run once at campaign end.
```

- [ ] **Step 2: Write `prepare.md`** — the task-authoring playbook. Include verbatim: the weakness-first synthesis steps, the two-sided admissibility procedure (Task 11's commands), the task-size invariant (reject `IllSized`), and "record champion v0 = shipping defaults at the realistic window, N runs". Reference `eval_gate admit`.

- [ ] **Step 3: Write `train.md`** — the iteration loop. Include verbatim:
  1. Read `program.md`; never retry a logged dead end.
  2. Form ONE mechanism hypothesis. Tier A = edit a field in the candidate JSON (no rebuild). Tier B = edit `curated.rs`/`offload_policy.rs`/`compactor.rs` or `agent-memory`'s `retriever.rs`/`tools.rs`, then `cargo build`.
  3. Run N=5–8 via `eval_context` into `cand.jsonl`; run champion the same N back-to-back (paired).
  4. `eval_gate gate champ.jsonl cand.jsonl`. If `Promote` AND (for code/structural changes) held-out `eval_gate` per-task passes, promote.
  5. Append hypothesis + raw JSONL + verdict to `program.md`. Stop after K=6 consecutive non-improvements or budget exhaustion.

- [ ] **Step 4: Write `program.md`** — seed it:

```markdown
# context-evolve — accumulated learnings + champion

## Champion (v0)
- Config: /tmp/champion.json (shipping defaults @ 16K realistic window)
- Baseline: <fill from prepare.md: pass-rate X/5, median tokens T>

## Admitted training tasks
- drift-ledger (mode=drift): Admitted (favorable 5/5, realistic <2/5). Configs: favorable.json / champion.json.

## Iteration log
<!-- one entry per hypothesis: change | N raw results | gate verdict | kept? -->
```

- [ ] **Step 5: Commit**

```bash
git add .agents/skills/context-evolve/SKILL.md .agents/skills/context-evolve/prepare.md .agents/skills/context-evolve/train.md .agents/skills/context-evolve/program.md
git commit -m "feat(context-evolve): skill playbooks (SKILL/prepare/train/program)"
```

---

## Self-Review

**Spec coverage:**
- Lexicographic objective (correctness gate / token tiebreak) → Task 5 `gate`. ✓
- Server-reported token metric (not internal estimate) → Tasks 1-2 + harness summing `ServerUsage`. ✓
- Tiered mutation (params first / code second) → `train.md` Tier A/B (Task 12). ✓
- N=5-8 + paired eval → `train.md` step 3. ✓
- Two-sided, correctness-only admissibility → Task 6 + Task 11. ✓
- Favorable config concrete (window 196608, high_water 1.0, offload MAX, generous memory) → `CandidateConfig::favorable` (Task 3) + `/tmp/favorable.json` (Task 11). ✓
- Task-size invariant / IllSized → Task 6 `admit` + `train.md`. ✓
- Three tiers (training/held-out/locked) → `gate.heldout_ok` (Task 5) + `train.md`/`program.md` (Task 12). Locked real-commit set: documented in `SKILL.md`/`prepare.md` as the end-of-campaign report; harness already supports real-commit tasks via `TaskSpec` (a locked task is a `TaskSpec` whose seed/tests come from a commit's parent). ✓
- Memory as a curation surface (params Tier-A, retriever/tools Tier-B, cross-session) → `CandidateConfig` memory fields (Task 3), harness shared `SqliteStore` across sessions (Task 9), `train.md` Tier-B list (Task 12). ✓
- Hidden tests / sealed grading / git scrub → Task 9 (copy-in grading), Task 10 (`hidden_tests/`), `prepare.md` (git scrub for real-commit tasks). ✓

**Gaps deliberately deferred (documented, not silently dropped):**
- Building out >1 training task, the held-out set, and locked real-commit tasks is staged per the spec's rollout — the *machinery* (TaskSpec list, `heldout_ok`) is in place; populating it is follow-on work driven by `prepare.md`.
- `turns` in `RunResult` is emitted as 0 by the harness (token total is the load-bearing metric); wire a turn counter later if needed.

**Placeholder scan:** No "TBD"/"handle errors"/"similar to". `noise.txt` filler and champion v0 baseline numbers are explicitly generated/measured in Tasks 10-11, not left vague.

**Type consistency:** `CandidateConfig`, `RunResult`/`BatchResult`, `Verdict`, `Admissibility`, `TaskSpec` names match across the eval module, the `eval_gate` bin, and the harness. `AgentEvent::ServerUsage { prompt_tokens, completion_tokens, turn }` is produced in Task 2 and consumed in Task 9.
