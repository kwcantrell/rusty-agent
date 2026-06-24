# IPC: commands, events, and state

Tauri's IPC has two directions: **commands** (frontend → Rust, request/response)
and **events** (either direction, fire-and-forget broadcast). Shared backend data
lives in **managed state**.

## Commands (frontend → Rust)

### Define and register

```rust
// src-tauri/src/lib.rs
#[tauri::command]
fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub fn run() {
    tauri::Builder::default()
        // EVERY command must be listed here, or invoke() fails at runtime.
        .invoke_handler(tauri::generate_handler![add])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Multiple commands: `generate_handler![add, greet, save_file]`.

### Call from the frontend

```js
import { invoke } from '@tauri-apps/api/core';

const sum = await invoke('add', { a: 2, b: 3 }); // => 5
```

`invoke(commandName, args)` returns a Promise. `args` is a single object whose
keys are the command's parameter names.

### Argument naming: camelCase ↔ snake_case

By default JS `camelCase` keys map to Rust `snake_case` params:

```rust
#[tauri::command]
fn open_file(file_path: String) {}
```
```js
await invoke('open_file', { filePath: '/tmp/x.txt' }); // filePath → file_path
```

To keep an exact JS name, override the rename strategy:
`#[tauri::command(rename_all = "snake_case")]` then pass `{ file_path: ... }`.

### Returning values and errors

Return any `serde::Serialize` type to resolve the promise. Return
`Result<T, E>` where `E: Serialize` to **reject** it. `String` is the simplest
error; a custom error enum is better:

```rust
#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// Serialize the error so the frontend receives a useful message.
impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.to_string().as_ref())
    }
}

#[tauri::command]
fn read(path: String) -> Result<String, Error> {
    Ok(std::fs::read_to_string(path)?)
}
```
```js
try { const text = await invoke('read', { path }); }
catch (err) { /* err is the serialized error */ }
```

### Async commands

Mark `async fn` (or return a `Future`) to avoid blocking the main thread. Async
commands run on a separate thread; this is required for long-running work and for
borrowing `State` across `.await`:

```rust
#[tauri::command]
async fn fetch(url: String) -> Result<String, Error> { /* ... */ Ok(String::new()) }
```

### Injected parameters

Tauri injects these by type — they don't appear in the JS `args`:
- `app: tauri::AppHandle` — clone-cheap handle to the app (emit events, access
  state, manage windows from anywhere).
- `window: tauri::Window` / `webview_window: tauri::WebviewWindow` — the caller's
  window.
- `state: tauri::State<'_, T>` — managed state of type `T` (see below).

```rust
#[tauri::command]
fn whoami(app: tauri::AppHandle, window: tauri::Window) -> String {
    format!("{} on window {}", app.package_info().name, window.label())
}
```

## State management

Register shared state once with `.manage()`, then request it by type. State must
be `Send + Sync`; use interior mutability (`Mutex`/`RwLock`) to mutate:

```rust
use std::sync::Mutex;

struct Counter(Mutex<i32>);

#[tauri::command]
fn increment(state: tauri::State<'_, Counter>) -> i32 {
    let mut n = state.0.lock().unwrap();
    *n += 1;
    *n
}

pub fn run() {
    tauri::Builder::default()
        .manage(Counter(Mutex::new(0)))
        .invoke_handler(tauri::generate_handler![increment])
        .run(tauri::generate_context!())
        .unwrap();
}
```

Access state outside a command via `app_handle.state::<Counter>()`. `manage`
stores one instance per type; calling it twice for the same type panics.

## Events (broadcast, either direction)

Events are untyped JSON payloads delivered to listeners — good for progress,
notifications, and backend-initiated updates.

### Rust → frontend

```rust
use tauri::Emitter;
app.emit("download-progress", 42).unwrap();          // to all listeners
window.emit_to("main", "download-progress", 42).unwrap(); // to one window
```
```js
import { listen } from '@tauri-apps/api/event';
const unlisten = await listen('download-progress', (e) => console.log(e.payload));
// later: unlisten();
```

### Frontend → Rust

```js
import { emit } from '@tauri-apps/api/event';
await emit('user-action', { kind: 'save' });
```
```rust
use tauri::Listener;
app.listen("user-action", |event| { /* event.payload() is a JSON string */ });
```

Commands vs events: use a **command** when you need a return value or
back-pressure (request/response); use an **event** for one-to-many or
fire-and-forget signaling.

## Calling from the frontend without a bundler

If the frontend has no module bundler, set `app.withGlobalTauri: true` in
`tauri.conf.json` to expose the API on `window.__TAURI__` (e.g.
`window.__TAURI__.core.invoke`). Otherwise import from `@tauri-apps/api/*`.
