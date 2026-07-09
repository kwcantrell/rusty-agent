---
type: Source
title: "Inter-Process Communication | Tauri"
description: "Comprehensive overview of Tauri's asynchronous message-passing IPC model and its two core primitives."
resource: "https://v2.tauri.app/concept/inter-process-communication/"
tags: [ipc-architecture]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

## Core Philosophy
IPC "allows isolated processes to communicate securely" and represents the "key to building more complex applications." Tauri implements Asynchronous Message Passing, where processes exchange serialized requests and responses. This approach is "safer technique than shared memory or direct function access because the recipient is free to reject or discard requests." The Tauri Core can dismiss potentially malicious requests without execution.

## Two IPC Primitives

### Events
Fire-and-forget, one-way messages suited for lifecycle events and state changes. Both the Frontend and Tauri Core can emit events bidirectionally. Events support:
- Global broadcasting to all webviews
- Webview-specific targeting
- Lifecycle event handling
- State change notifications

### Commands
A foreign function interface-like abstraction using IPC messages underneath. The primary API is `invoke`, comparable to the browser's `fetch` API, allowing the Frontend to invoke Rust functions, pass arguments, and receive data. Commands support:
- Type-safe function invocation from frontend
- Arguments and return values
- Asynchronous operations
- Error handling with Result semantics
- Channel-based streaming for high-performance data transfer

## Protocol Details
Commands use a "JSON-RPC like protocol under the hood to serialize requests and responses," meaning all arguments and return data must be JSON-serializable.

## Message Flow Architecture
The documentation uses sequence diagrams to illustrate:
- Event message flows between Webview Frontend and Core Backend (bidirectional)
- Command request/response patterns (frontend initiates, backend responds)
- The asynchronous, non-blocking nature of all IPC communication
