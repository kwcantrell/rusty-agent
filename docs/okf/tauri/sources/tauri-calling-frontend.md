---
type: Source
title: "Calling the Frontend from Rust"
description: "Tauri API reference for Rust code to communicate with frontend via events, channels, and JavaScript evaluation."
resource: "https://v2.tauri.app/develop/calling-frontend/"
tags: [ipc-architecture]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

## Three Primary Approaches

### Event System
The built-in event mechanism for bidirectional communication works best "for situations where small amounts of data need to be streamed."

**Global Events:**
```rust
use tauri::{AppHandle, Emitter};

#[tauri::command]
fn download(app: AppHandle, url: String) {  
  app.emit("download-started", &url).unwrap();  
  for progress in [1, 15, 50, 80, 100] {    
    app.emit("download-progress", progress).unwrap();  
  }  
  app.emit("download-finished", &url).unwrap();
}
```

**Webview-Specific Events:**
```rust
use tauri::{AppHandle, Emitter};

#[tauri::command]
fn login(app: AppHandle, user: String, password: String) {  
  let authenticated = user == "tauri-apps" && password == "tauri";  
  let result = if authenticated { "loggedIn" } else { "invalidCredentials" };  
  app.emit_to("login", "login-result", result).unwrap();
}
```

### Channels
For higher-performance scenarios requiring ordered data delivery, channels handle "streaming operations such as download progress, child process output and WebSocket messages."

**Channel Implementation:**
```rust
use tauri::{AppHandle, ipc::Channel};
use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
enum DownloadEvent<'a> {
  Started {
    url: &'a str,
    download_id: usize,
    content_length: usize,
  },
  Progress {
    download_id: usize,
    chunk_length: usize,
  },
  Finished {
    download_id: usize,
  },
}

#[tauri::command]
fn download(app: AppHandle, url: String, on_event: Channel<DownloadEvent>) {  
  let content_length = 1000;  
  let download_id = 1;

  on_event.send(DownloadEvent::Started {
    url: &url,
    download_id,
    content_length,
  }).unwrap();

  for chunk_length in [15, 150, 35, 500, 300] {    
    on_event.send(DownloadEvent::Progress {
      download_id,
      chunk_length,
    }).unwrap();  
  }

  on_event.send(DownloadEvent::Finished { download_id }).unwrap();
}
```

### JavaScript Evaluation
The simplest approach for direct execution uses `WebviewWindow#eval`:

```rust
use tauri::Manager;

tauri::Builder::default()  
  .setup(|app| {
    let webview = app.get_webview_window("main").unwrap();
    webview.eval("console.log('hello from Rust')")?;
    Ok(())  
  })
```

## Communication Strategy
- Use **Events** for small, infrequent data or lifecycle notifications
- Use **Channels** for high-volume streaming (downloads, logs, WebSocket data)
- Use **JavaScript Evaluation** only for simple, immediate frontend state changes
