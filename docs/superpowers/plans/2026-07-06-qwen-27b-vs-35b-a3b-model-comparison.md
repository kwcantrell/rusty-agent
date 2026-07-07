# Qwen3.6-27B vs 35B-A3B Model Comparison Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Decide which local model — Qwen3.6-27B dense (Q5_K_XL) or Qwen3.6-35B-A3B MoE (IQ4_XS, incumbent) — is the better daily driver for the rust-agent-runtime, via paired same-night eval blocks on the existing eval suite.

**Architecture:** This is an *operational benchmark*, not a code change. Tasks are ops procedures: container swaps on one RTX 3090, a capacity probe, sanity gates, two N=5×3-task eval blocks driven by the existing `eval_context` test, then scoring and a committed report. No harness or genome code is modified.

**Tech Stack:** docker + `ghcr.io/ggml-org/llama.cpp:server-cuda`, `agent/scripts/chat.sh`, `cargo test -p agent-runtime-config --test eval_context`, `jq`, `python3`.

**Spec:** `docs/superpowers/specs/2026-07-06-qwen-27b-vs-35b-a3b-model-comparison-design.md`

## Global Constraints

- **`locked-website` must NOT be run.** It is a one-shot metric owned by the harness-evolve campaign.
- **`memory-roster` runs with `CONFIG_JSON=champion_k10.json` — NEVER `realistic.json`.**
- **Never restart the server mid-batch.** A batch interrupted by a restart is discarded whole and re-run.
- **All eval env paths ABSOLUTE; `SKILLS_DIR` must be unset** (`unset SKILLS_DIR` in every eval shell).
- **`{"passed":false,"tokens":0,"turns":0}` on every run of a batch = server down** — check `docker ps`, discard the batch, restart, re-run. Do not debug the model.
- `cargo` needs `source ~/.cargo/env` first. Eval commands run from `/home/kalen/rust-agent-runtime/agent`.
- **Port 8080 is unreachable from the tool sandbox** — every command that talks to `localhost:8080` (curl, chat.sh, eval runs) or to docker must run with sandbox disabled (`dangerouslyDisableSandbox: true`).
- **Eval runs are long.** Run each eval invocation as its own Bash call with `timeout: 600000`, one run per call (the loop is unrolled). If a single web-multipage run exceeds 10 min, use `run_in_background` and wait.
- Working files live in `/tmp/bench-qwen/`; final artifacts are committed under `docs/superpowers/bench/`.
- The session must end with a healthy resident server (A3B by default). Verify `curl -s localhost:8080/health` returns `{"status":"ok"}` before declaring done.
- Commit only the files each task names. Conventional commits.

## Reference: server commands

**Incumbent A3B (exact tuned command, verified 2026-06-29):**

```bash
docker run -d --name llama-agent --gpus all -p 8080:8080 \
  -v /mnt/storage/models:/models:ro --restart no \
  ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/qwen3.6-35b-a3b-gguf/Qwen3.6-35B-A3B-UD-IQ4_XS.gguf \
  --alias qwen3.6-35b-a3b \
  -ngl 99 -c 196608 -np 4 --kv-unified \
  --cache-type-k q8_0 --cache-type-v q8_0 \
  --cache-ram 24576 --jinja --metrics --host 0.0.0.0
```

**Challenger 27B (context `$C` filled by the Task 2 probe):**

```bash
docker run -d --name llama-agent --gpus all -p 8080:8080 \
  -v /mnt/storage/models:/models:ro --restart no \
  ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/qwen3.6-27b-gguf/Qwen3.6-27B-UD-Q5_K_XL.gguf \
  --alias qwen3.6-27b \
  -ngl 99 -c $C -np 1 \
  --cache-type-k q8_0 --cache-type-v q8_0 \
  --jinja --metrics --host 0.0.0.0
```

**Swap procedure (always):** `docker stop llama-agent; docker rm llama-agent;` then the run command; then poll health:

```bash
for i in $(seq 1 60); do
  s=$(curl -s -m 2 localhost:8080/health | jq -r .status 2>/dev/null)
  [ "$s" = ok ] && echo HEALTHY && break
  sleep 5
done
```

## Reference: eval invocation

One eval run (from `.agents/skills/context-evolve/train.md`, env vars verified against `agent/crates/agent-runtime-config/tests/eval_context.rs`):

```bash
source ~/.cargo/env && cd /home/kalen/rust-agent-runtime/agent && unset SKILLS_DIR
AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=<alias> \
  TASK_JSON=<abs task.json> CONFIG_JSON=<abs config.json> HIDDEN_TESTS_DIR=<abs hidden_tests> \
  [EVAL_REAL_EMBEDDINGS=1 FASTEMBED_CACHE=/home/kalen/rust-agent-runtime/src-tauri/.fastembed_cache] \
  cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture 2>&1 \
  | grep -E '^\{"passed"' >> <results.jsonl>
```

Task matrix (paths all under `/home/kalen/rust-agent-runtime/.agents/skills/`):

| task | TASK_JSON | CONFIG_JSON | HIDDEN_TESTS_DIR | extra env |
|---|---|---|---|---|
| web-multipage | `harness-evolve/tasks/web-multipage/task.json` | `harness-evolve/tasks/web-multipage/champion_v0.json` | `harness-evolve/tasks/web-multipage/hidden_tests` | — |
| memory-roster | `context-evolve/tasks/memory-roster/task.json` | `context-evolve/tasks/memory-roster/champion_k10.json` | `context-evolve/tasks/memory-roster/hidden_tests` | `EVAL_REAL_EMBEDDINGS=1 FASTEMBED_CACHE=…` |
| locked-portmap | `context-evolve/tasks/locked-portmap/task.json` | `context-evolve/tasks/drift-ledger/champion_v4.json` (canonical champion-v4 copy; guard sweeps grade portmap on champion v4) | `context-evolve/tasks/locked-portmap/hidden_tests` | `EVAL_REAL_EMBEDDINGS=1 FASTEMBED_CACHE=…` |

Result JSONLs in `/tmp/bench-qwen/`: `<model>-<task>.jsonl` where model ∈ {`27b`,`a3b`} and task ∈ {`web`,`roster`,`portmap`} — e.g. `/tmp/bench-qwen/27b-web.jsonl`. Six files total, 5 lines each.

---

### Task 1: Preflight — record incumbent, verify assets, build the eval driver

**Files:**
- Create: `/tmp/bench-qwen/incumbent-inspect.json` (docker inspect snapshot)
- Create: `/tmp/bench-qwen/ledger.md` (running ops log: every config, timestamp, and anomaly gets a line)

**Interfaces:**
- Produces: `/tmp/bench-qwen/` directory; a verified-buildable eval driver; verified web-multipage seed + node image. Later tasks assume all of this and consume the incumbent run command from the Reference section.

- [ ] **Step 1: Create the working dir and snapshot the incumbent container**

```bash
mkdir -p /tmp/bench-qwen
docker inspect llama-agent > /tmp/bench-qwen/incumbent-inspect.json
jq -r '.[0].Config.Cmd | join(" ")' /tmp/bench-qwen/incumbent-inspect.json
```

Expected: the printed Cmd matches the Reference A3B command's arguments (`-m /models/qwen3.6-35b-a3b-gguf/… --alias qwen3.6-35b-a3b -ngl 99 -c 196608 -np 4 --kv-unified …`). If it differs, STOP and report — the Reference command must be corrected to the live one before any container is destroyed.

- [ ] **Step 2: Start the ledger**

```bash
{ echo "# bench ledger — qwen 27B vs 35B-A3B, $(date -Is)"; \
  echo "incumbent Cmd: $(jq -r '.[0].Config.Cmd | join(" ")' /tmp/bench-qwen/incumbent-inspect.json)"; } \
  > /tmp/bench-qwen/ledger.md
```

- [ ] **Step 3: Verify both model files and the server image**

```bash
ls -lh /mnt/storage/models/qwen3.6-27b-gguf/Qwen3.6-27B-UD-Q5_K_XL.gguf \
       /mnt/storage/models/qwen3.6-35b-a3b-gguf/Qwen3.6-35B-A3B-UD-IQ4_XS.gguf
docker image inspect ghcr.io/ggml-org/llama.cpp:server-cuda --format OK
```

Expected: both files listed (19G / 17G), `OK`.

- [ ] **Step 4: Verify the node sandbox image and web-multipage seed**

```bash
docker image inspect node:22-bookworm-slim --format OK || docker pull node:22-bookworm-slim
ls /home/kalen/rust-agent-runtime/.agents/skills/harness-evolve/tasks/web-multipage/seed/node_modules >/dev/null 2>&1 \
  && echo SEED-OK \
  || (cd /home/kalen/rust-agent-runtime/.agents/skills/harness-evolve/tasks/web-multipage && bash seed.sh && echo SEEDED)
```

Expected: `OK` (or a successful pull) and `SEED-OK`/`SEEDED`.

- [ ] **Step 5: Build the eval driver once (so eval batches never pay compile time mid-block)**

```bash
source ~/.cargo/env && cd /home/kalen/rust-agent-runtime/agent && \
  cargo build -p agent-runtime-config --tests 2>&1 | tail -3
```

Expected: `Finished` line, no errors.

- [ ] **Step 6: Verify the embeddings cache exists**

```bash
ls /home/kalen/rust-agent-runtime/src-tauri/.fastembed_cache >/dev/null && echo CACHE-OK
```

Expected: `CACHE-OK`. (If missing, the first memory-task run downloads the model — note it in the ledger; do not fail the task.)

- [ ] **Step 7: Log completion to the ledger** (append `preflight PASS $(date -Is)`). No commit — everything so far is in `/tmp`.

---

### Task 2: 27B bring-up + capacity probe

**Files:**
- Modify: `/tmp/bench-qwen/ledger.md` (probe results, final config, VRAM)

**Interfaces:**
- Consumes: Reference 27B command; swap procedure.
- Produces: a healthy 27B server on :8080 with alias `qwen3.6-27b`, and `C_FINAL` (the probed max context) recorded in the ledger. Tasks 3–4 use alias `qwen3.6-27b`; Task 4 re-launches with exactly this config.

- [ ] **Step 1: Take down the incumbent**

```bash
docker stop llama-agent && docker rm llama-agent
```

Expected: both commands print `llama-agent`. (Recovery from any point: the Reference A3B command restores the incumbent.)

- [ ] **Step 2: Probe the ladder 65536 → 49152 → 32768**

For each `C` in that order, launch with the Reference 27B command (substituting `-c $C`), poll health for up to 5 min, and check for OOM:

```bash
C=65536  # then 49152, then 32768
docker stop llama-agent 2>/dev/null; docker rm llama-agent 2>/dev/null
docker run -d --name llama-agent --gpus all -p 8080:8080 \
  -v /mnt/storage/models:/models:ro --restart no \
  ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/qwen3.6-27b-gguf/Qwen3.6-27B-UD-Q5_K_XL.gguf \
  --alias qwen3.6-27b -ngl 99 -c $C -np 1 \
  --cache-type-k q8_0 --cache-type-v q8_0 --jinja --metrics --host 0.0.0.0
for i in $(seq 1 60); do s=$(curl -s -m 2 localhost:8080/health | jq -r .status 2>/dev/null); [ "$s" = ok ] && echo HEALTHY && break; sleep 5; done
docker logs llama-agent 2>&1 | grep -iE 'out of memory|cuda error|failed' | tail -3
```

Stop at the FIRST `C` that reaches `HEALTHY` with no OOM lines → that is `C_FINAL`. If even 32768 fails, try `-ngl 60` at 32768 as a last resort (partial offload — note the speed penalty in the ledger); if that also fails, the challenger is non-viable: restore the A3B (Task 5 Step 1) and report per spec Phase 1.

- [ ] **Step 3: Record VRAM headroom and the final config**

```bash
nvidia-smi --query-gpu=memory.used,memory.total --format=csv
echo "27B C_FINAL=$C ngl=99 np=1; vram=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader) $(date -Is)" >> /tmp/bench-qwen/ledger.md
```

Expected: used ≤ ~23500 MiB (leave ≥1 GB free; if tighter, step `C` down one rung even though it booted).

- [ ] **Step 4: Confirm the model identifies correctly**

```bash
BASE=http://localhost:8080 /home/kalen/rust-agent-runtime/agent/scripts/chat.sh models
```

Expected: `qwen3.6-27b`.

---

### Task 3: 27B sanity gate + speed probe

**Files:**
- Create: `/tmp/bench-qwen/speed-27b.json`
- Modify: `/tmp/bench-qwen/ledger.md`

**Interfaces:**
- Consumes: healthy 27B server (Task 2).
- Produces: PASS/FAIL sanity verdict (gate for Task 4); `speed-27b.json` with `{"prompt_per_second": …, "predicted_per_second": …}` short- and long-context entries, consumed by Task 6's report.

All `chat.sh` calls: `CHAT=/home/kalen/rust-agent-runtime/agent/scripts/chat.sh`.

- [ ] **Step 1: Basic completion + thinking**

```bash
$CHAT ask "In one sentence, what does a context manager in an LLM agent do?"
```

Expected: a reasoning block and a coherent one-sentence answer.

- [ ] **Step 2: Single tool call**

```bash
jq -n '{model:"qwen3.6-27b", temperature:0.2, messages:[{role:"user",content:"What is the weather in Paris? You MUST use the get_weather tool."}], tools:[{type:"function","function":{name:"get_weather",parameters:{type:"object",properties:{city:{type:"string"}},required:["city"]}}}]}' \
  | RAW=1 $CHAT raw | jq '.choices[0].message.tool_calls'
```

Expected: one tool_call, `function.name == "get_weather"`, arguments containing `"Paris"`.

- [ ] **Step 3: Parallel tool calls (two in ONE turn)**

```bash
jq -n '{model:"qwen3.6-27b", temperature:0.2, messages:[{role:"user",content:"Fetch the weather for BOTH Paris and Tokyo. Call get_weather once per city, both calls in this single turn."}], tools:[{type:"function","function":{name:"get_weather",parameters:{type:"object",properties:{city:{type:"string"}},required:["city"]}}}]}' \
  | RAW=1 $CHAT raw | jq '.choices[0].message.tool_calls | length'
```

Expected: `2`. (If 1, retry once at TEMP=0.7; two consecutive singles = FAIL — record and stop the gate.)

- [ ] **Step 4: Tool-result round-trip**

```bash
jq -n '{model:"qwen3.6-27b", temperature:0.2, messages:[
  {role:"user",content:"What is the weather in Paris? Use get_weather."},
  {role:"assistant",content:"",tool_calls:[{id:"call_1",type:"function","function":{name:"get_weather",arguments:"{\"city\":\"Paris\"}"}}]},
  {role:"tool",tool_call_id:"call_1",content:"{\"temp_c\":31,\"sky\":\"clear\"}"}
], tools:[{type:"function","function":{name:"get_weather",parameters:{type:"object",properties:{city:{type:"string"}},required:["city"]}}}]}' \
  | RAW=1 $CHAT raw | jq -r '.choices[0].message.content'
```

Expected: an answer mentioning 31°C / clear skies (the tool result was consumed).

- [ ] **Step 5: `preserve_thinking` render check (template ground truth, NOT model behaviour)**

```bash
jq -n '[{role:"user",content:"Say OK."},{role:"assistant",content:"OK",reasoning_content:"PURPLE-MARKER reasoning"},{role:"user",content:"Again."}]' \
  > /tmp/bench-qwen/preserve-probe.json
PRESERVE=1 $CHAT render /tmp/bench-qwen/preserve-probe.json | grep -c PURPLE-MARKER
PRESERVE=0 $CHAT render /tmp/bench-qwen/preserve-probe.json | grep -c PURPLE-MARKER || true
```

Expected: `1` then `0` — prior reasoning kept only when `preserve_thinking` is on.

- [ ] **Step 6: Speed probe — short and long context**

llama.cpp includes a `timings` object in completion responses:

```bash
jq -n '{model:"qwen3.6-27b", temperature:0.2, max_tokens:256, messages:[{role:"user",content:"Count from 1 to 50, one number per line."}]}' \
  | RAW=1 $CHAT raw | jq '{short: .timings}' > /tmp/bench-qwen/speed-27b.json
python3 -c "print(('lorem ipsum dolor sit amet, consectetur adipiscing elit. ' * 900))" > /tmp/bench-qwen/longprompt.txt
jq -n --rawfile p /tmp/bench-qwen/longprompt.txt \
  '{model:"qwen3.6-27b", temperature:0.2, max_tokens:128, messages:[{role:"user",content:($p + "\n\nHow many times does the word lorem appear above, roughly? One line.")}]}' \
  | RAW=1 $CHAT raw | jq '{long: .timings}' >> /tmp/bench-qwen/speed-27b.json
cat /tmp/bench-qwen/speed-27b.json
```

Expected: both entries show `prompt_per_second` and `predicted_per_second` (long prompt ≈ 12–13K tokens — well inside any probed `C_FINAL`). If `timings` is null, fall back to wall-clock: `time` the call and divide `usage.completion_tokens` by seconds; record that instead.

- [ ] **Step 7: Verdict.** Append `sanity 27B: PASS|FAIL (<detail>) $(date -Is)` to the ledger. **FAIL on any of steps 2–5 stops the plan**: skip to Task 5 Step 1 (restore A3B), then Task 6 writes a non-viability report instead of a comparison.

---

### Task 4: 27B eval block (fresh start, then 15 runs, no restarts)

**Files:**
- Create: `/tmp/bench-qwen/27b-web.jsonl`, `/tmp/bench-qwen/27b-roster.jsonl`, `/tmp/bench-qwen/27b-portmap.jsonl`
- Modify: `/tmp/bench-qwen/ledger.md`

**Interfaces:**
- Consumes: `C_FINAL` from the ledger (Task 2); the eval invocation Reference.
- Produces: three JSONL files, 5 lines each, every line shaped `{"passed":bool,"tokens":N,"turns":N}`. Task 6 consumes them.

- [ ] **Step 1: Fresh container immediately before the block** — re-run the Task 2 Step 2 launch (with the recorded `C_FINAL`), wait for `HEALTHY`. Append `27B block start, C=$C_FINAL $(date -Is)` to the ledger.

- [ ] **Step 2: web-multipage ×5.** Run this FIVE times (one Bash call per run, `timeout: 600000`):

```bash
source ~/.cargo/env && cd /home/kalen/rust-agent-runtime/agent && unset SKILLS_DIR
T=/home/kalen/rust-agent-runtime/.agents/skills/harness-evolve/tasks/web-multipage
AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-27b \
  TASK_JSON=$T/task.json CONFIG_JSON=$T/champion_v0.json HIDDEN_TESTS_DIR=$T/hidden_tests \
  cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture 2>&1 \
  | grep -E '^\{"passed"' >> /tmp/bench-qwen/27b-web.jsonl
tail -1 /tmp/bench-qwen/27b-web.jsonl
```

Expected per run: one new line `{"passed":…,"tokens":…,"turns":…}` with tokens > 0.

- [ ] **Step 3: memory-roster ×5** (five calls):

```bash
source ~/.cargo/env && cd /home/kalen/rust-agent-runtime/agent && unset SKILLS_DIR
T=/home/kalen/rust-agent-runtime/.agents/skills/context-evolve/tasks/memory-roster
AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-27b \
  TASK_JSON=$T/task.json CONFIG_JSON=$T/champion_k10.json HIDDEN_TESTS_DIR=$T/hidden_tests \
  EVAL_REAL_EMBEDDINGS=1 FASTEMBED_CACHE=/home/kalen/rust-agent-runtime/src-tauri/.fastembed_cache \
  cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture 2>&1 \
  | grep -E '^\{"passed"' >> /tmp/bench-qwen/27b-roster.jsonl
tail -1 /tmp/bench-qwen/27b-roster.jsonl
```

- [ ] **Step 4: locked-portmap ×5** (five calls):

```bash
source ~/.cargo/env && cd /home/kalen/rust-agent-runtime/agent && unset SKILLS_DIR
T=/home/kalen/rust-agent-runtime/.agents/skills/context-evolve/tasks/locked-portmap
CV4=/home/kalen/rust-agent-runtime/.agents/skills/context-evolve/tasks/drift-ledger/champion_v4.json
AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-27b \
  TASK_JSON=$T/task.json CONFIG_JSON=$CV4 HIDDEN_TESTS_DIR=$T/hidden_tests \
  EVAL_REAL_EMBEDDINGS=1 FASTEMBED_CACHE=/home/kalen/rust-agent-runtime/src-tauri/.fastembed_cache \
  cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture 2>&1 \
  | grep -E '^\{"passed"' >> /tmp/bench-qwen/27b-portmap.jsonl
tail -1 /tmp/bench-qwen/27b-portmap.jsonl
```

- [ ] **Step 5: Block integrity check**

```bash
wc -l /tmp/bench-qwen/27b-*.jsonl
grep -c '"tokens":0' /tmp/bench-qwen/27b-*.jsonl || true
docker ps --format '{{.Names}} {{.Status}}'
```

Expected: `5` lines per file; zero `"tokens":0` lines (any all-zeros pattern = server died: verify with `docker ps`, restart, DISCARD the affected task's file whole, re-run that task's 5). Append pass counts per file to the ledger: `jq -s 'map(select(.passed))|length' <file>`.

---

### Task 5: A3B restore + eval block

**Files:**
- Create: `/tmp/bench-qwen/a3b-web.jsonl`, `/tmp/bench-qwen/a3b-roster.jsonl`, `/tmp/bench-qwen/a3b-portmap.jsonl`
- Modify: `/tmp/bench-qwen/ledger.md`

**Interfaces:**
- Consumes: Reference A3B command (cross-checked in Task 1 Step 1).
- Produces: three JSONL files, 5 lines each, same shape as Task 4's. The A3B is again the resident server from here on.

- [ ] **Step 1: Swap back to the A3B**

```bash
docker stop llama-agent; docker rm llama-agent
docker run -d --name llama-agent --gpus all -p 8080:8080 \
  -v /mnt/storage/models:/models:ro --restart no \
  ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/qwen3.6-35b-a3b-gguf/Qwen3.6-35B-A3B-UD-IQ4_XS.gguf \
  --alias qwen3.6-35b-a3b \
  -ngl 99 -c 196608 -np 4 --kv-unified \
  --cache-type-k q8_0 --cache-type-v q8_0 \
  --cache-ram 24576 --jinja --metrics --host 0.0.0.0
for i in $(seq 1 60); do s=$(curl -s -m 2 localhost:8080/health | jq -r .status 2>/dev/null); [ "$s" = ok ] && echo HEALTHY && break; sleep 5; done
BASE=http://localhost:8080 /home/kalen/rust-agent-runtime/agent/scripts/chat.sh models
```

Expected: `HEALTHY`, model id `qwen3.6-35b-a3b`. Append `A3B block start $(date -Is)` to the ledger.

- [ ] **Step 2: A3B speed probe** — repeat Task 3 Step 6 verbatim with `model:"qwen3.6-35b-a3b"`, writing `/tmp/bench-qwen/speed-a3b.json`.

- [ ] **Step 3: web-multipage ×5** — identical to Task 4 Step 2 except `AGENT_E2E_MODEL=qwen3.6-35b-a3b` and output `>> /tmp/bench-qwen/a3b-web.jsonl`.

- [ ] **Step 4: memory-roster ×5** — identical to Task 4 Step 3 except model alias and output `>> /tmp/bench-qwen/a3b-roster.jsonl`.

- [ ] **Step 5: locked-portmap ×5** — identical to Task 4 Step 4 except model alias and output `>> /tmp/bench-qwen/a3b-portmap.jsonl`.

- [ ] **Step 6: Block integrity check** — same as Task 4 Step 5, on `a3b-*.jsonl`. Sanity reference (NOT a gate): historical A3B rates were web-multipage ~5/10, roster ≥9/10 @ k10, portmap 10/10. A wild deviation (e.g. roster 1/5) suggests an env mistake — check `CONFIG_JSON` and the server before accepting the batch.

---

### Task 6: Scoring + report

**Files:**
- Create: `docs/superpowers/bench/2026-07-06-qwen-27b-vs-35b-a3b-report.md`
- Create: `docs/superpowers/bench/2026-07-06-qwen-27b-vs-35b-a3b-runs.jsonl` (all six batches concatenated, each line augmented with `model`/`task`)
- Test: the numbers in the report reproduce from the JSONLs (Step 3 check)

**Interfaces:**
- Consumes: six `/tmp/bench-qwen/*-*.jsonl` files, two `speed-*.json` files, the ledger.
- Produces: the committed report — the deliverable of the whole plan.

- [ ] **Step 1: Compute the score table**

```bash
cd /tmp/bench-qwen
for f in 27b-web 27b-roster 27b-portmap a3b-web a3b-roster a3b-portmap; do
  echo "$f: pass=$(jq -s 'map(select(.passed))|length' $f.jsonl)/5 median_tokens_passing=$(jq -s '[.[]|select(.passed)|.tokens]|sort|if length==0 then null else .[(length/2|floor)] end' $f.jsonl)"
done
```

- [ ] **Step 2: Fisher exact on web-multipage (two-sided, no scipy dependency)**

```bash
python3 - <<'EOF'
import json, math
def passes(p):
    return sum(1 for l in open(p) if json.loads(l)["passed"])
a = passes("/tmp/bench-qwen/27b-web.jsonl"); b = 5 - a
c = passes("/tmp/bench-qwen/a3b-web.jsonl"); d = 5 - c
n, r1, c1 = a+b+c+d, a+b, a+c
def hyp(x): return math.comb(r1,x)*math.comb(n-r1,c1-x)/math.comb(n,c1)
p0 = hyp(a)
p = sum(hyp(x) for x in range(max(0,c1-(n-r1)), min(r1,c1)+1) if hyp(x) <= p0 + 1e-12)
print(f"27B {a}/5 vs A3B {c}/5, Fisher two-sided p={p:.4f}")
EOF
```

- [ ] **Step 3: Build the combined runs ledger and verify it reproduces the table**

```bash
mkdir -p /home/kalen/rust-agent-runtime/docs/superpowers/bench
: > /home/kalen/rust-agent-runtime/docs/superpowers/bench/2026-07-06-qwen-27b-vs-35b-a3b-runs.jsonl
for m in 27b a3b; do for t in web roster portmap; do
  jq -c --arg m $m --arg t $t '. + {model:$m, task:$t}' /tmp/bench-qwen/$m-$t.jsonl \
    >> /home/kalen/rust-agent-runtime/docs/superpowers/bench/2026-07-06-qwen-27b-vs-35b-a3b-runs.jsonl
done; done
jq -s 'group_by(.model+.task) | map({k: (.[0].model+"-"+.[0].task), pass: map(select(.passed))|length})' \
  /home/kalen/rust-agent-runtime/docs/superpowers/bench/2026-07-06-qwen-27b-vs-35b-a3b-runs.jsonl
```

Expected: 30 lines total; the grouped pass counts match Step 1's table exactly.

- [ ] **Step 4: Write the report** at `docs/superpowers/bench/2026-07-06-qwen-27b-vs-35b-a3b-report.md` with these sections (fill every number from Steps 1–2, `speed-*.json`, and the ledger — no placeholders):
  - **Verdict** — one paragraph. Decision rule from the spec: web-multipage pass rate is the headline; if pass counts tie, correctness-gated token tiebreak (lower median tokens among passing runs wins); if still effectively tied or the Fisher p is large with a small gap, say **"no defensible call at N=5"** and offer the N=10 web-multipage extension. State explicitly that the resident server remains the A3B unless the user chooses to switch.
  - **Score table** — per model × task: pass/5, median tokens (passing only), Fisher p on the web row.
  - **Capacity split** — any 27B failure whose run log/ledger shows a context-capacity cause (prompt > `C_FINAL`) tagged and the headline restated excluding them (informational; they still count per spec decision 3).
  - **Server configs** — both exact docker commands, `C_FINAL`, VRAM used, per the ledger.
  - **Speed table** — short/long prompt_per_second + predicted_per_second per model.
  - **Method + caveats** — N=5 noise (cite the campaign's N=2 lesson), sequential-not-interleaved blocks (single GPU), same-night pairing honored, sampler defaults (TEMP=0.2 probes; eval harness defaults for eval runs), links to spec and the raw runs JSONL.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add docs/superpowers/bench/2026-07-06-qwen-27b-vs-35b-a3b-report.md \
        docs/superpowers/bench/2026-07-06-qwen-27b-vs-35b-a3b-runs.jsonl
git commit -m "docs(bench): qwen 27B-dense vs 35B-A3B comparison — report + raw runs"
```

---

### Task 7: End state + memory updates

**Files:**
- Modify: `/home/kalen/.claude/projects/-home-kalen-rust-agent-runtime/memory/local-llama-server.md` (add the 27B's measured `C_FINAL` + viability note; resident-model line only if the user switched)
- Create: `/home/kalen/.claude/projects/-home-kalen-rust-agent-runtime/memory/qwen-27b-vs-a3b-comparison.md`
- Modify: `/home/kalen/.claude/projects/-home-kalen-rust-agent-runtime/memory/MEMORY.md` (one index line)

**Interfaces:**
- Consumes: the committed report (Task 6); a running A3B server (Task 5).

- [ ] **Step 1: Verify the end state**

```bash
curl -s localhost:8080/health
BASE=http://localhost:8080 /home/kalen/rust-agent-runtime/agent/scripts/chat.sh models
docker ps --format '{{.Names}} {{.Status}}'
```

Expected: `{"status":"ok"}`, `qwen3.6-35b-a3b`, `llama-agent Up …`.

- [ ] **Step 2: Write the comparison memory** — frontmatter `type: project`, name `qwen-27b-vs-a3b-comparison`; body: verdict + headline numbers, `C_FINAL`, speed summary, pointer to the report path, and the "no defensible call → optional N=10 extension" state if applicable. Link `[[local-llama-server]]`.

- [ ] **Step 3: Update `local-llama-server.md`** — append a dated paragraph: 27B Q5_K_XL boots at `C_FINAL` context / `-np 1` on the 3090 (or is non-viable, if so), exact challenger docker command, and that the A3B remains resident.

- [ ] **Step 4: Add the MEMORY.md index line** — `- [Qwen 27B vs A3B comparison](qwen-27b-vs-a3b-comparison.md) — <one-line verdict>`.

- [ ] **Step 5: Report to the user** — verdict, table, capacity split, speed, and the offer: switch resident to the 27B (if it won) or extend to N=10 (if no defensible call).
