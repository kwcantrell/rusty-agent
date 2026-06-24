# Advanced inference

Speculative decoding, LoRA adapters, slot save/restore, code infill, and
multi-model router mode.

## Speculative decoding (draft model)

A small **draft** model proposes tokens that the main model verifies in batch —
faster generation when the draft agrees often.

```bash
llama-server -m main-model.gguf -ngl 99 \
  -md draft-model.gguf \
  --spec-draft-n-max 5
```

- `-md, --spec-draft-model FNAME` (or `-hfd` for a HF draft repo).
- `--spec-draft-n-max N` — tokens to draft per step (default `3`).
- `--spec-draft-n-min N` — minimum draft tokens.

The draft model should share the main model's vocabulary/family. Speedups are
workload-dependent (great for predictable text, marginal for high-entropy
output); measure before committing.

## LoRA adapters

Apply one or more LoRA adapters on top of the base model, optionally scaled:

```bash
llama-server -m base.gguf \
  --lora adapter-a.gguf \
  --lora-scaled adapter-b.gguf:0.5
```

- `--lora FNAME` — apply an adapter at full strength.
- `--lora-scaled FNAME:SCALE` — apply with a scale factor.
- `--lora-init-without-apply` — load adapters but start with them disabled.

**Hot-swap at runtime** without restarting:
- `GET /lora-adapters` → list loaded adapters with current scales.
- `POST /lora-adapters` with `[{"id":0,"scale":1.0},{"id":1,"scale":0.0}]` →
  change scales live.

> Requests using different LoRA scale configurations **cannot be batched
> together**, which lowers throughput — keep scales uniform when serving many
> concurrent users.

## Slot save / restore (prompt-cache persistence)

Persist a slot's prompt + KV cache to disk and reload it later — skip
reprocessing a long shared prefix (system prompt, document) across restarts.

Start with a save directory, then drive via the API:

```bash
llama-server -m model.gguf --slot-save-path ./slot-cache
```

```bash
curl -X POST "localhost:8080/slots/0?action=save"    -d '{"filename":"session1.bin"}'
curl -X POST "localhost:8080/slots/0?action=restore" -d '{"filename":"session1.bin"}'
curl -X POST "localhost:8080/slots/0?action=erase"
```

## Code infill (FIM)

`POST /infill` does fill-in-the-middle for code completion:

```bash
curl http://localhost:8080/infill -H 'Content-Type: application/json' -d '{
  "input_prefix": "def add(a, b):\n    return ",
  "input_suffix": "\n\nprint(add(2,3))",
  "input_extra": [{"filename":"util.py","text":"# helpers"}]
}'
```

It uses the model's FIM tokens when present; `--spm-infill` switches to the
suffix/prefix/middle ordering for models that expect it.

## Router / multi-model mode

Serve several models from one process and route per request. Launch **without**
`-m`, pointing at a directory or preset file:

```bash
llama-server --models-dir ./models --models-max 4
```

- `--models-dir DIR` — directory of GGUF models to serve.
- `--models-preset FILE` — INI of named model presets.
- `--models-max N` — max simultaneously-loaded models (default `4`; `0` = no limit).

Route by naming the model: the `model` field in the JSON body (POST endpoints) or
`?model=...` (GET endpoints). Manage models live:

| Endpoint | Purpose |
|----------|---------|
| `GET /models` | List available/loaded models (`?reload=1` to refresh). |
| `POST /models/load` `{"model":"..."}` | Load a model. |
| `POST /models/unload` `{"model":"..."}` | Unload a model. |
| `GET /models/sse` | SSE stream of model status/download events. |

Pair with `--sleep-idle-seconds N` to auto-unload idle models and free memory.
