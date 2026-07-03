# Maintain-at-Start-of-Turn Curation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run `maintain()` before every model call (fixing the text-only-turn curation gap) with two guards — a trivial-chatter summarizer skip and a monotone prior-summary guard — that make per-turn maintenance non-destructive, then re-baseline the champion on every admitted eval task.

**Architecture:** `AgentLoop::run_with_cancel` moves its end-of-turn `maintain()` to start-of-turn (before `build()`); `CuratedContext::compact_old_span` gains (a) an early return for degenerate spans (all-assistant, <256 est tok) and (b) rejection of candidate summaries smaller than the prior they replace. Spec: `docs/superpowers/specs/2026-07-02-maintain-start-of-turn-curation-design.md`.

**Tech Stack:** Rust (agent/ Cargo workspace), tokio tests, ScriptedModel/CollectingSink testkit, context-evolve eval harness (`eval_context.rs`).

## Global Constraints

- `source ~/.cargo/env` before any cargo command; run cargo from `agent/` (its own workspace).
- Conventional commits `type(scope): summary`; commit per task; work stays on branch `evolve/maintain-start-of-turn`, NOT pushed.
- `cargo fmt` before `bash scripts/ci.sh` (clippy doc-lint traps).
- Constants: `TRIVIAL_CHATTER_SPAN_TOKENS = 256` (est tokens). Monotone guard uses strict `<` (equal-size candidate is accepted).
- The eval re-baseline is a hard gate for merge: no admitted task may lose a pass vs its v3 ceiling (see Task 6 table).

---

### Task 1: Relocate maintain() to start-of-turn (loop_.rs)

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (turn loop ~472-490; loop-bottom maintain ~949-963; new test + `SeqCtx` stub in the test module near `OverflowCtx` ~4680)

**Interfaces:**
- Consumes: `ContextManager::maintain`, `crate::MaintCtx`, existing test helpers `registry()`, `policy()`, `PassthroughProtocol`, `AlwaysApprove`, `ScriptedModel`, `CollectingSink`.
- Produces: the ordering invariant "maintain precedes every build/model-call"; text-only runs perform ≥1 maintain. No signature changes.

- [ ] **Step 1: Write the failing test**

Add to the `loop_.rs` test module (near `OverflowCtx`):

```rust
    /// Context stub recording the order of maintain() and build() calls.
    struct SeqCtx {
        history: Vec<Message>,
        calls: std::sync::Mutex<Vec<&'static str>>,
    }
    #[async_trait::async_trait]
    impl ContextManager for SeqCtx {
        fn append(&mut self, m: Message) {
            self.history.push(m);
        }
        fn set_system(&mut self, _: Message) {}
        fn set_recall(&mut self, _: Vec<String>) {}
        fn set_goal(&mut self, _: String) {}
        fn build(&self, _limit: usize) -> Vec<Message> {
            self.calls.lock().unwrap().push("build");
            self.history.clone()
        }
        async fn maintain(&mut self, _deps: &crate::MaintCtx<'_>) -> crate::MaintReport {
            self.calls.lock().unwrap().push("maintain");
            crate::MaintReport::default()
        }
        fn request_compaction(&mut self) {}
    }

    #[tokio::test]
    async fn text_only_turn_maintains_before_the_model_call() {
        // The text-reply exit used to return before the end-of-turn maintain,
        // so a pure chat run was never curated at all — only silently
        // truncated by build(). Start-of-turn maintenance must cover it, and
        // must run BEFORE the window is built so the model call sees the
        // curated context.
        let ws = std::env::temp_dir();
        let model = std::sync::Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "just a chat reply".into(),
        )]));
        let sink = Arc::new(CollectingSink::default());
        let agent = AgentLoop::new(
            model,
            Arc::new(PassthroughProtocol),
            registry(),
            policy(ws.clone()),
            Arc::new(AlwaysApprove),
            sink,
            LoopConfig {
                model_limit: 100_000,
                max_turns: 10,
                max_retries: 1,
                temperature: 0.0,
                max_tokens: None,
                workspace: ws,
                tool_timeout: std::time::Duration::from_secs(5),
                stream_idle_timeout: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        );
        let mut ctx = SeqCtx {
            history: vec![],
            calls: Default::default(),
        };
        agent.run(&mut ctx, "hello".into()).await.unwrap();
        let calls = ctx.calls.lock().unwrap().clone();
        assert!(
            calls.contains(&"maintain"),
            "a text-only run must still be curated: {calls:?}"
        );
        let m = calls.iter().position(|c| *c == "maintain").unwrap();
        let b = calls.iter().position(|c| *c == "build").unwrap();
        assert!(m < b, "maintain must precede the first build: {calls:?}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-core text_only_turn_maintains_before_the_model_call`
Expected: FAIL — `a text-only run must still be curated: ["build"]`

- [ ] **Step 3: Relocate the maintain block**

In `run_with_cancel`, immediately after the top-of-loop cancel check (before `let messages = ctx.build(...)`), insert:

```rust
            // Curate BEFORE building, so every model call sees a maintained
            // window — including the just-appended user prompt. Start-of-turn
            // also covers text-only turns, whose early `return` skipped the
            // old end-of-turn pass entirely: a pure chat conversation was
            // never curated at all, only silently truncated by build().
            let deps = crate::MaintCtx {
                model_limit: self.effective_model_limit(),
                model: self.maint_model(),
                sink: &self.sink,
                cancel: &cancel,
            };
            let report = ctx.maintain(&deps).await;
            if report.offloaded > 0 || report.compacted_turns > 0 {
                tracing::debug!(
                    offloaded = report.offloaded,
                    offloaded_bytes = report.offloaded_bytes,
                    compacted_turns = report.compacted_turns,
                    "context maintained"
                );
            }
```

Delete the now-duplicate block at the loop bottom (the `let deps = crate::MaintCtx { ... }; let report = ctx.maintain(&deps).await; if report.offloaded > 0 || ... { tracing::debug!(...) }` block that sits after the stuck-nudge append, just before the loop's closing brace). The overflow-recovery maintain inside the completion retry loop stays untouched.

- [ ] **Step 4: Run the crate tests**

Run: `cargo test -p agent-core`
Expected: PASS (including `overflow_compacts_rebuilds_and_recovers_once` — its `maintains >= 1` still holds; tool-bearing runs now maintain at each turn start).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs
git commit -m "feat(core): maintain context at start-of-turn, covering text-only turns"
```

---

### Task 2: Trivial-chatter summarizer skip (curated.rs)

**Files:**
- Modify: `agent/crates/agent-core/src/curated.rs` (`compact_old_span` ~line 315; new const near `PLACEHOLDER_UNIT_MAX_TOKENS` ~line 221; test module)

**Interfaces:**
- Consumes: `message_tokens`, `Role` (both already in scope), the durable/summarizable partition in `compact_old_span`.
- Produces: `const TRIVIAL_CHATTER_SPAN_TOKENS: usize = 256;` and the skip semantics Task 3/4 tests assume: an all-assistant span under the floor returns early with history and `compaction_summary` untouched.

- [ ] **Step 1: Write the failing test + the cadence-preservation test**

Add to the `curated.rs` test module:

```rust
    #[tokio::test]
    async fn trivial_assistant_chatter_skips_the_summarizer() {
        // A prior summary plus a handful of tiny acks must NOT re-run the
        // summarizer — each pass over `prior + trivia` risks degrading the
        // prior (observed collapsing the running summary to "No new
        // information provided" under per-turn maintenance). The chatter
        // simply accumulates until the span is substantial.
        let mut c = ctx();
        c.high_water_pct = 0.0; // pressure permanently on
        c.config.keep_recent = 1;
        c.compaction_summary = Some(Message::system(
            "Summary of earlier conversation:\nledger entries 1-12 recorded",
        ));
        for i in 0..4 {
            c.append(Message::assistant(format!("ok {i}"), None));
        }
        c.append(Message::user("next instruction"));
        let model: Arc<dyn ModelClient> =
            Arc::new(ScriptedModel::new(vec![Scripted::Text("DEGRADED".into())]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert_eq!(report.compacted_turns, 0, "trivial span must not compact");
        assert!(
            c.compaction_summary
                .as_ref()
                .unwrap()
                .content
                .contains("entries 1-12 recorded"),
            "prior summary untouched"
        );
        // The acks stay in history, awaiting a substantial span.
        assert!(c.history().iter().any(|m| m.content == "ok 0"));
    }

    #[tokio::test]
    async fn tiny_tool_bearing_span_still_compacts() {
        // The degenerate-span skip must NOT throttle tool-bearing spans: a
        // flat recompaction floor did exactly that and regressed
        // locked-portmap 10/10 -> ~4/6 (delayed compaction left no single
        // complete source at write time). Tool-bearing spans of any size
        // keep the per-turn cadence the eval ceilings were measured under.
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction
        c.config.keep_recent = 1;
        c.compaction_summary = Some(Message::system("Summary:\nbase"));
        c.append(parent("c1"));
        c.append(Message::tool(
            "c1",
            "shell",
            "a short tool result with a few extra words of output here",
        ));
        c.append(Message::assistant("done", None));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "Summary:\nbase plus the shell call output".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert!(
            report.compacted_turns > 0,
            "tool-bearing spans keep the per-turn cadence"
        );
    }
```

- [ ] **Step 2: Run tests to verify the red/green split**

Run: `cargo test -p agent-core trivial_assistant_chatter_skips_the_summarizer`
Expected: FAIL — summary replaced by "DEGRADED", `compacted_turns` == 4.
Run: `cargo test -p agent-core tiny_tool_bearing_span_still_compacts`
Expected: PASS already (documents current cadence; guards the skip's scope).

- [ ] **Step 3: Implement the skip**

Add the const below `PLACEHOLDER_UNIT_MAX_TOKENS` (~line 221):

```rust
/// Minimum estimated size of a PURE-ASSISTANT chatter span before it is
/// worth a summarizer pass. Re-running the summarizer over `prior + one
/// trivial ack` is generation loss — a small model degrades the prior
/// instead of extending it — and even without a prior it wastes a model
/// call. Tool-bearing spans are exempt at ANY size: their per-turn cadence
/// is load-bearing (a flat floor here regressed locked-portmap).
const TRIVIAL_CHATTER_SPAN_TOKENS: usize = 256;
```

In `compact_old_span`, immediately after the `if to_summarize.is_empty() { return; }` early return, add:

```rust
        // Degenerate span: pure assistant chatter too small to be worth a
        // summarizer pass (see TRIVIAL_CHATTER_SPAN_TOKENS). The chatter
        // accumulates until the span is substantial or gains a tool-bearing
        // unit; history and the prior summary stay untouched.
        let span_tokens: usize = to_summarize.iter().map(message_tokens).sum();
        let all_assistant = to_summarize
            .iter()
            .all(|m| matches!(m.role, Role::Assistant));
        if all_assistant && span_tokens < TRIVIAL_CHATTER_SPAN_TOKENS {
            return;
        }
```

- [ ] **Step 4: Fatten the three existing tests the skip legitimately batches**

These tests exercise compaction *mechanics* with chatter that is now (correctly) below the floor; raise their span sizes past 256 est tok so they keep testing what they were written for:

In `maintain_compacts_old_span_when_over_high_water` (~line 515):

```rust
            c.append(Message::assistant(
                format!("turn {i} {}", "with a fair bit of padding text here ".repeat(12)),
                None,
            ));
```

In `maintain_keeps_user_instructions_verbatim_through_compaction` (~line 544):

```rust
            c.append(Message::assistant(
                format!("ok, acknowledged {i}, {}", "lots of filler chatter ".repeat(15)),
                None,
            ));
```

In `maintain_keeps_offload_placeholders_verbatim_through_compaction` (~line 630):

```rust
            c.append(Message::assistant(
                format!("chatter {i} {}", "with plenty of padding text to summarize ".repeat(12)),
                None,
            ));
```

(`compaction_offloads_departing_tool_results_before_summarizing` needs no change: its boundary offload happens before the skip and it does not assert `compacted_turns`.)

- [ ] **Step 5: Run the crate tests**

Run: `cargo test -p agent-core`
Expected: PASS, including both new tests and the three fattened ones.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/curated.rs
git commit -m "feat(core): skip the summarizer for trivial all-assistant spans"
```

---

### Task 3: Monotone prior-summary guard (curated.rs)

**Files:**
- Modify: `agent/crates/agent-core/src/curated.rs` (`compact_old_span` `Ok(summary)` arm ~line 319; test module)

**Interfaces:**
- Consumes: `prior` (the cloned `compaction_summary`), `message_tokens`, `compaction_is_worthwhile`.
- Produces: the invariant "an accepted summary is never smaller (est tokens, strict) than the prior it replaces".

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn shrinking_summary_is_rejected_keeping_prior() {
        // The compaction prompt mandates a strict superset of the prior; a
        // candidate SMALLER than the prior is by definition a degraded pass
        // (the collapse mechanism under repeated re-compaction) and must be
        // discarded — prior kept, span left in history for a later pass.
        let mut c = ctx();
        c.high_water_pct = 0.0; // force compaction
        c.config.keep_recent = 1;
        let fat_prior = format!(
            "Summary of earlier conversation:\n{}",
            "ledger entry detail ".repeat(30)
        );
        c.compaction_summary = Some(Message::system(fat_prior.clone()));
        c.append(parent("c1"));
        c.append(Message::tool("c1", "shell", "output ".repeat(60)));
        c.append(Message::assistant("done", None));
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            "No new information provided".into(),
        )]));
        let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
        let cancel = CancellationToken::new();
        let report = c.maintain(&maint_deps(&model, &sink, &cancel)).await;
        assert_eq!(
            report.compacted_turns, 0,
            "shrinking candidate must be discarded"
        );
        assert_eq!(c.compaction_summary.as_ref().unwrap().content, fat_prior);
        // The span stays in history for a later, larger pass.
        assert!(c.history().iter().any(|m| matches!(m.role, Role::Tool)));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p agent-core shrinking_summary_is_rejected_keeping_prior`
Expected: FAIL — the tiny candidate is a huge token win, so `compaction_is_worthwhile` accepts it and the summary collapses.

- [ ] **Step 3: Implement the guard**

In the `Ok(summary) if compaction_is_worthwhile(&summary, &replaced)` arm, right after `let tokens_after = message_tokens(&summary);`:

```rust
                // Monotone prior guard: the compaction prompt mandates a
                // strict superset of the prior summary, so a candidate
                // smaller than the prior it replaces is a degraded pass —
                // the collapse mechanism under repeated re-compaction. Keep
                // the prior; the span stays in history and is retried once
                // it has grown. (`compaction_is_worthwhile` cannot catch
                // this: a collapsed summary looks like a huge token win.)
                if let Some(p) = prior.as_ref() {
                    if tokens_after < message_tokens(p) {
                        tracing::debug!("compaction shrank the prior summary; discarded");
                        return;
                    }
                }
```

- [ ] **Step 4: Run the crate tests**

Run: `cargo test -p agent-core`
Expected: PASS (in `tiny_tool_bearing_span_still_compacts`, the scripted candidate is longer than its prior, so the guard does not fire).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/curated.rs
git commit -m "feat(core): reject compaction summaries that shrink the prior"
```

---

### Task 4: Stress-test cadence bound + full suite

**Files:**
- Modify: `agent/crates/agent-runtime-config/tests/stress_context_management.rs` (~line 389, the `compactions >= 50` assertion)

**Interfaces:**
- Consumes: the Task 2 skip (each stress turn's summarizable span is one ~100-est-tok all-assistant ack, so compaction now batches ~every 3rd turn).
- Produces: nothing new — bound adjustment only.

- [ ] **Step 1: Run the stress test to observe the new cadence**

Run: `cargo test -p agent-runtime-config --test stress_context_management repeated_compaction`
Expected: FAIL — `compaction should fire most turns; got ~33`.

- [ ] **Step 2: Adjust the bound with the reason in a comment**

```rust
    // Compaction actually happened repeatedly (not a no-op test). It batches:
    // each turn's summarizable span is one ~100-est-tok assistant ack, below
    // TRIVIAL_CHATTER_SPAN_TOKENS, so the summarizer re-runs roughly every
    // 3rd turn — per-turn re-compaction over trivia degrades the running
    // summary (see curated.rs).
    assert!(
        compactions >= 25,
        "compaction should fire regularly (batched); got {compactions}"
    );
```

- [ ] **Step 3: Run both crates' suites**

Run: `cargo test -p agent-core && cargo test -p agent-runtime-config`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add agent/crates/agent-runtime-config/tests/stress_context_management.rs
git commit -m "test(runtime-config): batched compaction cadence bound under trivia skip"
```

---

### Task 5: fmt + CI gate

- [ ] **Step 1:** `cd agent && cargo fmt`
- [ ] **Step 2:** `cd .. && bash scripts/ci.sh` — Expected: green (fmt + clippy + agent tests + web typecheck/vitest). Fix anything it flags; commit fixes as `fix(core): ...` or fold trivial fmt into an amend of the touched-file commit ONLY if unpushed and same-file.
- [ ] **Step 3:** Commit any residue: `git add -A && git commit -m "chore(core): fmt/clippy for start-of-turn maintenance"` (skip if clean).

---

### Task 6: Eval re-baseline (hard merge gate)

Protocol details in the spec (`docs/superpowers/specs/2026-07-02-maintain-start-of-turn-curation-design.md`). This is a **baseline shift** — new-code champion vs the v3 ceilings, equal N, NOT a candidate promotion.

- [ ] **Step 1: Bring-up.** `curl -s localhost:8080/health` → `{"status":"ok"}` (relaunch command in the `local-llama-server` memory if down). `source ~/.cargo/env && cd agent && cargo build -p agent-runtime-config --tests`.
- [ ] **Step 2: Run every admitted task at its N** using the train.md template (absolute paths; `2>&1 | grep -E '^\{"passed"'`):

```bash
source ~/.cargo/env && cd agent
S=/home/kalen/rust-agent-runtime/.agents/skills/context-evolve/tasks
run() { AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
  TASK_JSON=$1/task.json CONFIG_JSON=$2 HIDDEN_TESTS_DIR=$1/hidden_tests \
  cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture 2>&1 \
  | grep -E '^\{"passed"'; }
# memory tasks additionally: EVAL_REAL_EMBEDDINGS=1 FASTEMBED_CACHE=/home/kalen/rust-agent-runtime/src-tauri/.fastembed_cache
```

| task | N | champion config | v3 ceiling |
|---|---|---|---|
| locked-portmap | 10 | `$S/drift-ledger/champion_v3.json` | 10/10 |
| drift-ledger | 6 | same | 6/6 |
| offload-recall | 5 | same | 5/5 |
| longhaul-codename | 5 | same | 5/5 |
| memory-recall (real emb) | 5 | memory-mode champion params (see task dir / program.md) | 5/5 |
| memory-roster (real emb) | 5 | same (k=10) | 5/5 |
| longhaul-manifest | 5 | `$S/drift-ledger/champion_v3.json` | 0/5 expected |

Confirm each task dir's exact config file names before running; record every JSONL line.
- [ ] **Step 3: Gate.** Acceptance = no task below its v3 ceiling (manifest may improve, not required). On regression: swap Task 1's ordering for the exit-path fallback (spec Alternatives), re-run the regressed task + full sweep; if still regressed, revert branch and log the dead end in program.md.
- [ ] **Step 4: Record.** New baseline block in `.agents/skills/context-evolve/program.md` (v3 code+config plus this fix; the numbers from Step 2; NOT a v4 unless correctness improved somewhere) + iteration-log entry. Commit: `docs(skill): re-baseline champion under start-of-turn maintenance`.

---

### Task 7: Merge + memory

- [ ] **Step 1:** `git checkout main && git merge --no-ff evolve/maintain-start-of-turn -m "Merge evolve/maintain-start-of-turn: start-of-turn curation + cadence guards (re-baseline recorded)"`
- [ ] **Step 2:** `git branch -d evolve/maintain-start-of-turn`
- [ ] **Step 3:** Update the `context-evolve-campaign-state` memory (+ MEMORY.md hook): phase 1 done, new baseline numbers, resume point = phase 2 (manifest re-attempt).
