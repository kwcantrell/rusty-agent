# train.md — the per-iteration loop

One iteration = ONE mechanism-level hypothesis, tested under the gate. Stop
after K=6 consecutive non-improvements; then run the locked task once.

1. **Read `program.md`.** Never retry a logged dead end.
2. **Diagnose before designing.** CE_DEBUG-style window/trace dumps on a
   failing champion run BEFORE forming the hypothesis (both context-evolve
   2026-07-03 wins came from window dumps, not param sweeps). Remove all
   diagnostics pre-merge.
3. **One change.** Tier A: edit one genome field in `cand.json`. Tier A′: one
   skill-file variant under `artifacts/agent-skills/`. Tier B: one code change
   + rebuild + snapshot binaries per code state. Tier C: dedicated spike only.
4. **Eval paired, equal N (N=5 for web tasks — runs are minutes).** Candidate
   AND champion re-run back-to-back the same night; no mid-batch edits; a
   batch interrupted by a server restart is discarded whole.
5. **Gate:** `eval_gate gate champ.jsonl cand.jsonl`. Known artifacts: 0-pass
   champion → token-artifact Reject (read passes() directly; strictly-more-
   passes = promote); passes-increased → token Reject artifact (same rule).
6. **GUARD SWEEP — NOT OPTIONAL.** Before any promotion:
   - harness-evolve held-outs (as they accrue), AND
   - **context-evolve's admitted set** at its v4 ceilings: longhaul-manifest
     5/5 (20/20 entries), locked-portmap 10/10, drift-ledger ≥11/12,
     longhaul-codename 5/5, offload-recall 5/5, memory-recall 5/5 (REAL
     embeddings), memory-roster ≥9/10 (~5–10%/batch storage-slip noise).
   Tier-A changes provably inert to curation (e.g. sampler-only) may run a
   reduced sweep; Tier B / prompt / skills / tools changes run it ALL. When in
   doubt, full sweep.
7. **Attribute single misses by prefix identity** (llama.cpp is not
   bit-deterministic at temp 0): could the failing call's context differ from
   the champion's AT ALL? Paired same-night batches beat more N.
8. **Append to program.md** (hypothesis, change, batches, verdict) — promote
   or not. On promote: update the champion block + config; on Tier-B promote,
   merge the code per repo conventions (spec'd change, ci.sh green).
