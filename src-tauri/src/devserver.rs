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
