---
type: Practice
title: State management and the Channel::send gotcha
description: Share mutable state across Tauri commands and threads via the Manager API — preferring std Mutex, avoiding Arc, dodging the runtime-panic type mismatch, and knowing that Channel::send can block the caller.
tags: [ipc-architecture, performance]
timestamp: 2026-07-09T00:00:00Z
---

# State management and the Channel::send gotcha

Tauri manages shared application state through its `Manager` API: register state
once during setup with `app.manage(...)`, then access it in any command via a
`State<T>` parameter, or from event handlers and threads via `AppHandle` [1].
Getting the mutability and typing right avoids two real footguns — a runtime panic
and a needless `Arc` — and the streaming path has its own blocking caveat.

## Prefer std Mutex; do not reach for Arc

Shared *mutable* state needs interior mutability, and the standard-library `Mutex`
is the recommended default — it "takes precedence over async mutexes in most
cases" [1]. Register it with `app.manage(Mutex::new(AppState::default()))` [1].
Per the Tokio guidance the docs quote, "it is ok and often preferred to use the
ordinary Mutex from the standard library in asynchronous code," except when you
hold a mutex guard across an `await` point [1]. Do **not** wrap state in
`Arc<Mutex<T>>`: the `State` wrapper already provides shared ownership, so the
framework handles that for you and `Arc` is redundant [1].

```rust
Builder::default().setup(|app| {
  app.manage(Mutex::new(AppState::default()));
  Ok(())
});
```

## Sync vs async access

In a synchronous command, take `State<'_, Mutex<AppState>>` and `lock().unwrap()`
[1]. In an async command, use an async mutex's `lock().await` and return a `Result`
so the boundary can surface errors [1]. Outside commands — window-event handlers,
spawned threads — go through the `Manager` trait on an `AppHandle`:
`app_handle.state::<Mutex<AppState>>()` [1]:

```rust
#[tauri::command]
fn increase(state: State<'_, Mutex<AppState>>) -> u32 {
  let mut s = state.lock().unwrap();
  s.counter += 1;
  s.counter
}
```

## The runtime-panic trap

The sharpest gotcha: a type mismatch in a `State` parameter panics at *runtime*,
not compile time [1]. Declaring `State<AppState>` when the managed value is
`Mutex<AppState>` compiles fine and then panics when the command runs [1]. Defend
against it with a type alias for the managed type and use that alias everywhere you
name the state, so the wrapped shape can never drift between `manage` and the
command signature [1].

## Channel::send can block the caller

Channels are the high-throughput streaming primitive
([/practices/command-design.md](/practices/command-design.md)), but `Channel::send`
is not free. A user reported it blocking the calling thread for 30–50 ms when
sending ~3 MB video frames (YUV420P), which disrupted the Tokio executor in a
decode-and-render pipeline [2]. Treat large-payload `send` as potentially blocking:
budget for it on hot paths, and consider a custom URI-scheme protocol for very
high-volume transfers rather than pushing multi-megabyte frames through the channel
([/practices/command-design.md](/practices/command-design.md)). One caveat on the
evidence — the only resolution in that thread (marking the byte slice `&'static
[u8]`) is a self-answer from the reporter with no maintainer confirmation, so treat
it as a lead to measure, not a guaranteed fix [2].

# Citations

1. [State Management](/sources/tauri-state-management.md)
2. [Channel::send blocking discussion](/sources/tauri-discussion-channel-blocking.md)
