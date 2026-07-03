---
name: harness-evolve
description: >-
  Use to run a self-improving optimization campaign on this runtime's WHOLE
  harness (system prompt, tools, sub-agents, agent-side SDLC skills, memory,
  sampling, server topology) against long-running web-coding tasks. Iteratively
  edits genome/config/code, evals against a live model on frozen tasks, and
  keeps a change only when task success holds and total tokens drop. Invoke to
  optimize the harness for complex programming tasks; for context-curation
  tuning use context-evolve.
---

# harness-evolve

Optimize the harness so the running model finishes long, complex programming
tasks (canonical: a working TypeScript website — Vite, typecheck, tests) at a
realistic window. Sibling campaign to `context-evolve` — same method, wider
genome. Spec: `docs/superpowers/specs/2026-07-03-harness-evolve-campaign-design.md`.

- `prepare.md` — author/admit a web task (offline seed, exec_profile, ladder).
- `train.md` — the per-iteration loop and the cross-campaign guard sweep.
- `program.md` — append-only research memory + current champion. READ FIRST.

## The objective (never violate)

1. A change that lowers the pass count on the training set is **rejected**.
2. Among correctness-preserving changes, prefer **lower median tokens** (passing runs).
3. A promotion must not regress ANY held-out task NOR any task in
   context-evolve's admitted set (shared runtime — hard gate).
4. The honest metric is the **locked task** (canonical end-to-end site), run
   once at campaign end. `wall_ms` is diagnostic only; it never gates.

## Tiers

- **Tier A (genome, no rebuild):** a CandidateConfig JSON — context/memory knobs
  plus v2 axes: `system_prompt`, `protocol`, `active_skills`, `skills_dirs`,
  `temperature`/`top_p`/`top_k`/`min_p`, `subagents`/`subagent_max_turns`/
  `subagent_max_depth`/`subagent_model`, `tool_descriptions`,
  `max_result_bytes`, `max_turns`. **Tier A′:** editing candidate runtime-skill
  FILES under `artifacts/agent-skills/<variant>/` (no rebuild).
- **Tier B (code, rebuild):** runtime code (`agent-core`, `agent-tools`,
  `agent-skills`, `agent-memory`, prompts.rs). Snapshot-binary pairing; FULL
  guard sweep mandatory.
- **Tier C (server topology, restart):** llama-server flags / model swaps.
  Every Tier-C change is a RE-BASELINING EVENT — dedicated spikes only, never a
  per-iteration variable.

## Prerequisites

- Live server (`llama-server` skill): `AGENT_E2E_URL=http://localhost:8080`,
  `AGENT_E2E_MODEL=qwen3.6-35b-a3b`. `{"passed":false,"tokens":0,"turns":0}`
  on every run ⇒ server down or URL wrong — check `docker ps` first.
- `source ~/.cargo/env && cd agent && cargo build -p agent-runtime-config --tests --bins`
- Web tasks: `docker pull node:22-bookworm-slim` once, and the task's `seed.sh`
  run once (builds `seed/node_modules` offline seeds).
- Memory-mode tasks: `EVAL_REAL_EMBEDDINGS=1 FASTEMBED_CACHE=src-tauri/.fastembed_cache`.

**Do not** use this skill for one-off harness tweaks or any change that skips
the eval gate; do not tune memory params against the stub embedder.
