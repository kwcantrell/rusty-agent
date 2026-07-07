# Design-Tab Hardening (Audit Cluster 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close audit findings 8.1–8.6 (2026-07-06 harness+SDLC audit §8): workspace containment for the dev-server launcher, anchored URL host matching, graceful SIGTERM teardown, PDEATHSIG orphan prevention, explicit CSP `frame-src`, and an Image-artifact outbound-fetch guard.

**Architecture:** Five fixes land in the Tauri desktop shell (`src-tauri/` — its own Cargo workspace, separate from `agent/`), one in the web SPA (`web/`). All are localized hardening of existing code paths: `DevServerManager` (findings 8.1, 8.2, 8.4, 8.6), the desktop CSP (8.3), and `ArtifactRenderer` (8.5). No new modules, no wire changes.

**Tech Stack:** Rust (tokio, libc) in `src-tauri/`; JSON config (`tauri.conf.json`); React + vitest + @testing-library in `web/`.

## Global Constraints

- **Branch:** `feature/audit-design-tab-hardening`, off local `main` tip, in an isolated worktree.
- **Two Cargo workspaces:** all Rust work here is in `src-tauri/` (NOT `agent/`). Run `source ~/.cargo/env` first if `cargo` is missing.
- **NEVER run `cargo fmt` in `src-tauri/`** — it is hand-formatted by convention (compact style). Match the surrounding style by hand. (`agent/` is untouched by this plan, so its fmt gate is unaffected.)
- **Worktree discipline (every task):** run `git rev-parse --show-toplevel` and confirm it prints the worktree path (not `/home/kalen/rust-agent-runtime`) before EVERY commit. Never run `git reset` or `git rebase`.
- **Conventional commits:** `type(scope): summary`, e.g. `fix(devserver): …`.
- **If `ld` dies with SIGBUS:** the disk is full — run `df -h` and report back; do not retry blindly.
- **Test command:** `cd <worktree>/src-tauri && cargo test --workspace` (GTK deps are present on this machine). Targeted: `cargo test -p rust-agent-runtime-desktop --lib <name>` — check the crate name in `src-tauri/Cargo.toml` first; plain `cargo test <filter>` from `src-tauri/` also works.
- Unique fake-server ports per test (existing tests use 5199/5298; new tests use 5296/5297) so parallel test runs never collide.

---

### Task 1: Workspace containment for dev-server launch (finding 8.1, med)

`DevServerManager::start` trusts the SPA-provided `cand.dir` verbatim; `npm run dev` executes an arbitrary `dev` script from any attacker-planted `package.json`. Canonicalize and require containment in the current workspace.

**Files:**
- Modify: `src-tauri/src/devserver.rs` (`start()` signature + validation, existing tests)
- Modify: `src-tauri/src/lib.rs:79-85` (`dev_server_start` passes the workspace root)

**Interfaces:**
- Produces: `pub async fn start(&self, cand: DevScriptCandidate, workspace_root: &Path) -> Result<DevServerStatus, String>` — **all later tasks' tests call this two-arg form**, passing the tempdir as root.
- Consumes: `Bridge::current_workspace()` (exists, `src-tauri/src/bridge.rs:27`).

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `devserver.rs`; also make the whole file compile by updating call sites in Step 2 — signature changes make this task's red/green span both steps)

```rust
    #[tokio::test]
    async fn start_rejects_dir_outside_workspace_and_keeps_running_server() {
        let ws = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mgr = DevServerManager::new();

        // A live server inside the workspace...
        let cand = fake_server_candidate(ws.path());
        mgr.start(cand, ws.path()).await.expect("should discover URL");

        // ...must survive a rejected out-of-workspace start (validate-before-stop).
        let evil = fake_server_candidate(outside.path());
        let err = mgr.start(evil, ws.path()).await.unwrap_err();
        assert!(err.contains("outside the workspace"), "err: {err}");
        assert!(mgr.status().is_some(), "running server must survive a rejected start");

        // Traversal out of the workspace is caught by canonicalization.
        let mut sneaky = fake_server_candidate(outside.path());
        sneaky.dir = format!("{}/..", outside.path().display());
        let err = mgr.start(sneaky, ws.path()).await.unwrap_err();
        assert!(err.contains("outside the workspace"), "err: {err}");
        mgr.stop();
    }

    #[tokio::test]
    async fn start_rejects_nonexistent_dir() {
        let ws = tempfile::tempdir().unwrap();
        let mgr = DevServerManager::new();
        let cand = DevScriptCandidate {
            dir: format!("{}/does-not-exist", ws.path().display()),
            script: "dev".into(), package_manager: "npm".into(), label: "x".into(),
        };
        assert!(mgr.start(cand, ws.path()).await.is_err());
        assert!(mgr.status().is_none());
    }
```

- [ ] **Step 2: Change the signature and thread the root through existing call sites**

In `devserver.rs`, `start()` becomes (only the head and validation block change; body from `let mut cmd = …` on is untouched except `current_dir(&dir)`):

```rust
    pub async fn start(&self, cand: DevScriptCandidate, workspace_root: &Path)
        -> Result<DevServerStatus, String> {
        // Validate before touching the running server — a bogus candidate must not
        // tear down a live server. Package manager and script are whitelisted, and
        // the directory must sit inside the current workspace: the SPA picks among
        // detect()'s candidates, but the IPC boundary must not trust the echoed dir
        // (a planted package.json elsewhere would run an arbitrary `dev` script).
        const ALLOWED_PMS: &[&str] = &["npm", "pnpm", "yarn"];
        if !ALLOWED_PMS.contains(&cand.package_manager.as_str()) {
            return Err(format!("refusing to launch: {} is not an allowed package manager", cand.package_manager));
        }
        if !SERVER_SCRIPTS.contains(&cand.script.as_str()) {
            return Err(format!("refusing to launch: {} is not a recognized dev script", cand.script));
        }
        let root = workspace_root.canonicalize()
            .map_err(|e| format!("workspace root {} is not accessible: {e}", workspace_root.display()))?;
        let dir = Path::new(&cand.dir).canonicalize()
            .map_err(|e| format!("refusing to launch: {} is not accessible: {e}", cand.dir))?;
        if !dir.starts_with(&root) {
            return Err(format!("refusing to launch: {} is outside the workspace {}",
                dir.display(), root.display()));
        }
        self.stop(); // one server at a time
```

and the spawn uses the canonicalized dir: `.current_dir(&dir)` (replacing `.current_dir(&cand.dir)`).

In `lib.rs`, `dev_server_start` becomes:

```rust
#[tauri::command]
async fn dev_server_start(
    state: tauri::State<'_, AppState>,
    candidate: devserver::DevScriptCandidate,
) -> Result<devserver::DevServerStatus, String> {
    let ws = state.bridge.current_workspace().await;
    state.dev.start(candidate, &ws).await
}
```

Update the five existing tests that call `mgr.start(cand)` to `mgr.start(cand, tmp.path())` (they all already have a `tmp` tempdir whose path contains the candidate dir): `start_discovers_url_then_stop_kills_process_group`, `start_discovers_url_from_stderr_after_stdout_closes`, `start_rejects_disallowed_package_manager`, `start_rejects_unrecognized_script`, `start_reports_error_when_no_url_appears`.

- [ ] **Step 3: Run the devserver tests**

Run: `cd src-tauri && cargo test devserver`
Expected: PASS, including both new tests.

- [ ] **Step 4: Commit**

```bash
git rev-parse --show-toplevel   # MUST print the worktree path
git add src-tauri/src/devserver.rs src-tauri/src/lib.rs
git commit -m "fix(devserver): require workspace containment for dev-server launch dir (audit 8.1)"
```

---

### Task 2: Anchor the dev-server URL host check (finding 8.4, low)

`parse_url` prefix-matches (`host.starts_with("localhost")`), accepting `http://localhost.evil.com:5173/` and `http://localhost:1234@evil.com/`. Anchor the authority exactly, mirroring `agent-tools`' `validate_local_url` posture (refuse `@`, exact host, numeric port).

**Files:**
- Modify: `src-tauri/src/devserver.rs:115-130` (`parse_url` + new helper, tests)

**Interfaces:**
- Produces: `fn is_local_authority(rest: &str) -> bool` (private helper); `parse_url` signature unchanged.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn parse_url_rejects_prefix_and_userinfo_lookalike_hosts() {
        assert_eq!(parse_url("Local: http://localhost.evil.com:5173/"), None);
        assert_eq!(parse_url("Local: http://127.0.0.1.evil.com:5173/"), None);
        assert_eq!(parse_url("Local: http://localhost:1234@evil.com/"), None);
        assert_eq!(parse_url("Local: http://localhost@evil.com/"), None);
        assert_eq!(parse_url("Local: http://localhost:/"), None); // empty port
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test parse_url_rejects_prefix`
Expected: FAIL — current prefix match accepts all five.

- [ ] **Step 3: Implement the anchored check**

Replace the body of `parse_url`'s host test and add the helper (above `parse_url`):

```rust
/// Exact-host check for the authority part (everything up to the first `/?#`):
/// host must be exactly `localhost` or `127.0.0.1`, optionally `:port` (digits),
/// and any userinfo (`@`) is refused — prefix matching would accept
/// `localhost.evil.com` and `localhost:1234@evil.com`.
fn is_local_authority(rest: &str) -> bool {
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.contains('@') { return false; }
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    (host == "localhost" || host == "127.0.0.1")
        && port.is_none_or(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}
```

In `parse_url`, replace

```rust
        if host.starts_with("localhost") || host.starts_with("127.0.0.1") {
```

with

```rust
        if is_local_authority(host) {
```

and update `parse_url`'s doc comment to: ``/// First `http(s)` URL on the line whose authority is exactly `localhost`/`127.0.0.1` (optional numeric port), or None.``

- [ ] **Step 4: Run the tests**

Run: `cd src-tauri && cargo test parse_url`
Expected: PASS — all four existing `parse_url_*` tests plus the new one (existing positives carry ports/paths and still anchor-match).

- [ ] **Step 5: Commit**

```bash
git rev-parse --show-toplevel   # MUST print the worktree path
git add src-tauri/src/devserver.rs
git commit -m "fix(devserver): anchor localhost host check in parse_url (audit 8.4)"
```

---

### Task 3: SIGTERM grace window before SIGKILL in stop() (finding 8.6, low)

`stop()` sends SIGTERM and SIGKILL back-to-back, so the graceful signal is meaningless and the "backstop" doc comment lies. Give the group a bounded grace window, then SIGKILL as a real backstop (a no-op ESRCH if everything already exited).

**Files:**
- Modify: `src-tauri/src/devserver.rs:164-174` (`stop()`), plus a `STOP_GRACE` const and tests

**Interfaces:**
- Consumes: two-arg `start(cand, root)` from Task 1.
- Produces: `stop()` signature unchanged; new `const STOP_GRACE: std::time::Duration`.

- [ ] **Step 1: Write the failing test** (graceful path — the SIGTERM handler must get to run)

```rust
    /// stop() must give the group a grace window: a server that traps SIGTERM
    /// writes a marker and exits cleanly; the immediate-SIGKILL bug killed it
    /// before the handler ran.
    #[tokio::test]
    async fn stop_gives_sigterm_grace_before_sigkill() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "package.json", r#"{"scripts":{"dev":"node graceful.js"}}"#);
        write(tmp.path(), "graceful.js", "\
const fs = require('fs');\n\
process.on('SIGTERM', () => { fs.writeFileSync('term.marker', ''); process.exit(0); });\n\
console.log('  Local:   http://localhost:5297/');\n\
setInterval(() => {}, 100000);\n");
        let cand = DevScriptCandidate {
            dir: tmp.path().to_string_lossy().into_owned(),
            script: "dev".into(), package_manager: "npm".into(), label: "graceful — dev".into(),
        };
        let mgr = DevServerManager::new();
        mgr.start(cand, tmp.path()).await.expect("should discover URL");
        mgr.stop();
        // stop() waits for the direct child, which outlives the node process,
        // so by the time it returns the handler has run.
        assert!(tmp.path().join("term.marker").exists(),
            "SIGTERM handler should have run before SIGKILL");
    }

    /// The SIGKILL backstop still fires for a server that ignores SIGTERM.
    #[tokio::test]
    async fn stop_still_kills_a_sigterm_ignoring_server() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "package.json", r#"{"scripts":{"dev":"node stubborn.js"}}"#);
        write(tmp.path(), "stubborn.js", "\
process.on('SIGTERM', () => {});\n\
console.log('  Local:   http://localhost:5296/');\n\
setInterval(() => {}, 100000);\n");
        let cand = DevScriptCandidate {
            dir: tmp.path().to_string_lossy().into_owned(),
            script: "dev".into(), package_manager: "npm".into(), label: "stubborn — dev".into(),
        };
        let mgr = DevServerManager::new();
        mgr.start(cand, tmp.path()).await.expect("should discover URL");
        let pid = mgr.running_pid().expect("running pid");
        mgr.stop();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        // kill(-pgid, 0) probes the whole group; ESRCH once every member is gone.
        let alive = unsafe { libc::kill(-(pid as i32), 0) } == 0;
        assert!(!alive, "group should be SIGKILLed after the grace window");
    }
```

- [ ] **Step 2: Run to verify the graceful test fails**

Run: `cd src-tauri && cargo test stop_gives_sigterm_grace`
Expected: FAIL — no `term.marker` (SIGKILL arrives before the handler runs). `stop_still_kills_a_sigterm_ignoring_server` may already pass; that is fine, it pins the backstop.

- [ ] **Step 3: Implement the grace window**

Add near the other consts (below `TAIL_LINES`):

```rust
/// How long stop() lets the group act on SIGTERM before the SIGKILL backstop.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);
```

Replace `stop()` (keep it sync — callers include Drop and the window-event handler; the wait blocks at most STOP_GRACE and only when a child ignores SIGTERM):

```rust
    /// SIGTERM the whole process group, wait up to STOP_GRACE for the direct
    /// child to exit (so servers can flush/remove pidfiles), then SIGKILL the
    /// group as a backstop — a no-op (ESRCH) if everything already exited.
    pub fn stop(&self) {
        let taken = self.current.lock().unwrap().take();
        let Some(mut r) = taken else { return };
        #[cfg(unix)]
        unsafe { libc::kill(-(r.pid as i32), libc::SIGTERM); }
        // Poll the direct child (try_wait also reaps it); a graceful exit
        // short-circuits straight to the backstop.
        let deadline = std::time::Instant::now() + STOP_GRACE;
        while std::time::Instant::now() < deadline {
            match r.child.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(25)),
            }
        }
        // Group members that ignored SIGTERM (or outlived the leader) die here.
        #[cfg(unix)]
        unsafe { libc::kill(-(r.pid as i32), libc::SIGKILL); }
        let _ = r.child.start_kill();
    }
```

Note: `let taken = …; let Some(mut r) = taken else …` deliberately drops the mutex guard before the wait loop, so `status()` never blocks behind a stop in progress.

- [ ] **Step 4: Run the devserver tests**

Run: `cd src-tauri && cargo test devserver`
Expected: PASS — both new tests and the existing `start_discovers_url_then_stop_kills_process_group` (node's default SIGTERM disposition exits immediately, so the grace loop is fast).

- [ ] **Step 5: Commit**

```bash
git rev-parse --show-toplevel   # MUST print the worktree path
git add src-tauri/src/devserver.rs
git commit -m "fix(devserver): bounded SIGTERM grace before SIGKILL backstop in stop() (audit 8.6)"
```

---

### Task 4: PDEATHSIG so the dev server dies with a killed app (finding 8.2, low)

`process_group(0)` detaches the child; every reaper (`kill_on_drop`, `Drop`, `WindowEvent::Destroyed`) is destructor/handler-based, so a SIGKILLed or aborted app orphans the server. Set `PR_SET_PDEATHSIG` in a `pre_exec`.

**Files:**
- Modify: `src-tauri/src/devserver.rs` (new `harden_child` fn, call in `start()`, test)

**Interfaces:**
- Produces: `#[cfg(target_os = "linux")] fn harden_child(cmd: &mut Command)` (private).

- [ ] **Step 1: Write the failing test** (a child can read its own pdeathsig via `PR_GET_PDEATHSIG` = 2; python3 is on this machine and the GH runner, and the src-tauri ci.sh leg only runs where GTK deps exist)

```rust
    /// harden_child must arm PR_SET_PDEATHSIG: the child reads its own death
    /// signal back via PR_GET_PDEATHSIG and reports it on stdout.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn harden_child_sets_pdeathsig_sigterm() {
        let mut cmd = Command::new("python3");
        cmd.arg("-c")
            .arg("import ctypes;v=ctypes.c_int();ctypes.CDLL(None).prctl(2,ctypes.byref(v));print(v.value)")
            .stdout(Stdio::piped());
        harden_child(&mut cmd);
        let out = cmd.output().await.expect("python3 probe");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(),
                   libc::SIGTERM.to_string(),
                   "child should see PDEATHSIG=SIGTERM");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test harden_child_sets_pdeathsig`
Expected: FAIL to compile (`harden_child` not defined) — that counts as red.

- [ ] **Step 3: Implement `harden_child` and call it from `start()`**

Add above `impl DevServerManager` (or near `parse_url` — match file layout):

```rust
/// Arm the child to die with the app: kill_on_drop and the Destroyed/Drop
/// reapers only cover graceful exits, and process_group(0) detaches the child,
/// so a SIGKILLed/aborted app would orphan the server. PDEATHSIG closes that
/// path. SIGTERM (not SIGKILL) so the package manager can forward it to the
/// actual server process. Caveat: the signal fires when the spawning *thread*
/// dies; tokio's core workers live for the runtime's (= app's) lifetime.
#[cfg(target_os = "linux")]
fn harden_child(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The app may have died between fork and prctl; bail if reparented.
            if libc::getppid() == 1 {
                return Err(std::io::Error::other("parent died before PDEATHSIG was armed"));
            }
            Ok(())
        });
    }
}
```

In `start()`, right after `cmd.process_group(0);`:

```rust
        #[cfg(target_os = "linux")]
        harden_child(&mut cmd);
```

- [ ] **Step 4: Run the devserver tests**

Run: `cd src-tauri && cargo test devserver`
Expected: PASS, including the new probe test.

- [ ] **Step 5: Commit**

```bash
git rev-parse --show-toplevel   # MUST print the worktree path
git add src-tauri/src/devserver.rs
git commit -m "fix(devserver): arm PR_SET_PDEATHSIG so dev server dies with a killed app (audit 8.2)"
```

---

### Task 5: Explicit CSP frame-src for the live-preview iframe (finding 8.3, med)

The desktop CSP has no `frame-src`, so frames fall back to `default-src 'self'`, which blocks the UrlArtifact iframe (`http://localhost:*`) in bundled builds — dev only works because the Vite-served webview doesn't get the production CSP.

**Files:**
- Modify: `src-tauri/tauri.conf.json:17`
- Modify: `src-tauri/src/lib.rs` (regression test in `cmd_tests`)

**Interfaces:** none (config + test only).

- [ ] **Step 1: Write the failing test** (append inside `mod cmd_tests` in `lib.rs`)

```rust
    /// The live-preview iframe (UrlArtifact, http://localhost:*) must be allowed
    /// by an explicit frame-src — with none, frames fall back to
    /// default-src 'self' and bundled builds block the Design canvas.
    #[test]
    fn csp_declares_frame_src_for_localhost_preview() {
        let conf = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))
            .expect("read tauri.conf.json");
        let v: serde_json::Value = serde_json::from_str(&conf).expect("parse tauri.conf.json");
        let csp = v["app"]["security"]["csp"].as_str().expect("csp string");
        let frame = csp.split(';').map(str::trim)
            .find(|d| d.starts_with("frame-src"))
            .expect("csp must declare an explicit frame-src");
        for src in ["'self'", "http://localhost:*", "http://127.0.0.1:*"] {
            assert!(frame.contains(src), "frame-src must allow {src}: {frame}");
        }
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test csp_declares_frame_src`
Expected: FAIL — "csp must declare an explicit frame-src".

- [ ] **Step 3: Add the directive** (one line in `tauri.conf.json`)

```json
      "csp": "default-src 'self'; connect-src 'self' ws://127.0.0.1:* ws://localhost:* http://localhost:*; frame-src 'self' http://localhost:* http://127.0.0.1:*; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:"
```

- [ ] **Step 4: Run the test**

Run: `cd src-tauri && cargo test csp_declares_frame_src`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git rev-parse --show-toplevel   # MUST print the worktree path
git add src-tauri/tauri.conf.json src-tauri/src/lib.rs
git commit -m "fix(desktop): explicit CSP frame-src for localhost live preview (audit 8.3)"
```

*(Live verification that the canvas renders under the production CSP happens in the whole-branch review phase — see Final Verification below — because it needs a bundled-asset build, not a per-task step.)*

---

### Task 6: Image artifact outbound-fetch guard (finding 8.5, low)

`ArtifactRenderer` puts an agent-supplied http(s) URL straight into `<img src>`; the desktop CSP blocks it but the browser SPA ships no CSP, making it an exfil-beacon channel. Allow `data:` URIs and the same localhost set the Url artifact accepts; block remote http(s) with a notice.

**Files:**
- Modify: `web/src/components/inspector/ArtifactRenderer.tsx:57-61`
- Create: `web/src/components/inspector/ArtifactRenderer.image.test.tsx`

**Interfaces:**
- Consumes: `isLocalUrl` from `web/src/components/inspector/urlGuard.ts` (exists, unchanged).

- [ ] **Step 1: Write the failing test**

```tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ArtifactRenderer } from "./ArtifactRenderer";
import type { Display } from "../../wire";

const img = (data: string, mime = "image/png"): Display =>
  ({ Image: { mime, data } }) as Display;

describe("ArtifactRenderer Image", () => {
  it("renders data: URIs verbatim", () => {
    render(<ArtifactRenderer display={img("data:image/png;base64,AAAA")} />);
    expect(screen.getByRole("img")).toHaveAttribute("src", "data:image/png;base64,AAAA");
  });

  it("wraps raw base64 into a data URI", () => {
    render(<ArtifactRenderer display={img("AAAA")} />);
    expect(screen.getByRole("img")).toHaveAttribute("src", "data:image/png;base64,AAAA");
  });

  it("renders localhost http images", () => {
    render(<ArtifactRenderer display={img("http://localhost:5173/chart.png")} />);
    expect(screen.getByRole("img")).toHaveAttribute("src", "http://localhost:5173/chart.png");
  });

  it("blocks remote http(s) images with a notice instead of an img", () => {
    render(<ArtifactRenderer display={img("https://evil.com/pixel.gif")} />);
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
    expect(screen.getByText(/blocked remote image/i)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run to verify the blocking test fails**

Run: `cd web && npx vitest run src/components/inspector/ArtifactRenderer.image.test.tsx`
Expected: 3 pass, "blocks remote http(s) images" FAILS (img renders today).

- [ ] **Step 3: Implement the guard**

In `ArtifactRenderer.tsx`, add the import and replace the Image branch:

```tsx
import { isLocalUrl } from "./urlGuard";
```

```tsx
  if ("Image" in display) {
    const { mime, data } = display.Image;
    // Agent-controlled http(s) srcs are an outbound-fetch channel (tracking
    // pixel / exfil beacon) on the browser path, which ships no CSP — allow
    // only data: URIs and the same localhost set UrlArtifact accepts.
    if (data.startsWith("http") && !isLocalUrl(data)) {
      return (
        <div className="p-3 text-sm" style={{ color: "var(--text-muted)" }}>
          Blocked remote image URL — only data: and localhost image sources render here.
        </div>
      );
    }
    const src = data.startsWith("http") || data.startsWith("data:") ? data : `data:${mime};base64,${data}`;
    return <div className="p-3"><img src={src} alt="rendered artifact" className="max-w-full rounded" /></div>;
  }
```

- [ ] **Step 4: Run the web suite**

Run: `cd web && npm run typecheck && npx vitest run`
Expected: PASS (typecheck clean; new file 4/4; no other suite regressions).

- [ ] **Step 5: Commit**

```bash
git rev-parse --show-toplevel   # MUST print the worktree path
git add web/src/components/inspector/ArtifactRenderer.tsx web/src/components/inspector/ArtifactRenderer.image.test.tsx
git commit -m "fix(web): block remote http(s) Image artifact srcs (audit 8.5)"
```

---

## Final Verification (orchestrator, whole-branch phase)

- [ ] `bash scripts/ci.sh` in the worktree — all legs green, including the conditional src-tauri clippy+test leg (this machine has GTK) and the web leg.
- [ ] **Production-CSP live check (finding 8.3's second clause), best effort:** build bundled-asset app (`cd web && npm run build`, then a release/bundled src-tauri build) and confirm the Design canvas live-preview iframe renders under the production CSP (auto-drive-tauri WebDriver recipe; xdotool fallback). If the environment blocks this (build cost/display), record "production-CSP render verification pending" as a residual in the ledger and audit re-stamp rather than skipping silently. Watch disk (`df -h`) — release builds are large and ld SIGBUS = disk full.
- [ ] Whole-branch code review (requesting-code-review), then `--no-ff` merge to `main`.
- [ ] Re-stamp findings 8.1–8.6 in `.agents/skills/harness-engineering/audit.md`; ledger section in `.superpowers/sdd/progress.md`; memory update.
