# context-evolve — accumulated learnings + champion

Append-only research memory. The loop reads this first every iteration and never
retries a logged dead end.

## Champion (v4) — promoted 2026-07-03 (Tier-B: extractive fold to the goal-block ledger) — CURRENT

- **Config:** `tasks/drift-ledger/champion_v4.json` — byte-identical to v3/v2
  params; the promotion is CODE. Spec:
  `docs/superpowers/specs/2026-07-03-extractive-fold-pinned-ledger-design.md`.
- **The mechanism (curated.rs + compactor.rs):** when `plan_retention` would
  evict a `Role::User` unit, the OLDEST user units (down to
  `USER_FOLD_LOW_WATERMARK_PCT = 0.25` of the window, `keep_recent` tail
  untouched) are **folded**: one `run_extraction` model call (dedicated
  fact-extraction prompt) turns them into compact verbatim fact lines; the
  lines are appended to a **numbered ledger rendered INSIDE the goal block**
  ("copy EVERY numbered line" task-conditional directive); the verbatim
  originals go to the offload store (id advertised, non-eliciting); the units
  leave history. All-or-nothing per batch (extraction failure = retry next
  maintain); ledger capped at `FOLDED_FACTS_MAX_TOKENS = 512` (oldest lines
  drop; originals stay recoverable). Enabled by the 2026-07-03 baseline shift:
  the text-only-exit maintain is what lets folds fire during chat-only ack runs
  BEFORE the model call that needs them.
- **THE MANIFEST GAP IS CLOSED: longhaul-manifest 0/5 → 5/5, with 20/20 correct
  entries in every run** (median 77,620 tok — below even favorable's ~92–118K
  passing reference). Extraction fidelity was perfect in every observed fold
  (9+8, 10+6 entries, zero errors).
- **Guard sweep (equal N, same night, no regression):** portmap **10/10**
  (52,211), codename **5/5** (53,931 — folds DO fire here on the filler turns;
  goal-block dilution did not hurt), offload **5/5**, mem-recall **5/5**,
  mem-roster **9/10** (== the baseline's own 9/10; noise), drift **5/6 then
  6/6 re-run = 11/12** — the single miss wrote all 8 increments correctly and
  then INVENTED a ninth step (+9 → 116): perfect context fidelity, model-bound
  generation, the documented drift failure mode.
- Ships with unit tests: fold trigger/extraction/removal, no-op when users
  fit, extraction-failure leaves history intact, ledger cap drops oldest,
  ledger survives compaction, ledger rides inside the goal block.

## Baseline shift (2026-07-03) — text-only-exit curation + summarizer guards — CURRENT BASELINE

**Champion remains v3 (code + config); this is a baseline shift like calibrated
budgeting, not a promotion.** Merged from `evolve/maintain-start-of-turn` (spec:
`docs/superpowers/specs/2026-07-02-maintain-start-of-turn-curation-design.md`).
Closes open issue #1 of the overflow-fold round: `maintain()` never ran on
text-only turns (chat-only sessions were never curated, only silently truncated
by `build()`).

**What shipped (3 coupled changes):**
1. **Text-only-exit curation (`loop_.rs`).** A run that ends in a text reply now
   maintains at the exit, before `Done` — but ONLY when no loop-bottom maintain
   fired this run (`run_maintained` flag). Tool-bearing runs keep the exact v3
   maintenance cadence by construction; pure chat runs are curated once per run.
2. **Trivial-chatter skip (`curated.rs`).** The summarizer is skipped when the
   span is all-assistant AND < `TRIVIAL_CHATTER_SPAN_TOKENS` (256 est tok) —
   ack chatter accumulates instead of degrading the prior via per-turn re-passes.
   Tool-bearing spans are exempt at ANY size (a flat floor here broke portmap in
   the attic round). An explicit `request_compaction()` (overflow recovery)
   bypasses the skip.
3. **Monotone prior guard (`curated.rs`).** A candidate summary smaller (est tok,
   strict) than the prior it replaces is discarded — a shrinking "superset" is a
   degraded pass; collapse ("No new information provided") is now impossible by
   construction, not just unlikely.

**Re-baselined numbers (gated binary, window 4000, 2026-07-03 night):**
- locked-portmap **10/10** (median pass 57,239 tok; paired v3 same night 10/10 @ 53,669)
- drift-ledger **6/6** (55,822), offload-recall **5/5** (34,299),
  longhaul-codename **5/5** (53,932), memory-recall **5/5** real-emb (19,906)
- memory-roster **9/10** real-emb k=10 (67,944; paired v3 10/10 @ 69,746). The
  single miss stored 7/8 facts — a session-1 slip at fact #3 whose preceding
  runs all had remember tool turns, i.e. a byte-identical-to-v3 code path up to
  the failing call. Attribution: server nondeterminism (llama.cpp batching is
  not bit-deterministic at temp 0), NOT the change. Roster shows a ~5-10%
  per-batch storage-slip rate across ALL non-start-of-turn code states.
- longhaul-manifest **0/5** — unchanged, still the open discriminator. Failure
  shape on the new baseline: manifests written with entries #2–5 missing
  (ladder-evicted early-middle; entry #1 survives via the pinned goal); one run
  CONFABULATED the missing entries (invented `bravo_window = 3291` etc. — wrong
  names AND values); models call `context_compact`/speculative `context_recall`
  — pressure is noticed but nothing points at the evicted entries.

**How the ordering was settled (3 paired iterations, one variable each):**
- **Start-of-turn maintain** (the "natural fix"): held every ceiling EXCEPT
  memory-roster **6/10** vs paired v3 **10/10** — systematic session-1 storage
  misses (model acks "remember X" without calling the tool, instruction verbatim
  in-window). Mechanism: maintaining with the fresh user prompt appended pushes
  the previous remember tool-turn into the compactable span one run earlier; the
  model's most recent visible template becomes ack-without-tool-call and it
  imitates it. REVERTED.
- **Unconditional exit maintain:** roster recovered (9/10, miss = retrieval
  churn with all 8 stored) but portmap wobbled **9/10** (a 6/8 merge-dropout) —
  the exit pass after a tool-bearing run's ack added an extra compaction beyond
  v3 cadence. REFINED.
- **Gated exit maintain** (shipped): portmap **10/10**, roster **9/10** (see
  attribution above), all other ceilings held.

## Champion (v3) — promoted 2026-07-02 (Tier-B: durable-anchor curation) — superseded by v4

- **Config:** `tasks/drift-ledger/champion_v3.json` — byte-identical to v2 (`default_k=10`);
  the promotion is CODE. Three coupled retention changes in `agent-core`
  (`context.rs::plan_retention`, `curated.rs` build/compaction):
  1. **User-priority build retention.** `build()` no longer keeps a contiguous
     newest-first suffix. A priority ladder charges est-tokens in order: (1) the newest
     unit unconditionally, (2) `Role::User` units newest-first, (3) everything else
     newest-first (`plan_retention`, whole units, orphan-safe). v1 kept user turns
     verbatim in *history*, but `build()`'s token-bounded eviction silently dropped
     them from the *window* — the guarantee has to hold at every discarding layer.
  2. **Boundary offload.** Tool results leaving the window through compaction are
     lifted into the offload store FIRST (age protection no longer applies to a
     departing result), so the summarizer can never destroy the last copy of a tool
     result — the recall chain survives compaction.
  3. **Durable placeholder units.** Offload-placeholder units (every result starts
     `[tool_result#`, unit ≤160 est tok) are partitioned out of the summarizer
     verbatim, like user turns — a paraphrased placeholder severs `context_recall`.
- **Trigger — STATE DRIFT, not a checkpoint hypothesis:** the 2026-07-02
  calibrated-budgeting merge (`ae3750d`) divides `model_limit` by the observed
  server/estimate token ratio (clamped ≤4.0). At `context_limit=4000` the effective
  build/maintain budget is ~1000 est tok. Every pre-2026-07-02 number was stale;
  champion re-baselined on current main: drift-ledger **0/6** (was 3/6), offload-recall
  **1/5** (was 5/5), locked-portmap **8/10** (was 4/10 — accidentally improved).
- **Paired results (window 4000, current-main champion legs, same session):**
  - **locked-portmap N=10:** champ **8/10** (median pass 69,250 tok) → v3 **10/10**
    (50,093 tok, −28%). `eval_gate` = **Promote**. THE PORTMAP GAP IS CLOSED
    (favorable was 5/5; realistic now matches it).
  - **drift-ledger N=6:** champ **0/6** → v3 **6/6** (median 51,796 vs ~67–90K tok).
    Gate prints the known 0-passes token artifact; promote on correctness (6 > 0).
    6/6 exceeds even v1's pre-calibration 3/6 "model-bound" ceiling — boundary
    offload keeps noise out of the summarizer, and the verbatim in-window increments
    let the model sum correctly.
  - **offload-recall N=5:** champ **1/5** → v3 **5/5** (median 34,538 tok). Gate =
    **Promote**. Trajectories show the ideal shape again (read×3 → overwrite →
    `context_recall` → answer).
  - **Held-outs at ceiling, no regression:** longhaul-codename **5/5**, memory-recall
    (real emb) **5/5**, memory-roster (real emb, k=10) **5/5**.
- All three changes ship with unit tests (`plan_retention_keeps_user_units_over_newer_chatter`,
  `build_keeps_user_instructions_under_tight_budget`,
  `compaction_offloads_departing_tool_results_before_summarizing`,
  `maintain_keeps_offload_placeholders_verbatim_through_compaction`).

## Champion (v2) — promoted 2026-06-29 (default_k 5→10)

- **Config:** `tasks/drift-ledger/champion_v2.json` → `/tmp/champion.json` (canonical going
  forward). Identical to v0 except **`default_k` 5→10**. **Code:** v1 compaction (unchanged).
- **Why:** iteration #M1 (see Tier-A log). The shipping `default_k=5` under-recalls when a task
  needs >5 distinct memories (the **memory-roster** discriminator: 1/5 at k=5). At k=10 it is
  **5/5**, and the single-fact **memory-recall** guard stays **5/5** (no regression, no token
  change — 1 memory stored). Non-memory tasks (drift-ledger, offload-recall, longhaul) have
  `memory_enabled=false` → `default_k` inert → byte-identical behavior, so v0/v1 baselines and
  admissibility verdicts are all preserved.
- **Promote on correctness** (memory-roster 1→5 passes); the eval token cost is ~+1.1%.
- **Known production trade-off (accepted):** with a *populated* memory store, `auto_recall` now
  injects up to 10 memories/turn instead of 5 (~2× recall tokens) — justified by correctness on
  multi-fact recall; the eval understates it because it stores few memories. Revisit if a
  recall-token budget becomes the bottleneck (then prefer a smaller k + a multi-fact-aware
  retrieval change over a flat k bump).

## Champion (v1) — promoted 2026-06-25 (Tier-B compaction code)

- **Config:** unchanged from v0 (`tasks/drift-ledger/champion_v0.json` → `/tmp/champion.json`).
  The promotion is a **code** change, not a param change.
- **Code:** cumulative + user-preserving compaction in `agent-core` (`compactor.rs`,
  `curated.rs`). Two coupled changes:
  1. **Cumulative summaries.** The prior summary is no longer dumped into the span to be
     re-summarized (where the model collapsed it to the newest turn). It is passed as a
     distinct labelled "PRIOR RUNNING SUMMARY" block; `COMPACTION_SYSTEM` mandates the new
     summary be a strict **superset** (carry every number/step, never paraphrase away).
  2. **User turns are never lossily summarized.** `maintain` partitions the old span:
     `Role::User` messages (the durable, author-authored instructions) are kept **verbatim**
     in history; only assistant/tool chatter is sent to the summarizer.
- **Paired result on `drift-ledger` (N=6, same server session):**
  champion(old code) **0/6** (~70–72K tok) → candidate(new code) **3/6** (~73–74K tok).
  `eval_gate gate` prints `Reject: no passing runs to compare tokens` — a **token-tiebreaker
  artifact** (champion has 0 passing runs → no median to compare), NOT a correctness
  rejection. Per the lexicographic objective (correctness is the hard gate), 3 > 0 passes
  is an unambiguous **promote**.
- The remaining 3/6 failures are **model-bound**, not context-bound: with a perfect summary
  in-window (all 8 steps + correct 107) the 3B-active model still emits wrong sums
  (95, 64) or a malformed write — the adversarial "starts at 0 each step" framing trips its
  arithmetic. Context fidelity is now ~100%; the ceiling is the model.

## Champion (v0) — baseline

- **Config:** `tasks/drift-ledger/champion_v0.json` (canonical; copy to `/tmp/champion.json`
  to iterate) — shipping defaults at a pressured 4000-token window:
  `context_limit=4000, high_water_pct=0.85, keep_recent=2, output_min_bytes=1024,
  error_min_bytes=200, recall_budget=512` (memory off).
- **Baseline on `drift-ledger` (N=5, then re-confirmed N=6):** pass-rate **0/6**, median
  tokens (passing) **n/a** (all runs drift and report the wrong total). ~69–74K tokens.
- The loop's job: raise the pass-rate (don't lose correctness) while keeping tokens far
  below the favorable reference's ~223K.

## Admitted training tasks

- **drift-ledger** (mode=`drift`): **Admitted** on 2026-06-25.
  - Favorable (`/tmp/favorable.json`, window 196608): **5/5 pass**, ~221–224K tokens.
  - Realistic (`/tmp/champion.json`, window 4000): **0/5 pass**, ~69–74K tokens.
  - Verdict via `eval_gate admit` → `Admitted` (favorable ≥0.8, realistic <0.5).
  - **Key fact:** large tool outputs are offloaded, so the workspace `noise.txt` does
    NOT fill the window. The drift pressure comes from a small `context_limit` (4000)
    forcing compaction of the early "+N" instruction turns. 16000 does NOT discriminate.

- **longhaul-manifest** (mode=`compaction`, `tasks/longhaul-manifest/`): **Admitted 2026-07-02**
  — the extreme-scale long-horizon discriminator the longhaul-codename notes called for.
  20 padded fact-bearing user turns (~86–106 est tok each, ~1719 total — far beyond the
  effective ~1000 window), acks pinned to a bare "OK" so facts cannot leak into summarizable
  chatter; the final turn assembles `manifest.txt` with all 20 `<name> = <value>` lines,
  graded by hidden greps (all 20 must match).
  - **Admit verdict = `Admitted`:** favorable **5/5** (~92–118K tok); realistic (champion v3)
    **0/5** (~78–86K tok).
  - Champion failure shape: `plan_retention`'s user pass drops the OLDEST user units once
    user turns alone exceed the build budget. Entry #1 always survives via the pinned goal
    block (`set_goal` captures the first prompt!); manifests come out ~16/20 with exactly
    the ladder-evicted early-middle entries missing.
  - **Optimization headroom is real but unclaimed** — see the 2026-07-02 overflow-fold
    iteration log below: seven variants, none promoted.
  - **2026-07-03 update: GAP CLOSED by champion v4 — 5/5 at 20/20 correct entries**
    (extractive fold to the goal-block ledger; see the v4 champion block).

## Locked tasks (real commits) — generalization report (run 2026-06-29)

- **locked-hostpolicy** (mode=`code`, `tasks/locked-hostpolicy/`) — a **real** unit of this
  codebase: `agent-http`'s `NetworkPolicy` host allowlist (commit `fbe1312`). Seeded as a
  std-only mini-crate (`Cargo.toml` + `src/lib.rs` with `decide()` stubbed = parent state);
  the agent implements the 3 matching rules (empty→Ask; exact case-insensitive, NOT substring;
  leading-dot→apex+subdomains but `notrust-lang.org`→Ask) delivered across turns under
  noise/offload pressure. **Graded by real `cargo test`** against the module's actual tests
  (sealed oracle copied into `tests/` at grading). Required harness changes (committed): allow
  `cargo` in the agent command allowlist (std-only crate, no deps/build.rs → bounded) and
  `create_dir_all` for nested seed paths (`src/lib.rs`).
- **Result (run ONCE, N=10 each, window 4000):** favorable (full window) ≈ **2/3**;
  **champion (v1 code + v2 config) 6/10  ==  v0 6/10** — a statistical tie.
  - **Honest conclusion:** the champion **does not regress** on real coding work, but shows
    **no measurable improvement** here. The task turned out **capability-bound** (favorable only
    ~2/3 — the 3B-active model is unreliable on the subtle Rust even with full context), so the
    model's coding ability, not context retention, is the bottleneck; failures track *longer/
    harder* runs, not compaction loss. The champion's context-management win is real where
    **context is the bottleneck** (drift-ledger 0/6→3/6) and simply doesn't surface where it
    isn't. A cleaner locked task would need favorable ≈ 5/5 to isolate the curation effect —
    delivered next (locked-portmap).

- **locked-portmap** (mode=`code`, `tasks/locked-portmap/`) — the **clean** locked task: same
  curation pressure as drift-ledger (8 `service→port` facts delivered across turns, each behind a
  noise read, at window 4000) but a **near-zero capability bar** — the code is a trivial `match`
  *transcribing* the given values (no logic to derive), graded by real `cargo test`. Parent state
  stubs `port_for()→None`. This **isolates context management**: any realistic-window failure is
  context LOSS, not capability.
  - **Result (run ONCE, N=10, window 4000):** favorable (full window) **5/5**;
    **champion (v1+v2) 4/10**; **v0 0/10**.
  - **The generalization win, finally on real cargo-tested code.** v0's lossy compaction loses the
    early service→port mappings → an incomplete/wrong `match` → **every** run fails (0/10). v1's
    verbatim user-turn retention + cumulative summary keeps them → **4/10** — a clear, no-overlap
    advantage (0→4) that mirrors drift-ledger's 0/6→3/6 on a real coding task. Champion is below
    favorable's 5/5, so some context degradation remains at 4000 even with v1 (partial recovery,
    not full) — but the compaction improvement is decisively better than v0 here, not just on the
    synthetic ledger.
  - **2026-07-02 update: GAP CLOSED by champion v3 — 10/10 at window 4000** (the 4/10
    above is a stale pre-calibration number; see the v3 block and the v2→v3 iteration log).

## Learnings (accumulated; never re-tried)

- **Calibrated budgeting rewrote the eval landscape (2026-07-02).** `ae3750d` divides
  the configured window by the observed server/estimate ratio (clamp 4.0): configured
  4000 → effective ~1000 est tok. ANY number recorded before it is stale — re-baseline
  the champion under current main before comparing a candidate. It regressed
  drift-ledger 3/6→0/6 and offload-recall 5/5→1/5 while accidentally improving
  portmap 4/10→8/10 (harder window → more compactions → the cumulative summary
  carried the facts more often).
- **A curation guarantee must hold at every layer that discards content.** v1's
  "user turns verbatim" held in history but not in the built window: `build()`'s
  newest-first suffix eviction silently dropped exactly the turns v1 preserved.
  Diagnosis came from an env-gated eviction dump, not param sweeps (again).
- **Compaction can outrace age-based offload.** With `keep_recent=2` protecting the
  newest tool results and compaction firing nearly every turn at ~1000 budget, a
  large fresh result is summarized (destroyed) before the age pass ever placeholders
  it — no pointer, no `context_recall`, offload-recall 0/5 with zero recall calls in
  any trajectory. Offloading at the compaction boundary is the timing-independent
  invariant; a `keep_recent` tweak would have been luck.
- **Protecting placeholders is useless if the placeholder never forms.** Candidate B
  (durable placeholder units alone) measured 0/5 on offload-recall — same as A. The
  fix needed the boundary offload (C) to create the placeholder first; B's partition
  rule then keeps it. Ship invariants in the right order: create, then protect.
- **Marker salience is positional (2026-07-02).** The model acts on markers it reads in
  conversation flow (offload placeholders in tool results: 5/5 recall) and ignores the
  same information appended to a pinned system block (0/5, zero recalls). But in-history
  user markers OVER-elicit unless consolidated and task-conditional — and even then the
  final assembly can be capability-bound.
- **Per-turn maintenance requires batched compaction.** `over_high_water` measured on
  the BUILT context is ~always true once saturated (build fills to the budget), so
  maintain-every-turn ⇒ compact-every-turn without a span-size floor — and repeated
  re-summarization destroys the running summary. Conversely, a span-size floor delays
  summaries enough to break tasks that relied on the per-tool-turn cadence (portmap
  10/10→~4/6). The compaction cadence is load-bearing in BOTH directions; treat any
  change to it as a re-baselining event.
- **The guard sweep is not optional.** The 2026-07-02 evening round looked promising on
  its target task's mechanics (recall elicited, content correct once) while silently
  destroying portmap (10/10→1/6) and drift (6/6→1/6). Non-regression on the full suite
  is the only thing that caught it.
- **Maintain ORDERING is behaviorally live, not plumbing (2026-07-03).** Moving
  maintain to start-of-turn — with zero change to what maintain does — regressed
  memory-roster 10/10→6/10: with the fresh prompt already appended, the split
  (`len - keep_recent`) lands one message deeper, the previous tool turn becomes
  compactable one run earlier, and the model IMITATES the visible
  ack-without-tool-call pattern instead of calling the tool. The window's most
  recent turns are a behavioral template, not just facts.
- **An extra maintain per run is a cadence change too (2026-07-03).** An
  unconditional text-exit maintain after tool-bearing runs (one extra compaction
  per run beyond v3) produced single-miss wobbles on exactly the fact-delivery
  tasks (portmap 9/10 merge-dropout, roster 9/10). Gating the exit pass to pure
  text-only runs restored portmap 10/10. When a task's ceiling was measured
  under a cadence, ship changes that leave that cadence byte-identical wherever
  possible.
- **Attribute single misses by prefix identity, not batch counts (2026-07-03).**
  Roster session-1 has a ~5-10% per-batch storage-slip rate from server
  nondeterminism (llama.cpp batching at temp 0). A 9/10 vs 10/10 batch says
  little; what settles attribution is whether the failing call's context could
  differ from the old code AT ALL (under the gated design it provably could not
  — every preceding run took a byte-identical path). Paired same-night batches +
  identical-prefix reasoning beat chasing flakes with more N.
- **Silent eviction invites confabulation (2026-07-03).** A manifest run with
  entries #2–5 evicted didn't just omit them — it INVENTED plausible
  names/values for them. The cost of losing a fact is not only absence; the
  model fills gaps with fabrications. Phase-2 fold/marker work is also a
  fabrication-prevention measure.
- **Pinned salience is a hierarchy, and the goal block sits alone at the top
  (2026-07-03).** The same 17-line fact ledger was: partially used as a
  standalone pinned block (mid-list block skipped), no better numbered, and
  **perfectly transcribed 5/5 once merged INTO the goal block**. The model
  treats generic pinned system blocks as skimmable reference but reads the
  goal block attentively every run. Content that MUST be acted on belongs in
  (or attached to) the goal block; this extends the marker-salience learning
  from actions to data.
- **A 3B extractor is reliable at verbatim fact extraction (2026-07-03).**
  Every observed `run_extraction` batch (9, 8, 10, 6 messages) produced
  perfect `name = value` lines — extraction (copy) is far below the
  capability edge that assembly (merge) sits on. Compress via extraction,
  not summarization, when fidelity is the requirement.
- **Self-anchoring confabulation:** a model that believes it already did the
  task ("Done. The manifest has been assembled" visible in-window) may
  regenerate FROM MEMORY instead of re-reading sources, fabricating at scale
  even with perfect data pinned in front of it.

- **Diagnostic beats param-guessing.** An env-gated `eprintln` of the compaction summary
  (since reverted) was worth more than any blind Tier-A sweep: it showed the summary
  literally **collapsing to the most-recent step** on each re-compaction. That made the
  Tier-A levers (`high_water_pct`, `keep_recent`) obviously insufficient — they change the
  *number* of compactions, not the per-pass loss — so the campaign went straight to Tier B.
- **Re-compaction is generation loss.** Feeding the prior summary back into the span to be
  re-summarized makes a small model treat it as stale chatter and drop it. Carrying it as a
  distinct "reproduce this verbatim, superset only" block fixes the collapse.
- **User turns are the wrong thing to lossily summarize.** They're few, small, and carry the
  task-critical facts; routing them through the summarizer is pure downside. Keep verbatim.
- **Gate CLI caveat:** when the champion has **0 passing runs**, `gate` always returns
  `Reject: no passing runs to compare tokens` even if the candidate passes more. Read
  `passes()` directly; treat strictly-more-passes as promote (correctness dominates tokens).
- **`keep_recent` is shared** (offload-protection AND compaction split). Tuning it trades
  noise-retention against recent-verbatim — a confound; Tier B decoupled the concern.
- **Tradeoff introduced:** verbatim user-turn retention is bounded by the **token window**
  (build() truncates newest-first), not by message count. The `repeated_compaction` stress
  test was updated to assert the new contract (token-bounded; chatter collapsed; user
  instructions durable). Very-long-horizon refinement — fold *oldest* user turns into the
  summary instead of letting build() hard-drop them — is **deferred** (see Held-out).
- **Pre-existing breakage (not ours):** `cargo test --workspace` fails to compile
  `agent-server` — `AgentEvent::ServerUsage` (added by the eval-harness merge) is unhandled
  in its match. Reproduces on clean `HEAD`. `agent-core`/`agent-runtime-config` are clean.

## Held-out tasks

- **offload-recall** (mode=`offload`, `tasks/offload-recall/`) — added 2026-06-25 to guard the
  offload→`context_recall` path (a *different* mode from drift-ledger's compaction). The agent
  reads 3 large files (each >1024B → offloaded), **overwrites** alpha.txt (so the original
  secret survives ONLY in the offloaded read result — re-reading the file returns 'archived'),
  then must write alpha's original `SECRET CODE`. This defeats the re-read escape hatch, so a
  pass means the model genuinely recalled offloaded content.
  - **Validation result (N=5 each):** favorable **5/5**; v0 realistic@4000 **5/5**;
    v1 realistic@4000 **5/5** (also 5/5 at tighter windows 2500). `heldout_ok(v0,v1)` =
    **PASS** (1.0 ≥ 1.0) → **v1 does not regress offload**.
  - **Finding:** v1's compaction summarizes the `Role::Tool` placeholder (tool turns aren't
    kept verbatim), yet recall still works — the model recovers the secret even with the file
    overwritten. The offload round-trip is robust under v1's cumulative summaries.
  - **Admit verdict = `NoWeakness`** (realistic passes for BOTH v0 and v1). So this is a
    **regression guard**, not a discriminator: neither version finds offload+recall hard at
    these windows. A truly weakness-first offload task would need a harder retrieval barrier
    (e.g. multiple competing placeholders + a derived multi-file answer) — deferred.
- **longhaul-codename** (mode=`compaction`, `tasks/longhaul-codename/`) — added 2026-06-29 to
  probe the v1 truncation tradeoff: an early user turn plants a codename (FALCON-9), followed by
  13 large filler user turns (~280 tok each) meant to overflow a 4000-tok window; the final turn
  must recover the codename. Because the fact arrives as a USER turn (not a tool result),
  `context_recall` cannot save it — if truncated, it is gone.
  - **Validation result (N=5 each):** favorable **5/5**; v0 realistic@4000 **5/5**;
    v1 realistic@4000 **5/5**. `heldout_ok(v0,v1)` = **PASS** (1.0 ≥ 1.0).
  - **Admit verdict = `NoWeakness`**, and notably v0 == v1 == 5/5: the codename survives under
    BOTH versions, so this scale does NOT exercise the truncation tradeoff. The model echoes
    the codename in its first ack and compaction (v0 and v1 alike) preserves it as a key
    fact/identifier; v1 additionally keeps the user turns verbatim. **Finding:** v1's
    long-horizon fact retention is robust — forcing a real failure would require extreme scale
    (≫13 distinct facts, overflowing even the cumulative summary). Kept as a regression guard;
    a discriminating long-horizon task is **deferred** (needs the harsher design).
- **memory-recall** (mode=`memory`, `tasks/memory-recall/`) — added 2026-06-29. **The first
  ADMITTED, genuinely discriminating held-out task** (besides drift-ledger). Cross-session: in
  session 1 the agent `remember`s `the deployment token is ZX-99-QUASAR`; session 2 (fresh
  window — the fact is NOT in-window, only in the SqliteStore) must recall it and write it. Run
  with **memory-mode configs** (`/tmp/mem_fav.json`, `/tmp/mem_real.json` = favorable/champion
  params but `memory_enabled=true`).
  - **Admit verdict = `Admitted`:** favorable (generous: `relevance_threshold=0.0`, `k=20`,
    `auto_recall`) **5/5**; realistic (champion: `relevance_threshold=0.3`, `k=5`) **0/5**.
  - **Mechanism:** the eval's `StubEmbedder` is FNV-hash, exact-match only (identical text→1.0,
    distinct→near-orthogonal). A natural session-2 query is near-orthogonal to the stored fact,
    so `query_memories` retains it only when `relevance_threshold≈0`. At 0.3 the match is
    filtered → nothing recalled → drift. (Real BGE embeddings would behave differently — this
    weakness is partly a stub artifact, but the params it exercises — `relevance_threshold`,
    `default_k` — are exactly what context-evolve may tune.)
  - **Validation (stub):** v0 realistic **0/5**, v1 realistic **0/5** → `heldout_ok` PASS (0≥0).

  **CORRECTION (2026-06-29) — the stub "weakness" was an EVAL ARTIFACT, not a real one.**
  Optimizing the realistic config was investigated and **rejected as gaming the metric.** The
  offline `StubEmbedder` cosine of the session-2 query vs the stored fact is **+0.016** (and an
  explicit `recall("deployment token")` is **−0.016**) — i.e. *near-orthogonal regardless of
  meaning*, because the stub is FNV-hash exact-match, not semantic. So `0.3` filtered it and only
  `threshold≈0` admits it — but at ≈0 the stub admits *everything* (all cosines cluster at 0),
  which is degenerate and would **mis-tune the production default** (real BGE gives related
  memories ~0.4–0.6; `0.3` is correct there). The honest fix is to the **eval, not the config**:
  - Wired an env-gated real-embedder path into the harness (`eval_context.rs`):
    `EVAL_REAL_EMBEDDINGS=1` (+ `FASTEMBED_CACHE=<dir>`) → real BGE-Small (onnx, default feature;
    model cached at `src-tauri/.fastembed_cache`). Default stays the deterministic stub.
  - **Under real embeddings, realistic@0.3 passes 5/5** (favorable 5/5; v0 5/5, v1 5/5 →
    `heldout_ok` PASS). Real-embedding runs are also *cheaper* (~12K vs ~21K tok): recall
    succeeds immediately instead of the model retrying a failing `recall`.
  - **Conclusion:** `relevance_threshold=0.3` needs **no change** — it is correct for the
    production embedder. memory-recall is therefore **NoWeakness under real embeddings** (a
    recall regression guard, not a discriminator) and **MUST be run with `EVAL_REAL_EMBEDDINGS=1`**
    to be meaningful; the stub run is misleading. Configs persisted at
    `tasks/memory-recall/{favorable,realistic}.json`.
  - **Lesson for the campaign:** never tune memory params (`relevance_threshold`, `default_k`,
    `dedup/forget_threshold`) against the stub embedder — its scores are non-semantic. Memory-mode
    tasks require the real embedder. (Still-open *genuine* memory weakness to author under real
    embeddings: many stored memories + low `default_k`/`max_recall_chars` so the RIGHT one is
    crowded out — that would be a legitimate discriminator.)

- **memory-roster** (mode=`memory`, `tasks/memory-roster/`) — added 2026-06-29. **The
  many-memories crowd-out discriminator** the memory-recall note called for, and the **first
  ADMITTED task under REAL embeddings.** Session 1 stores **8 HOMOGENEOUS** facts
  (`registry token <CODE> maps to value <N>` — no topical sub-structure); session 2 (fresh
  window) must recall ALL 8 and write them. Run with `EVAL_REAL_EMBEDDINGS=1` and the
  `tasks/memory-roster/{favorable,realistic}.json` configs (`dedup_threshold=0.99` so the
  near-template roster coexists — a construction necessity, orthogonal to the `default_k` lever).
  - **Why homogeneous matters:** a first attempt used topically-distinct facts (db/deploys/
    backups) and came back **NoWeakness** — the model worked around `default_k=5` by issuing
    *multiple topical `recall` calls* (realistic used 21–25 turns vs favorable's 19). Homogeneous
    facts give no query handles, so every recall returns the *same* top-5 → the cap bites.
  - **Admit verdict = `Admitted`:** favorable (`default_k=20`) **5/5**; realistic
    (`default_k=5`) **1/5**. Mechanism: `default_k=5 < 8` needed → only 5 retrievable, model
    can't separate them → incomplete → fail. A *genuine* weakness of the shipping `default_k=5`.
  - **Validation:** v0 realistic **1/5**, v1 realistic **1/5** → `heldout_ok` PASS (compaction is
    inert here — storage is per-prompt, session-2 retrieval is a single-prompt fresh window).

**Across 4 held-out probes (offload-recall, longhaul-codename, memory-recall, memory-roster) v1 is
robustly non-regressing.** drift-ledger and memory-roster are the genuinely discriminating tasks
with optimization headroom; the rest are regression guards.

## Iteration log (Tier-A — memory)

- **#M1 — `default_k` 5→10** (hypothesis: `default_k=5` under-recalls when a task needs >5
  distinct memories; raise it). Candidate vs champion on **memory-roster** (real embeddings, N=5):
  champion(k=5) **1/5** → candidate(k=10) **5/5**. `gate` printed `Reject: tokens not improved
  (56208 ≥ 55570)` — the **passes-increased gate artifact** (median-token compare assumes equal
  correctness; here passes jumped 1→5). **PROMOTE on correctness** (5 > 1; +1.1% tokens is the
  tiny, expected cost of recalling 8 vs 5). Held-out check: memory-recall (single fact) **5/5 →
  5/5** at k=10 (no regression, no token change); non-memory tasks have memory off (unaffected).
  - **Status: PROMOTED 2026-06-29 → `champion_v2.json` (default_k=10), CURRENT champion.** The
    production token trade-off (≈2× `auto_recall` injection on a populated store) was reviewed and
    accepted as the cost of correct multi-fact recall; see the Champion (v2) block. `champion_v0.json`
    is kept frozen as the baseline record; the per-task memory `realistic.json` files stay at their
    admit-time `default_k=5` (frozen, so the Admitted/0-1-of-5 verdicts remain reproducible). Under
    the v2 champion (k=10), memory-roster passes 5/5 — the weakness is fixed.

**Operational note (2026-06-29):** the `llama-agent` server was down (container removed); all
runs returned `{"passed":false,"tokens":0,"turns":0}` until relaunched. Zero tokens/turns ⇒
suspect the server, not the curation. Exact relaunch command is in the [[local-llama-server]]
memory.

## Iteration log (Tier-B — extractive fold, 2026-07-03) — PROMOTED → CHAMPION v4

Phase 2 of the manifest arc. Design anchored in a CE_DEBUG diagnosis (final
assembly call: goal pins entry #1, window holds #13-20 verbatim, #2-5 have NO
surviving representation; the model transcribes everything visible with zero
dropout and confabulates what's missing). Constraint math: 20 padded entries
(~81 est tok each) can never fit a ~1350-tok budget verbatim; condensed
`name = value` lines (~8 tok) fit trivially. Three iterations, one rendering
variable each, paired vs the sweep-3 champion legs:

- **#7a standalone pinned ledger:** extraction PERFECT (every fold, every
  line), block confirmed in the final window — and 0/5: the model used the
  ledger only partially (took lines 10-17, skipped 2-9) or ignored it in favor
  of parametric confabulation. One run wrote a PERFECT unprompted mid-session
  manifest from the ledger, then confabulated 15 entries at the real final
  prompt (anchored on its own "Done. The manifest has been assembled" reply).
- **#7b numbered lines + copy-all directive:** 0/5 with an IDENTICAL missing
  set (#2-9) in 4/5 runs — numbering alone didn't fix pinned-block neglect.
- **#7c ledger merged INTO the goal block:** **5/5 at 20/20.** The goal block
  is the one pinned region with demonstrated per-run attention (its fact was
  reproduced in 100% of every prior batch); riding it transfers that attention
  to the whole ledger. PROMOTED after the full guard sweep (see champion
  block).

## Iteration log (Tier-B — maintain ordering + summarizer guards, 2026-07-03) — BASELINE SHIFT, MERGED

Spec'd change (repo SDLC: spec + plan committed on the branch) closing open
issue #1 from the overflow-fold round. One ordering variable per iteration,
paired against snapshot v3 binaries per the Tier-B pairing method.

- **Guards (curated.rs), constant across iterations:** trivial-chatter skip
  (all-assistant span < 256 est tok skips the summarizer; explicit requests
  exempt; tool-bearing spans exempt at any size — the attic's flat floor is why
  portmap broke last round) + monotone prior guard (candidate summary smaller
  than the prior = discarded). 5 new unit tests; 4 existing tests fattened past
  the floor (they tested compaction mechanics with tiny chatter); stress-test
  compaction bound 50→25 (batching is now intended behavior).
- **Ordering #1 — start-of-turn:** full sweep held every ceiling (portmap 10/10,
  drift 6/6, offload 5/5, codename 5/5, mem-recall 5/5) EXCEPT memory-roster
  6/10 vs paired v3 10/10 — systematic storage-phase misses (see Learnings).
  REVERTED per the spec's fallback clause.
- **Ordering #2 — unconditional exit maintain:** roster 9/10 (miss = retrieval
  churn, all 8 stored — pre-existing mode), but portmap 9/10 (6/8 merge-dropout,
  mechanism-consistent with the extra per-run compaction); paired v3 portmap
  10/10. REFINED.
- **Ordering #3 — exit maintain gated to pure text-only runs (SHIPPED):**
  portmap 10/10, roster 9/10 (single miss on a byte-identical-to-v3 prefix →
  server nondeterminism), drift 6/6, offload 5/5, codename 5/5, mem-recall 5/5,
  manifest 0/5 (expected). Full numbers in the Baseline-shift block above.

## Iteration log (Tier-B — overflow-user folding, post-v3, 2026-07-02 evening) — NO PROMOTION, ALL REVERTED

One hypothesis family — "fold overflow user turns instead of silently evicting them" —
tested against the freshly admitted longhaul-manifest. Seven evidence-driven variants,
every one **0/5** (== champion). Diffs archived in `attic/2026-07-02-overflow-fold/`.

- **#6a fold-to-summary:** overflow users routed through the summarizer at compaction.
  The summary carried all 12 folded pairs PERFECTLY — and the model still wrote 16/20,
  deterministically skipping mid-list entries when transcribing from summary prose.
  Dead end: summary *presence* ≠ summary *use*.
- **#6b recall-only fold, marker in the pinned summary block:** model NEVER called
  context_recall (0/5, zero recalls in any trajectory) — a pinned block line has no
  salience. Also surfaced **speculative-recall poisoning**: the model guessed
  `context_recall(1)` before any fold existed, got NotFound, and the summarizer
  immortalized "recall #1 fails" as a durable fact that suppressed the later real recall.
- **#6c eviction-triggered compaction:** compact the moment the plan would evict a user
  unit. CATASTROPHIC: per-turn re-compaction collapsed the cumulative summary to
  "No new information provided" within ~16 passes — the superset prompt survives one
  pass, not sixteen. (Mechanism kept as a *sync, summarizer-free* fold trigger instead.)
- **#6d maintain-at-start-of-turn (loop_.rs):** found that `maintain()` NEVER RUNS on
  text-only turns (the text-reply path returns before the end-of-turn maintain) — pure
  chat sessions get no curation at all, only silent build() truncation; longhaul-manifest's
  20 ack turns ran completely unmaintained, so every fold fired only after the final write.
  Moving maintain to start-of-turn fixes that ordering — but see the guard sweep below.
- **#6e in-history user marker (like the loop's stuck-nudge):** finally elicits recall —
  but OVER-elicits: 5–13 recalls/run, recall churn during routine acks, one run burned 33
  turns without ever writing.
- **#6f consolidated task-conditional marker + trivial-span re-compaction guard (256 est
  tok):** ideal trajectories appear (one recall → one write with all 20 correct pairs once,
  killed by a missing `path` arg) and still 0/5 — the 20-pair merge at an effective
  ~1000-token window sits at the 3B-active model's capability edge.
- **GUARD SWEEP FAILED → FULL REVERT.** The final tree (maintain-at-start + trivial-span
  guard + fold) on the v3 ceilings: locked-portmap **1/6** (v3: 10/10), drift-ledger
  **1/6** (v3: 6/6), offload-recall 5/5, longhaul-codename 5/5, memory-recall 5/5,
  memory-roster 4/5. Bisect (loop change reverted, guard+fold kept): portmap 4/6 — both
  the loop-ordering change and the guard/fold contribute. A failing portmap run's lib.rs
  was missing exactly 'cache'+'search' (the early-middle entries) with a fully-correct
  summary in-window — the same merge-across-sources dropout as the manifest task, exposed
  by the guard's delayed compaction leaving no single complete source at write time.
  **Champion stays v3; all code reverted to the v3 merge (1ad10c8).**

**Open issues recorded (owner-level, do NOT slip into a campaign round):**
1. `maintain()` skipped on text-only turns is a REAL structural gap (chat-only sessions
   are never curated) — but it is also the semantics every admitted verdict and champion
   result was measured under. Fixing it re-baselines the whole eval landscape (as
   calibrated budgeting did) and, naively combined with per-turn compaction, destroys
   running summaries and regressed portmap/drift hard. Needs its own spec'd change with
   a full re-baseline, not a drive-by fix. Patch preserved in the attic.
2. Summary poisoning by transient tool errors (see #6b) — the superset prompt happily
   carries "tool X failed" forward forever.
3. `set_goal` pins the FIRST user prompt verbatim — task authors must not put
   load-bearing facts in prompt #1 (it silently survives all curation; both manifest
   and portmap diagnostics show entry #1 always rescued).

## Iteration log (Tier-B — retention, v2→v3, 2026-07-02)

- **Re-baseline (forced by state drift).** CE_DEBUG diagnostic run on locked-portmap
  showed `budget+pinned ≈ 1000`, not 4000 → traced to calibrated budgeting
  (`effective_model_limit`, merged post-checkpoint). All 8 fact-bearing user turns
  were being evicted from the built window; facts survived only via summary luck.
  Champion re-baselined on current main: portmap 8/10, drift 0/6, offload 1/5.
- **Tier-B #3 — user-priority build retention (cand A).** Paired portmap N=10:
  champ 8/10 → A **10/10**, median pass tokens 69,250→60,095; `gate`=Promote.
  Paired drift N=6: 0/6 → **3/6** (~35% fewer tokens). Held-outs: longhaul 5/5,
  memory-recall 5/5, memory-roster 5/5, but **offload-recall 0/5 vs champ 1/5 →
  blocked on the held-out hard gate** (placeholder pointers lost to the summarizer).
- **Tier-B #4 — durable placeholder units (cand B = A + partition).** offload-recall
  still **0/5**, zero `context_recall` calls in any trajectory. Diagnosis: the
  placeholder never forms — compaction consumes the raw alpha read while
  `keep_recent=2` still age-protects it. **Dead end standalone; kept as C's guard.**
  (B side-data: portmap 6/6, drift 5/6, longhaul/memory 5/5 — the A-part held.)
- **Tier-B #5 — boundary offload (cand C = A+B + offload departing results at the
  compaction split).** offload-recall **5/5** (gate=Promote), portmap **10/10**
  (gate=Promote), drift **6/6**, longhaul 5/5, memory-recall 5/5, memory-roster 5/5.
  **PROMOTED 2026-07-02 → champion v3** (code merged to main; config file
  `champion_v3.json` = v2 params, frozen).

## Iteration log (Tier-B — compaction, v0→v1)

<!-- one entry per hypothesis: change | N raw results (or pass-rate + median) | gate verdict | kept? -->
- **Tier-A (skipped, by diagnosis).** Instrumented one champion run: compaction summaries
  collapse to the newest step on re-compaction (numbers vanish). Mechanism shows `high_water_pct`/
  `keep_recent` cannot fix per-pass loss → went straight to Tier B. No param iteration run.
- **Tier-B #1 — cumulative superset summaries** (compactor.rs prompt + prior-as-distinct-block).
  CE_DEBUG run: summaries now accumulate all 8 steps (✓) but one variant echoed the prompt
  scaffolding into the body (507s, rambling) → tightened to neutral section labels + "output
  only the summary". Kept as part of #2.
- **Tier-B #2 — preserve user turns verbatim** (curated.rs partition; only chatter summarized).
  Paired N=6: champion **0/6** vs candidate **3/6**. `gate` → `Reject: no passing runs…`
  (token artifact); **PROMOTED on correctness** (3 > 0). New unit test
  `maintain_keeps_user_instructions_verbatim_through_compaction`; stress test updated. **Kept.**

## Guard-config clarification (2026-07-03 night / 07-04) — roster "ceiling drift" RESOLVED: config mismatch, ceilings STAND

- **Suspicion (harness-evolve iteration 6, 2026-07-04):** an "unmodified
  baseline" memory-roster batch scored 5/10 against the recorded ≥9/10 ceiling
  → both campaigns flagged GUARD-CEILING DRIFT and queued a server-restart
  re-baseline of all admitted rates.
- **Resolution (this record, after a fresh `llama-agent` restart, same night,
  N=10 each):** roster @ `tasks/memory-roster/realistic.json` = **2/10**;
  roster @ champion params (`realistic.json` + `default_k=10`) = **10/10**
  (storage 8/8 every run in both; Fisher p≈7e-5). **There was no drift.**
  `realistic.json` is FROZEN at its admit-time `default_k=5`, whose documented
  rate is 1/5 (#M1: "1/5 at k=5 … 5/5 at k=10"); every roster ceiling (9/10,
  5/5) was recorded under champion k=10. harness-evolve's guard sweeps
  (iterations 1–6) ran roster @ `realistic.json` — an admission red-side
  config — and compared against the champion-config ceiling. The iteration-6
  arms (5/10, 3/10, 3/10) and tonight's 2/10 are ordinary draws from the k=5
  config's ~0.2–0.35 true rate.
- **Consequences:** (1) THIS campaign's admitted ceilings are all VALID —
  nothing to correct; the queued full re-baseline is cancelled. (2) New file
  `tasks/memory-roster/champion_k10.json` (= realistic.json + `default_k=10`,
  dedup 0.99 kept — construction necessity) is the canonical roster GUARD
  config; sweeps must never grade roster on `realistic.json`. (3) The paired
  guard protocol adopted 2026-07-04 (interleaved baseline+candidate arms,
  relative criterion) STAYS — it is exactly what catches both real drift and
  this class of config error. (4) General learning: **a guard ceiling is a
  (config, rate) pair — a task id is not enough.** Admission configs are
  deliberately red-side and must never be reused as guard configs after the
  champion moves past them.

## External-merge guard sweep (2026-07-07) — audit cluster 5 goal-block cap: ALL CEILINGS HOLD

Audit-drain cluster 5 (merge `35a9cea`) capped the pinned goal block at
`GOAL_MAX_TOKENS = 512` in `set_goal` — a champion-v4 spine touch (the fold
ledger renders inside the goal block), so the triage spec mandated this sweep.

- **Structural inertness verified first:** every sweep task's FIRST prompt is
  ≤ 96 est tokens, so the cap cannot fire on this suite — `set_goal` output is
  byte-identical to pre-merge. The sweep below is the (config, rate) evidence.
- **Results (champion params; roster @ `champion_k10.json`; recall @ its
  `realistic.json`; N = recorded-ceiling N):** manifest **5/5** (med 80,397),
  portmap **10/10** (50,788), codename **5/5** (55,537), offload **5/5**
  (36,270), mem-recall **5/5** (20,927), mem-roster **10/10** (75,770 — a point
  ABOVE the 9/10 ceiling), drift **11/12** (59,297 — exactly the ceiling; the
  miss is the documented model-bound invented-step mode).
- **INCIDENT (new pairing gotcha, protocol hardened):** the first sweep pass
  omitted `EVAL_REAL_EMBEDDINGS=1` → eval_context wired the exact-match
  StubEmbedder → memory-recall **0/5** with a deterministic zero-hit-recall
  trajectory shape (remember, 4-5 fruitless recalls, writes UNKNOWN). This is
  the roster-2026-07-03 class again from a new direction: **the env var is part
  of every memory-task ceiling's (config, rate) pair.** train.md's run()
  recipe now sets it unconditionally. Diagnostic signature for next time:
  memory-task collapse where recall returns nothing EVERY run = check the
  embedder wiring before suspecting the curation code.
