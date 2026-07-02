# Running the agent

> **Two front-ends, one agent core.** This doc covers the **terminal CLI** (`agent-cli`)
> driving a local model directly — the fastest way to a working agent. There is also a
> **browser UI**: the `agent-server` daemon dials out to a Cloudflare Worker that a React
> SPA (`web/`) talks to, so you can drive your *local* agent from a browser. For that path
> see [`../../cloud/RUNNING.md`](../../cloud/RUNNING.md). Both use the same model server
> (below) and the same agent core.

## 1. Start an inference server (OpenAI-compatible)

**SGLang (primary target):**
```bash
python -m sglang.launch_server --model-path <hf-model> --port 30000
```
vLLM (`--port 8000`) and llama.cpp's `llama-server` (`--port 8080`) expose the same
`/v1/chat/completions` API and work identically — just change `--base-url`.

### Alternative: the Claude Code CLI as backend (no server, uses your subscription)

If you have an authenticated [Claude Code](https://docs.claude.com/en/docs/claude-code)
CLI on this machine, you can skip the inference server entirely and let the agent drive
`claude` as its model — piggybacking on your Claude subscription (no API key, no GPU):

```bash
cargo run -p agent-cli -- \
  --backend claude-cli \
  --model sonnet \
  --workspace /path/to/project
# --claude-binary <path>   # if `claude` isn't on PATH
```

How it works: `ClaudeCliClient` spawns `claude -p --output-format stream-json
--allowedTools "" --model <model>` per turn as a **pure text generator** (its own tools
are disabled), and the Rust loop owns tool execution through its policy engine as usual.
Tool calls are produced/parsed via the **prompted** protocol, so `--protocol` is forced
to `prompted` for this backend (you'll see a one-line note confirming it). `--base-url`
and `AGENT_API_KEY` are ignored. `--model` takes a Claude model alias (`sonnet`, `opus`).

Caveats (local-dev scope; see
[`../../docs/superpowers/context/claude-cli-inference.md`](../../docs/superpowers/context/claude-cli-inference.md)
for the full spike + follow-ups): each turn is a fresh `claude` process (~seconds of
latency, no warm reuse), sustained loops hit the subscription's rolling rate cap, and the
machine's `SessionStart` hooks fire inside the nested invocation (context pollution).

## 2. Run the CLI

```bash
cd agent
cargo run -p agent-cli -- \
  --backend openai \
  --base-url http://localhost:30000 \
  --model <served-model-name> \
  --protocol native \
  --workspace /path/to/project
```
`--backend` defaults to `openai` (the inference-server path above); pass
`--backend claude-cli` for the subscription path. Set `AGENT_API_KEY` if your endpoint
requires a bearer token. Tune log output with `RUST_LOG=agent_core=debug`.

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
  -ngl 99 -np 1 -c 262144 -fa on \
  --cache-type-k q8_0 --cache-type-v q8_0 \
  --host 0.0.0.0 --port 8080 --jinja
```

Key flags:
- `--jinja` — **required** for native tool-calling (activates the model's
  tool-capable chat template; without it the agent's `--protocol native` won't work).
- `-ngl 99` — all layers on GPU. `-a` sets the served model id (used as `--model` below).
- `-np 1` — a single sequence slot (the agent only needs one), so the full context
  is dedicated to one conversation.
- `-c 262144` — context window. This is the model's full native maximum
  (`n_ctx_train = 262144`); no rope/YaRN scaling needed, so no quality penalty.
- `-fa on --cache-type-k q8_0 --cache-type-v q8_0` — flash attention + q8_0-quantized
  KV cache. This model uses grouped-query attention, so KV is tiny (~11 MB per 1k
  tokens at q8_0) and large contexts are nearly free.

Measured VRAM on a 24 GB RTX 3090 (KV pre-allocated at load — it does **not** grow as
the context fills, so these are steady-state ceilings):

| `-c` | VRAM used |
|------|-----------|
| 32768 (32k)   | ~19.3 GB |
| 131072 (128k) | ~20.4 GB |
| **262144 (256k)** | **~22.2 GB** |

256k fits with ~2.4 GB to spare — comfortable on a dedicated/headless GPU. If the card
also drives a display (or you want a wider safety margin), drop to `-c 131072`.

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
  --context-limit 32768                 # how much the agent actually fills (see note)
```

**`--stream-timeout-secs <secs>`** (default 120): idle timeout for model streaming. If the
backend produces no stream progress (no open, no new chunk) for this many seconds, the
turn fails with a retryable timeout instead of hanging. Covers both the SGLang/OpenAI and
`claude-cli` backends.

**Server `-c` vs agent `--context-limit`:** `-c` is the server's *capacity* (set high,
262144, so it's never the bottleneck). `--context-limit` is how many tokens the agent
actually fills per turn before its sliding window evicts old history. Keep it **well
below** `-c` for latency — prefilling a 256k-token prompt is slow even when it fits, and
a coding agent rarely needs it. Run at `--context-limit 32768` (or `65536`) day-to-day
and raise it only for a task that genuinely needs a giant context.

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

### Web fetching: `fetch_url` tool and `--allow-host`

The `fetch_url` tool lets the agent fetch URLs and returns readable text:
- **GET-only** web fetch: makes HTTP GET requests and returns the response as text.
- **Content rendering:** HTML is passed through a readability extractor to extract
  main content; JSON and plain text pass through as-is; binary/non-text content
  (images, archives, etc.) is refused with a tool error.
- **Response bounds:** downloads are capped at ~2 MiB; the text returned to the model
  is capped at ~8 KB.

#### Controlling access: `--allow-host`

Use the `--allow-host` flag (repeatable on both `agent-cli` and `agent-serverd run`)
to name hosts that `fetch_url` may contact **without requiring an approval prompt**:

```bash
cargo run -p agent-cli -- \
  --base-url http://localhost:8080 \
  --model qwen3.6-35b-a3b \
  --workspace . \
  --allow-host docs.rs \
  --allow-host .rust-lang.org
```

Matching is **case-insensitive**. An exact host (`--allow-host example.com`) matches
only that host; a leading-dot suffix (`--allow-host .rust-lang.org`) matches the
apex domain (`rust-lang.org`) and any subdomain (`docs.rust-lang.org`, etc.).

#### Approval behavior

- **Allowlisted hosts:** fetched immediately without any prompt.
- **Non-allowlisted hosts:** trigger an interactive approval prompt:
  ```
  Allow: GET <url> ? [y]es / [n]o / [a]lways
  ```
  Choose `y` to allow once, `n` to deny, or `a` to allow this host for the rest of
  the session.

#### SSRF safety (non-overridable)

Regardless of the allowlist, `fetch_url` **always blocks** requests that resolve to:
- Loopback addresses (127.0.0.0/8, ::1)
- Private ranges (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, fc00::/7)
- Link-local (169.254.0.0/16, fe80::/10)
- Cloud metadata (169.254.169.254, fe80::a9fe:a9fe)
- Other reserved ranges

These are blocked with a `denied: ... (SSRF guard)` error, regardless of whether the
host is allowlisted. The check is applied to the resolved IP and re-applied on every
redirect, so redirects to unsafe IPs are also blocked.

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

## MCP servers (optional)

The agent can consume tools from external MCP servers over stdio. Copy
`mcp.example.json` to `mcp.json`, edit the server list, and pass `--mcp-config`:

    cargo run -p agent-cli -- --base-url http://localhost:8080 --model qwen3.6-35b-a3b \
      --workspace . --mcp-config mcp.json

On startup the CLI prints a one-line summary, e.g.
`mcp: filesystem ✓ (11 tools)`. Each server's tools appear namespaced as
`server__tool`. By default every MCP tool requires approval on each call; set a
server's `"trust": "allow"` to auto-approve a server you operate yourself.

A server that fails to start is skipped with a warning — it never blocks the agent.
The daemon takes the same `--mcp-config` flag on its `run` subcommand.

## Skills

A *skill* is a directory containing a `SKILL.md` (YAML-style frontmatter with `name` + `description`, then a markdown body) and any bundled files. Skills are discovered from:

- `<workspace>/.agent/skills` (project-local, writable — where `create_skill` writes), and
- `~/.agent/skills` (user-global, read-only),

or from explicit `--skills-dir <path>` flags (repeatable; the first is the writable root). Earlier roots win on a name conflict.

The agent gets four tools:
- `list_skills` — show the catalog (name + when-to-use).
- `use_skill {name}` — load a skill's full body + the paths of its bundled files into context.
- `read_skill_file {skill, path}` — read one bundled file (read-only, confined to the skill's directory).
- `create_skill {name, description, body, files?}` — author a new skill under the writable root (a Write action → goes through approval).

Bundled scripts are run with the ordinary `execute_command` tool, gated by the command allow/deny policy + approval — the skills subsystem never executes anything itself.

### Skill examples

Put worked exemplars under a skill's `examples/` directory. They surface as a
distinct "Examples" section when the skill is loaded (with guidance to imitate
their shape, not copy content), get an `[N examples]` marker in `list_skills`,
and are read on demand with `read_skill_file` — nothing is injected into the
prompt until the model asks.

Preload a skill as a **preset** (its body injected into the system prompt from the first turn):

    cargo run -p agent-cli -- ... --skill code-review --skill changelog

`--skills-dir` and `--skill` are accepted by both `agent-cli` and `agent-serverd run`.

### Sampling & thinking flags

- `--top-p`, `--top-k`, `--min-p`, `--presence-penalty`, `--repeat-penalty` — optional
  sampler overrides; omitted from the request when unset (server default applies).
  `top-k`/`min-p`/`repeat-penalty` are llama.cpp/SGLang extensions, ignored by stock OpenAI.
- `--no-thinking` — turn off model reasoning (sends `chat_template_kwargs.enable_thinking=false`).
  Reasoning is on by default and shown dimmed in the terminal.
- `--preserve-thinking` — keep prior `<think>` reasoning in history across turns
  (default: stripped, per Qwen3 multi-turn guidance). Ignored by the `claude-cli` backend.
