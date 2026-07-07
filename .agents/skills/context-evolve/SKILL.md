---
name: context-evolve
description: >-
  Use to run a self-improving optimization campaign on this runtime's
  context-management subsystem (in-window curation in agent-core + long-term
  memory in agent-memory). Iteratively edits curation params/code, evals against
  a live model on frozen tasks, and keeps a change only when correctness holds and
  total tokens drop. Invoke when asked to optimize/tune context management, reduce
  drift on long tasks, or cut token usage without losing correctness.
---

# context-evolve

Optimize the context manager so the running model solves hard, long-horizon tasks
**without drifting** and in **fewer total tokens**. The objective is lexicographic:
**correctness is a hard gate; tokens are only a tiebreaker.** Three playbooks:

- `prepare.md` — author/admit a task (weakness-first, two-sided test) and set the
  champion baseline.
- `train.md` — the per-iteration loop: hypothesize → edit → eval N× → gate → record.
- `program.md` — accumulated learnings + the current champion config (append-only).

**Do not** use this skill for one-off context tuning or manual config edits
(→ `context-management` skill), nor for any change that would skip the eval
gate — it exists only for the full hypothesize → eval → gate campaign loop.

## Prerequisites

- A live server (see the `llama-server` skill). Export `AGENT_E2E_URL` (e.g.
  `http://localhost:8080`) and `AGENT_E2E_MODEL` (e.g. `qwen3.6-35b-a3b`).
- Build the harness + CLI once:
  `cd agent && cargo build -p agent-runtime-config --tests --bins`

## The objective (never violate)

1. A change that lowers the pass count on the training set is **rejected**.
2. Among correctness-preserving changes, prefer **lower median tokens** (passing runs only).
3. A promotion must not regress **any** held-out task's pass rate (hard gate).
4. The honest success metric is the **locked real-commit set**, run **once** at campaign end.

## What you may change

- **Tier A (params, no rebuild):** edit a candidate config JSON — in-window
  (`context_limit`, `high_water_pct`, `keep_recent`, `error_min_bytes`,
  `output_min_bytes`, `recall_budget`) and memory (`default_k`,
  `relevance_threshold`, `dedup_threshold`, `forget_threshold`, `max_recall_chars`,
  `recall_token_budget`, `auto_recall`). Prove the signal here first.
- **Tier B (code, rebuild):** edit the curation logic itself —
  `agent-core/src/{curated,offload_policy,compactor}.rs` or `agent-memory`'s
  `retriever.rs` / `tools.rs`. Unlock only after Tier A has moved the metric.

## How a run works

`eval_context` drives the real `assemble_loop` on a frozen task under one config and
prints one `RunResult` JSON line `{"passed":bool,"tokens":u64,"turns":n}`. The token
figure is the faithful server-reported total (prompt+completion summed over turns).
`eval_gate` turns batches of those lines into a gate/admissibility verdict.

Token = the cost we minimize; passed = the gate we never sacrifice.
