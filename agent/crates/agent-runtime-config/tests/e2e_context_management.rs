//! Deterministic, CI-runnable e2e for the advanced context-management subsystem,
//! driven through the real `AgentLoop` with a scripted model. Complements the
//! per-crate unit tests (offload policy, store, curated build) and the
//! `assemble.rs` registration test by exercising the full live round-trip:
//! a tool result is offloaded out of the window by `maintain`, then the model
//! pulls it back with `read_file` over the artifact path and the loop re-injects
//! the exact bytes.
//!
//! See docs/superpowers/specs/2026-06-25-context-management-design.md.

use agent_core::testkit::{AlwaysApprove, Scripted, ScriptedModel};
use agent_core::{
    AgentEvent, AgentLoop, ContextCurationMiddleware, ContextManager, CuratedContext, EventSink,
    LoopConfig, OffloadConfig, SessionArtifacts,
};
use agent_model::{Message, NativeProtocol, Role};
use agent_policy::RulePolicy;
use agent_tools::backend::{Backend, CompositeBackend, HostBackend, ReadOnlyToTools};
use agent_tools::{
    Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolRegistry, ToolSchema,
};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The guarded composite the loop's file tools see: the two artifact mounts
/// (read-only) over a HostBackend at the workspace root — exactly as assemble
/// builds it, so migrated `read_file large_tool_results/…` calls resolve.
fn composite_over(artifacts: &Arc<SessionArtifacts>, ws: &std::path::Path) -> Arc<dyn Backend> {
    Arc::new(CompositeBackend::new(
        vec![
            (
                "large_tool_results/".into(),
                Arc::new(ReadOnlyToTools(artifacts.results.clone())) as Arc<dyn Backend>,
            ),
            (
                "conversation_history/".into(),
                Arc::new(ReadOnlyToTools(artifacts.history.clone())) as Arc<dyn Backend>,
            ),
        ],
        Arc::new(HostBackend::new(ws.to_path_buf())),
    ))
}

/// Records each successful `ToolResult` as (name, content) so a test can assert
/// the exact bytes a tool returned — `read_file` returns the rehydrated
/// content as its result, captured here before any later re-offload.
#[derive(Default)]
struct Capture {
    tool_results: Mutex<Vec<(String, String)>>,
    /// Every ToolResult regardless of status — the denial pin needs the
    /// non-Ok write_file result BEFORE curation can offload it out of the window.
    all_results: Mutex<Vec<(String, String)>>,
    done: Mutex<bool>,
}
impl EventSink for Capture {
    fn emit(&self, e: AgentEvent) {
        match e {
            // Failures now also emit ToolResult (Task 1); filter on Ok so this sink counts
            // only successful blob results, mirroring pre-change semantics.
            AgentEvent::ToolResult {
                name,
                output,
                status,
                ..
            } => {
                self.all_results
                    .lock()
                    .unwrap()
                    .push((name.clone(), output.content.clone()));
                if matches!(status, agent_core::ToolStatus::Ok) {
                    self.tool_results
                        .lock()
                        .unwrap()
                        .push((name, output.content));
                }
            }
            AgentEvent::Done(_) => *self.done.lock().unwrap() = true,
            _ => {}
        }
    }
}
impl Capture {
    fn recall_content(&self) -> Option<String> {
        self.tool_results
            .lock()
            .unwrap()
            .iter()
            .find(|(n, _)| n == "read_file")
            .map(|(_, c)| c.clone())
    }
    /// The content of the first tool result emitted for `name`, any status.
    fn first_result(&self, name: &str) -> Option<String> {
        self.all_results
            .lock()
            .unwrap()
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| c.clone())
    }
}

/// A tool that always fails with a large error body — the canonical thing the
/// offload policy lifts out of the live window.
struct BoomTool {
    message: String,
}
#[async_trait::async_trait]
impl Tool for BoomTool {
    fn name(&self) -> &str {
        "boom"
    }
    fn description(&self) -> &str {
        "always fails with a large error"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "boom".into(),
            description: "always fails".into(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        }
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "boom".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "boom".into(),
        })
    }
    async fn execute(&self, _a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
        Err(ToolError::Failed {
            message: self.message.clone(),
            stderr: None,
        })
    }
}

fn loop_config(workspace: std::path::PathBuf) -> LoopConfig {
    LoopConfig {
        model_limit: 100_000,
        max_turns: 6,
        max_retries: 1,
        temperature: 0.0,
        max_tokens: Some(256),
        workspace,
        tool_timeout: Duration::from_secs(30),
        stream_idle_timeout: Duration::from_secs(120),
        ..Default::default()
    }
}

/// `artifacts`/`flag` are the SAME handles the test's `CuratedContext` uses.
/// The loop carries a composite over `artifacts` so the model's migrated
/// `read_file large_tool_results/…` recovery resolves (spec §5.5/§5.6).
fn build_loop(
    reg: ToolRegistry,
    model: ScriptedModel,
    sink: Arc<Capture>,
    ws: std::path::PathBuf,
    artifacts: Arc<SessionArtifacts>,
    flag: Arc<AtomicBool>,
) -> AgentLoop {
    let backend = composite_over(&artifacts, &ws);
    AgentLoop::new(
        Arc::new(model),
        Arc::new(NativeProtocol),
        Arc::new(reg),
        Arc::new(RulePolicy {
            workspace: ws.clone(),
            command_allowlist: vec![],
            command_denylist: vec![],
        }),
        Arc::new(AlwaysApprove),
        sink,
        loop_config(ws),
    )
    .with_middleware(vec![Arc::new(ContextCurationMiddleware::new(flag))])
    .with_backend(backend)
}

/// The headline round-trip: a large tool error is auto-offloaded by `maintain`
/// after turn 1; on turn 2 the model calls `read_file` on the artifact path and
/// the loop returns the original error verbatim (spec §5.5/§5.6).
#[tokio::test]
async fn offload_then_read_file_round_trips_through_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();

    // Shared artifacts: the same handle the composite reads and the context offloads into.
    let artifacts = Arc::new(SessionArtifacts::new());
    let flag = Arc::new(AtomicBool::new(false));

    let big_message = format!("disk exploded at sector {}", "9".repeat(300));
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(BoomTool {
        message: big_message.clone(),
    }));
    reg.register(Arc::new(agent_tools::fs::ReadFile {
        max_bytes: 16 * 1024,
    }));

    let model = ScriptedModel::new(vec![
        Scripted::Call("c1".into(), "boom".into(), "{}".into()),
        Scripted::Call(
            "c2".into(),
            "read_file".into(),
            r#"{"path":"large_tool_results/1-c1"}"#.into(),
        ),
        Scripted::Text("Recovered the error; all done.".into()),
    ]);
    let sink = Arc::new(Capture::default());

    // keep_recent: 0 makes the single turn-1 error immediately eligible to offload.
    let mut ctx = CuratedContext::new(Message::system("SYS"), artifacts.clone(), flag.clone())
        .with_offload_config(OffloadConfig {
            keep_recent: 0,
            error_min_bytes: 50,
            ..Default::default()
        });

    build_loop(reg, model, sink.clone(), ws, artifacts.clone(), flag)
        .run(&mut ctx, "Trigger the failure, then recover it.".into())
        .await
        .unwrap();

    // The boom error was offloaded (key 1-c1) and read_file returned it verbatim.
    let expected = format!("ERROR: failed: {big_message}");
    assert_eq!(
        artifacts.results.read("1-c1").await.unwrap(),
        expected,
        "stored content must be the raw error"
    );
    assert_eq!(
        sink.recall_content().as_deref(),
        Some(expected.as_str()),
        "read_file must return the exact offloaded bytes"
    );
    assert!(
        *sink.done.lock().unwrap(),
        "the run reached a normal completion"
    );
}

/// The error path: reading an artifact path that was never written feeds a
/// normal tool error back to the model, which continues rather than crashing.
#[tokio::test]
async fn read_missing_artifact_feeds_error_back_and_continues() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();

    let artifacts = Arc::new(SessionArtifacts::new());
    let flag = Arc::new(AtomicBool::new(false));

    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(agent_tools::fs::ReadFile {
        max_bytes: 16 * 1024,
    }));

    let model = ScriptedModel::new(vec![
        Scripted::Call(
            "c1".into(),
            "read_file".into(),
            r#"{"path":"large_tool_results/999-nope"}"#.into(),
        ),
        Scripted::Text("Nothing there; carrying on.".into()),
    ]);
    let sink = Arc::new(Capture::default());

    let mut ctx = CuratedContext::new(Message::system("SYS"), artifacts.clone(), flag.clone());

    build_loop(reg, model, sink.clone(), ws, artifacts, flag)
        .run(&mut ctx, "Read the missing artifact.".into())
        .await
        .unwrap();

    // The loop continued to a clean finish despite the read error.
    assert!(
        *sink.done.lock().unwrap(),
        "a missing-artifact read must not abort the run"
    );
    // The not-found error was appended to the transcript as a tool message.
    let built = ctx.build(100_000);
    let fed_back = built.iter().any(|m| {
        matches!(m.role, Role::Tool)
            && m.name.as_deref() == Some("read_file")
            && m.content.contains("999-nope")
    });
    assert!(
        fed_back,
        "the not-found error must be fed back as a read_file tool result"
    );
}

/// Guard pin (spec §5.2 ReadOnlyToTools; §7 guard pin): turn 1 offloads a big
/// result; turn 2 the model tries to overwrite the artifact and is denied by the
/// read-only mount; turn 3 reads it back and gets the ORIGINAL bytes, not the
/// forgery. Cloned from `offload_then_read_file_round_trips_through_the_loop`
/// (same build_loop, same oversized first result) with two extra scripted calls.
#[tokio::test]
async fn model_write_into_artifacts_is_denied_and_bytes_survive() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();

    let artifacts = Arc::new(SessionArtifacts::new());
    let flag = Arc::new(AtomicBool::new(false));

    let big_message = format!("disk exploded at sector {}", "9".repeat(300));
    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(BoomTool {
        message: big_message.clone(),
    }));
    reg.register(Arc::new(agent_tools::fs::ReadFile {
        max_bytes: 16 * 1024,
    }));
    reg.register(Arc::new(agent_tools::fs::WriteFile));

    let model = ScriptedModel::new(vec![
        Scripted::Call("c1".into(), "boom".into(), "{}".into()),
        // Turn 2: try to overwrite the offloaded artifact with a forgery.
        Scripted::Call(
            "c2".into(),
            "write_file".into(),
            r#"{"path":"large_tool_results/1-c1","content":"forged"}"#.into(),
        ),
        // Turn 3: read the artifact back — must be the original, not "forged".
        Scripted::Call(
            "c3".into(),
            "read_file".into(),
            r#"{"path":"large_tool_results/1-c1"}"#.into(),
        ),
        Scripted::Text("Tried to overwrite; read it back.".into()),
    ]);
    let sink = Arc::new(Capture::default());

    let mut ctx = CuratedContext::new(Message::system("SYS"), artifacts.clone(), flag.clone())
        .with_offload_config(OffloadConfig {
            keep_recent: 0,
            error_min_bytes: 50,
            ..Default::default()
        });

    build_loop(reg, model, sink.clone(), ws, artifacts.clone(), flag)
        .run(
            &mut ctx,
            "Trigger the failure, overwrite it, read it back.".into(),
        )
        .await
        .unwrap();

    let expected = format!("ERROR: failed: {big_message}");
    // The write into the read-only artifact mount was denied — the guard message
    // reached the model as the write_file tool result (ERROR: denied: …). Captured
    // at emit time, before curation can offload this error out of the window.
    let write_result = sink
        .first_result("write_file")
        .expect("write_file tool result emitted");
    assert!(
        write_result.starts_with("ERROR: denied:"),
        "the artifact write must be denied: {write_result}"
    );
    assert!(
        write_result.contains("read-only records of offloaded context"),
        "the denial must carry the guard message: {write_result}"
    );
    // The forgery never landed — the stored artifact is the original bytes.
    assert_eq!(
        artifacts.results.read("1-c1").await.unwrap(),
        expected,
        "stored artifact bytes must survive the denied overwrite"
    );
    // And the model's read-back returned the original, byte-for-byte.
    assert_eq!(
        sink.recall_content().as_deref(),
        Some(expected.as_str()),
        "read_file after the denied write must return the original bytes"
    );
    assert!(*sink.done.lock().unwrap(), "the run reached completion");
}

/// Deep-recovery pin (spec §5.5): grep-then-read_file in exactly two tool calls
/// recovers an evicted span. A CuratedContext-owned history.md is seeded with
/// three `## folded-N` sections via three real fold passes, then a `GrepTool`
/// locates `## folded-2` (one hit, carrying a line number) and a `ReadFile`
/// at that line offset returns the span's marker fact. Tool-level over the same
/// composite the loop uses — fast and sufficient.
#[tokio::test]
async fn deep_recovery_is_grep_then_read_file_in_two_calls() {
    use agent_core::testkit::CollectingSink;
    use agent_core::MaintCtx;
    use agent_model::ModelClient;
    use tokio_util::sync::CancellationToken;

    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();

    let artifacts = Arc::new(SessionArtifacts::new());
    let flag = Arc::new(AtomicBool::new(false));
    let mut ctx = CuratedContext::new(Message::system("SYS"), artifacts.clone(), flag)
        .with_offload_config(OffloadConfig {
            keep_recent: 1,
            ..Default::default()
        });

    let sink: Arc<dyn EventSink> = Arc::new(CollectingSink::default());
    let cancel = CancellationToken::new();

    // Three fold passes → three `## folded-{1,2,3}` sections in history.md. Each
    // pass appends a fresh batch of oversized user units, then a tiny model_limit
    // forces the oldest to fold (facts → ledger, verbatim originals → history.md).
    for batch in 0..3 {
        for i in 0..12 {
            ctx.append(Message::user(format!(
                "batch {batch} entry {i}: setting item_{batch}_{i} is assigned \
                 value {batch}{batch}{i}{i} for the manifest marker"
            )));
        }
        ctx.append(Message::assistant("working on it", None));
        // A distinct extraction line per pass so each fold commits a section.
        let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![Scripted::Text(
            format!("fold_{batch}_fact = {batch}"),
        )]));
        let deps = MaintCtx {
            model_limit: 250,
            model: &model,
            sink: &sink,
            cancel: &cancel,
        };
        ctx.maintain(&deps).await;
    }

    // Sanity: history.md carries all three fold sections.
    let history = artifacts.history.read("history.md").await.unwrap();
    for s in ["## folded-1", "## folded-2", "## folded-3"] {
        assert!(history.contains(s), "history.md missing {s}:\n{history}");
    }

    // The composite the loop's file tools see (the same shape as build_loop's).
    let backend = composite_over(&artifacts, &ws);
    let tool_ctx = |backend: Arc<dyn Backend>| ToolCtx {
        workspace: ws.clone(),
        timeout: Duration::from_secs(30),
        cancel: CancellationToken::new(),
        sandbox: Arc::new(agent_tools::HostExecutor),
        backend,
        call_id: "t1".into(),
    };

    // Call 1: grep for the middle section header, scoped to conversation_history/.
    let grep = agent_tools::fs::GrepTool
        .execute(
            serde_json::json!({"pattern": "## folded-2", "path": "conversation_history/"}),
            &tool_ctx(backend.clone()),
        )
        .await
        .unwrap();
    // Exactly one hit, rendered `path:line: text`, carrying a line number.
    let lines: Vec<&str> = grep.content.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one hit: {}", grep.content);
    let hit = lines[0];
    assert!(
        hit.starts_with("conversation_history/history.md:"),
        "hit re-prefixed to the mount path: {hit}"
    );
    // Parse `conversation_history/history.md:<line>: ## folded-2`.
    let after_path = hit
        .strip_prefix("conversation_history/history.md:")
        .unwrap();
    let line: usize = after_path
        .split(':')
        .next()
        .unwrap()
        .parse()
        .expect("hit carries a numeric line offset");
    assert!(line >= 1, "1-based line offset: {line}");

    // Call 2: read_file from the hit's line offset — the span's marker fact is
    // in the following lines (the verbatim [user] originals of batch 1).
    let read = (agent_tools::fs::ReadFile {
        max_bytes: 16 * 1024,
    })
    .execute(
        serde_json::json!({"path": "conversation_history/history.md", "offset": line}),
        &tool_ctx(backend),
    )
    .await
    .unwrap();
    assert!(
        read.content.contains("## folded-2"),
        "read starts at the section header: {}",
        read.content
    );
    assert!(
        read.content.contains("batch 1 entry 0"),
        "the section's verbatim span marker fact is recovered: {}",
        read.content
    );
}
