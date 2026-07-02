# Eval Flywheel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Trajectory capture + gold comparator in the eval harness, a data-driven adversarial policy corpus, a coverage CI job, and denial visibility.

**Architecture:** Additive serde fields on `RunResult`/`TaskSpec` + a pure comparator (eval/result.rs, eval/task.rs); sink-side capture in tests/eval_context.rs; a TSV-driven integration test in agent-runtime-config/tests; one extra GitHub Actions job. Gate logic untouched.

**Tech Stack:** Rust (workspace `agent/`), GitHub Actions YAML.

**Spec:** `docs/superpowers/specs/2026-07-02-eval-flywheel-design.md` — behavior authority; read fully before any task.

## Global Constraints

- Run cargo from `agent/` (`source ~/.cargo/env` if missing); `cargo fmt` the touched crates before every commit; conventional commits; `bash scripts/ci.sh` green at cluster end (run it in the BACKGROUND if a fresh rebuild is needed — it can exceed 10 minutes cold).
- All new serde fields `#[serde(default)]` — old recorded JSON lines and frozen task files MUST keep parsing (pinned by test).
- `eval/gate.rs` and `scripts/ci.sh` must not change.
- Corpus expected values pin CURRENT behavior (regression net, not wishlist).

---

### Task 1: Types + comparator (eval/result.rs, eval/task.rs)

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/eval/result.rs` (TrajectoryStep, RunResult fields, comparator, tests)
- Modify: `agent/crates/agent-runtime-config/src/eval/task.rs` (gold_trajectory field, serde test)
- Modify: `agent/crates/agent-runtime-config/src/eval/mod.rs` (re-export TrajectoryStep + trajectory_matches_gold, matching existing export style)

**Interfaces:**
- Produces: `TrajectoryStep { pub tool: String, pub args: serde_json::Value }`; `RunResult` extra fields `trajectory: Vec<TrajectoryStep>`, `denials: usize`, `gold_matched: Option<bool>` (all `#[serde(default)]`); `TaskSpec.gold_trajectory: Vec<String>` (`#[serde(default)]`); `pub fn trajectory_matches_gold(trajectory: &[TrajectoryStep], gold: &[String]) -> bool`. Task 2 consumes all of these.

- [ ] **Step 1: Failing tests** (result.rs tests mod):

```rust
fn step(tool: &str) -> TrajectoryStep {
    TrajectoryStep { tool: tool.into(), args: serde_json::json!({}) }
}
fn gold(names: &[&str]) -> Vec<String> { names.iter().map(|s| s.to_string()).collect() }

#[test]
fn gold_subsequence_semantics() {
    let traj: Vec<_> = ["read_file", "grep_x", "edit_file", "cargo_test"].iter().map(|t| step(t)).collect();
    assert!(trajectory_matches_gold(&traj, &gold(&["read_file", "edit_file", "cargo_test"]))); // extras ok
    assert!(trajectory_matches_gold(&traj, &gold(&[])));                                        // empty gold vacuous
    assert!(!trajectory_matches_gold(&traj, &gold(&["edit_file", "read_file"])));               // order violated
    assert!(!trajectory_matches_gold(&traj, &gold(&["write_file"])));                           // missing tool
    assert!(!trajectory_matches_gold(&[], &gold(&["read_file"])));                              // empty traj, non-empty gold
}

#[test]
fn run_result_old_json_still_parses() {
    let old = r#"{"passed":true,"tokens":123,"turns":4}"#;
    let r: RunResult = serde_json::from_str(old).unwrap();
    assert!(r.trajectory.is_empty());
    assert_eq!(r.denials, 0);
    assert_eq!(r.gold_matched, None);
}
```

And in task.rs tests: parse the existing `JSON` const (no gold_trajectory field) → `gold_trajectory.is_empty()`; parse a variant with `"gold_trajectory": ["read_file"]`.

- [ ] **Step 2: Verify failing** — `cd agent && cargo test -p agent-runtime-config gold_` → FAIL (missing items).

- [ ] **Step 3: Implement** per spec §1-2:

```rust
/// One tool invocation observed during an eval run (ToolStart order).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrajectoryStep {
    pub tool: String,
    pub args: serde_json::Value,
}

/// True iff `gold` (tool names) appears as an ordered subsequence of the
/// trajectory's tool names. Extra calls in between are allowed; order is not.
/// Empty gold is vacuously true. Diagnostic only — the promotion gate does not
/// consume it (spec 2026-07-02 eval-flywheel).
pub fn trajectory_matches_gold(trajectory: &[TrajectoryStep], gold: &[String]) -> bool {
    let mut want = gold.iter();
    let mut next = want.next();
    for s in trajectory {
        if let Some(g) = next {
            if &s.tool == g {
                next = want.next();
            }
        }
    }
    next.is_none()
}
```

RunResult additions (after `turns`):

```rust
    /// Ordered ToolStart capture (diagnostic; additive — old lines parse).
    #[serde(default)]
    pub trajectory: Vec<TrajectoryStep>,
    /// SafeApproval denials during the run (silent friction made visible).
    #[serde(default)]
    pub denials: usize,
    /// Some(matched) when the task defines a non-empty gold_trajectory.
    #[serde(default)]
    pub gold_matched: Option<bool>,
```

TaskSpec addition: `#[serde(default)] pub gold_trajectory: Vec<String>` with a doc comment ("ordered tool-name subsequence a correct run is expected to contain; empty = no process expectation"). Fix any struct literals that now miss fields (result.rs tests `rr` helper, eval_context.rs/soak_live.rs compile in Task 2 — if they break HERE, add `..Default::default()`-style or explicit fields minimally; RunResult has no Default — extend the `rr` helper explicitly).

- [ ] **Step 4: Verify** — `cargo test -p agent-runtime-config` (lib+tests may fail to compile in tests/eval_context.rs because RunResult literal misses fields — if so, add the three fields there with empty/0/None values as a stopgap; Task 2 fills them properly) → PASS.

- [ ] **Step 5:** fmt, commit — `feat(eval): TrajectoryStep + RunResult trajectory/denials/gold_matched + gold-subsequence comparator`

---

### Task 2: Harness wiring (tests/eval_context.rs)

**Files:**
- Modify: `agent/crates/agent-runtime-config/tests/eval_context.rs` (TokenMeter capture, denials emit, gold_matched fill)

**Interfaces:**
- Consumes Task 1's types. The `TokenMeter` sink (eval_context.rs ~line 68) and `SafeApproval` (~line 25) are file-local — read them fully first.

- [ ] **Step 1:** Extend `TokenMeter` with `trajectory: Mutex<Vec<TrajectoryStep>>`; in its `emit`, add a `ToolStart { name, args, .. }` arm pushing `TrajectoryStep { tool: name, args }`. (The struct derives Default — Mutex<Vec<_>> is Default-compatible.)

- [ ] **Step 2:** At RunResult construction (~line 239): move `SafeApproval` behind a named binding BEFORE `assemble_loop` (it's currently constructed inline as `Arc::new(SafeApproval {..})` — hoist to `let approval = Arc::new(SafeApproval { denied: Mutex::new(Vec::new()) });` and pass `approval.clone()`), then:

```rust
let denied = approval.denied.lock().unwrap();
for d in denied.iter() {
    eprintln!("eval-denied: {d}");
}
let trajectory = meter.trajectory.lock().unwrap().clone();
let gold_matched = if task.gold_trajectory.is_empty() {
    None
} else {
    Some(agent_runtime_config::eval::trajectory_matches_gold(
        &trajectory,
        &task.gold_trajectory,
    ))
};
let result = RunResult {
    passed: status.success(),
    tokens: meter.total.load(Ordering::Relaxed),
    turns: meter.turns.load(Ordering::Relaxed) as usize,
    trajectory,
    denials: denied.len(),
    gold_matched,
};
```

(stdout stays exactly one JSON line; denials go to stderr only.)

- [ ] **Step 3: Verify** — `cargo test -p agent-runtime-config` (this file is an ignored live test; compilation is the gate here) plus `cargo test -p agent-runtime-config --test eval_context` compiles. Run the full crate suite.

- [ ] **Step 4:** fmt, commit — `feat(eval): capture trajectory + denials in the eval harness, fill gold_matched`

---

### Task 3: Adversarial policy corpus

**Files:**
- Create: `agent/crates/agent-runtime-config/tests/policy_corpus.rs`
- Create: `agent/crates/agent-runtime-config/tests/policy_corpus.tsv`

- [ ] **Step 1:** Read how production builds the policy engine: `assemble_loop`/`loop_config_from` in `agent-runtime-config/src/assemble.rs` (grep `RulePolicy` / `PolicyEngine`) and `agent-policy/src/engine.rs::check`. The test must construct the SAME engine with `default_denylist()` + `default_allowlist()` and feed `ToolIntent`s the way `execute_command` produces them (read `agent-tools/src/shell.rs::intent` for the exact ToolIntent shape: tool name, Access, command field).

- [ ] **Step 2:** Write the TSV (≥30 rows, `expected<TAB>command`, `#` comments allowed). Content per spec §4 seed list — every row's expectation verified against CURRENT behavior by running the test, with `#` comments on rows documenting accepted asymmetries (redirection, exec-vehicle→ask). Layout sample:

```
# expected	command
deny	rm -rf /
deny	rm  -rf   /
deny	:(){ :|:& };:
deny	mkfs.ext4 /dev/sda1
deny	dd if=/dev/zero of=/dev/sda
ask	echo x > /dev/sda	# redirection asymmetry — documented, reaches Ask not Deny
ask	git push --force
ask	git reset --hard
ask	git clean -fdx
ask	git log --output=/tmp/x	# cluster-4 arg-scan
allow	git log --oneline
allow	git diff --output-indicator-new=+
allow	git status
allow	cargo build
ask	bash -c "sudo reboot"	# exec-vehicle residual — Ask, not Deny
ask	find . -exec sudo reboot ;
allow	grep -o pat file
...
```

(Tabs are load-bearing; inline `#` comments after a tab are stripped by the parser you write. Fill the full seed list from spec §4.)

- [ ] **Step 3:** The test: parse the TSV (loud errors with line numbers), for each row compute the decision through the engine, collect ALL mismatches, assert none with a report listing every failing line. Temporarily flip one row to prove loud failure; revert (do not commit the flip).

- [ ] **Step 4: Verify** — `cargo test -p agent-runtime-config --test policy_corpus` → PASS; full crate suite green.

- [ ] **Step 5:** fmt, commit — `test(policy): data-driven adversarial command corpus — one-line regression additions`

---

### Task 4: Coverage CI job

**Files:**
- Modify: `.github/workflows/ci.yml` (add the `coverage` job exactly as spec §5; match the existing job's checkout/toolchain/cache action versions — read the file first and reuse its action pins)

- [ ] **Step 1:** Add the job. `scripts/ci.sh` untouched.
- [ ] **Step 2: Verify** — YAML parses (`python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"`); if `cargo llvm-cov --version` exists locally, smoke-run `cargo llvm-cov -p agent-policy --summary-only` (else note in report that verification is deferred to the first Actions run).
- [ ] **Step 3:** Commit — `ci: coverage job (cargo llvm-cov summary) alongside the main gate`

---

### Task 5: Cluster gate

- [ ] `bash scripts/ci.sh` (background if cold) → green. No commit expected.
