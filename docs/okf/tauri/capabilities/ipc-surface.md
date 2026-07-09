---
type: Capability
title: IPC surface
description: Tauri's three IPC primitives — commands, events, and channels — their contracts, the JSON serialization boundary, and where each fits.
tags: [ipc-architecture, core, performance]
timestamp: 2026-07-09T00:00:00Z
---
# IPC surface

Inter-Process Communication is how the WebView frontend and the Rust Core talk, and Tauri frames it as "the key to building more complex applications" [1]. The model is **asynchronous message passing**: processes exchange serialized requests and responses rather than sharing memory or calling each other's functions directly [1]. Tauri presents this as a deliberate safety property — because the recipient is free to reject or discard requests, the Core can dismiss a potentially malicious message without executing it [1].

The overview describes two core primitives, commands and events [1]; in practice the develop-facing API surface exposes three distinct mechanisms, since channels are a first-class streaming path [2][3].

## Commands

Commands are a foreign-function-interface-like abstraction built on IPC messages, and they are the recommended way to call Rust from the frontend [1][2]. A Rust function annotated with `#[tauri::command]` and registered through `invoke_handler(generate_handler![...])` becomes callable from JavaScript via `invoke`, which the docs compare to the browser's `fetch` API [1][2]. Commands carry a richer contract than events: typed arguments and return values, `async` support, `Result`-based error handling that surfaces as a rejected promise on the JS side, and dependency injection of framework values such as the `WebviewWindow`, the `AppHandle`, and managed `State` [1][2]. Command state access is one place the type system does not protect you — a `State<T>` whose `T` does not match what was registered panics at runtime rather than failing to compile [4].

## Events

Events are fire-and-forget, one-way messages suited to lifecycle notifications and state changes [1]. Both the frontend and the Core can emit, so the channel is bidirectional, and an event can be broadcast globally to all webviews or targeted at a specific webview [1][3]. Events are simpler but less type-safe than commands: payloads are JSON-only and delivery is one-way, which the docs frame as best for small amounts of data being streamed [2][3]. Emission from Rust uses the `Emitter` trait (`app.emit`, `app.emit_to`); listening from JS uses `listen` [2][3].

## Channels

Channels are the high-throughput path — an ordered, typed stream from Rust to the frontend built for "streaming operations such as download progress, child process output and WebSocket messages" [3]. A command takes a `tauri::ipc::Channel<T>` parameter and calls `.send()` repeatedly; the JS side receives via a `Channel` whose `onmessage` handler fires per message [2][3].

## The serialization boundary and its ceiling

All command arguments and return values must be JSON-serializable: commands use a JSON-RPC-like protocol under the hood [1]. That boundary is also a performance ceiling. Maintainers describe the root cause as webview library constraints forcing all parameters and return values to be serialized as strings, and note that Tauri v2's asynchronous URI-scheme protocol allows delayed responses but does not support true streaming or chunked transfer for long-lived connections [5]. The documented escape hatch for maximum throughput is registering a custom URI-scheme protocol (`register_uri_scheme_protocol`) to bypass standard serialization, with platform-specific strategies — `ipc.postMessage` or custom protocols on Linux, custom protocols on macOS, and `sharedBuffer` zero-copy messaging on Windows [5]. Even channels are not free of the copy cost: a user reported `Channel::send` blocking the calling thread 30–50ms when transmitting roughly 3MB video frames, though that thread carries only a self-answer and no maintainer confirmation [6].

## Hardening the surface

The IPC surface can be wrapped by the Isolation pattern, which injects JavaScript into a sandboxed iframe to intercept, validate, and optionally modify frontend-to-Core messages before they reach the Core — encrypting each with AES-GCM using keys generated fresh on startup [7]. It targets development threats such as compromised frontend dependencies, and carries caveats: on Windows external files do not load in the sandboxed iframe (scripts must be inlined at build time), and ES Modules do not load [7].

# Citations

1. [Inter-Process Communication | Tauri](/sources/tauri-ipc-overview.md)
2. [Calling Rust from the Frontend](/sources/tauri-calling-rust.md)
3. [Calling the Frontend from Rust](/sources/tauri-calling-frontend.md)
4. [State Management | Tauri](/sources/tauri-state-management.md)
5. [IPC Improvements · tauri-apps · Discussion #5690 · GitHub](/sources/tauri-discussion-ipc-improvements.md)
6. [How can I make Channel::send non-blocking? · tauri-apps · Discussion #11589 · GitHub](/sources/tauri-discussion-channel-blocking.md)
7. [Isolation Pattern | Tauri](/sources/tauri-ipc-isolation.md)
