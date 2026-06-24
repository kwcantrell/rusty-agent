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

# Serving local LLMs with llama-server

`llama-server` is the HTTP inference server bundled with **llama.cpp**
(`ggml-org/llama.cpp`, in `tools/server`). It loads a **GGUF** model and exposes
an **OpenAI-compatible** REST API plus a native API and a built-in web UI. It is
a single self-contained binary — no Python, no runtime — and runs CPU-only or
GPU-accelerated (CUDA / Metal / ROCm / Vulkan / SYCL).

This file is the hub. Depth lives in `references/` — pull in only the file the
current task needs (see the decision table in §4).

## When to use this skill

- Use when serving a GGUF model over HTTP locally or on a server: the user wants
  a local OpenAI-compatible endpoint, mentions `llama-server`, or is wiring an
  app/agent to `http://localhost:8080/v1`.
- **Do not** use this skill for: training/fine-tuning; the other llama.cpp tools
  (`llama-cli`, `llama-bench`, `llama-quantize`); or as documentation for Ollama,
  vLLM, or TGI — they overlap conceptually but have different flags and APIs.

## 1. Get the binary

Pick whichever is easiest for the platform — all yield the same `llama-server`:

- **Package manager:** `brew install llama.cpp` (macOS/Linux), `winget install
  llama.cpp` (Windows), Nix, etc.
- **Prebuilt release:** download the asset for your OS/arch from the project's
  GitHub Releases and run the `llama-server` binary inside.
- **Build from source** (for a specific GPU backend):
  ```bash
  cmake -B build -DGGML_CUDA=ON          # or -DGGML_METAL=ON / -DGGML_VULKAN=ON
  cmake --build build --config Release -j
  ./build/bin/llama-server --help
  ```
- **Docker (GHCR):** images are `ghcr.io/ggml-org/llama.cpp:server` (and `:full`
  / `:light`), with GPU variants suffixed `-cuda` / `-rocm` / `-vulkan` /
  `-intel` / `-musa`. GPU images need `--gpus all` **and** `-ngl`; bind to
  `0.0.0.0` so the port is reachable outside the container.

Details, env-var config, API keys, TLS, and the web UI → `references/launching.md`.

## 2. 30-second quickstart

```bash
# Serve a model (auto-downloaded from Hugging Face), all layers on GPU:
llama-server -hf ggml-org/gemma-3-1b-it-GGUF -c 4096 -ngl 99
# ...or from a local file:
llama-server -m ./model.gguf -c 4096 -ngl 99
```

Then call the OpenAI-compatible endpoint:

```bash
curl http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"Say hello in one word."}]}'
```

The web UI is also live at <http://localhost:8080>.

## 3. The five things that bite people

1. **`-c` is the *total* KV context, split across slots.** With `-np N` slots,
   each request gets `ctx / N` tokens. `-c 16384 -np 4` ⇒ ~4096 tokens **per
   request**, not 16384. Size `-c` as `slots × expected_context`.
2. **`--jinja` is required for tool/function calling.** It is default-on in
   current builds (was off in older ones). Without a correct chat template the
   server returns `finish_reason: "stop"` and ignores your `tools`.
3. **`/health` only reports load state.** It returns `200 {"status":"ok"}` once
   the model is loaded, `503` while loading — nothing else (since PR #9056). For
   slot/queue saturation, read `/slots`.
4. **Default bind is `127.0.0.1`.** Nothing outside the machine (or outside a
   Docker container) can reach it until you pass `--host 0.0.0.0`.
5. **`-ngl` (GPU layers) vs VRAM is the main OOM/speed lever.** Raise it to put
   more layers on the GPU (faster) until you hit out-of-memory; lower it (or
   quantize the KV cache) when loading fails.

## 4. Where to read next

| Task | Read |
|------|------|
| Install / launch / env vars / API keys / web UI | `references/launching.md` |
| Look up a CLI flag | `references/cli-reference.md` |
| Call an HTTP endpoint | `references/http-api.md` |
| Force JSON / a grammar / tool & function calling | `references/structured-output-and-tools.md` |
| Generate embeddings or rerank documents | `references/embeddings-and-reranking.md` |
| Vision / audio (image or audio input) | `references/multimodal.md` |
| Speculative decoding, LoRA, slot save/restore, multi-model router | `references/advanced-inference.md` |
| Throughput, concurrency, KV-cache & context math | `references/performance-tuning.md` |
| Use it from Rust (HTTP client or native bindings) | `references/rust-integration.md` |
| Something is broken | `references/troubleshooting.md` |

## 5. Verify your work

```bash
curl -s localhost:8080/health           # -> {"status":"ok"}  (503 = still loading)
curl -s localhost:8080/props | head     # effective settings, chat template, slots
curl -s localhost:8080/v1/chat/completions -H 'Content-Type: application/json' \
  -d '{"messages":[{"role":"user","content":"ping"}]}'   # -> choices[0].message.content
```

If `/health` is `200`, `/props` shows the model you expect, and the chat call
returns a non-empty `choices[0].message.content`, the server is correctly up.
