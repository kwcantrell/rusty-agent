# Tool "When NOT to Call" Contract + Required-Param Descriptions — Design

**Date:** 2026-06-30
**Status:** Approved (brainstorming) → ready for plan
**Source:** Finding 2 of the harness-engineering audit re-run
(`.agents/skills/harness-engineering/audit.md`). Anchors re-verified against
current `main` on 2026-06-30.

## Principle

"Each tool has a clear name, tight description, and explicit *when NOT to call*
guidance" (Anthropic — Writing Tools for Agents). Today the `Tool` trait exposes
`name / description / schema / intent / execute` with no slot for exclusion prose,
and 8 tools ship required parameters with no description. Both make it easier for
the model to pick the wrong tool or mis-fill arguments. This adds a structural,
enforced contract for both.

## Current state (verified on `main`)

- `Tool` trait (`agent-tools/src/tool.rs:4-13`): `name/description/schema/intent/execute`, no exclusion slot.
- The model only reads `ToolSchema` (`name/description/parameters`) via
  `ToolRegistry::schemas()` (`agent-tools/src/registry.rs:21-23`) →
  `CompletionRequest.tools` (`agent-core/src/loop_.rs:205`). All 20 production
  tools set `schema.description == self.description()` (single source); `description()`
  is otherwise only a helper.
- **8 tools have ≥1 required param with no description:** `execute_command`
  (`command`), `git_commit` (`message`), `read_file` (`path`), `list_directory`
  (`path`), `write_file` (`path`,`content`), `edit_file` (`path`,`old`,`new`),
  `render` (`kind`), `recall` (`query`).
- **Three confusable clusters:** `recall`↔`context_recall`,
  `read_file`↔`read_skill_file`, `write_file`↔`edit_file`. Only `fetch_url` has
  any "when to use" hint today.
- The full registry is assembled in `assemble_loop` (`agent-runtime-config/src/assemble.rs`):
  `build_registry` (base agent-tools) + injected memory tools (`LoopParts.memory_tools`)
  + `build_skills(...)` + `agent_core::context_tools(...)`. **Memory tools are
  runtime-injected** (`build_memory_full` in CLI/server); assemble tests inject a
  *fake* memory tool, so an `agent-runtime-config` test does not see the real `recall`.

## Decisions (from brainstorming)

1. **Structural trait method + enforce** (not prose-only).
2. **Curated confusable set + test** (not auto-detect, not all-tools). The set is a
   maintained ratchet: it guards known clusters against regression; a new confusable
   tool must be added to the list by a human.
3. **Backfill required-param descriptions + a registry-wide test** requiring every
   required param of every (statically-assembled) tool to have a non-empty description.
   Optional params left as-is.

## Architecture

### Component 1 — trait method + registry fold (`agent-tools`)

`tool.rs` — add to the `Tool` trait a defaulted method:

```rust
    /// Guidance on when the model should NOT call this tool (and which sibling to
    /// prefer). `None` for tools whose name/purpose already disambiguate. The
    /// registry folds this into the model-facing schema description; it is not a
    /// separate wire field.
    fn when_not_to_call(&self) -> Option<&str> { None }
```

`registry.rs` — `schemas()` folds it into the description (the only place the
model sees it; the raw `description()`/`schema()` stay clean for logs/UI):

```rust
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| {
            let mut s = t.schema();
            if let Some(excl) = t.when_not_to_call() {
                s.description = format!("{}\n\nWhen NOT to call: {excl}", s.description);
            }
            s
        }).collect()
    }
```

Also in `agent-tools` (new small `contract` module or additions to an existing
one), the shared, pure enforcement surface:

```rust
/// The marker `schemas()` prepends; also the string enforcement tests grep for.
pub const WHEN_NOT_TO_CALL_MARKER: &str = "When NOT to call:";

/// Tools that are genuinely confusable with a sibling and MUST carry
/// `when_not_to_call` prose. A maintained ratchet — add a new confusable tool
/// here by hand. Clusters: recall/context_recall (semantic memory vs offload
/// rehydration), read_file/read_skill_file (workspace vs skill dir), write_file/
/// edit_file (create-or-overwrite vs unique-substring replace).
/// NOTE: `recall` is runtime-injected, so it is enforced in agent-memory's own
/// test rather than the agent-runtime-config test (see Enforcement).
pub const CONFUSABLE_TOOLS: &[&str] = &[
    "recall", "context_recall", "read_file", "read_skill_file", "write_file", "edit_file",
];

/// Names of required params on `schema` whose `properties[name].description` is
/// missing or empty. Empty vec = compliant.
pub fn required_params_missing_description(schema: &ToolSchema) -> Vec<String> {
    let p = &schema.parameters;
    let required = p.get("required").and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let props = p.get("properties").and_then(|v| v.as_object());
    required.iter().filter_map(|r| r.as_str()).filter(|name| {
        let desc = props.and_then(|o| o.get(*name))
            .and_then(|prop| prop.get("description")).and_then(|d| d.as_str());
        desc.map(|s| s.trim().is_empty()).unwrap_or(true)
    }).map(|s| s.to_string()).collect()
}
```

### Component 2 — disambiguation prose (6 confusable tools)

Each overrides `when_not_to_call()` in its own crate. Text (final wording pinned
in the plan; each ≤ ~200 chars, model-facing):

- `read_file` (`agent-tools/src/fs/read.rs`): "Not for files bundled inside a
  loaded skill's directory — use read_skill_file for those. Use read_file for
  workspace paths."
- `write_file` (`agent-tools/src/fs/write.rs`): "Not for a small change to an
  existing file — use edit_file to replace a specific substring. Use write_file to
  create a new file or fully overwrite one."
- `edit_file` (`agent-tools/src/fs/write.rs`): "Not for creating a new file or
  rewriting a whole file — use write_file. Use edit_file to replace one unique
  existing substring."
- `read_skill_file` (`agent-skills/src/tools.rs`): "Not for arbitrary workspace
  files — use read_file. Use read_skill_file only for files bundled inside a loaded
  skill's directory."
- `recall` (`agent-memory/src/tools.rs`): "Not for rehydrating offloaded
  conversation context — use context_recall. Use recall only for semantic search
  over saved long-term memories."
- `context_recall` (`agent-core/src/context_tools.rs`): "Not for semantic search of
  saved memories — use recall. Use context_recall only to rehydrate a specific
  offloaded entry by its id."

### Component 3 — required-param description backfill (8 tools)

Add a non-empty `"description"` to each bare required param. Exact JSON keys are
pinned against each tool's live `schema()` in the plan; intended text:

| tool | param | description |
|---|---|---|
| execute_command | command | The shell command line to execute. |
| git_commit | message | The commit message. |
| read_file | path | Workspace-relative path of the file to read. |
| list_directory | path | Workspace-relative path of the directory to list. |
| write_file | path | Workspace-relative path of the file to create or overwrite. |
| write_file | content | The full contents to write to the file. |
| edit_file | path | Workspace-relative path of the file to edit. |
| edit_file | old | The exact existing substring to replace; must occur exactly once. |
| edit_file | new | The replacement text. |
| render | kind | Which artifact kind to render; one of the allowed enum values. |
| recall | query | Natural-language query to search saved memories for. |

### Component 4 — enforcement (two tests, one shared const/helper)

`BuiltLoop` (`agent-runtime-config/src/assemble.rs`) gains
`pub schemas: Vec<ToolSchema>` (the assembled, *folded* schemas);
`registered_names` derives from it (`schemas.iter().map(|s| s.name.clone())`).

**Test 1 — `agent-runtime-config`** over `assemble_loop(all-features)`, i.e. a
config with `memory = true` and default skills/context, reusing the existing
`cfg()` / `parts(...)` test harness:
- **Required params:** for every `built.schemas`, assert
  `required_params_missing_description(schema)` is empty.
- **Confusable marker:** for every `CONFUSABLE_TOOLS` name that is present in
  `built.schemas`, assert its description contains `WHEN_NOT_TO_CALL_MARKER`.
- **Coverage ratchet:** assert the set of `CONFUSABLE_TOOLS` names *absent* from
  `built.schemas` equals exactly `{"recall"}` — the one runtime-injected confusable
  tool. If a future confusable tool becomes runtime-injected (and thus invisible
  here) without being covered elsewhere, this fails and forces a decision.

**Test 2 — `agent-memory`** over the real `Recall` (built with the existing
`StubEmbedder::d384()` + `InMemoryStore` seed helper):
- assert `Recall::when_not_to_call()` is `Some(non-empty)`;
- assert `required_params_missing_description(&Recall.schema())` is empty (covers
  `query`).

MCP tools are dynamic/server-defined and only present with a live server, so they
are out of scope for these static tests (noted, not enforced).

## Error handling & edge cases

- **Default `None`:** non-confusable tools are unchanged; no filler prose.
- **Fold idempotence:** `schemas()` folds once per call; the raw `schema()` is
  never mutated, so repeated `schemas()` calls each produce one marker.
- **`read_skill_file` presence:** if `build_skills` with empty `skills_dirs` does
  not register the skill tools, Test 1 won't see `read_skill_file`; the plan
  verifies registration and, if absent, the coverage ratchet's expected-absent set
  is widened to include it and it is enforced in an `agent-skills` test. (Verified
  in the plan, not assumed here.)
- **No execution/behavior change:** this is description/schema surface + tests only.

## Testing

- Component-4 Test 1 + Test 2 are the core guardrails.
- `agent-tools` unit tests: `schemas()` folds `when_not_to_call` into the
  description with the marker, and leaves a `None` tool's description untouched;
  `required_params_missing_description` returns the missing names for a bare schema
  and empty for a described one.
- A spot check that two confusable tools' `when_not_to_call()` return the expected
  substrings (e.g. `read_file` mentions `read_skill_file`).

## Files touched

- `agent-tools/src/tool.rs` — trait method.
- `agent-tools/src/registry.rs` — `schemas()` fold + unit tests.
- `agent-tools/src/{contract.rs or lib.rs}` — `WHEN_NOT_TO_CALL_MARKER`,
  `CONFUSABLE_TOOLS`, `required_params_missing_description` + unit tests.
- `agent-tools/src/fs/read.rs`, `agent-tools/src/fs/write.rs`,
  `agent-tools/src/shell.rs`, `agent-tools/src/git.rs`, `agent-tools/src/render.rs`
  — `when_not_to_call` overrides (read/write/edit) + required-param descriptions.
- `agent-skills/src/tools.rs` — `read_skill_file` override.
- `agent-core/src/context_tools.rs` — `context_recall` override.
- `agent-memory/src/tools.rs` — `recall` override + `query` description + Test 2.
- `agent-runtime-config/src/assemble.rs` — `BuiltLoop.schemas` + Test 1.
