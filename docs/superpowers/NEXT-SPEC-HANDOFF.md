# Next-Spec Handoff Prompt

A reusable, paste-ready prompt for starting a **fresh agent session** to brainstorm and
spec the next deferred subsystem of this platform.

**How to use:** copy the block below into a new session. Fill in the bracketed
"Subsystem to spec this session" line with one subsystem (or leave it for the agent to
help you choose from the build order). The prompt deliberately points at the on-disk
docs rather than restating everything, so the new agent reads current truth.

Keep this file updated as the project state changes (crates added, subsystems completed,
environment changes).

---

```
I'm continuing work on a local-first AI coding-agent platform. The first slice — the
Rust "agent core" — is built, reviewed, merged to `main`, and validated end-to-end
against a real local model. I want to brainstorm and spec the NEXT subsystem.

## Repo
/home/kalen/rust-agent-runtime  (git repo, on `main`)

## What already exists (don't re-derive — read the docs below)
A Cargo workspace under `agent/` of 5 trait-decoupled crates implementing a CLI-driven
ReAct agent: agent-tools (Tool trait, registry, fs/shell/git tools with a workspace
path guard + command allow/deny policy), agent-policy (PolicyEngine + ApprovalChannel),
agent-model (OpenAI-compatible SSE streaming client + native & prompted tool-call
protocols), agent-core (the ReAct loop, token-windowed ContextManager, EventSink/
AgentEvent model), agent-cli (terminal renderer + approval + REPL). 48 tests pass,
clippy `-D warnings` clean. Every cross-crate seam is a trait, so new subsystems bolt
on ADDITIVELY through two seams: `EventSink` (observe/stream the agent) and
`ApprovalChannel` (gate tools through a different UI). New tools register in
`ToolRegistry`; alternate context strategies implement `ContextManager`.

It's been driven end-to-end against a real local model (Qwen3.6-35B-A3B via llama.cpp):
the loop, native tool-calling, approvals, diffs, and execute_command all work. So treat
the core as proven, not just compiled.

## Read these first (authoritative context)
- Core design spec:    docs/superpowers/specs/2026-06-22-rust-agent-core-design.md
- Core impl plan:      docs/superpowers/plans/2026-06-22-rust-agent-core.md
- Deferred-subsystem primers + recommended build order:
                       docs/superpowers/context/README.md
  (one primer each for: http-tool, os-sandboxing, mcp-client, memory-system,
   cloudflare-control-plane, react-frontend — each states what it is, which core seam
   it attaches to, open questions, and definition of done)
- How to run / the live model setup:  agent/docs/RUNNING.md

## Project intent / constraints (carry these forward)
- Learning/portfolio, local-first. Optimize for a clean, well-architected vertical
  slice over production hardening; the trait-decoupled architecture is the showcase.
- Each subsystem is its own spec → plan → implement cycle. Do ONE subsystem now.
- Keep the core untouched where possible; attach via the existing seams.
- Carry these notes from the core's final review (relevant to sandboxing/MCP specs):
  the command allow/deny policy is a guardrail, not a sandbox; the workspace path guard
  is lexical (no symlink resolution); OS-level sandboxing is deliberately deferred.

## Subsystem to spec this session
[FILL IN ONE — or ask me to choose. Per the build order in context/README.md:
 Cloudflare control plane (#5) is the path to the browser experience and needs a new
 `agent-server` (Axum) crate exposing EventSink/ApprovalChannel over WebSocket (the
 React frontend, #6, is its follow-up); http-tool / mcp-client / memory-system /
 os-sandboxing (#1–#4) are independent local deepeners doable in any order.]

## Your task
Use the brainstorming skill to turn the chosen subsystem's primer into an approved,
written design spec (saved under docs/superpowers/specs/), then stop at the
writing-plans handoff. Read the relevant context primer first. Ask clarifying questions
one at a time. Do NOT write code or scaffold anything until I approve the design.

## Environment notes
- Rust 1.96 via rustup, but cargo is NOT on PATH — run `source "$HOME/.cargo/env"`
  before any cargo command. Build/test from `agent/`: `cargo test --workspace`,
  `cargo clippy --all-targets -- -D warnings`.
- A live model is available for testing if relevant: llama.cpp server
  (OpenAI-compatible) on http://localhost:8080, model id `qwen3.6-35b-a3b`, running in
  Docker as container `llama-agent` at 256k context. If it's not up:
  `docker start llama-agent` (or see the launch command in agent/docs/RUNNING.md).
- Drive it: `cargo run -p agent-cli -- --base-url http://localhost:8080 --model
  qwen3.6-35b-a3b --workspace <dir> --context-limit 32768`. Keep --context-limit well
  below the server's capacity for latency (see RUNNING.md for the -c vs --context-limit
  note).
```
