# rust-agent-runtime

A local-first LLM agent runtime. A Rust core drives a local model (llama.cpp /
SGLang / vLLM) or the Claude CLI through a tool/policy loop, exposed three ways:

- **Terminal CLI** (`agent/crates/agent-cli`)
- **Desktop app** (`src-tauri/` — Tauri 2)
- **Browser SPA** (`web/` — React 19 + Vite) that reaches your *local* agent
  through a Cloudflare Worker

## Quickstart

Rust core (needs a model server or the Claude CLI — see
[`agent/docs/RUNNING.md`](agent/docs/RUNNING.md)):

```bash
cd agent
cargo build
cargo run -p agent-cli -- --backend claude-cli --model sonnet --workspace .
```

Web UI: `cd web && npm install && npm run dev`

Desktop app (from repo root): `npm install && npm run desktop:dev`

## Layout

| path | what |
|------|------|
| `agent/` | Rust Cargo workspace — agent core, tools, policy, sandbox, memory, skills, server, CLI |
| `src-tauri/` | Tauri 2 desktop wrapper (its own Cargo workspace) |
| `web/` | React SPA (Context Explorer UI) |
| `docs/` | specs, plans, audits (`docs/superpowers/`), knowledge bundles (`docs/okf/`) |

Working on this repo with an AI agent? Start at [`AGENTS.md`](AGENTS.md).

## License

[MIT](LICENSE)
