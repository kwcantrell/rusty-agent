# Troubleshooting

Symptom → likely cause → fix. Triage tools at the bottom.

## Server won't start / unreachable

**`bind: address already in use` / port busy**
- Cause: another process (or a stale `llama-server`) holds the port.
- Fix: pick another `--port`, or find and kill the holder
  (`lsof -i :8080` / `ss -ltnp | grep 8080`).

**Can't connect from another machine or from Docker host**
- Cause: server bound to the default `127.0.0.1` (localhost only).
- Fix: start with `--host 0.0.0.0`. For Docker, also publish the port
  (`-p 8080:8080`) and bind `0.0.0.0` *inside* the container.

**`401`/`Unauthorized`**
- Cause: `--api-key` is set and the client didn't send a matching bearer token.
- Fix: send `Authorization: Bearer <key>`, or unset `--api-key`.

## Memory / loading failures

**`CUDA out of memory` / `failed to allocate` at load**
- Cause: too many GPU layers (`-ngl`) and/or too large a context (`-c`) for VRAM.
- Fix, in order: lower `-ngl`; lower `-c` (remember it's divided by `-np`);
  quantize the KV cache (`-ctk q8_0 -ctv q8_0`); use a smaller / more-quantized
  model. Use `--fit on` (default) to auto-trim.

**Loads but host RAM thrashes / very slow**
- Cause: model doesn't fit in RAM and is paging.
- Fix: smaller quant, offload more to GPU (`-ngl`), or add `--mlock` only if it
  genuinely fits.

## Generation problems

**Output cut off / "context overflow" / truncated**
- Cause: prompt + generation exceeds the **per-slot** context (`ctx / n_parallel`).
- Fix: raise `-c`, lower `-np`, raise `--keep`, or shorten the prompt. See the
  context math in `performance-tuning.md`.

**Slow first token (then fast)**
- Cause: prompt processing bottleneck (long prompt, cold cache, CPU-bound).
- Fix: more `-ngl`, larger `-b`/`-ub`, rely on prompt-cache reuse for shared
  prefixes.

**Slow every token**
- Cause: generation bottleneck.
- Fix: more `-ngl`, smaller model, or speculative decoding (`advanced-inference.md`).

**Nondeterministic results across identical requests**
- Cause: continuous batching / prompt caching changes batch numerics.
- Fix: set a fixed `seed`, reduce `-np`, disable prompt caching for that request.

## Tool calling & templates

**Tools ignored / `finish_reason:"stop"` with `tools` set, or malformed tool calls**
- Cause: `--jinja` not enabled, or the wrong/missing chat template for the model.
- Fix: start with `--jinja`; set `--chat-template` (or `--chat-template-file`) to
  match the model family; verify the rendered prompt with `POST /apply-template`.
  See `structured-output-and-tools.md`.

**Tool-call `arguments` fails to deserialize (strict clients)**
- Cause: older builds emitted `arguments` as a JSON object, not a string
  (llama.cpp #20198, fixed by PR #20213).
- Fix: update the server, or parse defensively.

**JSON schema not fully respected**
- Cause: JSON-Schema → GBNF supports only a subset; advanced keywords are dropped.
- Fix: simplify the schema; validate output client-side. See
  `structured-output-and-tools.md`.

## Health / readiness

**`/health` returns `503`**
- Cause: the model is still loading — this is normal at startup.
- Fix: poll `/health` until `200`. Note `/health` reports **only** load state
  (since PR #9056); for slot/queue saturation read `/slots` (and
  `?fail_on_no_slot=1` to make a busy server return `503` instead of queuing).

## Model download (`-hf`)

**`-hf` download fails / model not found / cache weirdness**
- Cause: missing `HF_TOKEN` for gated repos, network, or HF-cache layout changes
  (llama.cpp #21364).
- Fix: set `HF_TOKEN`; clear the HF cache; or download the GGUF manually and use
  `-m ./model.gguf`. Use `--offline` to force cache-only and fail fast.

## Multimodal

**Garbled / nonsensical image or audio output**
- Cause: wrong or missing `--mmproj` projector for the model.
- Fix: use the projector that matches the model (or let `-hf` auto-download it);
  confirm modalities via `GET /props`. See `multimodal.md`.

## Triage tools

- `GET /props` — what actually loaded (model, chat template, slot count, settings).
- `GET /slots` — live per-slot state; spot saturation/queuing.
- `POST /apply-template` — see the exact prompt the chat template produces.
- `-v` / `--verbose` (or `-lv N`) — verbose server logs.
- `GET /metrics` (with `--metrics`) — throughput and KV-cache usage over time.
