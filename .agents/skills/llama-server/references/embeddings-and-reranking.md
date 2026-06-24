# Embeddings & reranking

`llama-server` can run as an **embedding** server or a **reranking** server.
Either mode is dedicated: a server started with `--embedding` or `--reranking`
serves only that use case (it won't also do chat) — run a separate instance for
generation.

## Embeddings

Start with `--embedding` and an embedding-capable model:

```bash
llama-server -m embed-model.gguf --embedding --pooling mean
```

- `--embedding` / `--embeddings` — restrict the server to embeddings.
- `--pooling {none,mean,cls,last,rank}` — how token vectors are pooled into one
  vector. `mean`/`cls`/`last` are the usual choices; `none` returns per-token
  vectors; `rank` is for reranking (below).
- `--embd-normalize N` — normalization (default `2` = Euclidean/L2).

### Endpoints

| Endpoint | Notes |
|----------|-------|
| `POST /v1/embeddings` | OpenAI-compatible. Requires `--pooling` ≠ `none`. |
| `POST /embedding` | Native; single or batched `content`. |
| `POST /embeddings` | Native; supports all pooling types incl. `none` (per-token). |

```bash
curl http://localhost:8080/v1/embeddings -H 'Content-Type: application/json' -d '{
  "input": ["the quick brown fox", "lorem ipsum"]
}'
# -> {"data":[{"embedding":[...]}, {"embedding":[...]}], ...}
```

Use the OpenAI shape (`input`, response `data[].embedding`) for drop-in
compatibility with OpenAI embedding clients.

## Reranking

Reranking scores a set of documents against a query and returns them ranked —
useful as the second stage of retrieval. Needs a rerank/cross-encoder model and
rank pooling:

```bash
llama-server -m reranker.gguf --reranking --pooling rank
```

### Endpoint

`POST /v1/rerank` (aliases: `/rerank`, `/reranking`):

```bash
curl http://localhost:8080/v1/rerank -H 'Content-Type: application/json' -d '{
  "query": "what is the capital of France?",
  "documents": ["Paris is the capital of France.",
                "Bananas are a fruit.",
                "France is in Europe."],
  "top_n": 2
}'
```

Returns the documents with relevance scores, ordered best-first (and limited to
`top_n` when given).

## Gotchas

- Embedding/rerank mode is exclusive — you can't chat on the same server.
- If `/v1/embeddings` errors about pooling, you started with `--pooling none`;
  switch to `mean`/`cls`/`last` or use the native `/embeddings` endpoint.
- Match the model to the task: a chat model is a poor embedder; use a model
  trained for embeddings/reranking.
