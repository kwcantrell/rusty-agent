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

## Champion (v0) — pending admission

<!-- Filled by the web-multipage admission run: config, pass-rate, median
tokens, wall_ms medians, failure shape. -->

## Admitted training tasks

<!-- web-multipage entry goes here on Admitted verdict: both configs, N=5
numbers each side, realistic window found, failure shape. -->

## Learnings (accumulated; never re-tried)

- (seed) Favorable ≈5/5 or the signal is mud — locked-hostpolicy precedent.
- (seed) `gate`'s 0-pass/passes-increased token artifacts — read passes().
- (seed) Attribute single misses by prefix identity, not batch counts.

## Iteration log

<!-- one entry per hypothesis: change | N results | gate verdict | kept? -->
