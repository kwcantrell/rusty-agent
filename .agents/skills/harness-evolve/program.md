# harness-evolve — accumulated learnings + champion

Append-only research memory. The loop reads this first every iteration and
never retries a logged dead end. Campaign spec:
`docs/superpowers/specs/2026-07-03-harness-evolve-campaign-design.md`.

## Hard constraints (verified 2026-07-03)

- RTX 3090 24 GB; 60 GB RAM. Server: llama.cpp docker `llama-agent`, :8080,
  `-np 4 --kv-unified -c 196608` (see the local-llama-server memory; does NOT
  survive reboot).
- 35B-A3B IQ4_XS = 17.7 GB resident (settles ~21.6/24.6 GB). 27B Q5_K_XL =
  20 GB; 27B Q4_K_XL = 17.6 GB. **NO 27B variant co-resides with the 35B.**
  gpt-oss-120b (MoE ~5B active, 60 GB mxfp4) on disk — CPU-heavy wildcard,
  spike-tier only.
- Startup-only flags (restart = Tier-C re-baseline): -c, -np, -ngl, KV type,
  -fa, --cache-ram. Per-request (Tier-A): temp/top_p/top_k/min_p/penalties,
  max_tokens, tools.

## Phase-0 decisions (2026-07-03)

- node-offline profile: docker sandbox ENFORCED for web tasks
  (node:22-bookworm-slim, network NONE, HOME=/tmp default), node_modules
  pre-seeded at authoring time, grading in-container. Agent never installs.
- Allowlist (eval SafeApproval, node profile only): node, npx, tsc, vitest,
  vite. NOT npm install/ci/run — package.json scripts are agent-writable.
- exec_profile: None tasks byte-identical to pre-campaign semantics (no
  re-baseline of context-evolve).

## Hypothesis backlog (unlock order per spec §roadmap)

1. System prompt variants (BASE_SYSTEM_PROMPT has never been evaluated).
2. Agent-side SDLC skill via skills_dirs/active_skills — start with ONE
   verify-before-done skill.
3. Sampler sweep (temperature first; champion inherits 0.2).
4. Tool descriptions (seam live as of 2026-07-03); then missing tools
   (dev-server probe) as Tier B.
5. Sub-agent policy: when to delegate; orchestrator-as-role on the SAME 35B
   (subagent_model + role) — topology spike #1.
6. Memory axes (REAL embeddings only).
7. Tier-C spikes: serial model swap (expect run-cost fail; measure once);
   partial-offload co-residency (expected dead end — record the arithmetic).
8. Audit carry-overs: summary poisoning by transient tool errors;
   max_result_bytes realism.

## Champion (v0) — baseline, set at admission (2026-07-03)

- **Config:** `tasks/web-multipage/champion_v0.json` — context-evolve champion-v4
  params (`high_water_pct=0.85, keep_recent=2, error_min=200, output_min=1024,
  recall_budget=512, default_k=10`, memory off) at **`context_limit=3000`**,
  `max_turns=25`. Code state = this branch (v4 curation + phase-0 driver).
- **Baseline on `web-multipage` (N=5): 2/5 pass**, passing tokens 82,894 /
  98,122 (median-of-passing 90,508); wall ~35–60 s/run. Favorable reference:
  5/5 at ~195K tokens, ~33 s.
- **The loop's job:** raise the realistic pass-rate toward favorable's 5/5
  without token blowup — via HARNESS axes (prompt, SDLC skill, tools,
  sub-agents, sampling), not context-curation code (that is context-evolve's
  lane; Tier-B curation edits here must sweep BOTH campaigns).

## Admitted training tasks

- **web-multipage** (mode=`code`, exec_profile=`node-offline`,
  `tasks/web-multipage/`): **Admitted 2026-07-03.**
  - Favorable (`favorable.json`, window 196608): **5/5**, tokens
    183–214K (median 195,863), 21–23 turns, ~33 s/run.
  - Realistic (`champion_v0.json`, window 3000): **2/5**, tokens
    55–131K. `eval_gate admit` → **Admitted** (favorable ≥0.8, realistic <0.5).
  - **Window ladder (task unchanged, config-only):** 8000 → 5/5 NoWeakness;
    4000 → 3/5 NoWeakness (boundary); **3000 → 2/5 Admitted**. Calibrated
    budgeting makes effective ≈ window/4 (~750 est tok at 3000).
  - **Failure shape at 3000 — GOAL-DRIFT CHURN, not fact loss:** the three
    failing runs never `write_file` router.ts at all. One run: 44 read_file +
    9 list_directory over 37 turns / 131K tokens, a single speculative
    `context_recall`, zero implementation. The model loses the PLAN (what to
    do next), not the facts — it re-orients by re-reading the workspace until
    turns run out. Distinct from longhaul-manifest's fact-eviction mode (v4
    already fixed that); this is the whole-harness weakness the campaign
    exists to attack (goal restatement, SDLC skill, verify-loop discipline
    are the obvious first levers).
  - Passing realistic runs cost LESS than failing ones (52–98K vs 55–131K) —
    churn is expensive; correctness and token economy point the same way.

## Learnings (accumulated; never re-tried)

- (seed) Favorable ≈5/5 or the signal is mud — locked-hostpolicy precedent.
- (seed) `gate`'s 0-pass/passes-increased token artifacts — read passes().
- (seed) Attribute single misses by prefix identity, not batch counts.
- (bring-up) The eval driver applies the SKILLS_DIR env hook AFTER apply_to — an exported SKILLS_DIR silently overrides a candidate's skills_dirs genome; unset it for axis-5 iterations.

## Iteration log

<!-- one entry per hypothesis: change | N results | gate verdict | kept? -->

### Iteration 1 (2026-07-03) — H1 system-prompt: restate-then-act discipline — REJECTED BY GUARD SWEEP (non-improvement 1/6)

- **Diagnosis (admit_realistic.jsonl trajectories, before designing):** the three
  admission failures split two ways. (a) Runs 2/3 STOPPED EARLY — 16/18 turns of
  25, zero write_file, zero execute_command: after eight "read noise.txt →
  one-sentence ack, no code yet" turns, the implement turn imitates the ack
  TEMPLATE instead of the instruction (roster's template-imitation mode,
  resurfacing at act time). (b) Run 0 implemented (edit_file ×2), ran vitest
  once, then re-orientation churn (30+ reads, noise.txt ×5 more) instead of
  read-failure→fix. No failing run restated requirements at implement time. The
  eval default prompt's "then give a short final reply" legitimizes the early
  prose exit; nothing anchors the plan at act time.
- **Hypothesis:** the implement turn has no plan anchor and no completion
  criterion; adding restate-requirements-first → write-immediately →
  verify-and-fix discipline to `system_prompt` converts prose-exit/read-churn
  turns into write-then-verify turns.
- **Change (Tier A, one field):** cand.json = champion_v0 + `system_prompt` =
  eval default + this paragraph (verbatim, for never-retry):
  > Discipline for implementation requests: when asked to implement or modify
  > code, START your turn by restating every requirement collected so far as one
  > numbered list (pull them from the entire conversation and any pinned
  > context, without re-reading workspace files), then write the code
  > immediately with write_file or edit_file, then run the requested
  > verification commands and fix what fails. An implementation request is
  > complete only after the files are written and verification has run — never
  > finish with prose alone, and never substitute re-reading already-read files
  > for writing code.
- **Paired batch (interleaved, same session, N=5, web-multipage @ window 3000):**
  champ 3/5 (passing 60,240/103,516/115,283; median 103,516); cand 3/5 (passing
  83,536/85,979/89,272; median 85,979). `eval_gate` → **Promote** (equal passes,
  −17% median; passes() read directly). Champion's 3/5 vs admission's 2/5 is
  cross-night drift — trust same-night pairs only.
- **Failure-shape shift (mechanism confirmed on the training task):** both cand
  failures WROTE code and RAN verification (one died mid-fix at turn cap; one
  over-acted — rebuilt the whole scaffold from scratch, own package.json/
  vite.config over the seed, then hit the offline npm wall). The never-write
  prose-exit mode was absent from the cand batch; both champ failures were
  classic churn.
- **Guard sweep (candidate prompt OVERLAID on each guard task's champion/
  realistic config — sweep convention for prompt axes; untouched guard configs
  would make the sweep vacuous):** portmap **10/10** ✓, manifest **5/5** ✓
  (check.sh enforces 20/20), codename **5/5** ✓, offload **5/5** ✓, mem-recall
  **5/5** ✓, drift **10/12** ✗ (ceiling ≥11/12; directionally consistent, not
  independently attributed), **mem-roster 0/10** ✗✗ (ceiling ≥9/10).
- **Roster 0/10 mechanism (from remember/recall args):** storage PERFECT — 8/8
  codes stored verbatim every run. Retrieval-side kill: recall queries are
  generic ("registry token"/"token"/"registry"); k=5 over eight near-identical
  texts ranks deterministically, so the SAME code (RV-219) misses in 8/10 runs
  (the other two wrote exactly one k=5 recall's worth). The prompt's
  "restate what you have, then write immediately" makes the model treat its
  restated subset as the full inventory and write facts.txt at once; it cannot
  targeted-query an unknown-unknown (run 5: 10 recalls, all anchored on codes it
  already had, still missed RV-219). Baseline behavior keeps gathering until the
  count matches → 9/10.
- **Verdict: REJECT.** Champion stays v0. **Learning (general): an
  act-to-completion prompt discipline is ANTI-RETRIEVAL — "write immediately"
  truncates iterative gather loops on inventory/recall tasks. Any future prompt
  candidate must scope the discipline (e.g. "once every stated requirement is
  in the list" / gather-until-inventory-matches-count before acting) rather
  than command unconditional immediate writes.** Queued refinement (H1b, one
  new hypothesis): keep restate-first + verify-before-done, drop "immediately",
  add an explicit completeness check ("if the task states a count or list,
  confirm your restatement covers ALL of it; gather what is missing first").
  Watch the new over-scaffolding pathology (cand run 1) in any H1 descendant.

### Iteration 2 (2026-07-03) — H1b: gather-until-count prompt — REJECTED AT TRAINING GATE (non-improvement 2/6)

- **Hypothesis:** H1's discipline minus "immediately", plus a completeness
  check, keeps the plan anchor (H1's training win) while un-truncating the
  gather loop that killed mem-roster.
- **Change (Tier A, one field):** cand = champion_v0 + `system_prompt` = eval
  default + H1's paragraph with two edits (verbatim, for never-retry):
  > Discipline for implementation requests: when asked to implement or modify
  > code, START your turn by restating every requirement collected so far as
  > one numbered list (pull them from the entire conversation and any pinned
  > context, without re-reading workspace files). If the task states a count or
  > a list of items, confirm your restatement covers ALL of them; gather what
  > is missing first. Then write the code with write_file or edit_file, then
  > run the requested verification commands and fix what fails. An
  > implementation request is complete only after the files are written and
  > verification has run — never finish with prose alone, and never substitute
  > re-reading already-read files for writing code.
- **Paired batch (interleaved, same session, N=5, web-multipage @ 3000):**
  champ 3/5 (passing 57,155/57,556/75,824; median 57,556); cand 2/5 (passing
  96,546/110,758; median 103,652). `eval_gate` → **Reject: correctness
  regressed (2 < 3)**. No sweep run. Champion's median moved 103.5K→57.6K
  across same-night batches — cross-batch comparison stays banned.
- **Mechanism (pinned from cand trajectories): WRONG-DOMAIN GATHERING.** The
  completeness clause names no bounded source of truth, so the model hunts for
  the "missing" requirements in the WORKSPACE: cand run 2 = 122 read_file over
  47 turns / 168,936 tok, including ~20 NONEXISTENT paths
  (src/components/Header.tsx, App.tsx, … — fishing for requirement-bearing
  files the seed never had), implements only at the bitter end (and via the
  deny-listed `npm test` instead of npx vitest). Run 1: pure read churn +
  2 context_recall, zero writes. Run 4: implemented, turn cap hit mid-verify.
  Both cand PASSES cost 96–111K vs champ's 57–76K — the clause taxes every
  run. H1's over-scaffolding pathology did NOT recur (no scaffold rebuilds);
  the pathology moved from over-acting to over-gathering.
- **Verdict: REJECT.** Champion stays v0. **Learning (general): a completeness
  directive MUST name its bounded source of truth. "Gather what is missing"
  without saying WHERE sends the model to the wrong domain (workspace files —
  including invented paths) instead of conversation/pinned-context/recall.
  Roster needed recall-domain gathering; web needs conversation-domain; files
  are never the requirements domain in either.** Queued (H1c, the two pinned
  mechanisms combined): restate-first + act + verify, gathering scoped
  EXPLICITLY to conversation/pinned-context/recall tools ("requirements never
  come from searching workspace files; read a file only to see the code you
  are changing"), completeness check retained, "immediately" stays dropped.

### Iteration 3 (2026-07-03) — H1c: bounded-gather prompt — REJECTED AT TRAINING GATE (non-improvement 3/6)

- **Hypothesis:** combining the two pinned mechanisms (restate-then-act helps;
  gathering must name a bounded source) yields the win without the collateral:
  requirements domain = conversation/pinned-context/recall ONLY, files only for
  the code being changed, "immediately" dropped, completeness check kept.
- **Change (Tier A, one field):** cand = champion_v0 + `system_prompt` = eval
  default + (verbatim, for never-retry):
  > Discipline for implementation requests: when asked to implement or modify
  > code, START your turn by restating every requirement collected so far as
  > one numbered list. Requirements come from the conversation, pinned context,
  > and your recall tools — never from searching workspace files; read a
  > workspace file only to see the code you are changing. If the task stated a
  > count or delivered items one at a time, confirm your list covers ALL of
  > them, and retrieve anything missing with your recall tools before
  > proceeding. Then write the files with write_file or edit_file, run the
  > requested verification commands, and fix what fails. The request is
  > complete only after the files are written and verification has run — never
  > finish with prose alone, and never substitute re-reading already-read
  > files for writing code.
- **Paired batch (interleaved, same session, N=5, web-multipage @ 3000):**
  champ 3/5 (passing 79,034/96,750/107,770; median 96,750); cand 3/5 (passing
  99,871/117,530/132,487; median 117,530). `eval_gate` → **Reject: tokens not
  improved (117,530 ≥ 96,750)** — equal passes, genuine +21% token regression,
  not an artifact. No sweep run.
- **Shape (the bound WORKED, the tax didn't pay):** workspace-fishing is gone
  (zero phantom/nonexistent-path reads across all 10 runs; the one gathering
  failure did 6 context_recall instead of file hunting — right domain, still
  no write). But every cand pass cost more than its champ counterpart; the
  paragraph taxes ALL runs.
- **Verdict: REJECT.** Champion stays v0. **Learning (structural, closes the
  H1 prompt family): at a realistic window an always-on system-prompt
  discipline is paid EVERY turn — ~200 extra prompt tokens inside a
  3000-token window (~7%) squeeze the curated context and raise total tokens
  even when behavior improves. Three variants (act-now / gather-unbounded /
  gather-bounded) each fixed their predecessor's pathology and introduced or
  kept a cost: the axis oscillates around the gate, it does not cross it.
  Next prompt-shaped intervention should be DELIVERED ON-DEMAND (axis-2 SDLC
  skill via skills_dirs/active_skills — loaded at act time, not resident) or
  attack a different mechanism entirely (axis-3 sampler; axis-4 tool
  descriptions).** H1 descendants are a logged dead end absent NEW
  trajectory evidence of a different mechanism.
