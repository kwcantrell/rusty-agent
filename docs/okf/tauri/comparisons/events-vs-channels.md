---
type: Comparison
title: "Events vs channels vs eval: pushing data from Rust to the frontend"
description: "When to reach for the event system, IPC channels, or JavaScript evaluation to send data from the Tauri core to the webview, and the trade-offs the sources support."
tags: [ipc-architecture, performance]
timestamp: 2026-07-09T00:00:00Z
---

Tauri gives Rust three ways to push data into the frontend: the event system, IPC channels, and direct JavaScript evaluation [1]. All three ride the same asynchronous message-passing IPC layer, in which processes exchange serialized requests and responses rather than sharing memory [2]. The choice between them is a choice about volume, ordering, and how much machinery you want.

## The event system

Events are Tauri's built-in mechanism for bidirectional communication, and the docs scope them explicitly to "situations where small amounts of data need to be streamed" [1]. They are fire-and-forget, one-way messages suited to lifecycle notifications and state changes, and both the frontend and the core can emit them [2]. You can broadcast globally with `emit` or target a specific webview with `emit_to` [1].

Reach for events when the payload is small and infrequent — a download starting, a login result, a state change the whole UI should hear about [1]. Because a global emit reaches every listener, events are the natural fit when several windows or components need the same signal [2].

## Channels

Channels exist for the higher-volume case events were never meant to carry. The docs recommend them for "streaming operations such as download progress, child process output and WebSocket messages" [1], and the IPC overview lists channel-based streaming as the command layer's answer to high-performance data transfer [2]. A channel preserves ordered delivery, which matters when the frontend must reassemble a stream in sequence [1].

The trade-off is that `Channel::send` is not free. A user transmitting roughly 3MB video frames reported it blocking the calling thread for 30–50ms per send, enough to disrupt a tokio async pipeline decoding h264 to YUV420P for canvas rendering [3]. That report carries an important caveat: it is a self-answer from the question author, with no maintainer confirmation of the blocking behavior or of the proposed fix (using `&'static [u8]` to avoid borrowed references) [3]. Treat it as a signal, not a settled rule: for genuinely large or high-frequency payloads, do not assume `send` is instantaneous — measure it off the hot async path, and be aware the underlying IPC serializes all parameters and return values as strings because of webview library constraints, a ceiling channels do not remove [4].

## JavaScript evaluation

The simplest approach is `WebviewWindow#eval`, which runs a string of JavaScript directly in the webview [1]. The docs position it for the narrowest case: "simple, immediate frontend state changes" [1]. It skips the event and channel machinery entirely, but it carries no ordering guarantees, no streaming semantics, and no structured payload — you are handing the webview raw code to execute. Use it only for direct, one-off execution [1].

## Choosing

- **Events** — small or infrequent data, lifecycle notifications, signals many listeners should hear [1][2].
- **Channels** — high-volume ordered streams (downloads, logs, WebSocket data), accepting that `send` can block on large payloads and that string serialization bounds throughput [1][3][4].
- **Eval** — only simple, immediate frontend state changes where no payload structure or ordering is needed [1].

When even channels hit the serialization ceiling, the maintainer-endorsed escape hatch is to sidestep standard IPC altogether: register a custom URI scheme protocol via `register_uri_scheme_protocol` and return browser-compatible responses, with platform-specific paths such as `sharedBuffer` zero-copy messaging on Windows [4]. That is a heavier architecture and outside the three primitives, but it is where the sources point for maximum throughput [4].

# Citations

1. [Calling the Frontend from Rust](/sources/tauri-calling-frontend.md)
2. [Inter-Process Communication | Tauri](/sources/tauri-ipc-overview.md)
3. [How can I make Channel::send non-blocking? · Discussion #11589](/sources/tauri-discussion-channel-blocking.md)
4. [IPC Improvements · Discussion #5690](/sources/tauri-discussion-ipc-improvements.md)
