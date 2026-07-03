# Post-execution Validator Hook Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A config-driven, once-per-turn post-execution validator hook: after a turn where a mutating tool succeeded, run configured shell commands through the sandbox and append any failures to the model's context as feedback.

**Architecture:** `RuntimeConfig.post_tool_validators: Vec<String>` → `LoopConfig` (Task 1). A sink-free `run_validator` helper + turn-loop integration that triggers on successful Write/Destroy calls, emits synthetic `post_tool_validate` tool events, and appends a failure message (Task 2). Default empty = fully disabled.

**Tech Stack:** Rust (`agent/` workspace), tokio, serde.

**Spec:** `docs/superpowers/specs/2026-07-02-post-tool-validator-hook-design.md`

## Global Constraints

- `agent/` Cargo workspace (`cd agent`; `source ~/.cargo/env` if needed). ALWAYS run `cargo fmt` before committing (CI's `fmt --check` is a gate).
- Additive only: serde-default config field; NO new event kinds (reuse ToolStart/ToolResult); old-SPA wire-compat holds.
- Validation is best-effort: a runner error NEVER fails the run.
- Default `post_tool_validators` empty → zero behavior change (regression-pinned).
- Conventional commits.
- Line numbers are from 2026-07-02 main post-cluster-C; re-locate by anchor text if drifted.

---

### Task 1: config plumbing

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (struct, PartialRuntimeConfig, base constructor, merge, tests)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (`loop_config_from` + its test)
- Modify: `agent/crates/agent-core/src/loop_.rs` (`LoopConfig` struct + `Default`)

**Interfaces:**
- Produces: `RuntimeConfig.post_tool_validators: Vec<String>`, `LoopConfig.post_tool_validators: Vec<String>`.

- [ ] **Step 1: Write failing config tests**

In `runtime_config.rs` tests (mirroring `max_parallel_tools_defaults_and_merges`):

```rust
#[test]
fn post_tool_validators_default_empty_and_merge() {
    let mut v = serde_json::to_value(base()).unwrap();
    v.as_object_mut().unwrap().remove("post_tool_validators");
    let parsed: RuntimeConfig = serde_json::from_value(v).unwrap();
    assert!(parsed.post_tool_validators.is_empty(), "serde default is empty");

    let merged = base().merge(
        serde_json::from_str::<PartialRuntimeConfig>(
            r#"{"post_tool_validators": ["cargo check"]}"#,
        )
        .unwrap(),
    );
    assert_eq!(merged.post_tool_validators, vec!["cargo check".to_string()]);
}
```

(Use the file's actual base-config helper — `base()` — as the sibling tests do.)

- [ ] **Step 2: Verify failure**

Run: `cd agent && cargo test -p agent-runtime-config post_tool_validators`
Expected: compile error (no field).

- [ ] **Step 3: Implement config field**

`runtime_config.rs`:
```rust
// struct RuntimeConfig, near max_parallel_tools:
    /// Shell commands run once after any turn in which a mutating (Write/Destroy)
    /// tool call succeeded; failures are fed back to the model. Empty = disabled.
    #[serde(default)]
    pub post_tool_validators: Vec<String>,

// PartialRuntimeConfig:
    post_tool_validators: Option<Vec<String>>,

// the flag-derived base constructor (lists every field):
    post_tool_validators: Vec::new(),

// merge():
    if let Some(v) = p.post_tool_validators {
        self.post_tool_validators = v;
    }
```
(No `validate()` rule — any string list is legal; an unspawnable command is handled at runtime as `Skipped`.)

`agent-core/src/loop_.rs` — `LoopConfig` struct + its `Default`:
```rust
// struct LoopConfig, near max_parallel_tools:
    /// Shell commands run after a mutating turn (see RuntimeConfig). Empty = off.
    pub post_tool_validators: Vec<String>,

// impl Default for LoopConfig:
    post_tool_validators: Vec::new(),
```

`assemble.rs::loop_config_from` — add to the constructed `LoopConfig`:
```rust
        post_tool_validators: cfg.post_tool_validators.clone(),
```

- [ ] **Step 4: Extend the assemble passthrough test**

In `assemble.rs` tests, alongside the `max_parallel_tools` passthrough pin:
```rust
    let mut cfg3 = cfg.clone();
    cfg3.post_tool_validators = vec!["cargo check".into()];
    assert_eq!(
        loop_config_from(&cfg3, /* same trailing args as the existing call */)
            .post_tool_validators,
        vec!["cargo check".to_string()]
    );
```

- [ ] **Step 5: fmt + test + commit**

```bash
cd agent && cargo fmt && cargo test -p agent-runtime-config && cargo test -p agent-core --lib
git add agent/crates/agent-runtime-config/src/runtime_config.rs agent/crates/agent-runtime-config/src/assemble.rs agent/crates/agent-core/src/loop_.rs
git commit -m "feat(config): post_tool_validators field (RuntimeConfig + LoopConfig)"
```
Expected: all green (empty default changes nothing).

---

### Task 2: validator runner + turn-loop integration

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs` (ReadyCall.access, executed tuple, `run_validator` helper + `ValidatorOutcome`, the post-Phase-3 integration, tests)

**Interfaces:**
- Consumes: `LoopConfig.post_tool_validators` (Task 1); `agent_tools::{Access, CommandSpec, ProcKind, SandboxError, SandboxStrategy}`; `self.config.sandbox`, `self.config.tool_timeout`, `self.config.workspace`.
- Produces: `enum ValidatorOutcome { Passed, Failed { code: i32, output: String }, Skipped { reason: String } }`; `async fn run_validator(sandbox: &Arc<dyn SandboxStrategy>, workspace: &Path, command: &str, timeout: Duration, cancel: &CancellationToken) -> ValidatorOutcome`.

- [ ] **Step 1: Write the failing tests**

In the loop_.rs test module. A small helper builds a loop with a given validator list and a scripted model that issues one tool call then finishes. Reuse the `server_usage_event_carries_token_totals` construction shape; add `post_tool_validators: vec![...]` to the `LoopConfig`. Use `DetailSink` (records tool_results as `(id, status, content)`) — extend it if needed to also capture appended context, OR assert via a `CollectingSink` on the `post_tool_validate` tool events, plus inspect `ctx` messages after the run for the appended validation message.

Tests to write (write the sketches as full code following the harness):

```rust
// A write-tier tool that succeeds, then a failing validator -> validation
// message appended + a post_tool_validate Error event.
#[tokio::test]
async fn failing_validator_appends_feedback_and_emits_event() { /*
  workspace with a file; ScriptedModel: [Call(write_file-like OR a Write-access
  stub tool), Text("done")]; post_tool_validators: vec!["sh -c 'echo boom >&2; exit 1'".into()]
  (note: command is already sh -c'd by run_validator, so use just "false" or
  "echo boom 1>&2; exit 3"); assert a post_tool_validate ToolResult with Error
  status is emitted AND the context's last user message contains "boom" or the
  exit code. */ }

#[tokio::test]
async fn passing_validator_emits_event_but_appends_nothing() { /* validators:
  ["true"]; assert a post_tool_validate Ok event emitted and NO extra user
  message appended (context user-message count unchanged from the no-validator
  baseline). */ }

#[tokio::test]
async fn read_only_turn_does_not_run_validators() { /* the only call is a
  read_file (Access::Read); validators: ["false"]; assert NO post_tool_validate
  event emitted at all. */ }

#[tokio::test]
async fn empty_validators_is_a_noop() { /* validators: vec![]; a write-tier
  call succeeds; assert NO post_tool_validate event, run behaves exactly as
  before. */ }

#[tokio::test]
async fn validator_helper_truncates_large_output() { /* unit-test run_validator
  directly with a command emitting >4 KiB (e.g. "yes x | head -c 20000"); assert
  Failed/Passed output length <= ~4 KiB and ends with the truncation marker.
  Use HostExecutor as the sandbox. */ }
```

For a deterministic Write-access tool in tests, either reuse an existing writing
tool in the registry or add a tiny test-only `Tool` whose `intent` returns
`Access::Write` and whose `execute` returns Ok (mirror the existing test tools
like `HangsUntilCancel` at ~loop_.rs:1385). Keep it local to the test module.

- [ ] **Step 2: Verify failure**

Run: `cd agent && cargo test -p agent-core validator`
Expected: FAIL (no `post_tool_validate` events; helper absent).

- [ ] **Step 3: Implement the helper + outcome**

Add near `execute_isolated` (loop_.rs ~1100):

```rust
/// Outcome of one post-execution validator command. Best-effort: a runner
/// failure is `Skipped`, never a run failure.
enum ValidatorOutcome {
    Passed,
    Failed { code: i32, output: String },
    Skipped { reason: String },
}

const VALIDATOR_OUTPUT_CAP: usize = 4096;

/// Run one validator command via `sh -c` through the sandbox, cwd = workspace.
/// Sink-free (caller owns event emission). Combined stdout+stderr capped to
/// VALIDATOR_OUTPUT_CAP (char-boundary safe). A degraded sandbox / spawn error /
/// cancellation yields `Skipped`.
async fn run_validator(
    sandbox: &Arc<dyn agent_tools::SandboxStrategy>,
    workspace: &std::path::Path,
    command: &str,
    timeout: Duration,
    cancel: &CancellationToken,
) -> ValidatorOutcome {
    use tokio::io::AsyncReadExt;
    if cancel.is_cancelled() {
        return ValidatorOutcome::Skipped { reason: "run cancelled".into() };
    }
    let spec = agent_tools::CommandSpec {
        program: "sh".into(),
        args: vec!["-c".into(), command.to_string()],
        cwd: workspace.to_path_buf(),
        env: Default::default(),
        kind: agent_tools::ProcKind::OneShot,
    };
    let mut child = match sandbox.launch(spec) {
        Ok(c) => c,
        Err(agent_tools::SandboxError::Unavailable(m)) => {
            return ValidatorOutcome::Skipped { reason: format!("sandbox refused: {m}") }
        }
        Err(e) => return ValidatorOutcome::Skipped { reason: e.to_string() },
    };
    let mut out_pipe = child.take_stdout();
    let mut err_pipe = child.take_stderr();
    let read_out = async {
        let mut s = String::new();
        if let Some(p) = out_pipe.as_mut() { let _ = p.read_to_string(&mut s).await; }
        s
    };
    let read_err = async {
        let mut s = String::new();
        if let Some(p) = err_pipe.as_mut() { let _ = p.read_to_string(&mut s).await; }
        s
    };
    let run = async {
        let (status, o, e) = tokio::join!(child.wait(), read_out, read_err);
        (status, o, e)
    };
    let (status, stdout, stderr) = tokio::select! {
        _ = cancel.cancelled() => return ValidatorOutcome::Skipped { reason: "run cancelled".into() },
        r = tokio::time::timeout(timeout, run) => match r {
            Ok(v) => v,
            Err(_) => return ValidatorOutcome::Skipped { reason: format!("validator exceeded {timeout:?}") },
        },
    };
    let mut combined = stdout;
    if !stderr.is_empty() {
        if !combined.is_empty() { combined.push('\n'); }
        combined.push_str(&stderr);
    }
    let output = truncate_on_char_boundary(&combined, VALIDATOR_OUTPUT_CAP);
    // status: Ok(ExitStatus). A signal-killed child has no code(); treat as failure.
    let code = status.ok().and_then(|s| s.code());
    match code {
        Some(0) => ValidatorOutcome::Passed,
        Some(c) => ValidatorOutcome::Failed { code: c, output },
        None => ValidatorOutcome::Failed { code: -1, output },
    }
}

/// Truncate to at most `cap` bytes on a char boundary, appending a marker when cut.
fn truncate_on_char_boundary(s: &str, cap: usize) -> String {
    if s.len() <= cap { return s.to_string(); }
    let mut end = cap;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    format!("{}\n…(truncated)", &s[..end])
}
```

Check `SandboxedChild::wait()`'s return type in `agent-tools/src/sandbox.rs` and
match it (the snippet assumes `Result<ExitStatus, _>` via `.ok()`; adjust the
`code` extraction to the real signature — shell.rs already consumes `wait()`, so
mirror how it reads the exit status).

- [ ] **Step 4: Thread access + integrate into the turn loop**

`ReadyCall` (loop_.rs:1051): add `access: agent_tools::Access,`. Populate it in
`gate_tool` where `ReadyCall` is built (loop_.rs:1040) from `intent.access`
(the `intent` is in scope there).

The Phase-2 `executed` collection (loop_.rs:751-771): carry access through. The
closure captures `rc` — add `let access = rc.access;` (Access is Copy) before the
`async move` and include it in the returned tuple `(id, name, ex, duration, timeout, access)`;
widen the `executed` type and the `for (id, name, ex, duration_ms, timeout, _access) in executed`
destructure accordingly. Before consuming `executed` in that loop, compute:

```rust
    let turn_mutated = executed.iter().any(|(_, _, ex, _, _, access)| {
        matches!(ex, Executed::Ok(_))
            && matches!(access, agent_tools::Access::Write | agent_tools::Access::Destroy)
    });
```

After the Phase-3 append loop (loop_.rs:849) and BEFORE the nudge append (855),
insert the validation pass:

```rust
    if turn_mutated && !self.config.post_tool_validators.is_empty() {
        let mut failures: Vec<String> = Vec::new();
        for (n, command) in self.config.post_tool_validators.iter().enumerate() {
            let vid = format!("validate:{}:{}", turn + 1, n);
            self.sink.emit(AgentEvent::ToolStart {
                id: vid.clone(),
                name: "post_tool_validate".into(),
                args: serde_json::json!({ "command": command }),
                parent_id: None,
            });
            let started = std::time::Instant::now();
            let outcome = run_validator(
                &self.config.sandbox,
                &self.config.workspace,
                command,
                self.config.tool_timeout,
                &cancel,
            )
            .await;
            let (status, content) = match &outcome {
                ValidatorOutcome::Passed => (ToolStatus::Ok, "validator passed".to_string()),
                ValidatorOutcome::Failed { code, output } => {
                    failures.push(format!("$ {command}  (exit {code})\n{output}"));
                    (ToolStatus::Error, format!("exit {code}\n{output}"))
                }
                ValidatorOutcome::Skipped { reason } => {
                    (ToolStatus::Error, format!("validator skipped: {reason}"))
                }
            };
            self.sink.emit(AgentEvent::ToolResult {
                id: vid,
                name: "post_tool_validate".into(),
                status,
                output: agent_tools::ToolOutput { content, display: None },
                duration_ms: started.elapsed().as_millis() as u64,
                parent_id: None,
            });
        }
        if !failures.is_empty() {
            ctx.append(Message::user(format!(
                "Post-edit validation reported problems. Fix these before continuing:\n\n{}",
                failures.join("\n\n")
            )));
        }
    }
```

(Note: `Skipped` outcomes emit an event but do not populate `failures`, so a
fully-skipped pass appends nothing — matching all-pass silence, per spec.)

- [ ] **Step 5: fmt + run the crate suite; repair fallout**

Run: `cd agent && cargo fmt && cargo test -p agent-core`
Expected: the new validator tests pass; all existing loop tests still pass
(empty-default is a no-op so no existing test should shift). If a test that
scripts a Write-tier tool now sees an unexpected validator event, it configured
no validators, so it will not — but double-check `dispatch_tool.rs` if any child
config sets validators (none should).

- [ ] **Step 6: Smoke dependent crates**

Run: `cd agent && cargo test -p agent-runtime-config && cargo test -p agent-server && cargo clippy -p agent-core`
Expected: green (ReadyCall/executed are private; no API break).

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs
git commit -m "feat(core): once-per-turn post-execution validator hook with model feedback"
```

---

### Task 3: CI gate

- [ ] Run: `bash scripts/ci.sh` (repo root). Expected: green. Fix anything red (run `cargo fmt` if `fmt --check` trips).
