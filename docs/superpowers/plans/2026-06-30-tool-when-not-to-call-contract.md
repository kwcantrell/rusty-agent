# Tool "When NOT to Call" Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give tools a structural, enforced "when NOT to call" contract folded into the model-facing schema, and backfill descriptions on every required parameter.

**Architecture:** Add a defaulted `Tool::when_not_to_call()`; `ToolRegistry::schemas()` folds it into each `schema.description` (the only text the model reads). Six confusable tools override it; eight tools gain required-param descriptions. A shared const + pure helper in agent-tools back two enforcement tests (one in agent-runtime-config over the assembled registry, one in agent-memory for the runtime-injected `recall`).

**Tech Stack:** Rust (Cargo workspace `agent/`), serde_json.

## Global Constraints

- The exclusion prose reaches the model ONLY via `ToolRegistry::schemas()` folding it into `schema.description` with the exact marker `When NOT to call:`. The raw `schema()`/`description()` are never mutated.
- `when_not_to_call()` default is `None`; only the six confusable tools override it: `recall`, `context_recall`, `read_file`, `read_skill_file`, `write_file`, `edit_file`.
- `CONFUSABLE_TOOLS` is a maintained const ratchet in agent-tools; do not auto-detect.
- Every REQUIRED param of every statically-assembled tool must have a non-empty `description`. Optional params are left as-is.
- No execution/behavior change — description/schema surface + tests only.
- Run cargo from `agent/` (`source ~/.cargo/env` first if needed). Conventional commits. Branch is already `fix/tool-when-not-to-call-contract`.

---

### Task 1: agent-tools — mechanism, shared contract, and agent-tools tool content

**Files:**
- Modify: `agent/crates/agent-tools/src/lib.rs` (add `mod contract; pub use contract::*;`)
- Create: `agent/crates/agent-tools/src/contract.rs`
- Modify: `agent/crates/agent-tools/src/tool.rs` (add defaulted trait method)
- Modify: `agent/crates/agent-tools/src/registry.rs` (fold in `schemas()` + unit test)
- Modify: `agent/crates/agent-tools/src/fs/read.rs` (read_file: `when_not_to_call` + `path` desc; list_directory: `path` desc)
- Modify: `agent/crates/agent-tools/src/fs/write.rs` (write_file/edit_file: `when_not_to_call` + param descs)
- Modify: `agent/crates/agent-tools/src/shell.rs` (`command` desc)
- Modify: `agent/crates/agent-tools/src/git.rs` (`message` desc)
- Modify: `agent/crates/agent-tools/src/render.rs` (`kind` desc)

**Interfaces:**
- Produces: `Tool::when_not_to_call(&self) -> Option<&str>` (default `None`)
- Produces: `agent_tools::WHEN_NOT_TO_CALL_MARKER: &str = "When NOT to call:"`
- Produces: `agent_tools::CONFUSABLE_TOOLS: &[&str]`
- Produces: `agent_tools::required_params_missing_description(&ToolSchema) -> Vec<String>`
- Produces: `ToolRegistry::schemas()` now folds `when_not_to_call` into each description.

- [ ] **Step 1: Add the defaulted trait method**

In `agent/crates/agent-tools/src/tool.rs`, add to the `Tool` trait, after `fn schema(&self) -> ToolSchema;` (line 8):

```rust
    /// Guidance on when the model should NOT call this tool (and which sibling to
    /// prefer). `None` for tools whose name/purpose already disambiguate. The
    /// registry folds this into the model-facing schema description; it is not a
    /// separate wire field.
    fn when_not_to_call(&self) -> Option<&str> { None }
```

- [ ] **Step 2: Create the contract module + its unit tests**

Create `agent/crates/agent-tools/src/contract.rs`:

```rust
use crate::ToolSchema;

/// Marker `ToolRegistry::schemas()` prepends before folded exclusion prose; also
/// the string the enforcement tests grep for.
pub const WHEN_NOT_TO_CALL_MARKER: &str = "When NOT to call:";

/// Tools genuinely confusable with a sibling that MUST carry `when_not_to_call`
/// prose. A maintained ratchet — add a new confusable tool here by hand.
/// Clusters: recall/context_recall (semantic memory vs offload rehydration),
/// read_file/read_skill_file (workspace vs skill dir), write_file/edit_file
/// (create-or-overwrite vs unique-substring replace).
/// NOTE: `recall` is runtime-injected, so it is enforced in agent-memory's own
/// test rather than the agent-runtime-config enforcement test.
pub const CONFUSABLE_TOOLS: &[&str] = &[
    "recall", "context_recall", "read_file", "read_skill_file", "write_file", "edit_file",
];

/// Names of `schema`'s required params whose `properties[name].description` is
/// missing or empty. Empty vec = compliant.
pub fn required_params_missing_description(schema: &ToolSchema) -> Vec<String> {
    let params = &schema.parameters;
    let required = params.get("required").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let props = params.get("properties").and_then(|v| v.as_object());
    required
        .iter()
        .filter_map(|r| r.as_str())
        .filter(|name| {
            let desc = props
                .and_then(|o| o.get(*name))
                .and_then(|prop| prop.get("description"))
                .and_then(|d| d.as_str());
            desc.map(|s| s.trim().is_empty()).unwrap_or(true)
        })
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(parameters: serde_json::Value) -> ToolSchema {
        ToolSchema { name: "t".into(), description: "d".into(), parameters }
    }

    #[test]
    fn flags_required_param_without_description() {
        let s = schema(json!({"type":"object",
            "properties":{"path":{"type":"string"}}, "required":["path"]}));
        assert_eq!(required_params_missing_description(&s), vec!["path".to_string()]);
    }

    #[test]
    fn empty_description_counts_as_missing() {
        let s = schema(json!({"type":"object",
            "properties":{"path":{"type":"string","description":"  "}}, "required":["path"]}));
        assert_eq!(required_params_missing_description(&s), vec!["path".to_string()]);
    }

    #[test]
    fn described_required_param_is_compliant() {
        let s = schema(json!({"type":"object",
            "properties":{"path":{"type":"string","description":"the path"}}, "required":["path"]}));
        assert!(required_params_missing_description(&s).is_empty());
    }

    #[test]
    fn optional_undescribed_param_is_ignored() {
        let s = schema(json!({"type":"object",
            "properties":{"path":{"type":"string","description":"p"},"k":{"type":"integer"}},
            "required":["path"]}));
        assert!(required_params_missing_description(&s).is_empty());
    }
}
```

Add to `agent/crates/agent-tools/src/lib.rs` after `mod registry;` (line 4):

```rust
mod contract;
```
and after `pub use registry::*;` (line 12):

```rust
pub use contract::*;
```

- [ ] **Step 3: Run the contract tests**

Run: `cargo test -p agent-tools contract 2>&1 | tail -12`
Expected: PASS (4 tests).

- [ ] **Step 4: Write the failing registry-fold test**

In `agent/crates/agent-tools/src/registry.rs`, add to the `#[cfg(test)] mod tests` block (it already has `Echo`, which does not override `when_not_to_call`):

```rust
    struct Confusable;
    #[async_trait]
    impl Tool for Confusable {
        fn name(&self) -> &str { "confusable" }
        fn description(&self) -> &str { "does a thing" }
        fn when_not_to_call(&self) -> Option<&str> { Some("use echo instead for X") }
        fn schema(&self) -> ToolSchema {
            ToolSchema { name: "confusable".into(), description: "does a thing".into(),
                         parameters: json!({"type":"object"}) }
        }
        fn intent(&self, _args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
            Ok(ToolIntent { tool: "confusable".into(), access: Access::Read, paths: vec![],
                            command: None, summary: "c".into() })
        }
        async fn execute(&self, _args: serde_json::Value, _ctx: &ToolCtx)
            -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput { content: "ok".into(), display: None })
        }
    }

    #[test]
    fn schemas_fold_when_not_to_call_into_description() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(Echo));         // no override -> untouched
        r.register(Arc::new(Confusable));   // override -> folded
        let schemas = r.schemas();
        let echo = schemas.iter().find(|s| s.name == "echo").unwrap();
        let conf = schemas.iter().find(|s| s.name == "confusable").unwrap();
        assert_eq!(echo.description, "echoes", "None tools keep their description verbatim");
        assert!(conf.description.contains(WHEN_NOT_TO_CALL_MARKER), "marker present: {}", conf.description);
        assert!(conf.description.contains("use echo instead for X"), "prose present: {}", conf.description);
        assert!(conf.description.starts_with("does a thing"), "original description preserved");
    }
```

- [ ] **Step 5: Run the fold test to verify it fails**

Run: `cargo test -p agent-tools schemas_fold 2>&1 | tail -15`
Expected: FAIL — `conf.description` does not yet contain the marker (schemas() has not been changed).

- [ ] **Step 6: Fold `when_not_to_call` into `schemas()`**

In `agent/crates/agent-tools/src/registry.rs`, replace the `schemas` method:

```rust
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }
```

with:

```rust
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| {
            let mut s = t.schema();
            if let Some(excl) = t.when_not_to_call() {
                s.description = format!("{}\n\n{} {}", s.description, WHEN_NOT_TO_CALL_MARKER, excl);
            }
            s
        }).collect()
    }
```

- [ ] **Step 7: Run the fold test to verify it passes**

Run: `cargo test -p agent-tools schemas_fold 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 8: Add the agent-tools confusable prose (read_file, write_file, edit_file)**

In `agent/crates/agent-tools/src/fs/read.rs`, inside `impl Tool for ReadFile`, after `fn description` (line 16), add:

```rust
    fn when_not_to_call(&self) -> Option<&str> {
        Some("Not for files bundled inside a loaded skill's directory — use \
              read_skill_file for those. Use read_file for workspace paths.")
    }
```

In `agent/crates/agent-tools/src/fs/write.rs`, inside `impl Tool for WriteFile`, after `fn description` (line 20), add:

```rust
    fn when_not_to_call(&self) -> Option<&str> {
        Some("Not for a small change to an existing file — use edit_file to replace \
              a specific substring. Use write_file to create a new file or fully \
              overwrite one.")
    }
```

In `agent/crates/agent-tools/src/fs/write.rs`, inside `impl Tool for EditFile`, after `fn description` (line 54), add:

```rust
    fn when_not_to_call(&self) -> Option<&str> {
        Some("Not for creating a new file or rewriting a whole file — use write_file. \
              Use edit_file to replace one unique existing substring.")
    }
```

- [ ] **Step 9: Backfill required-param descriptions (agent-tools tools)**

Edit each schema's JSON to add a `"description"` to the listed required properties. Exact replacements:

`agent/crates/agent-tools/src/fs/read.rs` — ReadFile schema (line 19), replace:
```rust
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}) }
```
with:
```rust
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"Workspace-relative path of the file to read."}},
                "required":["path"]}) }
```

`agent/crates/agent-tools/src/fs/read.rs` — ListDirectory schema (line 44), replace:
```rust
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}) }
```
with:
```rust
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"Workspace-relative path of the directory to list."}},
                "required":["path"]}) }
```

`agent/crates/agent-tools/src/fs/write.rs` — WriteFile schema (lines 23-25), replace the `parameters:` value with:
```rust
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"Workspace-relative path of the file to create or overwrite."},
                "content":{"type":"string","description":"The full contents to write to the file."}},
                "required":["path","content"]}) }
```

`agent/crates/agent-tools/src/fs/write.rs` — EditFile schema (lines 57-59), replace the `parameters:` value with:
```rust
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"Workspace-relative path of the file to edit."},
                "old":{"type":"string","description":"The exact existing substring to replace; must occur exactly once in the file."},
                "new":{"type":"string","description":"The replacement text."}},
                "required":["path","old","new"]}) }
```

`agent/crates/agent-tools/src/shell.rs` — ExecuteCommand schema, replace the `parameters:` value with:
```rust
            parameters: json!({"type":"object","properties":{
                "command":{"type":"string","description":"The shell command line to execute."}},
                "required":["command"]}) }
```

`agent/crates/agent-tools/src/git.rs` — GitCommit schema, replace the `parameters:` value with:
```rust
            parameters: json!({"type":"object","properties":{
                "message":{"type":"string","description":"The commit message."}},
                "required":["message"]}) }
```

`agent/crates/agent-tools/src/render.rs` — RenderArtifact schema, replace the `kind` property (lines 30-31):
```rust
                    "kind": {"type": "string",
                        "enum": ["markdown","code","html","mermaid","table","image"]},
```
with:
```rust
                    "kind": {"type": "string",
                        "enum": ["markdown","code","html","mermaid","table","image"],
                        "description": "Which artifact kind to render; one of the allowed enum values."},
```

- [ ] **Step 10: Add a compliance unit test for the agent-tools tools**

In `agent/crates/agent-tools/src/registry.rs` `#[cfg(test)] mod tests`, add:

```rust
    #[test]
    fn agent_tools_confusable_prose_and_required_descs() {
        use crate::fs::{ReadFile, WriteFile, EditFile, ListDirectory};
        use crate::{shell::ExecuteCommand, git::GitCommit, RenderArtifact};
        // Confusable prose mentions the sibling tool.
        assert!(ReadFile.when_not_to_call().unwrap().contains("read_skill_file"));
        assert!(WriteFile.when_not_to_call().unwrap().contains("edit_file"));
        assert!(EditFile.when_not_to_call().unwrap().contains("write_file"));
        // Every required param on these tools now has a description.
        for s in [ReadFile.schema(), WriteFile.schema(), EditFile.schema(),
                  ExecuteCommand.schema(), GitCommit.schema(),
                  RenderArtifact.schema(), ListDirectory.schema()] {
            assert!(required_params_missing_description(&s).is_empty(),
                "{} has undescribed required params: {:?}", s.name, required_params_missing_description(&s));
        }
    }
```

(Canonical public paths verified: `crate::fs::{ReadFile,WriteFile,EditFile,ListDirectory}`, `crate::shell::ExecuteCommand`, `crate::git::GitCommit`, `crate::RenderArtifact`.)

- [ ] **Step 11: Build + test agent-tools**

Run: `cargo test -p agent-tools 2>&1 | tail -15`
Expected: all pass (contract + registry fold + compliance + existing).

Run: `cargo build -p agent-tools 2>&1 | tail -3`
Expected: `Finished`, no warnings.

- [ ] **Step 12: Commit**

```bash
git add agent/crates/agent-tools/src/
git commit -m "feat(tools): when_not_to_call contract + required-param descriptions

Add defaulted Tool::when_not_to_call folded into the model-facing schema by
ToolRegistry::schemas(); add shared CONFUSABLE_TOOLS + required_params_missing_description
(agent-tools contract module); give read_file/write_file/edit_file disambiguation
prose and backfill required-param descriptions across the agent-tools tools. Audit Finding 2."
```

---

### Task 2: Cross-crate confusable overrides + agent-memory enforcement

**Files:**
- Modify: `agent/crates/agent-core/src/context_tools.rs` (context_recall: `when_not_to_call`)
- Modify: `agent/crates/agent-skills/src/tools.rs` (read_skill_file: `when_not_to_call`)
- Modify: `agent/crates/agent-memory/src/tools.rs` (recall: `when_not_to_call` + `query` desc + enforcement test)

**Interfaces:**
- Consumes: `Tool::when_not_to_call` (Task 1), `agent_tools::required_params_missing_description` (Task 1).

- [ ] **Step 1: context_recall prose**

In `agent/crates/agent-core/src/context_tools.rs`, inside `impl Tool for ContextRecallTool`, after the `fn description` block (ends line ~27, before `fn schema`), add:

```rust
    fn when_not_to_call(&self) -> Option<&str> {
        Some("Not for semantic search of saved memories — use recall. Use \
              context_recall only to rehydrate a specific offloaded entry by its id.")
    }
```

- [ ] **Step 2: read_skill_file prose**

In `agent/crates/agent-skills/src/tools.rs`, inside `impl Tool for ReadSkillFile`, after `fn description` (ends line ~268, before `fn schema`), add:

```rust
    fn when_not_to_call(&self) -> Option<&str> {
        Some("Not for arbitrary workspace files — use read_file. Use read_skill_file \
              only for files bundled inside a loaded skill's directory.")
    }
```

- [ ] **Step 3: recall prose + query description**

In `agent/crates/agent-memory/src/tools.rs`, inside `impl Tool for Recall`, after `fn description` (ends line ~164, before `fn schema`), add:

```rust
    fn when_not_to_call(&self) -> Option<&str> {
        Some("Not for rehydrating offloaded conversation context — use context_recall. \
              Use recall only for semantic search over saved long-term memories.")
    }
```

Then in the same `Recall::schema()` (lines 169-176), replace the `properties` for `query`:
```rust
                    "query": {"type": "string"},
```
with:
```rust
                    "query": {"type": "string", "description": "Natural-language query to search saved memories for."},
```

- [ ] **Step 4: Write the failing agent-memory enforcement test**

In `agent/crates/agent-memory/src/tools.rs`, add a new `#[cfg(test)] mod` (or into an existing test mod that already imports `StubEmbedder`/`InMemoryStore`; there are several — reuse one that has `use crate::embedder::StubEmbedder;` and `use crate::store::InMemoryStore;`):

```rust
    #[test]
    fn recall_carries_disambiguation_and_described_query() {
        use crate::embedder::StubEmbedder;
        use crate::store::InMemoryStore;
        let rec = Recall {
            embedder: std::sync::Arc::new(StubEmbedder::d384()),
            store: std::sync::Arc::new(InMemoryStore::new()),
            cfg: std::sync::Arc::new(MemoryConfig::default()),
            project_key: "A".into(),
        };
        // Confusable contract present in the curated list AND on the tool.
        assert!(agent_tools::CONFUSABLE_TOOLS.contains(&"recall"));
        let wntc = rec.when_not_to_call().expect("recall must disambiguate vs context_recall");
        assert!(wntc.contains("context_recall"), "prose names the sibling: {wntc}");
        // Required param `query` is described.
        assert!(agent_tools::required_params_missing_description(&rec.schema()).is_empty(),
            "recall.query must have a description");
    }
```

- [ ] **Step 5: Run the agent-memory test to verify RED then GREEN**

First confirm it was failing before Steps 3 were applied — since you apply Step 3 before Step 4 in sequence, instead verify GREEN now and confirm the test is non-vacuous by temporarily reasoning: without Step 3's prose, `when_not_to_call()` would be `None` and `.expect(...)` would panic; without the query description, the last assert would fail.

Run: `cargo test -p agent-memory recall_carries 2>&1 | tail -12`
Expected: PASS.

- [ ] **Step 6: Build + test the three crates**

Run: `cargo test -p agent-core -p agent-skills -p agent-memory 2>&1 | grep -E "^test result|error" | tail -20`
Expected: all pass, no errors.

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-core/src/context_tools.rs agent/crates/agent-skills/src/tools.rs agent/crates/agent-memory/src/tools.rs
git commit -m "feat(tools): disambiguation prose for context_recall, read_skill_file, recall

Override when_not_to_call on the remaining confusable tools; describe recall's
required query param; enforce recall's contract in agent-memory (it is
runtime-injected and not visible to the agent-runtime-config enforcement test)."
```

---

### Task 3: Enforcement over the assembled registry (agent-runtime-config)

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs` (`BuiltLoop.schemas` test-only field + populate + enforcement test)

**Interfaces:**
- Consumes: `agent_tools::{CONFUSABLE_TOOLS, WHEN_NOT_TO_CALL_MARKER, required_params_missing_description, ToolSchema}` (Task 1); the folded `schemas()` (Task 1); the confusable overrides (Tasks 1-2).

- [ ] **Step 1: Expose the assembled schemas on BuiltLoop (test-only)**

In `agent/crates/agent-runtime-config/src/assemble.rs`, in the `BuiltLoop` struct (after the `#[cfg(test)] pub registered_names: Vec<String>,` field, ~line 43), add:

```rust
    /// Assembled, folded tool schemas — retained so tests can assert the tool contract.
    #[cfg(test)]
    pub schemas: Vec<agent_tools::ToolSchema>,
```

Replace the `registered_names` computation (line 117-118):

```rust
    #[cfg(test)]
    let registered_names: Vec<String> = registry.schemas().into_iter().map(|s| s.name).collect();
```

with (capture the folded schemas once, derive names from them):

```rust
    #[cfg(test)]
    let schemas = registry.schemas();
    #[cfg(test)]
    let registered_names: Vec<String> = schemas.iter().map(|s| s.name.clone()).collect();
```

And in the `BuiltLoop { ... }` literal (after `#[cfg(test)] registered_names,`, ~line 145), add:

```rust
        #[cfg(test)]
        schemas,
```

- [ ] **Step 2: Write the enforcement test**

In `agent/crates/agent-runtime-config/src/assemble.rs` `#[cfg(test)] mod tests` (which already has `cfg()` and `parts(...)` helpers), add:

```rust
    #[test]
    fn every_required_param_is_described_in_the_assembled_registry() {
        let dir = tempfile::tempdir().unwrap();
        // Default config (memory off): base + context + skill tools are real; the
        // runtime-injected `recall` is intentionally absent (enforced in agent-memory).
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        for s in &built.schemas {
            let missing = agent_tools::required_params_missing_description(s);
            assert!(missing.is_empty(), "{} has undescribed required params: {missing:?}", s.name);
        }
    }

    #[test]
    fn confusable_tools_carry_disambiguation_in_the_assembled_registry() {
        use std::collections::HashSet;
        let dir = tempfile::tempdir().unwrap();
        let built = assemble_loop(&cfg(), parts(dir.path().to_path_buf(), vec![]));
        let present: HashSet<&str> = built.schemas.iter().map(|s| s.name.as_str()).collect();

        // Every confusable tool that IS assembled here must carry the folded marker.
        for name in agent_tools::CONFUSABLE_TOOLS {
            if let Some(s) = built.schemas.iter().find(|s| s.name == *name) {
                assert!(s.description.contains(agent_tools::WHEN_NOT_TO_CALL_MARKER),
                    "{name} is missing '{}' in its description: {}",
                    agent_tools::WHEN_NOT_TO_CALL_MARKER, s.description);
            }
        }

        // Coverage ratchet: the ONLY confusable tool absent from this assembly is
        // `recall` (runtime-injected, enforced in agent-memory). If a future
        // confusable tool becomes invisible here without separate coverage, this
        // fails and forces a decision.
        let absent: HashSet<&str> = agent_tools::CONFUSABLE_TOOLS.iter().copied()
            .filter(|n| !present.contains(n)).collect();
        assert_eq!(absent, HashSet::from(["recall"]),
            "unexpected confusable tools missing from the assembled registry: {absent:?}");
    }
```

- [ ] **Step 3: Run the enforcement tests**

Run: `cargo test -p agent-runtime-config every_required_param confusable_tools 2>&1 | tail -15`
Expected: PASS both. (If `read_skill_file` were unexpectedly absent, the coverage-ratchet assert would fail with a clear message — that would mean `build_skills` did not register it; it does, per `build_skills`.)

- [ ] **Step 4: Full workspace build + test**

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished`, no warnings.

Run: `cargo test -p agent-runtime-config -p agent-tools -p agent-memory -p agent-core -p agent-skills 2>&1 | grep -E "^test result|error" | tail -20`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "test(config): enforce tool contract over the assembled registry

Expose folded schemas on BuiltLoop (test-only); assert every required param has a
description and every assembled confusable tool carries the 'When NOT to call:'
marker, with a coverage ratchet pinning the only absent confusable tool to the
runtime-injected recall."
```

---

## Final verification

- [ ] From `agent/`: `cargo build 2>&1 | tail -3` — whole workspace, no warnings.
- [ ] From `agent/`: `cargo test -p agent-tools -p agent-core -p agent-skills -p agent-memory -p agent-runtime-config 2>&1 | grep -E "^test result|error"` — all green.
- [ ] Spot-confirm the model-facing effect: in a scratch test or by reading `schemas()`, `read_file`'s assembled description ends with `When NOT to call: Not for files bundled inside a loaded skill's directory …`.

## Notes for the implementer

- Adding a defaulted trait method is backward-compatible: the ~14 non-confusable tools and all test doubles compile unchanged.
- The fold happens once per `schemas()` call and never mutates the stored tool, so repeated calls each yield exactly one marker.
- `StubEmbedder` (`crate::embedder::StubEmbedder`) and `InMemoryStore` (`crate::store::InMemoryStore`) are reachable from within agent-memory's own test module — reuse an existing test mod's imports (see the `seed()`/`seeded()` helpers around `tools.rs:287+`).
- MCP tools are dynamic/server-defined and only present with a live server, so they are intentionally out of scope for the static enforcement tests.
