# E2E lifecycle & stress suite — GUI, CLI, and surface-switching

**Date:** 2026-07-10
**Status:** Draft — pending adversarial panel + owner gate
**Depends on:** Phase 4B (durable HITL: park-point checkpoints, cross-process resume, CLI sessions list/reopen, bridge parked-run frames) — merged to main at `a6f2f3b`.

## 1. Problem & goals

Phase 4B made runs durable across processes and surfaces: a run can park at an
approval gate in the Tauri GUI, survive a kill, and be reopened from the CLI —
and vice versa. That cross-surface lifecycle is the newest, least-tested,
highest-risk surface in the runtime. Existing coverage is strong at the unit
and single-surface integration level (checkpoint MAC round-trips, re-emit
re-derivation, reap guards) but **nothing today drives two surfaces against the
same session on disk**, kills real processes at hostile moments, or races the
resume lock from two directions.

**Goals:**

- G1. An e2e suite that drives the CLI binary and the GUI's exact protocol
  surface (the WS bridge) against shared session state, including switching
  surfaces mid-lifecycle.
- G2. Deterministic stress coverage of the failure situations enumerated in §5
  — crashes, races, corruption, adversarial tampering, model misbehavior.
- G3. The deterministic tier joins the `ci.sh` pre-push gate (no model, no
  display, no GTK required).
- G4. A thin real-GUI (WebDriver) tier and a thin live-model tier as
  `--ignored` reality checks, matching existing conventions.

**Non-goals:**

- Not a performance/latency benchmark. "Stress" here means hostile state and
  event orderings, not load numbers.
- Not a replacement for existing unit/integration tests in
  `checkpoint.rs` / `loop_.rs` / `session.rs` / `resume.rs` — those stay; this
  suite covers only what they structurally cannot (multi-process,
  multi-surface, real kills).
- No new product behavior. Where current behavior is a known gap (e.g. stale
  `resume.lock` has no auto-recovery), tests assert the documented behavior
  and mark it as a product-change candidate — they do not change it.
- No CI provisioning for WebDriver (xvfb etc.); Tier 2 remains on-demand on
  this machine.

## 2. Architecture

### 2.1 Key enabling fact

The desktop app's bridge (`src-tauri/src/bridge.rs`) is a thin wrapper over
`agent_server::daemon::serve`, which lives in the **`agent/` workspace**.
Driving `agent-server`'s daemon directly exercises the byte-identical protocol
the GUI uses, without `src-tauri` (whose ci.sh leg is conditional on GTK
deps). This is what lets the cross-surface matrix be unconditionally CI-gated.

### 2.2 New crate: `agent/crates/agent-e2e`

A tests-only crate (empty or near-empty `lib.rs`; all substance in `tests/`
plus shared test-support modules). It owns three drivers and the scenario
matrix:

1. **Scripted model stub** — an in-process HTTP server speaking
   OpenAI-compatible `/v1/chat/completions`, driven by a per-test scenario
   queue: each incoming request pops the next scripted response. Response
   kinds: a tool call that triggers an approval gate, plain text completion,
   malformed JSON, a stalled/slow stream, a mid-stream connection drop.
   The stub **records all incoming requests** so tests can assert on what
   reached the model (e.g. deny-feedback text present in the next request).
   Both entry surfaces accept an arbitrary `base_url` for the `openai`
   backend (CLI flags; daemon launch params), so injection needs no code
   changes to the product.
2. **CLI driver** — spawns the real `agent` binary (workspace-built) as a
   subprocess with an isolated sessions root (`trace_dir` override in a
   per-test runtime config) and scripted stdin for the terminal approval
   prompt. Can deliver signals (`SIGKILL`, `SIGINT`) at chosen,
   *observably-synchronized* moments.
3. **Bridge driver** — starts `agent_server::daemon::serve` in-process on an
   ephemeral port against the same sessions root and exchanges JSON frames
   over `tokio_tungstenite`. "GUI restart" = tear down the daemon and serve
   again on the same root. Frame helpers cover: `user_input`,
   approval-resolution, and assertions on `parked_runs`, `approval_request`,
   `approval_resolved`, `resumed`, `event(done/error/token)` frames.

### 2.3 Isolation & determinism rules

- Every test gets a fresh tempdir workspace **and** a fresh tempdir sessions
  root. The CLI and bridge legs of one scenario share that root — that is
  what makes surface-switching real. No test touches `~/.rusty-agent`.
- No wall-clock sleeps as synchronization. Kill points and phase transitions
  synchronize on observable events: a file appearing (`parked.json`,
  `manifest.json`, `resume.lock`), a frame arriving, a stub request landing,
  or subprocess stdout markers. The stub controls all model-side timing.
- Tests are parallel-safe across each other (disjoint tempdirs, ephemeral
  ports); no global state.
- A test that needs "kill between checkpoint payload write and manifest
  write" gets that window via the stub (hold the model response that
  triggers the next step) or via filesystem watching — never via timing
  guesses. If a window proves impossible to hit deterministically from
  outside the process, the scenario is descoped to a synthesized on-disk
  state (write the torn state directly) — equally valid for testing the
  *reader's* behavior, and noted as such in the test.

## 3. Tiers

| Tier | What | Where | Gate |
|------|------|-------|------|
| 1 | Deterministic mock-backed matrix (§5) — CLI subprocess + in-process daemon | `agent/crates/agent-e2e/tests/` | **ci.sh, unconditional** |
| 2 | Real-GUI WebDriver checks of GUI-only surfaces | `src-tauri/tests/` beside `gui_smoke.rs`, reusing `e2e_harness` | `--ignored`, on-demand, serialized |
| 3 | Live-model reality check | `agent/crates/agent-e2e/tests/`, `--ignored` | on-demand, needs :8080 |

**Tier 2 content** (small, DOM-asserted, one WebDriver session per test,
`--test-threads=1`): parked-run banner appears after app restart with a parked
session on disk; approval prompt renders the re-derived intent; deny with
feedback flows from the real DOM; resume clears the banner. Session state is
pre-seeded on disk by harness code (not driven turn-by-turn through the DOM)
to keep each WebDriver session short.

**Tier 3 content**: one full-fidelity pass — the real model (qwen3.6 on
:8080) is prompted so it emits a genuinely approval-gated tool call; park via
bridge; reopen via CLI; approve; run completes. Accepts model nondeterminism
by allowing retry-with-stronger-prompt once before failing.

## 4. Budget

Whole Tier 1 target: **under ~2 minutes** on this machine, on top of the
existing `cargo test` build. If the matrix outgrows that, split a `soak`
subset behind `--ignored` before slowing the pre-push gate.

## 5. Scenario catalog

Each scenario names the invariant it guards; regression anchors reference the
bug/commit that motivated them. (Anchors are orientation only — locate code by
content, per AGENTS.md.)

### 5a. Cross-surface lifecycle

| # | Scenario | Invariant guarded |
|---|----------|-------------------|
| 1 | Park via bridge → CLI `sessions reopen` → approve → completes | attach-to-resume across surfaces; shared checkpoint semantics |
| 2 | CLI park-and-exit → bridge attach → `parked_runs` + re-derived `approval_request` → approve → `resumed` → done | re-derivation from stored args (never persisted display text); frame ordering (`resumed` before task output) |
| 3 | Park → **deny with feedback** on one surface → stub asserts feedback text in the next model request → repark → approve on the other surface → complete | MAC-bound deny feedback travels e2e |
| 4 | Deny→repark→reopen ×N (N≥3) alternating surfaces | tally-floor clamp — regression `2fad367` (deny desynced persisted tally; next checkpoint read as corrupt) |
| 5 | Cancel mid-resume (SIGINT on CLI; cancel path on bridge) → park retained → reopen succeeds | "Ok ≠ completed" reap guards — regressions `01179d8` (server), `76d81d5` (CLI) |
| 6 | Answer committed (`answer.json` written), process killed before resume runs → reopen consumes the committed answer without re-prompting, exactly once | answer durability + consumed-once; committed-root-answer path in reopen (`76d81d5`) |

### 5b. Crash & kill

| # | Scenario | Invariant guarded |
|---|----------|-------------------|
| 7 | Torn checkpoint (payload written, no manifest — synthesized or kill-window per §2.3) → reopen + list | manifest-written-last: torn checkpoint refused as corrupt with a clean error; `sessions list` unaffected |
| 8 | SIGKILL while `resume.lock` held → second reopen attempt | documented stale-lock behavior: explicit contention error, no corruption. Asserted as-documented; flagged as product-change candidate (no auto-recovery today) |
| 9 | Daemon torn down and re-served over existing session dirs → attach | epoch-prefixed ownership of orphaned dirs; `spawn_parked_reemit` re-emits parked runs from prior epochs |

### 5c. Concurrency & races

| # | Scenario | Invariant guarded |
|---|----------|-------------------|
| 10 | CLI reopen and bridge resume race the same parked run | exactly one wins `O_EXCL` on `resume.lock`; loser gets a clean, actionable error |
| 11 | Approve via bridge while CLI reopen is mid-prompt on the same ask | double-resolution safety: one resolution wins, the other observes resolved state (no double-resume, no panic) |
| 12 | Several parked + completed sessions → `sessions list` order and marks; reopen addresses the right session | epoch sort newest-first; parked marking; id addressing |
| 13 | Bridge attach while an ask is live vs attach after park | both re-emit paths (`reemit_pending` vs `spawn_parked_reemit`) produce a correct, single `approval_request` |

### 5d. Adversarial & corruption

| # | Scenario | Invariant guarded |
|---|----------|-------------------|
| 14 | Flip bytes in `parked.json` / `manifest.json` | HMAC verification refuses; both surfaces produce clean errors, not panics |
| 15 | Forged `answer.json` (wrong MAC) and legacy-format answer (pre-versioning) | fail-closed: answer discarded, re-prompt occurs |
| 16 | `descriptor.json` missing or regenerated | checkpoint MACs invalidated **cleanly** (refusal path, not crash) |
| 17 | Workspace directory deleted before reopen | workspace validation error; session dir left intact |

### 5e. General robustness

| # | Scenario | Invariant guarded |
|---|----------|-------------------|
| 18 | Stub drops the connection mid-stream | error surfaced to the surface in use; session not corrupted; next turn on same session works |
| 19 | Stub returns malformed output (invalid JSON; tool call naming a nonexistent tool) | loop error handling on both surfaces; no wedged session |
| 20 | Rapid-fire `user_input` frames while a turn is running | input handling stays serialized; no interleaved/corrupt transcript state |
| 21 | WS client disconnects mid-turn, reattaches | reattach shows consistent state (running, parked, or done — never wedged); live ask re-emitted |
| 22 | Soak: N park/approve cycles alternating CLI↔bridge on one session (N≥5) | no cumulative state drift; checkpoint stays verifiable; tally monotonicity holds |

### Deliberately not covered here

Covered by existing tests (do not duplicate): checkpoint MAC round-trip unit
coverage (`checkpoint.rs` tests), single-surface re-emit derivation
(`session.rs` tests), reap-guard state machine units (`loop_.rs`,
`resume_cleanup_decision` in `agent-cli`), answer-lifecycle units
(`resume.rs`).

Known coverage gaps this suite also does **not** take on (recorded for a
future phase): multi-ask dispatch-tree resume with multiple parked children;
workspace *moved* (not deleted) across restart.

## 6. Error handling & flakiness policy

- Zero tolerated flakes in Tier 1: any intermittent failure is a bug in the
  test (fix the synchronization) or in the product (file it), never retried
  into green.
- Tier 3 may retry once (model nondeterminism), and says so in its output.
- Tests asserting known-gap behavior (scenario 8) carry a
  `// PRODUCT-GAP:` comment so a future behavior change knows to update them
  deliberately.

## 7. CI integration

`agent/crates/agent-e2e` is a member of the `agent/` workspace, so the
existing `cargo test` leg in `scripts/ci.sh` picks it up with no new leg.
The CLI subprocess tests depend on the `agent` binary being built;
`cargo test` builds workspace bins for integration tests via Cargo's
convention (verify in planning: if bin-dependency isn't automatic, add an
explicit `cargo build -p agent-cli` before the test leg — a one-line ci.sh
change).

Tier 2 documentation lands in the auto-drive-tauri skill (a one-paragraph
pointer), since that skill is the discovery surface for GUI e2e on this
machine.

## 8. Open questions (for planning, not blocking)

- Exact mechanism for the CLI approval prompt scripting (pty vs plain stdin
  pipe) — depends on how `TerminalApproval` reads input; resolve in the plan
  by reading the source.
- Whether the daemon exposes a clean in-process shutdown for "GUI restart"
  teardown or the test drops the serve future; resolve in the plan.
- Stub implementation base: hand-rolled axum vs wiremock (repo already uses
  wiremock in `llama_health.rs`); pick whichever expresses scripted
  sequential responses + request recording more simply.

## Panel & review log

_(pending)_
