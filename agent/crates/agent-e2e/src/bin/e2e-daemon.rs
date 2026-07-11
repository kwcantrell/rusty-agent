//! Real-process `Session` host for the e2e lifecycle & stress suite
//! (spec: docs/superpowers/specs/2026-07-10-e2e-lifecycle-stress-design.md).
//!
//! Two modes:
//! - `run` — constructs a `Session` exactly like `Rig::session` (same
//!   `agent_server::setup::local_params` + config-overlay wiring), attaches a
//!   stdout-JSON `EventOut` sink (`EV <json>` lines, one `ServerEvent` per
//!   line), optionally sends `--task`, then serves line commands from stdin
//!   until EOF.
//! - `hold-lock` — claims the resume lock on a checkpoint dir and holds it,
//!   as a kill target for the concurrent-resume-contention scenario.
use agent_server::wire::{Decision, EventOut, ServerEvent};
use std::io::{BufRead, Write};
use std::sync::Arc;

/// Print a line to stdout and flush immediately. Load-bearing: stdout is
/// block-buffered (not line-buffered) once it's a pipe, so a bare `println!`
/// here can sit in the buffer indefinitely and the parent's `wait_for_output`
/// / `wait_for_event` deadlines waiting for bytes that were never flushed.
fn out(line: &str) {
    println!("{line}");
    let _ = std::io::stdout().flush();
}

struct StdoutSink;
impl EventOut for StdoutSink {
    fn send(&self, ev: ServerEvent) {
        out(&format!("EV {}", serde_json::to_string(&ev).unwrap()));
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .map(|i| args[i + 1].clone())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hold-lock") => hold_lock_mode(&args),
        Some("run") => run_mode(&args).await,
        other => {
            eprintln!("unknown mode {other:?}");
            std::process::exit(2);
        }
    }
}

/// `--dir` is the session's CHECKPOINT dir (`rig::ckpt(&session_dir)`), not
/// the session dir itself — `claim_resume` operates on the dir that holds
/// `resume.lock` (mirrors `agent-cli/src/main.rs`'s resume path and
/// `Session::start_resume`, both of which pass the checkpoint root).
fn hold_lock_mode(args: &[String]) {
    let dir = std::path::PathBuf::from(flag(args, "--dir").expect("--dir"));
    match agent_core::checkpoint::claim_resume(&dir) {
        Ok(true) => {
            out("LOCKED");
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
        _ => std::process::exit(3),
    }
}

async fn run_mode(args: &[String]) {
    let ws = flag(args, "--workspace").expect("--workspace");
    let sessions = flag(args, "--sessions").expect("--sessions");
    let meta = flag(args, "--meta").expect("--meta");
    let base_url = flag(args, "--base-url").expect("--base-url");

    // Same file-overlay wiring as `Rig::session`: the config-path overlay
    // carries the trace/metadata roots so an overlay load can't undo them.
    let cfg_path = std::path::PathBuf::from(&meta).join("agent-runtime.json");
    std::fs::write(
        &cfg_path,
        serde_json::json!({"trace_dir": sessions, "metadata_dir": meta}).to_string(),
    )
    .unwrap();
    let mut params = agent_server::setup::local_params(
        ws.clone().into(),
        cfg_path,
        base_url.clone(),
        "stub-model".into(),
    );
    params.config.trace_dir = Some(sessions);
    params.config.metadata_dir = Some(meta);
    let session = agent_server::session::Session::from_params(params);
    // Attach the sink BEFORE printing READY. Note this can synchronously
    // re-emit prior parked runs (`set_event_out` -> `spawn_parked_reemit`) —
    // those `EV` lines may land before `READY`. That's fine: the driver's
    // `wait_for_event` scans the whole transcript rather than assuming
    // ordering relative to `READY`.
    session.set_event_out(Arc::new(StdoutSink));
    out("READY");
    if let Some(task) = flag(args, "--task") {
        let _ = session.send_input(task);
    }
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let mut it = line.splitn(3, ' ');
        match (it.next(), it.next(), it.next()) {
            (Some("input"), Some(rest), tail) => {
                let text = match tail {
                    Some(t) => format!("{rest} {t}"),
                    None => rest.into(),
                };
                let _ = session.send_input(text);
            }
            (Some("approve"), Some(id), _) => session.approve(id, Decision::Approve),
            (Some("always"), Some(id), _) => session.approve(id, Decision::ApproveAlways),
            (Some("deny"), Some(id), fb) => session.approve(
                id,
                Decision::Deny {
                    feedback: fb.map(str::to_string),
                },
            ),
            (Some("cancel"), _, _) => session.cancel(),
            _ => eprintln!("bad cmd: {line}"),
        }
    }
    // stdin EOF: park-and-stay — sleep so the test controls lifetime via kill.
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}
