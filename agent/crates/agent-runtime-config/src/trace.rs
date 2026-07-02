//! JSONL session trace + composite observability sink. NEVER fails a run:
//! every I/O error warns once and disables further writes.
use agent_core::{AgentEvent, ContextEvent, EventSink, SessionStats};
use agent_model::StopReason;
use serde::Serialize;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const TRACE_SCHEMA: u32 = 1;
const RETAIN_FILES: usize = 50;

pub struct TraceWriter {
    session_id: String,
    max_bytes: u64,
    inner: Mutex<Inner>,
}
struct Inner {
    w: Option<BufWriter<fs::File>>,
    written: u64,
    seq: u64,
}

impl TraceWriter {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn create(dir: &Path, max_mb: u64) -> Option<Arc<TraceWriter>> {
        if let Err(e) = fs::create_dir_all(dir) {
            tracing::warn!(target: "trace", error = %e, dir = %dir.display(),
                "cannot create trace dir; session tracing disabled");
            return None;
        }
        prune_retention(dir, RETAIN_FILES - 1); // -1: our new file makes 50
        let session_id = mint_session_id();
        let path = dir.join(format!("{session_id}.jsonl"));
        let file = match fs::OpenOptions::new().create(true).append(true).open(&path) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(target: "trace", error = %e, path = %path.display(),
                    "cannot open trace file; session tracing disabled");
                return None;
            }
        };
        let mut w = BufWriter::new(file);
        let header = serde_json::json!({
            "schema": TRACE_SCHEMA, "session": session_id, "started_ms": epoch_ms() });
        let _ = writeln!(w, "{header}");
        let _ = w.flush();
        Some(Arc::new(TraceWriter {
            session_id,
            max_bytes: max_mb.saturating_mul(1024 * 1024),
            inner: Mutex::new(Inner {
                w: Some(w),
                written: 0,
                seq: 0,
            }),
        }))
    }

    /// Append one event. Infallible; disables itself on error or cap breach.
    /// (Borrow order matters: read `seq`/`written` before taking `w` mutably.)
    pub fn record(&self, event: &AgentEvent) {
        self.write_record(None, None, event);
    }

    /// Record a sub-agent child event, attributed to dispatch ordinal `n` and
    /// the dispatching call's id `parent_id` (spec 2026-07-02 E4). The record's
    /// `parent_id` joins a zero-tool-call child's transcript to its dispatch row
    /// under parallel dispatch. Same file, same seq counter, same size cap.
    pub fn record_child(&self, n: u64, parent_id: &str, event: &AgentEvent) {
        self.write_record(Some(n), Some(parent_id), event);
    }

    fn write_record(&self, sub: Option<u64>, parent_id: Option<&str>, event: &AgentEvent) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if inner.w.is_none() {
            return;
        }
        let rec = TraceRecord {
            seq: inner.seq,
            ts_ms: epoch_ms(),
            sub,
            parent_id,
            event: trace_event(event),
        };
        let line = match serde_json::to_string(&rec) {
            Ok(l) => l,
            Err(_) => return,
        };
        if inner.written + line.len() as u64 + 1 > self.max_bytes {
            tracing::warn!(target: "trace", cap_mb = self.max_bytes / (1024 * 1024),
                "trace size cap reached; tracing disabled for this session");
            if let Some(w) = inner.w.as_mut() {
                let _ = w.flush();
            }
            inner.w = None;
            return;
        }
        let flush = matches!(event, AgentEvent::Done(_) | AgentEvent::Error(_));
        let failed = match inner.w.as_mut() {
            Some(w) => writeln!(w, "{line}").is_err() || (flush && w.flush().is_err()),
            None => return,
        };
        if failed {
            tracing::warn!(target: "trace", "trace write failed; tracing disabled for this session");
            inner.w = None;
            return;
        }
        inner.written += line.len() as u64 + 1;
        inner.seq += 1;
    }
}

fn mint_session_id() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs}-{}", std::process::id())
}
fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Keep only the newest `keep` *.jsonl files (name-sorted; epoch-prefixed names sort chronologically).
fn prune_retention(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<_> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .map(|e| e.path())
        .collect();
    names.sort();
    if names.len() > keep {
        let excess = names.len() - keep;
        for p in names.into_iter().take(excess) {
            let _ = fs::remove_file(p);
        }
    }
}

#[derive(Serialize)]
struct TraceRecord<'a> {
    seq: u64,
    ts_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    sub: Option<u64>,
    /// Dispatching call's id on child lines (record-level lineage join); None on
    /// parent lines. A zero-tool-call child still ties to its dispatch row here.
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<&'a str>,
    event: TraceEvent<'a>,
}

/// Serializable mirror of AgentEvent — a stable on-disk schema decoupled from
/// the in-process enum (same pattern as wire.rs's ServerEvent).
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TraceEvent<'a> {
    Token {
        text: &'a str,
    },
    Reasoning {
        text: &'a str,
    },
    Usage {
        prompt_tokens: usize,
        context_limit: usize,
        turn: usize,
        max_turns: usize,
    },
    ServerUsage {
        prompt_tokens: u32,
        completion_tokens: u32,
        reasoning_tokens: Option<u32>,
        cached_tokens: Option<u32>,
        cost_usd: Option<f64>,
        turn_duration_ms: u64,
        turn: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<&'a str>,
    },
    ToolStart {
        id: &'a str,
        name: &'a str,
        args: &'a serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<&'a str>,
    },
    ToolResult {
        id: &'a str,
        name: &'a str,
        status: &'static str,
        duration_ms: u64,
        content: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<&'a str>,
    },
    Approval {
        summary: &'a str,
        command: Option<&'a str>,
    },
    Error {
        message: &'a str,
    },
    Done {
        reason: &'static str,
    },
    Context {
        kind: &'static str,
        detail: serde_json::Value,
    },
    SandboxDegraded {
        mechanism: &'a str,
        reason: &'a str,
    },
}

fn trace_event(e: &AgentEvent) -> TraceEvent<'_> {
    match e {
        AgentEvent::Token(t) => TraceEvent::Token { text: t },
        AgentEvent::Reasoning(t) => TraceEvent::Reasoning { text: t },
        AgentEvent::Usage {
            prompt_tokens,
            context_limit,
            turn,
            max_turns,
        } => TraceEvent::Usage {
            prompt_tokens: *prompt_tokens,
            context_limit: *context_limit,
            turn: *turn,
            max_turns: *max_turns,
        },
        AgentEvent::ServerUsage {
            prompt_tokens,
            completion_tokens,
            reasoning_tokens,
            cached_tokens,
            cost_usd,
            turn_duration_ms,
            turn,
            parent_id,
        } => TraceEvent::ServerUsage {
            prompt_tokens: *prompt_tokens,
            completion_tokens: *completion_tokens,
            reasoning_tokens: *reasoning_tokens,
            cached_tokens: *cached_tokens,
            cost_usd: *cost_usd,
            turn_duration_ms: *turn_duration_ms,
            turn: *turn,
            parent_id: parent_id.as_deref(),
        },
        AgentEvent::ToolStart {
            id,
            name,
            args,
            parent_id,
        } => TraceEvent::ToolStart {
            id,
            name,
            args,
            parent_id: parent_id.as_deref(),
        },
        AgentEvent::ToolResult {
            id,
            name,
            status,
            output,
            duration_ms,
            parent_id,
        } => TraceEvent::ToolResult {
            id,
            name,
            status: status.as_str(),
            duration_ms: *duration_ms,
            content: &output.content,
            parent_id: parent_id.as_deref(),
        },
        AgentEvent::Approval(req) => TraceEvent::Approval {
            summary: &req.intent.summary,
            command: req.intent.command.as_deref(),
        },
        AgentEvent::Error(m) => TraceEvent::Error { message: m },
        AgentEvent::Done(r) => TraceEvent::Done {
            reason: stop_reason_str(r),
        },
        AgentEvent::Context(c) => match c {
            ContextEvent::Offloaded { id, bytes, tool } => TraceEvent::Context {
                kind: "offloaded",
                detail: serde_json::json!({"id": id, "bytes": bytes, "tool": tool}),
            },
            ContextEvent::Compacted {
                turns_replaced,
                tokens_before,
                tokens_after,
            } => TraceEvent::Context {
                kind: "compacted",
                detail: serde_json::json!({"turns_replaced": turns_replaced,
                        "tokens_before": tokens_before, "tokens_after": tokens_after}),
            },
            ContextEvent::CompactionFailed { reason } => TraceEvent::Context {
                kind: "compaction_failed",
                detail: serde_json::json!({"reason": reason}),
            },
            ContextEvent::Evicted {
                messages,
                est_tokens,
            } => TraceEvent::Context {
                kind: "evicted",
                detail: serde_json::json!({"messages": messages, "est_tokens": est_tokens}),
            },
            ContextEvent::OverflowRecovery => TraceEvent::Context {
                kind: "overflow_recovery",
                detail: serde_json::json!({}),
            },
        },
        AgentEvent::SandboxDegraded { mechanism, reason } => {
            TraceEvent::SandboxDegraded { mechanism, reason }
        }
    }
}

fn stop_reason_str(r: &StopReason) -> &'static str {
    match r {
        StopReason::Stop => "stop",
        StopReason::ToolCalls => "tool_calls",
        StopReason::Length => "length",
        StopReason::BudgetExhausted => "budget_exhausted",
        StopReason::Cancelled => "cancelled",
        StopReason::Error => "error",
    }
}

/// `SubagentTrace` over the session TraceWriter: child transcript lines land in
/// the same JSONL with a `sub` ordinal (spec E4).
pub struct ChildTraceTap(pub Arc<TraceWriter>);
impl agent_core::SubagentTrace for ChildTraceTap {
    fn record(&self, n: u64, parent_id: &str, event: &agent_core::AgentEvent) {
        self.0.record_child(n, parent_id, event);
    }
}

/// Composite sink: fold stats, write trace, forward to the frontend sink.
pub struct ObservedSink {
    pub inner: Arc<dyn EventSink>,
    pub stats: Arc<RwLock<SessionStats>>,
    pub trace: Option<Arc<TraceWriter>>,
}
impl EventSink for ObservedSink {
    fn emit(&self, event: AgentEvent) {
        if let Ok(mut s) = self.stats.write() {
            s.fold(&event);
        }
        if let Some(t) = &self.trace {
            t.record(&event);
        }
        self.inner.emit(event);
    }
}

/// Frontend helper: config → optional trace writer (None when disabled or dir unusable).
pub fn build_trace(cfg: &crate::RuntimeConfig) -> Option<Arc<TraceWriter>> {
    if !cfg.trace {
        return None;
    }
    let dir = match &cfg.trace_dir {
        Some(d) => std::path::PathBuf::from(d),
        None => std::path::PathBuf::from(std::env::var_os("HOME")?)
            .join(".agent")
            .join("sessions"),
    };
    TraceWriter::create(&dir, cfg.trace_max_mb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{AgentEvent, ToolStatus};
    use agent_tools::ToolOutput;

    fn ev_ok() -> AgentEvent {
        AgentEvent::ToolResult {
            id: "c1".into(),
            name: "read_file".into(),
            status: ToolStatus::Ok,
            output: ToolOutput {
                content: "hi".into(),
                display: None,
            },
            duration_ms: 7,
            parent_id: None,
        }
    }

    #[test]
    fn trace_parent_id_skipped_when_none_present_when_some() {
        let dir = tempfile::tempdir().unwrap();
        let w = TraceWriter::create(dir.path(), 64).unwrap();
        w.record(&agent_core::AgentEvent::ToolStart {
            id: "a".into(),
            name: "t".into(),
            args: serde_json::json!({}),
            parent_id: None,
        });
        w.record(&agent_core::AgentEvent::ToolStart {
            id: "b".into(),
            name: "t".into(),
            args: serde_json::json!({}),
            parent_id: Some("d1".into()),
        });
        w.record(&AgentEvent::Done(agent_model::StopReason::Stop)); // flushes the BufWriter
        let path = dir.path().join(format!("{}.jsonl", w.session_id()));
        let content = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert!(!lines[1].contains("parent_id"), "{}", lines[1]); // [0] is the header
        assert!(lines[2].contains(r#""parent_id":"d1""#), "{}", lines[2]);
    }

    #[test]
    fn record_child_lines_carry_sub_ordinal_and_normal_lines_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let w = TraceWriter::create(dir.path(), 1024 * 1024).unwrap();
        w.record(&agent_core::AgentEvent::Token("parent".into()));
        w.record_child(3, "d1", &agent_core::AgentEvent::Token("child".into()));
        w.record(&AgentEvent::Done(agent_model::StopReason::Stop)); // flushes the BufWriter
        let path = dir.path().join(format!("{}.jsonl", w.session_id()));
        let content = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        // Parent line: neither the `sub` ordinal nor the record-level parent_id.
        assert!(!lines[1].contains(r#""sub""#), "{}", lines[1]);
        assert!(!lines[1].contains(r#""parent_id""#), "{}", lines[1]);
        // Child line: both — `sub` ordinal AND the dispatch-call join key.
        assert!(lines[2].contains(r#""sub":3"#), "{}", lines[2]);
        assert!(lines[2].contains(r#""parent_id":"d1""#), "{}", lines[2]);
        // seq stays monotonic across both write paths:
        assert!(lines[2].contains(r#""seq":1"#), "{}", lines[2]);
    }

    #[test]
    fn trace_writes_parseable_jsonl_with_header() {
        let dir = tempfile::tempdir().unwrap();
        let w = TraceWriter::create(dir.path(), 64).unwrap();
        w.record(&ev_ok());
        w.record(&AgentEvent::Done(agent_model::StopReason::Stop)); // Done flushes
        let path = dir.path().join(format!("{}.jsonl", w.session_id()));
        let body = std::fs::read_to_string(path).unwrap();
        let lines: Vec<serde_json::Value> = body
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["schema"], 1);
        assert_eq!(lines[0]["session"], w.session_id());
        assert_eq!(lines[1]["event"]["type"], "tool_result");
        assert_eq!(lines[1]["event"]["status"], "ok");
        assert_eq!(lines[1]["seq"], 0);
        assert_eq!(lines[2]["event"]["type"], "done");
    }

    #[test]
    fn trace_respects_size_cap() {
        let dir = tempfile::tempdir().unwrap();
        let w = TraceWriter::create(dir.path(), 0).unwrap(); // 0 MB => cap hit immediately
        w.record(&ev_ok());
        w.record(&AgentEvent::Done(agent_model::StopReason::Stop));
        let path = dir.path().join(format!("{}.jsonl", w.session_id()));
        let body = std::fs::read_to_string(path).unwrap();
        assert_eq!(body.lines().count(), 1); // header only; cap stopped event writes
    }

    #[test]
    fn trace_prunes_to_retention() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..60 {
            std::fs::write(dir.path().join(format!("{:010}-1.jsonl", i)), "x").unwrap();
        }
        let _w = TraceWriter::create(dir.path(), 64).unwrap();
        let count = std::fs::read_dir(dir.path()).unwrap().count();
        assert!(count <= 50, "expected <=50 files after prune, got {count}");
    }

    #[test]
    fn trace_survives_unwritable_dir() {
        // A path that cannot be created (a FILE where the dir should be).
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocked");
        std::fs::write(&blocker, "not a dir").unwrap();
        assert!(TraceWriter::create(&blocker, 64).is_none()); // None, no panic
    }
}
