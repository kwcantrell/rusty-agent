# Claude CLI Follow-ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the three residuals from the claude-cli optimization branch: a checkout-keyed session pool in `ClaudeCliClient` (concurrent subagents at any depth get correct, working session reuse), `rt.validate()` on the agent-cli startup path, and the assemble.rs fresh-client dedup helper.

**Architecture:** `ClaudeCliClient`'s single `Mutex<Option<SessionState>>` becomes a bounded pool `Mutex<Vec<SessionState>>` with take-on-plan / re-insert-on-success (checkout) semantics — the contamination corner becomes impossible by construction and the failure-path "reset" happens at checkout time, ahead of any yield. agent-cli hard-exits (code 2) on `validate()` failure after clap assembly. The duplicated fresh-client block in assemble.rs folds into one helper.

**Tech Stack:** Rust (tokio, async-stream, std Mutex), existing fake-CLI proc-test harness in `claude_cli.rs`.

**Spec:** `docs/superpowers/specs/2026-07-07-claude-cli-followups-design.md`

## Global Constraints

- Rust work happens in the `agent/` workspace: `cd /home/kalen/rust-agent-runtime/agent`. Never touch `src-tauri/`.
- Conventional commits: `type(scope): summary`.
- No `ModelClient` trait changes. No agent-core changes (Approach B was declined).
- `MAX_POOLED_SESSIONS = 8`; eviction is oldest-first by insertion order (re-insert moves an entry to the back).
- The four existing state-machine proc tests (`session_reuse_walks_ephemeral_persisted_resume`, `history_rewrite_resets_to_ephemeral`, `stream_error_resets_session_state`, `reuse_off_is_always_ephemeral_full_send`) must pass **unchanged** — single-caller behavior is byte-compatible.
- With `session_reuse: false` there is **no pool interaction at all** (today's stateless behavior exactly).
- The assemble.rs distinct-instance arms from `5c9bf24` stay (belt-and-suspenders); only their duplication and stale comments change.

---

### Task 1: Session pool with checkout semantics

**Files:**
- Modify: `agent/crates/agent-model/src/claude_cli.rs` (struct field ~line 82, `with_options` ~line 99, `plan_spawn` ~line 143, `stream()` state handling ~lines 242-311, new tests in `mod tests`)

**Interfaces:**
- Consumes: existing `SessionState`, `SpawnPlan`, `fingerprint`, `is_strict_extension`, `write_recording_fake`/`drain`/`read` test helpers (all already in the file).
- Produces: `sessions: Arc<Mutex<Vec<SessionState>>>` field (replaces `state`), `pub(crate) const MAX_POOLED_SESSIONS: usize = 8` (crate-visible so tests reference it), same `plan_spawn` signature. No public API change.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` (the async ones go in `mod proc_tests` next to the existing recording-fake tests; the two sync ones can sit beside them — they need no fake):

```rust
#[tokio::test]
#[serial]
async fn interleaved_transcript_families_each_reach_resume() {
    // The sibling-subagent pattern: two independent transcript families
    // through ONE client. The old single state slot made them clobber each
    // other (every call re-planned fresh); the pool must give each family
    // its own resume track.
    let dir = tempfile::tempdir().unwrap();
    let fake = write_recording_fake(dir.path(), 0);
    let client = ClaudeCliClient::with_options(
        fake.to_str().unwrap(),
        "sonnet",
        ClaudeCliOptions {
            session_reuse: true,
            ..Default::default()
        },
    );

    let a1 = vec![Message::system("sys"), Message::user("task-a")];
    let b1 = vec![Message::system("sys"), Message::user("task-b")];
    drain(&client, a1.clone()).await.unwrap(); // call 1: A fresh ephemeral
    drain(&client, b1.clone()).await.unwrap(); // call 2: B fresh ephemeral

    let mut a2 = a1.clone();
    a2.push(Message::assistant("ok1", None));
    a2.push(Message::user("a-next"));
    let mut b2 = b1.clone();
    b2.push(Message::assistant("ok2", None));
    b2.push(Message::user("b-next"));
    drain(&client, a2.clone()).await.unwrap(); // call 3: A first extension -> persisted (sess-3)
    drain(&client, b2.clone()).await.unwrap(); // call 4: B first extension -> persisted (sess-4)

    let mut a3 = a2.clone();
    a3.push(Message::assistant("ok3", None));
    a3.push(Message::user("a-more"));
    let mut b3 = b2.clone();
    b3.push(Message::assistant("ok4", None));
    b3.push(Message::user("b-more"));
    drain(&client, a3).await.unwrap(); // call 5: A resumes sess-3
    drain(&client, b3).await.unwrap(); // call 6: B resumes sess-4

    let argv5 = read(dir.path(), "argv.5");
    assert!(argv5.contains("--resume sess-3"), "argv5: {argv5}");
    let stdin5 = read(dir.path(), "stdin.5");
    assert!(stdin5.contains("a-more"), "stdin5: {stdin5}");
    assert!(!stdin5.contains("task-a"), "stdin5 resent prefix: {stdin5}");

    let argv6 = read(dir.path(), "argv.6");
    assert!(argv6.contains("--resume sess-4"), "argv6: {argv6}");
    let stdin6 = read(dir.path(), "stdin.6");
    assert!(stdin6.contains("b-more"), "stdin6: {stdin6}");
    assert!(!stdin6.contains("task-b"), "stdin6 resent prefix: {stdin6}");
}

#[tokio::test]
#[serial]
async fn session_pool_is_bounded_and_evicts_oldest() {
    let dir = tempfile::tempdir().unwrap();
    let fake = write_recording_fake(dir.path(), 0);
    let client = ClaudeCliClient::with_options(
        fake.to_str().unwrap(),
        "sonnet",
        ClaudeCliOptions {
            session_reuse: true,
            ..Default::default()
        },
    );
    // MAX + 2 unrelated one-shot transcripts each commit one pool entry.
    let first = vec![Message::system("sys"), Message::user("task-0")];
    let first_fps: Vec<u64> = first.iter().map(fingerprint).collect();
    for i in 0..(MAX_POOLED_SESSIONS + 2) {
        let msgs = vec![Message::system("sys"), Message::user(format!("task-{i}"))];
        drain(&client, msgs).await.unwrap();
    }
    let pool = client.sessions.lock().unwrap();
    assert_eq!(pool.len(), MAX_POOLED_SESSIONS);
    // The oldest entry (task-0) was evicted.
    assert!(
        !pool.iter().any(|s| s.fingerprints == first_fps),
        "oldest entry should have been evicted"
    );
}

#[test]
fn checkout_prevents_concurrent_resume_of_same_session() {
    // Two same-prefix planners racing: the first checks the entry out, the
    // second must find nothing and degrade to a fresh send — never a second
    // Resume against the same session id.
    let client = ClaudeCliClient::with_options(
        "claude",
        "sonnet",
        ClaudeCliOptions {
            session_reuse: true,
            ..Default::default()
        },
    );
    let base = vec![Message::system("sys"), Message::user("u1")];
    let base_fps: Vec<u64> = base.iter().map(fingerprint).collect();
    client.sessions.lock().unwrap().push(SessionState {
        session_id: Some("sess-x".into()),
        persisted: true,
        fingerprints: base_fps,
    });
    let mut ext = base.clone();
    ext.push(Message::assistant("ok", None));
    ext.push(Message::user("u2"));

    let (plan1, _) = client.plan_spawn(&ext);
    assert!(matches!(plan1, SpawnPlan::Resume { .. }));
    let (plan2, _) = client.plan_spawn(&ext);
    assert!(matches!(plan2, SpawnPlan::FreshEphemeral));
}

#[test]
fn checkout_picks_longest_matching_prefix() {
    let client = ClaudeCliClient::with_options(
        "claude",
        "sonnet",
        ClaudeCliOptions {
            session_reuse: true,
            ..Default::default()
        },
    );
    let base = vec![Message::system("sys"), Message::user("u1")];
    let mut longer = base.clone();
    longer.push(Message::assistant("ok", None));
    let base_fps: Vec<u64> = base.iter().map(fingerprint).collect();
    let longer_fps: Vec<u64> = longer.iter().map(fingerprint).collect();
    {
        let mut pool = client.sessions.lock().unwrap();
        pool.push(SessionState {
            session_id: Some("sess-short".into()),
            persisted: true,
            fingerprints: base_fps,
        });
        pool.push(SessionState {
            session_id: Some("sess-long".into()),
            persisted: true,
            fingerprints: longer_fps,
        });
    }
    let mut ext = longer.clone();
    ext.push(Message::user("u2"));
    let (plan, _) = client.plan_spawn(&ext);
    match plan {
        SpawnPlan::Resume { session_id, suffix_start } => {
            assert_eq!(session_id, "sess-long");
            assert_eq!(suffix_start, 3);
        }
        _ => panic!("expected Resume from the longest matching entry"),
    }
    // The shorter entry is still in the pool (only the match is checked out).
    assert_eq!(client.sessions.lock().unwrap().len(), 1);
}
```

Notes: `Message::user(format!("task-{i}"))` — if the constructor takes `&str` only, pass `&format!("task-{i}")`. The sync tests construct `SessionState` directly; the tests module is a child of this file's module, so private fields and `plan_spawn` are reachable.

- [ ] **Step 2: Run to verify failure**

```bash
cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-model claude_cli
```

Expected: FAIL to compile — `client.sessions` and `MAX_POOLED_SESSIONS` do not exist yet.

- [ ] **Step 3: Implement the pool**

3a. Add the constant near `SessionState` (~line 34) and swap the field:

```rust
/// Upper bound on pooled session states. One-shot callers (compaction, evals
/// through a reuse-enabled client) commit entries that never match again; the
/// cap bounds that growth. Eviction is oldest-first by insertion order.
pub(crate) const MAX_POOLED_SESSIONS: usize = 8;
```

In `ClaudeCliClient` (~line 82) replace

```rust
    state: Arc<Mutex<Option<SessionState>>>,
```

with

```rust
    /// Pool of resumable session states, one per transcript family this client
    /// has served (parent loop, sibling subagents, compaction one-shots). An
    /// entry is CHECKED OUT (removed) while a call built on it is in flight and
    /// re-inserted only on success — see `plan_spawn`.
    sessions: Arc<Mutex<Vec<SessionState>>>,
```

and in `with_options` (~line 99) replace `state: Arc::new(Mutex::new(None)),` with `sessions: Arc::new(Mutex::new(Vec::new())),`.

3b. Replace the body of `plan_spawn` (~line 143):

```rust
    /// Decide how to spawn for this transcript and produce the pending state to
    /// commit on success.
    ///
    /// Checkout semantics: the longest strict-prefix entry is REMOVED from the
    /// pool while this call is in flight. A concurrent caller with the same
    /// prefix then matches nothing and degrades to a fresh send — two callers
    /// can never resume the same session simultaneously. Success re-inserts the
    /// updated state (in `stream()`); failure simply never re-inserts, so the
    /// loop's retry lands on a fresh full send.
    fn plan_spawn(&self, messages: &[Message]) -> (SpawnPlan, SessionState) {
        let fps: Vec<u64> = messages.iter().map(fingerprint).collect();
        let fresh = |persisted: bool| SessionState {
            session_id: None,
            persisted,
            fingerprints: fps.clone(),
        };
        if !self.opts.session_reuse {
            return (SpawnPlan::FreshEphemeral, fresh(false));
        }
        let checked_out = {
            let mut pool = self.sessions.lock().expect("session pool lock");
            let best = pool
                .iter()
                .enumerate()
                .filter(|(_, s)| is_strict_extension(&s.fingerprints, &fps))
                .max_by_key(|(_, s)| s.fingerprints.len())
                .map(|(i, _)| i);
            best.map(|i| pool.remove(i))
        };
        match checked_out {
            Some(s) => {
                if !s.persisted {
                    // First extension: pay one full send to make the session resumable.
                    return (SpawnPlan::FreshPersisted, fresh(true));
                }
                let suffix_start = s.fingerprints.len();
                let suffix_has_content = messages[suffix_start..]
                    .iter()
                    .any(|m| m.role != Role::Assistant);
                match (s.session_id, suffix_has_content) {
                    (Some(id), true) => (
                        SpawnPlan::Resume {
                            session_id: id.clone(),
                            suffix_start,
                        },
                        SessionState {
                            session_id: Some(id),
                            persisted: true,
                            fingerprints: fps,
                        },
                    ),
                    // No id captured or assistant-only suffix: degrade safely.
                    _ => (SpawnPlan::FreshEphemeral, fresh(false)),
                }
            }
            // First call of a family, or history was rewritten: start over.
            None => (SpawnPlan::FreshEphemeral, fresh(false)),
        }
    }
```

3c. In `stream()`: replace `let state = Arc::clone(&self.state);` (~line 242) with `let sessions = Arc::clone(&self.sessions);`. **Delete** all three failure-path reset blocks (the `if track_state { *state.lock().expect("session state lock") = None; }` at the parse-error, io-error, non-zero-exit, and wait-error arms) — the matched entry was already checked out in `plan_spawn`, so on failure there is nothing to reset; leave a single comment at the first failure arm:

```rust
                        // No state reset needed on failure: plan_spawn already
                        // checked the matched pool entry out, and we only
                        // re-insert on success.
```

Replace the success-path commit (~lines 284-291) with:

```rust
                        if track_state {
                            // Prefer the id the CLI just reported (a resume may
                            // continue under the same id or, with future CLIs, a
                            // forked one — the init event is authoritative).
                            pending.session_id =
                                parser.session_id.take().or(pending.session_id.take());
                            let mut pool = sessions.lock().expect("session pool lock");
                            pool.push(pending);
                            if pool.len() > MAX_POOLED_SESSIONS {
                                pool.remove(0); // evict oldest
                            }
                        }
```

`track_state` stays: with reuse off, nothing is ever inserted (Global Constraints).

- [ ] **Step 4: Run the tests**

```bash
cargo test -p agent-model claude_cli
```

Expected: PASS — the 4 new tests plus all existing ones, **including the four untouched state-machine proc tests** (their observable argv/stdin behavior is unchanged: a single sequential caller's entry is always either the sole match or absent).

- [ ] **Step 5: Full crate + clippy + fmt**

```bash
cargo test -p agent-model && cargo clippy -p agent-model -- -D warnings && cargo fmt --check
```

Expected: PASS, no warnings. (If fmt flags the file, `cargo fmt -p agent-model`.)

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-model/src/claude_cli.rs
git commit -m "feat(model): checkout-keyed session pool for concurrent claude-cli callers"
```

---

### Task 2: `rt.validate()` on the agent-cli startup path

**Files:**
- Modify: `agent/crates/agent-cli/src/main.rs` (main() ~line 223, new tests beside `claude_cli_knob_flags_absent_leave_defaults` ~line 388)

**Interfaces:**
- Consumes: `RuntimeConfig::validate()` (existing, agent-runtime-config), `runtime_config_from_cli` (existing, this file).
- Produces: nothing new — startup behavior only (exit 2 on invalid config).

- [ ] **Step 1: Write the tests**

Add to the existing `mod tests` in `main.rs`:

```rust
    #[test]
    fn cli_assembled_config_passes_validate() {
        // Guards the startup gate: default flags must never trip validate(),
        // or every plain `agent-cli` run would exit 2.
        let cli = Cli::parse_from(["agent-cli"]);
        let rc = runtime_config_from_cli(&cli, "prompted");
        assert!(rc.validate().is_ok(), "default CLI config must validate");
    }

    #[test]
    fn cli_bad_claude_effort_fails_validate() {
        let cli = Cli::parse_from(["agent-cli", "--claude-effort", "banana"]);
        let rc = runtime_config_from_cli(&cli, "prompted");
        let err = rc.validate().unwrap_err();
        assert!(err.contains("claude_effort"), "got: {err}");
    }
```

- [ ] **Step 2: Run the tests**

```bash
cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-cli
```

Expected: PASS already — `validate()` exists; these tests pin the contract the startup gate relies on. (The RED phase for this task is the missing wiring, verified in Step 4.)

- [ ] **Step 3: Wire the gate into main()**

In `main()`, immediately after `let rt = runtime_config_from_cli(&cli, protocol_name);` (~line 223) and before `build_model`:

```rust
    if let Err(e) = rt.validate() {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
```

- [ ] **Step 4: Verify the gate end-to-end (no API calls — exits before any connection)**

```bash
cargo run -p agent-cli -- --claude-effort banana --workspace . < /dev/null; echo "exit=$?"
```

Expected: stderr contains `error: claude_effort 'banana' not recognized: use one of low | medium | high | xhigh | max` and `exit=2`. Then confirm the happy path still starts and exits cleanly on empty stdin:

```bash
cargo run -p agent-cli -- --workspace . < /dev/null; echo "exit=$?"
```

Expected: prints the usual startup lines (`agent ready...`), `exit=0`.

- [ ] **Step 5: Test + fmt, commit**

```bash
cargo test -p agent-cli && cargo fmt --check
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-cli/src/main.rs
git commit -m "feat(cli): validate runtime config at startup (exit 2 on bad knobs)"
```

---

### Task 3: assemble.rs fresh-client dedup helper + comment refresh

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (compaction arm ~lines 205-228, child arm ~lines 245-264, new helper near the top-level fns)

**Interfaces:**
- Consumes: `crate::build_model`, `crate::claude_cli_opts`, `RuntimeConfig` (all existing).
- Produces: `fn fresh_claude_cli_client(cfg: &RuntimeConfig, claude_binary: &str, api_key: Option<String>) -> Arc<dyn ModelClient>` (private to assemble.rs).

- [ ] **Step 1: Add the helper**

Above `assemble_loop` (match the file's existing free-fn placement):

```rust
/// Fresh claude-cli client with the parent's exact construction parameters.
/// Distinct instances keep each loop's session pool private (belt-and-
/// suspenders: the pool in ClaudeCliClient makes Arc-sharing safe, but
/// separate instances also keep the parent's pool unpolluted by child and
/// compaction entries). See docs/superpowers/specs/2026-07-07-claude-cli-followups-design.md.
fn fresh_claude_cli_client(
    cfg: &RuntimeConfig,
    claude_binary: &str,
    api_key: Option<String>,
) -> Arc<dyn ModelClient> {
    crate::build_model(
        &cfg.backend,
        &cfg.base_url,
        &cfg.model,
        claude_binary,
        api_key,
        crate::claude_cli_opts(cfg),
    )
}
```

(If `RuntimeConfig`/`ModelClient` aren't already in scope at that spot, use the same paths the surrounding code uses — the file already names both.)

- [ ] **Step 2: Use it in both arms and refresh the stale comments**

Compaction arm (~lines 216-224): replace the inline `Some(crate::build_model(...))` block with

```rust
            if cfg.backend == "claude-cli" {
                Some(fresh_claude_cli_client(
                    cfg,
                    &parts.claude_binary,
                    parts.api_key.clone(),
                ))
            } else {
                None
            }
```

Child arm (~lines 255-262): replace the inline `crate::build_model(...)` with

```rust
            None if cfg.backend == "claude-cli" => fresh_claude_cli_client(
                cfg,
                &parts.claude_binary,
                parts.api_key.clone(),
            ),
```

Update the two comment blocks (compaction ~lines 205-210, child ~lines 249-254): they currently claim sharing "would let the compaction call overwrite the parent's session fingerprints" — since Task 1, the session pool makes sharing safe. Reword both to the belt-and-suspenders rationale, e.g. for the child arm:

```rust
            // For the openai backend, cloning the Arc is harmless: the client is
            // stateless. For claude-cli, a distinct instance keeps the parent's
            // session pool private (the pool itself makes sharing safe since the
            // checkout-keyed rework — this is belt-and-suspenders isolation).
```

and the compaction block header comment analogously.

- [ ] **Step 3: Run the assemble tests**

```bash
cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-runtime-config
```

Expected: PASS — the existing distinct-instance tests (`Arc::ptr_eq` pin and compaction `is_some()`) prove the refactor is behavior-neutral.

- [ ] **Step 4: Clippy + fmt, commit**

```bash
cargo clippy -p agent-runtime-config -- -D warnings && cargo fmt --check
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "refactor(config): dedup fresh claude-cli client construction in assemble"
```

---

### Task 4: Full gate

**Files:** none new.

**Interfaces:**
- Consumes: everything above.
- Produces: green `scripts/ci.sh`.

- [ ] **Step 1: Run the CI gate**

```bash
cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh
```

Expected: all legs pass (okf, fmt, clippy, cargo test, web typecheck, vitest; src-tauri leg skips without GTK deps). Known gotcha: `ld terminated with signal 7` = check `df -h` (disk), not the code.

- [ ] **Step 2: Commit any gate fixes**

```bash
git add -A && git commit -m "chore: ci gate fixes for claude-cli follow-ups" || echo "nothing to fix"
```

---

## Self-review notes (done at authoring time)

- Spec coverage: §1 pool/checkout → Task 1 (plan/commit/failure semantics, longest-match, cap-8 LRU, reuse-off no-op, four-existing-tests-unchanged constraint); §2 CLI validate hard-exit → Task 2 (including the guard test that defaults never trip the gate); §3 dedup helper → Task 3 (plus the stale-comment refresh the pool makes necessary); §4 tests/gate → Tasks 1-2 test code + Task 4. Error-handling table: checked-out-entry-stays-dropped → Task 1 Step 3c comment + existing `stream_error_resets_session_state`; concurrent-same-prefix → `checkout_prevents_concurrent_resume_of_same_session`; overflow → `session_pool_is_bounded_and_evicts_oldest`; CLI exit 2 → Task 2 Step 4.
- Type consistency: `sessions: Arc<Mutex<Vec<SessionState>>>` matches between Task 1 field, plan_spawn, and commit block; `MAX_POOLED_SESSIONS` is `pub(crate)` and referenced only from same-crate tests; `fresh_claude_cli_client` signature matches both call sites (`&parts.claude_binary`, `parts.api_key.clone()`).
- Walkthrough of the interleave test against the fake's `sess-<n>` numbering: A persists on call 3 (sess-3), B on call 4 (sess-4); calls 5/6 assert those ids — consistent with `write_recording_fake`.
- Graphify `--update` is deliberately absent (ops action post-merge, per spec Out of scope).
