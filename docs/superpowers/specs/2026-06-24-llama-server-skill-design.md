# Design: `llama-server` agent skill

**Date:** 2026-06-24
**Status:** Approved (brainstorming complete) → implementation plan next

## Goal

Author a self-contained, **repo-agnostic** agent skill that teaches any agent (in
any repository) to build/obtain, launch, operate, call, and troubleshoot
**`llama-server`** — the OpenAI-compatible HTTP server shipped by
[`ggml-org/llama.cpp`](https://github.com/ggml-org/llama.cpp) under `tools/server`.

The skill MUST NOT reference anything specific to this repository. All examples
are generic (`model.gguf`, `localhost:8080`, plain `curl`) and usable verbatim
elsewhere.

## Format & convention

Mirror the existing `.agents/skills/tauri/` skill exactly:

- One **hub** `SKILL.md` with YAML frontmatter (`name`, trigger-rich
  `description`), numbered sections, a decision table, and a "Verify your work"
  section.
- A `references/` folder of focused, copy-pasteable topic files. The hub stays
  short and high-signal; depth lives in references, pulled in only as needed.

## Directory layout

```
.agents/skills/llama-server/
├─ SKILL.md
└─ references/
   ├─ launching.md
   ├─ cli-reference.md
   ├─ http-api.md
   ├─ structured-output-and-tools.md
   ├─ embeddings-and-reranking.md
   ├─ multimodal.md
   ├─ advanced-inference.md
   ├─ performance-tuning.md
   ├─ rust-integration.md
   └─ troubleshooting.md
```

## SKILL.md (hub) contents

YAML frontmatter:
- `name: llama-server`
- `description`: trigger-rich — fires on mentions of llama-server, llama.cpp
  server, serving a GGUF model over HTTP, OpenAI-compatible local endpoint,
  `-m`/`-hf`/`-ngl`/`-c`/`-np`, `/v1/chat/completions`, `--jinja`, port 8080,
  "run a local LLM server".

Sections:
1. **What it is / when to use** — and when not (not training; not `llama-cli`,
   `llama-bench`; not a drop-in for Ollama/vLLM though it overlaps).
2. **Get the binary** — package managers, prebuilt release binaries, build from
   source (CMake `./build/bin/llama-server`), Docker GHCR tags
   (`full`/`light`/`server` + `-cuda`/`-rocm`/`-vulkan`/`-intel`/`-musa`).
3. **30-second quickstart** — `llama-server -m model.gguf -c 4096 -ngl 99`, then
   a `curl` to `/v1/chat/completions`.
4. **The 5 concepts that bite people** (promoted to the hub — highest-value
   gotchas):
   - `-c` is the **total** KV context **divided across `-np` slots**
     (`-c 16384 -np 4` ≈ 4096 tokens/slot).
   - `--jinja` is **default-on** and **required** for OpenAI-style tool/function
     calling.
   - `/health` returns only 200/503 (post PR #9056); slot/processing state lives
     at `/slots`.
   - `--host 0.0.0.0` is required for non-localhost / Docker access (default is
     `127.0.0.1`).
   - `-ngl` (GPU layers) vs available VRAM — the main OOM / speed lever.
5. **Decision table** → which reference to read for which task.
6. **Verify your work** — `curl /health`, `/props`, and a real completion.

## References (each focused, version-caveated)

| File | Covers |
|------|--------|
| `launching.md` | Every way to obtain + launch; env vars (`LLAMA_ARG_*`), API keys (`--api-key`/file), SSL, web UI (`--ui`, deprecated `--webui`), offline mode |
| `cli-reference.md` | Flags grouped by category: model loading, context/cache, GPU/devices, threads, slots/parallel, server/network, sampling, chat template/jinja |
| `http-api.md` | Every endpoint — method, path, purpose, key request/response fields; native `/completion` vs OpenAI `/v1/*`; `/props`, `/health`, `/slots`, `/tokenize`, `/detokenize`, `/apply-template`, `/metrics` |
| `structured-output-and-tools.md` | GBNF grammars, `json_schema`/`response_format` (JSON-Schema→GBNF **subset** caveat), `--jinja` tool/function calling, supported model families, reasoning/thinking flags |
| `embeddings-and-reranking.md` | `--embedding`, `--pooling`, `/v1/embeddings` + native `/embedding`/`/embeddings`, `--rerank` + `/v1/rerank` |
| `multimodal.md` | `--mmproj`, auto-download via `-hf`, image/audio input message shapes |
| `advanced-inference.md` | Speculative decoding (draft model), LoRA + hot-swap via `/lora-adapters`, slot save/restore, router/multi-model mode |
| `performance-tuning.md` | Slots/context math, continuous batching, KV-cache types (`-ctk`/`-ctv`), prompt caching (+ nondeterminism note), `-ngl`/batch/ubatch tuning |
| `rust-integration.md` | **(added)** Two paths: (A) calling llama-server's OpenAI-compatible HTTP API from Rust — `async-openai` with a custom base URL, plus raw `reqwest`+`serde_json` with SSE `data:`/`[DONE]` parsing; (B) **native llama.cpp Rust bindings** (`llama-cpp-2` et al.) as the no-server alternative, with build/feature-flag notes. Clearly labels each crate as FFI binding / HTTP client / own engine. |
| `troubleshooting.md` | Port conflicts, OOM, context overflow/shift, slow first token, chat-template mismatches, Docker networking, health/readiness checks |

## Ground-truth sources

- Canonical README (`tools/server/README.md`) — fetched and captured during
  research; primary authority for flags, endpoints, defaults.
- Deep-research verified facts (corroborated against `common/arg.cpp`,
  `docs/function-calling.md`, `docs/docker.md`, manpage, PR #9056, issue #18155).
- Rust-integration facts — separate focused research pass (crates.io / docs.rs /
  crate READMEs).

**Version-dependent items to flag in-text** (research surfaced these as
recently changed): HF-cache migration (issue #21364); `--jinja` default-on (was
off in older builds); `-ngl` `auto`/`all` is recent; `/health` vs `/slots` split
(PR #9056) is breaking; `--webui` deprecated for `--ui` (issue #18155);
JSON-Schema→GBNF is a subset.

## Build approach

- Fan out the **reference files** to parallel subagents (per-file writing is
  independent), each handed the verified research facts + the canonical README
  excerpt as ground truth, plus a strict "no repo references, generic examples
  only" constraint and the house style (focused, copy-pasteable, version-caveated).
- Write the **hub `SKILL.md`** directly (it's the synthesis/navigation layer).
- Final consistency + repo-agnosticism review pass over the whole skill.
- **Model selection per step:** stronger model for the hub synthesis and the
  gotcha-heavy files (structured-output, performance, troubleshooting,
  rust-integration); standard model for the mechanical reference tables
  (cli-reference, http-api, launching).

## Out of scope (YAGNI)

- Training / fine-tuning.
- Non-server llama.cpp tools (`llama-cli`, `llama-bench`, `llama-quantize`).
- Language-specific client SDKs beyond Rust (we show raw `curl` + note generic
  OpenAI-SDK compatibility; Rust is covered because it was explicitly requested).

## Success criteria

- `.agents/skills/llama-server/SKILL.md` + 10 reference files exist and are
  internally consistent.
- No string in the skill references this repo, its paths, or its crates.
- Every flag/endpoint/claim traces to the canonical README or verified research;
  version-dependent items are explicitly flagged.
- An agent with zero prior llama-server knowledge could go from nothing to a
  working served model + a successful chat/structured/tool call using only the
  skill.
