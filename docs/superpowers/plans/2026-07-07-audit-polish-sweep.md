# Audit Polish Sweep (Cluster 6) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the last 12 in-repo findings of the 2026-07-06 harness+SDLC audit (cluster 6: prose, skills & ledger sweep) on one branch of small one-file edits, plus the out-of-repo 9.4 MEMORY.md trim done controller-side in the same cycle.

**Architecture:** No new subsystems. Six code-touching fixes (2.2 tool steering, 2.4 nested contract recursion, 2.5 description dedup, 3.2 doc fix, 3.4 parity test, 3.5 single-sandbox plumbing) land in the `agent/` workspace with TDD; two skill-prose fixes (1.1, 9.1) and three ledger/process fixes (11.2, 11.3, 11.4) are docs-only. Finding source of truth: `docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md`; triage: `docs/superpowers/specs/2026-07-07-audit-drain-action-plan-design.md` (Cluster 6 table).

**Tech Stack:** Rust (agent/ Cargo workspace only — src-tauri untouched), markdown.

## Global Constraints

- Branch: `feature/audit-polish-sweep`, off **local** main tip, in a worktree (superpowers:using-git-worktrees). Known gotcha: EnterWorktree may branch from stale `origin/main` — after creation run `git reset --hard <local-main-tip>` and verify.
- **Before every commit:** `git rev-parse --show-toplevel` must print the worktree path (implementers have repeatedly drifted into the main checkout). Never run `git reset`/`git rebase` in the worktree after work starts.
- Conventional commits: `type(scope): summary`.
- Every code fix lands with the regression test its audit verifier entry implies; prose/ledger fixes need no new tests but must keep `python3 scripts/skills_lint.py` and the full suite green.
- Implementer verification per code task: `cargo fmt --all --check` (from `agent/`), `cargo clippy --workspace -- -D warnings`, `cargo test -p <touched crates>`.
- Both Cargo workspaces exist; **all code in this plan is under `agent/`** — run cargo from `agent/`.
- Final gate before merge: `bash scripts/ci.sh` green.
- Finding 9.4 edits `~/.claude/projects/-home-kalen-rust-agent-runtime/memory/MEMORY.md` — **outside the repo**: controller does it in the same cycle, NOT part of the branch/commits (see "Controller-side work" at the end).

---

### Task 1: execute_command steering (finding 2.2)

**Files:**
- Modify: `agent/crates/agent-tools/src/contract.rs:7-21` (CONFUSABLE_TOOLS + doc comment)
- Modify: `agent/crates/agent-tools/src/shell.rs:14-30` (add `when_not_to_call`)
- Test: existing ratchet `confusable_tools_carry_disambiguation_in_the_assembled_registry` in `agent/crates/agent-runtime-config/src/assemble.rs:860`

**Interfaces:**
- Consumes: `Tool::when_not_to_call(&self) -> Option<&str>` (default `None`, `agent-tools/src/tool.rs:13`); registry folds it after the `WHEN_NOT_TO_CALL_MARKER`.
- Produces: `execute_command` listed in `agent_tools::CONFUSABLE_TOOLS`; its folded description contains `"When NOT to call:"`.

- [ ] **Step 1: Extend the ratchet (write the failing state)**

In `agent/crates/agent-tools/src/contract.rs`, add `"execute_command"` to the array and extend the cluster doc comment:

```rust
/// Tools genuinely confusable with a sibling that MUST carry `when_not_to_call`
/// prose. A maintained ratchet — add a new confusable tool here by hand.
/// Clusters: recall/context_recall (semantic memory vs offload rehydration),
/// read_file/read_skill_file (workspace vs skill dir), write_file/edit_file
/// (create-or-overwrite vs unique-substring replace),
/// execute_command/read_file+list_directory+git_* (a shell subsumes the
/// dedicated Read-tier tools but at Write-tier friction).
/// NOTE: `recall` is runtime-injected, so it is enforced in agent-memory's own
/// test rather than the agent-runtime-config enforcement test.
pub const CONFUSABLE_TOOLS: &[&str] = &[
    "recall",
    "context_recall",
    "read_file",
    "read_skill_file",
    "write_file",
    "edit_file",
    "execute_command",
];
```

- [ ] **Step 2: Run the ratchet to verify it fails**

Run (from `agent/`): `cargo test -p agent-runtime-config confusable_tools_carry_disambiguation`
Expected: FAIL with `execute_command is missing 'When NOT to call:' in its description`.

- [ ] **Step 3: Implement `when_not_to_call` on ExecuteCommand**

In `agent/crates/agent-tools/src/shell.rs`, inside `impl Tool for ExecuteCommand`, after `description()`:

```rust
    fn when_not_to_call(&self) -> Option<&str> {
        Some(
            "Not for operations a dedicated tool does directly: use read_file (not \
             `cat`), list_directory (not `ls`), git_status/git_diff (not `git \
             status`/`git diff`) — those are Read-tier and path-policy-aware, while \
             shell commands are Write-tier and may need approval. Use execute_command \
             for real shell work: builds, tests, pipes, scripts.",
        )
    }
```

(Only `git_status` and `git_diff` exist as tools — do NOT mention `git_log`; there is no such tool.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-runtime-config confusable_tools` and `cargo test -p agent-tools`
Expected: PASS (the ratchet's "absent set" assertion still equals `{"recall"}` because execute_command IS in the assembled registry).

- [ ] **Step 5: Commit**

```bash
git add crates/agent-tools/src/contract.rs crates/agent-tools/src/shell.rs
git commit -m "fix(tools): steer execute_command away from read_file/list_directory/git_* (audit 2.2)"
```

---

### Task 2: Nested required-param contract recursion + missing schema descriptions (finding 2.4)

**Files:**
- Modify: `agent/crates/agent-tools/src/contract.rs:25-45` (`required_params_missing_description`)
- Modify: `agent/crates/agent-skills/src/tools.rs:225-232` (create_skill `files.items` descriptions)
- Modify: `agent/crates/agent-tools/src/render.rs:81-82` (columns/rows descriptions)
- Test: new unit tests in `contract.rs`; existing ratchet `every_required_param_is_described_in_the_assembled_registry` (`assemble.rs:834`)

**Interfaces:**
- Consumes: `ToolSchema { name, description, parameters: serde_json::Value }`.
- Produces: `required_params_missing_description(&ToolSchema) -> Vec<String>` now also reports nested array-item required params as `"files[].path"`-style names. Existing consumers (assemble ratchet, agent-memory contract test at `agent-memory/src/tools.rs:755`, MCP connect-time lint at `agent-mcp/src/manager.rs:151`) get nested coverage for free — the MCP consumer stays warn-only, so no behavior break.

- [ ] **Step 1: Write the failing unit tests**

Append to the `tests` module in `agent/crates/agent-tools/src/contract.rs`:

```rust
    #[test]
    fn nested_array_item_required_params_are_flagged() {
        // Audit 2.4: required params inside array `items` object schemas are
        // part of the tool contract too.
        let s = schema(json!({"type":"object",
            "properties":{
                "files":{"type":"array","description":"bundled files","items":{
                    "type":"object",
                    "properties":{
                        "path":{"type":"string"},
                        "content":{"type":"string","description":"file body"}},
                    "required":["path","content"]}}},
            "required":[]}));
        assert_eq!(
            required_params_missing_description(&s),
            vec!["files[].path".to_string()]
        );
    }

    #[test]
    fn described_nested_array_item_params_are_compliant() {
        let s = schema(json!({"type":"object",
            "properties":{
                "files":{"type":"array","description":"bundled files","items":{
                    "type":"object",
                    "properties":{
                        "path":{"type":"string","description":"where"},
                        "content":{"type":"string","description":"what"}},
                    "required":["path","content"]}}},
            "required":[]}));
        assert!(required_params_missing_description(&s).is_empty());
    }

    #[test]
    fn string_items_arrays_are_ignored() {
        // Arrays of scalars (tags, columns) have no nested contract to check.
        let s = schema(json!({"type":"object",
            "properties":{"tags":{"type":"array","description":"labels",
                "items":{"type":"string"}}},
            "required":[]}));
        assert!(required_params_missing_description(&s).is_empty());
    }
```

- [ ] **Step 2: Run tests to verify the new ones fail**

Run: `cargo test -p agent-tools contract`
Expected: `nested_array_item_required_params_are_flagged` FAILS (returns `[]`); the other two pass vacuously today — keep them, they pin the non-regression edges.

- [ ] **Step 3: Implement the recursion**

Replace `required_params_missing_description` in `contract.rs` with:

```rust
/// Names of `schema`'s required params whose `properties[name].description` is
/// missing or empty, including required params of array-`items` object schemas
/// (reported as `parent[].child`). Empty vec = compliant.
/// Scope is deliberately array-items only (audit 2.4): plain object properties
/// with their own `required` don't occur in our schemas, and recursing into
/// them would flood the warn-only MCP connect-time lint.
pub fn required_params_missing_description(schema: &ToolSchema) -> Vec<String> {
    let mut out = Vec::new();
    collect_missing(&schema.parameters, "", &mut out);
    out
}

fn collect_missing(obj: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
    let props = obj.get("properties").and_then(|v| v.as_object());
    if let Some(required) = obj.get("required").and_then(|r| r.as_array()) {
        for name in required.iter().filter_map(|v| v.as_str()) {
            let desc = props
                .and_then(|o| o.get(name))
                .and_then(|prop| prop.get("description"))
                .and_then(|d| d.as_str());
            if desc.map(|s| s.trim().is_empty()).unwrap_or(true) {
                out.push(format!("{prefix}{name}"));
            }
        }
    }
    for (name, prop) in props.into_iter().flatten() {
        if let Some(items) = prop.get("items") {
            if items.get("properties").is_some() {
                collect_missing(items, &format!("{prefix}{name}[]."), out);
            }
        }
    }
}
```

Behavior notes to preserve (the existing four unit tests pin them): required-but-absent-from-properties still flags; whitespace-only description still flags; optional undescribed top-level params still ignored.

- [ ] **Step 4: Run the workspace ratchets to find the real violators**

Run: `cargo test -p agent-tools contract && cargo test -p agent-runtime-config every_required_param`
Expected: contract unit tests PASS; the assemble ratchet now FAILS on `create_skill` with `["files[].path", "files[].content"]` (the only nested-object-items schema in the workspace).

- [ ] **Step 5: Describe create_skill's nested params and render's columns/rows**

In `agent/crates/agent-skills/src/tools.rs` (create_skill schema, ~line 225):

```rust
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string", "description": "File path relative to the skill directory (e.g. examples/basic.md)." },
                                "content": { "type": "string", "description": "Full text content of the file." }
                            },
                            "required": ["path", "content"]
                        }
```

In `agent/crates/agent-tools/src/render.rs` (~lines 81-82), match the sibling `lang`/`mime` per-kind pattern:

```rust
                    "columns": {"type": "array", "items": {"type": "string"},
                        "description": "table column headers (kind=table)"},
                    "rows": {"type": "array", "items": {"type": "array", "items": {"type": "string"}},
                        "description": "table rows, one cell string per column (kind=table)"}
```

- [ ] **Step 6: Run tests to verify everything passes**

Run: `cargo test -p agent-tools && cargo test -p agent-skills && cargo test -p agent-runtime-config && cargo test -p agent-memory && cargo test -p agent-mcp`
Expected: PASS across all five (memory + mcp consume the checker; confirm no new flags).

- [ ] **Step 7: Commit**

```bash
git add crates/agent-tools/src/contract.rs crates/agent-tools/src/render.rs crates/agent-skills/src/tools.rs
git commit -m "fix(tools): recurse required-param description ratchet into array items; describe create_skill files[] + render table params (audit 2.4)"
```

---

### Task 3: Memory arg-prose dedup + stale eval comment (findings 2.5, 10.3)

**Files:**
- Modify: `agent/crates/agent-memory/src/tools.rs:106-109,244-247,326-329` (three descriptions)
- Modify: `agent/crates/agent-runtime-config/src/eval/config.rs:184-186` (comment only)
- Test: new ratchet test in `agent-memory/src/tools.rs` `recall_contract_tests` module

**Interfaces:**
- Consumes: `Remember`/`Recall`/`Forget` structs (fields `embedder`, `store`, `cfg`, `project_key`), `StubEmbedder::d384()`, `InMemoryStore::new()`, `MemoryConfig::default()` — exactly as the existing `recall_carries_disambiguation_and_described_query` test constructs them.
- Produces: base descriptions with no `"Args:"` sentence; per-param schema descriptions remain the single source (unchanged).

- [ ] **Step 1: Write the failing ratchet test**

Append to the `recall_contract_tests` module in `agent/crates/agent-memory/src/tools.rs`:

```rust
    #[test]
    fn base_descriptions_do_not_duplicate_arg_lists() {
        // Audit 2.5: per-param schema descriptions are the single source of
        // truth for argument prose; an "Args:" sentence in the base
        // description is a second, drift-prone copy of the same contract.
        let embedder: std::sync::Arc<dyn crate::embedder::Embedder> =
            std::sync::Arc::new(StubEmbedder::d384());
        let store: std::sync::Arc<dyn crate::store::MemoryStore> =
            std::sync::Arc::new(InMemoryStore::new());
        let cfg = std::sync::Arc::new(MemoryConfig::default());
        let descriptions = [
            Remember {
                embedder: embedder.clone(),
                store: store.clone(),
                cfg: cfg.clone(),
                project_key: "A".into(),
            }
            .description()
            .to_string(),
            Recall {
                embedder: embedder.clone(),
                store: store.clone(),
                cfg: cfg.clone(),
                project_key: "A".into(),
            }
            .description()
            .to_string(),
            Forget {
                embedder,
                store,
                cfg,
                project_key: "A".into(),
            }
            .description()
            .to_string(),
        ];
        for d in &descriptions {
            assert!(!d.contains("Args:"), "duplicated arg list in: {d}");
        }
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p agent-memory base_descriptions_do_not_duplicate`
Expected: FAIL on remember's description (all three contain `Args:`).

- [ ] **Step 3: Drop the Args sentences**

The three `description()` bodies become:

```rust
    // Remember (~line 106)
    fn description(&self) -> &str {
        "Store a fact in long-term memory for recall in future sessions."
    }
```

```rust
    // Recall (~line 244)
    fn description(&self) -> &str {
        "Search long-term memory for facts relevant to a query. Returns the most similar \
         stored memories from this project and the global tier."
    }
```

```rust
    // Forget (~line 326)
    fn description(&self) -> &str {
        "Remove a single memory, selected by exact id or by a query matched against \
         stored text. Never mass-deletes."
    }
```

(Forget's "best match only if confidently similar" detail already lives in its `query` param description — don't re-duplicate it.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-memory`
Expected: PASS, including the pre-existing S9 contract test (params still described) and the new ratchet.

- [ ] **Step 5: Reword the stale eval comment (10.3)**

In `agent/crates/agent-runtime-config/src/eval/config.rs`, `favorable_disables_curation` test (~line 184), replace:

```rust
        // Ingestion cap is neutralized for the whole eval harness (not part of
        // the candidate genome) — otherwise a size-based cap would apply.
```

with:

```rust
        // Ingestion cap is neutralized by DEFAULT (None = cap off); candidates
        // may opt into a realistic cap via max_result_bytes (harness-evolve
        // genome axis 8) — favorable stays cap-off.
```

- [ ] **Step 6: Run tests, then commit**

Run: `cargo test -p agent-runtime-config favorable_disables_curation && cargo test -p agent-memory`
Expected: PASS (comment-only change; assertions untouched).

```bash
git add crates/agent-memory/src/tools.rs crates/agent-runtime-config/src/eval/config.rs
git commit -m "fix(memory,eval): drop duplicated Args prose from memory tool descriptions; fix stale ingestion-cap comment (audit 2.5, 10.3)"
```

---

### Task 4: build_sandbox doc fix + CLI sandbox-defaults parity (findings 3.2, 3.4)

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/lib.rs:303` (doc comment only)
- Modify: `agent/crates/agent-cli/src/main.rs:366-378` (rework `sandbox_defaults` test)

**Interfaces:**
- Consumes: `RuntimeConfig::from_launch(backend, base_url, model, protocol, context_limit)` — the canonical defaults source (fills every `sandbox_*` field from the `default_sandbox_*` fns).
- Produces: nothing new — a doc line and a test.

- [ ] **Step 1: Fix the doc comment (3.2)**

In `agent/crates/agent-runtime-config/src/lib.rs` (~line 303), replace:

```rust
/// - anything else (e.g. `"auto"`) → `DockerSandbox` in `Mode::Auto` (degrades to host).
```

with:

```rust
/// - anything else (e.g. `"auto"`) → `DockerSandbox` in `Mode::Auto` (fail-closed:
///   refuses exec while Docker is unavailable, re-probes on each launch).
```

This matches live behavior (`strategy.rs` auto arm returns `SandboxError::Unavailable`; the old degrade-to-host arm was removed 2026-07-01).

- [ ] **Step 2: Rework `sandbox_defaults` into a parity test (3.4)**

In `agent/crates/agent-cli/src/main.rs` tests, replace the body of `sandbox_defaults` (which re-hardcodes literals) with comparisons against the runtime-config source of truth, and rename it:

```rust
    #[test]
    fn cli_sandbox_defaults_match_runtime_config_defaults() {
        // Audit 3.4: clap default_value attrs are hand-written mirrors of
        // runtime-config's default_sandbox_* fns (the documented clap-shadowing
        // gotcha class). from_launch is the canonical source; drift here means
        // a bumped server-side default silently leaves the CLI behind.
        let cli = Cli::parse_from(["agent-cli"]);
        let base = RuntimeConfig::from_launch(
            "openai".into(),
            "http://x".into(),
            "m".into(),
            "native".into(),
            8192,
        );
        assert_eq!(cli.sandbox_mode, base.sandbox_mode);
        assert_eq!(cli.sandbox_image, base.sandbox_image);
        assert_eq!(cli.sandbox_network, base.sandbox_network);
        assert_eq!(cli.sandbox_memory, base.sandbox_memory);
        assert_eq!(cli.sandbox_cpus, base.sandbox_cpus);
        assert_eq!(cli.sandbox_pids, base.sandbox_pids);
        assert_eq!(cli.sandbox_fsize, base.sandbox_fsize);
        assert_eq!(cli.sandbox_tmp_size, base.sandbox_tmp_size);
        assert_eq!(cli.sandbox_extra_rw, base.sandbox_extra_rw);
        assert_eq!(cli.sandbox_extra_ro, base.sandbox_extra_ro);
    }
```

If any field name differs on `RuntimeConfig` (e.g. `sandbox_fsize` typing), match the struct — the assertions must compare CLI field to the *same-named* RuntimeConfig field, no literals.

- [ ] **Step 3: Verify the test discriminates**

Temporarily change one clap default (e.g. `sandbox_memory` to `"3g"`), run `cargo test -p agent-cli cli_sandbox_defaults`, confirm FAIL; revert, confirm PASS. (This is the RED step — the parity test must be able to catch drift.)

- [ ] **Step 4: Run tests and commit**

Run: `cargo test -p agent-cli && cargo test -p agent-runtime-config`
Expected: PASS.

```bash
git add crates/agent-runtime-config/src/lib.rs crates/agent-cli/src/main.rs
git commit -m "fix(config,cli): correct build_sandbox auto-mode doc (fail-closed) + pin CLI sandbox defaults to runtime-config (audit 3.2, 3.4)"
```

---

### Task 5: One sandbox instance per frontend (finding 3.5)

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (LoopParts + `loop_config_from` + `assemble_loop` + test helpers/call sites at 452, 491, 500, 633, 651, 659 and the `parts()` literal)
- Modify: `agent/crates/agent-cli/src/main.rs:239,274` (reuse the one instance)
- Modify: `agent/crates/agent-server/src/runtime.rs:341-390` (`build_loop` passes a sandbox)
- Modify: `agent/crates/agent-runtime-config/tests/e2e_auto_retrieval.rs:115,180,242`, `tests/soak_live.rs:229`, `tests/eval_context.rs:281`, `tests/e2e_robustness.rs:74` (add the new field)
- Test: new pin test in `assemble.rs`

**Interfaces:**
- Consumes: `build_sandbox(&RuntimeConfig) -> Arc<dyn SandboxStrategy>` (unchanged), `agent_tools::{SandboxStrategy, HostExecutor}`, `AgentLoop::sandbox_descriptor()` (`agent-core/src/loop_.rs:194`).
- Produces:
  - `LoopParts.sandbox: Arc<dyn agent_tools::SandboxStrategy>` (new required field),
  - `loop_config_from(cfg: &RuntimeConfig, workspace: PathBuf, stream_idle_timeout: Duration, sandbox: Arc<dyn agent_tools::SandboxStrategy>) -> LoopConfig` (new 4th param; no longer calls `build_sandbox` itself).

- [ ] **Step 1: Write the failing pin test**

Append to the tests module in `agent/crates/agent-runtime-config/src/assemble.rs`:

```rust
    #[test]
    fn assemble_uses_the_injected_sandbox_not_a_fresh_build() {
        // Audit 3.5: one isolation boundary, one authoritative instance. If
        // assemble rebuilt from cfg, enforce-mode would yield mechanism
        // "docker"; seeing "host" proves the caller's Arc is used.
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg();
        c.sandbox_mode = "enforce".into();
        let mut p = parts(dir.path().to_path_buf(), vec![]);
        p.sandbox = Arc::new(agent_tools::HostExecutor);
        let built = assemble_loop(&c, p);
        assert_eq!(built.loop_.sandbox_descriptor().mechanism, "host");
    }
```

RED here is a compile error (`LoopParts` has no `sandbox` field yet) — accepted RED form per prior cluster precedent. Record the compile failure, then proceed.

- [ ] **Step 2: Add the field and thread the Arc**

In `assemble.rs`:

1. `LoopParts` gains (after `compact_flag`):

```rust
    /// The frontend's single sandbox instance — one probe + one availability
    /// cache per frontend (audit 3.5). Callers that also connect MCP must
    /// pass the SAME Arc they gave `connect_mcp`.
    pub sandbox: Arc<dyn agent_tools::SandboxStrategy>,
```

2. `loop_config_from` signature + body:

```rust
pub fn loop_config_from(
    cfg: &RuntimeConfig,
    workspace: PathBuf,
    stream_idle_timeout: Duration,
    sandbox: Arc<dyn agent_tools::SandboxStrategy>,
) -> LoopConfig {
```

and in the struct literal replace `sandbox: build_sandbox(cfg),` with `sandbox,`.

3. In `assemble_loop` (~line 246):

```rust
    let loop_config = loop_config_from(
        cfg,
        parts.workspace.clone(),
        parts.stream_idle_timeout,
        parts.sandbox.clone(),
    );
```

4. Remove `build_sandbox` from assemble.rs's `use crate::{...}` import if now unused **in production code**; test call sites below still use it via `crate::build_sandbox` — keep the import if tests need it.

- [ ] **Step 3: Update every construction/call site**

- `agent-cli/src/main.rs`: `let sandbox = build_sandbox(&rt);` (line 239) already exists — add `sandbox: sandbox.clone(),` to the `LoopParts` literal (~line 274). This is the actual dedup: the same Arc now serves `connect_mcp` and the loop.
- `agent-server/src/runtime.rs` `build_loop`: add `sandbox: agent_runtime_config::build_sandbox(cfg),` to its `LoopParts` literal (~line 371) — one instance per loop build, same count as today (add `build_sandbox` to the `use agent_runtime_config::{...}` list at line 11 and drop the qualified path).
- `assemble.rs` tests: in the `parts()` helper add `sandbox: crate::build_sandbox(&cfg()),`; at the five direct `loop_config_from(...)` call sites (lines ~491, 500, 633, 651, 659) append a 4th arg `crate::build_sandbox(&<the cfg variable in scope>)` (`&c`, `&cfg2`, `&cfg3` as appropriate).
- Integration tests (`e2e_auto_retrieval.rs` ×3, `soak_live.rs`, `eval_context.rs`, `e2e_robustness.rs`): in each `LoopParts` literal add `sandbox: agent_runtime_config::build_sandbox(<the cfg in scope>),` — preserving today's behavior exactly (these files construct a `RuntimeConfig` nearby; pass a reference to it).

- [ ] **Step 4: Run tests to verify they pass**

Run (from `agent/`): `cargo test --workspace`
Expected: PASS including `assemble_uses_the_injected_sandbox_not_a_fresh_build`. (Live-only ignored tests stay ignored.)

- [ ] **Step 5: Commit**

```bash
git add crates/agent-runtime-config/src/assemble.rs crates/agent-cli/src/main.rs crates/agent-server/src/runtime.rs crates/agent-runtime-config/tests/
git commit -m "fix(config,cli,server): one sandbox instance per frontend — LoopParts carries the Arc (audit 3.5)"
```

---

### Task 6: Skill prose — agent-sdlc boundary + auto-drive-tauri stale test pointer (findings 1.1, 9.1)

**Files:**
- Modify: `.agents/skills/agent-sdlc/SKILL.md` (frontmatter + new Do-not block)
- Modify: `.agents/skills/auto-drive-tauri/SKILL.md:56-58` (one paragraph)

**Interfaces:** none (markdown). `.claude/skills/` symlinks pick the edits up automatically — do NOT touch the symlinks.

- [ ] **Step 1: Narrow agent-sdlc's frontmatter**

Replace the `description:` block (lines 3-11) with:

```yaml
description: >-
  Use for any question about how to build, evaluate, deploy, or operate AI
  agents — evals, tool design, context engineering, multi-agent decomposition,
  memory, human-in-the-loop gates, monitoring — or how agents should run a
  software lifecycle (spec-first workflows, verification-first coding). Routes
  to the source-backed knowledge bundle at docs/okf/agent-sdlc/ (36 first-party
  sources, 23 cited concepts): the EVIDENCE layer behind agent-architecture
  decisions, specs, and reviews. For hands-on design/build/audit work on THIS
  repo's harness, use harness-engineering (the playbook layer) and load this
  bundle alongside it for citations. Also use when extending or fixing the
  bundle (see authoring.md).
```

(The change: "harness design" is dropped from the capability list — that phrase is harness-engineering's verbatim trigger — and the boundary is stated in the routing signal itself, not just the post-load body.)

- [ ] **Step 2: Add the house-style Do-not block**

Insert after the intro paragraph (after line 22, before the `"Agent SDLC" means two things` paragraph):

```markdown
**Do not** use this skill to *do* harness design/build/audit work in this
repo — that is `harness-engineering`'s playbooks; this bundle is the evidence
layer behind them. **Do not** edit bundle files without reading
[authoring.md](authoring.md) and re-running `python3 scripts/okf_check.py
docs/okf/agent-sdlc` — frontmatter, citation, and link rules are
machine-checked.
```

- [ ] **Step 3: Replace auto-drive-tauri's dead bridge.rs paragraph**

In `.agents/skills/auto-drive-tauri/SKILL.md`, replace lines 56-58:

```markdown
`src-tauri/tests/bridge.rs` (`bridge_serves_local_runtime`) is an L0/L1 hybrid: it
exercises the full bridge→serve wiring with a **closed** model port, so it proves
the plumbing without needing :8080. Copy its pattern for new protocol tests.
```

with:

```markdown
There is no offline bridge-wiring hybrid test anymore (`src-tauri/tests/bridge.rs`
was deleted in `474b7af`). For new protocol tests copy the pattern of
`src-tauri/tests/smoke_context_explorer.rs` (L1: drives the in-process bridge +
Session exactly like the desktop app's Tauri commands and asserts on the event
stream; needs :8080), using `src-tauri/tests/llama_health.rs` as the fast
no-model gate (wiremock-backed, no server required).
```

- [ ] **Step 4: Verify**

Run: `python3 scripts/skills_lint.py` → OK (10 skills). `grep -c "Do not" .agents/skills/agent-sdlc/SKILL.md` → ≥1. `grep -rn "bridge.rs" .agents/skills/auto-drive-tauri/SKILL.md` → only the "was deleted" mention. Confirm `python3 scripts/okf_check.py docs/okf/agent-sdlc` still OK (no bundle files touched; ci.sh runs it anyway).

- [ ] **Step 5: Commit**

```bash
git add .agents/skills/agent-sdlc/SKILL.md .agents/skills/auto-drive-tauri/SKILL.md
git commit -m "docs(skills): agent-sdlc Do-not block + narrowed trigger; auto-drive-tauri points at live tests (audit 1.1, 9.1)"
```

---

### Task 7: Docs-on-main exception in AGENTS.md (finding 11.4) — branch part only

> **Scope note (pre-flight discovery):** `.superpowers/` is git-ignored
> (`.gitignore:20`) — the sdd ledgers exist only in the main checkout's
> filesystem and cannot be committed or edited from the worktree. Findings
> 11.2 and 11.3 (ledger close-out + archive renames) are therefore
> **controller-side filesystem edits in the main checkout**, done in the same
> cycle — see "Post-plan process" below. This task's branch commit covers
> 11.4 only.

**Files:**
- Modify: `AGENTS.md` § How we work

**Interfaces:** none (markdown).

- [ ] **Step 1: Record the docs-on-main exception (11.4)**

In root `AGENTS.md`, at the end of the `## How we work` section (after the
adversarial-review subsection), append:

```markdown
### Docs-only exception

Docs/ledger-only campaigns (audits, triage records, campaign ledgers, memory
bookkeeping) may commit directly to `main` without a feature branch.
Compensating control: `main` is never pushed automatically, and the
whole-campaign review must pass before any push — review findings land as fix
waves on main, not silent history edits. Anything touching code, tests, or CI
still branches.
```

- [ ] **Step 2: Verify and commit**

`python3 scripts/skills_lint.py` still OK (AGENTS.md is not a skill — sanity only); the section renders under `## How we work`.

```bash
git add AGENTS.md
git commit -m "docs(process): record the docs-on-main exception + compensating control (audit 11.4)"
```

---

### Controller-side ledger work (findings 11.2, 11.3 — main checkout, NOT committed)

Done by the controller in the main checkout (`/home/kalen/rust-agent-runtime`), same cycle as the branch; the files are git-ignored scratch, so there is nothing to commit and no worktree involvement.

**11.2 — Append the MERGED close-out:**

In `.superpowers/sdd/progress.md`, directly after the line
`- BRANCH READY TO MERGE @ 370b64a. All 7 tasks + 3 fixes complete, all gates green.`
append:

```markdown
- MERGED to main @ dfec8b7 (--no-ff; branch feature/auto-dev-server-canvas deleted). AUTO-DEV-SERVER-CANVAS COMPLETE. Accepted residuals above stand (async in-flight guard, SIGTERM→SIGKILL grace — since closed by design-tab cluster 8.6, dev_server_status wiring). Close-out appended 2026-07-07 per audit 11.2.
```

(Verify the 8.6 clause before writing it: `git log --oneline 686d889 -1` merged stop()-grace work — if unsure whether it covers the dev-server manager's grace specifically, drop the "— since closed…" clause and keep the plain residual list.)

**11.3 — Stamp and rename the five completed ledgers:**

For each, first verify the claimed integration with git, then append the stamp as the last line, then plain `mv <name>.md <name>.archive.md` (files are untracked — no `git mv`):

| Ledger | Verify | Stamp line to append |
|---|---|---|
| `progress-parallel-dispatch.md` | `git merge-base --is-ancestor 7329bd1 main` | `- ARCHIVED 2026-07-07 (audit 11.3): merged to main — branch commits (96ec134, 171f573, 7329bd1) are ancestors of main; parallel tool-call dispatch live in agent-core (569a3cf e2e). Ledger closed.` |
| `progress-sandbox-degraded.md` | `git merge-base --is-ancestor be67413 main` (also try `9cf68e7`) | `- ARCHIVED 2026-07-07 (audit 11.3): merged to main (branch tip be67413 is an ancestor of main; sandbox degraded-mode work shipped 2026-07-01). Ledger closed.` |
| `progress-harness-engineering-skill.md` | `git log --oneline -1 6a8449e` | `- ARCHIVED 2026-07-07 (audit 11.3): MERGED to main @ 6a8449e (Merge branch 'docs/harness-engineering-skill'). Ledger closed.` |
| `progress-context-explorer-feature.md` | text already says merged | `- ARCHIVED 2026-07-07 (audit 11.3): merge already recorded above (main fast-forwarded, 22 commits); rename-only close-out.` |
| `progress-context-explorer-backlog.md` | `git merge-base --is-ancestor e37117f main` | `- ARCHIVED 2026-07-07 (audit 11.3): merged to main (tip e37117f is an ancestor of main). Ledger closed.` |

If any verification fails, STOP and report — do not stamp a merge that git doesn't confirm.

Verify at the end: `ls .superpowers/sdd/ | grep -v archive | grep progress-` → empty (only `progress.md` remains unarchived).

---

### Task 8: Full gate

- [ ] **Step 1:** From the worktree root: `bash scripts/ci.sh`
Expected: all legs green (okf check, skills lint, fmt, clippy, cargo test agent/, conditional src-tauri leg, web typecheck/vitest).
- [ ] **Step 2:** If any leg fails, fix in place (small fixes amend into a `fix:` commit), re-run to green. Report the final output.

---

## Post-plan process (controller, not implementer tasks)

1. **Whole-branch review** (fable) over `main..feature/audit-polish-sweep`, then `--no-ff` merge as `Merge feature/audit-polish-sweep: prose/skills/ledger sweep (audit-drain cluster 6)`. NOT pushed.
2. **Finding 9.4 (out-of-repo, same cycle):** trim the four longest `MEMORY.md` index lines (context-evolve campaign state 936ch, harness-deep-audit 909ch, harness-product-decisions 827ch, harness-evolve campaign state 601ch) to one-sentence pointers. Before trimming each line, verify every fact in it already exists in its topic file — move anything missing into the file first. KEEP one-clause guardrails in the index ("6 DECLINED-BY-OWNER — don't re-propose", resume pointers, ceiling pairs).
3. **Bookkeeping:** cluster-6 ledger section in `.superpowers/sdd/progress.md` (opened at start, MERGED stamp at close); dated re-stamps for 1.1, 2.2, 2.4, 2.5, 3.2, 3.4, 3.5, 9.1, 9.4, 10.3, 11.2, 11.3, 11.4 in `.agents/skills/harness-engineering/audit.md`. This closes the 2026-07-06 audit drain (6/6 clusters).

## Known merge caution

None remaining: the sdd ledgers are git-ignored main-checkout scratch (pre-flight discovery), so the branch never touches them — 11.2/11.3 are controller-side filesystem edits with no merge surface.
