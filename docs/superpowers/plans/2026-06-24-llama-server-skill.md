# llama-server Agent Skill — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Author a self-contained, repo-agnostic `.agents/skills/llama-server/` skill (hub `SKILL.md` + 10 reference files) teaching any agent to build/obtain, launch, operate, call, and troubleshoot llama.cpp's `llama-server`.

**Architecture:** Mirror the existing `.agents/skills/tauri/` skill: a short high-signal hub `SKILL.md` (frontmatter + numbered sections + decision table + verify section) that delegates depth to focused `references/*.md` files. Reference files are independent and can be authored in parallel by subagents; the hub is written as the synthesis/navigation layer. A final pass enforces internal consistency and repo-agnosticism.

**Tech Stack:** Markdown only. Ground truth = the canonical `tools/server/README.md` (captured during research) + deep-research-verified facts + a focused Rust-integration research pass. No code, no build step.

## Global Constraints

- **Repo-agnostic, always.** No string may reference this repository, its paths, crates, services, ports, or model names. Generic placeholders only: `model.gguf`, `localhost:8080`, `127.0.0.1`, plain `curl`. Verification: `grep -rIEli 'rust-agent-runtime|agent-server\.json|graphify|cloudflare|qwen3\.6|/home/kalen' .agents/skills/llama-server` must return nothing.
- **Format parity with `tauri` skill.** YAML frontmatter with `name` + a trigger-rich `description`. Prose style: focused, copy-pasteable fenced code blocks, version-caveated claims.
- **Subject = `llama-server` only.** Not `llama-cli`/`llama-bench`/`llama-quantize`; not training. The one exception is `rust-integration.md`, which additionally covers native llama.cpp Rust bindings as the explicit "no-server alternative."
- **Every claim traces to ground truth** (canonical README or verified research). Flag version-dependent items in-text: HF-cache migration (#21364); `--jinja` default-on (was off in older builds); `-ngl auto`/`all` recent; `/health` vs `/slots` split (PR #9056, breaking); `--webui` deprecated → `--ui` (#18155); JSON-Schema→GBNF is a subset.
- **Default port/host:** `8080` / `127.0.0.1`. **Key flag facts** (reuse verbatim across files): `-c`/`--ctx-size` is TOTAL KV split across `-np` slots; `-ngl`/`--gpu-layers` = GPU offload layers (`99`/`all`/`auto`); `-hf <user>/<repo>[:quant]` defaults to `Q4_K_M`; `--jinja` required for tool calling; `--host 0.0.0.0` for external/Docker access; env vars are `LLAMA_ARG_*` (CLI overrides them).
- **Model selection per task** (the user asked to pick the right model per step): use a **strong** model (opus) for the hub and the judgment-heavy files (`structured-output-and-tools`, `performance-tuning`, `troubleshooting`, `rust-integration`) and final review; a **standard** model (sonnet) for the mechanical reference tables (`launching`, `cli-reference`, `http-api`, `embeddings-and-reranking`, `multimodal`, `advanced-inference`).

## File Structure

```
.agents/skills/llama-server/
├─ SKILL.md                              # hub (Task 1)
└─ references/
   ├─ launching.md                       # Task 2
   ├─ cli-reference.md                   # Task 3
   ├─ http-api.md                        # Task 4
   ├─ structured-output-and-tools.md     # Task 5
   ├─ embeddings-and-reranking.md        # Task 6
   ├─ multimodal.md                      # Task 7
   ├─ advanced-inference.md              # Task 8
   ├─ performance-tuning.md              # Task 9
   ├─ rust-integration.md                # Task 10
   └─ troubleshooting.md                 # Task 11
docs/superpowers/plans/...               # this plan
```

Each reference file has one responsibility and is independently readable. The hub never duplicates reference depth — it links out via the decision table.

## Verification model (applies to every task)

Because this is documentation, each task's "test cycle" is:
1. **Write** the file with the exact sections listed in the task.
2. **Verify** (the task's gate), running all that apply:
   - Frontmatter present & parseable (hub only): `head -1` is `---`; `name:` and `description:` keys exist.
   - Repo-agnostic: the Global-Constraints `grep` returns nothing for the new file.
   - Self-contained: every internal link target (`references/<x>.md`) exists on disk.
   - Fact-trace: spot-check flag/endpoint names against `tools/server/README.md` ground truth (no invented flags).
3. **Commit** the file.

A task is "done" only when its Verify step passes with shown output.

---

### Task 1: Scaffold + hub `SKILL.md`

**Model:** opus (synthesis/navigation layer).

**Files:**
- Create: `.agents/skills/llama-server/SKILL.md`

**Interfaces:**
- Produces: the decision table that names all 10 reference paths (Tasks 2–11 must create exactly these filenames).

- [ ] **Step 1: Create the directory**

Run: `mkdir -p .agents/skills/llama-server/references`

- [ ] **Step 2: Write `SKILL.md`** with this exact structure:

Frontmatter:
```yaml
---
name: llama-server
description: >-
  Use when serving a local GGUF LLM over HTTP with llama.cpp's `llama-server`
  (the OpenAI-compatible server in ggml-org/llama.cpp, tools/server) — building
  or obtaining the binary, launching and configuring it, calling its HTTP API
  (/v1/chat/completions, /v1/completions, /v1/embeddings, /completion, /props,
  /health, /slots), structured output (GBNF / JSON schema), tool/function
  calling (--jinja), embeddings/reranking, multimodal (--mmproj), speculative
  decoding, LoRA, performance/slot tuning, and troubleshooting. Trigger on
  mentions of llama-server, llama.cpp server, "run a local LLM server", a GGUF
  model on localhost:8080, -m/-hf/-ngl/-c/-np, --jinja, or an OpenAI-compatible
  local endpoint.
---
```

Body sections (scale to the design spec `docs/superpowers/specs/2026-06-24-llama-server-skill-design.md`):
1. `# Serving local LLMs with llama-server` + 2–3 sentence intro: what it is (OpenAI-compatible HTTP server for GGUF models, part of llama.cpp), and the hub/references model.
2. `## When to use this skill` / when NOT (not training; not `llama-cli`/`llama-bench`; overlaps but isn't Ollama/vLLM).
3. `## 1. Get the binary` — bullets: package managers (`brew install llama.cpp`, winget, etc.), prebuilt GitHub release binaries, build from source (`cmake -B build && cmake --build build --config Release -j` → `./build/bin/llama-server`; GPU via `-DGGML_CUDA=ON`/`-DGGML_METAL=ON`/`-DGGML_VULKAN=ON`), Docker GHCR (`ghcr.io/ggml-org/llama.cpp:server` and `:full`/`:light`, GPU suffixes `-cuda`/`-rocm`/`-vulkan`/`-intel`/`-musa`; needs `--gpus all` + `-ngl`). One-line pointer to `references/launching.md`.
4. `## 2. 30-second quickstart` — fenced block:
   ```bash
   # Serve a model (auto-download from Hugging Face), all layers on GPU:
   llama-server -hf ggml-org/gemma-3-1b-it-GGUF -c 4096 -ngl 99
   # or a local file:  llama-server -m ./model.gguf -c 4096 -ngl 99
   ```
   then:
   ```bash
   curl http://localhost:8080/v1/chat/completions -H "Content-Type: application/json" \
     -d '{"messages":[{"role":"user","content":"Say hello in one word."}]}'
   ```
5. `## 3. The five things that bite people` — the five gotchas from the spec, each 1–2 lines: (a) `-c` is total KV split across `-np` slots; (b) `--jinja` default-on & required for tool calling; (c) `/health` = 200/503 only, slot state at `/slots` (post PR #9056); (d) `--host 0.0.0.0` for non-localhost/Docker; (e) `-ngl` vs VRAM is the main OOM/speed lever.
6. `## 4. Where to read next` — decision table:

   | Task | Read |
   |------|------|
   | Install / launch / env vars / API keys / web UI | `references/launching.md` |
   | Look up a CLI flag | `references/cli-reference.md` |
   | Call an HTTP endpoint | `references/http-api.md` |
   | Force JSON / a grammar / tool & function calling | `references/structured-output-and-tools.md` |
   | Generate embeddings or rerank | `references/embeddings-and-reranking.md` |
   | Vision / audio input | `references/multimodal.md` |
   | Speculative decoding, LoRA, slot save/restore, multi-model router | `references/advanced-inference.md` |
   | Throughput, concurrency, KV-cache & context math | `references/performance-tuning.md` |
   | Use it from Rust (HTTP client or native bindings) | `references/rust-integration.md` |
   | Something is broken | `references/troubleshooting.md` |

7. `## 5. Verify your work` — `curl -s localhost:8080/health` (expect `{"status":"ok"}`), `curl -s localhost:8080/props | head`, and a real `/v1/chat/completions` round-trip returns a `choices[0].message.content`.

- [ ] **Step 3: Verify** — frontmatter parses (`head -1` = `---`; `name:`/`description:` present); Global-Constraints grep returns nothing; all 10 reference paths in the decision table are spelled consistently with the File Structure.

Run:
```bash
head -1 .agents/skills/llama-server/SKILL.md
grep -rIEli 'rust-agent-runtime|agent-server\.json|graphify|cloudflare|qwen3\.6|/home/kalen' .agents/skills/llama-server/SKILL.md || echo "CLEAN"
```
Expected: `---` then `CLEAN`.

- [ ] **Step 4: Commit**

```bash
git add .agents/skills/llama-server/SKILL.md
git commit -m "feat(skill): add llama-server hub SKILL.md"
```

---

### Task 2: `references/launching.md`

**Model:** sonnet (mechanical reference).

**Files:** Create `.agents/skills/llama-server/references/launching.md`

- [ ] **Step 1: Write the file** covering:
  - **Obtaining the binary** (expanded from hub §1): package managers; prebuilt release binaries per-OS; build-from-source CMake invocation incl. GPU backends; Docker GHCR tags (`full`/`light`/`server`, GPU suffixes), note `--gpus all` + `-ngl` and `-v` model volume mount.
  - **Minimal launch** and **common launch recipes** (CPU-only, full-GPU, multi-GPU `-ts`/`-sm`, HF auto-download `-hf`, local `-m`).
  - **Networking:** `--host` (default `127.0.0.1`; use `0.0.0.0` for external/Docker), `--port` (default `8080`), `--api-prefix`, `--timeout`.
  - **Auth & TLS:** `--api-key`, `--api-key-file`; `--ssl-key-file`/`--ssl-cert-file` (note build flag).
  - **Web UI:** on by default; disable with `--no-webui`; note `--webui`→`--ui` rename (#18155).
  - **Env-var configuration:** `LLAMA_ARG_*` table (`LLAMA_ARG_MODEL`, `_CTX_SIZE`, `_N_GPU_LAYERS`, `_N_PARALLEL`, `_HOST`, `_PORT`, `LLAMA_API_KEY`, `HF_TOKEN`); CLI overrides env; boolean forms; `--offline`.
- [ ] **Step 2: Verify** — Global-Constraints grep returns nothing; flags spot-checked against README.
Run: `grep -rIEli 'rust-agent-runtime|agent-server\.json|graphify|cloudflare|qwen3\.6|/home/kalen' .agents/skills/llama-server/references/launching.md || echo CLEAN`
Expected: `CLEAN`.
- [ ] **Step 3: Commit** — `git add … && git commit -m "feat(skill): launching.md reference"`

---

### Task 3: `references/cli-reference.md`

**Model:** sonnet.

**Files:** Create `.agents/skills/llama-server/references/cli-reference.md`

- [ ] **Step 1: Write the file** — flags grouped by category with short/long forms + one-line purpose + default (source: captured README reference). Categories & must-include flags:
  - **Model loading:** `-m/--model`, `-mu/--model-url`, `-hf/--hf-repo` (`:quant`, default Q4_K_M), `-hff/--hf-file`, `-dr/--docker-repo`.
  - **Context & cache:** `-c/--ctx-size` (0=from model; TOTAL split across slots), `-n/--predict`, `-b/--batch-size` (2048), `-ub/--ubatch-size` (512), `--keep`, `-ctk/--cache-type-k`, `-ctv/--cache-type-v`, `--mlock`, `--no-mmap`.
  - **GPU/devices:** `-ngl/--gpu-layers` (auto/all), `-dev/--device`, `--list-devices`, `-sm/--split-mode`, `-ts/--tensor-split`, `-mg/--main-gpu`, `-fit/--fit`.
  - **Threads:** `-t/--threads`, `-tb/--threads-batch`, `--threads-http`, `--prio`, `--poll`.
  - **Slots/parallel:** `-np/--parallel` (-1 auto), `-cb/--cont-batching` (on), `--no-warmup`, `--slot-save-path`.
  - **Server/network:** `--host`, `--port`, `--api-prefix`, `-to/--timeout`, `--ssl-*`.
  - **Sampling:** `--temp` (0.8), `--top-k` (40), `--top-p` (0.95), `--min-p` (0.05), `--repeat-penalty`, `--presence-penalty`, `--frequency-penalty`, `--mirostat`, `--grammar`, `-j/--json-schema`, `-s/--seed`, `--samplers`.
  - **Chat template:** `--jinja`/`--no-jinja` (default on), `--chat-template`, `--chat-template-file`, `--chat-template-kwargs`.
  - Pointers: multimodal/spec-decode/lora/embeddings flags → their dedicated references.
  - Note: `llama-server --help` is the authoritative, build-specific list; flags are version-dependent.
- [ ] **Step 2: Verify** — grep CLEAN; flag names match README (no invented flags).
- [ ] **Step 3: Commit** — `git commit -m "feat(skill): cli-reference.md"`

---

### Task 4: `references/http-api.md`

**Model:** sonnet.

**Files:** Create `.agents/skills/llama-server/references/http-api.md`

- [ ] **Step 1: Write the file** — every endpoint with method, path, purpose, key request/response fields, and a `curl` example for the common ones. Group:
  - **Health/status:** `GET /health` (200 `{"status":"ok"}` / 503 loading — only these post #9056), `GET /props`, `POST /props` (needs `--props`), `GET /v1/models`.
  - **Completion:** `POST /completion` (native: `prompt` string/array/mixed, `n_predict`, `stream`, `stop`, `grammar`, `json_schema`; response `content`/`tokens`/`timings`/`stop_type`), `POST /v1/completions` (OpenAI), `POST /v1/chat/completions` (OpenAI: `messages`, `stream`, `response_format`, `tools`, multimodal content; response `choices`/`usage`/`timings`).
  - **Tokenize:** `POST /tokenize` (`add_special`, `with_pieces`), `POST /detokenize`, `POST /apply-template`.
  - **Slots:** `GET /slots` (`--no-slots` to disable; `?fail_on_no_slot=1`), `POST /slots/{id}?action=save|restore|erase`.
  - **Monitoring:** `GET /metrics` (needs `--metrics`).
  - Pointers: embeddings/rerank → `embeddings-and-reranking.md`; `/infill` → `advanced-inference.md`; LoRA endpoints → `advanced-inference.md`.
  - **Streaming note:** SSE — responses are `data: {json}` lines terminated by `data: [DONE]`.
- [ ] **Step 2: Verify** — grep CLEAN; endpoint paths match README.
- [ ] **Step 3: Commit** — `git commit -m "feat(skill): http-api.md"`

---

### Task 5: `references/structured-output-and-tools.md`

**Model:** opus (judgment-heavy; correctness matters).

**Files:** Create `.agents/skills/llama-server/references/structured-output-and-tools.md`

- [ ] **Step 1: Write the file** covering:
  - **GBNF grammars:** `--grammar`/`grammar` field; short example grammar; when to use vs JSON schema.
  - **JSON-schema-constrained output:** `-j/--json-schema` flag and `json_schema` request field; OpenAI `response_format: {"type":"json_schema","json_schema":{…}}` and `{"type":"json_object"}`; **caveat: JSON-Schema→GBNF supports a subset** (note unsupported keywords degrade/ignore).
  - **Tool / function calling:** requires `--jinja` (default-on; explicitly enable on older builds); request `tools` + `tool_choice`; response `tool_calls`; native template handlers for Llama 3.1/3.2/3.3, Functionary, Hermes 2/3, Qwen 2.5, Mistral Nemo, Firefunction v2, Command R7B, DeepSeek R1, else a Generic handler; mismatch → wrong/no tool calls (point to troubleshooting).
  - **Reasoning/thinking:** `-rea/--reasoning [on|off|auto]`, `--reasoning-format`, `--reasoning-budget`; `reasoning_content` in responses; `/v1/chat/completions/control` to end reasoning early.
  - Worked `curl` example: a chat completion with a `tools` array returning a `tool_calls` response.
- [ ] **Step 2: Verify** — grep CLEAN; `--jinja` requirement and subset caveat both present.
Run: `grep -l 'subset' .agents/skills/llama-server/references/structured-output-and-tools.md && grep -l 'jinja' .agents/skills/llama-server/references/structured-output-and-tools.md`
Expected: file path printed twice.
- [ ] **Step 3: Commit** — `git commit -m "feat(skill): structured-output-and-tools.md"`

---

### Task 6: `references/embeddings-and-reranking.md`

**Model:** sonnet.

**Files:** Create `.agents/skills/llama-server/references/embeddings-and-reranking.md`

- [ ] **Step 1: Write the file** covering:
  - **Embeddings server:** launch with `--embedding` (+ `--pooling {none,mean,cls,last,rank}`, default model-dependent; `--embd-normalize`). Endpoints: `POST /v1/embeddings` (OpenAI, needs pooling≠none), native `POST /embedding` and `POST /embeddings` (supports `none` pooling → per-token). `curl` example + response shape.
  - **Reranking:** launch with `--reranking` (+ `--pooling rank`) and a rerank-capable model. `POST /v1/rerank` (aliases `/rerank`, `/reranking`): `{query, documents, top_n}` → ranked results. `curl` example.
  - Note: a server started `--embedding`/`--reranking` is restricted to that use case (can't also chat).
- [ ] **Step 2: Verify** — grep CLEAN.
- [ ] **Step 3: Commit** — `git commit -m "feat(skill): embeddings-and-reranking.md"`

---

### Task 7: `references/multimodal.md`

**Model:** sonnet.

**Files:** Create `.agents/skills/llama-server/references/multimodal.md`

- [ ] **Step 1: Write the file** covering:
  - **Enabling vision/audio:** `--mmproj <projector.gguf>` (or `-hf` auto-downloads a matching projector; `--no-mmproj-auto` to disable; `--mmproj-offload`). `--image-min-tokens`/`--image-max-tokens`.
  - **Sending images/audio:** OpenAI `messages` content array with `{"type":"image_url","image_url":{"url":"data:image/png;base64,…"}}` and `input_audio`; note marker-count must match data count in native `/completion`. `--media-path` for local files.
  - Worked `curl` with a base64 image.
  - Note projector must match the model; mismatch → garbage (point to troubleshooting).
- [ ] **Step 2: Verify** — grep CLEAN.
- [ ] **Step 3: Commit** — `git commit -m "feat(skill): multimodal.md"`

---

### Task 8: `references/advanced-inference.md`

**Model:** sonnet.

**Files:** Create `.agents/skills/llama-server/references/advanced-inference.md`

- [ ] **Step 1: Write the file** covering:
  - **Speculative decoding:** `-md/--spec-draft-model` (or `-hfd`), `--spec-draft-n-max`/`-n-min`; draft model must share vocab family; gains are workload-dependent.
  - **LoRA:** `--lora`, `--lora-scaled FILE:SCALE`, `--lora-init-without-apply`; hot-swap scales via `GET/POST /lora-adapters`; note different LoRA configs prevent request batching.
  - **Slot save/restore:** `--slot-save-path`; `POST /slots/{id}?action=save|restore|erase` with `filename`; use to persist/resume KV/prompt cache.
  - **Code infill:** `POST /infill` (`input_prefix`, `input_suffix`, `input_extra`); FIM tokens vs SPM (`--spm-infill`).
  - **Router / multi-model mode:** launch without `-m` using `--models-dir`/`--models-preset`; route by `model` field (body) or `?model=` (query); `POST /models/load|unload`, `GET /models`, `/models/sse`.
- [ ] **Step 2: Verify** — grep CLEAN.
- [ ] **Step 3: Commit** — `git commit -m "feat(skill): advanced-inference.md"`

---

### Task 9: `references/performance-tuning.md`

**Model:** opus (math + judgment).

**Files:** Create `.agents/skills/llama-server/references/performance-tuning.md`

- [ ] **Step 1: Write the file** covering:
  - **Concurrency/context math (lead with this):** `-c` is TOTAL KV divided across `-np` slots → worked example (`-c 16384 -np 4` ≈ 4096/slot); size `-c` for `slots × expected_context`.
  - **Continuous batching:** on by default (`-cb`); throughput vs latency.
  - **GPU offload tuning:** `-ngl` raise until VRAM-bound; partial offload tradeoffs; `-fit` auto-fit.
  - **KV-cache quantization:** `-ctk`/`-ctv` (e.g. `q8_0`) to fit larger context; quality/speed note.
  - **Batch sizes:** `-b`/`-ub` and their effect on prompt-processing throughput.
  - **Prompt/KV-cache reuse:** on by default; **nondeterminism caveat** from batch-size variation; slot-prompt-similarity reuse.
  - **Threads:** `-t`/`-tb`/`--threads-http` for CPU/hybrid.
  - A short "diagnose slow first token vs slow generation" decision blurb → troubleshooting.
- [ ] **Step 2: Verify** — grep CLEAN; the `-c`/`-np` division example is present.
Run: `grep -n 'np 4' .agents/skills/llama-server/references/performance-tuning.md || echo MISSING`
Expected: a matching line (not `MISSING`).
- [ ] **Step 3: Commit** — `git commit -m "feat(skill): performance-tuning.md"`

---

### Task 10: `references/rust-integration.md`

**Model:** opus (synthesizes the Rust research; correctness-sensitive).

**Files:** Create `.agents/skills/llama-server/references/rust-integration.md`

**Interfaces:**
- Consumes: the background Rust-integration research report (crate names, current versions, maintenance status, snippets, URLs). If unavailable at execution time, re-run a focused `WebSearch`/`WebFetch` pass over crates.io/docs.rs before writing — do NOT invent crate APIs.

- [ ] **Step 1: Write the file** with two clearly separated parts. Every crate must be explicitly labeled **(A) native FFI binding** / **(B) HTTP client for an OpenAI-compatible server** / **(C) own pure-Rust engine**.
  - **Part A — Talk to a running llama-server over HTTP (recommended default):**
    - `async-openai`: construct an `OpenAIConfig` with `.with_api_base("http://localhost:8080/v1")` and a dummy/real api key; non-streaming and streaming chat-completion snippets; gotchas with llama-server's partial OpenAI compatibility.
    - Brief mention of other OpenAI-compatible clients found in research (e.g. `openai-dive`, `openai-api-rs`) with status.
    - Plain `reqwest` + `serde_json`: `POST /v1/chat/completions`, and SSE streaming parsing of `data:` lines + the `[DONE]` sentinel. (Keep generic — no repo-specific client code.)
  - **Part B — Native llama.cpp bindings (no server, in-process):**
    - `llama-cpp-2` (utilityai/llama-cpp-rs) — the most-maintained FFI binding: backend init → model load → context → batch decode → sampler → token-to-string minimal pattern; feature flags (`cuda`/`metal`/`vulkan`); build requirements (C/C++ toolchain, links libllama).
    - Note other crates with honest status from research (`llama_cpp`/binary-banter, `drama_llama`, rustformers `llm` [deprecated], `mistral.rs` [own engine, label C], `kalosm`).
    - Higher-level frameworks that can point at an OpenAI base URL (e.g. `rig`, `langchain-rust`) — label B, note base-URL support.
  - **Choosing:** one short paragraph — HTTP client (Part A) for most apps (process isolation, hot-swap models, language-agnostic); native bindings (Part B) when you need single-process/no-network/embedded.
  - Flag any crate the research could not verify or that looked stale.
- [ ] **Step 2: Verify** — grep CLEAN; both Part A and Part B headings present; each named crate carries an (A)/(B)/(C) label.
- [ ] **Step 3: Commit** — `git commit -m "feat(skill): rust-integration.md"`

---

### Task 11: `references/troubleshooting.md`

**Model:** opus (diagnosis quality).

**Files:** Create `.agents/skills/llama-server/references/troubleshooting.md`

- [ ] **Step 1: Write the file** as a symptom → cause → fix table/list:
  - **Port already in use** → another server / stale process → change `--port` or kill it.
  - **Can't reach server from another host/Docker** → bound to `127.0.0.1` → `--host 0.0.0.0` (+ publish the port).
  - **OOM / CUDA out of memory at load** → too many `-ngl` layers or `-c` too large → lower `-ngl`, lower `-c`, quantize KV (`-ctk q8_0`), smaller model.
  - **"Context overflow" / truncated output** → prompt+gen exceeds per-slot context → raise `-c` (remember /`-np`), lower `-np`, enable context shift, or trim prompt.
  - **Slow first token (high prompt-eval)** → big prompt on CPU / cold cache → more `-ngl`, raise `-b`/`-ub`, enable prompt caching/slot reuse.
  - **Tool calls ignored / wrong format** → `--jinja` off or wrong template for the model → enable `--jinja`, set `--chat-template`, verify with `POST /apply-template`.
  - **503 from `/health`** → model still loading → poll until 200 (note: `/health` no longer reports slot saturation — use `/slots` for that, post #9056).
  - **`-hf` download / cache issues** → HF cache migration (#21364) / missing `HF_TOKEN` → set token, clear cache, or use `-m` with a local file.
  - **Garbled multimodal output** → wrong/missing `--mmproj` projector → use the matching projector.
  - **General triage tools:** `/props` (effective settings), `/slots` (live state), `--verbose`, `/metrics`.
- [ ] **Step 2: Verify** — grep CLEAN; `/health` 200/503 + `/slots` distinction present.
- [ ] **Step 3: Commit** — `git commit -m "feat(skill): troubleshooting.md"`

---

### Task 12: Whole-skill consistency & repo-agnosticism review

**Model:** opus (final adversarial review).

**Files:** Read-only review of all files under `.agents/skills/llama-server/`; fix any issues found.

- [ ] **Step 1: Repo-agnostic sweep** (whole skill):

Run:
```bash
grep -rIEli 'rust-agent-runtime|agent-server\.json|graphify|cloudflare|qwen3\.6|/home/kalen|worktree' .agents/skills/llama-server || echo "ALL CLEAN"
```
Expected: `ALL CLEAN`. Fix any hit.

- [ ] **Step 2: Link integrity** — every `references/<x>.md` named in `SKILL.md` exists:

Run:
```bash
cd .agents/skills/llama-server
for f in $(grep -oE 'references/[a-z-]+\.md' SKILL.md | sort -u); do [ -f "$f" ] && echo "OK $f" || echo "MISSING $f"; done
```
Expected: 10× `OK`, no `MISSING`.

- [ ] **Step 3: Cross-file consistency** — read all files; confirm flag names, defaults (port 8080, host 127.0.0.1, `-c` divide-by-`-np`, `--jinja` on, Q4_K_M), and endpoint paths are identical everywhere; confirm the six version-dependent caveats appear where relevant. Fix discrepancies inline.

- [ ] **Step 4: Fact-trace spot check** — pick 10 flags and 8 endpoints at random; confirm each appears in the canonical README ground truth (no invented surface). Fix or remove anything unverifiable.

- [ ] **Step 5: Commit** any fixes:

```bash
git add .agents/skills/llama-server
git commit -m "chore(skill): consistency + repo-agnosticism review pass"
```

---

## Self-Review (plan vs spec)

- **Spec coverage:** hub (Task 1); all 10 references map 1:1 to spec's reference table (Tasks 2–11, incl. the added `rust-integration.md`); final review enforces success criteria (Task 12). ✓
- **Version-dependent items:** the six flagged items are assigned to specific files (jinja→Task5, health/slots→Tasks 4/11, webui→Task 2, ngl→Tasks 1/9, JSON-schema subset→Task 5, HF-cache→Task 11). ✓
- **Placeholder scan:** each task lists concrete sections + exact flags/endpoints/commands; no "TBD"/"add error handling". ✓
- **Naming consistency:** the 10 reference filenames are identical across File Structure, the Task headers, and the Task 1 decision table. ✓
- **Model-per-step:** every task names opus vs sonnet per the Global Constraint. ✓
- **Risk:** Task 10 depends on external research; mitigation (re-run focused web research) is written into the task. ✓
