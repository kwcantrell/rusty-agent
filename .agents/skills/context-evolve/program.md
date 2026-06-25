# context-evolve — accumulated learnings + champion

Append-only research memory. The loop reads this first every iteration and never
retries a logged dead end.

## Champion (v0)

- **Config:** `tasks/drift-ledger/champion_v0.json` (canonical; copy to `/tmp/champion.json`
  to iterate) — shipping defaults at a pressured 4000-token window:
  `context_limit=4000, high_water_pct=0.85, keep_recent=2, output_min_bytes=1024,
  error_min_bytes=200, recall_budget=512` (memory off).
- **Baseline on `drift-ledger` (N=5):** pass-rate **0/5**, median tokens (passing) **n/a**
  (all runs drift and report the wrong total). Failing runs cost ~69–74K tokens.
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

## Iteration log

<!-- one entry per hypothesis: change | N raw results (or pass-rate + median) | gate verdict | kept? -->
- (campaign not yet started — Tier-A iterations begin here.)
