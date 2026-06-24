# CLI flag reference

Flags grouped by category, with short/long forms, purpose, and default. This is
a curated subset of the most-used flags.

> **The authoritative, build-specific list is `llama-server --help`.** Flags
> change between versions — when something here disagrees with `--help` on your
> binary, trust `--help`. Items called out as "recent" / "default-on now" are
> version-dependent.

## Model loading

| Flag | Purpose |
|------|---------|
| `-m, --model FNAME` | Path to a local GGUF model. |
| `-mu, --model-url URL` | Download a GGUF from a URL. |
| `-hf, --hf-repo <user>/<repo>[:quant]` | Download from Hugging Face. No `:quant` ⇒ a default quant (commonly `Q4_K_M`). |
| `-hff, --hf-file FILE` | Pin a specific GGUF file within the HF repo. |
| `-dr, --docker-repo [<repo>/]<model>[:quant]` | Pull a model packaged on Docker Hub. |
| `--check-tensors` | Validate tensor data on load. |

## Context & cache

| Flag | Default | Purpose |
|------|---------|---------|
| `-c, --ctx-size N` | `0` (= from model) | **Total** KV context, divided across slots (see `performance-tuning.md`). |
| `-n, --predict N` | `-1` (∞) | Max tokens to generate per request. |
| `-b, --batch-size N` | `2048` | Logical max batch size (prompt processing). |
| `-ub, --ubatch-size N` | `512` | Physical max batch size. |
| `--keep N` | `0` | Tokens kept from the prompt when context overflows. |
| `-ctk, --cache-type-k TYPE` | `f16` | KV cache type for K (`f16`, `q8_0`, `q4_0`, …). |
| `-ctv, --cache-type-v TYPE` | `f16` | KV cache type for V. |
| `--mlock` | off | Lock model in RAM (no swap). |
| `--no-mmap` | mmap on | Disable memory-mapping the model file. |

## GPU & devices

| Flag | Purpose |
|------|---------|
| `-ngl, --gpu-layers N` | Layers to offload to GPU. `99`/`all` = everything; `auto` lets it choose; `0` = CPU-only. Recent builds accept `auto`/`all`. |
| `-dev, --device <d1,d2,…>` | Restrict to specific devices. |
| `--list-devices` | Print detected devices and exit. |
| `-sm, --split-mode {none,layer,row,tensor}` | How to split a model across GPUs. |
| `-ts, --tensor-split F0,F1,…` | Fraction of the model per GPU (e.g. `0.5,0.5`). |
| `-mg, --main-gpu INDEX` | Primary GPU (default `0`). |
| `-fit, --fit [on\|off]` | Auto-adjust unset args to fit device memory (default on). |

## Threads

| Flag | Purpose |
|------|---------|
| `-t, --threads N` | Generation threads (default `-1` = auto). |
| `-tb, --threads-batch N` | Threads for prompt/batch processing. |
| `--threads-http N` | HTTP server worker threads. |
| `--prio N` | Process priority: low(-1)/normal(0)/medium(1)/high(2). |
| `--poll <0..100>` | Busy-poll level while waiting for work. |

## Slots & parallelism

| Flag | Default | Purpose |
|------|---------|---------|
| `-np, --parallel N` | `-1` (auto) | Number of concurrent request slots. Context is split across these. |
| `-cb, --cont-batching` | on | Continuous batching (interleave requests). Disable: `--no-cont-batching`. |
| `--no-warmup` | warmup on | Skip the empty warmup run at startup. |
| `--slot-save-path DIR` | — | Directory for slot KV save/restore (see `advanced-inference.md`). |

## Server & network

| Flag | Default | Purpose |
|------|---------|---------|
| `--host HOST` | `127.0.0.1` | Listen address. Use `0.0.0.0` for external/Docker. |
| `--port PORT` | `8080` | Listen port. |
| `--api-prefix PREFIX` | — | Serve under a path prefix. |
| `-to, --timeout N` | `3600` | Read/write timeout (s). |
| `--api-key KEY` / `--api-key-file FILE` | — | Require bearer auth. |
| `--ssl-key-file` / `--ssl-cert-file` | — | Serve HTTPS (needs OpenSSL build). |

## Sampling

| Flag | Default | Purpose |
|------|---------|---------|
| `--temp N` | `0.80` | Temperature. |
| `--top-k N` | `40` | Top-k (0 = off). |
| `--top-p N` | `0.95` | Top-p / nucleus. |
| `--min-p N` | `0.05` | Min-p. |
| `--repeat-penalty N` | `1.00` | Repetition penalty. |
| `--presence-penalty N` / `--frequency-penalty N` | `0.00` | OpenAI-style penalties. |
| `--mirostat N` | `0` | Mirostat sampling (1 or 2 to enable). |
| `--grammar GRAMMAR` | — | Constrain output with a GBNF grammar. |
| `-j, --json-schema SCHEMA` | — | Constrain output to a JSON schema. |
| `-s, --seed N` | `-1` (random) | RNG seed. |
| `--samplers "a;b;c"` | — | Sampler order. |

(Per-request, most of these can also be set in the JSON body — see `http-api.md`.)

## Chat template & jinja

| Flag | Default | Purpose |
|------|---------|---------|
| `--jinja` / `--no-jinja` | **on** | Use the Jinja chat-template engine. **Required for tool/function calling.** (Was off in older builds.) |
| `--chat-template NAME_OR_TEMPLATE` | from model | Override the chat template. |
| `--chat-template-file FILE` | — | Load a chat template from a file. |
| `--chat-template-kwargs JSON` | — | Extra kwargs passed to the template (e.g. enable/disable thinking). |

## Pointers to specialized flags

- Multimodal (`--mmproj`, …) → `multimodal.md`
- Embeddings / reranking (`--embedding`, `--rerank`, `--pooling`) → `embeddings-and-reranking.md`
- Speculative decoding / LoRA / router (`-md`, `--lora`, `--models-dir`) → `advanced-inference.md`
- Reasoning (`-rea`, `--reasoning-format`, `--reasoning-budget`) → `structured-output-and-tools.md`
- Metrics / props / slots monitoring (`--metrics`, `--props`, `--no-slots`) → `http-api.md`
