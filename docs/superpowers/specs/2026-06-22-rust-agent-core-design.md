# Rust Agent Core — Design Spec

**Date:** 2026-06-22
**Status:** Approved design, ready for implementation planning
**Scope:** First slice of a larger local-first AI agent platform — the Rust agent runtime core, driven from a CLI.

---

## 1. Context & goal

The broader vision is a Claude Code–style, local-first AI coding agent: a browser frontend (Cloudflare Pages) and control plane (Workers/Durable Objects/D1/R2) front a **local Rust agent daemon** that executes tools on the user's machine and talks to a **local inference server** (SGLang on GPU).

That full platform is **8+ independently substantial subsystems**. This spec covers only the **first and most central piece: the Rust agent core**, runnable and testable from a CLI with no web layer yet. The goal is a working, well-architected agent loop that demonstrates clean, decoupled design.

**Project intent:** learning / portfolio. Optimize for a working end-to-end vertical slice and clean architecture (the architecture *is* the showcase) over production hardening.

### In scope (this spec)
- ReAct agent loop (`agent-core`)
- Model client, OpenAI-compatible, SGLang-first but pluggable (`agent-model`)
- Dual tool-call protocols behind a trait: native function-calling + prompted-JSON (`agent-model`)
- Tool system with fs/shell/git tools (`agent-tools`)
- Real permission/policy engine with pluggable approval channel (`agent-policy`)
- Context manager: token counting + sliding window (`agent-core`)
- CLI binary that wires it together and renders streamed output (`agent-cli`)

### Deferred to their own later specs
- Cloudflare control plane (Workers, Durable Objects, D1, R2, auth, presence)
- React frontend (chat, diffs, terminal, permission UI, settings)
- Vector memory / long-term memory system (Qdrant/pgvector/LanceDB)
- MCP client support
- OS-level sandboxing (Linux namespaces, containers, Firejail, WASM)
- `http_request` / browser tool

The core's interfaces are deliberately designed so these later pieces bolt on **additively** through two seams (`EventSink`, `ApprovalChannel`) without modifying the core.

Each deferred subsystem has a high-level **context primer** in [`../context/`](../context/README.md) — enough for a fresh agent session to understand it and run its own `brainstorming` → spec → plan cycle.

---

## 2. Key decisions (and why)

| Decision | Choice | Rationale |
|---|---|---|
| Code structure | Cargo workspace of 5 focused crates | One job per crate, clean interfaces, testable in isolation; later API-server crate depends on `agent-core` with no rewrite. |
| Control loop | ReAct (reason → act → observe → repeat), no explicit upfront plan | Simplest and most robust on open models; what Claude Code essentially does. A todo/plan tool can be added later. |
| Tool-call mechanism | `ToolCallProtocol` trait with **both** native function-calling and prompted-JSON implementations, selectable per model | Maximally flexible across open models; showcases clean abstraction. |
| Dev inference backend | Local GPU + SGLang/vLLM | Validate against the real target from day one; client stays OpenAI-compatible/pluggable. |
| Permissions | Real policy engine now + interactive CLI approval | Enforcement logic is reusable; web UI later just swaps the approval frontend. |
| Context manager | Token counting + sliding window | Enough to run real tasks; `ContextManager` trait lets summarization slot in later. |
| UI decoupling | Loop emits structured events to an `EventSink` | Core is UI-agnostic from day one; terminal now, Worker/WebSocket later. |

---

## 3. Workspace layout

```
agent/
├── Cargo.toml                # workspace manifest
└── crates/
    ├── agent-core/           # agent loop, context manager, event model
    ├── agent-tools/          # Tool trait + ToolRegistry + fs/shell/git tools
    ├── agent-model/          # ModelClient + ToolCallProtocol + backends
    ├── agent-policy/         # PolicyEngine + ApprovalChannel + rules
    └── agent-cli/            # binary: wiring, terminal UI, ApprovalChannel impl
```

Dependency direction (no cycles):
```
agent-cli ──> agent-core ──> agent-model
          └─> agent-tools <──┘  (core uses tools)
          └─> agent-policy
```
`agent-core` depends on `agent-model`, `agent-tools`, `agent-policy` (via their traits). `agent-cli` depends on all and supplies the concrete `ModelClient`, `ApprovalChannel`, and `EventSink`.

---

## 4. Architecture & data flow

The control flow is a single async loop in `agent-core`. One turn:

```
┌─────────────────────────────────────────────────────────────┐
│ agent-cli (binary)                                           │
│   • reads user input, renders streamed output + diffs        │
│   • implements ApprovalChannel (terminal y/n prompt)         │
│   • implements EventSink (renders agent events)              │
└───────────────┬─────────────────────────────────────────────┘
                │ owns & drives
┌───────────────▼─────────────────────────────────────────────┐
│ agent-core :: AgentLoop                                      │
│                                                              │
│  loop {                                                      │
│    1. ContextManager.build()  → Vec<Message> (fits window)   │
│    2. ModelClient.stream(req) → Chunk stream                 │
│       └─ ToolCallProtocol.parse() → text + Vec<ToolCall>     │
│    3. if tool_calls:                                         │
│         for each call:                                       │
│           PolicyEngine.check(intent) → Allow | Deny | Ask    │
│             └─ Ask → ApprovalChannel.request() (async)       │
│           ToolRegistry.execute(call) → ToolResult            │
│           ContextManager.append(tool result)                 │
│       else:                                                  │
│         emit final assistant message → break                 │
│  }   // bounded by a turn budget                             │
└──┬────────────────┬───────────────┬─────────────────────────┘
   │                │               │
┌──▼──────────┐ ┌───▼──────────┐ ┌──▼───────────────┐
│ agent-model │ │ agent-tools  │ │ agent-policy     │
│ ModelClient │ │ ToolRegistry │ │ PolicyEngine     │
│ +Protocol   │ │ +Tool trait  │ │ +ApprovalChannel │
└─────────────┘ └──────────────┘ └──────────────────┘
```

### Decoupling seams (all traits)
- **`ModelClient`** — swap SGLang / vLLM / llama.cpp / cloud without touching the loop.
- **`ToolCallProtocol`** — native vs prompted, chosen per model.
- **`Tool`** — add tools without touching the loop.
- **`ApprovalChannel`** — terminal prompt now, WebSocket-to-browser later.
- **`EventSink`** — the loop emits structured events; CLI subscribes and renders. This is the seam the Cloudflare layer later taps to stream to the browser. **The loop never knows whether it's talking to a terminal or a Worker.**

---

## 5. Key interfaces

Signatures are illustrative (final names may shift in implementation). `async` shown via `async-trait`.

### `agent-model` — inference

```rust
/// One backend connection (SGLang / vLLM / llama.cpp / cloud).
#[async_trait]
pub trait ModelClient: Send + Sync {
    async fn stream(
        &self,
        req: CompletionRequest,
    ) -> Result<BoxStream<'static, Result<Chunk, ModelError>>, ModelError>;
}

pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
}

pub enum Chunk {
    Text(String),                 // streamed assistant text/reasoning
    ToolCallDelta(RawToolCall),   // partial/complete tool call (protocol-dependent)
    Done(StopReason),
}

/// Native function-calling vs prompted-JSON. Chosen per model.
pub trait ToolCallProtocol: Send + Sync {
    /// Inject tool schemas into the outbound request (prompt or `tools` field).
    fn prepare(&self, req: &mut CompletionRequest);
    /// Turn a finished assistant turn into clean text + structured calls.
    fn parse(&self, raw: &AssistantTurn) -> Result<ParsedTurn, ProtocolError>;
}

pub struct ParsedTurn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,   // normalized: id, name, json args
}
```

Backends implemented in this slice: an `OpenAiCompatClient` (covers SGLang, vLLM, llama.cpp server, and cloud OpenAI-compatible endpoints via base-URL/config). Two `ToolCallProtocol` impls: `NativeProtocol` (uses the `tools`/`tool_calls` fields) and `PromptedJsonProtocol` (injects schemas into the system prompt, parses a JSON/XML block from content).

### `agent-tools` — tool framework

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> ToolSchema;          // JSON Schema for args
    /// Declares what this call wants to do, so policy can judge it BEFORE execution.
    fn intent(&self, args: &Value) -> ToolIntent;
    async fn execute(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError>;
}

pub struct ToolCtx {
    pub workspace: PathBuf,        // root boundary
    pub timeout: Duration,
    pub cancel: CancellationToken, // cooperative cancel for long ops
}

pub struct ToolOutput {
    pub content: String,           // what goes back to the model
    pub display: Option<Display>,  // richer UI payload: Diff, Terminal, etc.
}
```

`intent()` separates **declaration from execution**: a tool describes what it will do (paths touched, command, read vs write), and `PolicyEngine` rules on it before `execute()` runs. Policy logic stays out of individual tools, and OS-sandboxing (later) plugs in at this one place.

`ToolOutput.display` carries structured render data (diffs, terminal blocks) so the eventual frontend gets rich visualization for free; the CLI renders a text fallback.

### `agent-policy` — permissions & approval

```rust
pub trait PolicyEngine: Send + Sync {
    fn check(&self, intent: &ToolIntent) -> Decision;   // Allow | Deny(reason) | Ask
}

#[async_trait]
pub trait ApprovalChannel: Send + Sync {
    async fn request(&self, req: ApprovalRequest) -> ApprovalResponse; // Approve | ApproveAlways | Deny
}
```

`ApprovalChannel` is implemented by `agent-cli` as a terminal y/n now; later by a WebSocket bridge to the browser. `ApproveAlways` mutates session policy so the user isn't re-prompted for equivalent intents.

### `agent-core` — loop seams

```rust
pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}

pub enum AgentEvent {
    Token(String),
    ToolStart { name: String, args: Value },
    ToolResult { name: String, output: ToolOutput },
    ApprovalRequest(ApprovalRequest),
    Error(AgentError),
    Done(StopReason),
}

pub trait ContextManager: Send + Sync {
    fn append(&mut self, msg: Message);
    fn build(&self, model_limit: usize) -> Vec<Message>;  // fits the window
}
```

The default `ContextManager` impl: count tokens per message; when the assembled context nears `model_limit`, drop/compact the oldest turns while always preserving the system prompt and the most recent turns + their tool results. Summarization is a future impl behind the same trait.

---

## 6. Tool set (initial)

Each implements `Tool` and declares `intent()`:

| Tool | Intent / policy treatment |
|---|---|
| `read_file` | Read within workspace → Allow; outside workspace → Ask |
| `write_file` | Write → Ask by default (diff shown in approval) |
| `edit_file` | String-replace edit → Ask, diff shown |
| `list_directory` | Read-only within workspace → Allow |
| `execute_command` | Checked against command allowlist; non-allowlisted → Ask; hard-denylist (e.g. `rm -rf /`, `sudo`) → Deny |
| `git_status` / `git_diff` | Read-only → Allow |
| `git_commit` | Mutating → Ask |

`http_request`/browser deferred — not needed for the core demo and widens the security surface.

---

## 7. Error handling

The loop is resilient, not crash-prone:

- **`ToolError`** is returned **to the model** as a tool result (so it can self-correct), not propagated as fatal. Carries a kind: `Denied`, `Timeout`, `NotFound`, `Failed { stderr }`.
- **`ModelError`** (network / 5xx / malformed stream): retry with backoff (configurable, default 3 attempts); exhausted → emit `Error` event, pause loop, surface to user.
- **`ProtocolError`** (unparseable tool call): feed a corrective message back to the model ("your tool call was malformed: …") for one self-repair attempt before failing the turn.
- Every tool runs under `tokio::time::timeout` with cooperative cancellation via `CancellationToken`.
- A **turn budget** (max iterations, default ~25) prevents infinite loops; hitting it emits `Done(BudgetExhausted)`.

---

## 8. Observability

- `tracing` throughout, with structured spans per turn and per tool-call.
- JSON log output option.
- This is the substrate the later R2 log-shipping taps; no rework needed.

---

## 9. Testing strategy

TDD throughout: trait → failing test → impl.

- **Unit (per crate, isolated):**
  - Tools tested against a temp workspace directory.
  - `PolicyEngine` tested as a pure function over `ToolIntent` values.
  - Protocol parsers tested against **golden fixtures** of captured real-model outputs (both native and prompted formats).
  - `ContextManager` tested for window-fitting and preservation rules.
- **Loop tests (`agent-core`):** run against a `MockModelClient` that replays scripted turns (text, tool calls, errors). Fast, deterministic, no GPU/network. Asserts the loop executes tools, handles denials, retries on `ModelError`, self-repairs on `ProtocolError`, and respects the turn budget.
- **Integration (feature-flagged):** one end-to-end test hitting a real SGLang endpoint with a tiny task ("read this file and tell me what it says"); run manually or in CI-with-GPU.

---

## 10. Forward-compatibility summary

The core is architected so deferred subsystems are additive:

- **Cloudflare/web layer** taps `EventSink` (stream events to browser) and provides a WebSocket `ApprovalChannel` — the loop is unchanged.
- **OS sandboxing** plugs in at the `intent()` → `PolicyEngine`/execution boundary.
- **Vector memory** becomes an alternate/extended `ContextManager` impl and/or a retrieval tool.
- **MCP** becomes additional `Tool` implementations backed by an MCP client, registered in `ToolRegistry`.
- **Summarization** is a drop-in `ContextManager` impl behind the existing trait.

---

## Recap

A Cargo workspace of 5 focused crates implementing a trait-decoupled, event-driven **ReAct agent core**: a pluggable OpenAI-compatible model client (SGLang-first), dual tool-call protocols, a real permission engine with pluggable approval, fs/shell/git tools with declared intents, and token-windowed context — runnable from a CLI and architected so the Cloudflare/web layers, memory, MCP, and sandboxing later bolt on through the `EventSink` and `ApprovalChannel` seams without touching the core.
