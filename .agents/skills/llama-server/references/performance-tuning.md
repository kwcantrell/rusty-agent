# Performance & concurrency tuning

The levers that determine throughput, latency, and how much you can fit in
memory — and the context math people get wrong.

## Context is split across slots (read this first)

`-c` / `--ctx-size` is the **total** KV context. With `-np N` slots that total is
**divided** across slots:

```
per-slot context ≈ ctx_size / n_parallel
```

So `-c 16384 -np 4` gives each concurrent request only ~**4096** tokens. If a
request's prompt + generation exceeds its slot's share, it truncates or shifts.

**Size it deliberately:** `ctx_size = n_parallel × expected_context_per_request`.
Serving 4 users who each need 8K context ⇒ `-np 4 -c 32768` (and enough VRAM/RAM
for that KV cache).

## Concurrency: slots & continuous batching

- `-np N` sets the number of slots = max concurrent requests. Extra requests
  queue (or get `503` with `?fail_on_no_slot=1` on `/slots`).
- `-cb` / continuous batching is **on by default**: the server interleaves tokens
  from multiple active requests in one batch, raising aggregate throughput. Leave
  it on for multi-user serving.
- More slots ⇒ higher total throughput but less context each and more KV memory.

## GPU offload (`-ngl`)

The single biggest speed lever. Raise `-ngl` to put more layers on the GPU until
you run out of VRAM:

- `-ngl 99` / `-ngl all` — everything on GPU (fastest if it fits).
- Partial (`-ngl 20`) — split CPU/GPU; each non-offloaded layer costs speed.
- `-ngl 0` — CPU-only.
- `-fit on` (default) auto-trims unset args to fit device memory.

If load OOMs, lower `-ngl` first, then `-c`, then quantize the KV cache (below).

## KV-cache quantization

`-ctk` / `-ctv` set the KV cache data type. Dropping from `f16` to `q8_0` (or
`q4_0`) roughly halves (or quarters) KV memory, letting you raise `-c` or `-np`:

```bash
llama-server -m model.gguf -c 32768 -np 4 -ngl 99 -ctk q8_0 -ctv q8_0
```

Trade-off: lower precision can slightly reduce quality — and notably degrades
**tool-calling** reliability, so be conservative when serving function calls.

## Batch sizes (prompt throughput)

- `-b` / `--batch-size` (default `2048`) — logical batch; larger speeds up long
  prompt ingestion.
- `-ub` / `--ubatch-size` (default `512`) — physical batch; affects memory and
  prompt-eval speed.

Raise these to accelerate **prompt processing** (time-to-first-token on long
prompts) at the cost of memory.

## Prompt / KV cache reuse

Prompt caching is on by default: a request that shares a prefix with a slot's
prior prompt skips reprocessing that prefix — big win for a fixed system prompt
or shared document.

> **Nondeterminism caveat:** with caching/continuous batching, results can vary
> run-to-run because batch composition changes the numerics. For reproducible
> output, fix the `seed`, reduce `-np`, and/or disable prompt caching for that
> request.

Persist prefixes across restarts with slot save/restore (`advanced-inference.md`).

## CPU threads

For CPU or hybrid serving:
- `-t` / `--threads` — generation threads (default auto).
- `-tb` / `--threads-batch` — prompt-processing threads.
- `--threads-http` — HTTP worker threads (raise under many concurrent clients).

## Quick diagnosis

- **Slow *first* token, fast after** → prompt processing is the bottleneck: more
  `-ngl`, larger `-b`/`-ub`, enable prompt cache reuse.
- **Slow *every* token** → generation is the bottleneck: more `-ngl`, smaller/
  more-quantized model, or speculative decoding (`advanced-inference.md`).
- **Requests queueing** → too few slots: raise `-np` (and `-c` to match).

Watch live behavior via `/slots`, `/props`, and `/metrics` (see `http-api.md`),
and see `troubleshooting.md` for OOM and overflow symptoms.
