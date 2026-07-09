//! LIVE SOAK TEST — drive the full, real `assemble_loop` against a live model for
//! a sustained run (~5 min) and verify every component keeps working correctly
//! under a long, nondeterministic conversation.
//!
//! Exercised end-to-end: the agent loop, streaming model client, the policy +
//! approval gate, the sandbox (host mode), the real fs/shell tools, AND the whole
//! context-management subsystem (offload → placeholder, compaction, re-grounding,
//! context_compact) over a single growing `CuratedContext`.
//!
//! Safety: runs in a throwaway temp workspace, host sandbox, and a `SafeApproval`
//! gate that only permits read-only / workspace-bounded operations no matter what
//! the model asks. Opt-in (needs a live server). Run with:
//!   AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
//!   SOAK_SECS=300 cargo test -p agent-runtime-config --test soak_live \
//!     -- --ignored --nocapture

use agent_core::{AgentEvent, ContextEvent, CuratedContext, EventSink, SessionArtifacts};
use agent_model::{Message, OpenAiCompatClient};
use agent_policy::{ApprovalChannel, ApprovalRequest, ApprovalResponse};
use agent_runtime_config::{assemble_loop, LoopParts, RuntimeConfig};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Approval gate that bounds the blast radius regardless of model behaviour:
/// workspace-bounded fs + context tools are allowed; `execute_command` is allowed
/// only for a fixed set of read-only commands; everything else is denied.
struct SafeApproval {
    denied: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl ApprovalChannel for SafeApproval {
    async fn request(&self, r: ApprovalRequest) -> ApprovalResponse {
        let allow = match r.intent.tool.as_str() {
            "read_file" | "list_directory" | "write_file" | "edit_file" | "render"
            | "git_status" | "git_diff" | "grep" | "context_compact" => true,
            "execute_command" => r
                .intent
                .command
                .as_deref()
                .map(|c| {
                    let first = c.split_whitespace().next().unwrap_or("");
                    let base = first.rsplit('/').next().unwrap_or(first);
                    matches!(
                        base,
                        "ls" | "cat"
                            | "wc"
                            | "head"
                            | "tail"
                            | "echo"
                            | "grep"
                            | "find"
                            | "pwd"
                            | "sort"
                            | "uniq"
                            | "true"
                            | "date"
                            | "nl"
                    )
                })
                .unwrap_or(false),
            _ => false,
        };
        if allow {
            ApprovalResponse::Approve
        } else {
            self.denied
                .lock()
                .unwrap()
                .push(format!("{}:{:?}", r.intent.tool, r.intent.command));
            ApprovalResponse::Deny
        }
    }
}

/// Aggregates everything the loop emits so we can verify each component worked.
#[derive(Default)]
struct SoakMonitor {
    turns: AtomicUsize, // one Usage event per model turn
    peak_prompt: AtomicUsize,
    offloads: AtomicUsize,
    offload_bytes: AtomicUsize,
    compactions: AtomicUsize,
    compaction_fails: AtomicUsize,
    recalls: AtomicUsize,
    done: AtomicUsize,
    tool_results: Mutex<BTreeMap<String, usize>>,
    errors: Mutex<Vec<String>>,
}
impl EventSink for SoakMonitor {
    fn emit(&self, e: AgentEvent) {
        match e {
            AgentEvent::Usage { prompt_tokens, .. } => {
                self.turns.fetch_add(1, Ordering::Relaxed);
                self.peak_prompt.fetch_max(prompt_tokens, Ordering::Relaxed);
            }
            AgentEvent::ToolResult { name, .. } => {
                // Offload recovery now goes through read_file (Phase 2); count it
                // as the recall proxy for the diagnostic report.
                if name == "read_file" {
                    self.recalls.fetch_add(1, Ordering::Relaxed);
                }
                *self.tool_results.lock().unwrap().entry(name).or_insert(0) += 1;
            }
            AgentEvent::Error(m) => self.errors.lock().unwrap().push(m),
            AgentEvent::Done(_) => {
                self.done.fetch_add(1, Ordering::Relaxed);
            }
            AgentEvent::Context(ContextEvent::Offloaded { bytes, .. }) => {
                self.offloads.fetch_add(1, Ordering::Relaxed);
                self.offload_bytes.fetch_add(bytes, Ordering::Relaxed);
            }
            AgentEvent::Context(ContextEvent::Compacted { .. }) => {
                self.compactions.fetch_add(1, Ordering::Relaxed);
            }
            AgentEvent::Context(ContextEvent::CompactionFailed { reason }) => {
                self.compaction_fails.fetch_add(1, Ordering::Relaxed);
                self.errors
                    .lock()
                    .unwrap()
                    .push(format!("compaction_failed: {reason}"));
            }
            _ => {}
        }
    }
}

/// Count of keys in the results store (successor of `store.len()`).
async fn results_count(artifacts: &Arc<SessionArtifacts>) -> usize {
    artifacts.results.ls("").await.unwrap().len()
}

fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

/// A varied, tool-exercising task. `store_len` gates recall coverage (only ask
/// once something has actually been offloaded).
// Legacy lint, unrelated to this branch.
#[allow(clippy::manual_is_multiple_of)]
fn make_task(rng: &mut u64, i: usize, store_len: usize) -> String {
    // Guarantee recall coverage: every 5th step (once something is offloaded),
    // ask the model to pull an offloaded artifact back. Otherwise pick at random.
    if store_len > 0 && i % 5 == 0 {
        return "Some earlier tool output was offloaded out of context, leaving a \
             [tool_result offloaded to large_tool_results/…] placeholder. read_file the \
             large_tool_results/ path named in the placeholder (or grep large_tool_results/ \
             to find it) and quote the first 40 characters of what you get back."
            .into();
    }
    let pick = lcg(rng) % 8;
    let f = 1 + (lcg(rng) % 3); // big1..big3
    match pick {
        0 => format!("Read the file big{f}.txt with read_file and report its final line verbatim."),
        1 => "Use list_directory on '.' then tell me how many files are in the workspace.".into(),
        2 => format!("Use execute_command to run `wc -l big{f}.txt` and report just the number."),
        3 => format!(
            "Use write_file to create note_{i}.txt containing one sentence describing step {i}, \
             then confirm the byte count."
        ),
        4 => "Call read_file for big1.txt, big2.txt and big3.txt in the SAME turn (parallel), \
              then give me the first line of each."
            .into(),
        5 => "Use execute_command to run `head -3 data.csv` and report the rows.".into(),
        6 if store_len > 0 => "Some earlier tool output was offloaded out of context, leaving a \
             [tool_result offloaded to large_tool_results/…] placeholder. read_file the \
             large_tool_results/ path named in the placeholder (or grep large_tool_results/ \
             to find it) and quote the first 40 characters of what you get back."
            .into(),
        7 => "This conversation is getting long. Call context_compact to compress older history, \
              then briefly confirm you did."
            .into(),
        _ => format!("Read big{f}.txt and summarize it in one sentence."),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live soak: requires AGENT_E2E_URL / AGENT_E2E_MODEL and a live server"]
async fn soak_all_components_live() {
    let url = std::env::var("AGENT_E2E_URL").expect("set AGENT_E2E_URL");
    let model_name = std::env::var("AGENT_E2E_MODEL").expect("set AGENT_E2E_MODEL");
    let budget = Duration::from_secs(
        std::env::var("SOAK_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300),
    );
    const CONTEXT_LIMIT: usize = 4000; // small enough that offload + compaction both fire

    // Throwaway workspace, seeded with files large enough to be offloaded when read.
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    for n in 1..=3 {
        let body: String = (1..=80)
            .map(|l| format!("big{n} line {l}: the quick brown fox jumps over the lazy dog\n"))
            .collect();
        std::fs::write(
            ws.join(format!("big{n}.txt")),
            format!("{body}LAST-LINE-big{n}\n"),
        )
        .unwrap();
    }
    std::fs::write(
        ws.join("data.csv"),
        "id,name,score\n1,alpha,10\n2,bravo,20\n3,charlie,30\n4,delta,40\n",
    )
    .unwrap();

    let mut cfg = RuntimeConfig::from_launch(
        "openai".into(),
        url.clone(),
        model_name.clone(),
        "native".into(),
        CONTEXT_LIMIT,
    );
    cfg.memory = false;
    cfg.sandbox_mode = "off".into(); // host executor — no docker needed
    cfg.max_turns = 8; // bound a single task

    let artifacts = Arc::new(SessionArtifacts::new());
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let todos = Arc::new(std::sync::Mutex::new(Vec::new()));
    let monitor = Arc::new(SoakMonitor::default());
    let approval = Arc::new(SafeApproval {
        denied: Mutex::new(Vec::new()),
    });

    let built = assemble_loop(
        &cfg,
        LoopParts {
            model: Arc::new(OpenAiCompatClient::new(
                url,
                model_name,
                std::env::var("AGENT_API_KEY").ok(),
            )),
            sink: monitor.clone(),
            approval: approval.clone(),
            workspace: ws.clone(),
            mcp_tools: vec![],
            memory_tools: vec![],
            memory_retriever: None,
            stream_idle_timeout: Duration::from_secs(120),
            base_system_prompt:
                "You are a coding agent operating in a sandboxed workspace. Use the \
                provided tools (read_file, list_directory, write_file, execute_command, \
                grep, context_compact) to complete each task, then give a short final \
                reply. Offloaded tool output is recovered with read_file/grep over the \
                large_tool_results/ path in its placeholder. Keep replies concise."
                    .into(),
            artifacts: artifacts.clone(),
            compact_flag: flag.clone(),
            todos: todos.clone(),
            sandbox: agent_runtime_config::build_sandbox(&cfg),
            stats: Arc::new(std::sync::RwLock::new(agent_core::SessionStats::default())),
            trace: None,
            api_key: None,
            claude_binary: "claude".into(),
        },
    );
    let agent = built.loop_;

    // One long-lived conversation so context management accumulates across tasks.
    let mut ctx = CuratedContext::new(
        Message::system(built.system_prompt),
        artifacts.clone(),
        flag,
    )
    .with_recall_budget(256);

    let mut rng = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u64
        | 1;
    let start = Instant::now();
    let mut tasks = 0usize;
    let mut task_timeouts = 0usize;

    eprintln!(
        "[soak] starting; budget={}s context_limit={CONTEXT_LIMIT}",
        budget.as_secs()
    );
    while start.elapsed() < budget {
        tasks += 1;
        let store_len = results_count(&artifacts).await;
        let task = make_task(&mut rng, tasks, store_len);
        let cancel = tokio_util::sync::CancellationToken::new();
        // Bound each task so one stuck turn can't eat the whole budget.
        let run = agent.run_with_cancel(&mut ctx, task.clone(), cancel.clone());
        let outcome = tokio::time::timeout(Duration::from_secs(90), run).await;
        match outcome {
            Ok(Ok(())) => {}
            Ok(Err(e)) => monitor.errors.lock().unwrap().push(format!("run: {e}")),
            Err(_) => {
                cancel.cancel();
                task_timeouts += 1;
                monitor.errors.lock().unwrap().push("task_timeout".into());
            }
        }

        // Heartbeat: one line per task so the run can be watched live (--nocapture).
        eprintln!(
            "[hb] t={:>3}s task={:<3} turns={} offloads={} compactions={} recalls={} \
             peak={}/{} store={} errs={} | {}",
            start.elapsed().as_secs(),
            tasks,
            monitor.turns.load(Ordering::Relaxed),
            monitor.offloads.load(Ordering::Relaxed),
            monitor.compactions.load(Ordering::Relaxed),
            monitor.recalls.load(Ordering::Relaxed),
            monitor.peak_prompt.load(Ordering::Relaxed),
            CONTEXT_LIMIT,
            store_len,
            monitor.errors.lock().unwrap().len(),
            task.chars().take(48).collect::<String>(),
        );
    }

    // ---- Final report ----
    let turns = monitor.turns.load(Ordering::Relaxed);
    let peak = monitor.peak_prompt.load(Ordering::Relaxed);
    let offloads = monitor.offloads.load(Ordering::Relaxed);
    let offload_kb = monitor.offload_bytes.load(Ordering::Relaxed) / 1024;
    let compactions = monitor.compactions.load(Ordering::Relaxed);
    let compaction_fails = monitor.compaction_fails.load(Ordering::Relaxed);
    let recalls = monitor.recalls.load(Ordering::Relaxed);
    let done = monitor.done.load(Ordering::Relaxed);
    let tool_results = monitor.tool_results.lock().unwrap().clone();
    let errors = monitor.errors.lock().unwrap().clone();
    let denied = approval.denied.lock().unwrap().clone();

    eprintln!("\n================ SOAK REPORT ================");
    eprintln!("elapsed         : {}s", start.elapsed().as_secs());
    eprintln!("tasks started   : {tasks}  (completed={done}, timeouts={task_timeouts})");
    eprintln!("model turns      : {turns}");
    eprintln!("peak prompt tok  : {peak} / {CONTEXT_LIMIT}");
    let store_entries = results_count(&artifacts).await;
    eprintln!("offloads         : {offloads}  ({offload_kb} KB lifted out)");
    eprintln!("store entries    : {store_entries}");
    eprintln!("compactions      : {compactions}  (failed={compaction_fails})");
    eprintln!("read_file recalls: {recalls}");
    eprintln!("tool results     : {tool_results:?}");
    eprintln!(
        "policy-denied    : {} {:?}",
        denied.len(),
        &denied[..denied.len().min(5)]
    );
    eprintln!(
        "errors           : {} {:?}",
        errors.len(),
        &errors[..errors.len().min(5)]
    );

    // Recoverability spot-check: a sample of offloaded artifacts still read back intact.
    let sample_keys: Vec<String> = artifacts
        .results
        .ls("")
        .await
        .unwrap()
        .into_iter()
        .take(8)
        .map(|e| e.name)
        .collect();
    let mut recovered = 0;
    for key in &sample_keys {
        if artifacts
            .results
            .read(key)
            .await
            .map(|c| !c.is_empty())
            .unwrap_or(false)
        {
            recovered += 1;
        }
    }
    eprintln!("recoverable(<=8) : {recovered}");
    eprintln!("=============================================\n");

    // ---- Invariants ----
    // HARD: the window never blew the budget across the whole soak.
    assert!(peak > 0, "expected model turns to have happened");
    assert!(
        peak <= CONTEXT_LIMIT,
        "peak prompt {peak} exceeded context_limit {CONTEXT_LIMIT}"
    );
    // The soak actually did sustained work.
    assert!(turns >= 20, "expected a sustained run; only {turns} turns");
    assert!(tasks >= 5, "expected several tasks; only {tasks}");
    // Components actually engaged: real tools ran, and offloading happened.
    assert!(
        tool_results.values().sum::<usize>() >= 5,
        "expected real tool activity; got {tool_results:?}"
    );
    assert!(
        offloads >= 1,
        "expected the offload path to engage over a long run"
    );
    // Everything offloaded is still recoverable.
    assert_eq!(
        recovered,
        sample_keys.len(),
        "every sampled offloaded entry must still read back"
    );
    // No compaction should have hard-failed.
    assert_eq!(
        compaction_fails, 0,
        "compaction failures: {compaction_fails}"
    );
    // The safety gate held: nothing outside the allow-set executed.
    assert!(
        !tool_results
            .keys()
            .any(|k| k == "git_commit" || k == "fetch_url"),
        "a disallowed tool produced a result: {tool_results:?}"
    );
}
