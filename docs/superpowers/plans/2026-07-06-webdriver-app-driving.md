# WebDriver-Based Desktop App Driving Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace pixel-level xdotool driving of the Tauri desktop app with DOM-level WebDriver automation (`tauri-driver` + WebKitWebDriver), as a checked-in e2e suite plus an updated interactive-driving skill.

**Architecture:** `tauri-driver` (cargo-installed shim) speaks standard WebDriver on a local port and proxies to the apt-installed `WebKitWebDriver`, which launches the existing debug binary in automation mode. A Rust harness module in `src-tauri/tests/` owns the Vite/tauri-driver/session lifecycle; tests drive the real app through `thirtyfour`. Interactive sessions use `/usr/bin/python3` + selenium against the same driver.

**Tech Stack:** tauri-driver, webkitgtk-webdriver (apt), thirtyfour (Rust WebDriver client), python3-selenium (interactive), existing tokio/reqwest/libc deps.

**Spec:** `docs/superpowers/specs/2026-07-06-webdriver-app-driving-design.md`

## Global Constraints

- Do NOT modify `scripts/ci.sh` — src-tauri is not part of the CI gate.
- Both new tests are `#[ignore]`d (need a display + Vite; turn test also needs llama on :8080). Run: `cd src-tauri && cargo test --test gui_smoke -- --ignored --test-threads=1 --nocapture`.
- `src-tauri/` is its own Cargo workspace — run cargo from `src-tauri/`, not `agent/`. If `cargo` is missing from PATH: `source ~/.cargo/env`.
- The debug binary loads its frontend from Vite `http://localhost:5173` (see `tauri.conf.json` build.devUrl), NOT bundled assets — Vite must be up before the app renders.
- Kill spawned processes by held child PID / process group only — never `pkill -f` pattern matching (self-match kills the shell, exit 144).
- Interactive selenium uses `/usr/bin/python3` — the PATH `python3` is a uv shim without apt site-packages.
- Conventional commits: `type(scope): summary`.

---

### Task 1: System setup and manual stack proof

Install the three tools and prove the full WebDriver stack works against the real app **before** writing any Rust. No repo changes to commit in this task (Cargo.lock churn from `cargo install` does not touch the repo).

**Files:** none created/modified (system-level task).

**Interfaces:**
- Produces: `tauri-driver` at `~/.cargo/bin/tauri-driver`; `WebKitWebDriver` on PATH (`/usr/bin/WebKitWebDriver`); working `/usr/bin/python3 -c "import selenium"`. Task 2's harness assumes `tauri-driver` and `WebKitWebDriver` are both on PATH.

- [ ] **Step 1: Install system packages** (needs sudo — if the sandbox blocks it, ask the user to run it via `! sudo apt install -y webkitgtk-webdriver python3-selenium`)

```bash
sudo apt install -y webkitgtk-webdriver python3-selenium
```

Expected: both packages install. Verify:

```bash
command -v WebKitWebDriver || dpkg -L webkitgtk-webdriver | grep -m1 -i webdriver
/usr/bin/python3 -c "import selenium; print(selenium.__version__)"
dpkg -l libwebkit2gtk-4.1-0 webkitgtk-webdriver | grep ^ii   # versions must match (same source)
```

Expected: a WebKitWebDriver path prints (normally `/usr/bin/WebKitWebDriver`), selenium prints a 4.x version, and both webkit packages show the same 2.52.x version. If WebKitWebDriver is NOT on PATH, note its real path — the harness (Task 2) honors a `WEBKIT_WEBDRIVER` env var override.

- [ ] **Step 2: Install tauri-driver**

```bash
source ~/.cargo/env 2>/dev/null; cargo install tauri-driver --locked
~/.cargo/bin/tauri-driver --help
```

Expected: help text showing `--port`, `--native-port`, `--native-driver` flags.

- [ ] **Step 3: Ensure a debug binary and Vite**

```bash
cd /home/kalen/rust-agent-runtime/src-tauri && cargo build 2>&1 | tail -1
curl -s -o /dev/null -w '%{http_code}' http://localhost:5173 || setsid npm --prefix /home/kalen/rust-agent-runtime/web run dev >/tmp/claude-vite.log 2>&1 &
```

Expected: ``Finished `dev` profile`` and, after a few seconds, `curl -s -o /dev/null -w '%{http_code}' http://localhost:5173` prints `200`. (Grep the build log for `Finished`, don't process-match — rustc cmdlines false-positive.)

- [ ] **Step 4: Manual stack proof — drive the app via selenium**

```bash
setsid ~/.cargo/bin/tauri-driver --port 4444 --native-port 4445 >/tmp/claude-tauri-driver.log 2>&1 &
sleep 2; curl -s http://127.0.0.1:4444/status
```

Expected: JSON with `"ready":true`. Then:

```bash
/usr/bin/python3 - <<'EOF'
from selenium.webdriver import Remote
from selenium.webdriver.common.options import ArgOptions
from selenium.webdriver.common.by import By
o = ArgOptions()
o.set_capability("tauri:options", {"application": "/home/kalen/rust-agent-runtime/src-tauri/target/debug/rust-agent-runtime-desktop"})
o.set_capability("browserName", "wry")
d = Remote("http://127.0.0.1:4444", options=o)
ta = d.find_element(By.CSS_SELECTOR, "textarea[aria-label='prompt']")
tabs = [b.text for b in d.find_elements(By.CSS_SELECTOR, "button[role='tab']")]
print("TABS:", tabs)
d.save_screenshot("/tmp/claude-webdriver-proof.png")
d.quit()
print("OK")
EOF
```

Expected: `TABS: ['Workspace', 'Context', 'Design', 'Architecture', 'Config']` (5 tabs — Architecture/Config prove real Tauri IPC is injected) then `OK`. The app window opens and closes by itself; no focus click, no xdotool. Read `/tmp/claude-webdriver-proof.png` to confirm the UI rendered. If `ArgOptions` rejects `set_capability` on this selenium version, substitute `from selenium.webdriver.common.options import BaseOptions` — same calls.

- [ ] **Step 5: Teardown**

Kill tauri-driver by the PID captured at spawn (capture `$!` on the `setsid ... &` line and `kill` that — never pattern-match). Leave Vite running if Task 2 follows immediately.

---

### Task 2: e2e harness + boot smoke test

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `thirtyfour` dev-dependency)
- Create: `src-tauri/tests/e2e_harness/mod.rs`
- Create: `src-tauri/tests/gui_smoke.rs` (Test file)

**Interfaces:**
- Consumes: `tauri-driver` + `WebKitWebDriver` on PATH (Task 1).
- Produces: `e2e_harness::Gui` with `pub async fn launch() -> Gui`, `pub driver: thirtyfour::WebDriver`, `pub async fn shutdown(self)`. Task 3's test uses exactly these.

- [ ] **Step 1: Add the thirtyfour dev-dependency**

```bash
cd /home/kalen/rust-agent-runtime/src-tauri && cargo add --dev thirtyfour
```

Expected: `Adding thirtyfour v0.3x` to `[dev-dependencies]`.

- [ ] **Step 2: Write the failing test**

Create `src-tauri/tests/gui_smoke.rs`:

```rust
//! GUI e2e smoke over WebDriver (tauri-driver + WebKitWebDriver). Drives the
//! REAL desktop app's DOM — no xdotool, no coordinates, no focus games.
//!
//! Run: `cd src-tauri && cargo test --test gui_smoke -- --ignored --test-threads=1 --nocapture`
//! Needs: a display, `tauri-driver` + `WebKitWebDriver` on PATH, npm (Vite is
//! auto-spawned if :5173 is down). `turn_smoke` additionally needs llama-server
//! on :8080 (qwen3.6-35b-a3b).

mod e2e_harness;

use e2e_harness::Gui;
use std::time::Duration;
use thirtyfour::prelude::*;

#[tokio::test]
#[ignore = "live: needs display + tauri-driver + vite (auto-spawned)"]
async fn boot_smoke() {
    let gui = Gui::launch().await;

    // Composer rendered → frontend loaded from Vite into the webview.
    gui.driver
        .query(By::Css("textarea[aria-label='prompt']"))
        .wait(Duration::from_secs(30), Duration::from_millis(500))
        .first()
        .await
        .expect("composer textarea should render");

    // Architecture/Config tabs only render when isTauri() sees real IPC —
    // this is the assertion that we drove the app, not a plain browser.
    let tabs = gui
        .driver
        .query(By::Css("button[role='tab']"))
        .wait(Duration::from_secs(10), Duration::from_millis(500))
        .all_from_selector()
        .await
        .expect("tab buttons");
    let mut labels = Vec::new();
    for t in &tabs {
        labels.push(t.text().await.unwrap_or_default());
    }
    assert!(
        labels.contains(&"Architecture".to_string()) && labels.contains(&"Config".to_string()),
        "expected Tauri-only tabs in {labels:?}"
    );

    gui.shutdown().await;
}
```

- [ ] **Step 3: Run to verify it fails**

```bash
cd /home/kalen/rust-agent-runtime/src-tauri && cargo test --test gui_smoke -- --ignored 2>&1 | tail -20
```

Expected: FAIL to compile — `file not found for module e2e_harness`.

- [ ] **Step 4: Write the harness**

Create `src-tauri/tests/e2e_harness/mod.rs`:

```rust
//! Shared WebDriver e2e harness: owns Vite (only if it spawned it),
//! tauri-driver, and the WebDriver session. Teardown kills by held child
//! PID / process group only — never pattern-match (pkill self-match footgun).

use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use thirtyfour::prelude::*;
use thirtyfour::Capabilities;

const VITE_URL: &str = "http://localhost:5173";

pub struct Gui {
    pub driver: WebDriver,
    tauri_driver: Child,
    vite: Option<Child>, // Some only if WE spawned it
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

async fn http_ok(url: &str) -> bool {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn wait_http(url: &str, timeout: Duration, what: &str) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if http_ok(url).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    panic!("{what} never became ready at {url}");
}

impl Gui {
    pub async fn launch() -> Gui {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let web_dir = manifest.parent().unwrap().join("web");
        let app_bin = manifest.join("target/debug/rust-agent-runtime-desktop");
        assert!(app_bin.exists(), "debug binary missing — run `cargo build` first");

        // 1. Vite: the debug binary renders from the devUrl, not bundled assets.
        let vite = if http_ok(VITE_URL).await {
            None
        } else {
            let child = Command::new("npm")
                .args(["--prefix", web_dir.to_str().unwrap(), "run", "dev"])
                .process_group(0)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn vite (npm --prefix web run dev)");
            wait_http(VITE_URL, Duration::from_secs(60), "vite").await;
            Some(child)
        };

        // 2. tauri-driver on free ports; WEBKIT_WEBDRIVER env overrides the
        //    native driver path if WebKitWebDriver isn't on PATH.
        let port = free_port();
        let native_port = free_port();
        let mut cmd = Command::new("tauri-driver");
        cmd.args(["--port", &port.to_string(), "--native-port", &native_port.to_string()])
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Ok(native) = std::env::var("WEBKIT_WEBDRIVER") {
            cmd.args(["--native-driver", &native]);
        }
        let tauri_driver = cmd.spawn().expect("spawn tauri-driver (is it on PATH? ~/.cargo/bin)");
        let base = format!("http://127.0.0.1:{port}");
        wait_http(&format!("{base}/status"), Duration::from_secs(15), "tauri-driver").await;

        // 3. WebDriver session — this launches the app in automation mode.
        let mut caps = Capabilities::new();
        caps.insert(
            "tauri:options".to_string(),
            serde_json::json!({ "application": app_bin.to_str().unwrap() }),
        );
        caps.insert("browserName".to_string(), serde_json::json!("wry"));
        let driver = WebDriver::new(&base, caps).await.expect("webdriver session");

        Gui { driver, tauri_driver, vite }
    }

    /// Graceful end: deletes the session (closes the app), then Drop reaps
    /// the child processes.
    pub async fn shutdown(self) {
        let _ = self.driver.clone().quit().await;
    }
}

impl Drop for Gui {
    fn drop(&mut self) {
        // SIGTERM the whole process group (tauri-driver → WebKitWebDriver → app).
        unsafe { libc::kill(-(self.tauri_driver.id() as i32), libc::SIGTERM) };
        let _ = self.tauri_driver.wait();
        if let Some(v) = &mut self.vite {
            unsafe { libc::kill(-(v.id() as i32), libc::SIGTERM) };
            let _ = v.wait();
        }
    }
}
```

Note: `quit(self)` consumes the handle; `WebDriver` is a cloneable Arc'd handle in current thirtyfour, so `self.driver.clone().quit()` compiles. If the resolved version isn't `Clone`, switch the field to `driver: Option<WebDriver>` (accessor `pub fn driver(&self) -> &WebDriver`) and `shutdown` does `self.driver.take().unwrap().quit().await`. Either way, keep the public surface Task 3 uses stable: `launch()`, driver access, `shutdown()`.

- [ ] **Step 5: Run to verify it passes**

```bash
cd /home/kalen/rust-agent-runtime/src-tauri && cargo test --test gui_smoke -- --ignored --test-threads=1 --nocapture 2>&1 | tail -15
```

Expected: `test boot_smoke ... ok`. The app window appears and closes on its own. Also verify no orphans: `ps -eo pid=,comm= | awk '$2 ~ /^rust-agent-runt|^tauri-driver|^WebKitWebDriver/'` prints nothing (Vite may remain if it pre-existed).

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime && git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tests/e2e_harness/mod.rs src-tauri/tests/gui_smoke.rs
git commit -m "test(desktop): WebDriver GUI e2e harness + boot smoke"
```

---

### Task 3: Live turn smoke test

**Files:**
- Modify: `src-tauri/tests/gui_smoke.rs` (append test)

**Interfaces:**
- Consumes: `e2e_harness::Gui::{launch, shutdown}`, `gui.driver` (Task 2).
- Produces: nothing downstream; final suite shape.

- [ ] **Step 1: Write the test** (append to `gui_smoke.rs`)

```rust
#[tokio::test]
#[ignore = "live: needs display + tauri-driver + vite + llama-server on :8080 (qwen3.6-35b-a3b)"]
async fn turn_smoke() {
    // Gate on the model server exactly like smoke_context_explorer.
    let health = reqwest::get("http://localhost:8080/health").await;
    assert!(
        health.map(|r| r.status().is_success()).unwrap_or(false),
        "llama-server not healthy on :8080 — start it before this test"
    );

    let gui = Gui::launch().await;
    let d = &gui.driver;

    let marker = "SQUIRREL7";
    let ta = d
        .query(By::Css("textarea[aria-label='prompt']"))
        .wait(Duration::from_secs(30), Duration::from_millis(500))
        .first()
        .await
        .expect("composer");
    ta.click().await.expect("focus composer");
    ta.send_keys(format!("Reply with exactly: {marker}")).await.expect("type prompt");
    // \u{E007} is the W3C WebDriver Enter keycode; the composer submits on Enter.
    ta.send_keys("\u{E007}").await.expect("send Enter");

    // The user bubble echoes the marker once; the assistant reply makes it >= 2.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let ret = d
            .execute("return document.body.innerText;", vec![])
            .await
            .expect("read innerText");
        let text = ret.json().as_str().unwrap_or_default().to_string();
        if text.matches(marker).count() >= 2 {
            break;
        }
        assert!(Instant::now() < deadline, "no assistant reply containing {marker} within 120s");
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }

    // Context tab shows a populated breakdown: the total line and the system chip.
    d.query(By::XPath("//button[@role='tab' and normalize-space()='Context']"))
        .wait(Duration::from_secs(10), Duration::from_millis(500))
        .first()
        .await
        .expect("Context tab")
        .click()
        .await
        .expect("click Context tab");
    d.query(By::XPath("//div[contains(text(), 'tokens')]"))
        .wait(Duration::from_secs(15), Duration::from_millis(500))
        .first()
        .await
        .expect("breakdown total line ('N / M tokens')");
    d.query(By::XPath("//button[contains(., 'system')]"))
        .wait(Duration::from_secs(10), Duration::from_millis(500))
        .first()
        .await
        .expect("system legend chip");

    // Evidence screenshot (webview-exact, focus-independent).
    let _ = d
        .screenshot(std::path::Path::new("target/gui_smoke_turn.png"))
        .await;

    gui.shutdown().await;
}
```

Also add `use std::time::Instant;` to the imports at the top of `gui_smoke.rs`.

- [ ] **Step 2: Run to verify it fails/passes appropriately**

First confirm the compile + gate: with llama down this must fail fast on the health assert, not hang. If llama is up (`curl -s -m 2 localhost:8080/health` → `{"status":"ok"}`):

```bash
cd /home/kalen/rust-agent-runtime/src-tauri && cargo test --test gui_smoke turn_smoke -- --ignored --nocapture 2>&1 | tail -15
```

Expected: `test turn_smoke ... ok` in well under the 120s reply window; `target/gui_smoke_turn.png` exists — Read it and confirm the transcript shows the marker reply and the Context pane is visible.

- [ ] **Step 3: Run the whole suite once**

```bash
cd /home/kalen/rust-agent-runtime/src-tauri && cargo test --test gui_smoke -- --ignored --test-threads=1 --nocapture 2>&1 | tail -6
```

Expected: `test result: ok. 2 passed`. (`--test-threads=1` is required — each test launches its own app instance.)

- [ ] **Step 4: Commit**

```bash
cd /home/kalen/rust-agent-runtime && git add src-tauri/tests/gui_smoke.rs
git commit -m "test(desktop): live turn smoke over WebDriver (composer -> reply -> Context tab)"
```

---

### Task 4: Rewrite auto-drive-tauri skill — WebDriver primary, xdotool fallback

**Files:**
- Modify: `.agents/skills/auto-drive-tauri/SKILL.md`

**Interfaces:**
- Consumes: everything above (commands and snippets must match what Tasks 1–3 proved).
- Produces: the skill future sessions load; no code interfaces.

- [ ] **Step 1: Update the driving ladder and stale test references**

In the ladder table, replace the L1 row's command `cargo test --test e2e_live -- --ignored --nocapture` (that test is GONE) and the L2 row:

```markdown
| L1 protocol e2e (**default for "does it work end to end"**) | webview→bridge→runtime→model, real token stream | `cd src-tauri && cargo test --test smoke_context_explorer -- --ignored --nocapture` (fast no-model gate: `--test llama_health`) | **yes** |
| L2 GUI driving (WebDriver) | the actual rendered webview / UX, via the DOM | `cd src-tauri && cargo test --test gui_smoke -- --ignored --test-threads=1` or drive interactively — see §GUI driving (WebDriver) | boot: no · turn: yes |
| L3 pixel fallback | native GTK chrome only (file dialogs, window decorations) | see §Pixel fallback | — |
```

Also fix the two prose references to `e2e_live.rs` (lines ~22–23 and ~77–79 in the current file) to point at `smoke_context_explorer.rs` as the reference bridge test.

- [ ] **Step 2: Replace the "GUI driving — last resort" section**

Change the heading `## GUI driving — last resort (only to validate actual rendering/UX)` to `## GUI driving — WebDriver (primary)` and replace the old intro paragraph with this content, keeping everything through the end of the old section for Step 3 to retitle:

```markdown
## GUI driving — WebDriver (primary)

Verified 2026-07-06: `tauri-driver` + WebKitGTK WebDriver drives the real app's
DOM — CSS selectors, real key events, `execute_script`, focus-independent
screenshots. No coordinates, no KWin consent dialog, no xclip, no focus click.

**Bring-up (order matters):**
1. Vite on :5173 (`curl -s -o /dev/null -w '%{http_code}' localhost:5173` → 200,
   else `setsid npm --prefix web run dev` and wait).
2. Debug binary built (`cd src-tauri && cargo build`).
3. `setsid ~/.cargo/bin/tauri-driver --port 4444 --native-port 4445 >/tmp/td.log 2>&1 &`
   then `curl -s 127.0.0.1:4444/status` → `"ready":true`. Do NOT launch the app
   yourself — the WebDriver session launches (and owns) it.

**Drive with `/usr/bin/python3` (the PATH python is a uv shim without selenium):**

```python
/usr/bin/python3 - <<'EOF'
from selenium.webdriver import Remote
from selenium.webdriver.common.options import ArgOptions
from selenium.webdriver.common.by import By
o = ArgOptions()
o.set_capability("tauri:options", {"application": "src-tauri/target/debug/rust-agent-runtime-desktop"})  # absolute path!
o.set_capability("browserName", "wry")
d = Remote("http://127.0.0.1:4444", options=o)
d.find_element(By.CSS_SELECTOR, "textarea[aria-label='prompt']").send_keys("Reply with: pong\ue007")  # \ue007 = W3C Enter, submits
# poll d.execute_script("return document.body.innerText") for the reply; assert on DOM text, not pixels
d.save_screenshot("/tmp/shot.png")   # webview-exact, focus-independent
d.quit()                             # ALWAYS quit — see session-lifetime note
EOF
```

**Useful selectors:** composer `textarea[aria-label='prompt']` (Enter submits —
there is no send button); tabs `button[role='tab']` with texts Workspace /
Context / Design / Architecture / Config (the last two render only under real
Tauri IPC); Context breakdown total `//div[contains(text(),'tokens')]`, legend
chips `//button[contains(., 'system')]`.

**Session lifetime = app lifetime.** WebKitWebDriver allows ONE session; a
python process that exits without `quit()` wedges the driver (kill tauri-driver
by PID to reset). You cannot attach to an already-running app. So: plan the
whole drive sequence, run it as ONE script, always `quit()`.

**Scripted equivalent:** `src-tauri/tests/gui_smoke.rs` (`boot_smoke`,
`turn_smoke`) with the reusable harness in `src-tauri/tests/e2e_harness/mod.rs`
— copy its lifecycle pattern for new GUI tests.

**Preflight if the driver misbehaves:** `dpkg -l libwebkit2gtk-4.1-0
webkitgtk-webdriver | grep ^ii` — the two versions must match.
```

- [ ] **Step 3: Retitle the old xdotool content as the fallback**

The old section's body (XWayland gates, KWin consent, XTEST rules, spectacle) stays verbatim, under a new heading inserted where the old intro ended:

```markdown
## Pixel fallback (native GTK chrome only)

WebDriver cannot reach native chrome: the `pick_workspace` GTK file dialog,
window decorations, or an already-running instance you can't relaunch. Only
then, fall back to the xdotool recipe below (verified 2026-06-24):
```

- [ ] **Step 4: Add the new failure modes to Common mistakes**

Append to the `## Common mistakes` list:

```markdown
- Launching the app yourself and then trying to WebDriver it → no attach
  semantics; the session must launch the app. Kill your instance first.
- Selenium script died without `quit()` → next session hangs at create. Kill
  tauri-driver by PID and restart it.
- Using PATH `python3` for selenium → uv shim, no apt packages. Use `/usr/bin/python3`.
- Driving pixels for something the DOM can answer → use WebDriver first; pixels
  are only for native chrome (§Pixel fallback).
```

- [ ] **Step 5: Verify the skill's own instructions**

Re-run the skill's bring-up + python snippet exactly as written in the updated file (fresh shell, from repo root). Expected: the snippet works verbatim — tabs print, screenshot saved, clean quit. Fix any drift between doc and reality now.

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime && git add .agents/skills/auto-drive-tauri/SKILL.md
git commit -m "docs(skills): auto-drive-tauri — WebDriver as primary GUI layer, xdotool demoted to fallback"
```

---

## Verification (spec success criteria)

1. `cd src-tauri && cargo test --test gui_smoke -- --ignored --test-threads=1` → 2 passed, no xdotool/xclip, no manual focus click (Tasks 2–3).
2. Interactive selenium one-script drive: type → send → read reply from DOM (Task 1 Step 4, re-proven in Task 4 Step 5).
3. Fresh-session skill sufficiency: Task 4 Step 5 runs the skill text verbatim.
