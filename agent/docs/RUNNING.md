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
