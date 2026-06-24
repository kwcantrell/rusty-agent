# Using llama.cpp from Rust

Two ways to drive llama.cpp from Rust:

- **Part A — talk to a running `llama-server` over HTTP** (recommended default).
  Process isolation, hot-swappable models, language-agnostic, no C toolchain.
- **Part B — native bindings that embed `libllama` in-process** (no server). Use
  when you need single-process / offline / embedded inference.

Each crate below is tagged **(A)** HTTP client for an OpenAI-compatible server,
**(B)** native FFI binding to llama.cpp, or **(C)** its own (non-llama.cpp)
engine. Versions are as of mid-2026 — check crates.io for current releases.

---

## Part A — HTTP client against llama-server

`llama-server`'s OpenAI surface is at `/v1/chat/completions` (default
`:8080`). It **ignores** the `Authorization` header (pass any non-empty key) and
ignores the request `model` field (it serves whatever was loaded at startup).

### async-openai **(A)** — recommended

Mature OpenAI client; point it at llama-server with a custom base URL.

```toml
# Cargo.toml
async-openai = "0.41"
tokio = { version = "1", features = ["full"] }
futures = "0.3"
```

```rust
use async_openai::{Client, config::OpenAIConfig};
use async_openai::types::chat::{
    ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionRequestArgs,
};

let config = OpenAIConfig::new()
    .with_api_base("http://localhost:8080/v1")
    .with_api_key("sk-no-key-required");      // ignored by llama-server, must be non-empty
let client = Client::with_config(config);

let request = CreateChatCompletionRequestArgs::default()
    .model("local")                            // ignored; any string
    .max_tokens(512u32)
    .messages([
        ChatCompletionRequestSystemMessage::from("You are helpful.").into(),
        ChatCompletionRequestUserMessage::from("Hello!").into(),
    ])
    .build()?;

let response = client.chat().create(request).await?;
println!("{:?}", response.choices[0].message.content);
```

Streaming (the lib consumes the `[DONE]` sentinel, so the stream just ends):

```rust
use futures::StreamExt;
let mut stream = client.chat().create_stream(request).await?;
while let Some(Ok(resp)) = stream.next().await {
    for choice in &resp.choices {
        if let Some(content) = &choice.delta.content { print!("{content}"); }
    }
}
```

**Compatibility gotchas:**
- **Tool calls need the server's `--jinja` flag** — without it you get
  `finish_reason:"stop"` even with `tools` set.
- Some older server builds returned `function.arguments` as a JSON *object*
  instead of a *string*, breaking strict deserialization (llama.cpp #20198, fixed
  by PR #20213). Verify your build if tool-call parsing fails.
- KV-cache quantization (`-ctk q4_0`) noticeably degrades tool-calling quality.

### Other OpenAI-compatible clients **(A)**

- **`openai_dive`** — actively maintained; `client.set_base_url("http://localhost:8080/v1")`, streaming behind a feature, `extra_body` for non-standard params.
- **`openai-api-rs`** — maintained; base URL via `OpenAIClient::builder().with_endpoint(...)` or `OPENAI_API_BASE`.
- Avoid the older/less-complete `openai-api` (openai-rs) for new work.

### Plain reqwest + serde_json **(A)** — zero abstraction

```toml
reqwest = { version = "0.12", features = ["json"] }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
futures-util = "0.3"
eventsource-stream = "0.2"   # robust SSE framing
```

Non-streaming:

```rust
let body = serde_json::json!({
    "messages": [{"role": "user", "content": "Hello!"}],
    "stream": false
});
let resp: serde_json::Value = reqwest::Client::new()
    .post("http://localhost:8080/v1/chat/completions")
    .bearer_auth("sk-no-key-required")
    .json(&body).send().await?.json().await?;
println!("{}", resp["choices"][0]["message"]["content"]);
```

Streaming — parse on **SSE event boundaries**, not raw byte chunks (a TCP read
may hold a partial or multiple `data:` frames). Use `eventsource-stream`:

```rust
use eventsource_stream::Eventsource;
use futures_util::StreamExt;

let res = reqwest::Client::new()
    .post("http://localhost:8080/v1/chat/completions")
    .json(&serde_json::json!({ "messages":[{"role":"user","content":"Hi"}], "stream":true }))
    .send().await?;

let mut stream = res.bytes_stream().eventsource();
while let Some(event) = stream.next().await {
    let ev = event?;
    if ev.data == "[DONE]" { break; }
    let chunk: serde_json::Value = serde_json::from_str(&ev.data)?;
    if let Some(c) = chunk["choices"][0]["delta"]["content"].as_str() { print!("{c}"); }
}
```

(`reqwest-eventsource` wraps this and adds reconnection for long-lived streams.)

---

## Part B — Native llama.cpp bindings (in-process, no server)

### llama-cpp-2 **(B)** — recommended native binding

The most-maintained FFI binding (the `utilityai/llama-cpp-rs` project), wrapping
llama.cpp via the generated `llama-cpp-sys-2`.

```toml
llama-cpp-2 = { version = "0.1", features = ["cuda"] }   # or "metal" / "vulkan"
```

**Build requirements:** `clang` (bindgen), `cmake`, and a C/C++ compiler — it
compiles llama.cpp from source. Backend features: `cuda` (CUDA Toolkit), `metal`
(auto on Apple Silicon), `vulkan` (`VULKAN_SDK`), `rocm`, `mkl`, plus `mtmd`
(multimodal), `sampler`, `llguidance` (grammars). It is explicitly **unsafe
FFI** — API misuse can cause UB.

Minimal generate loop:

```rust
use llama_cpp_2::{
    context::params::LlamaContextParams, llama_backend::LlamaBackend,
    llama_batch::LlamaBatch, model::{params::LlamaModelParams, AddBos, LlamaModel},
    sampling::LlamaSampler,
};

let backend = LlamaBackend::init()?;                                  // 1. backend
let model = LlamaModel::load_from_file(                               // 2. model
    &backend, "model.gguf", &LlamaModelParams::default())?;
let mut ctx = model.new_context(&backend, LlamaContextParams::default())?; // 3. context

let tokens = model.str_to_token("Hello", AddBos::Always)?;           // 4. tokenize
let mut batch = LlamaBatch::new(512, 1);
let last = tokens.len() as i32 - 1;
for (i, tok) in (0_i32..).zip(tokens.iter()) {
    batch.add(*tok, i, &[0], i == last)?;
}
ctx.decode(&mut batch)?;

let mut sampler = LlamaSampler::chain_simple([                       // 5. sampler
    LlamaSampler::dist(1234), LlamaSampler::greedy(),
]);
// loop: sampler.sample(&ctx, ...) -> accept -> token_to_piece -> re-add -> decode
```

### Other native / engine crates

| Crate | Tag | Notes |
|-------|-----|-------|
| `llama_cpp` (edgenai) | (B) | Ergonomic API but **abandoned** (last release 2024). Prefer `llama-cpp-2`. |
| `drama_llama` | (B) | Ergonomic native binding on `llama-cpp-sys-3`; self-described WIP, "not for production", low adoption. |
| `mistralrs` (mistral.rs) | (C) | **Own engine on HuggingFace Candle, not llama.cpp.** Loads GGUF + safetensors; runs as its own OpenAI/Anthropic-compatible server or embedded `Runner`. Actively maintained. |
| `kalosm` | (C) | Pure-Rust engine on Candle (not llama.cpp); loads GGUF, no C toolchain. Was stale as of mid-2026. |
| `llm` (graniet) | (A) | Multi-provider HTTP client — **not** a llama.cpp binding. |

> **Name-collision warning:** the old `rustformers/llm` (a GGML local-inference
> engine) was **archived in June 2024**. The current `llm` crate on crates.io is
> an unrelated HTTP client. Old blog posts using `llm` for local GGUF inference
> refer to the dead project — don't adopt it.

---

## Part C — Higher-level frameworks (point at llama-server)

These don't embed models; they speak to llama-server's OpenAI endpoint.

- **`rig` (`rig-core`) (A)** — actively maintained agent/RAG framework:
  ```rust
  let client = rig::providers::openai::Client::from_url("any-key", "http://localhost:8080");
  ```
- **`langchain-rust` (A)** — uses `async-openai` under the hood; set
  `OpenAIConfig::default().with_api_base("http://localhost:8080/v1").with_api_key("dummy")`.
  Functional but less actively maintained.

---

## Choosing a path

- **Most apps → Part A (HTTP).** `async-openai` for ergonomics, or raw `reqwest`
  + `eventsource-stream` for full control of the wire format. You get process
  isolation, can hot-swap/restart the model independently, and stay
  language-agnostic. Add `rig` when you want agents/RAG on top.
- **Embedded / offline / single-process → Part B (native).** `llama-cpp-2` is the
  maintained choice; budget for a C/C++ build toolchain and the right backend
  feature flag, and treat the API as unsafe FFI.
