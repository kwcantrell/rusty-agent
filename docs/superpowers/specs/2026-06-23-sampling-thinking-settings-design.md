# Design Spec — Sampling & Thinking Settings

**Date:** 2026-06-23
**Status:** Approved design. Next step: `writing-plans` → implementation plan.
**Extends:** the built Settings capability ([`2026-06-23-settings-capability-design.md`](2026-06-23-settings-capability-design.md)) — the editable `RuntimeConfig` surface, the `settings_*` wire frames, and the web `SettingsPanel`.
**Touches core:** unlike subsystems #5/#6 (zero core changes), this slice **intentionally modifies the core crates** `agent-model` and `agent-core`, because the new knobs *are* core inference behavior. That trade-off is accepted, not a regression — see §9.

## 1. Purpose & scope

Add seven inference controls to the agent's editable settings and plumb them end-to-end (config → request → loop → wire → both front-ends):

- **Five sampling params** — `top_p`, `top_k`, `min_p`, `presence_penalty`, `repeat_penalty`. Each is *optional*: when the user leaves it unset, it is omitted from the request body so the model server applies its own default.
- **`enable_thinking`** (default `true`) — toggles server-side reasoning via the chat template; sent as `chat_template_kwargs.enable_thinking`.
- **`preserve_thinking`** (default `false`) — controls whether the model's reasoning is re-fed into context on later turns.

The model emits reasoning today only as `<think>…</think>` riding inline in `content`; nothing in the codebase separates, displays, or strips it. This subsystem introduces a **distinct reasoning channel** captured from the stream, surfaced as its own event for live display, and either re-injected into history or dropped per `preserve_thinking`.

### In scope (full vertical slice)

`RuntimeConfig` persistence + validation → `CompletionRequest` body → reasoning capture in the OpenAI client → `LoopConfig` + history handling in the core loop → a new reasoning event across the wire → CLI flags + terminal display → web `SettingsPanel` inputs + a collapsible "Thinking" display.

### Out of scope

- Per-request / per-message overrides (settings are global runtime config, as today).
- Reasoning-aware context budgeting beyond the simple strip/preserve toggle.
- Anthropic "extended thinking" semantics for the `claude-cli` backend — that backend ignores all seven knobs (§6).
- Provider-specific sampler names beyond the OpenAI-compatible + llama.cpp/SGLang superset listed above.

## 2. Architecture & data flow

```
RuntimeConfig (agent-runtime-config)      ← persisted to disk, mirrored on the settings wire frame
   │  from_launch / merge / validate
   ▼
LoopConfig (agent-core)                    ← daemon: build_loop()   CLI: built directly from flags
   │  run() builds the request
   ▼
CompletionRequest (agent-model)            ← 5 Option sampling fields + enable_thinking
   │  OpenAiCompatClient::body()
   ▼
HTTP body to model server (llama.cpp/SGLang/OpenAI-compatible)
   ▲
   │  SSE deltas:  delta.content  +  delta.reasoning_content
   ▼
parse_sse_line + ThinkingSplitter  →  Chunk::Text | Chunk::Reasoning
   ▼
one_completion → AssistantTurn { text (answer-only), reasoning, … }
   │  emits AgentEvent::Token  and  AgentEvent::Reasoning
   ▼
history: append assistant message (reasoning re-wrapped iff preserve_thinking) ; wire: WireEvent::Reasoning
```

The decisive design choice is **how reasoning is captured** (chosen: option B):

- **B — `reasoning_content` delta + an inline `<think>` streaming splitter.** Capture `delta.reasoning_content` natively *and* run a small streaming state machine over `delta.content` that routes between-tag text to the reasoning channel. Robust to both server configurations, because our live target (Qwen3 via llama.cpp) can emit either shape depending on `--reasoning-format`. The splitter is one isolated, unit-tested component.
- Rejected: **A** (`reasoning_content` only) leaks inline `<think>` into the answer and tool-call parsing; **C** (post-hoc split after the turn) loses live token-by-token reasoning display.

## 3. Data model — `agent-runtime-config`

Add to `RuntimeConfig`:

```rust
pub top_p: Option<f32>,
pub top_k: Option<u32>,
pub min_p: Option<f32>,
pub presence_penalty: Option<f32>,
pub repeat_penalty: Option<f32>,
pub enable_thinking: bool,     // default true
pub preserve_thinking: bool,   // default false
```

- Mirror every field in `PartialRuntimeConfig` and extend `merge` so a per-field on-disk override works. The two bools mirror as `Option<bool>` (absent → keep base). The five sampling params are *already* `Option` in `RuntimeConfig`, so their partial mirror is `Option<Option<…>>` with `#[serde(default)]`: key absent → `None` (don't override), explicit `null` → `Some(None)` (set to unset), value → `Some(Some(v))`. `merge` overrides only when the outer is `Some`. (If the double-`Option` proves awkward in practice, an acceptable simplification is to treat the sampling params as "absent or value" only — i.e. a plain `Option<f32>` partial where absent and null both mean "keep base" — since there is no meaningful difference between "unset in file" and "explicitly null" for these knobs.)
- `from_launch`: sampling params seed to `None`; `enable_thinking = true`; `preserve_thinking = false`.
- `validate()` — checked **only when `Some`** for the sampling params:
  - `top_p` ∈ `0.0..=1.0`
  - `min_p` ∈ `0.0..=1.0`
  - `presence_penalty` ∈ `-2.0..=2.0`
  - `repeat_penalty` > `0.0`
  - `top_k`: any `u32` (`0` = disabled — no bound)
  - booleans: no validation.
- Persistence + wire propagation come for free: `RuntimeConfig` already serializes through `save`/`load_over` and is the payload of the `settings_state` / `settings_update` frames. Older on-disk files (missing the new keys) fall back to base via the existing partial-merge — confirmed by extending the round-trip and partial-file tests.

## 4. Request model — `agent-model`

- `CompletionRequest` gains `top_p/top_k/min_p/presence_penalty/repeat_penalty: Option<…>` and `enable_thinking: bool`.
- `OpenAiCompatClient::body()`:
  - inserts each sampling field into the JSON body **only when `Some`**;
  - always sets `"chat_template_kwargs": { "enable_thinking": <bool> }` so the Qwen3 template toggle is honored. (Servers that don't recognize the key ignore it; llama.cpp/SGLang/vLLM accept it.)
- New stream variant `Chunk::Reasoning(String)`.
  - `parse_sse_line` reads `choices[0].delta.reasoning_content` → `Chunk::Reasoning`.
  - A new **`ThinkingSplitter`** unit feeds `delta.content` through a `<think>`/`</think>` state machine, emitting `Chunk::Reasoning` for in-tag text and `Chunk::Text` for the rest. It buffers a partial tag straddling a chunk boundary, so `"<thi"` + `"nk>…"` is recognized. One splitter instance lives per stream (closure/stateful adapter over the SSE line iterator).
- `AssistantTurn` gains `reasoning: String`. The answer `text` stays **reasoning-free**, so `ToolCallProtocol::parse` and tool-call extraction are unaffected.

## 5. Loop & history — `agent-core`

- `LoopConfig` gains the five sampling `Option`s + `enable_thinking` + `preserve_thinking`. `run()` copies them into each `CompletionRequest`.
- `one_completion` accumulates `Chunk::Reasoning` into `AssistantTurn.reasoning` and emits each as `AgentEvent::Reasoning(String)` (a new variant), distinct from `AgentEvent::Token`.
- **History strip / preserve** — at the point `run()` appends the assistant message:
  - `preserve_thinking == true`: store `format!("<think>{reasoning}</think>\n{answer}")` so the reasoning re-enters context next turn (re-fed to the model).
  - `preserve_thinking == false` (default): store the answer text only; reasoning is shown live but dropped from history (matches Qwen3 multi-turn guidance, saves context tokens).
  - When `reasoning` is empty, both branches store the answer unchanged.

## 6. Backend applicability — `claude-cli`

The `claude-cli` backend is a prompted text generator with no sampling controls and no `reasoning_content`. It **ignores all seven knobs**: `ClaudeCliClient::stream` already consumes only `messages`, so the sampling fields and `enable_thinking` are simply not forwarded, and no `Chunk::Reasoning` is produced. This is documented behavior, not an error — `validate()` does **not** reject sampling values under `claude-cli` (they are inert), keeping the rule consistent with how the backend already ignores `temperature`/`max_tokens`.

## 7. Wire + cloud

- New `WireEvent::Reasoning { text }` (tagged `"type":"reasoning"`); `wire_event_from` maps `AgentEvent::Reasoning(t) → WireEvent::Reasoning { text: t }`.
- The new `RuntimeConfig` fields flow over the existing `settings_state` / `settings_update` frames with no frame-shape change.
- **Cloudflare Worker: no change** — it relays event and settings frames opaquely and records them; it never inspects these fields.

## 8. Front-ends

### CLI — `agent-cli`
- New clap flags, all optional: `--top-p`, `--top-k`, `--min-p`, `--presence-penalty`, `--repeat-penalty`; `--enable-thinking` / `--no-thinking` (default on); `--preserve-thinking` (default off). They populate the directly-built `LoopConfig` in `main.rs`.
- The terminal renderer displays `AgentEvent::Reasoning` tokens in a visually distinct (dimmed) style, separated from answer tokens.

### Web — `web/`
- Extend the `RuntimeSettings` TS type (`wire.ts`/`state.ts`) with the seven fields.
- `SettingsPanel`: five number inputs where **empty ↔ `unset`** (empty string serializes to absent/`null`, not `0`), and two checkboxes for the booleans. Map cleanly back to `Option` on the Rust side.
- Handle `WireEvent::Reasoning` by accumulating reasoning per assistant turn and rendering it in a **collapsible "Thinking" section** above the answer, collapsed by default.

## 9. Core-touch trade-off (called out explicitly)

#5 and #6 each changed **zero** lines of the core crates; that discipline was the showcase. This subsystem deliberately breaks that streak in `agent-model` (`CompletionRequest`, `Chunk`, `AssistantTurn`, `body()`, the splitter) and `agent-core` (`LoopConfig`, `AgentEvent`, `run()` history handling). This is intrinsic: sampling and reasoning-capture *are* the model/loop's job — there is no seam that could carry them without the core understanding them. The change stays **additive** (new optional fields / new enum variants; existing call sites compile by adding fields), and every existing core test must still pass unchanged except where a struct literal gains fields.

## 10. Testing

**Rust (`cargo test --workspace`, clippy `-D warnings`):**
- `validate()` bounds: each sampling param rejected out-of-range when `Some`, accepted when `None`; bools always accepted.
- `RuntimeConfig` round-trip including the new fields; partial-file load falls back per-field for an older file missing them.
- `body()`: each sampling field present iff `Some`; absent keys never serialized; `chat_template_kwargs.enable_thinking` always present and reflects the flag.
- `ThinkingSplitter`: tag split across chunk boundaries; no-tag passthrough; nested/garbage tolerance; `reasoning_content` delta path produces `Chunk::Reasoning`.
- Loop history: `preserve_thinking=true` re-wraps `<think>…</think>` into the stored message; `false` stores answer-only; empty reasoning leaves the message unchanged. `AssistantTurn.text` stays reasoning-free so tool-call parsing is unaffected.
- `wire_event_from` maps `Reasoning` to the tagged `WireEvent::Reasoning`.

**Web (`npm test`):**
- `SettingsPanel` empty↔unset mapping for the five number fields; checkbox round-trip for the two bools.
- Reasoning event accumulates and renders in the collapsible section.

**Live (manual, Qwen3 via llama.cpp):**
- Drive the CLI with default (`enable_thinking` on) and `--no-thinking`; confirm reasoning is separated from the answer and that history is stripped by default. Spot-check a sampling override (e.g. `--top-k 20`) reaches the server.

## 11. Follow-ups durability

Per the project convention, record every Minor finding from the final whole-branch review (plus any Accepted won't-fix items) into `docs/superpowers/context/follow-ups.md` under a dated `## 2026-06-23 sampling-thinking-settings` section before finishing the branch.
