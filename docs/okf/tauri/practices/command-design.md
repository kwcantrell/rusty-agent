---
type: Practice
title: Command design and events-vs-channels
description: Design the Rust/JS IPC boundary well — type-safe commands with Result-based error handling, and choosing between events and channels by data volume, mindful of the JSON-serialization ceiling.
tags: [ipc-architecture]
timestamp: 2026-07-09T00:00:00Z
---

# Command design and events-vs-channels

Tauri's IPC is asynchronous message passing — processes exchange serialized
requests and responses, which is safer than shared memory because the recipient
is free to reject or discard requests, letting Tauri Core dismiss malicious ones
without executing them [1]. Two primitives sit on top: **commands** (a
foreign-function-interface-like call, `invoke`, comparable to `fetch`) and
**events** (fire-and-forget one-way messages) [1]. Designing the boundary well
means picking the right primitive and respecting the serialization constraints
underneath.

## Commands: the type-safe default

Commands are the primary, recommended way to call Rust with type safety [2].
Annotate a function with `#[tauri::command]`, register it with
`generate_handler!`, and call it from JS via `invoke` [2]:

```rust
#[tauri::command]
fn my_command(invoke_message: String) { /* ... */ }

tauri::Builder::default()
  .invoke_handler(tauri::generate_handler![my_command])
```

Commands take typed arguments and return typed values, may be `async`, and can
receive injected framework types — `tauri::WebviewWindow`, `tauri::State<T>` (see
[/practices/state-management.md](/practices/state-management.md)), or a streaming
`Channel` [2]. Errors cross the boundary via `Result<T, E>`: return `Err(...)` and
the JS `invoke` promise rejects, so model failures as a `Result` return type
rather than panicking [2]:

```rust
#[tauri::command]
fn login(user: String, password: String) -> Result<String, String> {
  if user == "tauri" && password == "tauri" { Ok("logged_in".into()) }
  else { Err("invalid credentials".into()) }
}
```

## Events vs channels: choose by data volume and direction

For pushing data *from Rust to the frontend*, there are three approaches, and the
right one depends on volume [3]. **Events** are the built-in bidirectional
mechanism, best "for situations where small amounts of data need to be streamed"
— lifecycle notifications, state changes, infrequent updates [3][1]. Emit globally
with `app.emit(...)` or to a specific webview with `app.emit_to(label, ...)`;
payloads are JSON-only and one-way [3][2]. **Channels** are for higher-performance,
*ordered* delivery — "streaming operations such as download progress, child
process output and WebSocket messages" — and can be received as a typed enum on the
Rust side and passed into a command as a `Channel<T>` argument, then sent to as the
work progresses [3][2]. **JavaScript evaluation** (`WebviewWindow#eval`) is the
simplest, for immediate one-off frontend state changes only [3].

The rule of thumb: events for small/infrequent data or lifecycle signals, channels
for high-volume streaming, and `eval` only for trivial immediate changes [3].

## Respect the serialization ceiling

Commands use a JSON-RPC-like protocol under the hood, so *all* arguments and return
data must be JSON-serializable [1]. This is a genuine performance ceiling, not a
detail — parameters and return values are serialized as strings due to webview
library constraints, which is the fundamental bottleneck for standard `invoke`
calls [4]. When that ceiling bites, the maintainer-endorsed escape hatch is to
register a custom URI-scheme protocol via `register_uri_scheme_protocol`, reading
headers as parameters and returning browser-native responses to bypass the
serialization overhead [4]. Platform-specific paths exist too: `ipc.postMessage` or
custom protocols on Linux, custom protocols on macOS, and Windows `sharedBuffer`
for zero-copy messaging [4]. Note the limit even here — v2's async URI-scheme
protocol allows delayed responses but does *not* support true streaming or chunked
transfer for long-lived connections, reflecting WebView2 and equivalent platform
constraints [4]. For the blocking behavior of `Channel::send` under large payloads,
see [/practices/state-management.md](/practices/state-management.md).

# Citations

1. [Inter-Process Communication overview](/sources/tauri-ipc-overview.md)
2. [Calling Rust from the Frontend](/sources/tauri-calling-rust.md)
3. [Calling the Frontend from Rust](/sources/tauri-calling-frontend.md)
4. [IPC Improvements discussion](/sources/tauri-discussion-ipc-improvements.md)
