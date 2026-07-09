---
type: Source
title: "State Management | Tauri"
description: "Built-in state management system through Manager API with interior mutability patterns for shared state across commands and threads."
resource: https://v2.tauri.app/develop/state-management/
tags: [ipc-architecture]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---
# Summary

Tauri provides a built-in state management system through its `Manager` API, enabling developers to initialize and access application state across commands, event handlers, and threads. State is registered during application setup via `app.manage()` and accessed through the `State` type parameter in commands.

## Core Patterns

**State Registration**: State is managed during app setup using the Builder pattern:
```rust
Builder::default()
  .setup(|app| {
    app.manage(AppData {
      welcome_message: "Welcome to Tauri!",
    });
    Ok(())
  })
```

**Interior Mutability Requirements**: Shared mutable state requires interior mutability. The standard library's `Mutex` is the recommended approach and takes precedence over async mutexes in most cases:
```rust
app.manage(Mutex::new(AppState::default()));
```

Per Tokio guidance quoted in the documentation: "it is ok and often preferred to use the ordinary Mutex from the standard library in asynchronous code" except when holding mutex guards across await points.

**Arc Not Required**: The `State` wrapper already provides shared ownership; developers don't need `Arc<Mutex<T>>` patterns—the framework handles ownership internally.

## Accessing State

**In Synchronous Commands**:
```rust
#[tauri::command]
fn increase_counter(state: State<'_, Mutex<AppState>>) -> u32 {
  let mut state = state.lock().unwrap();
  state.counter += 1;
  state.counter
}
```

**In Async Commands**: Use async mutex methods and `Result` return types:
```rust
#[tauri::command]
async fn increase_counter(state: State<'_, Mutex<AppState>>) -> Result<u32, ()> {
  let mut state = state.lock().await;
  state.counter += 1;
  Ok(state.counter)
}
```

**Via Manager Trait** (event handlers, threads): Use `AppHandle`:
```rust
fn on_window_event(window: &Window, _event: &WindowEvent) {
  let app_handle = window.app_handle();
  let state = app_handle.state::<Mutex<AppState>>();
  let mut state = state.lock().unwrap();
  state.counter += 1;
}
```

## Critical Caveat

Type mismatches in `State` parameters cause runtime panics rather than compile-time errors. Using `State<AppState>` instead of `State<Mutex<AppState>>` will fail at runtime. The guide recommends using type aliases to prevent this mistake.
