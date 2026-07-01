//! Deterministic, CI-runnable STRESS suite for the context-management subsystem.
//! Where `e2e_context_management.rs` proves single round-trips, this file pushes
//! the machinery under volume and asserts its invariants never break:
//!
//!   1. Loop-level: ~60 large tool outputs + interleaved recalls — the prompt
//!      sent to the model stays bounded by `model_limit` every single turn, and
//!      arbitrary offloaded entries recall to their exact bytes.
//!   2. Direct: hundreds of offload cycles — `build()` stays within budget,
//!      every offloaded entry is recoverable, and tool<->result linkage holds.
//!   3. Compaction: repeated high-water compaction over 100 turns keeps history
//!      bounded and coherent, never panics, and preserves the newest turns.
//!   4. Concurrency: 16 tasks hammering the shared store allocate unique ids and
//!      never lose or corrupt an entry.
//!
//! All deterministic (ScriptedModel / no network), so they run in CI.

use agent_core::testkit::{AlwaysApprove, Scripted, ScriptedModel};
use agent_core::{
    built_tokens, AgentEvent, AgentLoop, ContextManager, ContextRecallTool, CuratedContext,
    EventSink, InMemoryOffloadStore, LoopConfig, MaintCtx, MaintReport, OffloadConfig,
    OffloadEntry, OffloadKind, OffloadStore,
};
use agent_model::{Message, ModelClient, NativeProtocol, OpenAiCompatClient, Role};
use agent_policy::RulePolicy;
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolRegistry, ToolSchema};
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// ~1.4 KB of body so each output crosses `output_min_bytes` (1024) and is
/// eligible to offload. A unique `<<n=K>>` marker lets a recall assert exact bytes.
fn blob_body(n: u64) -> String {
    format!("<<n={n}>> {}", "lorem ipsum dolor sit amet ".repeat(50))
}

/// A tool that echoes a large, uniquely-marked body built from its `n` argument.
struct BlobTool;
#[async_trait::async_trait]
impl Tool for BlobTool {
    fn name(&self) -> &str {
        "blob"
    }
    fn description(&self) -> &str {
        "returns a large uniquely-marked blob"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "blob".into(),
            description: "returns a large blob".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "n": { "type": "integer" } },
                "required": ["n"]
            }),
        }
    }
    fn intent(&self, _a: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: "blob".into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "blob".into(),
        })
    }
    async fn execute(&self, a: serde_json::Value, _c: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let n = a.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok(ToolOutput { content: blob_body(n), display: None })
    }
}

/// Captures the max prompt size the loop ever sent, plus every recall result and
/// enough structure for the live test to assert real volume + clean completion.
#[derive(Default)]
struct StressSink {
    max_prompt_tokens: Mutex<usize>,
    recalls: Mutex<Vec<String>>,
    blob_results: Mutex<usize>,
    errors: Mutex<Vec<String>>,
    done: Mutex<bool>,
}
impl EventSink for StressSink {
    fn emit(&self, e: AgentEvent) {
        match e {
            AgentEvent::Usage { prompt_tokens, .. } => {
                let mut m = self.max_prompt_tokens.lock().unwrap();
                *m = (*m).max(prompt_tokens);
            }
            AgentEvent::ToolResult { name, output, .. } if name == "context_recall" => {
                self.recalls.lock().unwrap().push(output.content)
            }
            AgentEvent::ToolResult { name, .. } if name == "blob" => {
                *self.blob_results.lock().unwrap() += 1
            }
            AgentEvent::Error(m) => self.errors.lock().unwrap().push(m),
            AgentEvent::Done(_) => *self.done.lock().unwrap() = true,
            _ => {}
        }
    }
}

/// 1. LOOP-LEVEL STRESS — many large tool outputs + interleaved recalls.
/// Invariant: the prompt sent to the model never exceeds `model_limit`, however
/// many large results pile up; and offloaded entries recall to their exact bytes.
#[tokio::test]
async fn loop_stays_bounded_under_many_large_outputs_and_recalls() {
    const BLOBS: u64 = 60;
    const MODEL_LIMIT: usize = 4000;

    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    let store: Arc<dyn OffloadStore> = Arc::new(InMemoryOffloadStore::new());
    let flag = Arc::new(AtomicBool::new(false));

    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(BlobTool));
    reg.register(Arc::new(ContextRecallTool::new(store.clone())));

    // Script: BLOBS turns each calling blob(n=i), then recall a spread of ids that
    // are guaranteed to have been offloaded, then finish.
    let mut script: Vec<Scripted> = (1..=BLOBS)
        .map(|i| Scripted::Call(format!("b{i}"), "blob".into(), format!(r#"{{"n":{i}}}"#)))
        .collect();
    let recall_ids = [1u64, 15, 30, 45];
    for id in recall_ids {
        script.push(Scripted::Call(
            format!("r{id}"),
            "context_recall".into(),
            format!(r#"{{"id":{id}}}"#),
        ));
    }
    script.push(Scripted::Text("done".into()));

    let sink = Arc::new(StressSink::default());
    // keep_recent: 1 means each turn's blob offloads once the next arrives, so by
    // the end ids 1..=BLOBS-1 are in the store. high_water_pct 2.0 disables
    // compaction so this test isolates the offload+window path.
    let mut ctx = CuratedContext::new(Message::system("SYS"), store.clone(), flag)
        .with_offload_config(OffloadConfig { keep_recent: 1, ..Default::default() })
        .with_high_water_pct(2.0);

    let agent = AgentLoop::new(
        Arc::new(ScriptedModel::new(script)),
        Arc::new(NativeProtocol),
        Arc::new(reg),
        Arc::new(RulePolicy { workspace: ws.clone(), command_allowlist: vec![], command_denylist: vec![] }),
        Arc::new(AlwaysApprove),
        sink.clone(),
        LoopConfig {
            model_limit: MODEL_LIMIT,
            max_turns: (BLOBS as usize) + recall_ids.len() + 4,
            max_retries: 1,
            temperature: 0.0,
            max_tokens: Some(128),
            workspace: ws.clone(),
            tool_timeout: Duration::from_secs(30),
            stream_idle_timeout: Duration::from_secs(120),
            ..Default::default()
        },
    );
    agent.run(&mut ctx, "stress the context window".into()).await.unwrap();

    // INVARIANT 1: the window never blew the budget, despite 60 x ~1.4KB outputs.
    let peak = *sink.max_prompt_tokens.lock().unwrap();
    assert!(peak > 0, "we should have seen Usage events");
    assert!(
        peak <= MODEL_LIMIT,
        "prompt sent to model ({peak} tok) must never exceed model_limit ({MODEL_LIMIT})"
    );

    // INVARIANT 2: lots actually got offloaded (not a trivially-passing test).
    assert!(store.len() >= (BLOBS as usize) - 2, "most blobs should have offloaded; got {}", store.len());

    // INVARIANT 3: each interleaved recall returned the exact bytes for that id.
    let recalls = sink.recalls.lock().unwrap().clone();
    assert_eq!(recalls.len(), recall_ids.len(), "every recall produced a result");
    for (slot, id) in recall_ids.iter().enumerate() {
        let marker = format!("<<n={id}>>");
        assert!(
            recalls[slot].contains(&marker),
            "recall #{id} must return its exact blob (looking for {marker})"
        );
    }
}

fn maint<'a>(
    model: &'a Arc<dyn ModelClient>,
    sink: &'a Arc<dyn EventSink>,
    cancel: &'a tokio_util::sync::CancellationToken,
    limit: usize,
) -> MaintCtx<'a> {
    MaintCtx { model_limit: limit, model, sink, cancel }
}

struct NullSink;
impl EventSink for NullSink {
    fn emit(&self, _e: AgentEvent) {}
}

/// 2. DIRECT VOLUME STRESS — hundreds of offload cycles straight at the context.
/// Invariants: build() stays within budget, every offloaded entry is recoverable
/// by id with exact bytes, and tool<->result linkage is never corrupted.
#[tokio::test]
async fn offload_table_recovers_every_entry_over_hundreds_of_cycles() {
    const TURNS: u64 = 500;
    const MODEL_LIMIT: usize = 6000;

    let store: Arc<dyn OffloadStore> = Arc::new(InMemoryOffloadStore::new());
    let flag = Arc::new(AtomicBool::new(false));
    let mut ctx = CuratedContext::new(Message::system("SYS"), store.clone(), flag)
        .with_offload_config(OffloadConfig { keep_recent: 0, error_min_bytes: 20, ..Default::default() })
        .with_high_water_pct(2.0); // never compact — isolate the offload path

    // No compaction => the model is never called.
    let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![]));
    let sink: Arc<dyn EventSink> = Arc::new(NullSink);
    let cancel = tokio_util::sync::CancellationToken::new();

    let mut total_offloaded = 0usize;
    for i in 1..=TURNS {
        let id = format!("call-{i}");
        // A realistic assistant->tool pair: assistant message carries the tool_call,
        // the tool message carries a large, uniquely-marked error body.
        ctx.append(Message::assistant(
            format!("calling blob {i}"),
            Some(vec![agent_tools::ToolCall {
                id: id.clone(),
                name: "blob".into(),
                args: serde_json::json!({ "n": i }),
            }]),
        ));
        ctx.append(Message::tool(id, "blob", format!("ERROR: {}", blob_body(i))));

        let report: MaintReport = ctx.maintain(&maint(&model, &sink, &cancel, MODEL_LIMIT)).await;
        total_offloaded += report.offloaded;

        // INVARIANT: build() never exceeds the budget, on every single turn.
        let built = ctx.build(MODEL_LIMIT);
        assert!(
            built_tokens(&built) <= MODEL_LIMIT,
            "turn {i}: built {} tok exceeds limit {MODEL_LIMIT}",
            built_tokens(&built)
        );
    }

    // INVARIANT: every blob was offloaded exactly once and the store holds them all.
    assert_eq!(total_offloaded, TURNS as usize, "each turn must offload its one large result");
    assert_eq!(store.len(), TURNS as usize, "store must hold every offloaded entry");

    // INVARIANT: every entry recovers to its exact bytes (spot-check the full range).
    for i in 1..=TURNS {
        let entry = store.get(i).unwrap_or_else(|| panic!("entry #{i} vanished"));
        assert_eq!(entry.content, format!("ERROR: {}", blob_body(i)), "entry #{i} corrupted");
    }

    // INVARIANT: linkage intact — every tool message kept its id+name; for each one
    // there is an assistant turn whose tool_calls reference that id.
    let all = ctx.build(usize::MAX);
    let assistant_ids: HashSet<String> = all
        .iter()
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .map(|c| c.id.clone())
        .collect();
    for m in all.iter().filter(|m| matches!(m.role, Role::Tool)) {
        let tcid = m.tool_call_id.as_ref().expect("tool message kept its tool_call_id");
        assert!(m.name.is_some(), "tool message kept its name");
        assert!(assistant_ids.contains(tcid), "tool result {tcid} still paired to an assistant call");
    }
}

/// 3. COMPACTION STRESS — force compaction every turn for 100 turns.
/// Invariants: history stays bounded (it doesn't grow without limit), the summary
/// is always present, the newest turns survive verbatim, and nothing panics.
#[tokio::test]
async fn repeated_compaction_keeps_history_bounded_and_coherent() {
    const TURNS: usize = 100;
    const MODEL_LIMIT: usize = 50_000;
    const KEEP: usize = 2;

    let store: Arc<dyn OffloadStore> = Arc::new(InMemoryOffloadStore::new());
    let flag = Arc::new(AtomicBool::new(false));
    let mut ctx = CuratedContext::new(Message::system("SYS"), store, flag)
        .with_offload_config(OffloadConfig { keep_recent: KEEP, ..Default::default() })
        .with_high_water_pct(0.0); // over high-water every turn => always try to compact

    // A scripted model that hands back a short, non-empty summary every time it is
    // called — over-provisioned so the deque never drains across the run.
    let model: Arc<dyn ModelClient> = Arc::new(ScriptedModel::new(vec![
        Scripted::Text("compact summary of prior turns".into());
        TURNS + 20
    ]));
    let sink: Arc<dyn EventSink> = Arc::new(NullSink);
    let cancel = tokio_util::sync::CancellationToken::new();

    let mut compactions = 0usize;
    for i in 0..TURNS {
        // A reasonably large user+assistant turn so the compaction is a net win.
        ctx.append(Message::user(format!("turn {i}: {}", "detail ".repeat(40))));
        ctx.append(Message::assistant(format!("ack {i}: {}", "reasoning ".repeat(40)), None));
        let report = ctx.maintain(&maint(&model, &sink, &cancel, MODEL_LIMIT)).await;
        if report.compacted_turns > 0 {
            compactions += 1;
        }

        // INVARIANT: build() never exceeds the budget.
        let built = ctx.build(MODEL_LIMIT);
        assert!(built_tokens(&built) <= MODEL_LIMIT, "turn {i} exceeded budget");
    }

    // Compaction actually happened repeatedly (not a no-op test).
    assert!(compactions >= 50, "compaction should fire most turns; got {compactions}");

    let built = ctx.build(MODEL_LIMIT);
    // INVARIANT: a summary block is pinned in.
    assert!(
        built.iter().any(|m| m.content.contains("compact summary of prior turns")),
        "a compaction summary must be present"
    );
    // INVARIANT: the very last turn survived verbatim (within keep_recent).
    assert!(
        built.iter().any(|m| m.content.starts_with(&format!("ack {}", TURNS - 1))),
        "the newest assistant turn must be kept verbatim"
    );
    // INVARIANT: the live context never exceeds the token budget — this, not message
    // count, is the real "bounded" guarantee (build() truncates newest-first).
    assert!(built_tokens(&built) <= MODEL_LIMIT, "built context must fit the budget");
    // INVARIANT: assistant/tool CHATTER is collapsed into the summary rather than
    // accumulating — only the keep_recent newest non-user turns survive verbatim.
    let assistant_kept = built.iter().filter(|m| matches!(m.role, Role::Assistant)).count();
    assert!(
        assistant_kept <= KEEP + 2,
        "assistant chatter must be summarized away, not accumulate; kept {assistant_kept}"
    );
    // INVARIANT: user instructions are DURABLE — kept verbatim across every compaction,
    // never routed through the lossy summarizer. Both the first and last survive.
    assert!(
        built.iter().any(|m| m.content.starts_with("turn 0:")),
        "the earliest user instruction must survive compaction verbatim"
    );
    assert!(
        built.iter().any(|m| m.content.starts_with(&format!("turn {}:", TURNS - 1))),
        "the latest user instruction must survive compaction verbatim"
    );
}

/// 4. CONCURRENCY STRESS — 16 tasks pounding the shared store at once.
/// Invariants: ids are globally unique, nothing is lost, every entry reads back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn store_is_safe_and_lossless_under_concurrent_writers() {
    const WRITERS: u64 = 16;
    const PER_WRITER: u64 = 200;

    let store: Arc<dyn OffloadStore> = Arc::new(InMemoryOffloadStore::new());
    let mut handles = Vec::new();
    for w in 0..WRITERS {
        let s = store.clone();
        handles.push(tokio::spawn(async move {
            let mut mine = Vec::new();
            for k in 0..PER_WRITER {
                let content = format!("w{w}-k{k}");
                let id = s.put(OffloadEntry {
                    id: 0,
                    tool_call_id: format!("w{w}k{k}"),
                    tool_name: "blob".into(),
                    kind: OffloadKind::Output,
                    content: content.clone(),
                    bytes: content.len(),
                    turn: 0,
                });
                mine.push((id, content));
            }
            mine
        }));
    }

    let mut all = Vec::new();
    for h in handles {
        all.extend(h.await.unwrap());
    }

    let total = (WRITERS * PER_WRITER) as usize;
    // INVARIANT: no id collisions across threads.
    let ids: HashSet<u64> = all.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids.len(), total, "every put must get a unique id (no races)");
    assert_eq!(store.len(), total, "store must hold every concurrent write");
    // INVARIANT: every write reads back with the exact content the writer stored.
    for (id, content) in &all {
        assert_eq!(store.get(*id).unwrap().content, *content, "entry #{id} corrupted under contention");
    }
}

/// 5. LIVE STRESS — the same machinery, but driven by a real model instead of a
/// script. Opt-in (needs a running OpenAI-compatible server). Run with:
///   AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
///     cargo test -p agent-runtime-config --test stress_context_management \
///     live_window_stays_bounded -- --ignored --nocapture
///
/// The model is asked to emit a stream of large `blob` outputs and then recall an
/// early one. Whatever the model does, the HARD invariant must hold: the prompt
/// the loop builds each turn never exceeds `model_limit`. Volume + exact-recall
/// are asserted with a model-behaviour caveat (inconclusive, not a loop bug).
#[tokio::test]
#[ignore = "requires AGENT_E2E_URL / AGENT_E2E_MODEL and a live server"]
async fn live_window_stays_bounded_under_model_driven_volume() {
    let url = std::env::var("AGENT_E2E_URL").expect("set AGENT_E2E_URL");
    let model_name = std::env::var("AGENT_E2E_MODEL").expect("set AGENT_E2E_MODEL");
    // Small limit so even modest live output forces the window to evict + offload.
    const MODEL_LIMIT: usize = 3000;
    const ASK: u64 = 12;

    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    let store: Arc<dyn OffloadStore> = Arc::new(InMemoryOffloadStore::new());
    let flag = Arc::new(AtomicBool::new(false));

    let mut reg = ToolRegistry::new();
    reg.register(Arc::new(BlobTool));
    reg.register(Arc::new(ContextRecallTool::new(store.clone())));

    let sink = Arc::new(StressSink::default());
    // keep_recent 2; compaction disabled so the live model only drives blob/recall
    // and the offload+window path is what's under stress.
    let mut ctx = CuratedContext::new(Message::system(
        "You are a tool-using agent. The `blob` tool returns a large block of text \
         for a given integer `n`. When you call a tool, wait for its result.",
    ), store.clone(), flag)
        .with_offload_config(OffloadConfig { keep_recent: 2, ..Default::default() })
        .with_high_water_pct(2.0);

    let agent = AgentLoop::new(
        Arc::new(OpenAiCompatClient::new(url, model_name, std::env::var("AGENT_API_KEY").ok())),
        Arc::new(NativeProtocol),
        Arc::new(reg),
        Arc::new(RulePolicy { workspace: ws.clone(), command_allowlist: vec![], command_denylist: vec![] }),
        Arc::new(AlwaysApprove),
        sink.clone(),
        LoopConfig {
            model_limit: MODEL_LIMIT,
            max_turns: (ASK as usize) + 12,
            max_retries: 2,
            temperature: 0.0,
            max_tokens: Some(256),
            workspace: ws.clone(),
            tool_timeout: Duration::from_secs(60),
            stream_idle_timeout: Duration::from_secs(120),
            ..Default::default()
        },
    );

    agent
        .run(
            &mut ctx,
            format!(
                "Call the `blob` tool {ASK} times, once per turn, with n = 1, 2, 3, … up to {ASK}. \
                 After all {ASK} calls are done, call `context_recall` with id 1 to pull back the \
                 first blob, and reply with the exact `<<n=...>>` marker you find in it."
            ),
        )
        .await
        .expect("the run must not error out");

    let peak = *sink.max_prompt_tokens.lock().unwrap();
    let blobs = *sink.blob_results.lock().unwrap();
    let recalls = sink.recalls.lock().unwrap().clone();
    let offloaded = store.len();
    eprintln!(
        "[live stress] peak_prompt_tokens={peak} blob_calls={blobs} offloaded={offloaded} \
         recalls={} done={}",
        recalls.len(),
        *sink.done.lock().unwrap(),
    );

    // HARD INVARIANT (our guarantee, independent of model behaviour): the prompt
    // built each turn never exceeded the model limit, even as large outputs piled up.
    assert!(peak > 0, "expected at least one Usage event");
    assert!(
        peak <= MODEL_LIMIT,
        "window must stay bounded: peak prompt {peak} tok > model_limit {MODEL_LIMIT}"
    );

    // VOLUME (model behaviour): we need real offloading to have occurred for this to
    // be a meaningful stress; if the model under-produced, that's inconclusive.
    assert!(
        offloaded >= 3,
        "INCONCLUSIVE: model produced too little to stress offload (offloaded={offloaded}, \
         blob_calls={blobs}); re-run or adjust the prompt. Not a loop bug."
    );

    // EXACT RECALL (only if the model chose to recall): bytes must be exact.
    if let Some(first) = recalls.first() {
        assert!(
            first.contains("<<n=1>>") || first.contains("no offloaded entry"),
            "a recall result must be either the exact offloaded blob or a clean not-found, got: {}",
            &first.chars().take(80).collect::<String>()
        );
    }
}
