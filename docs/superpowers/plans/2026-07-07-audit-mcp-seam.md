# Audit Drain Cluster 2 — MCP Seam Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close audit findings 5.1 (`Access::TrustedWrite` so trusted-MCP mutations trip the post-exec validator), 2.1 (connect-time MCP schema lint surfaced via `ServerStatus.schema_warnings`), and 3.1 (docker env secrets out of argv) per `docs/superpowers/specs/2026-07-07-mcp-seam-design.md`.

**Architecture:** Bottom-up through the tier: add the `TrustedWrite` variant + policy-gate arm first (agent-tools/agent-policy), then make the loop's validator trigger count it (agent-core), then map MCP `Trust::Allow` onto it (agent-mcp). The schema lint and the docker env change are independent of that chain and land after. Every task is TDD in its own crate.

**Tech Stack:** Rust (cargo workspace `agent/`), tokio tests, existing MockTransport/ScriptedModel test doubles.

**Branch:** `feature/audit-mcp-seam` off `main` (create via superpowers:using-git-worktrees at execution start; if EnterWorktree lands on stale `origin/main`, `git reset --hard` to local main's tip first — see the cluster-1 gotcha in the ledger).

## Global Constraints

- `source ~/.cargo/env` first if `cargo` is not on PATH.
- All code lives in the `agent/` workspace — run cargo from `agent/`.
- Conventional commits: `type(scope): summary`.
- Gate behavior for today's MCP intents must be **identical** after 5.1: `Trust::Allow` with empty paths stays auto-allowed (`Decision::Allow`). Only post-exec validation visibility changes.
- Warn-don't-reject for the schema lint: violating tools still register.
- `-e HOME=/tmp` stays a literal in docker argv (non-secret default) when `spec.env` lacks HOME.
- The Destroy floor (never auto-allowed) is untouched.
- Code excerpts below were read from live source at `899ac84`; if a hunk doesn't match, re-read the file before editing.
- The ledger (`.superpowers/sdd/progress.md`) lives untracked in the MAIN checkout — never `git add` it.

---

### Task 1: `Access::TrustedWrite` variant + policy-gate arm

**Files:**
- Modify: `agent/crates/agent-tools/src/types.rs:24-31` (the `Access` enum)
- Modify: `agent/crates/agent-policy/src/engine.rs:59-77` (the access match) + tests in the same file
- Test: `agent/crates/agent-policy/src/engine.rs` (existing `mod tests` has an `intent(access, paths, cmd)` helper)

**Interfaces:**
- Produces: `agent_tools::Access::TrustedWrite` — used by Task 2 (loop trigger) and Task 3 (MCP mapping). Gate semantics: identical to `Access::Read` (workspace-bounded paths; empty paths → `Decision::Allow`).

- [ ] **Step 1: Write the failing tests**

In `agent/crates/agent-policy/src/engine.rs` `mod tests`, next to the existing Read/Write tier tests (which use the module's `intent(...)` helper — match its exact signature when you open the file):

```rust
    #[test]
    fn trusted_write_auto_allows_with_empty_paths() {
        assert!(matches!(
            policy().check(&intent(Access::TrustedWrite, vec![], None)),
            Decision::Allow
        ));
    }

    #[test]
    fn trusted_write_escaping_path_asks() {
        assert!(matches!(
            policy().check(&intent(Access::TrustedWrite, vec!["/etc/passwd"], None)),
            Decision::Ask
        ));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-policy trusted_write 2>&1 | tail -5`
Expected: compile error — `no variant named TrustedWrite` (that IS the red state).

- [ ] **Step 3: Add the variant**

In `agent/crates/agent-tools/src/types.rs`, the enum currently reads:

```rust
pub enum Access {
    Read,
    Write,
    /// Irreversible destruction (e.g. deleting a stored record). Never auto-allowed:
    /// the policy floor for Destroy is Ask — no allowlist or workspace-boundary rule
    /// may return Allow for it. The hard floor can still Deny it.
    Destroy,
}
```

Insert between `Write` and the `Destroy` doc comment:

```rust
    /// Third-party mutation pre-approved by config (MCP `Trust::Allow`): the
    /// approval gate auto-allows it (Read-like, workspace-bounded), but
    /// post-execution validation counts it as a mutation. Never Destroy-tier.
    TrustedWrite,
```

- [ ] **Step 4: Add the gate arm**

In `agent/crates/agent-policy/src/engine.rs`, the access match currently reads:

```rust
        match intent.access {
            Access::Read => {
```

Change that one arm's pattern (body unchanged), with the comment:

```rust
        match intent.access {
            // TrustedWrite (pre-approved third-party mutation, e.g. MCP Trust::Allow)
            // shares Read's gate semantics: auto-allow inside the workspace boundary.
            // Post-exec validation — not this gate — is where its mutations surface.
            Access::Read | Access::TrustedWrite => {
```

The `Access::Write => Decision::Ask` and `Access::Destroy => Decision::Ask` arms stay as they are. The command branch above (`intent.access != Access::Destroy && is_auto_allowed`) is untouched — MCP intents carry no command.

- [ ] **Step 5: Run the tests, then the whole workspace**

Run: `cd agent && cargo test -p agent-policy 2>&1 | tail -3`
Expected: PASS including both new tests.

Run: `cd agent && cargo build --workspace 2>&1 | tail -3`
Expected: clean build. If any other exhaustive `match` on `Access` fails to compile, STOP and report it (the spec expects exactly one — the arm you just edited); a new site is design input, not something to pattern-match silently.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-tools/src/types.rs agent/crates/agent-policy/src/engine.rs
git commit -m "feat(policy): Access::TrustedWrite tier — auto-allowed at the gate, mutation-visible downstream (audit 5.1)"
```

---

### Task 2: `turn_mutated` counts TrustedWrite; validator-trigger test

**Files:**
- Modify: `agent/crates/agent-core/src/loop_.rs:828-834` (the `turn_mutated` computation) and its test module (validator tests start ~line 5593)

**Interfaces:**
- Consumes: `Access::TrustedWrite` from Task 1.
- Produces: the loop behavior Task 3's mapping relies on. Also changes the test helper `validator_loop` to take the scripted tool name: `validator_loop(ws: PathBuf, validators: Vec<String>, registry: Arc<ToolRegistry>, tool_name: &str)`.

- [ ] **Step 1: Parameterize the test helper**

In `loop_.rs`'s validator test module, `validator_loop` currently hardcodes the scripted call:

```rust
        let model = Arc::new(ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "write_stub".into(), "{}".into()),
            Scripted::Text("done".into()),
        ]));
```

Add a `tool_name: &str` parameter and use `tool_name.into()` in the `Scripted::Call`. Update every existing call site to pass the name it currently scripts (they all script `"write_stub"` today — verify against each stub's `name()` as you edit; `FailStub`/self-cancel stubs may reuse `"write_stub"` or have their own registries).

- [ ] **Step 2: Write the failing test**

Add the stub next to `WriteStub` (mirror its shape exactly) and the test next to `read_only_turn_does_not_run_validators`:

```rust
    /// A TrustedWrite-tier stub (the MCP Trust::Allow encoding): auto-allowed
    /// at the gate, but a successful call must count as a mutation and trigger
    /// configured validators (audit 5.1).
    struct TrustedStub;
    #[async_trait::async_trait]
    impl Tool for TrustedStub {
        fn name(&self) -> &str {
            "trusted_stub"
        }
        fn description(&self) -> &str {
            "trusted third-party stub"
        }
        fn schema(&self) -> agent_tools::ToolSchema {
            agent_tools::ToolSchema {
                name: "trusted_stub".into(),
                description: "".into(),
                parameters: serde_json::json!({"type":"object"}),
            }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<agent_tools::ToolIntent, ToolError> {
            Ok(agent_tools::ToolIntent {
                tool: "trusted_stub".into(),
                access: agent_tools::Access::TrustedWrite,
                paths: vec![],
                command: None,
                summary: "trusted mutation".into(),
            })
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolCtx,
        ) -> Result<agent_tools::ToolOutput, ToolError> {
            Ok(agent_tools::ToolOutput {
                content: "did".into(),
                display: None,
            })
        }
    }

    /// TrustedWrite success = mutating turn: configured validators must run
    /// (this is the audit-5.1 regression pin — Trust::Allow MCP mutations were
    /// invisible to the validator while encoded as Access::Read).
    #[tokio::test]
    async fn trusted_write_turn_runs_validators() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let mut r = ToolRegistry::new();
        r.register(Arc::new(TrustedStub));
        let (agent, sink) = validator_loop(ws, vec!["true".into()], Arc::new(r), "trusted_stub");
        let mut ctx = WindowContext::new(Message::system("sys"));
        agent.run(&mut ctx, "go".into()).await.unwrap();

        let ev = validate_events(&sink);
        assert_eq!(
            ev.len(),
            1,
            "TrustedWrite success must trigger validators: {ev:?}"
        );
        assert_eq!(ev[0].1, ToolStatus::Ok);
    }
```

- [ ] **Step 3: Run to verify failure**

Run: `cd agent && cargo test -p agent-core trusted_write_turn_runs_validators 2>&1 | tail -5`
Expected: FAIL — `ev.len()` is 0 (TrustedWrite not yet counted as mutating).

- [ ] **Step 4: Implement**

At `loop_.rs:828-834`, the trigger currently reads:

```rust
            let turn_mutated = executed.iter().any(|(_, _, ex, _, _, access)| {
                matches!(ex, Executed::Ok(_))
                    && matches!(
                        access,
                        agent_tools::Access::Write | agent_tools::Access::Destroy
                    )
            });
```

Extend the access pattern:

```rust
                    && matches!(
                        access,
                        agent_tools::Access::Write
                            | agent_tools::Access::Destroy
                            | agent_tools::Access::TrustedWrite
                    )
```

Keep the comment above it accurate — extend "at least one Write/Destroy call" to "at least one Write/Destroy/TrustedWrite call".

- [ ] **Step 5: Run the validator test module, then the crate**

Run: `cd agent && cargo test -p agent-core validator 2>&1 | tail -3` then `cargo test -p agent-core 2>&1 | tail -3`
Expected: PASS — new test green, `read_only_turn_does_not_run_validators` and every existing validator test still green.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-core/src/loop_.rs
git commit -m "feat(core): post-exec validator trigger counts Access::TrustedWrite turns (audit 5.1)"
```

---

### Task 3: MCP `Trust::Allow` maps to TrustedWrite

**Files:**
- Modify: `agent/crates/agent-mcp/src/tool.rs:69-86` (the `intent()` mapping + comment) and its tests

**Interfaces:**
- Consumes: `Access::TrustedWrite` (Task 1) with the loop behavior from Task 2.
- Produces: MCP `Trust::Allow` intents are `TrustedWrite`; `Trust::Ask → Write` unchanged.

- [ ] **Step 1: Update the test to the new contract (failing first)**

In `tool.rs` tests, replace `allow_trust_maps_to_policy_allow` with:

```rust
    #[tokio::test]
    async fn allow_trust_maps_to_trusted_write_and_policy_allow() {
        let tool = McpTool::new("fs", client_that(|_| vec![]), raw(), Trust::Allow);
        let intent = tool.intent(&json!({})).unwrap();
        assert_eq!(intent.access, Access::TrustedWrite);
        assert!(intent.paths.is_empty());
        assert!(matches!(policy().check(&intent), Decision::Allow));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-mcp allow_trust 2>&1 | tail -5`
Expected: FAIL — access is `Read` today.

- [ ] **Step 3: Implement the mapping**

At `tool.rs:69-75`, the mapping and its stale comment currently read:

```rust
        // Trust is encoded onto the policy's Read/Write axis (zero policy change):
        // Ask → Write (RulePolicy asks); Allow → Read with empty paths (vacuously true → Allow).
        let access = match self.trust {
            Trust::Allow => Access::Read,
            Trust::Ask => Access::Write,
        };
```

Replace with:

```rust
        // Trust maps onto the policy axis: Ask → Write (RulePolicy asks);
        // Allow → TrustedWrite (auto-allowed like Read at the gate, but counted
        // as a mutation by the post-exec validator trigger — audit 5.1).
        let access = match self.trust {
            Trust::Allow => Access::TrustedWrite,
            Trust::Ask => Access::Write,
        };
```

- [ ] **Step 4: Run the crate suite**

Run: `cd agent && cargo test -p agent-mcp 2>&1 | tail -3`
Expected: PASS — the renamed test green, `ask_trust_maps_to_policy_ask` untouched and green.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-mcp/src/tool.rs
git commit -m "feat(mcp): Trust::Allow intents are Access::TrustedWrite — validator-visible mutations (audit 5.1)"
```

---

### Task 4: Connect-time schema lint + `ServerStatus.schema_warnings`

**Files:**
- Modify: `agent/crates/agent-mcp/src/manager.rs` (ServerStatus, connect Ok arm, `summary_line`, `from_parts` callers, tests)

**Interfaces:**
- Consumes: `agent_tools::required_params_missing_description(&ToolSchema)` (re-exported at the agent-tools crate root via `pub use contract::*` — verified live).
- Produces: `ServerStatus` gains `pub schema_warnings: usize`; private `fn schema_lint(tools: &[Arc<dyn Tool>]) -> Vec<String>` in manager.rs; summary format `name ✓ (N tools, W schema warnings)` when W > 0.

**Test-strategy note (sanctioned adjustment to the spec's pin wording):** `McpManager::connect` hardwires `StdioTransport::spawn`, so a "connect over MockTransport" test is not reachable without a transport-injection refactor that the spec does not ask for. The same pins are covered as: (a) `schema_lint` unit-tested directly over `McpTool`s built on `MockTransport` (bad → 2 warnings, clean → 0, tool still present), (b) the status field + summary format via `from_parts`, (c) the two wiring lines in the connect Ok arm carried by review.

- [ ] **Step 1: Write the failing tests**

In `manager.rs` `mod tests` (which already imports `McpServersConfig`; add the imports the code below needs — `crate::client::{McpClient, RawTool}`, `crate::config::Trust`, `crate::tool::McpTool`, `crate::transport::MockTransport`):

```rust
    fn mock_client() -> Arc<McpClient> {
        McpClient::new(Arc::new(MockTransport::scripted(|_| vec![])))
    }

    #[test]
    fn schema_lint_flags_empty_description_and_undescribed_required_param() {
        let bad = RawTool {
            name: "create".into(),
            description: "   ".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"x": {"type": "string"}},
                "required": ["x"]
            }),
        };
        let tools: Vec<Arc<dyn Tool>> =
            vec![Arc::new(McpTool::new("srv", mock_client(), bad, Trust::Ask))];
        let w = schema_lint(&tools);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w.iter().any(|m| m.contains("empty description")), "{w:?}");
        assert!(w.iter().any(|m| m.contains("`x`")), "{w:?}");
    }

    #[test]
    fn schema_lint_clean_schema_yields_no_warnings() {
        let clean = RawTool {
            name: "create".into(),
            description: "Create an issue".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"x": {"type": "string", "description": "the thing"}},
                "required": ["x"]
            }),
        };
        let tools: Vec<Arc<dyn Tool>> =
            vec![Arc::new(McpTool::new("srv", mock_client(), clean, Trust::Ask))];
        assert!(schema_lint(&tools).is_empty());
    }

    #[test]
    fn summary_line_shows_schema_warnings_when_nonzero() {
        let mgr = McpManager::from_parts(
            vec![],
            vec![ServerStatus {
                name: "github".into(),
                connected: true,
                tool_count: 3,
                schema_warnings: 2,
                error: None,
            }],
        );
        assert_eq!(mgr.summary_line(), "mcp: github \u{2713} (3 tools, 2 schema warnings)");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-mcp schema_lint 2>&1 | tail -5`
Expected: compile error — `schema_lint` and the `schema_warnings` field don't exist.

- [ ] **Step 3: Implement**

(a) `ServerStatus` gains the field (after `tool_count`):

```rust
    pub tool_count: usize,
    /// Contract-lint violations across this server's tools (empty descriptions,
    /// undescribed required params). Warn-only — the tools still register.
    pub schema_warnings: usize,
```

(b) Private helper above `connect_one`:

```rust
/// One warning string per contract-lint violation across a server's wrapped
/// tools: empty description, or a required param with no description
/// (`agent_tools::required_params_missing_description`). Warn-don't-reject.
fn schema_lint(tools: &[Arc<dyn Tool>]) -> Vec<String> {
    let mut warnings = Vec::new();
    for t in tools {
        let s = t.schema();
        if s.description.trim().is_empty() {
            warnings.push(format!("{}: empty description", s.name));
        }
        for p in agent_tools::required_params_missing_description(&s) {
            warnings.push(format!("{}: required param `{p}` has no description", s.name));
        }
    }
    warnings
}
```

(c) The connect Ok arm currently reads:

```rust
                Ok((name, client, server_tools)) => {
                    statuses.push(ServerStatus {
                        name,
                        connected: true,
                        tool_count: server_tools.len(),
                        error: None,
                    });
```

Becomes:

```rust
                Ok((name, client, server_tools)) => {
                    let warnings = schema_lint(&server_tools);
                    for w in &warnings {
                        tracing::warn!(target: "mcp", server = %name, violation = %w,
                            "MCP tool schema fails contract lint (tool still registered)");
                    }
                    statuses.push(ServerStatus {
                        name,
                        connected: true,
                        tool_count: server_tools.len(),
                        schema_warnings: warnings.len(),
                        error: None,
                    });
```

(d) The Err arm's `ServerStatus` gains `schema_warnings: 0`.

(e) `summary_line`'s connected arm currently formats `format!("{} \u{2713} ({} tools)", s.name, s.tool_count)`. Becomes:

```rust
                if s.connected {
                    if s.schema_warnings > 0 {
                        format!(
                            "{} \u{2713} ({} tools, {} schema warnings)",
                            s.name, s.tool_count, s.schema_warnings
                        )
                    } else {
                        format!("{} \u{2713} ({} tools)", s.name, s.tool_count)
                    }
                } else {
```

(f) Fix compile fallout in this file only: the two `ServerStatus` literals in `summary_line_formats_mixed_statuses` gain `schema_warnings: 0`. (`from_parts` itself takes the vec — no change.)

- [ ] **Step 4: Run the crate suite**

Run: `cd agent && cargo test -p agent-mcp 2>&1 | tail -3` and `cd agent && cargo build --workspace 2>&1 | tail -3`
Expected: PASS; workspace build clean (compiler surfaces any other `ServerStatus` literal — fix each by adding `schema_warnings: 0`, and list them in your report).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-mcp/src/manager.rs
git commit -m "feat(mcp): contract-lint tool schemas at connect; surface count on ServerStatus (audit 2.1)"
```

(Include any other files the workspace build forced you to touch, and name them in the commit body.)

---

### Task 5: Docker env values leave argv

**Files:**
- Modify: `agent/crates/agent-sandbox/src/docker.rs:73-83` (env loop) + tests in the same file
- Modify: `agent/crates/agent-sandbox/src/strategy.rs:179-186` (`spawn_docker`)

**Interfaces:**
- Consumes: `CommandSpec.env: BTreeMap<String, String>` (sorted iteration — determinism preserved).
- Produces: argv carries name-only `-e KEY` for spec env; values travel via `cmd.envs(&spec.env)` on the docker client process.

- [ ] **Step 1: Write the failing test + update the HOME test**

In `docker.rs` `mod tests`:

```rust
    #[test]
    fn env_values_stay_out_of_argv() {
        let mut spec = oneshot();
        spec.env.insert("API_KEY".into(), "sekret-value".into());
        let v = docker_run_args(&policy(false), &spec, "n", "1000:1000");
        let s = v.join(" ");
        assert!(s.contains("-e API_KEY"), "name-only -e for spec env: {s}");
        assert!(!s.contains("sekret-value"), "value must never reach argv: {s}");
        assert!(!s.contains("API_KEY="), "no KEY=VALUE form in argv: {s}");
    }
```

And `home_defaults_to_tmp_unless_spec_sets_it`'s second half currently asserts the interpolated form:

```rust
        let mut spec = oneshot();
        spec.env.insert("HOME".into(), "/workspace".into());
        let s = docker_run_args(&policy(false), &spec, "n", "1000:1000").join(" ");
        assert!(s.contains("-e HOME=/workspace") && !s.contains("-e HOME=/tmp"));
```

Replace those last two lines with:

```rust
        let s = docker_run_args(&policy(false), &spec, "n", "1000:1000").join(" ");
        assert!(
            s.contains("-e HOME") && !s.contains("HOME=/workspace") && !s.contains("-e HOME=/tmp"),
            "spec-set HOME is name-only; value travels via client env: {s}"
        );
```

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-sandbox env_values_stay_out_of_argv home_defaults 2>&1 | tail -6`
Expected: both FAIL against the current `-e KEY=VALUE` emission.

- [ ] **Step 3: Implement**

(a) `docker_run_args:73-77` currently reads:

```rust
    // Env (-e KEY=VAL), sorted for determinism.
    for (k, v) in &spec.env {
        a.push("-e".into());
        a.push(format!("{k}={v}"));
    }
```

Becomes:

```rust
    // Env: name-only `-e KEY`, sorted for determinism (BTreeMap). Values travel
    // on the docker CLIENT process env (spawn_docker sets cmd.envs) so secrets
    // never appear in world-readable argv or `docker inspect` (audit 3.1).
    for k in spec.env.keys() {
        a.push("-e".into());
        a.push(k.clone());
    }
```

The `HOME=/tmp` default block below it stays exactly as-is.

(b) `spawn_docker` in `strategy.rs` currently starts:

```rust
        let args = docker_run_args(&self.policy, spec, name, &self.uid_gid);
        let mut cmd = tokio::process::Command::new("docker");
        cmd.args(&args)
            .kill_on_drop(true)
```

Becomes:

```rust
        let args = docker_run_args(&self.policy, spec, name, &self.uid_gid);
        let mut cmd = tokio::process::Command::new("docker");
        // Values for the name-only `-e KEY` args in docker_run_args: docker
        // forwards them from the client process env into the container.
        cmd.envs(&spec.env);
        cmd.args(&args)
            .kill_on_drop(true)
```

- [ ] **Step 4: Run the crate suite**

Run: `cd agent && cargo test -p agent-sandbox 2>&1 | tail -3`
Expected: PASS — both changed tests plus all existing docker/strategy tests green.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-sandbox/src/docker.rs agent/crates/agent-sandbox/src/strategy.rs
git commit -m "fix(sandbox): docker env secrets leave argv — name-only -e + client-process env (audit 3.1)"
```

---

### Task 6: Full gate, ledger, finish, re-stamps (controller)

**Files:**
- Modify: `.superpowers/sdd/progress.md` (main checkout, untracked), `.agents/skills/harness-engineering/audit.md` (post-merge)

- [ ] **Step 1:** Run `bash scripts/ci.sh` — expect all six legs green (`CI gate passed.`).
- [ ] **Step 2:** Ledger the completed tasks in the cluster-2 section (main checkout).
- [ ] **Step 3:** Final whole-branch review (superpowers:requesting-code-review template, most capable model), then superpowers:finishing-a-development-branch (`--no-ff` merge, worktree removal after verifying the branch tip is an ancestor of main, branch deletion).
- [ ] **Step 4:** Post-merge: append the dated re-stamp note for findings 5.1/2.1/3.1 to `.agents/skills/harness-engineering/audit.md` (match the cluster-1 note's shape at its tail: date, cluster 2/6, merge hash, per-finding one-liners), commit as `docs(skills): re-stamp audit findings 5.1/2.1/3.1 (cluster 2 merged)`. Update the audit memory file's drain-progress line.

---

## Verification summary

| Finding | Proof it's closed |
|---|---|
| 5.1 | `trusted_write_turn_runs_validators` (loop), `trusted_write_auto_allows_with_empty_paths` + escaping-path Ask (engine), `allow_trust_maps_to_trusted_write_and_policy_allow` (mcp) — gate outcome unchanged, validator now fires |
| 2.1 | `schema_lint_*` tests (2 warnings bad / 0 clean, tools still registered), `summary_line_shows_schema_warnings_when_nonzero` |
| 3.1 | `env_values_stay_out_of_argv`, updated `home_defaults_to_tmp_unless_spec_sets_it`; `cmd.envs` wiring line review-carried |
