# E2E lifecycle & stress suite — GUI, CLI, and surface-switching

**Date:** 2026-07-10 (rev 2 — post-panel)
**Status:** Gate-approved 2026-07-10 (E1 adopted, E2 adopted, E3 gaps accepted, E4 queued) — feeding the implementation plan
**Depends on:** Phase 4B (durable HITL: park-point checkpoints, cross-process resume, CLI sessions list/reopen, GUI parked-run frames) — merged to main at `a6f2f3b`.

## 1. Problem & goals

Phase 4B made runs durable across processes and surfaces: a run can park at an
approval gate in the Tauri GUI, survive a kill, and be reopened from the CLI —
and vice versa. That cross-surface lifecycle is the newest, least-tested,
highest-risk surface in the runtime. Existing coverage is strong at the unit
and single-surface integration level (checkpoint MAC round-trips, re-emit
re-derivation, reap guards) but **nothing today drives two surfaces against the
same session on disk**, kills real processes at hostile moments, or contends
for the resume lock from two directions.

**Goals:**

- G1. An e2e suite that drives the CLI binary and the GUI's exact protocol
  surface — the `agent_server::session::Session` API plus its
  `ServerEvent` stream, which is precisely what the Tauri commands call and
  the webview consumes — against shared session state, including switching
  surfaces mid-lifecycle.
- G2. Deterministic stress coverage of the failure situations enumerated in §5
  — crashes, races, corruption, adversarial tampering, model misbehavior —
  with **real process kills** for the scenarios that require them (§5 marks
  them; the descope rule in §2.4 does not apply to those).
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
  multi-surface, real kills). §5 lists the cuts made to avoid duplication.
- No new product behavior, **except two small, explicitly declared test
  seams, gate-approved 2026-07-10** (§9 E1, E2). Everything else asserts
  current behavior; known gaps are asserted as-documented with a
  `// PRODUCT-GAP:` marker.
- No CI provisioning for WebDriver (xvfb etc.); Tier 2 remains on-demand on
  this machine.

## 2. Architecture

### 2.1 Key enabling fact (corrected by panel)

There is **no WebSocket bridge**. The former WS control plane was removed in
`7245526`; `agent-server` is a library crate and the desktop transport is
Tauri IPC. `src-tauri/src/bridge.rs` merely holds an
`Arc<Session>` built via `Session::from_params(agent_server::setup::local_params(...))`,
and the Tauri commands (`send_input`, `approve`, `cancel`, `subscribe`, …)
call `Session` methods directly, with events flowing out through an
`EventOut` sink as `ServerEvent` values (`agent-server/src/wire.rs`).

Consequence: the GUI's exact surface is drivable **in-process from the
`agent/` workspace** — construct a `Session` against a sessions root, register
an `EventOut` capture (the pattern `src-tauri/tests/smoke_context_explorer.rs`
already uses), call the same methods the Tauri commands call, and assert on
`ServerEvent` variants (`parked_runs`, `approval_request`,
`approval_resolved`, `resumed`, `token`, `done`, `error` — these wire names
are verified current). No ports, no frame (de)serialization helpers, and the
compiler — not hand-maintained fixtures — tracks protocol changes, because
the driver imports `agent_server::wire::ServerEvent`.

"GUI restart" = drop the `Session`, construct a new one over the same
sessions root (`spawn_parked_reemit` fires on subscribe/attach).

### 2.2 New crate: `agent/crates/agent-e2e`

A tests-only crate (near-empty `lib.rs`; substance in `tests/` + test-support
modules). Components:

1. **Scripted model stub** — **wiremock** (already a workspace dep, used by
   four crates; `agent-model`'s own tests stub SSE streams with it) serving
   OpenAI-compatible `/v1/chat/completions` from a per-test script.
   Discipline (panel-mandated):
   - each script step carries a **request matcher** (at minimum message
     count + an expected substring); a request that matches no step, or
     arrives past the script's end, **fails the test immediately**;
   - teardown asserts the script was **fully consumed**;
   - assertions on "what reached the model" check the **recorded request
     body text** (e.g. the deny-feedback string), never mere arrival.
   Response kinds: an approval-gated tool call (`write_file` gates
   deterministically under default policy: `Access::Write ⇒ Ask`), plain
   text, malformed JSON, bogus tool name, delayed-whole-response.
   **Mid-stream drop / stalled-mid-stream is not expressible in wiremock**
   (it sends the body as one canned buffer): scenario 20 uses a ~20-line raw
   `tokio::net::TcpListener` helper that sends partial SSE then closes.
   Naming/shape follows `agent_core::testkit`'s `Scripted*` precedent.
2. **CLI driver** — spawns the real `agent` binary as a subprocess with an
   isolated `HOME` (tempdir; see §2.3) and a runtime config setting
   `trace_dir`, `base_url` (stub), and low timeouts. Scripts the terminal
   approval prompt over a **plain stdin pipe** (verified: `stdin_prompt` uses
   `std::io::stdin().read_line`, no tty needed; reopen path takes any
   `BufRead`). Stdin rules (panel-mandated): the REPL and the approval
   prompt share one stdin, so scripts interleave task lines and answer lines
   deliberately, **hold the pipe open** across multi-line answers
   (deny + feedback = two lines), and never script a prompt *after* letting
   a previous prompt time out (the orphaned blocking reader steals the next
   line). Can deliver `SIGINT`/`SIGKILL` to the child at
   observably-synchronized moments.
3. **Session driver ("GUI leg")** — in-process `Session` construction per
   §2.1 for the bulk of the matrix, **plus a tiny harness binary**
   (`[[bin]]` in `agent-e2e`) that hosts the same Session driver as a real
   subprocess for the scenarios that must SIGKILL the daemon side (§5 marks
   them). The harness bin exists because in-process teardown runs `Drop`
   impls and graceful paths that a real death does not — and the daemon-side
   hard-death class is exactly where 4B-1's live drive found a real bug.
4. **Shared helpers are lifted, not copied**: the park-planting / ask-waiting
   / event-capture helpers that `agent-server`'s `session.rs` test mod
   already grew (`plant_parked_session`, `wait_for_ask_id`, capture sink)
   move into a `testkit` feature of `agent-server` (mirroring `agent-core`'s
   `#[cfg(any(test, feature = "testkit"))]` pattern) and are consumed from
   both places. MAC-forging/tampering helpers (§5d) take an explicit
   `key: [u8; 32]` parameter — **never** a path defaulting to
   `metadata_root()` — and live unexported in the tests-only crate.
5. **CLI binary freshness** — `CARGO_BIN_EXE_*` is only provided to the
   defining crate's own tests, and a bare `target/debug/agent` lookup can
   silently run a **stale** binary. The harness resolves the binary by
   invoking `cargo build -p agent-cli --quiet` once per test-binary run
   (`OnceLock`; a no-op when fresh) and reading the path from its JSON
   output (or via `escargot`) — bare target-path lookup is forbidden.
   Decide `cargo`-shellout vs `escargot` in the plan.

### 2.3 Isolation (corrected by panel)

`trace_dir` moves only the **sessions** root. The checkpoint HMAC secret
loads from `metadata_root()` = `$HOME/.rusty-agent/secret` on **both**
surfaces, so sessions-root isolation alone either splits the two surfaces
onto different keys (every cross-surface MAC check fails) or silently touches
the developer's real `~/.rusty-agent`. Neither is acceptable, and
process-global `env::set_var` inside a parallel test binary is racy.

**Mechanism (E1, gate-approved):** add a metadata-root override to
`RuntimeConfig` (mirroring `trace_dir`), honored by `metadata_root()`
consumers on both surfaces. It is a small, declared product seam — the one
exception to the no-product-changes non-goal, alongside E2. With it:

- every test gets a tempdir **metadata root** (secret), tempdir **sessions
  root**, and tempdir **workspace**; the CLI and Session legs of one scenario
  share all three;
- the harness **pre-creates the 32-byte secret file** before spawning
  anything (`load_or_create_secret` has a documented create race under
  concurrent first-touch);
- CLI subprocesses additionally get `HOME=<tempdir>` for belt-and-braces;
- no test reads or writes the real `~/.rusty-agent`, and tests are
  parallel-safe (disjoint tempdirs, no env mutation in-process).

(A subprocess-only fallback existed for the case of E1 being rejected; E1
was adopted at the gate, so the seam above is the mechanism.)

### 2.4 Determinism rules

- No wall-clock sleeps as synchronization. Kill points and phase transitions
  synchronize on observable events: a file appearing (`parked.json`,
  `manifest.json`, `resume.lock`), a `ServerEvent`, a stub request landing,
  or subprocess stdout markers.
- **Watchdog policy (panel-mandated):** every external wait — file
  appearance, event, subprocess exit — is deadline-bounded (~30s cap); on
  expiry the harness SIGKILLs the child's **process group**, fails the test,
  and prints captured stdout/stderr. Spawned CLIs always get explicit low
  `--stream-timeout-secs`. In a pre-push hook, a hang is strictly worse than
  a failure.
- **Process hygiene (panel-mandated):** spawned children are wrapped in a
  KillOnDrop guard (process-group kill via held `Child`, the
  `src-tauri/tests/e2e_harness` pattern); **no test ever kills by
  name/pattern** (pkill self-match footgun is repo history).
- **Synthesized-state rule:** a scenario may substitute hand-written on-disk
  state for a hard-to-hit crash window **only if** the state is produced by
  running the real writer up to a step boundary, or is pinned by a companion
  unit assertion on the writer's actual write order — and the descope is
  listed in the Panel & review log for owner sign-off, naming the real crash
  window it stands in for. Scenarios marked **[real-kill]** in §5 may not
  descope at all.
- **Positive-artifact rule:** checkpointing degrades silently to live-only
  when the secret/metadata root is unavailable, which would hollow out any
  scenario asserting only "no corruption". Every kill/reopen/tamper step is
  preceded by a positive assertion that `parked.json` exists.
- **Error assertions:** assert a stable short substring or discriminant plus
  two separate facts — "no panic" and "session dir intact" — never full
  error text, never bare `is_err()`.
- Gate-triggering tool calls must not *execute* files from the temp
  workspace (`/tmp` is noexec on this machine); `write_file` is the standard
  gated call.

## 3. Tiers

| Tier | What | Where | Gate |
|------|------|-------|------|
| 1 | Deterministic mock-backed matrix (§5) — CLI subprocess + in-process/harness-bin Session driving | `agent/crates/agent-e2e/tests/` | **ci.sh, unconditional** |
| 2 | Real-GUI WebDriver checks of GUI-only surfaces | `src-tauri/tests/` beside `gui_smoke.rs`, reusing `e2e_harness` | `--ignored`, on-demand, serialized |
| 3 | Live-model reality check | `agent/crates/agent-e2e/tests/`, `--ignored` | on-demand, needs :8080 |

**Tier 2 content** (small, DOM-asserted, one WebDriver session per test,
`--test-threads=1`):

- T2.1 parked-run banner appears after app restart with a parked session on
  disk; **Approve is clicked in the real DOM** and the banner clears.
- T2.2 deny **with feedback** typed in the real DOM reaches the checkpoint.
- T2.3 **real cross-surface switch** (panel addition): a run parks via a live
  turn driven from the real GUI DOM, then `agent sessions list` sees it and
  `agent sessions reopen` resumes it — guarding the `src-tauri` parameter
  plumbing (`local_params` wiring, workspace switching) that Tier 1's
  in-process driver bypasses.

Tier 2 requires launching the app with its sessions/metadata roots pointed at
temp dirs (via E1's seam + config file; exact env/config injection for the
WebDriver-launched app is a plan-verification item). If the app cannot be
pointed away from the real roots, Tier 2 state seeding is **descoped, not
worked around** — no harness code ever loads the user's real secret.

**Tier 3 content**: one full-fidelity pass — the real model (qwen3.6 on
:8080) is prompted to `write_file` (deterministically gated), parks via the
Session driver, reopened via CLI, approved, completes. One
retry-with-stronger-prompt allowed; says so in output.

## 4. Budget

Whole Tier 1 target: **under ~2 minutes** on this machine on top of the
existing build. Enforcement, not hope (panel-mandated): the plan implements
~3 representative scenarios first, measures, and extrapolates before writing
the rest; the soak (§5 #4) is pre-committed at N=4 and is the designated
first candidate for an `--ignored` split if measured Tier 1 exceeds ~90s.
Note the new crate also joins the workspace `clippy --all-targets` and
`cargo test` legs (compile-time tax) — acceptable, but measured.

## 5. Scenario catalog

**[real-kill]** = must deliver a real signal to a real subprocess; §2.4's
descope rule is unavailable. Anchors are orientation only — locate code by
content (AGENTS.md).

### 5a. Cross-surface lifecycle

| # | Scenario | Invariant guarded |
|---|----------|-------------------|
| 1 | Park via Session driver → CLI `sessions reopen` → approve → completes | attach-to-resume across surfaces; shared checkpoint semantics |
| 2 | CLI parks via timeout park-and-exit (E2 knob, gate-approved) → fresh Session attach → `parked_runs` + re-derived `approval_request` → approve → `resumed` → done | re-derivation from stored args; frame ordering; CLI→GUI direction |
| 3 | Park → deny **with feedback** on one surface — including a hostile-content variant (multibyte unicode, control chars, JSON-meta chars, ~10KB) → stub *matcher* asserts the feedback text in the recorded next request → repark → approve on the other surface → complete | MAC-bound deny feedback travels e2e; canonicalization/serialization under hostile input |
| 4 | **Soak** (merges old #4+#22): N=4 lifecycle cycles alternating deny-with-feedback and approve, alternating CLI↔Session surfaces, on one session; one cycle carries a large artifact store (multi-MB checkpoint) | tally-floor clamp (regression `2fad367`); tally monotonicity; no cumulative drift; checkpoint stays verifiable at size |
| 5 | Cancel mid-resume (**[real-kill]** SIGINT on CLI reopen; cancel path on Session) → park retained → reopen succeeds | "Ok ≠ completed" reap guards — regressions `01179d8`, `76d81d5`. CLI leg targets the **reopen** flow (the plain REPL path never claims `resume.lock`) |
| 6 | Answer committed (`answer.json` on disk), **[real-kill]** SIGKILL before resume runs → reopen consumes the committed answer without re-prompting, exactly once | answer durability + consumed-once (`76d81d5`) |
| 7 | Park under `openai`/native → reopen with divergent flags (`--backend claude-cli` forces prompted protocol) | park/resume config-divergence behavior is *defined and asserted* (current behavior documented; `PRODUCT-GAP` if it silently diverges) — reopen takes workspace from the descriptor but model/backend/protocol from current flags |
| 8 | ApproveAlways granted → restart (drop+rebuild Session) → identical tool call | the 4B deliberate downgrade: persisted "always" arrives as plain approve; the next identical call must **park again**, never silently auto-approve (security-relevant) |
| 9 | `send_input` of a *new* task on a session holding a parked run (not mid-turn) | defined behavior asserted: the park must survive or be resolved deliberately — never silently clobbered/orphaned (the GUI banner makes this the most reachable hostile ordering) |

### 5b. Crash & kill

| # | Scenario | Invariant guarded |
|---|----------|-------------------|
| 10 | Torn checkpoint (payload without manifest — per §2.4 synthesized-state rule) → reopen + `sessions list` | manifest-written-last: torn tree refused as corrupt with clean error; list unaffected |
| 11 | **[real-kill]** SIGKILL the harness-bin daemon while it holds `resume.lock` → second resume attempt from CLI | documented stale-lock behavior: explicit contention error naming the lock path; no corruption. `PRODUCT-GAP:` no auto-recovery today |
| 12 | **[real-kill]** SIGKILL the harness-bin daemon mid-run after park → start a new harness-bin process over the same roots → attach | orphaned-dir ownership across a real process boundary; `spawn_parked_reemit` re-emits; assert the new process's *classification* of the prior dir, not just the re-emitted event |
| 13 | Sessions root made read-only (chmod) before the park write | park-write failure is clean: run errors or degrades per documented behavior; the ask is not wedged; no torn tree |

### 5c. Concurrency (directed loser-paths, panel-reworked)

| # | Scenario | Invariant guarded |
|---|----------|-------------------|
| 14 | Session driver holds `resume.lock` → CLI reopen attempts | loser path, deterministic: CLI prints the contention guidance; park intact |
| 15 | CLI reopen holds the lock (mid-prompt, pipe held open) → Session resume attempts; then approve lands on the CLI | reverse loser path + double-resolution: one resolution wins, the other observes resolved state. Plan first verifies what the Session path does under a held lock (the CLI claims the lock *before* prompting, so mid-prompt contention may be structurally excluded — if so, assert that exclusion) |
| 16 | One barrier-synchronized genuine race (both contenders released simultaneously) | **symmetric postconditions only**: exactly one success, one clean error, park consumed exactly once — either order accepted |
| 17 | Several parked + completed sessions → reopen addresses the chosen one through the real binary | id addressing e2e (list marks/order stay unit-covered — `list_sessions_marks_parked`) |

### 5d. Adversarial & corruption

| # | Scenario | Invariant guarded |
|---|----------|-------------------|
| 18 | **Corruption sweep** (merges old #14/#15/#16): one parked session; iterate {tampered `parked.json` bytes, tampered `manifest.json`, forged `answer.json` MAC, legacy-format answer, missing `descriptor.json`} → assert per §2.4 error style on **both** scan surfaces (CLI reopen and Session attach diverge above the shared verifier) | e2e delta over existing unit coverage: each *surface* refuses cleanly — no panic, session dir intact, other sessions unaffected; forged answer ⇒ re-prompt occurs |
| 19 | Workspace directory deleted before reopen | workspace validation error; session dir left intact |

### 5e. General robustness

| # | Scenario | Invariant guarded |
|---|----------|-------------------|
| 20 | Mid-stream connection drop (raw-socket helper, not wiremock) | *recovery* signal: session not corrupted; next turn on the same session works (error-surfacing itself is covered by `e2e_robustness` T4 — not re-asserted on both surfaces) |
| 21 | Malformed model output (invalid JSON; tool call naming a nonexistent tool) | no wedged session; clean error; next turn works |
| 22 | Event-sink detach/reattach mid-turn (`set_event_out` swap — the Tauri Channel re-subscribe path) | consistent state on reattach (running/parked/done — never wedged); live ask re-emitted; guards the 4B-1 sync-subscribe abort class (`b86e21c`) |

### Cut as duplicate coverage (panel-verified against existing tests)

- old #13 (both re-emit paths): `attach_reemits_parked_ask_with_rederived_display`,
  `attach_sends_parked_runs_snapshot`, `attach_skips_reemit_for_already_resuming_session`
  already cover it at the same (now in-process) level; the cross-process half
  lives in #12.
- old #20 (rapid input during turn): `second_input_during_run_is_busy` is
  this test; no transport exists to add behavior.
- old #12's list-marking half, old #15's forgery mechanics, old #14/#16's
  refusal mechanics: unit-covered; only the surface-level deltas survive
  (in #17, #18).

### Recorded gaps — accepted at gate (E3, 2026-07-10)

Multi-ask dispatch-tree resume (multiple parked children); workspace *moved*
(not deleted) across restart. (A third candidate — the timeout-park arm — is
moot: E2 adopted the knob, so scenario 2 covers that arm.)

## 6. Error handling & flakiness policy

- Zero tolerated flakes in Tier 1: an intermittent failure is a bug in the
  test (fix the synchronization) or the product (file it) — never retried
  into green.
- Tier 3 may retry once (model nondeterminism), and says so in its output.
- Known-gap assertions carry `// PRODUCT-GAP:` (scenarios 7, 11 today).
- Watchdog, process-hygiene, artifact, and error-assertion rules: §2.4.

## 7. CI integration

`agent/crates/agent-e2e` joins the `agent/` workspace (`members = ["crates/*"]`
picks it up), so the existing `cargo test --workspace` leg in `scripts/ci.sh`
runs it, and `clippy --workspace --all-targets -D warnings` gates it.
Binary freshness is the harness's job (§2.2 item 5) — **not** a bare ci.sh build
line, so the guarantee holds in dev loops too, not just CI.

Tier 2 documentation lands in the auto-drive-tauri skill. Separately (E4),
that skill's "Bridge wire protocol" section still describes the removed WS
transport and misled rev 1 of this spec — it needs a factual de-stale pass,
tracked as its own follow-up.

## 8. Open questions (for planning, not blocking)

- Scenario 15: what the Session resume path does under a held lock — verify
  before promising double-resolution coverage; assert structural exclusion if
  that's the truth.
- Scenarios 7, 9, 13: current behavior is asserted, but the plan must first
  *characterize* it (read the code, run it) and mark anything surprising as
  `PRODUCT-GAP` + gate escalation rather than blessing it silently.
- Tier 2: how to inject config/env into the WebDriver-launched app
  (tauri-driver capabilities vs wrapper script); if impossible, T2 seeding
  descopes per §3.
- Binary-freshness mechanism: `cargo build` shellout vs `escargot`.

## 9. Gate decisions (decided 2026-07-10)

- **E1 — metadata-root override** (small product seam in `RuntimeConfig` +
  `metadata_root()` consumers), required for cross-surface secret sharing and
  true isolation (§2.3): **ADOPTED**.
- **E2 — CLI approval-timeout knob** (the terminal park-and-exit arm is
  hardwired to 300s, unreachable in a <2-min suite): **ADOPTED** — small
  config/flag knob; scenario 2 tests the real timeout-park-and-exit arm.
- **E3 — accepted gaps**: both gaps in §5 "Recorded gaps" **ACCEPTED**
  (multi-ask dispatch-tree resume; workspace moved across restart).
- **E4 — de-stale the auto-drive-tauri skill** (normative doc, separate pass
  with its own review per AGENTS.md): **APPROVED as follow-up**, not part of
  this suite's plan.

## Panel & review log

**2026-07-10 — adversarial spec panel (4 reviewers: requirements,
assumptions, failure/abuse, scope/simpler-design). All four: REWORK.
Synthesis applied as rev 2.**

**Blockers/majors fixed in place:**
- False §2.1 premise (WS `daemon::serve` removed in `7245526`) — all four
  reviewers; §2 rewritten around in-process `Session` + `EventOut`; scenarios
  20/21/old-#20 re-expressed; verified against live source by the
  orchestrator before rework.
- HMAC-secret isolation contradiction — rewritten as §2.3 + escalation E1.
- Park-and-exit 300s hardwired — scenario 2 now gated on E2.
- No daemon-side real kill in-process — harness bin added (§2.2 item 3);
  [real-kill] markers added; descope loophole closed (§2.4).
- Stale-binary misleading green — §2.2.5 freshness rule.
- Stub-desync/stdin discipline — §2.2 items 1–2; watchdog/orphan-process
  discipline — §2.4.
- Forging-helper hygiene + Tier 2 real-state pollution — §2.2 item 4, §3.
- Nondeterministic races — §5c reworked to directed loser-paths + one
  symmetric-postcondition race.
- Missing scenarios added: config/protocol divergence (#7), ApproveAlways
  downgrade (#8), input-while-parked (#9), park-write failure (#13),
  hostile-content feedback (in #3), real-GUI cross-surface switch (T2.3).
- Duplicate coverage cut/merged: old #13, #20 cut; #14/15/16 → #18;
  #4+#22 → #4 soak; #12, #18 trimmed to their e2e deltas.
- wiremock-vs-axum conflict resolved: wiremock default; raw-socket helper
  only for mid-stream drop (which wiremock cannot express).
- testkit lifting over copying (§2.2 item 4); budget enforcement (§4).

**Escalated to the gate:** E1, E2, E3, E4 — all decided at the owner gate
2026-07-10 (E1 adopt, E2 adopt-knob, E3 accept both gaps, E4 approved as
follow-up); dispositions recorded in §9 and folded into §1/§2.3/§5.

**Minors accepted as residual:** epoch-classification assertion depends on
plan-time verification of the ownership mechanism (#12); error-substring
choices deferred to implementation under §2.4's style rule; Tier 2 config
injection unresolved until plan (§8).

**2026-07-10 — Task 13 implementation (scenarios 5-6, `agent-e2e/tests/crashkill.rs`):**
§2.4 descope: scenario 6b's commit-window state (an answer committed but not
yet consumed) is produced by the REAL writer, `agent_core::checkpoint::write_answer`
(the same function `agent_server::resume::commit_answer` — and therefore
`agent-cli`'s `run_sessions_reopen` — bottoms out into), standing in for a
process killed between commit and consume; the window named is
write_answer→take_answer in reopen. Owner sign-off requested at branch review.

**2026-07-10 — Implementation notes (Task 22, whole-branch close-out):**

(a) **Budget outcome.** Tier 1 (`cargo test -p agent-e2e`, non-ignored) is
**33 tests across 6 binaries in ~32s wall** on this machine (`lifecycle.rs`
8, `robustness.rs` 4, `crashkill.rs` 8, `concurrency.rs` 4, `adversarial.rs`
2, `lib.rs`/`live_smoke.rs` 0+1-ignored) — comfortably inside §4's ~2min
target. The soak (§5#4, N=4) was **not** split to `--ignored`; §4's
pre-committed 90s trigger for a split was never reached.

(b) **Characterize-then-pin divergences from the spec's assumed shapes**
(§8's characterize-then-pin instruction for scenarios 7, 9, 13, 18, 21 —
plus 5/20, discovered during implementation):
- **Scenario 5** split into two tests, non-overlapping:
  `s05_sigint_mid_resume_cancels_clean_no_park_to_retain` (SIGINT during a
  plain-text resume step, no new ask — exits cleanly with "run cancelled",
  no park/lock artifact left; this is the accepted single-execution
  tradeoff, not the C-1 bug) and
  `s05b_sigint_while_reparked_at_new_ask_retains_park` (SIGINT while a
  *second* gated tool call has just re-parked at a genuinely new ask — the
  `Retain` arm finds the park and prints "run left parked; answer later
  with: agent sessions reopen `<id>`", the actual behavior `76d81d5`/
  `01179d8` fixed). Both pinned; the spec's single scenario 5 undersold two
  materially different outcomes depending on which resume step is
  interrupted.
- **Scenario 20** (mid-stream drop): production wiring hardcodes
  `max_retries: 3` with no test-facing seam to lower it. `RawDropStub`'s
  partial-SSE-then-close is classified `ModelError::Stream(_)` →
  `ErrorClass::Retryable`, so `completion_with_retry` absorbs the drop
  in-process — it emits `AgentEvent::StreamRetry`, retries, and the very
  first `send_input` already reaches `Done` with no `Error` event ever
  surfacing to the Session/GUI layer. `StreamRetry` is exercised and
  pinned; the scenario is complementary to `e2e_robustness`'s existing T4
  (which covers error-surfacing itself) rather than duplicating it, per
  the spec's own note.
- **Scenario 21** — the `MalformedJson` case was **not skipped**, but its
  outcome is one layer earlier than assumed: `openai.rs`'s stream loop
  treats a bad `data:` JSON line as transient corruption
  (`Some(Err(ModelError::Decode(e))) => continue`), so the malformed line
  is dropped and the following `data: [DONE]` still terminates cleanly —
  turn 1 is a clean `Done{reason:"stop"}` with empty text, not a
  session-level `Error` at all. Pinned as-is (`s21_malformed_and_bogus_tool`
  turn 1); turns 2-3 (bogus tool name, then a normal turn) proceed as the
  spec assumed.

(c) **PRODUCT FINDINGS pinned with `PRODUCT-GAP` for owner follow-up**
(none block this branch; all are asserted-current-behavior, not bugs
introduced by this work):
- **Scenario 7** (`s07_reopen_with_divergent_protocol_is_defined`):
  `sessions reopen --protocol=prompted` against a park whose model backend
  actually speaks native tool-calling **silently drops the second tool
  call and exits 0** — no error, no approval prompt; the wire layer
  believes the script was fully consumed. Confirmed via a positive/negative
  disk-artifact pair (the first, pre-approved write lands; the
  protocol-mismatched second write never does). Reopen accepts
  protocol-divergent flags without validation.
- **Scenario 15** (`s15_cli_holder_session_loser_then_cli_completes`):
  double-resolution is **not structurally excluded**. The CLI's pre-prompt
  `resume.lock` claim only blocks a second *resume attempt*
  (`start_resume`'s own `claim_resume`) — it does not stop a second surface
  from re-deriving the same parked ask and durably committing its own
  `answer.json` (a real, valid, MAC-bound write, not a no-op). What
  actually arbitrates is the lock: whichever process holds `resume.lock`
  is the one whose `take_answer` reads `answer.json` and drives the loop;
  the loser's `start_resume` call loses the lock race and surfaces
  `ServerEvent::Error` ("is being resumed elsewhere") without ever
  reaching `Done`. Net effect for the run itself: no panic, no
  double-execution, only one process ever completes — but the
  *answer-commit* race is real, and a future caller must not assume the
  first writer's answer is the one that is acted on.
- **Scenario 11** (stale `resume.lock`, no auto-recovery): already a known,
  pre-documented gap (§6 lists it), not a new finding — reconfirmed as-is
  by `s11_stale_lock_after_real_kill_refuses_contention`.

(d) **Tier 2 and Tier 3 live runs.** Tier 2 (`src-tauri/tests/gui_lifecycle.rs`,
T2.1/T2.2/T2.3) ran live against the real WebKitGTK webview, `tauri-driver`,
and `llama-server` — **3/3 pass**, 32.22s serialized
(`--test-threads=1`); state relocation via `HOME`/`XDG_*` env on the
`tauri-driver` spawn worked with no product-code seam needed, so **no
descope** (§3's contingency did not trigger). Tier 3
(`agent-e2e/tests/live_smoke.rs`) ran live against the real model twice
back-to-back — **2/2 pass**, ~11-14s each, first-attempt park both times
(the one allowed retry never fired). Both are recorded here as run at
least once on this machine per the branch-finish criterion; neither is
part of the `ci.sh` gate (Tier 1 only, per §3/§7).
