# Qwen3.6-27B (dense) vs Qwen3.6-35B-A3B (MoE) — local model comparison

**Date:** 2026-07-06
**Status:** Approved design
**Goal:** Decide which model is the better daily driver for the rust-agent-runtime
local agent, measured by agent-task quality on the existing eval suite, with
speed and context capacity recorded as secondary data.

## Contenders

| | 35B-A3B (incumbent) | 27B dense (challenger) |
|---|---|---|
| File | `/mnt/storage/models/qwen3.6-35b-a3b-gguf/Qwen3.6-35B-A3B-UD-IQ4_XS.gguf` | `/mnt/storage/models/qwen3.6-27b-gguf/Qwen3.6-27B-UD-Q5_K_XL.gguf` |
| Weights | 17 GB | 19 GB |
| Active params/token | ~3B (MoE) | 27B (dense) |
| Known server config | tuned: `-c 196608 -np 4 --kv-unified`, q8_0 KV, `--cache-ram 24576` | unknown — capacity probe required |

Hardware: single RTX 3090 (24 GB). The models cannot coexist; the comparison is
sequential with container swaps. Server image: `ghcr.io/ggml-org/llama.cpp:server-cuda`,
container name `llama-agent`, port 8080 (see the `llama-server` skill and the
`local-llama-server` memory for the verified relaunch command).

## Decisions (settled with the user — do not relitigate)

1. **Measure agent-task quality**, not raw benchmarks. Speed/latency is secondary data.
2. **Standard suite, N=5 per task per model:** `web-multipage` (harness-evolve),
   `memory-roster` @ `champion_k10.json`, `locked-portmap` (context-evolve).
3. **Each model at its own best server config.** The A3B keeps its tuned 192K setup;
   the 27B gets the largest context that empirically fits. Capacity-caused failures
   count as real losses but are tagged separately in the report.
4. **`locked-website` is out of scope.** It is the harness-evolve campaign's one-shot
   honest metric; a model comparison must not spend it.

## Method

### Phase 1 — 27B bring-up + capacity probe

1. Record the incumbent's exact run command from `docker inspect llama-agent`
   (cross-check against the memory copy), then `docker stop llama-agent && docker rm llama-agent`.
2. Launch the 27B with the same image, `--alias qwen3.6-27b`, q8_0 KV, `--jinja --metrics`.
   Probe the largest working context: start `-c 65536 -np 1`; on OOM step down the
   ladder 64K → 48K → 32K. Record the final config and free VRAM.
3. **Sanity gate (must pass before any eval spend):** `/health` ok; via
   `agent/scripts/chat.sh`: basic completion, a tool-call round-trip, parallel tool
   calls in one turn, and `reasoning_content` round-trip with `preserve_thinking`
   (the same capabilities verified on the A3B — see `qwen36-preserve-thinking` memory).
4. Record speed: prefill and generation tok/s from `/metrics` on a standardized probe
   (one short-context and one long-context request).

If the 27B cannot pass the sanity gate or hold ≥32K context, stop, report, and restore
the A3B — no eval spend on a non-viable challenger.

### Phase 2 — eval blocks (paired same-night, sequential by necessity)

Order: **27B block first** (it is the unknown), then a **fresh A3B block** — historical
A3B numbers are sanity references only; the comparison uses same-night runs on both sides.

Per model block:

- Fresh container start immediately before the block. Never restart mid-batch; a batch
  interrupted by a restart is discarded whole (harness-evolve protocol).
- Runs, all via `cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture`
  (in `agent/`, `source ~/.cargo/env` first), collecting the `{"passed":...}` JSONL lines:
  - `web-multipage` N=5 — `TASK_JSON`/`HIDDEN_TESTS_DIR` from
    `.agents/skills/harness-evolve/tasks/web-multipage/`, `CONFIG_JSON=champion_v0.json`
    (in that task dir). Requires `docker pull node:22-bookworm-slim` and a fresh
    `seed.sh` run if `seed/` isn't populated.
  - `memory-roster` N=5 — task dir `.agents/skills/context-evolve/tasks/memory-roster/`,
    `CONFIG_JSON=champion_k10.json` (NEVER `realistic.json`).
  - `locked-portmap` N=5 — task dir `.agents/skills/context-evolve/tasks/locked-portmap/`;
    `CONFIG_JSON` = the context-evolve champion v4 config (canonical copy at
    `tasks/drift-ledger/champion_v4.json`; confirm the window field suits portmap against
    `program.md`'s champion block at plan time).
- Env discipline (campaign ops notes): all paths ABSOLUTE; `SKILLS_DIR` unset;
  memory tasks run with `EVAL_REAL_EMBEDDINGS=1` and
  `FASTEMBED_CACHE=/home/kalen/rust-agent-runtime/src-tauri/.fastembed_cache`;
  `AGENT_E2E_URL=http://localhost:8080`; `AGENT_E2E_MODEL=<alias>` per block.
- `{"passed":false,"tokens":0,"turns":0}` on every run = server down; check
  `docker ps` before blaming the model. The block is then invalid and re-run whole.

### Phase 3 — scoring

- **Headline:** per-task pass rates; Fisher exact on `web-multipage` (the daily-driver
  metric).
- **Tiebreak:** median tokens among passing runs only (correctness-gated token
  tiebreak, per `harness-engineering/eval.md`).
- **Secondary table:** max context achieved, prefill tok/s, gen tok/s, median
  wall-clock per run.
- **Capacity attribution:** any 27B failure where the harness attempted a prompt
  exceeding the 27B's server context is tagged `capacity` and reported both included
  in and excluded from the headline (decision 3 keeps them included; the split is
  informational).
- A close race at N=5 is reported as "no defensible call" (campaign history: N=2
  margins were noise), with an optional N=10 extension on `web-multipage` only.

### Phase 4 — end state + rollback

The session ends with a healthy resident server, whichever way the verdict goes:
default is restoring the A3B with its exact tuned command unless the user, seeing the
report, chooses to switch. `/health` verified before completion. If the 27B OOMs or
wedges at any point, the A3B relaunch command in the `local-llama-server` memory is
the recovery path.

## Deliverables

1. Comparison report — tables, verdict, raw JSONL run ledger, capacity split —
   committed under `docs/superpowers/` alongside this spec.
2. Memory updates: new comparison-results memory; `local-llama-server` updated if the
   resident model changes (and with the 27B's measured max-context either way).

## Non-goals

- No harness/genome changes — champion configs are used as-is on both sides.
- No `locked-website` runs.
- No promotion decision for harness-evolve; this is a model comparison, not a
  campaign phase.
