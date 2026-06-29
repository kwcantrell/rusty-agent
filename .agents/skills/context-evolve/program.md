# context-evolve — accumulated learnings + champion

Append-only research memory. The loop reads this first every iteration and never
retries a logged dead end.

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

## Held-out tasks

- (none yet — add weakness-first tasks targeting *different* modes before trusting any
  accepted change; e.g. `offload`, `compaction`, `memory-under-recall`.)

## Locked tasks (real commits)

- (none yet — add 1–2 real-commit tasks; run once at campaign end for the honest
  generalization report.)

## Learnings (accumulated; never re-tried)

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

**Across 3 held-out probes (offload-recall, longhaul-codename, memory-recall) v1 is robustly
non-regressing** — no mode drops v1 below v0. The compaction change generalizes beyond the
drift-ledger it was tuned on; no regression surfaced. All three are NoWeakness regression guards
(under their correct embedder for memory-recall); drift-ledger remains the only genuinely
discriminating task with optimization headroom. A real memory discriminator is still TODO (see
memory-recall note: many-memories crowd-out under real embeddings).

**Operational note (2026-06-29):** the `llama-agent` server was down (container removed); all
runs returned `{"passed":false,"tokens":0,"turns":0}` until relaunched. Zero tokens/turns ⇒
suspect the server, not the curation. Exact relaunch command is in the [[local-llama-server]]
memory.

## Iteration log

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
