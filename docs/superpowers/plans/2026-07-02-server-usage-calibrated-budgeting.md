# Server-Usage-Calibrated Budgeting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Feed server-reported prompt_tokens back into context budgeting (shrink-only effective limit) and make recall refresh unconditional.

**Architecture:** One `AtomicU64` fixed-point ratio on `AgentLoop`, sampled per successful completion (server truth ÷ chars/4 estimate, EMA 0.5, clamp [1.0, 4.0]), applied through a private `effective_model_limit()` at the five budgeting sites. Recall: one-line unconditional `set_recall`. All in `agent-core/src/loop_.rs`; zero wire/trait changes.

**Tech Stack:** Rust (workspace `agent/`).

**Spec:** `docs/superpowers/specs/2026-07-02-server-usage-calibrated-budgeting-design.md` — behavior authority; read fully.

## Global Constraints

- Run cargo from `agent/` (`source ~/.cargo/env` if missing). Conventional commits. `cargo fmt -p agent-core` before every commit (the previous cluster's gate failed once on fmt).
- `Usage` events keep emitting the CONFIGURED `context_limit`; only `build()`/`MaintCtx` receive the effective limit.
- No changes to `estimate_tokens`, `ContextManager` trait, wire types, or `MaintCtx` shape.
- `bash scripts/ci.sh` green at cluster end.

---

### Task 1: Calibrated effective limit

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` — AgentLoop struct (~100-110), constructor (~114-134), the completion epilogue after `ServerUsage` emit (~395-404), the five budgeting sites (build at ~391 and ~430; MaintCtx at ~423-428 and ~777-782), tests at bottom.

**Interfaces:**
- Produces: `fn effective_model_limit(&self) -> usize` (private), `AgentLoop.calib_ratio_micros: AtomicU64` (private, init 1_000_000). Nothing external.

- [ ] **Step 1: Failing tests.** A recording ContextManager stub (wrap or mirror the file's existing test context patterns; it records the `model_limit` arg of every `build` call and forwards to an inner `WindowContext` or returns canned messages):

```rust
/// Server-reported prompt_tokens 2× our estimate must shrink the NEXT turn's
/// build/maintain budget (EMA 0.5: 1.0 → 1.5 → …), never the Usage event's
/// configured context_limit (spec §1).
#[tokio::test]
async fn server_usage_shrinks_effective_budget() {
    // Scripted: turn 1 completion carries Chunk::Usage with prompt_tokens set to
    // exactly 2 * built_tokens(turn-1 request messages); turn 2 plain-text stop.
    // model_limit = 100_000.
    // Assert: recorder saw build(100_000) on turn 1 and build(≈66_666) on turn 2
    //   (100_000 / 1.5, ±1 for rounding);
    // sink Usage events all carry context_limit == 100_000.
}

#[tokio::test]
async fn zero_prompt_tokens_leaves_budget_configured() { /* backend reports 0 → build(100_000) every turn */ }

#[tokio::test]
async fn calibration_clamps_at_4x_and_never_grows() {
    // sample of 100× est → ratio clamps to 4.0 → build(25_000) floor;
    // then samples of 0.1× est → ratio decays but clamps at 1.0 → build(100_000), never above.
    // (EMA from 4.0 with 0.5-alpha needs several low samples; script enough turns.)
}
```

- [ ] **Step 2: Verify failing** — `cd agent && cargo test -p agent-core server_usage_shrinks` → FAIL.

- [ ] **Step 3: Implement.**

Struct + init:

```rust
/// Observed (server prompt_tokens / chars-4 estimate) ratio, EMA-smoothed,
/// fixed-point micros. 1_000_000 = 1.0 = uncalibrated. Shrink-only: clamped
/// to [1.0, 4.0] and applied as a divisor on model_limit (spec 2026-07-02
/// server-usage-calibrated budgeting).
calib_ratio_micros: std::sync::atomic::AtomicU64,
```

(constructor: `calib_ratio_micros: std::sync::atomic::AtomicU64::new(1_000_000)` — also in the `Default` impl if it constructs field-wise.)

Helper (near `maint_model`):

```rust
/// The configured window shrunk by the observed estimate-undercount ratio.
/// Never exceeds the configured limit; floor at 1/4 of it. chars/4 stays the
/// per-message currency; this makes the *budget* honest (audit Spine B #2).
fn effective_model_limit(&self) -> usize {
    let ratio = self.calib_ratio_micros.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6;
    ((self.config.model_limit as f64 / ratio) as usize).max(1)
}

fn record_calibration_sample(&self, server_prompt_tokens: u32, est: usize) {
    if server_prompt_tokens == 0 || est == 0 {
        return;
    }
    let sample = server_prompt_tokens as f64 / est as f64;
    let _ = self.calib_ratio_micros.fetch_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |old| {
            let old_f = old as f64 / 1e6;
            let new_f = (0.5 * old_f + 0.5 * sample).clamp(1.0, 4.0);
            if (new_f - old_f).abs() / old_f > 0.05 {
                tracing::debug!(old = old_f, new = new_f, "token-estimate calibration shifted");
            }
            Some((new_f * 1e6) as u64)
        },
    );
}
```

Wiring:
- Capture `let est_prompt_tokens = built_tokens(&messages);` where the Usage event already computes it (reuse the same value for the event and the sample; the overflow-recovery rebuild recomputes both — the variable must be reassigned there so the sample matches the FINAL request sent).
- After the successful completion (right where `ServerUsage` is emitted, ~395):
  `self.record_calibration_sample(assistant.prompt_tokens, est_prompt_tokens);`
- Replace `self.config.model_limit` with `self.effective_model_limit()` at EXACTLY: build (~391), overflow-recovery MaintCtx (~423-428) + its rebuild build (~430), end-of-turn MaintCtx (~777-782). Leave BOTH Usage-event `context_limit` fields (~394, ~435) and everything else (snapshot paths, dispatch) as `self.config.model_limit`.

- [ ] **Step 4: Verify** — `cargo test -p agent-core` full crate → PASS (property/eval suites downstream are limit-parameterized; nothing else changes).

- [ ] **Step 5:** `cargo fmt -p agent-core`, commit — `feat(core): server-usage-calibrated context budgeting — shrink-only effective model limit (audit Spine B #2)`

---

### Task 2: Unconditional recall refresh

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs:322-327`; tests at bottom.

- [ ] **Step 1: Failing test:**

```rust
/// A run that retrieves nothing must CLEAR the previous run's recall block —
/// contexts persist across runs (spec §2, audit Spine B #4).
#[tokio::test]
async fn empty_retrieval_clears_stale_recall() {
    // Retriever stub: first call returns vec!["fact A"], second returns vec![].
    // Real WindowContext (or CuratedContext) shared across two loop.run() calls
    // with a Scripted plain-text model.
    // Assert: after run 1 the context's build() contains "fact A";
    //         after run 2 it does NOT.
}
```

- [ ] **Step 2: Verify failing** — `cargo test -p agent-core empty_retrieval` → FAIL (stale block persists).

- [ ] **Step 3: Implement** — replace the conditional:

```rust
if let Some(retriever) = &self.retriever {
    ctx.set_recall(retriever.retrieve(&user_input).await);
}
```

- [ ] **Step 4: Verify** — `cargo test -p agent-core` → PASS (check no existing test depended on empty-retrieval preserving recall).

- [ ] **Step 5:** fmt, commit — `fix(core): unconditional set_recall — stale recall blocks clear when retrieval is empty`

---

### Task 3: Cluster gate

- [ ] `bash scripts/ci.sh` → green. No commit expected.
