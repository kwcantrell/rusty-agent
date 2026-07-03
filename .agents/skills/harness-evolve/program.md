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
