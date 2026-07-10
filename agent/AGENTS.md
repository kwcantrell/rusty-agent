# agent/ — Rust core (Cargo workspace)

The agent core. One of two Cargo workspaces in this repo (the other is
`src-tauri/`) — `cargo -p <crate>` here targets only these crates.

## Crates (`crates/`)

| crate | responsibility |
|-------|----------------|
| `agent-core` | agent loop, context manager, event model |
| `agent-model` | model client, tool-call protocols (native/prompted), inference types |
| `agent-tools` | shared tool vocabulary and the `Tool` trait |
| `agent-http` | outbound HTTP fetch tool; gates egress in-tool |
| `agent-mcp` | MCP client — connect to external MCP servers |
| `agent-memory` | long-term semantic memory (remember/recall/forget over a local vector store) |
| `agent-policy` | permission policy engine + approval channel |
| `agent-sandbox` | sandboxed tool/command execution |
| `agent-skills` | discover, load-on-demand, author, preload markdown skills |
| `agent-server` | library crate bridging the local agent to the desktop UI over Tauri IPC (transport-agnostic `Session`/`EventOut` core) |
| `agent-cli` | terminal front-end binary |
| `agent-runtime-config` | shared loop wiring (tool registry, protocol picker, command lists) |

## Commands

```bash
cargo build                                  # whole workspace
cargo test -p <crate>                        # test one crate
cargo run -p agent-cli -- --backend claude-cli --model sonnet --workspace .   # run the CLI
AGENT_E2E_URL=… AGENT_E2E_MODEL=… cargo test -p agent-core --test e2e_sglang -- --ignored
```

## Config & docs

- `config.example.toml` — runtime config reference; `mcp.example.json` — MCP client config example.
- `docs/RUNNING.md` — full model-server setup (llama.cpp / SGLang / vLLM / Claude CLI).
- Session traces land in `~/.rusty-agent/sessions/<id>.jsonl` (disable with `"trace": false`
  in the runtime config).
