# harness-evolve Campaign Bring-Up Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the harness-evolve campaign per `docs/superpowers/specs/2026-07-03-harness-evolve-campaign-design.md`: phase-0 `node-offline` sandboxed exec profile, CandidateConfig v2 genome widening (+ tool-description seam + `wall_ms`), the campaign skill scaffolding, and the admitted `web-multipage` discriminator task.

**Architecture:** All eval-genome changes are additive `#[serde(default)]` fields that inherit on `None` (the shipped `system_prompt`/`protocol` pattern), applied onto `RuntimeConfig` by a new unit-testable `CandidateConfig::apply_to`. Web tasks opt into the docker sandbox via `TaskSpec.exec_profile: "node-offline"` (network none, pre-seeded node_modules, grading in-container); `exec_profile: None` tasks are byte-identical to today. The campaign skill mirrors context-evolve's four-doc layout.

**Tech Stack:** Rust (two crates: `agent-runtime-config`, `agent-tools`, plus one flag-default change in `agent-sandbox`), docker (`node:22-bookworm-slim`), Vite + TypeScript + Vitest for the task workspace.

## Global Constraints

- Everything runs in the `agent/` cargo workspace: `source ~/.cargo/env && cd /home/kalen/rust-agent-runtime/agent` before any cargo command.
- New serde fields MUST be `#[serde(default)]` and inherit-on-`None`/empty — existing frozen JSON (champion configs, task specs, RunResult JSONL) must parse unchanged; each task pins that with a back-compat test.
- `exec_profile: None` behavior must be byte-identical to today (sandbox off, host grading, 120 s prompt timeout, max_turns 12).
- Promotion/admission logic (`eval/gate.rs`, `eval/admissibility.rs`, `eval_gate` bin) is UNTOUCHED.
- Conventional commits (`type(scope): summary`); commit per task; do NOT push.
- Live-eval steps need: llama server healthy (`curl -s localhost:8080/health`), `AGENT_E2E_URL=http://localhost:8080` (no `/v1`), `AGENT_E2E_MODEL=qwen3.6-35b-a3b`, ABSOLUTE paths for `TASK_JSON`/`CONFIG_JSON`/`HIDDEN_TESTS_DIR`.
- Never modify frozen context-evolve task dirs.

---

### Task 1: CandidateConfig v2 — genome fields + `apply_to`

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/eval/config.rs`

**Interfaces:**
- Consumes: `crate::ModelRef` (exists, re-exported at crate root), `crate::RuntimeConfig`.
- Produces: new `CandidateConfig` fields (below) + `pub fn apply_to(&self, cfg: &mut RuntimeConfig)` — Task 4 calls it; `offload_config()` now honors `max_result_bytes`.

- [ ] **Step 1: Write the failing tests** (append to the existing `widening_tests` module in `eval/config.rs`):

```rust
    #[test]
    fn v2_fields_default_to_none_and_parse_from_v1_json() {
        // A pre-v2 config (the existing field set only) must still deserialize.
        let json = serde_json::to_value(CandidateConfig::favorable(8192)).unwrap();
        let mut obj = json.as_object().unwrap().clone();
        for k in [
            "active_skills", "skills_dirs", "temperature", "top_p", "top_k", "min_p",
            "subagents", "subagent_max_turns", "subagent_max_depth", "subagent_model",
            "tool_descriptions", "max_result_bytes", "max_turns",
        ] {
            obj.remove(k);
        }
        let cc: CandidateConfig = serde_json::from_value(serde_json::Value::Object(obj)).unwrap();
        assert!(cc.temperature.is_none() && cc.subagent_model.is_none());
        assert!(cc.tool_descriptions.is_none() && cc.max_turns.is_none());
        // favorable leaves every v2 field None
        let f = CandidateConfig::favorable(8192);
        assert!(f.active_skills.is_none() && f.skills_dirs.is_none());
        assert!(f.subagents.is_none() && f.max_result_bytes.is_none());
    }

    #[test]
    fn apply_to_inherits_on_none_and_overrides_on_some() {
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(), "http://x".into(), "m".into(), "native".into(), 8192,
        );
        let baseline = cfg.clone();
        CandidateConfig::favorable(8192).apply_to(&mut cfg);
        assert_eq!(cfg, baseline, "all-None candidate must not touch the config");

        let mut cc = CandidateConfig::favorable(8192);
        cc.temperature = Some(0.7);
        cc.top_k = Some(40);
        cc.subagents = Some(false);
        cc.subagent_max_turns = Some(4);
        cc.subagent_max_depth = Some(2);
        cc.subagent_model = Some(ModelRef { model: Some("other".into()), ..Default::default() });
        cc.skills_dirs = Some(vec!["/skills".into()]);
        cc.active_skills = Some(vec!["sdlc".into()]);
        cc.max_turns = Some(30);
        cc.tool_descriptions =
            Some([("read_file".to_string(), "OVERRIDE".to_string())].into_iter().collect());
        cc.apply_to(&mut cfg);
        assert_eq!(cfg.temperature, 0.7);
        assert_eq!(cfg.top_k, Some(40));
        assert!(!cfg.subagents);
        assert_eq!(cfg.subagent_max_turns, 4);
        assert_eq!(cfg.subagent_max_depth, 2);
        assert_eq!(cfg.subagent_model.as_ref().unwrap().model.as_deref(), Some("other"));
        assert_eq!(cfg.skills_dirs, vec!["/skills".to_string()]);
        assert_eq!(cfg.active_skills, vec!["sdlc".to_string()]);
        assert_eq!(cfg.max_turns, 30);
        assert_eq!(cfg.tool_description_overrides.get("read_file").unwrap(), "OVERRIDE");
    }

    #[test]
    fn max_result_bytes_defaults_to_neutralized_and_overrides() {
        let f = CandidateConfig::favorable(8192);
        assert_eq!(f.offload_config().max_result_bytes, usize::MAX);
        let mut cc = CandidateConfig::favorable(8192);
        cc.max_result_bytes = Some(16 * 1024);
        assert_eq!(cc.offload_config().max_result_bytes, 16 * 1024);
    }
```

NOTE: `apply_to`'s `tool_description_overrides` line requires Task 3's RuntimeConfig field. To keep this task independently green, write `apply_to` WITHOUT the tool_descriptions arm now, comment the two `tool_descriptions`/`tool_description_overrides` assert lines with `// enabled in Task 3`, and re-enable them in Task 3 Step 4.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent-runtime-config --lib eval::config`
Expected: FAIL — unknown fields / no method `apply_to`.

- [ ] **Step 3: Implement.** Add to `CandidateConfig` (after `protocol`):

```rust
    // ---- v2 genome axes (spec 2026-07-03 harness-evolve). All inherit-on-None. ----
    /// Runtime skills inlined into the system prompt (axis 5). None = inherit.
    #[serde(default)]
    pub active_skills: Option<Vec<String>>,
    /// Skill tree roots served to the loop (axis 5). None = inherit.
    #[serde(default)]
    pub skills_dirs: Option<Vec<String>>,
    /// Sampler overrides (axis 7a). None = inherit the harness default.
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub min_p: Option<f32>,
    /// Sub-agent policy (axis 4). None = inherit.
    #[serde(default)]
    pub subagents: Option<bool>,
    #[serde(default)]
    pub subagent_max_turns: Option<usize>,
    #[serde(default)]
    pub subagent_max_depth: Option<usize>,
    #[serde(default)]
    pub subagent_model: Option<crate::ModelRef>,
    /// Per-tool description overrides (axis 3). None = every tool's own schema text.
    #[serde(default)]
    pub tool_descriptions: Option<std::collections::HashMap<String, String>>,
    /// Ingestion cap (axis 8). None = the eval's historical "cap off" semantics.
    #[serde(default)]
    pub max_result_bytes: Option<usize>,
    /// Per-prompt turn budget. None = the driver default (12).
    #[serde(default)]
    pub max_turns: Option<usize>,
```

Add the resolver (below `resolved_protocol`):

```rust
    /// Overlay every `Some` v2 field onto a RuntimeConfig; `None` fields leave
    /// the config untouched (inherit). Unit-testable without the live harness.
    pub fn apply_to(&self, cfg: &mut crate::RuntimeConfig) {
        if let Some(v) = self.temperature {
            cfg.temperature = v;
        }
        if let Some(v) = self.top_p {
            cfg.top_p = Some(v);
        }
        if let Some(v) = self.top_k {
            cfg.top_k = Some(v);
        }
        if let Some(v) = self.min_p {
            cfg.min_p = Some(v);
        }
        if let Some(v) = self.subagents {
            cfg.subagents = v;
        }
        if let Some(v) = self.subagent_max_turns {
            cfg.subagent_max_turns = v;
        }
        if let Some(v) = self.subagent_max_depth {
            cfg.subagent_max_depth = v;
        }
        if let Some(v) = &self.subagent_model {
            cfg.subagent_model = Some(v.clone());
        }
        if let Some(v) = &self.skills_dirs {
            cfg.skills_dirs = v.clone();
        }
        if let Some(v) = &self.active_skills {
            cfg.active_skills = v.clone();
        }
        if let Some(v) = self.max_turns {
            cfg.max_turns = v;
        }
        // tool_descriptions arm added in Task 3 (needs the RuntimeConfig field):
        // if let Some(v) = &self.tool_descriptions {
        //     cfg.tool_description_overrides = v.clone();
        // }
    }
```

In `favorable(window)`, set every new field to `None`. In `offload_config()`, change the `max_result_bytes` line to:

```rust
            // None = the eval's historical "ingestion cap off" semantics (the
            // context-evolve champion was validated without the cap). Some(n)
            // lets a candidate opt into a realistic cap (harness-evolve axis 8).
            max_result_bytes: self.max_result_bytes.unwrap_or(usize::MAX),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-runtime-config --lib eval::config`
Expected: PASS (with the two Task-3 asserts commented).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/eval/config.rs
git commit -m "feat(eval): CandidateConfig v2 — skills/sampler/subagent/cap/turns genome axes with apply_to"
```

---

### Task 2: `wall_ms` on RunResult

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/eval/result.rs`

**Interfaces:**
- Produces: `RunResult.wall_ms: u64` (serde-default 0). Gate/admit read only `passed`/`tokens` — untouched.

- [ ] **Step 1: Write the failing test** (in `eval/result.rs` tests):

```rust
    #[test]
    fn wall_ms_defaults_to_zero_on_old_jsonl() {
        let old = r#"{"passed":true,"tokens":123,"turns":4}"#;
        let r: RunResult = serde_json::from_str(old).unwrap();
        assert_eq!(r.wall_ms, 0);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p agent-runtime-config --lib eval::result`
Expected: FAIL — no field `wall_ms`.

- [ ] **Step 3: Implement.** Add to `RunResult` after `gold_matched`:

```rust
    /// Wall-clock for the whole run in ms. DIAGNOSTIC ONLY — never gates
    /// (shared llama server makes wall-clock noisy); tokens stay the cost metric.
    #[serde(default)]
    pub wall_ms: u64,
```

Fix the two struct literals in this file's tests (`rr()` helper and `admissibility.rs`'s `batch()` helper) by adding `wall_ms: 0,`.

- [ ] **Step 4: Run the crate's unit tests**

Run: `cargo test -p agent-runtime-config --lib`
Expected: PASS (compiler surfaces every struct literal to fix).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/eval/result.rs agent/crates/agent-runtime-config/src/eval/admissibility.rs
git commit -m "feat(eval): additive wall_ms diagnostic on RunResult"
```

---

### Task 3: Tool-description override seam (registry + RuntimeConfig + assemble)

Builds the recorded sketch in `docs/superpowers/specs/2026-07-02-tool-description-override-seam-design.md` — its revisit trigger fired. Anchor points re-verified 2026-07-03: `ToolRegistry::schemas()` at `agent-tools/src/registry.rs:33`, per-turn consumption via `AgentLoop`, registry built inside `assemble_loop` with `let schemas = registry.schemas()` at `assemble.rs:266`.

**Files:**
- Modify: `agent/crates/agent-tools/src/registry.rs`
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs`
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs:266` (one line above it)
- Modify: `agent/crates/agent-runtime-config/src/eval/config.rs` (enable the Task-1 arm)

**Interfaces:**
- Produces: `ToolRegistry::set_description_overrides(HashMap<String,String>)`; `RuntimeConfig.tool_description_overrides: HashMap<String,String>` (serde-default empty).

- [ ] **Step 1: Write the failing registry tests** (in `agent-tools/src/registry.rs` tests, reusing the existing `Echo`/`Confusable` fixtures):

```rust
    #[test]
    fn description_override_replaces_base_but_keeps_exclusion_fold() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(Echo));
        r.register(Arc::new(Confusable));
        r.set_description_overrides(
            [
                ("echo".to_string(), "OVERRIDDEN".to_string()),
                ("confusable".to_string(), "NEW BASE".to_string()),
                ("missing".to_string(), "ignored".to_string()), // unknown: warn + ignore
            ]
            .into_iter()
            .collect(),
        );
        let schemas = r.schemas();
        let echo = schemas.iter().find(|s| s.name == "echo").unwrap();
        assert_eq!(echo.description, "OVERRIDDEN");
        let conf = schemas.iter().find(|s| s.name == "confusable").unwrap();
        assert!(conf.description.starts_with("NEW BASE"), "override replaces the BASE");
        assert!(
            conf.description.contains(WHEN_NOT_TO_CALL_MARKER)
                && conf.description.contains("use echo instead for X"),
            "when_not_to_call fold still appends after the override"
        );
    }

    #[test]
    fn empty_overrides_change_nothing() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(Echo));
        r.set_description_overrides(Default::default());
        assert_eq!(r.schemas()[0].description, "echoes");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-tools registry`
Expected: FAIL — no method `set_description_overrides`.

- [ ] **Step 3: Implement the registry layer.** In `ToolRegistry`:

```rust
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    /// Per-tool BASE-description replacements (eval/optimizer seam). The
    /// `when_not_to_call` fold still appends afterwards, so exclusion prose
    /// survives unless deliberately included in the override text.
    description_overrides: HashMap<String, String>,
}
```

```rust
    /// Replace base descriptions per tool name. Unknown names warn and are
    /// ignored (mirrors the unknown-preset handling in assemble).
    pub fn set_description_overrides(&mut self, overrides: HashMap<String, String>) {
        for name in overrides.keys() {
            if !self.tools.contains_key(name) {
                tracing::warn!(tool = %name, "description override for unknown tool — ignored");
            }
        }
        self.description_overrides = overrides;
    }
```

And in `schemas()`, before the `when_not_to_call` fold:

```rust
                let mut s = t.schema();
                if let Some(over) = self.description_overrides.get(t.name()) {
                    s.description = over.clone();
                }
                if let Some(excl) = t.when_not_to_call() {
```

- [ ] **Step 4: Thread through RuntimeConfig + assemble + eval genome.**
  - `runtime_config.rs`: add to `RuntimeConfig` (after `post_tool_validators`): `#[serde(default)] pub tool_description_overrides: std::collections::HashMap<String, String>,`; add `tool_description_overrides: Option<std::collections::HashMap<String, String>>,` to `PartialRuntimeConfig`; add the merge arm `if let Some(v) = p.tool_description_overrides { self.tool_description_overrides = v; }`; add `tool_description_overrides: Default::default(),` to `from_launch`.
  - `assemble.rs`: immediately before `let schemas = registry.schemas();` (line ~266, AFTER every registration including `DispatchAgentTool`):

```rust
    registry.set_description_overrides(cfg.tool_description_overrides.clone());
```

  - `eval/config.rs`: un-comment the Task-1 arm in `apply_to` and the two asserts in `apply_to_inherits_on_none_and_overrides_on_some`.

- [ ] **Step 5: Run both crates' tests**

Run: `cargo test -p agent-tools && cargo test -p agent-runtime-config --lib`
Expected: PASS. If a RuntimeConfig round-trip/exhaustiveness guard test fails on the new field, extend that test's literal with `tool_description_overrides: Default::default(),` — that guard existing is expected.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-tools/src/registry.rs agent/crates/agent-runtime-config/src/runtime_config.rs agent/crates/agent-runtime-config/src/assemble.rs agent/crates/agent-runtime-config/src/eval/config.rs
git commit -m "feat(tools): registry description-override seam wired through RuntimeConfig and the eval genome"
```

---

### Task 4: TaskSpec additions — `exec_profile`, `seed_dir`, `prompt_timeout_secs`

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/eval/task.rs`

**Interfaces:**
- Produces: `TaskSpec.exec_profile: Option<String>` (`None` | `Some("node-offline")`), `TaskSpec.seed_dir: Option<String>` (dir copied into the workspace before `seed_files`, resolved relative to task.json's parent), `TaskSpec.prompt_timeout_secs: Option<u64>` (default 120 applied by the driver).

- [ ] **Step 1: Write the failing test** (in `eval/task.rs` tests):

```rust
    #[test]
    fn phase0_fields_default_absent_and_parse_when_present() {
        let t = TaskSpec::from_json(JSON).unwrap(); // existing fixture: no new fields
        assert!(t.exec_profile.is_none() && t.seed_dir.is_none());
        assert!(t.prompt_timeout_secs.is_none());
        let json = r#"{
          "id": "w", "mode": "code", "realistic_window": 8000,
          "favorable_window": 196608, "memory_enabled": false, "seed_files": [],
          "test_cmd": "true", "sessions": [{ "prompts": ["p"] }],
          "exec_profile": "node-offline", "seed_dir": "seed",
          "prompt_timeout_secs": 600
        }"#;
        let t = TaskSpec::from_json(json).unwrap();
        assert_eq!(t.exec_profile.as_deref(), Some("node-offline"));
        assert_eq!(t.seed_dir.as_deref(), Some("seed"));
        assert_eq!(t.prompt_timeout_secs, Some(600));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-runtime-config --lib eval::task`
Expected: FAIL — unknown fields.

- [ ] **Step 3: Implement.** Add to `TaskSpec` after `gold_trajectory`:

```rust
    /// Execution profile. None = host semantics (sandbox off — every pre-2026-07-03
    /// task). "node-offline" = docker sandbox enforced, node image, network none,
    /// grading in-container (spec 2026-07-03 harness-evolve phase-0).
    #[serde(default)]
    pub exec_profile: Option<String>,
    /// Directory copied recursively into the workspace BEFORE seed_files, resolved
    /// relative to task.json's parent dir. Carries trees seed_files can't (node_modules).
    #[serde(default)]
    pub seed_dir: Option<String>,
    /// Per-prompt driver timeout in seconds. None = 120 (the historical value).
    #[serde(default)]
    pub prompt_timeout_secs: Option<u64>,
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agent-runtime-config --lib eval::task`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/eval/task.rs
git commit -m "feat(eval): TaskSpec exec_profile / seed_dir / prompt_timeout_secs for phase-0 web tasks"
```

---

### Task 5: Docker HOME default (node tooling inside `--read-only`)

With `--user uid:gid` and no passwd entry, HOME inside the container is `/` (read-only) — `npx`'s cache mkdir would fail. Default HOME to the writable tmpfs unless the spec sets it.

**Files:**
- Modify: `agent/crates/agent-sandbox/src/docker.rs`

**Interfaces:**
- Produces: `docker_run_args` emits `-e HOME=/tmp` iff `spec.env` has no `HOME` key.

- [ ] **Step 1: Write the failing test** (in `docker.rs` tests):

```rust
    #[test]
    fn home_defaults_to_tmp_unless_spec_sets_it() {
        let v = docker_run_args(&policy(false), &oneshot(), "n", "1000:1000");
        assert!(v.join(" ").contains("-e HOME=/tmp"), "default HOME on writable tmpfs");
        let mut spec = oneshot();
        spec.env.insert("HOME".into(), "/workspace".into());
        let s = docker_run_args(&policy(false), &spec, "n", "1000:1000").join(" ");
        assert!(s.contains("-e HOME=/workspace") && !s.contains("-e HOME=/tmp"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-sandbox`
Expected: FAIL on the new test.

- [ ] **Step 3: Implement.** In `docker_run_args`, immediately after the `for (k, v) in &spec.env` loop:

```rust
    // --user with no passwd entry leaves HOME=/ (read-only) — node/npx tooling
    // needs a writable HOME for caches. Default to the tmpfs unless the spec set one.
    if !spec.env.contains_key("HOME") {
        a.push("-e".into());
        a.push("HOME=/tmp".into());
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p agent-sandbox`
Expected: PASS (existing flag tests unaffected — they don't assert env absence).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-sandbox/src/docker.rs
git commit -m "fix(sandbox): default HOME=/tmp in docker runs so node tooling works under --read-only"
```

---

### Task 6: Eval driver — genome application, node-offline profile, sandboxed grading, wall_ms

**Files:**
- Modify: `agent/crates/agent-runtime-config/tests/eval_context.rs`

**Interfaces:**
- Consumes: `CandidateConfig::apply_to` (Task 1), TaskSpec fields (Task 4), `agent_sandbox::{docker_run_args, SandboxPolicy, WORKDIR}` (agent-runtime-config already depends on agent-sandbox via `build_sandbox`), `agent_tools::{CommandSpec, Limits, Mode, ProcKind}`.
- Produces: the live driver used by every admission/iteration run in Tasks 8–9.

This is test-binary code (`#[ignore]` live harness) — no unit tests of its own; Task 9's live runs are its verification. Make each edit exactly as shown.

- [ ] **Step 1: SafeApproval learns the node profile.** Change the struct and its match arm:

```rust
struct SafeApproval {
    denied: Mutex<Vec<String>>,
    /// True for exec_profile == "node-offline": node/npx/vite/vitest/tsc are
    /// approvable because execution is docker-contained (network none, read-only
    /// root). NOT extended to npm install/ci — offline by construction.
    node_profile: bool,
}
```

In the `"execute_command"` arm, replace the `matches!(...)` expression with:

```rust
                    matches!(
                        base,
                        "ls" | "cat" | "wc" | "head" | "tail" | "echo" | "grep" | "find" | "pwd"
                            | "sort" | "uniq" | "true" | "date" | "nl"
                            // `cargo` for code tasks (e.g. locked-hostpolicy): lets the agent
                            // build/check its work. Bounded: eval crates are std-only, no deps,
                            // no build.rs — so cargo only invokes rustc on trusted local source.
                            | "cargo"
                    ) || (self.node_profile
                        && matches!(base, "node" | "npx" | "tsc" | "vitest" | "vite"))
```

- [ ] **Step 2: Task-dir + profile resolution.** After `let hidden = …` add:

```rust
    let task_json_path = std::path::PathBuf::from(std::env::var("TASK_JSON").unwrap());
    let task_dir = task_json_path.parent().expect("TASK_JSON has a parent dir").to_path_buf();
    let node_offline = task.exec_profile.as_deref() == Some("node-offline");
```

Update the approval construction:

```rust
    let approval = Arc::new(SafeApproval {
        denied: Mutex::new(Vec::new()),
        node_profile: node_offline,
    });
```

- [ ] **Step 3: seed_dir copy (before the seed_files loop).**

```rust
    if let Some(sd) = &task.seed_dir {
        let src = task_dir.join(sd);
        assert!(src.is_dir(), "seed_dir {} missing — run the task's seed.sh first", src.display());
        // cp -a preserves the node_modules tree (symlinked .bin entries included).
        let st = std::process::Command::new("cp")
            .arg("-a")
            .arg(format!("{}/.", src.display()))
            .arg(&ws)
            .status()
            .unwrap();
        assert!(st.success(), "seed_dir copy failed");
    }
```

- [ ] **Step 4: Config application per session.** Replace the block

```rust
        cfg.sandbox_mode = "off".into();
        cfg.max_turns = 12;
```

with:

```rust
        if node_offline {
            // Phase-0 node-offline profile (spec 2026-07-03): enforced docker
            // sandbox, pinned node image, network stays none (the default).
            cfg.sandbox_mode = "enforce".into();
            cfg.sandbox_image = "node:22-bookworm-slim".into();
            cfg.sandbox_memory = "4g".into();
            cfg.sandbox_pids = 1024;
        } else {
            cfg.sandbox_mode = "off".into();
        }
        cfg.max_turns = 12; // historical default; candidates override via max_turns
        cc.apply_to(&mut cfg);
```

- [ ] **Step 5: Prompt timeout + wall clock.** Before the sessions loop add `let started = std::time::Instant::now();` and inside the prompt loop replace the fixed timeout:

```rust
        let per_prompt = Duration::from_secs(task.prompt_timeout_secs.unwrap_or(120));
        for prompt in &session.prompts {
            let cancel = tokio_util::sync::CancellationToken::new();
            let run = agent.run_with_cancel(&mut ctx, prompt.clone(), cancel.clone());
            let _ = tokio::time::timeout(per_prompt, run).await;
        }
```

- [ ] **Step 6: Sandboxed grading.** Replace the host `bash -c test_cmd` block with:

```rust
    let status = if node_offline {
        // Grade INSIDE the same container profile: vitest/vite execute agent-written
        // code (untrusted output) — it gets the same boundary as the agent.
        let uid = String::from_utf8(std::process::Command::new("id").arg("-u").output().unwrap().stdout).unwrap();
        let gid = String::from_utf8(std::process::Command::new("id").arg("-g").output().unwrap().stdout).unwrap();
        let policy = agent_sandbox::SandboxPolicy {
            mode: agent_tools::Mode::Enforce,
            image: "node:22-bookworm-slim".into(),
            network: false,
            limits: agent_tools::Limits {
                memory: "4g".into(),
                cpus: "2".into(),
                pids: 1024,
                fsize: None,
                tmp_size: "256m".into(),
            },
            extra_rw: vec![],
            extra_ro: vec![],
        };
        let spec = agent_tools::CommandSpec {
            program: "bash".into(),
            args: vec!["-c".into(), task.test_cmd.clone()],
            cwd: ws.clone(),
            env: Default::default(),
            kind: agent_tools::ProcKind::OneShot,
        };
        let name = format!("eval-grade-{}", std::process::id());
        let args = agent_sandbox::docker_run_args(
            &policy,
            &spec,
            &name,
            &format!("{}:{}", uid.trim(), gid.trim()),
        );
        std::process::Command::new("docker").args(&args).status().unwrap()
    } else {
        std::process::Command::new("bash")
            .arg("-c")
            .arg(&task.test_cmd)
            .current_dir(&ws)
            .status()
            .unwrap()
    };
```

Check the exact field/variant names in `agent-tools` (`Mode::Enforce`, `Limits`, `CommandSpec`, `ProcKind`) against source and adjust imports at the top of the file: `use agent_sandbox::docker_run_args;` etc. If `Mode` has no `Enforce` variant, use whichever variant the sandbox strategy uses for enforce — the field is not consulted by `docker_run_args`.

- [ ] **Step 7: wall_ms into the result.** In the `RunResult` literal add:

```rust
        wall_ms: started.elapsed().as_millis() as u64,
```

- [ ] **Step 8: Compile the test binary**

Run: `cargo build -p agent-runtime-config --tests`
Expected: clean build.

- [ ] **Step 9: Inertness spot-check (None-profile path unchanged).** With the llama server up:

```bash
T=/home/kalen/rust-agent-runtime/.agents/skills/context-evolve/tasks/locked-portmap
run() { AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
  TASK_JSON=$T/task.json CONFIG_JSON="$1" HIDDEN_TESTS_DIR=$T/hidden_tests \
  cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture 2>&1 \
  | grep -E '^\{"passed"'; }
: > /tmp/he_spotcheck.jsonl
for i in 1 2 3 4 5 6; do run /home/kalen/rust-agent-runtime/.agents/skills/context-evolve/tasks/drift-ledger/champion_v4.json >> /tmp/he_spotcheck.jsonl; done
```

Expected: 6/6 `"passed":true` (portmap ceiling is 10/10; a single miss → re-run per prefix-identity attribution before suspecting the driver). Also confirm `wall_ms` is nonzero in the output lines.

- [ ] **Step 10: Commit**

```bash
git add agent/crates/agent-runtime-config/tests/eval_context.rs
git commit -m "feat(eval): node-offline exec profile — sandboxed exec + grading, genome apply_to, wall_ms, per-task timeout"
```

---

### Task 7: Campaign skill scaffolding

**Files:**
- Create: `.agents/skills/harness-evolve/SKILL.md`
- Create: `.agents/skills/harness-evolve/prepare.md`
- Create: `.agents/skills/harness-evolve/train.md`
- Create: `.agents/skills/harness-evolve/program.md`
- Create: `.agents/skills/harness-evolve/artifacts/agent-skills/.gitkeep`

Content mirrors context-evolve's docs, adapted per the spec. Write the files exactly as drafted below (they are the deliverable; adjust only factual drift discovered during Tasks 1–6).

- [ ] **Step 1: Write SKILL.md**

```markdown
---
name: harness-evolve
description: >-
  Use to run a self-improving optimization campaign on this runtime's WHOLE
  harness (system prompt, tools, sub-agents, agent-side SDLC skills, memory,
  sampling, server topology) against long-running web-coding tasks. Iteratively
  edits genome/config/code, evals against a live model on frozen tasks, and
  keeps a change only when task success holds and total tokens drop. Invoke to
  optimize the harness for complex programming tasks; for context-curation
  tuning use context-evolve.
---

# harness-evolve

Optimize the harness so the running model finishes long, complex programming
tasks (canonical: a working TypeScript website — Vite, typecheck, tests) at a
realistic window. Sibling campaign to `context-evolve` — same method, wider
genome. Spec: `docs/superpowers/specs/2026-07-03-harness-evolve-campaign-design.md`.

- `prepare.md` — author/admit a web task (offline seed, exec_profile, ladder).
- `train.md` — the per-iteration loop and the cross-campaign guard sweep.
- `program.md` — append-only research memory + current champion. READ FIRST.

## The objective (never violate)

1. A change that lowers the pass count on the training set is **rejected**.
2. Among correctness-preserving changes, prefer **lower median tokens** (passing runs).
3. A promotion must not regress ANY held-out task NOR any task in
   context-evolve's admitted set (shared runtime — hard gate).
4. The honest metric is the **locked task** (canonical end-to-end site), run
   once at campaign end. `wall_ms` is diagnostic only; it never gates.

## Tiers

- **Tier A (genome, no rebuild):** a CandidateConfig JSON — context/memory knobs
  plus v2 axes: `system_prompt`, `protocol`, `active_skills`, `skills_dirs`,
  `temperature`/`top_p`/`top_k`/`min_p`, `subagents`/`subagent_max_turns`/
  `subagent_max_depth`/`subagent_model`, `tool_descriptions`,
  `max_result_bytes`, `max_turns`. **Tier A′:** editing candidate runtime-skill
  FILES under `artifacts/agent-skills/<variant>/` (no rebuild).
- **Tier B (code, rebuild):** runtime code (`agent-core`, `agent-tools`,
  `agent-skills`, `agent-memory`, prompts.rs). Snapshot-binary pairing; FULL
  guard sweep mandatory.
- **Tier C (server topology, restart):** llama-server flags / model swaps.
  Every Tier-C change is a RE-BASELINING EVENT — dedicated spikes only, never a
  per-iteration variable.

## Prerequisites

- Live server (`llama-server` skill): `AGENT_E2E_URL=http://localhost:8080`,
  `AGENT_E2E_MODEL=qwen3.6-35b-a3b`. `{"passed":false,"tokens":0,"turns":0}`
  on every run ⇒ server down or URL wrong — check `docker ps` first.
- `source ~/.cargo/env && cd agent && cargo build -p agent-runtime-config --tests --bins`
- Web tasks: `docker pull node:22-bookworm-slim` once, and the task's `seed.sh`
  run once (builds `seed/node_modules` offline seeds).
- Memory-mode tasks: `EVAL_REAL_EMBEDDINGS=1 FASTEMBED_CACHE=src-tauri/.fastembed_cache`.

**Do not** use this skill for one-off harness tweaks or any change that skips
the eval gate; do not tune memory params against the stub embedder.
```

- [ ] **Step 2: Write prepare.md**

```markdown
# prepare.md — author and admit a trustworthy web task

A task is trusted only when shown, red-first, to be **harness-bound** (not
capability-bound). Same two-sided test as context-evolve: favorable pass-rate
≥ 0.8 AND realistic < 0.5 (`eval_gate admit`, N=5 each side).

## Web-task anatomy (`tasks/<id>/`)

- `task.json` — TaskSpec with `"exec_profile": "node-offline"`,
  `"seed_dir": "seed"`, `"prompt_timeout_secs": 600`.
- `seed/` — the COMPLETE workspace: configs, src stubs, and `node_modules`
  (NOT in git; regenerated by `seed.sh`). Requirements are delivered in
  PROMPTS across turns (behind noise reads), never in seeded files.
- `seed.sh` — trusted, authoring-time only: `cd seed && npm install`
  (refreshes the committed `package-lock.json` + builds `node_modules`).
  THE AGENT NEVER INSTALLS — runs are offline by construction.
- `hidden_tests/` — sealed grading, copied in post-run: `check.sh` runs
  `npx tsc --noEmit`, copies the hidden vitest spec into `src/`, runs
  `npx vitest run`, `npx vite build`, then greps `dist/` for required content.
  Grading executes IN the sandbox container (network none).
- `favorable.json` / `champion_v0.json` — the two sides. Favorable = context
  manager neutralized (context-evolve's reference values) + `max_turns` ≥ 25.
  Realistic = champion params at a pressured window (start ~8000 for web —
  outputs are fatter than the Rust tasks; shrink until red).

## Authoring hazards (all inherited, all real)

- `set_goal` pins the FIRST user prompt verbatim — NO load-bearing facts in
  prompt #1.
- Window pressure comes from HISTORY (instruction turns + noise reads), not
  workspace file size (large tool outputs offload).
- Favorable must be ≈5/5 or the signal is mud (locked-hostpolicy lesson) —
  keep the capability bar near transcription; vanilla TS, no framework, until
  a champion exists.
- Never modify a frozen task dir; diagnostic copies go to the scratchpad.

## Admission ladder

- `CapabilityBound` → strip features in order: data-fetch formatting → the
  failing-test feature → routes-transcription core. Re-run.
- `NoWeakness` → shrink `context_limit` and/or add requirement turns.
- `IllSized` → shorten noise reads / requirement count.

Run the check exactly as context-evolve's prepare.md (absolute paths; N=5 per
side; `eval_gate admit fav.jsonl real.jsonl`). On `Admitted`: freeze the task,
record both configs + numbers in program.md; realistic config becomes
champion v0 for this task.

## Locked task (end-of-campaign only)

The canonical end-to-end build (empty-ish dir → working multi-page site, all
green). Author it AFTER the training task exists; run ONCE at campaign end.
```

- [ ] **Step 3: Write train.md**

```markdown
# train.md — the per-iteration loop

One iteration = ONE mechanism-level hypothesis, tested under the gate. Stop
after K=6 consecutive non-improvements; then run the locked task once.

1. **Read `program.md`.** Never retry a logged dead end.
2. **Diagnose before designing.** CE_DEBUG-style window/trace dumps on a
   failing champion run BEFORE forming the hypothesis (both context-evolve
   2026-07-03 wins came from window dumps, not param sweeps). Remove all
   diagnostics pre-merge.
3. **One change.** Tier A: edit one genome field in `cand.json`. Tier A′: one
   skill-file variant under `artifacts/agent-skills/`. Tier B: one code change
   + rebuild + snapshot binaries per code state. Tier C: dedicated spike only.
4. **Eval paired, equal N (N=5 for web tasks — runs are minutes).** Candidate
   AND champion re-run back-to-back the same night; no mid-batch edits; a
   batch interrupted by a server restart is discarded whole.
5. **Gate:** `eval_gate gate champ.jsonl cand.jsonl`. Known artifacts: 0-pass
   champion → token-artifact Reject (read passes() directly; strictly-more-
   passes = promote); passes-increased → token Reject artifact (same rule).
6. **GUARD SWEEP — NOT OPTIONAL.** Before any promotion:
   - harness-evolve held-outs (as they accrue), AND
   - **context-evolve's admitted set** at its v4 ceilings: longhaul-manifest
     5/5 (20/20 entries), locked-portmap 10/10, drift-ledger ≥11/12,
     longhaul-codename 5/5, offload-recall 5/5, memory-recall 5/5 (REAL
     embeddings), memory-roster ≥9/10 (~5–10%/batch storage-slip noise).
   Tier-A changes provably inert to curation (e.g. sampler-only) may run a
   reduced sweep; Tier B / prompt / skills / tools changes run it ALL. When in
   doubt, full sweep.
7. **Attribute single misses by prefix identity** (llama.cpp is not
   bit-deterministic at temp 0): could the failing call's context differ from
   the champion's AT ALL? Paired same-night batches beat more N.
8. **Append to program.md** (hypothesis, change, batches, verdict) — promote
   or not. On promote: update the champion block + config; on Tier-B promote,
   merge the code per repo conventions (spec'd change, ci.sh green).
```

- [ ] **Step 4: Write program.md** (day-one seed)

```markdown
# harness-evolve — accumulated learnings + champion

Append-only research memory. The loop reads this first every iteration and
never retries a logged dead end. Campaign spec:
`docs/superpowers/specs/2026-07-03-harness-evolve-campaign-design.md`.

## Hard constraints (verified 2026-07-03)

- RTX 3090 24 GB; 60 GB RAM. Server: llama.cpp docker `llama-agent`, :8080,
  `-np 4 --kv-unified -c 196608` (see the local-llama-server memory; does NOT
  survive reboot).
- 35B-A3B IQ4_XS = 17.7 GB resident (settles ~21.6/24.6 GB). 27B Q5_K_XL =
  20 GB; 27B Q4_K_XL = 17.6 GB. **NO 27B variant co-resides with the 35B.**
  gpt-oss-120b (MoE ~5B active, 60 GB mxfp4) on disk — CPU-heavy wildcard,
  spike-tier only.
- Startup-only flags (restart = Tier-C re-baseline): -c, -np, -ngl, KV type,
  -fa, --cache-ram. Per-request (Tier-A): temp/top_p/top_k/min_p/penalties,
  max_tokens, tools.

## Phase-0 decisions (2026-07-03)

- node-offline profile: docker sandbox ENFORCED for web tasks
  (node:22-bookworm-slim, network NONE, HOME=/tmp default), node_modules
  pre-seeded at authoring time, grading in-container. Agent never installs.
- Allowlist (eval SafeApproval, node profile only): node, npx, tsc, vitest,
  vite. NOT npm install/ci/run — package.json scripts are agent-writable.
- exec_profile: None tasks byte-identical to pre-campaign semantics (no
  re-baseline of context-evolve).

## Hypothesis backlog (unlock order per spec §roadmap)

1. System prompt variants (BASE_SYSTEM_PROMPT has never been evaluated).
2. Agent-side SDLC skill via skills_dirs/active_skills — start with ONE
   verify-before-done skill.
3. Sampler sweep (temperature first; champion inherits 0.2).
4. Tool descriptions (seam live as of 2026-07-03); then missing tools
   (dev-server probe) as Tier B.
5. Sub-agent policy: when to delegate; orchestrator-as-role on the SAME 35B
   (subagent_model + role) — topology spike #1.
6. Memory axes (REAL embeddings only).
7. Tier-C spikes: serial model swap (expect run-cost fail; measure once);
   partial-offload co-residency (expected dead end — record the arithmetic).
8. Audit carry-overs: summary poisoning by transient tool errors;
   max_result_bytes realism.

## Champion (v0) — pending admission

<!-- Filled by the web-multipage admission run: config, pass-rate, median
tokens, wall_ms medians, failure shape. -->

## Admitted training tasks

<!-- web-multipage entry goes here on Admitted verdict: both configs, N=5
numbers each side, realistic window found, failure shape. -->

## Learnings (accumulated; never re-tried)

- (seed) Favorable ≈5/5 or the signal is mud — locked-hostpolicy precedent.
- (seed) `gate`'s 0-pass/passes-increased token artifacts — read passes().
- (seed) Attribute single misses by prefix identity, not batch counts.

## Iteration log

<!-- one entry per hypothesis: change | N results | gate verdict | kept? -->
```

- [ ] **Step 5: Create the artifacts placeholder**

```bash
mkdir -p .agents/skills/harness-evolve/artifacts/agent-skills
touch .agents/skills/harness-evolve/artifacts/agent-skills/.gitkeep
```

- [ ] **Step 6: Commit**

```bash
git add .agents/skills/harness-evolve/
git commit -m "docs(skill): harness-evolve campaign scaffolding — SKILL/prepare/train/program"
```

---

### Task 8: Author the `web-multipage` task

**Files:**
- Create: `.agents/skills/harness-evolve/tasks/web-multipage/task.json`
- Create: `.agents/skills/harness-evolve/tasks/web-multipage/seed.sh` (executable)
- Create: `.agents/skills/harness-evolve/tasks/web-multipage/.gitignore` (`seed/node_modules/`)
- Create: `.agents/skills/harness-evolve/tasks/web-multipage/seed/{package.json,tsconfig.json,vite.config.ts,index.html,noise.txt,src/main.ts,src/router.ts,src/stats.ts,src/format.test.ts,public/stats.json}`
- Create: `.agents/skills/harness-evolve/tasks/web-multipage/hidden_tests/{check.sh,hidden.test.ts}`
- Create: `.agents/skills/harness-evolve/tasks/web-multipage/{favorable.json,champion_v0.json}`

**Interfaces:**
- Consumes: TaskSpec fields (Task 4), node-offline driver path (Task 6).
- Produces: the frozen task Task 9 admits.

- [ ] **Step 1: seed/package.json**

```json
{
  "name": "acme-site",
  "private": true,
  "version": "0.0.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "test": "vitest run",
    "typecheck": "tsc --noEmit"
  },
  "devDependencies": {
    "typescript": "~5.6.3",
    "vite": "^5.4.11",
    "vitest": "^2.1.8"
  }
}
```

- [ ] **Step 2: seed/tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "skipLibCheck": true,
    "types": ["vitest/globals"],
    "noEmit": true
  },
  "include": ["src"]
}
```

- [ ] **Step 3: seed/vite.config.ts**

```typescript
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: { globals: true, environment: "node" },
});
```

- [ ] **Step 4: seed/index.html**

```html
<!doctype html>
<html lang="en">
  <head><meta charset="UTF-8" /><title>Acme</title></head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

- [ ] **Step 5: seed/src/router.ts** (the stub the agent completes)

```typescript
/** One page of the site. */
export interface PageSpec {
  title: string;
  body: string;
}

/**
 * Return the page for a hash route path ("/", "/pricing", ...), or null for
 * unknown routes. TODO: implement per the requirements given in this session.
 */
export function routeFor(path: string): PageSpec | null {
  void path;
  return null;
}
```

- [ ] **Step 6: seed/src/stats.ts** (the data-hook stub)

```typescript
/** Raw stats as served by the backend fixture (public/stats.json). */
export interface RawStats {
  dau: number;
  p95_ms: number;
  uptime_pct: number;
}

/** View model rendered on the /stats page. */
export interface StatsView {
  dailyActive: number;
  latencyP95: string;
  uptime: string;
}

/** TODO: map raw stats to the view per the requirements given in this session. */
export function formatStats(raw: RawStats): StatsView {
  void raw;
  return { dailyActive: 0, latencyP95: "", uptime: "" };
}
```

- [ ] **Step 7: seed/src/main.ts** (working shell — renders whatever routeFor returns, so `vite build` output carries the titles once implemented)

```typescript
import { routeFor } from "./router";
import { formatStats, type RawStats } from "./stats";

async function render(): Promise<void> {
  const app = document.querySelector<HTMLDivElement>("#app")!;
  const path = location.hash.replace(/^#/, "") || "/";
  const page = routeFor(path);
  if (!page) {
    app.innerHTML = `<h1>Page Not Found</h1>`;
    return;
  }
  let extra = "";
  if (path === "/stats") {
    const raw = (await (await fetch("/stats.json")).json()) as RawStats;
    const v = formatStats(raw);
    extra = `<ul><li>${v.dailyActive}</li><li>${v.latencyP95}</li><li>${v.uptime}</li></ul>`;
  }
  app.innerHTML = `<h1>${page.title}</h1><p>${page.body}</p>${extra}`;
}

window.addEventListener("hashchange", render);
void render();
```

- [ ] **Step 8: seed/src/format.test.ts** (the SEEDED failing test that drives the feature)

```typescript
import { describe, expect, it } from "vitest";
import { formatStats } from "./stats";

describe("formatStats", () => {
  it("maps raw stats to the view model", () => {
    const v = formatStats({ dau: 1200, p95_ms: 142, uptime_pct: 99.95 });
    expect(v.dailyActive).toBe(1200);
    expect(v.latencyP95).toBe("142 ms");
    expect(v.uptime).toBe("99.95%");
  });
});
```

- [ ] **Step 9: seed/public/stats.json**

```json
{ "dau": 1200, "p95_ms": 142, "uptime_pct": 99.95 }
```

- [ ] **Step 10: seed/noise.txt** — reuse locked-portmap's lorem block verbatim (copy the `noise.txt` contents from `.agents/skills/context-evolve/tasks/locked-portmap/task.json`'s seed entry into a plain file).

- [ ] **Step 11: hidden_tests/hidden.test.ts**

```typescript
import { describe, expect, it } from "vitest";
import { routeFor } from "./router";
import { formatStats } from "./stats";

describe("routes", () => {
  it("home", () => {
    const p = routeFor("/")!;
    expect(p.title).toBe("Acme Dashboard Home");
    expect(p.body).toContain("Welcome to Acme");
  });
  it("pricing", () => {
    const p = routeFor("/pricing")!;
    expect(p.title).toBe("Plans & Pricing");
    expect(p.body).toContain("Starter: $9/mo");
  });
  it("about", () => {
    const p = routeFor("/about")!;
    expect(p.title).toBe("About Acme");
    expect(p.body).toContain("Founded 2019");
  });
  it("stats page", () => {
    const p = routeFor("/stats")!;
    expect(p.title).toBe("Usage Statistics");
    expect(p.body).toContain("Daily Active Users");
  });
  it("unknown routes are null", () => {
    expect(routeFor("/nope")).toBeNull();
  });
});

describe("stats view", () => {
  it("maps every field", () => {
    const v = formatStats({ dau: 88, p95_ms: 5, uptime_pct: 100 });
    expect(v.dailyActive).toBe(88);
    expect(v.latencyP95).toBe("5 ms");
    expect(v.uptime).toBe("100%");
  });
});
```

- [ ] **Step 12: hidden_tests/check.sh**

```bash
#!/usr/bin/env bash
# Sealed grading — runs INSIDE the node-offline container, cwd=/workspace.
set -euo pipefail
npx tsc --noEmit
cp hidden_tests/hidden.test.ts src/hidden.test.ts
npx vitest run
npx vite build
grep -rq "Plans & Pricing" dist/assets
grep -rq "Founded 2019" dist/assets
grep -rq "Usage Statistics" dist/assets
grep -rq "Acme Dashboard Home" dist/assets
echo "ALL CHECKS PASSED"
```

- [ ] **Step 13: seed.sh** (task root, `chmod +x`)

```bash
#!/usr/bin/env bash
# TRUSTED, AUTHORING-TIME ONLY. Builds the offline seed the eval copies into
# each run's workspace. The agent never installs — runs are network-none.
set -euo pipefail
cd "$(dirname "$0")/seed"
npm install
echo "seed ready: $(du -sh node_modules | cut -f1) node_modules"
```

- [ ] **Step 14: task.json.** Requirements live ONLY in prompts 2–9 (prompt #1 is pinned by set_goal — keep it fact-free); every requirement turn sits behind a noise read:

```json
{
  "id": "web-multipage",
  "mode": "code",
  "realistic_window": 8000,
  "favorable_window": 196608,
  "memory_enabled": false,
  "exec_profile": "node-offline",
  "seed_dir": "seed",
  "prompt_timeout_secs": 600,
  "seed_files": [],
  "test_cmd": "bash hidden_tests/check.sh",
  "sessions": [
    {
      "prompts": [
        "You are working in a small Vite + TypeScript site. Over the next messages I will give you the content requirements one at a time; do NOT write any code until I say so. Acknowledge in one sentence.",
        "Read noise.txt in full with read_file. Then note this requirement (no code yet): the route '/' must have the title 'Acme Dashboard Home' and its body must include the text 'Welcome to Acme'. Acknowledge in one sentence.",
        "Read noise.txt in full with read_file. Then note this requirement (no code yet): the route '/pricing' must have the title 'Plans & Pricing' and its body must include 'Starter: $9/mo'. Acknowledge in one sentence.",
        "Read noise.txt in full with read_file. Then note this requirement (no code yet): the route '/about' must have the title 'About Acme' and its body must include 'Founded 2019'. Acknowledge in one sentence.",
        "Read noise.txt in full with read_file. Then note this requirement (no code yet): the route '/stats' must have the title 'Usage Statistics' and its body must include 'Daily Active Users'. Acknowledge in one sentence.",
        "Read noise.txt in full with read_file. Then note this requirement (no code yet): in formatStats, the raw field 'dau' maps to the view field 'dailyActive' unchanged. Acknowledge in one sentence.",
        "Read noise.txt in full with read_file. Then note this requirement (no code yet): in formatStats, the raw field 'p95_ms' maps to 'latencyP95' as the number followed by a space and 'ms', e.g. '142 ms'. Acknowledge in one sentence.",
        "Read noise.txt in full with read_file. Then note this requirement (no code yet): in formatStats, the raw field 'uptime_pct' maps to 'uptime' as the number followed by '%', e.g. '99.95%'. Unknown routes must make routeFor return null. Acknowledge in one sentence.",
        "Now implement src/router.ts (routeFor) and src/stats.ts (formatStats) to satisfy EVERY requirement I gave across the previous messages. Use write_file to save each file. Then verify your work by running 'npx vitest run' and 'npx tsc --noEmit' with execute_command, fix any failures, and finish with a one-line summary."
      ]
    }
  ]
}
```

- [ ] **Step 15: favorable.json** (context-manager-neutralized reference + web turn budget)

```json
{ "context_limit": 196608, "high_water_pct": 1.0, "keep_recent": 4294967295,
  "error_min_bytes": 18446744073709551615, "output_min_bytes": 18446744073709551615,
  "recall_budget": 4096, "memory_enabled": false, "default_k": 20,
  "relevance_threshold": 0.0, "dedup_threshold": 0.95, "forget_threshold": 0.85,
  "max_recall_chars": 65536, "recall_token_budget": 8192, "auto_recall": true,
  "max_turns": 25 }
```

- [ ] **Step 16: champion_v0.json** (context-evolve champion-v4 params at the web-pressured window + web turn budget)

```json
{ "context_limit": 8000, "high_water_pct": 0.85, "keep_recent": 2,
  "error_min_bytes": 200, "output_min_bytes": 1024, "recall_budget": 512,
  "memory_enabled": false, "default_k": 10, "relevance_threshold": 0.3,
  "dedup_threshold": 0.95, "forget_threshold": 0.85, "max_recall_chars": 4096,
  "recall_token_budget": 512, "auto_recall": true,
  "max_turns": 25 }
```

- [ ] **Step 17: Build the seed and verify it locally (trusted, host-side, one-time)**

```bash
cd .agents/skills/harness-evolve/tasks/web-multipage && bash seed.sh
cd seed && npx tsc --noEmit && npx vitest run; cd ..
```

Expected: tsc PASSES on the stubs; vitest FAILS on `format.test.ts` (the seeded red test — correct pre-implementation state). Then verify grading rejects the stub state in-container:

```bash
docker pull node:22-bookworm-slim
WS=$(mktemp -d) && cp -a seed/. "$WS/" && mkdir -p "$WS/hidden_tests" && cp hidden_tests/* "$WS/hidden_tests/"
docker run --rm --network none --read-only --tmpfs /tmp:rw,size=256m \
  --cap-drop ALL --security-opt no-new-privileges --user "$(id -u):$(id -g)" \
  -e HOME=/tmp -v "$WS":/workspace -w /workspace node:22-bookworm-slim \
  bash -c "bash hidden_tests/check.sh"; echo "exit=$?"
```

Expected: nonzero exit (vitest red). Hand-edit `$WS/src/router.ts` + `$WS/src/stats.ts` with a correct reference implementation and re-run the docker command — expected exit=0 (proves the oracle is satisfiable offline in-container). Delete `$WS`.

- [ ] **Step 18: Commit (node_modules excluded)**

```bash
git add .agents/skills/harness-evolve/tasks/web-multipage
git commit -m "feat(skill): web-multipage discriminator task — offline Vite+TS seed, sealed sandboxed grading"
```

---

### Task 9: Admission run + program.md records

Live-eval work — needs the llama server healthy and Task 8's seed built.

- [ ] **Step 1: Bring-up.** `curl -s localhost:8080/health` → `{"status":"ok"}` (relaunch per local-llama-server memory if gone); `docker images | grep node` shows `22-bookworm-slim`; `cargo build -p agent-runtime-config --tests --bins`.

- [ ] **Step 2: Favorable N=5.**

```bash
cd /home/kalen/rust-agent-runtime/agent
T=/home/kalen/rust-agent-runtime/.agents/skills/harness-evolve/tasks/web-multipage
run() { AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
  TASK_JSON=$T/task.json CONFIG_JSON="$1" HIDDEN_TESTS_DIR=$T/hidden_tests \
  cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture 2>&1 \
  | grep -E '^\{"passed"'; }
: > /tmp/wm_fav.jsonl
for i in 1 2 3 4 5; do run $T/favorable.json >> /tmp/wm_fav.jsonl; done
cat /tmp/wm_fav.jsonl
```

Expected: ≥4/5 `"passed":true` (runs are minutes each — a favorable run may reach ~5–10 min).

- [ ] **Step 3: If favorable < 4/5 — apply the prepare.md ladder** (strip formatStats formatting → strip the failing-test feature → routes-only core), editing the task BEFORE freezing it (it is not frozen until Admitted), and re-run Step 2. Record every rung attempt in program.md's iteration log.

- [ ] **Step 4: Realistic N=5.**

```bash
: > /tmp/wm_real.jsonl
for i in 1 2 3 4 5; do run $T/champion_v0.json >> /tmp/wm_real.jsonl; done
cargo run -q -p agent-runtime-config --bin eval_gate -- admit /tmp/wm_fav.jsonl /tmp/wm_real.jsonl
```

Expected: `Admitted` (realistic <0.5). If `NoWeakness`: shrink `context_limit` in champion_v0.json (8000 → 6000 → 5000 → 4000) and/or extend the requirement turns; re-run. If `IllSized`: shorten noise reads.

- [ ] **Step 5: Record.** Fill program.md's `Champion (v0)` and `Admitted training tasks` blocks: both configs (verbatim), N=5 numbers per side, median tokens (passing) + median wall_ms, realistic failure shape (which requirements dropped — read the failing runs' trajectories/manifests), the final realistic window. Note the wall-clock cost of one paired iteration for train.md's honesty.

- [ ] **Step 6: Commit**

```bash
git add .agents/skills/harness-evolve/
git commit -m "feat(skill): web-multipage admitted — champion v0 baseline recorded"
```

(If admission needed task edits, this commit includes the final task.json/configs.)

---

### Task 10: CI, memory updates, merge

- [ ] **Step 1: Full gate.**

```bash
cd /home/kalen/rust-agent-runtime && cargo fmt --manifest-path agent/Cargo.toml --all
bash scripts/ci.sh
```

Expected: green (fmt + clippy + cargo test + web typecheck/vitest). Fix anything it flags.

- [ ] **Step 2: Memory updates.** In `/home/kalen/.claude/projects/-home-kalen-rust-agent-runtime/memory/`:
  - Create `harness-evolve-campaign-state.md` (type: project): campaign purpose, spec/plan paths, phase-0 decisions (node-offline profile, offline seeds, sandboxed grading), genome v2 axes list, admission outcome + champion v0 numbers, resume actions (= program.md backlog top), the co-residency arithmetic, link `[[context-evolve-campaign-state]]` + `[[local-llama-server]]`.
  - Update `context-evolve-campaign-state.md`: one line noting the sibling campaign exists and that ITS guard sweep now also protects context-evolve ceilings.
  - Add a MEMORY.md index line for the new file.

- [ ] **Step 3: Merge (repo conventions).**

```bash
cd /home/kalen/rust-agent-runtime
git checkout main && git merge --no-ff evolve/harness-evolve-campaign \
  -m "Merge evolve/harness-evolve-campaign: campaign scaffolding + phase-0 node-offline profile + web-multipage admitted"
git branch -d evolve/harness-evolve-campaign
```

Do NOT push.

---

## Self-Review (done at plan time)

- **Spec coverage:** phase-0 (Tasks 4–6, 8–9), genome widening incl. tool-description seam + wall_ms (Tasks 1–3, 6), scaffolding (Task 7), first admitted task (Tasks 8–9), guard/inertness (Task 6 Step 9), CI/memories/merge (Task 10). Tier-C spikes, axis iterations, and the locked task are explicitly out of scope (spec: campaign work, not bring-up).
- **Known judgment points for the implementer:** exact `agent_tools::{Mode, Limits, CommandSpec}` field names (Task 6 Step 6 says verify at source); the RuntimeConfig exhaustiveness guard test name (Task 3 Step 5); vitest/vite versions if `npm install` resolves differently (pin what seed.sh produced — the lockfile is committed).
- **Type consistency:** `apply_to(&self, cfg: &mut RuntimeConfig)` used in Tasks 1/3/6; `set_description_overrides(HashMap<String,String>)` in Tasks 3/6; TaskSpec field names identical across Tasks 4/6/8.
