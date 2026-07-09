---
type: Source
title: "Calling Rust from the Frontend"
description: "Tauri API reference for invoking Rust functions from JavaScript with examples of commands, events, and streaming."
resource: "https://v2.tauri.app/develop/calling-rust/"
tags: [ipc-architecture]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

## Communication Methods

### Commands System (Recommended)
Commands represent the primary approach for calling Rust functions with type safety. Define commands using the `#[tauri::command]` attribute:

```rust
#[tauri::command]
fn my_custom_command() {
  println!("I was invoked from JavaScript!");
}
```

Invoke from JavaScript via the `invoke` API:
```javascript
import { invoke } from '@tauri-apps/api/core';
invoke('my_custom_command');
```

### Command Features

**Arguments and Return Values:**
```rust
#[tauri::command]
fn my_custom_command(invoke_message: String) {
  println!("Message: {}", invoke_message);
}
```

**Async Operations:**
```rust
#[tauri::command]
async fn my_custom_command(value: String) -> String {
  some_async_function().await;
  value
}
```

**Error Handling:**
```rust
#[tauri::command]
fn login(user: String, password: String) -> Result<String, String> {
  if user == "tauri" && password == "tauri" {
    Ok("logged_in".to_string())
  } else {
    Err("invalid credentials".to_string())
  }
}
```

**Accessing WebviewWindow:**
```rust
#[tauri::command]
async fn my_custom_command(webview_window: tauri::WebviewWindow) {
  println!("WebviewWindow: {}", webview_window.label());
}
```

**State Management:**
```rust
struct MyState(String);

#[tauri::command]
fn my_custom_command(state: tauri::State<MyState>) {
  assert_eq!(state.0 == "some state value", true);
}
```

**Streaming Data via Channels:**
```rust
#[tauri::command]
async fn load_image(path: std::path::PathBuf, reader: tauri::ipc::Channel<&[u8]>) {
  let mut file = tokio::fs::File::open(path).await.unwrap();
  let mut chunk = vec![0; 4096];
  loop {
    let len = file.read(&mut chunk).await.unwrap();
    if len == 0 { break; }
    reader.send(&chunk).unwrap();
  }
}
```

### Event System
A simpler, less type-safe alternative supporting:
- JSON-only payloads
- Global and webview-specific targeting
- One-way communication

**Emit from Rust:**
```rust
use tauri::{AppHandle, Emitter};
app.emit("file-selected", "/path/to/file");
```

**Listen from JavaScript:**
```javascript
import { listen } from '@tauri-apps/api/event';

listen('download-started', (event) => {
  console.log(`Downloading ${event.payload.contentLength} bytes`);
});
```

## Command Registration
Register commands in the builder before running the application:
```rust
tauri::Builder::default()
  .invoke_handler(tauri::generate_handler![my_custom_command])
  .run(tauri::generate_context!())
  .expect("error while running tauri application")
```
