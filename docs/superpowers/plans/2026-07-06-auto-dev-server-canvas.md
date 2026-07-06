# Auto-launched dev server in the Design canvas — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the Tauri desktop harness detect a dev script in the workspace's `package.json`, launch that dev server on one click, render its live URL into the Design canvas, and let the user annotate the live preview by holding Shift while driving it.

**Architecture:** A new `DevServerManager` in `src-tauri` owns at most one child dev-server process (spawned via `tokio::process` in its own process group). Four Tauri commands expose detect/start/stop/status. The web launcher in `DesignPane` feeds the discovered URL into the *existing* `store.addUrlVersion(url)` seam, so all canvas rendering, the localhost guard, versioning, and pin feedback are reused untouched. Shift-to-annotate is a derived-state change to the canvas `passthrough` flag.

**Tech Stack:** Rust, Tauri 2, `tokio::process`, `libc` (Unix process-group kill), React 19 + TypeScript, Vitest.

## Global Constraints

- **Desktop-only.** Web launcher is `isTauri()`-gated (`web/src/transport.ts` `isTauri()`); commands live only in the `all_handlers!` macro in `src-tauri/src/lib.rs`. The browser-via-Worker path exposes no spawn surface.
- **One managed server at a time.** A new `start` stops the previous one first.
- **No arbitrary commands.** Only `<package_manager> run <script>` where `script` was detected in a `package.json` under the current workspace. Package manager is inferred from the nearest lockfile, never from UI input.
- **Guard unchanged.** The iframe is still gated by `isLocalUrl` (`web/src/components/inspector/urlGuard.ts`) even though we produced the URL.
- **URL discovery = parse stdout/stderr**, matching `https?://(localhost|127.0.0.1):<port>`. Strip ANSI escapes before matching (Vite injects color codes *inside* the URL).
- **Conventional commits**, e.g. `feat(desktop): …`, `feat(web): …`. Two separate Cargo workspaces — `src-tauri/` is its own; run `cargo` from `src-tauri/` for these Rust tasks.
- Spec: `docs/superpowers/specs/2026-07-06-auto-dev-server-canvas-design.md`.

## File Structure

**Rust (`src-tauri/`)**
- Create `src-tauri/src/devserver.rs` — `DevScriptCandidate`, `DevServerStatus`, `detect()`, `strip_ansi()`, `parse_url()`, `DevServerManager` (spawn/stop/status). Unit tests inline (`#[cfg(test)]`).
- Modify `src-tauri/Cargo.toml` — add `libc = "0.2"`.
- Modify `src-tauri/src/lib.rs` — `mod devserver;`, add manager to `AppState`, four commands, register in `all_handlers!`, teardown on workspace change + window close.

**Web (`web/`)**
- Create `web/src/components/design/devServer.ts` — typed invoke wrappers + TS types.
- Modify `web/src/components/design/DesignPane.tsx` — launcher row (detect on mount, picker, start→`addUrlVersion`, stop/restart).
- Modify `web/src/components/design/DesignCanvas.tsx` — Shift-hold → `passthrough` derivation + blur reset.
- Tests: `web/src/components/design/devServer.test.ts` (new), extend `DesignPane.test.tsx`, extend `DesignCanvas.test.tsx`.

---

### Task 1: Dev-script detection + ranking (pure)

**Files:**
- Create: `src-tauri/src/devserver.rs`
- Modify: `src-tauri/src/lib.rs:1` (add `pub mod devserver;`)

**Interfaces:**
- Produces:
  - `pub struct DevScriptCandidate { pub dir: String, pub script: String, pub package_manager: String, pub label: String }` (derives `Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize`)
  - `pub fn detect(root: &std::path::Path) -> Vec<DevScriptCandidate>` — ranked best-first.

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/devserver.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    #[test]
    fn detect_ranks_dev_first_and_infers_pm() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // root package.json: a "start" script, no lockfile -> npm
        write(root, "package.json", r#"{"scripts":{"start":"serve ."}}"#);
        // web/ package.json: a "dev" script + pnpm lockfile
        write(root, "web/package.json", r#"{"scripts":{"dev":"vite","build":"vite build"}}"#);
        write(root, "web/pnpm-lock.yaml", "lockfileVersion: 9\n");
        // node_modules must be ignored
        write(root, "node_modules/pkg/package.json", r#"{"scripts":{"dev":"nope"}}"#);

        let got = detect(root);

        // Two real candidates; node_modules ignored.
        assert_eq!(got.len(), 2, "got: {got:?}");
        // "dev" ranks before "start".
        assert_eq!(got[0].script, "dev");
        assert_eq!(got[0].package_manager, "pnpm");
        assert!(got[0].dir.ends_with("web"), "dir: {}", got[0].dir);
        assert_eq!(got[0].label, "web — dev");
        assert_eq!(got[1].script, "start");
        assert_eq!(got[1].package_manager, "npm");
    }

    #[test]
    fn detect_only_offers_server_scripts() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "package.json",
            r#"{"scripts":{"build":"vite build","test":"vitest","lint":"eslint ."}}"#);
        assert!(detect(tmp.path()).is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p rust-agent-runtime-desktop-lib devserver::tests::detect`
Expected: FAIL to compile — `detect` / `DevScriptCandidate` not defined.

- [ ] **Step 3: Write minimal implementation**

Prepend to `src-tauri/src/devserver.rs` (above the test module):

```rust
//! Detects and manages a single local dev server for the Design canvas.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Scripts we treat as "start a dev server". Ranked so `dev` wins ties.
const SERVER_SCRIPTS: &[&str] = &["dev", "start", "serve", "storybook", "preview"];
/// Directories never worth walking into.
const SKIP_DIRS: &[&str] = &["node_modules", ".git", "target", "dist", ".next"];
const MAX_DEPTH: usize = 3;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DevScriptCandidate {
    pub dir: String,
    pub script: String,
    pub package_manager: String,
    pub label: String,
}

/// Nearest lockfile from `dir` upward to (and including) `root` picks the pm.
fn package_manager(dir: &Path, root: &Path) -> String {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if d.join("pnpm-lock.yaml").exists() { return "pnpm".into(); }
        if d.join("yarn.lock").exists() { return "yarn".into(); }
        if d.join("package-lock.json").exists() { return "npm".into(); }
        if d == root { break; }
        cur = d.parent();
    }
    "npm".into()
}

fn label(dir: &Path, root: &Path, script: &str) -> String {
    let rel = dir.strip_prefix(root).ok().and_then(|p| {
        let s = p.to_string_lossy();
        if s.is_empty() { root.file_name().map(|n| n.to_string_lossy().into_owned()) }
        else { Some(s.into_owned()) }
    }).unwrap_or_else(|| dir.to_string_lossy().into_owned());
    format!("{rel} — {script}")
}

fn read_scripts(pkg: &Path) -> Vec<String> {
    let Ok(body) = std::fs::read_to_string(pkg) else { return vec![] };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) else { return vec![] };
    let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) else { return vec![] };
    SERVER_SCRIPTS.iter()
        .filter(|s| scripts.contains_key(**s))
        .map(|s| s.to_string())
        .collect()
}

fn walk(dir: &Path, root: &Path, depth: usize, out: &mut Vec<DevScriptCandidate>) {
    if depth > MAX_DEPTH { return; }
    let pkg = dir.join("package.json");
    if pkg.exists() {
        for script in read_scripts(&pkg) {
            out.push(DevScriptCandidate {
                dir: dir.to_string_lossy().into_owned(),
                package_manager: package_manager(dir, root),
                label: label(dir, root, &script),
                script,
            });
        }
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) { continue; }
        walk(&path, root, depth + 1, out);
    }
}

/// Rank: `dev`-named first, then shallower dir, then dir/script alpha.
fn rank_key(c: &DevScriptCandidate) -> (u8, usize, String, String) {
    let dev_first = if c.script == "dev" { 0 } else { 1 };
    let depth = Path::new(&c.dir).components().count();
    (dev_first, depth, c.dir.clone(), c.script.clone())
}

pub fn detect(root: &Path) -> Vec<DevScriptCandidate> {
    let root = root.canonicalize().unwrap_or_else(|_| PathBuf::from(root));
    let mut out = Vec::new();
    walk(&root, &root, 0, &mut out);
    out.sort_by_key(rank_key);
    out
}
```

Then add to the TOP of `src-tauri/src/lib.rs` (with the other `mod` lines near line 1):

```rust
pub mod devserver;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p rust-agent-runtime-desktop-lib devserver::tests::detect`
Expected: PASS (both `detect_*` tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/devserver.rs src-tauri/src/lib.rs
git commit -m "feat(desktop): detect and rank workspace dev scripts"
```

---

### Task 2: ANSI strip + localhost URL parsing (pure)

**Files:**
- Modify: `src-tauri/src/devserver.rs`

**Interfaces:**
- Produces:
  - `pub fn strip_ansi(s: &str) -> String`
  - `pub fn parse_url(line: &str) -> Option<String>` — returns the first `http(s)://localhost|127.0.0.1[:port][/path]` in the line, ANSI-stripped, trailing punctuation removed.

- [ ] **Step 1: Write the failing test**

Add inside the existing `mod tests`:

```rust
    #[test]
    fn parse_url_handles_vite_ansi_colored_line() {
        // Vite splits the port out of the URL with a bold ANSI code.
        let line = "  \u{1b}[32m➜\u{1b}[39m  \u{1b}[1mLocal\u{1b}[22m:   \
                    \u{1b}[36mhttp://localhost:\u{1b}[1m5173\u{1b}[22m\u{1b}[36m/\u{1b}[39m";
        assert_eq!(parse_url(line).as_deref(), Some("http://localhost:5173/"));
    }

    #[test]
    fn parse_url_matches_plain_and_loopback() {
        assert_eq!(parse_url("Local:   http://localhost:3000").as_deref(),
                   Some("http://localhost:3000"));
        assert_eq!(parse_url("On Your Network: http://127.0.0.1:8080/app").as_deref(),
                   Some("http://127.0.0.1:8080/app"));
    }

    #[test]
    fn parse_url_ignores_non_local_and_noise() {
        assert_eq!(parse_url("VITE ready in 240 ms"), None);
        assert_eq!(parse_url("Network: http://192.168.1.5:5173/"), None);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p rust-agent-runtime-desktop-lib devserver::tests::parse_url`
Expected: FAIL to compile — `parse_url` not defined.

- [ ] **Step 3: Write minimal implementation**

Add to `src-tauri/src/devserver.rs` (above the test module):

```rust
/// Remove CSI escape sequences (`ESC [ ... <final>`), e.g. color codes.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            // Skip until the final byte (0x40..=0x7E), e.g. 'm'.
            while let Some(&n) = chars.peek() {
                chars.next();
                if ('\u{40}'..='\u{7e}').contains(&n) { break; }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// First `http(s)://localhost|127.0.0.1` URL on the line, or None.
pub fn parse_url(line: &str) -> Option<String> {
    let clean = strip_ansi(line);
    for scheme in ["http://", "https://"] {
        let Some(start) = clean.find(scheme) else { continue };
        let rest = &clean[start..];
        // URL ends at the first whitespace.
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let url = rest[..end].trim_end_matches(['.', ',', ')', '"', '\'']);
        let host = &url[scheme.len()..];
        if host.starts_with("localhost") || host.starts_with("127.0.0.1") {
            return Some(url.to_string());
        }
    }
    None
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p rust-agent-runtime-desktop-lib devserver::tests::parse_url`
Expected: PASS (all three `parse_url_*` tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/devserver.rs
git commit -m "feat(desktop): parse localhost URL from ANSI-colored dev-server output"
```

---

### Task 3: DevServerManager — spawn, discover URL, stop without orphans

**Files:**
- Modify: `src-tauri/Cargo.toml:13-24` (add `libc`)
- Modify: `src-tauri/src/devserver.rs`

**Interfaces:**
- Consumes: `DevScriptCandidate`, `parse_url` (Tasks 1–2).
- Produces:
  - `pub struct DevServerStatus { pub url: String, pub candidate: DevScriptCandidate }` (derives `Clone, Debug, Serialize, Deserialize`)
  - `pub struct DevServerManager` with `pub fn new() -> Self`, `pub async fn start(&self, cand: DevScriptCandidate) -> Result<DevServerStatus, String>`, `pub fn stop(&self)`, `pub fn status(&self) -> Option<DevServerStatus>`.

- [ ] **Step 1: Add the `libc` dependency**

In `src-tauri/Cargo.toml`, under `[dependencies]`, add:

```toml
libc = "0.2"
```

- [ ] **Step 2: Write the failing test**

Add inside `mod tests`:

```rust
    // A fake dev server: prints a Local URL, then stays alive so we can kill it.
    fn fake_server_candidate(tmp: &Path) -> DevScriptCandidate {
        // package.json whose `dev` script runs node inline.
        write(tmp, "package.json", r#"{"scripts":{"dev":"node server.js"}}"#);
        write(tmp, "server.js",
            "console.log('  Local:   http://localhost:5199/');\nsetInterval(()=>{}, 100000);\n");
        DevScriptCandidate {
            dir: tmp.to_string_lossy().into_owned(),
            script: "dev".into(),
            package_manager: "npm".into(),
            label: "· — dev".into(),
        }
    }

    #[tokio::test]
    async fn start_discovers_url_then_stop_kills_process_group() {
        let tmp = tempfile::tempdir().unwrap();
        let cand = fake_server_candidate(tmp.path());
        let mgr = DevServerManager::new();

        let status = mgr.start(cand).await.expect("should discover URL");
        assert_eq!(status.url, "http://localhost:5199/");
        assert!(mgr.status().is_some());

        // Capture the group leader pid, then stop and confirm the group is reaped.
        let pid = mgr.running_pid().expect("running pid");
        mgr.stop();
        assert!(mgr.status().is_none());
        // Give the kernel a moment to reap, then confirm the group is gone:
        // kill(pid, 0) returns 0 while any process in the group lives, -1/ESRCH once dead.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
        assert!(!alive, "process group should be dead after stop()");
        mgr.stop(); // idempotent
    }

    #[tokio::test]
    async fn start_reports_error_when_no_url_appears() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "package.json", r#"{"scripts":{"dev":"node -e \"process.exit(1)\""}}"#);
        let cand = DevScriptCandidate {
            dir: tmp.path().to_string_lossy().into_owned(),
            script: "dev".into(), package_manager: "npm".into(), label: "x".into(),
        };
        let mgr = DevServerManager::new();
        let err = mgr.start(cand).await.unwrap_err();
        assert!(!err.is_empty(), "error should carry a message");
        assert!(mgr.status().is_none());
    }
```

> Note: these tests require `node` + `npm` on PATH. They belong to the desktop workspace's normal suite (the dev machine has both).

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p rust-agent-runtime-desktop-lib devserver::tests::start_`
Expected: FAIL to compile — `DevServerManager` / `DevServerStatus` not defined.

- [ ] **Step 4: Write minimal implementation**

Add to `src-tauri/src/devserver.rs` (above the test module). Discovery has a 30s cap; on success a background reader keeps draining the pipes (so the child never blocks on a full pipe buffer).

```rust
use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;

const DISCOVERY_TIMEOUT_SECS: u64 = 30;
const TAIL_LINES: usize = 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DevServerStatus {
    pub url: String,
    pub candidate: DevScriptCandidate,
}

struct Running {
    pid: u32, // process-group leader
    status: DevServerStatus,
    child: Child,
}

#[derive(Default)]
pub struct DevServerManager {
    current: Mutex<Option<Running>>,
}

impl DevServerManager {
    pub fn new() -> Self { Self::default() }

    pub fn status(&self) -> Option<DevServerStatus> {
        self.current.lock().unwrap().as_ref().map(|r| r.status.clone())
    }

    #[cfg(test)]
    pub(crate) fn running_pid(&self) -> Option<u32> {
        self.current.lock().unwrap().as_ref().map(|r| r.pid)
    }

    /// SIGTERM the whole process group, then SIGKILL as a backstop.
    pub fn stop(&self) {
        if let Some(mut r) = self.current.lock().unwrap().take() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(r.pid as i32), libc::SIGTERM);
                libc::kill(-(r.pid as i32), libc::SIGKILL);
            }
            let _ = r.child.start_kill();
        }
    }

    pub async fn start(&self, cand: DevScriptCandidate) -> Result<DevServerStatus, String> {
        self.stop(); // one server at a time

        let mut cmd = Command::new(&cand.package_manager);
        cmd.arg("run").arg(&cand.script)
            .current_dir(&cand.dir)
            .env("NO_COLOR", "1").env("FORCE_COLOR", "0").env("BROWSER", "none")
            .stdout(Stdio::piped()).stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0); // child becomes its own group leader (pgid == pid)

        let mut child = cmd.spawn().map_err(|e| format!("failed to launch {} run {}: {e}",
            cand.package_manager, cand.script))?;
        let pid = child.id().ok_or_else(|| "child exited before pid was available".to_string())?;

        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let tail = std::sync::Arc::new(Mutex::new(VecDeque::<String>::with_capacity(TAIL_LINES)));
        let (tx, rx) = oneshot::channel::<String>();

        // Reader task owns both pipes for the child's whole life.
        let tail_r = tail.clone();
        tokio::spawn(async move {
            let mut out = BufReader::new(stdout).lines();
            let mut err = BufReader::new(stderr).lines();
            let mut tx = Some(tx);
            loop {
                let line = tokio::select! {
                    l = out.next_line() => l, l = err.next_line() => l,
                };
                match line {
                    Ok(Some(l)) => {
                        if let Some(url) = parse_url(&l) {
                            if let Some(tx) = tx.take() { let _ = tx.send(url); }
                        }
                        let mut t = tail_r.lock().unwrap();
                        if t.len() == TAIL_LINES { t.pop_front(); }
                        t.push_back(l);
                    }
                    _ => break, // EOF or error on both streams
                }
            }
        });

        let dur = std::time::Duration::from_secs(DISCOVERY_TIMEOUT_SECS);
        match tokio::time::timeout(dur, rx).await {
            Ok(Ok(url)) => {
                let status = DevServerStatus { url, candidate: cand };
                *self.current.lock().unwrap() = Some(Running { pid, status: status.clone(), child });
                Ok(status)
            }
            _ => {
                #[cfg(unix)]
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL); }
                let _ = child.start_kill();
                let logs = tail.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n");
                Err(format!("dev server did not report a localhost URL within {DISCOVERY_TIMEOUT_SECS}s.\n{logs}"))
            }
        }
    }
}

impl Drop for DevServerManager {
    fn drop(&mut self) { self.stop(); }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p rust-agent-runtime-desktop-lib devserver::tests::start_`
Expected: PASS (both tests). If `node`/`npm` are missing the launch test errors — install Node first.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/devserver.rs
git commit -m "feat(desktop): manage one dev-server process with orphan-free teardown"
```

---

### Task 4: Tauri commands + AppState wiring + teardown

**Files:**
- Modify: `src-tauri/src/lib.rs` (AppState struct ~16-19; commands region ~59-67; `all_handlers!` ~161-183; `pick_workspace` ~96-109; builder `.setup`/`.run` ~188-215)

**Interfaces:**
- Consumes: `devserver::{DevServerManager, DevScriptCandidate, DevServerStatus}` (Tasks 1–3); `bridge.current_workspace()` (`src-tauri/src/bridge.rs:27`).
- Produces four IPC commands: `dev_scripts_detect() -> Vec<DevScriptCandidate>`, `dev_server_start(candidate) -> Result<DevServerStatus, String>`, `dev_server_stop()`, `dev_server_status() -> Option<DevServerStatus>`.

- [ ] **Step 1: Write the failing test**

Add to the `cmd_tests` module in `src-tauri/src/lib.rs` (mirrors the existing `architecture_get_returns_snapshot_over_ipc`). The mock app's workspace is an empty temp dir, so detect returns `[]`:

```rust
    /// Smoke test: `dev_scripts_detect` resolves over the mock IPC to a JSON array
    /// (empty for the temp workspace, which has no package.json).
    #[test]
    fn dev_scripts_detect_returns_array_over_ipc() {
        let app = app();
        let webview = tauri::WebviewWindowBuilder::new(&app, "dev", Default::default())
            .build()
            .unwrap();
        let res = tauri::test::get_ipc_response(
            &webview,
            tauri::webview::InvokeRequest {
                cmd: "dev_scripts_detect".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::default(),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        );
        assert!(res.is_ok(), "dev_scripts_detect should resolve: {res:?}");
        let v: serde_json::Value = res.unwrap().deserialize().unwrap();
        assert!(v.is_array(), "expected an array, got {v}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p rust-agent-runtime-desktop-lib cmd_tests::dev_scripts_detect`
Expected: FAIL — command `dev_scripts_detect` not registered (panics/errors over IPC).

- [ ] **Step 3: Write minimal implementation**

In `src-tauri/src/lib.rs`:

(a) Add the manager to `AppState` (struct near line 16):

```rust
struct AppState {
    bridge: Arc<bridge::Bridge>,
    config_path: PathBuf, // app.json (persisted workspace)
    dev: devserver::DevServerManager,
}
```

(b) Add the four commands (next to `architecture_get`, ~line 67):

```rust
#[tauri::command]
async fn dev_scripts_detect(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<devserver::DevScriptCandidate>, String> {
    let ws = state.bridge.current_workspace().await;
    Ok(devserver::detect(&ws))
}

#[tauri::command]
async fn dev_server_start(
    state: tauri::State<'_, AppState>,
    candidate: devserver::DevScriptCandidate,
) -> Result<devserver::DevServerStatus, String> {
    state.dev.start(candidate).await
}

#[tauri::command]
fn dev_server_stop(state: tauri::State<'_, AppState>) {
    state.dev.stop();
}

#[tauri::command]
fn dev_server_status(state: tauri::State<'_, AppState>) -> Option<devserver::DevServerStatus> {
    state.dev.status()
}
```

(c) Register them in `all_handlers!` (after `architecture_get,` / before `skill_save`):

```rust
            dev_scripts_detect,
            dev_server_start,
            dev_server_stop,
            dev_server_status,
```

(d) Stop the server when the workspace changes — in `pick_workspace`, right after the successful pick and before `set_workspace`:

```rust
    state.dev.stop(); // old server pointed at the previous workspace
    state.bridge.set_workspace(dir.clone()).await;
```

(e) Construct the manager in `.setup` (both the production `app.manage` at ~line 210 and the `cmd_tests::app()` `.manage` at ~line 238):

```rust
            app.manage(AppState { bridge, config_path, dev: devserver::DevServerManager::new() });
```

```rust
            .manage(AppState {
                bridge,
                config_path: PathBuf::from("/tmp/app.json"),
                dev: devserver::DevServerManager::new(),
            })
```

(f) Kill on window close — add before `.run(...)` in the builder chain (~line 213):

```rust
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                window.state::<AppState>().dev.stop();
            }
        })
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p rust-agent-runtime-desktop-lib`
Expected: PASS — including `dev_scripts_detect_returns_array_over_ipc` and all prior devserver tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(desktop): expose dev-server detect/start/stop/status commands"
```

---

### Task 5: Web invoke wrappers for the dev-server commands

**Files:**
- Create: `web/src/components/design/devServer.ts`
- Test: `web/src/components/design/devServer.test.ts`

**Interfaces:**
- Produces:
  - `export interface DevScriptCandidate { dir: string; script: string; package_manager: string; label: string }`
  - `export interface DevServerStatus { url: string; candidate: DevScriptCandidate }`
  - `export async function detectDevScripts(): Promise<DevScriptCandidate[]>`
  - `export async function startDevServer(candidate: DevScriptCandidate): Promise<DevServerStatus>`
  - `export async function stopDevServer(): Promise<void>`
  - `export async function devServerStatus(): Promise<DevServerStatus | null>`

- [ ] **Step 1: Write the failing test**

Create `web/src/components/design/devServer.test.ts`:

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { detectDevScripts, startDevServer, stopDevServer } from "./devServer";

describe("devServer wrappers", () => {
  beforeEach(() => invoke.mockReset());

  it("detect calls the dev_scripts_detect command", async () => {
    invoke.mockResolvedValueOnce([{ dir: "/w/web", script: "dev", package_manager: "pnpm", label: "web — dev" }]);
    const got = await detectDevScripts();
    expect(invoke).toHaveBeenCalledWith("dev_scripts_detect");
    expect(got[0].script).toBe("dev");
  });

  it("start passes the candidate as an argument", async () => {
    const cand = { dir: "/w/web", script: "dev", package_manager: "pnpm", label: "web — dev" };
    invoke.mockResolvedValueOnce({ url: "http://localhost:5173/", candidate: cand });
    const got = await startDevServer(cand);
    expect(invoke).toHaveBeenCalledWith("dev_server_start", { candidate: cand });
    expect(got.url).toBe("http://localhost:5173/");
  });

  it("stop calls dev_server_stop", async () => {
    invoke.mockResolvedValueOnce(undefined);
    await stopDevServer();
    expect(invoke).toHaveBeenCalledWith("dev_server_stop");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && npx vitest run src/components/design/devServer.test.ts`
Expected: FAIL — `./devServer` module not found.

- [ ] **Step 3: Write minimal implementation**

Create `web/src/components/design/devServer.ts`:

```ts
import { invoke } from "@tauri-apps/api/core";

export interface DevScriptCandidate {
  dir: string;
  script: string;
  package_manager: string;
  label: string;
}

export interface DevServerStatus {
  url: string;
  candidate: DevScriptCandidate;
}

export function detectDevScripts(): Promise<DevScriptCandidate[]> {
  return invoke<DevScriptCandidate[]>("dev_scripts_detect");
}

export function startDevServer(candidate: DevScriptCandidate): Promise<DevServerStatus> {
  return invoke<DevServerStatus>("dev_server_start", { candidate });
}

export function stopDevServer(): Promise<void> {
  return invoke("dev_server_stop");
}

export function devServerStatus(): Promise<DevServerStatus | null> {
  return invoke<DevServerStatus | null>("dev_server_status");
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd web && npx vitest run src/components/design/devServer.test.ts`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add web/src/components/design/devServer.ts web/src/components/design/devServer.test.ts
git commit -m "feat(web): typed invoke wrappers for the dev-server commands"
```

---

### Task 6: Dev-server launcher in DesignPane

**Files:**
- Modify: `web/src/components/design/DesignPane.tsx`
- Test: `web/src/components/design/DesignPane.test.tsx`

**Interfaces:**
- Consumes: `detectDevScripts`, `startDevServer`, `stopDevServer`, `DevScriptCandidate`, `DevServerStatus` (Task 5); `isTauri` (`web/src/transport.ts`); `store.addUrlVersion` + `LIVE_PREVIEW_ID` (`web/src/designStore.ts`).
- Behavior: on mount, if `isTauri()`, call `detectDevScripts()`. Render a launcher row: 0 → nothing (existing manual field remains); ≥1 → a candidate `<select>` (best-first, first pre-selected) + **Start dev server**; while a server runs → a status pill (`candidate.label` + url) with **Stop** and **Restart**. Start feeds `status.url` into `store.addUrlVersion` and selects the live-preview tab.

- [ ] **Step 1: Write the failing test**

Add to `web/src/components/design/DesignPane.test.tsx` (mock the wrappers + force Tauri):

```tsx
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";

const detectDevScripts = vi.fn();
const startDevServer = vi.fn();
const stopDevServer = vi.fn();
vi.mock("./devServer", () => ({ detectDevScripts, startDevServer, stopDevServer }));
vi.mock("../../transport", async (orig) => ({ ...(await orig()), isTauri: () => true }));

import { DesignPane } from "./DesignPane";

const cand = { dir: "/w/web", script: "dev", package_manager: "pnpm", label: "web — dev" };

describe("DesignPane dev-server launcher", () => {
  beforeEach(() => {
    detectDevScripts.mockReset(); startDevServer.mockReset(); stopDevServer.mockReset();
    detectDevScripts.mockResolvedValue([cand]);
  });

  it("starting a detected server renders it in the canvas", async () => {
    startDevServer.mockResolvedValue({ url: "http://localhost:5173/", candidate: cand });
    render(<DesignPane items={[]} sessionId="s1" onSend={() => {}} sendDisabled={false} />);

    const btn = await screen.findByRole("button", { name: /start dev server/i });
    fireEvent.click(btn);

    await waitFor(() => expect(startDevServer).toHaveBeenCalledWith(cand));
    // The live-preview iframe now exists (guard lets localhost through).
    await waitFor(() =>
      expect(screen.getByTitle(/live preview/i)).toBeInTheDocument());
    // Stop control appears once running.
    expect(screen.getByRole("button", { name: /stop/i })).toBeInTheDocument();
  });
});
```

> If the canvas iframe has no `title`, assert on the status pill text `web — dev` instead — `expect(screen.getByText(/web — dev/)).toBeInTheDocument()`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && npx vitest run src/components/design/DesignPane.test.tsx`
Expected: FAIL — no "Start dev server" button.

- [ ] **Step 3: Write minimal implementation**

Edit `web/src/components/design/DesignPane.tsx`. Add imports:

```tsx
import { useEffect, useState } from "react";
import { isTauri } from "../../transport";
import {
  detectDevScripts, startDevServer, stopDevServer,
  type DevScriptCandidate, type DevServerStatus,
} from "./devServer";
```

Inside `DesignPane`, after the existing `useState` hooks, add launcher state + effects:

```tsx
  const [candidates, setCandidates] = useState<DevScriptCandidate[]>([]);
  const [picked, setPicked] = useState(0);
  const [running, setRunning] = useState<DevServerStatus | null>(null);
  const [devError, setDevError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!isTauri()) return;
    detectDevScripts().then(setCandidates).catch(() => setCandidates([]));
  }, []);

  const launch = async (cand: DevScriptCandidate) => {
    setBusy(true); setDevError(null);
    try {
      const status = await startDevServer(cand);
      store.addUrlVersion(status.url);
      setActiveId(LIVE_PREVIEW_ID);
      setRunning(status);
    } catch (e) {
      setDevError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const stop = async () => { await stopDevServer().catch(() => {}); setRunning(null); };
```

Then render the launcher row above the manual URL block (inside the top `px-2 pt-2` div, before the manual field). Replace the opening of that block:

```tsx
      <div className="px-2 pt-2">
        {candidates.length > 0 && !running && (
          <div className="flex gap-1 pb-1">
            {candidates.length > 1 && (
              <select aria-label="dev script" value={picked}
                onChange={(e) => setPicked(Number(e.target.value))}
                className="min-w-0 flex-1 rounded px-2 py-1 text-xs"
                style={{ background: "var(--surface-base)", color: "var(--text-strong)",
                  border: "1px solid var(--border)" }}>
                {candidates.map((c, i) => <option key={c.dir + c.script} value={i}>{c.label}</option>)}
              </select>
            )}
            <button onClick={() => launch(candidates[picked])} disabled={busy}
              className="rounded px-2 py-1 text-xs disabled:opacity-40"
              style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>
              {busy ? "Starting…" : candidates.length > 1
                ? "Start dev server" : `Start dev server (${candidates[0].label})`}
            </button>
          </div>
        )}
        {running && (
          <div className="flex items-center gap-2 pb-1 text-xs" style={{ color: "var(--text-muted)" }}>
            <span className="min-w-0 flex-1 truncate">▶ {running.candidate.label} — {running.url}</span>
            <button onClick={() => launch(running.candidate)} disabled={busy}
              className="rounded px-2 py-0.5" style={{ border: "1px solid var(--border)" }}>Restart</button>
            <button onClick={stop} className="rounded px-2 py-0.5"
              style={{ border: "1px solid var(--border)" }}>Stop</button>
          </div>
        )}
        {devError && <p className="pb-1 text-xs" style={{ color: "var(--text-muted)" }}>{devError}</p>}
        <div className="flex gap-1">
```

(The existing manual-URL `<input>`/`Preview` button and its closing `</div>` stay exactly as they were — you are only inserting the launcher block and the `<div className="flex gap-1">` wrapper line already present.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd web && npx vitest run src/components/design/DesignPane.test.tsx`
Expected: PASS, including the new launcher test and the existing DesignPane tests.

- [ ] **Step 5: Typecheck**

Run: `cd web && npm run typecheck`
Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add web/src/components/design/DesignPane.tsx web/src/components/design/DesignPane.test.tsx
git commit -m "feat(web): one-click dev-server launcher feeds the live-preview canvas"
```

---

### Task 7: Shift-to-annotate over the live preview

**Files:**
- Modify: `web/src/components/design/DesignCanvas.tsx`
- Test: `web/src/components/design/DesignCanvas.test.tsx`

**Interfaces:**
- Consumes: existing `AnnotationOverlay` `passthrough` prop (`web/src/components/design/AnnotationOverlay.tsx`).
- Behavior: while a live URL is shown in Interact mode, holding **Shift** suppresses iframe passthrough so a click drops a pin; releasing Shift (or the window losing focus) restores driving.

- [ ] **Step 1: Write the failing test**

Add to `web/src/components/design/DesignCanvas.test.tsx` a case that renders a Url design in Interact mode and asserts Shift toggles the pin layer's pointer-events. Use the existing test helpers in that file for building a `Url` design; the key assertions:

```tsx
import { fireEvent } from "@testing-library/react";
// ...inside describe:
  it("holding Shift lets you pin while interacting with a live URL", () => {
    // Build a design whose latest version is a Url display, render it, enter Interact mode.
    renderUrlCanvasInteract(); // helper: renders DesignCanvas on a Url design + clicks "Interact"
    const layer = screen.getByTestId("pin-layer");
    // Interact mode: overlay passes through (pointer-events: none).
    expect(layer).toHaveStyle({ pointerEvents: "none" });
    // Hold Shift: overlay re-captures clicks.
    fireEvent.keyDown(window, { key: "Shift" });
    expect(layer).not.toHaveStyle({ pointerEvents: "none" });
    // Release Shift: back to passthrough.
    fireEvent.keyUp(window, { key: "Shift" });
    expect(layer).toHaveStyle({ pointerEvents: "none" });
    // Window blur clears a stuck Shift.
    fireEvent.keyDown(window, { key: "Shift" });
    fireEvent(window, new Event("blur"));
    expect(layer).toHaveStyle({ pointerEvents: "none" });
  });
```

> If no `renderUrlCanvasInteract` helper exists, inline it: build `design = { id: "design:live-preview", title: "Live preview", versions: [{ display: { Url: { url: "http://localhost:5173/", id: "design:live-preview", title: "Live preview" } }, renderable: true }] }`, render `<DesignCanvas design={design} sentPins={() => []} onSendFeedback={() => {}} sendDisabled={false} />`, then `fireEvent.click(screen.getByRole("button", { name: /interact/i }))`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && npx vitest run src/components/design/DesignCanvas.test.tsx`
Expected: FAIL — Shift keydown does not change `pointer-events` (passthrough is currently `interact` only).

- [ ] **Step 3: Write minimal implementation**

Edit `web/src/components/design/DesignCanvas.tsx`. Add `useEffect` to the import and a Shift tracker; derive passthrough with `!shiftHeld`.

Change the import line:

```tsx
import { useEffect, useState } from "react";
```

Add inside the component (after the existing `useState` hooks):

```tsx
  const [shiftHeld, setShiftHeld] = useState(false);
  useEffect(() => {
    const down = (e: KeyboardEvent) => { if (e.key === "Shift") setShiftHeld(true); };
    const up = (e: KeyboardEvent) => { if (e.key === "Shift") setShiftHeld(false); };
    const clear = () => setShiftHeld(false); // keyup can be swallowed by the iframe
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", clear);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", clear);
    };
  }, []);
```

Change the `AnnotationOverlay` usage's `passthrough` prop:

```tsx
            passthrough={!!liveUrl && interact && !shiftHeld}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd web && npx vitest run src/components/design/DesignCanvas.test.tsx`
Expected: PASS, including the new Shift test and existing canvas tests.

- [ ] **Step 5: Full gate**

Run: `cd web && npm run typecheck && npx vitest run` then `cd ../src-tauri && cargo test`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add web/src/components/design/DesignCanvas.tsx web/src/components/design/DesignCanvas.test.tsx
git commit -m "feat(web): hold Shift to pin feedback while driving the live preview"
```

---

## Self-Review

**Spec coverage:**
- One-click, auto-detected launch → Task 1 (detect) + Task 6 (launcher, no auto-spawn). ✓
- Parse stdout for URL → Task 2 (parse) + Task 3 (streamed discovery). ✓
- Ranked picker for multiple candidates → Task 1 (rank) + Task 6 (`<select>`). ✓
- One server, explicit Stop/Restart, kill on quit + workspace change → Task 3 (stop/Drop) + Task 4 (window Destroyed + `pick_workspace`) + Task 6 (Stop/Restart UI). ✓
- Shift-click always drops a pin → Task 7. ✓
- Reuse `addUrlVersion` + guard → Task 6 (`store.addUrlVersion`), guard is unchanged by construction. ✓
- Desktop-only → Task 6 `isTauri()` gate + Task 4 commands only in `all_handlers!`. ✓
- Error handling (spawn fail / URL timeout) → Task 3 (`Err` with log tail) + Task 6 (`devError` display). ✓
- Constrained command (pm from lockfile, script from detection) → Task 1 (`package_manager`) + Task 3 (`Command::new(pm).arg("run").arg(script)`). ✓
- ANSI-in-URL gotcha → Task 2 (`strip_ansi`) + `NO_COLOR` env in Task 3. ✓

**Placeholder scan:** No TBD/TODO; every code step carries complete code. The two "if the helper doesn't exist / if the iframe has no title" notes are explicit fallbacks with concrete inline code, not deferrals.

**Type consistency:** `DevScriptCandidate` / `DevServerStatus` field names (`dir`, `script`, `package_manager`, `label`, `url`, `candidate`) are identical across Rust (Tasks 1/3), the IPC boundary (Task 4), and TS (Task 5). Command names (`dev_scripts_detect`, `dev_server_start`, `dev_server_stop`, `dev_server_status`) match between `all_handlers!` (Task 4) and the wrappers (Task 5). `addUrlVersion` / `LIVE_PREVIEW_ID` match `designStore.ts`. `passthrough` matches `AnnotationOverlay`.

**Out of scope (from spec):** multiple concurrent servers, auto-restart on crash, full streaming console, persisted per-workspace choice — none are introduced by any task.
