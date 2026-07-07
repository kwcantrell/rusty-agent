//! Detects and manages a single local dev server for the Design canvas.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;

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

/// First `http(s)` URL on the line whose authority is exactly `localhost`/`127.0.0.1` (optional numeric port), or None.
pub fn parse_url(line: &str) -> Option<String> {
    let clean = strip_ansi(line);
    for scheme in ["http://", "https://"] {
        let Some(start) = clean.find(scheme) else { continue };
        let rest = &clean[start..];
        // URL ends at the first whitespace.
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let url = rest[..end].trim_end_matches(['.', ',', ')', '"', '\'']);
        let host = &url[scheme.len()..];
        if is_local_authority(host) {
            return Some(url.to_string());
        }
    }
    None
}

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

        let mut cmd = Command::new(&cand.package_manager);
        cmd.arg("run").arg(&cand.script)
            .current_dir(&dir)
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
        // Track per-stream EOF so we only exit once BOTH are drained.
        // Exiting early (on the first EOF) would leave the other pipe's buffer
        // filling up and deadlock/orphan the child, and would miss URLs printed
        // on the still-open stream.
        let tail_r = tail.clone();
        tokio::spawn(async move {
            let mut out = BufReader::new(stdout).lines();
            let mut err = BufReader::new(stderr).lines();
            let mut out_done = false;
            let mut err_done = false;
            let mut tx = Some(tx);
            let mut push = |line: String| {
                if let Some(url) = parse_url(&line) {
                    if let Some(s) = tx.take() { let _ = s.send(url); }
                }
                let mut t = tail_r.lock().unwrap();
                if t.len() == TAIL_LINES { t.pop_front(); }
                t.push_back(line);
            };
            loop {
                tokio::select! {
                    l = out.next_line(), if !out_done => match l {
                        Ok(Some(line)) => push(line),
                        _ => out_done = true,
                    },
                    l = err.next_line(), if !err_done => match l {
                        Ok(Some(line)) => push(line),
                        _ => err_done = true,
                    },
                    else => break,
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

    #[test]
    fn parse_url_rejects_prefix_and_userinfo_lookalike_hosts() {
        assert_eq!(parse_url("Local: http://localhost.evil.com:5173/"), None);
        assert_eq!(parse_url("Local: http://127.0.0.1.evil.com:5173/"), None);
        assert_eq!(parse_url("Local: http://localhost:1234@evil.com/"), None);
        assert_eq!(parse_url("Local: http://localhost@evil.com/"), None);
        assert_eq!(parse_url("Local: http://localhost:/"), None); // empty port
    }

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

        let status = mgr.start(cand, tmp.path()).await.expect("should discover URL");
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

    /// Regression test: URL is printed to STDERR *after* stdout has already closed.
    ///
    /// The old `_ => break` loop exited as soon as stdout sent Ok(None), so the
    /// still-open stderr was never drained and the 30-second timeout fired instead.
    /// The fixed loop disables each branch independently and only breaks via `else`
    /// once BOTH streams are drained, so it catches the stderr URL correctly.
    #[tokio::test]
    async fn start_discovers_url_from_stderr_after_stdout_closes() {
        let tmp = tempfile::tempdir().unwrap();
        // stdout ends immediately; 50 ms later stderr prints the URL and then the
        // node process keeps itself alive so stop() can kill it.
        write(tmp.path(), "package.json", r#"{"scripts":{"dev":"node stderr_url.js"}}"#);
        write(tmp.path(), "stderr_url.js", "\
process.stdout.end();\n\
setTimeout(() => {\n\
  console.error('  Local:   http://localhost:5298/');\n\
  setInterval(() => {}, 100000);\n\
}, 50);\n");
        let cand = DevScriptCandidate {
            dir: tmp.path().to_string_lossy().into_owned(),
            script: "dev".into(),
            package_manager: "npm".into(),
            label: "stderr-after-stdout-eof — dev".into(),
        };
        let mgr = DevServerManager::new();
        let status = mgr.start(cand, tmp.path()).await.expect("should discover URL from stderr");
        assert_eq!(status.url, "http://localhost:5298/");
        mgr.stop();
    }

    #[tokio::test]
    async fn start_rejects_disallowed_package_manager() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = DevServerManager::new();
        let cand = DevScriptCandidate {
            dir: tmp.path().to_string_lossy().into_owned(),
            script: "dev".into(),
            package_manager: "rm".into(),
            label: "x".into(),
        };
        let err = mgr.start(cand, tmp.path()).await.unwrap_err();
        assert!(!err.is_empty(), "error message must be non-empty");
        assert!(mgr.status().is_none(), "no server should run after rejection");
    }

    #[tokio::test]
    async fn start_rejects_unrecognized_script() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = DevServerManager::new();
        let cand = DevScriptCandidate {
            dir: tmp.path().to_string_lossy().into_owned(),
            script: "evil".into(),
            package_manager: "npm".into(),
            label: "x".into(),
        };
        let err = mgr.start(cand, tmp.path()).await.unwrap_err();
        assert!(!err.is_empty(), "error message must be non-empty");
        assert!(mgr.status().is_none(), "no server should run after rejection");
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
        let err = mgr.start(cand, tmp.path()).await.unwrap_err();
        assert!(!err.is_empty(), "error should carry a message");
        assert!(mgr.status().is_none());
    }

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
}
