# HTTP API reference

`llama-server` exposes two overlapping surfaces:

- **OpenAI-compatible** endpoints under `/v1/*` — drop-in for OpenAI client
  libraries.
- **Native** endpoints (`/completion`, `/embedding`, `/props`, `/slots`, …) with
  llama.cpp-specific features.

Base URL in examples: `http://localhost:8080`. If `--api-key` is set, add
`-H "Authorization: Bearer $KEY"`.

## Health & status

### `GET /health`
Load state only (since PR #9056):
- `200 {"status":"ok"}` — model loaded, ready.
- `503` — still loading.

It does **not** report slot saturation — use `/slots` for that.

### `GET /props`
Effective server properties: model metadata, default generation settings, slot
count, the active chat template, and supported modalities. Great for confirming
what actually loaded.

### `POST /props`
Change select global properties at runtime. Requires the server to be started
with `--props`.

### `GET /v1/models`
OpenAI-compatible model list. Returns a single entry describing the loaded model
(`id`, plus `n_ctx_train`, `n_embd`, `n_params`, …).

## Completion & chat

### `POST /v1/chat/completions` (OpenAI-compatible)
The main endpoint. Key request fields:
- `messages` — array of `{role, content}` (content may be a string or a
  multimodal array, see `multimodal.md`).
- `stream` — `true` for SSE token streaming.
- `temperature`, `top_p`, `max_tokens`, `stop`, `seed`, … (standard OpenAI).
- `response_format` — `{"type":"json_object"}` or `{"type":"json_schema", …}`
  (see `structured-output-and-tools.md`).
- `tools` / `tool_choice` — function calling (needs server `--jinja`).

Response: `choices[].message.content` (and/or `tool_calls`), `usage`, plus a
llama.cpp `timings` object (prompt/gen tokens and speeds).

```bash
curl http://localhost:8080/v1/chat/completions -H 'Content-Type: application/json' -d '{
  "messages":[{"role":"user","content":"Name three primary colors."}],
  "temperature":0.2, "max_tokens":64
}'
```

Streaming yields `data: {chunk}` SSE lines ending with `data: [DONE]`.

### `POST /v1/completions` (OpenAI-compatible)
Legacy text completion: `prompt` in, completion out. Same sampling options.

### `POST /completion` (native)
llama.cpp-native completion with extra power:
- `prompt` — string, **token-ID array**, or a **mixed** array like
  `[12, "text", 34]`.
- `n_predict`, `stream`, `stop`, `grammar`, `json_schema`, `cache_prompt`, …
- Response: `content`, `tokens`, `generation_settings`, `timings`, `stop_type`,
  `truncated`.

## Tokenization

### `POST /tokenize`
`{"content":"...", "add_special":true, "with_pieces":false}` → `tokens` (IDs, or
`{id,piece}` objects when `with_pieces`).

### `POST /detokenize`
`{"tokens":[...]}` → `{"content":"..."}`.

### `POST /apply-template`
Apply the chat template to `messages` **without** running inference. Returns the
rendered `prompt` string — invaluable for debugging template/tool-call issues.

## Slots

### `GET /slots`
Current processing state of every slot (enabled by default; disable with
`--no-slots`). Each entry has `is_processing`, `params`, etc. Add
`?fail_on_no_slot=1` to get `503` instead of queuing when all slots are busy.

### `POST /slots/{id}?action=save|restore|erase`
Persist / reload / clear a slot's prompt+KV cache (`save`/`restore` take
`{"filename":"..."}`; needs `--slot-save-path`). See `advanced-inference.md`.

## Monitoring

### `GET /metrics`
Prometheus-format metrics (prompt/generation token counts and throughput,
request counts, KV-cache usage). Requires `--metrics`.

## Specialized endpoints (covered elsewhere)

| Endpoint | See |
|----------|-----|
| `POST /v1/embeddings`, `POST /embedding`, `POST /embeddings` | `embeddings-and-reranking.md` |
| `POST /v1/rerank` (`/rerank`, `/reranking`) | `embeddings-and-reranking.md` |
| `POST /infill` | `advanced-inference.md` |
| `GET/POST /lora-adapters` | `advanced-inference.md` |
| `POST /models/load`, `/models/unload`, `GET /models`, `/models/sse` (router) | `advanced-inference.md` |

## Streaming (SSE) notes

When `stream:true`, the response is `text/event-stream`: each event is a
`data: {json}` line; the stream terminates with a literal `data: [DONE]`. Parse
on event boundaries (`\n\n`), not raw byte chunks — a single TCP read may contain
a partial or multiple SSE frames. (Rust example in `rust-integration.md`.)
