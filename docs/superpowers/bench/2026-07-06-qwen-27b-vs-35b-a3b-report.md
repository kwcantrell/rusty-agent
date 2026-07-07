# Qwen3.6 27B-dense vs 35B-A3B — Benchmark Report

**Date:** 2026-07-06  
**Challenger:** Qwen3.6-27B-UD-Q5_K_XL (dense, -np 1, C=65536)  
**Incumbent:** Qwen3.6-35B-A3B-UD-IQ4_XS (MoE, -np 4, C=196608)  
**Raw runs:** [`2026-07-06-qwen-27b-vs-35b-a3b-runs.jsonl`](2026-07-06-qwen-27b-vs-35b-a3b-runs.jsonl) (30 lines)

---

## Verdict

The 27B leads 4/5 vs 3/5 on the headline task (web-multipage). Fisher two-sided p=1.0000 on N=5 per arm: this is the "small gap, large p" case. **The honest verdict is: 27B leads on this sample; not statistically defensible at N=5.** No model switch is warranted by these numbers alone. The resident server remains the A3B unless the user explicitly chooses to switch.

Trade-off summary: the 27B showed equal-or-better pass rates on every task this night (web 4/5 vs 3/5, roster 5/5 vs 4/5, portmap 5/5 vs 5/5), at approximately 3.7× slower generation (32.98 vs 121.45 predicted tok/s) and one-third the context ceiling (65K vs 196K tokens). If the gap holds at N=10 web-multipage it becomes a defensible lead — a ten-run web extension is the natural next step for anyone who wants a stronger signal before committing to a swap.

---

## Score Table

| model | task         | pass/5 | median tokens (passing only) |
|-------|--------------|--------|------------------------------|
| 27B   | web-multipage | 4/5   | 83,615                       |
| 27B   | memory-roster | 5/5   | 70,638                       |
| 27B   | locked-portmap | 5/5  | 52,632                       |
| A3B   | web-multipage | 3/5   | 71,688                       |
| A3B   | memory-roster | 4/5   | 78,897                       |
| A3B   | locked-portmap | 5/5  | 56,204                       |

Fisher exact (two-sided) on web-multipage: **p=1.0000** (27B 4/5 vs A3B 3/5).

Note on median tokens: the tiebreak rule (lower median among passing runs) applies only when pass counts are equal. Here web pass counts differ (4 vs 3), so the tiebreak is not invoked. The A3B's lower passing-run median (71,688 vs 83,615) is informational only.

---

## Capacity Split

The 27B ran at C_FINAL=65536 with the eval harness context window at approximately 4K tokens per turn. **Zero runs came close to the context ceiling; zero failures are tagged as capacity failures.** The sole 27B failure (web run 2) was a behavioral miss: the model formatted the output as `latencyP95: \`${raw.p95_ms}ms\`` (no space before the unit) where the hidden test expected `"142 ms"`. This is a code-quality failure, not a context or server failure. The headline (4/5 web) is unchanged by the capacity split.

---

## Server Configs

**Challenger (27B):**
```
docker run -d --name llama-agent --gpus all -p 8080:8080 \
  -v /mnt/storage/models:/models:ro --restart no \
  ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/qwen3.6-27b-gguf/Qwen3.6-27B-UD-Q5_K_XL.gguf \
  --alias qwen3.6-27b -ngl 99 -c 65536 -np 1 \
  --cache-type-k q8_0 --cache-type-v q8_0 --jinja --metrics --host 0.0.0.0
```
C_FINAL=65536. VRAM at startup: 23,136 / 24,576 MiB (per ledger probe).

**Incumbent (A3B):**
```
docker run -d --name llama-agent --gpus all -p 8080:8080 \
  -v /mnt/storage/models:/models:ro --restart no \
  ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/qwen3.6-35b-a3b-gguf/Qwen3.6-35B-A3B-UD-IQ4_XS.gguf \
  --alias qwen3.6-35b-a3b \
  -ngl 99 -c 196608 -np 4 --kv-unified \
  --cache-type-k q8_0 --cache-type-v q8_0 \
  --cache-ram 24576 --jinja --metrics --host 0.0.0.0
```
C_FINAL=196608 (3× the challenger). VRAM at runtime: not recorded in ledger (--cache-ram 24576 MiB RAM-side KV budget configured).

**Config asymmetry note:** The A3B runs at -np 4 with a 192K shared context pool; the 27B ran at -np 1 at 64K. This follows the spec's "each at its best" decision — the 27B cannot hold 192K in a single slot at 24 GiB VRAM, and parallelism is meaningless at -np 1 for a single eval stream. The speed comparison below reflects this asymmetry and should be read accordingly.

---

## Speed Table

Measured via `/v1/completions` probes (TEMP=0.2, greedy-equivalent). See task-3 and task-5 reports for probe details.

| model | prompt    | prompt tok/s | predicted tok/s |
|-------|-----------|--------------|-----------------|
| 27B   | short (24 tok)  | 155.68 | 32.98  |
| 27B   | long (9027 tok) | 1170.51 | 31.94 |
| A3B   | short (24 tok)  | 110.69 | 121.45 |
| A3B   | long (9027 tok) | 3450.14 | 115.95 |

Generation speed ratio (predicted tok/s): A3B 121.45 / 27B 32.98 = **3.68×** faster. This gap reflects MoE sparse activation (A3B) vs dense computation (27B), compounded by A3B's -np 4 allowing greater GPU utilization across parallel slots.

---

## Method + Caveats

**Protocol:** Two sequential eval blocks on a single GPU (one model active at a time). The 27B block ran first (19:47–20:38 PDT); A3B was swapped in immediately after (20:39–20:59 PDT). Same-night pairing honored — no intervening reboots, GPU state stable throughout.

**N=5 noise:** Each task has five runs per model. At N=5, a one-run difference between models has Fisher p=1.0 or higher — no gap at this sample size is statistically distinguishable from noise. This benchmark's N=2 campaign lesson (from the context-evolve training) applies directly: N=5 is enough to detect a catastrophic regression but not enough to rank two similarly-capable models. Treat these results as directional signal, not a definitive ranking.

**Sampler defaults:** Speed probes used TEMP=0.2 explicitly. Eval harness runs used harness defaults (configured per task — web-multipage and roster use the harness's own sampler config; portmap uses champion_v4.json). No custom temperature overrides were applied during eval runs.

**Caveats:**
- Web run 5 for the 27B required a single redo: the original Bash client was killed by the 10-minute client-side cap (exit code 143). The server was confirmed healthy immediately after; the redo succeeded in 162s per the redo protocol. The redo result (pass=true, tokens=60,295) is the line of record.
- Web run 2 for the 27B failed on a latency formatting string ("142ms" vs expected "142 ms"). This is a behavioral quality issue at the code-generation level, not an infrastructure or context issue.
- A3B roster run 4 failed because the model issued only 7 of 8 required `remember` calls (HX-457 skipped). Server was healthy, config correct (champion_k10.json). Within historical variance.
- A3B web runs 1 and 4 failed with noise-reading loop / context exhaustion patterns, consistent with A3B's historical ~5/10 web-multipage rate.

**Links:**
- Raw runs: `docs/superpowers/bench/2026-07-06-qwen-27b-vs-35b-a3b-runs.jsonl`
- Spec: `.superpowers/sdd/task-6-brief.md` (scoring rules and decision tree)
- Task reports: `.superpowers/sdd/task-4-report.md` (27B block), `.superpowers/sdd/task-5-report.md` (A3B block)
