# Running the agent

## 1. Start an inference server (OpenAI-compatible)

**SGLang (primary target):**
```bash
python -m sglang.launch_server --model-path <hf-model> --port 30000
```
vLLM (`--port 8000`) and llama.cpp's `llama-server` (`--port 8080`) expose the same
`/v1/chat/completions` API and work identically — just change `--base-url`.

## 2. Run the CLI

```bash
cd agent
cargo run -p agent-cli -- \
  --base-url http://localhost:30000 \
  --model <served-model-name> \
  --protocol native \
  --workspace /path/to/project
```
Set `AGENT_API_KEY` if your endpoint requires a bearer token. Tune log output with
`RUST_LOG=agent_core=debug`.

## 3. End-to-end test against your server

```bash
AGENT_E2E_URL=http://localhost:30000 AGENT_E2E_MODEL=<name> \
  cargo test -p agent-core --test e2e_sglang -- --ignored --nocapture
```

---

## Quick start: current local setup (llama.cpp in Docker, NVIDIA GPU)

This is the concrete, working dev setup on this machine. The local models are
llama.cpp **GGUF** files (incl. Unsloth dynamic quants), which SGLang/vLLM can't
serve — so we use llama.cpp's `llama-server`, which exposes the same
OpenAI-compatible API, so the agent works identically.

### Launch the model server (`llama-agent` container)

```bash
docker rm -f llama-agent 2>/dev/null   # clear any old instance

docker run -d --name llama-agent --gpus all -p 8080:8080 \
  -v /mnt/storage/models:/models:ro \
  ghcr.io/ggml-org/llama.cpp:server-cuda \
  -m /models/qwen3.6-35b-a3b-gguf/Qwen3.6-35B-A3B-UD-IQ4_XS.gguf \
  -a qwen3.6-35b-a3b \
  -ngl 99 -c 32768 -fa on \
  --cache-type-k q8_0 --cache-type-v q8_0 \
  --host 0.0.0.0 --port 8080 --jinja
```

Key flags:
- `--jinja` — **required** for native tool-calling (activates the model's
  tool-capable chat template; without it the agent's `--protocol native` won't work).
- `-ngl 99` — all layers on GPU. `-a` sets the served model id (used as `--model` below).
- `-fa on --cache-type-k q8_0 --cache-type-v q8_0` — flash attention + quantized KV
  cache. Lets 32k context fit in ~19 GB on a 24 GB card (RTX 3090). This model is
  natively trained to 256k, so 32k needs no rope/YaRN scaling.

Verify it's up (the model loads in a few seconds):

```bash
curl -s localhost:8080/health           # -> {"status":"ok"}
curl -s localhost:8080/v1/models        # -> lists "qwen3.6-35b-a3b"
nvidia-smi --query-gpu=memory.used,memory.total --format=csv,noheader
```

### Drive it with the agent CLI

```bash
source "$HOME/.cargo/env"               # cargo is not on PATH by default here
cd agent
cargo run -p agent-cli -- \
  --base-url http://localhost:8080 \
  --model qwen3.6-35b-a3b \
  --protocol native \
  --workspace /path/to/your/project \
  --context-limit 32768                 # match the server's -c to use the full window
```

At the `›` prompt, type a task. The agent streams its work and calls tools:
- Read-only tools inside the workspace (`read_file`, `list_directory`, `git_status`)
  run automatically.
- Mutating tools (`write_file`, `edit_file`, `git_commit`) and non-allowlisted or
  metacharacter-containing shell commands prompt for approval: answer
  `y` (once) / `n` (deny) / `a` (approve this one). Type `exit` to quit.

Note on shell commands: a command is auto-approved only if its first token is in the
allowlist **and** it contains no shell metacharacters. So `cargo build` runs
unprompted, but `cargo build 2>&1` or `cmd && other` (redirects/operators) require
approval — by design, since the whole string is passed to `sh -c`.

### Manage the container

```bash
docker logs -f llama-agent      # watch server logs
docker stop llama-agent         # stop (keeps the container; `docker start llama-agent` to resume)
docker start llama-agent        # restart a stopped instance
docker rm -f llama-agent        # remove entirely
```

To serve a different local model, change `-m` (and `-a`); e.g. the dense
`Qwen3.6-27B-UD-Q4_K_XL.gguf`. Models larger than ~20 GB (e.g. `gpt-oss-120b`,
60 GB) exceed 24 GB VRAM and need CPU offload (slower).
