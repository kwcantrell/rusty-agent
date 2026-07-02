# Tool-Result Ingestion Cap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** No tool-result message inside any built model request exceeds `max_tool_result_bytes`; oversized results are eagerly offloaded whole (recallable by id) and the window keeps a bounded preview + recall marker.

**Architecture:** An eager size-based pass becomes step (0) of `CuratedContext::maintain` (which runs between tool-result append and every model call), reusing the existing offload store, `OffloadHit` selection shape, `ContextEvent::Offloaded`, and `MaintReport`. `context_recall` gains byte-offset pagination so capped content stays reachable; `read_file` gains line-based offset/limit. A new `RuntimeConfig.max_tool_result_bytes` tunes the cap.

**Tech Stack:** Rust (agent/ Cargo workspace), tokio, serde. No web changes.

**Spec:** `docs/superpowers/specs/2026-07-01-tool-result-ingestion-cap-design.md` — read it first; its Decisions and edge-case sections are binding.

## Global Constraints

- Two Cargo workspaces exist; everything here is in `agent/` — run cargo from `/home/kalen/rust-agent-runtime/agent` (`source ~/.cargo/env` if cargo is missing).
- Conventional commits: `type(scope): summary`.
- `agent-core/src/lib.rs` glob-re-exports every module (`pub use offload_policy::*;`) — new `pub` items need no export wiring.
- Existing behavior that must NOT change: `AgentEvent::ToolResult` still carries full tool output; tools still return full output (`shell.rs` test `captures_large_output_without_deadlock` stays untouched); `WindowContext` stays uncapped; small `context_recall` entries return verbatim.
- The final gate is `bash scripts/ci.sh` from the repo root (fmt + clippy + tests + web checks). Clippy is `-D warnings`: no unused imports, use `format!` inline args style (`{id}` not `{}`, matching existing code).

---

### Task 1: Eager selection, marker, and preview helpers (`agent-core/src/offload_policy.rs`)

**Files:**
- Modify: `agent/crates/agent-core/src/offload_policy.rs`

**Interfaces:**
- Consumes: existing `OffloadConfig`, `OffloadHit`, `OffloadEntry`, `classify`, `PLACEHOLDER_PREFIX`.
- Produces (used by Tasks 2 and 5):
  - `pub const DEFAULT_MAX_TOOL_RESULT_BYTES: usize = 16 * 1024;`
  - `OffloadConfig.max_result_bytes: usize` (new field, default = the const)
  - `pub fn select_oversized(history: &[Message], config: &OffloadConfig) -> Vec<OffloadHit>`
  - `pub fn truncation_marker(id: OffloadId, tool_name: &str, shown: usize, total: usize) -> String`
  - `pub fn capped_preview(content: &str, cap: usize, id: OffloadId, tool_name: &str) -> String`

- [ ] **Step 1: Write the failing tests** (append inside the existing `#[cfg(test)] mod tests`)

```rust
    fn cap_cfg(max: usize) -> OffloadConfig {
        OffloadConfig {
            max_result_bytes: max,
            ..Default::default()
        }
    }

    #[test]
    fn oversized_fresh_result_is_selected_despite_keep_recent() {
        // keep_recent protects by AGE; size-based selection must ignore it.
        let history = vec![tool_msg("shell", &"x".repeat(5000))];
        let hits = select_oversized(&history, &cap_cfg(1024));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].history_index, 0);
        assert_eq!(hits[0].entry.bytes, 5000);
        assert_eq!(hits[0].entry.kind, OffloadKind::Output);
    }

    #[test]
    fn at_cap_result_is_not_selected() {
        let history = vec![tool_msg("shell", &"x".repeat(1024))];
        assert!(select_oversized(&history, &cap_cfg(1024)).is_empty());
    }

    #[test]
    fn oversized_error_is_classified_error() {
        let history = vec![tool_msg("shell", &format!("ERROR: {}", "x".repeat(5000)))];
        let hits = select_oversized(&history, &cap_cfg(1024));
        assert_eq!(hits[0].entry.kind, OffloadKind::Error);
    }

    #[test]
    fn placeholders_and_excluded_tools_are_not_selected() {
        let placeholder = format!("[tool_result#7 offloaded: 9000B output \
             from \"shell\" — recall with context_recall(7)]{}", "x".repeat(2000));
        let history = vec![
            Message::tool("c1", "shell", &placeholder),
            Message::tool("c2", "use_skill", &"y".repeat(5000)),
        ];
        let cfg = OffloadConfig {
            max_result_bytes: 1024,
            exclude_tools: vec!["use_skill".into()],
            ..Default::default()
        };
        assert!(select_oversized(&history, &cfg).is_empty());
    }

    #[test]
    fn non_tool_messages_are_never_selected() {
        let history = vec![Message::user(&"x".repeat(5000))];
        assert!(select_oversized(&history, &cap_cfg(1024)).is_empty());
    }

    #[test]
    fn capped_preview_fits_cap_and_carries_the_recall_hint() {
        let content = "x".repeat(50_000);
        let out = capped_preview(&content, 1024, 42, "shell");
        assert!(out.len() <= 1024, "preview+marker {} > cap", out.len());
        assert!(out.starts_with("xxx"));
        assert!(out.contains("tool_result#42 truncated"));
        assert!(out.contains("of 50000B"));
        assert!(out.contains("context_recall(id: 42, offset: "));
    }

    #[test]
    fn capped_preview_respects_char_boundaries() {
        // 4-byte scalars; a byte-index cut inside one would panic on slicing.
        let content = "🦀".repeat(20_000);
        let out = capped_preview(&content, 1024, 1, "shell");
        assert!(out.len() <= 1024);
        assert!(out.starts_with('🦀'));
    }

    #[test]
    fn capped_preview_is_idempotent_under_reselection() {
        let content = "x".repeat(50_000);
        let out = capped_preview(&content, 1024, 1, "shell");
        let history = vec![Message::tool("c1", "shell", &out)];
        assert!(
            select_oversized(&history, &cap_cfg(1024)).is_empty(),
            "capped output must never be re-selected"
        );
    }

    #[test]
    fn pathological_small_cap_degrades_to_placeholder_prefix() {
        // cap smaller than the marker itself: output is marker-only and must
        // start with PLACEHOLDER_PREFIX so both selectors skip it forever.
        let content = "x".repeat(50_000);
        let out = capped_preview(&content, 16, 3, "shell");
        assert!(out.starts_with("[tool_result#3"));
        let history = vec![Message::tool("c1", "shell", &out)];
        assert!(select_oversized(&history, &cap_cfg(16)).is_empty());
    }
```

Note: `tool_msg` already exists in this test module. `Message::user` — check `agent-model`'s constructors (`Message::user(...)` exists; mirror whatever `context.rs` tests use if the signature differs).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-core offload_policy`
Expected: COMPILE ERROR — `max_result_bytes`, `select_oversized`, `capped_preview` not found.

- [ ] **Step 3: Implement**

Add the constant above `OffloadConfig`:

```rust
/// Default eager ingestion cap (`OffloadConfig::max_result_bytes`), also the
/// default for `RuntimeConfig::max_tool_result_bytes`. ~4K tokens: large
/// enough for real command output, small enough that one result cannot
/// swamp a small window.
pub const DEFAULT_MAX_TOOL_RESULT_BYTES: usize = 16 * 1024;
```

Extend `OffloadConfig` (field + Default):

```rust
    /// Eager cap: any tool result larger than this many bytes is offloaded at
    /// ingestion (bounded preview + recall marker), regardless of age.
    pub max_result_bytes: usize,
```

```rust
            max_result_bytes: DEFAULT_MAX_TOOL_RESULT_BYTES,
```

Add below `placeholder_for`:

```rust
/// Marker appended to an ingestion-capped preview. `shown`/`total` are byte counts.
pub fn truncation_marker(id: OffloadId, tool_name: &str, shown: usize, total: usize) -> String {
    format!(
        "\n[tool_result#{id} truncated: showing first {shown}B of {total}B from \
         \"{tool_name}\" — continue with context_recall(id: {id}, offset: {shown})]"
    )
}

/// Truncate `content` so preview + marker fit within `cap` bytes (char-boundary
/// safe). When `cap` cannot even hold the marker, degrades to a marker-only
/// string with no leading newline, which starts with `PLACEHOLDER_PREFIX` and
/// is therefore never re-selected.
pub fn capped_preview(content: &str, cap: usize, id: OffloadId, tool_name: &str) -> String {
    let total = content.len();
    // Budget against the widest the marker can render (`shown = total`), so the
    // final string can only come in under `cap`.
    let worst = truncation_marker(id, tool_name, total, total);
    let mut cut = cap.saturating_sub(worst.len()).min(total);
    while !content.is_char_boundary(cut) {
        cut -= 1;
    }
    if cut == 0 {
        return truncation_marker(id, tool_name, 0, total)
            .trim_start()
            .to_string();
    }
    format!("{}{}", &content[..cut], truncation_marker(id, tool_name, cut, total))
}

/// Select tool-result messages exceeding the eager ingestion cap, regardless
/// of age. Pure: no I/O, deterministic. Skips excluded tools and placeholders.
pub fn select_oversized(history: &[Message], config: &OffloadConfig) -> Vec<OffloadHit> {
    let mut hits = Vec::new();
    for (i, m) in history.iter().enumerate() {
        if !matches!(m.role, Role::Tool)
            || m.content.starts_with(PLACEHOLDER_PREFIX)
            || m.content.len() <= config.max_result_bytes
        {
            continue;
        }
        let tool_name = m.name.clone().unwrap_or_default();
        if config.exclude_tools.iter().any(|t| t == &tool_name) {
            continue;
        }
        hits.push(OffloadHit {
            history_index: i,
            entry: OffloadEntry {
                id: 0,
                tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
                tool_name,
                kind: classify(&m.content),
                content: m.content.clone(),
                bytes: m.content.len(),
                turn: i,
            },
        });
    }
    hits
}
```

Idempotence argument (why the tests hold): normal case output = `cut + marker(cut, total)` bytes where `cut = cap − worst_marker` and `marker(cut, total) ≤ worst_marker` (digit-width of `shown ≤ total`), so output ≤ cap and `len > cap` never re-triggers; marker-only case starts with `PLACEHOLDER_PREFIX` and is skipped explicitly.

Anywhere existing code/tests construct `OffloadConfig { ... }` with explicit fields and no `..Default::default()`, add the new field (grep: `rg "OffloadConfig \{" agent/crates`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-core offload_policy`
Expected: PASS (all new + existing).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/offload_policy.rs
git commit -m "feat(core): eager size-based offload selection + capped preview helpers"
```

---

### Task 2: Ingestion-cap pass in `CuratedContext::maintain` (`agent-core/src/curated.rs`)

**Files:**
- Modify: `agent/crates/agent-core/src/curated.rs`

**Interfaces:**
- Consumes (Task 1): `select_oversized`, `capped_preview`, `OffloadConfig.max_result_bytes`.
- Produces: `maintain` guarantees no history tool message exceeds `max_result_bytes` after each pass; eager offloads counted in `MaintReport.offloaded{,_bytes}` and emitted as `ContextEvent::Offloaded`.

- [ ] **Step 1: Write the failing tests** (in `curated.rs`'s existing test module; mirror the file's existing `maintain` test setup for `MaintCtx`/sink/model construction — compaction is not triggered in these tests, so the existing scripted-model stub works as-is)

```rust
    #[tokio::test]
    async fn ingestion_cap_offloads_fresh_oversized_result_before_next_build() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system("s"), store.clone(), flag)
            .with_offload_config(OffloadConfig {
                max_result_bytes: 1024,
                ..Default::default()
            });
        let big = "x".repeat(50_000);
        ctx.append(Message::tool("c1", "shell", &big));

        // Build MaintCtx exactly like the existing maintain tests in this file
        // (CollectingSink + scripted model + CancellationToken), with a large
        // model_limit so neither compaction nor eviction interferes.
        let sink = CollectingSink::default();
        /* deps setup per existing tests, model_limit = 1_000_000 */

        let report = ctx.maintain(&deps).await;

        assert_eq!(report.offloaded, 1);
        assert_eq!(report.offloaded_bytes, 50_000);
        let msg = &ctx.history()[0];
        assert!(msg.content.len() <= 1024, "window copy exceeds cap");
        assert!(msg.content.contains("truncated: showing first"));
        assert_eq!(msg.tool_call_id.as_deref(), Some("c1"), "id must survive");
        // Full content stored, recallable.
        let entry = store.get(1).expect("entry stored");
        assert_eq!(entry.content.len(), 50_000);
        // Offloaded event emitted.
        assert!(sink_events_contain_offloaded(&sink, 1, 50_000, "shell"));
        // Second pass is a no-op (idempotent).
        let report2 = ctx.maintain(&deps).await;
        assert_eq!(report2.offloaded, 0);
    }

    #[tokio::test]
    async fn capped_preview_is_age_offloaded_to_a_placeholder_later() {
        // keep_recent 0 lets the age pass run on the same maintain call: the
        // eager pass stores the full content (#1), then the age pass lifts the
        // preview into a second small entry (#2) whose content still carries
        // the marker to #1 — the recall chain stays intact.
        let store = Arc::new(InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system("s"), store.clone(), flag)
            .with_offload_config(OffloadConfig {
                max_result_bytes: 1024,
                output_min_bytes: 100,
                keep_recent: 0,
                ..Default::default()
            });
        ctx.append(Message::tool("c1", "shell", &"x".repeat(50_000)));
        /* deps setup as above */

        let report = ctx.maintain(&deps).await;

        assert_eq!(report.offloaded, 2, "eager + age in one pass");
        let msg = &ctx.history()[0];
        assert!(msg.content.starts_with("[tool_result#2 offloaded:"));
        assert!(store.get(2).unwrap().content.contains("context_recall(id: 1"));
        assert_eq!(store.get(1).unwrap().content.len(), 50_000);
    }

    #[tokio::test]
    async fn oversized_error_result_is_capped_too() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let flag = Arc::new(AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system("s"), store.clone(), flag)
            .with_offload_config(OffloadConfig {
                max_result_bytes: 1024,
                ..Default::default()
            });
        ctx.append(Message::tool("c1", "shell", &format!("ERROR: {}", "e".repeat(50_000))));
        /* deps setup as above */
        ctx.maintain(&deps).await;
        assert!(ctx.history()[0].content.len() <= 1024);
        assert!(matches!(store.get(1).unwrap().kind, OffloadKind::Error));
    }
```

Write `sink_events_contain_offloaded` as a small local helper over `CollectingSink`'s captured events (match `AgentEvent::Context(ContextEvent::Offloaded { id, bytes, tool })`) — or assert inline the way neighboring tests inspect the sink; keep whichever style the file already uses.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent-core curated`
Expected: FAIL — new tests panic (`report.offloaded == 0`, window copy still 50 000 B).

- [ ] **Step 3: Implement**

In `maintain`, insert step (0) before the existing step (a), and pull the shared store/replace/report/emit body into one private helper both steps call:

```rust
    async fn maintain(&mut self, deps: &MaintCtx<'_>) -> MaintReport {
        let mut report = MaintReport::default();

        // (0) Ingestion cap — an oversized fresh result is offloaded whole
        // before it can reach a model call; the window keeps a bounded
        // preview + recall marker. Age is irrelevant here, only size.
        let cap = self.config.max_result_bytes;
        for hit in select_oversized(&self.history, &self.config) {
            let content = hit.entry.content.clone();
            let tool = hit.entry.tool_name.clone();
            self.lift(hit, &mut report, deps, |id| {
                capped_preview(&content, cap, id, &tool)
            });
        }

        // (a) Deterministic age-based offload — sync, cheap, every turn.
        for hit in select_offloads(&self.history, &self.config) {
            let tool = hit.entry.tool_name.clone();
            let kind = hit.entry.kind.clone();
            let bytes = hit.entry.bytes;
            self.lift(hit, &mut report, deps, |id| {
                placeholder_for(id, &tool, &kind, bytes)
            });
        }
        // ... (b) compaction and (c) eviction visibility unchanged ...
    }
```

And in the `impl CuratedContext` block (beside `compact_old_span`):

```rust
    /// Store a hit's full content in the offload table and replace the window
    /// copy with whatever `replacement` renders for the assigned id. Shared by
    /// the ingestion-cap and age-based offload passes.
    fn lift(
        &mut self,
        hit: crate::offload_policy::OffloadHit,
        report: &mut MaintReport,
        deps: &MaintCtx<'_>,
        replacement: impl FnOnce(crate::OffloadId) -> String,
    ) {
        let idx = hit.history_index;
        let tool = hit.entry.tool_name.clone();
        let bytes = hit.entry.bytes;
        let id = self.store.put(hit.entry);
        self.history[idx].content = replacement(id);
        report.offloaded += 1;
        report.offloaded_bytes += bytes;
        deps.sink.emit(AgentEvent::Context(ContextEvent::Offloaded {
            id,
            bytes,
            tool,
        }));
    }
```

Update imports: `use crate::offload_policy::{capped_preview, placeholder_for, select_offloads, select_oversized, OffloadConfig};`. The existing step (a) loop body is replaced by the `lift` call shown above — no behavior change there (same order: store, replace, count, emit).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-core`
Expected: PASS — new tests plus every existing curated/offload/e2e_context test.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/curated.rs
git commit -m "feat(core): ingestion cap — maintain eagerly offloads oversized tool results"
```

---

### Task 3: `context_recall` pagination (`agent-core/src/context_tools.rs` + call sites)

**Files:**
- Modify: `agent/crates/agent-core/src/context_tools.rs`
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs:102` (call-site signature)

**Interfaces:**
- Consumes (Task 1): `DEFAULT_MAX_TOOL_RESULT_BYTES` (temporary default at the assemble call site until Task 5 threads the config value).
- Produces: `ContextRecallTool::new(store, page_bytes: usize)`; `pub fn context_tools(store, flag, recall_page_bytes: usize) -> Vec<Arc<dyn Tool>>`; optional `offset` param on the `context_recall` schema; total tool output ≤ `page_bytes` per call at production page sizes.

- [ ] **Step 1: Write the failing tests**

```rust
    fn put_entry(store: &InMemoryOffloadStore, content: &str) -> u64 {
        store.put(OffloadEntry {
            id: 0,
            tool_call_id: "c1".into(),
            tool_name: "shell".into(),
            kind: OffloadKind::Output,
            content: content.into(),
            bytes: content.len(),
            turn: 0,
        })
    }

    /// Extract the continuation offset from a page's trailing marker.
    fn continuation_offset(page: &str) -> Option<usize> {
        let tail = page.rsplit("offset: ").next()?;
        tail.split(')').next()?.trim().parse().ok()
    }

    #[tokio::test]
    async fn recall_pages_a_large_entry_to_completion() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let content: String = (0..10_000).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        let id = put_entry(&store, &content);
        let tool = ContextRecallTool::new(store, 4096);

        let mut reassembled = String::new();
        let mut offset = 0usize;
        loop {
            let out = tool
                .execute(json!({ "id": id, "offset": offset }), &tool_ctx())
                .await
                .unwrap();
            assert!(out.content.len() <= 4096, "page exceeds budget");
            match continuation_offset(&out.content) {
                Some(next) if out.content.contains("continue with context_recall") => {
                    let body = out.content.rsplit_once("\n[bytes ").unwrap().0;
                    assert!(next > offset, "no forward progress");
                    reassembled.push_str(body);
                    offset = next;
                }
                _ => {
                    reassembled.push_str(&out.content);
                    break;
                }
            }
        }
        assert_eq!(reassembled, content, "pages must reassemble the original");
    }

    #[tokio::test]
    async fn recall_small_entry_has_no_marker() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let id = put_entry(&store, "short");
        let tool = ContextRecallTool::new(store, 4096);
        let out = tool.execute(json!({ "id": id }), &tool_ctx()).await.unwrap();
        assert_eq!(out.content, "short");
    }

    #[tokio::test]
    async fn recall_offset_past_end_is_invalid_args() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let id = put_entry(&store, "short");
        let tool = ContextRecallTool::new(store, 4096);
        let err = tool
            .execute(json!({ "id": id, "offset": 999 }), &tool_ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn recall_slices_on_char_boundaries() {
        let store = Arc::new(InMemoryOffloadStore::new());
        let content = "🦀".repeat(3000); // 12 000 bytes of 4-byte scalars
        let id = put_entry(&store, &content);
        let tool = ContextRecallTool::new(store, 4096);
        let out = tool.execute(json!({ "id": id }), &tool_ctx()).await.unwrap();
        assert!(out.content.starts_with('🦀')); // no panic, clean boundary
    }
```

Also update the two existing tests (`recall_returns_full_content`, `recall_unknown_id_is_not_found`) to the new constructor: `ContextRecallTool::new(store, 8 * 1024)`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent-core context_tools`
Expected: COMPILE ERROR — `new` takes 1 argument.

- [ ] **Step 3: Implement**

```rust
pub struct ContextRecallTool {
    store: Arc<dyn OffloadStore>,
    /// Max bytes returned per call (slice + continuation marker together).
    page_bytes: usize,
}

impl ContextRecallTool {
    pub fn new(store: Arc<dyn OffloadStore>, page_bytes: usize) -> Self {
        Self { store, page_bytes }
    }
}
```

Schema `properties` gains (keep `required: ["id"]`):

```rust
"offset": { "type": "integer", "description":
    "Byte offset to continue from (default 0). Use the offset value given \
     in a previous page's continuation marker." }
```

Update `description()` to mention paging: `"Recall the content of a previously offloaded tool result by its id (the number in a [tool_result#N ...] placeholder or truncation marker). Large entries return in pages; follow the continuation marker's offset to read more."`

`execute` body:

```rust
        let id = args
            .get("id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ToolError::InvalidArgs("missing integer 'id'".into()))?;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let entry = self.store.get(id).ok_or_else(|| {
            ToolError::NotFound(format!("no offloaded entry #{id} (may have been cleared)"))
        })?;
        let total = entry.content.len();
        if offset > 0 && offset >= total {
            return Err(ToolError::InvalidArgs(format!(
                "offset {offset} is past the end of entry #{id} ({total} bytes)"
            )));
        }
        let mut start = offset;
        while !entry.content.is_char_boundary(start) {
            start -= 1;
        }
        let rest = &entry.content[start..];
        if rest.len() <= self.page_bytes {
            return Ok(ToolOutput {
                content: rest.to_string(),
                display: None,
            });
        }
        // Budget the slice against the widest the marker can render (end = total).
        let worst = recall_marker(id, start, total, total);
        let budget = self.page_bytes.saturating_sub(worst.len()).max(1);
        let mut cut = start + budget;
        while !entry.content.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut <= start {
            // Pathological page size smaller than one scalar + marker: still
            // make forward progress by taking exactly one char.
            cut = start + rest.chars().next().map_or(1, |c| c.len_utf8());
        }
        let content = format!(
            "{}{}",
            &entry.content[start..cut],
            recall_marker(id, start, cut, total)
        );
        Ok(ToolOutput {
            content,
            display: None,
        })
```

With a private helper above the impl:

```rust
/// Continuation marker for a paged recall. `start`/`end` are byte offsets.
fn recall_marker(id: u64, start: usize, end: usize, total: usize) -> String {
    format!(
        "\n[bytes {start}–{end} of {total} — continue with context_recall(id: {id}, offset: {end})]"
    )
}
```

Signature change at the bottom of the file:

```rust
/// The context-management tool pair, sharing handles with a `CuratedContext`.
/// `recall_page_bytes` bounds each `context_recall` page (callers pass the
/// ingestion cap so recall pages can never re-trip it).
pub fn context_tools(
    store: Arc<dyn OffloadStore>,
    flag: Arc<AtomicBool>,
    recall_page_bytes: usize,
) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ContextRecallTool::new(store, recall_page_bytes)),
        Arc::new(ContextCompactTool::new(flag)),
    ]
}
```

And in `agent/crates/agent-runtime-config/src/assemble.rs` (line ~102), keep the workspace compiling with the default (Task 5 swaps in the config value):

```rust
    for t in agent_core::context_tools(
        parts.offload_store.clone(),
        parts.compact_flag.clone(),
        agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES,
    ) {
        registry.register(t);
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-core context_tools && cargo build`
Expected: PASS, whole workspace builds (assemble call site updated).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/context_tools.rs agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "feat(core): context_recall byte-offset pagination bounded by a page budget"
```

---

### Task 4: `read_file` offset/limit pagination (`agent-tools/src/fs/read.rs`)

**Files:**
- Modify: `agent/crates/agent-tools/src/fs/read.rs`

**Interfaces:**
- Consumes: nothing from other tasks (independent).
- Produces: optional `offset` (1-based start line, default 1) and `limit` (max lines, default all) params on `read_file`; sliced output prefixed by a `[lines {first}–{last} of {n}]` header line. Whole-file default output byte-identical to today.

- [ ] **Step 1: Write the failing tests** (in the file's existing test module; mirror its tempdir/ToolCtx setup)

```rust
    #[tokio::test]
    async fn read_file_slices_with_offset_and_limit() {
        let (ctx, dir) = ctx_with_workspace(); // reuse/mirror the existing test helper
        std::fs::write(dir.join("f.txt"), "l1\nl2\nl3\nl4\nl5\n").unwrap();
        let out = ReadFile
            .execute(json!({"path": "f.txt", "offset": 2, "limit": 2}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "[lines 2–3 of 5]\nl2\nl3");
    }

    #[tokio::test]
    async fn read_file_limit_clamps_to_eof() {
        let (ctx, dir) = ctx_with_workspace();
        std::fs::write(dir.join("f.txt"), "l1\nl2\nl3\n").unwrap();
        let out = ReadFile
            .execute(json!({"path": "f.txt", "offset": 3, "limit": 99}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "[lines 3–3 of 3]\nl3");
    }

    #[tokio::test]
    async fn read_file_default_is_whole_file_unchanged() {
        let (ctx, dir) = ctx_with_workspace();
        std::fs::write(dir.join("f.txt"), "l1\nl2\n").unwrap();
        let out = ReadFile.execute(json!({"path": "f.txt"}), &ctx).await.unwrap();
        assert_eq!(out.content, "l1\nl2\n"); // byte-identical, incl. trailing newline
    }

    #[tokio::test]
    async fn read_file_offset_past_eof_is_invalid_args() {
        let (ctx, dir) = ctx_with_workspace();
        std::fs::write(dir.join("f.txt"), "l1\n").unwrap();
        let err = ReadFile
            .execute(json!({"path": "f.txt", "offset": 5}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn read_file_offset_limit_params_are_described() {
        let schema = ReadFile.schema();
        for p in ["offset", "limit"] {
            let d = schema.parameters["properties"][p]["description"]
                .as_str()
                .unwrap_or("");
            assert!(!d.is_empty(), "{p} must be described");
        }
    }
```

If the existing tests build `ToolCtx` inline rather than via a helper, follow that pattern instead of inventing `ctx_with_workspace`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent-tools read`
Expected: FAIL — offset/limit ignored (content mismatches), schema test fails.

- [ ] **Step 3: Implement**

Schema:

```rust
            parameters: json!({"type":"object","properties":{
                "path":{"type":"string","description":"Workspace-relative path of the file to read."},
                "offset":{"type":"integer","description":"1-based line number to start reading from (default 1)."},
                "limit":{"type":"integer","description":"Maximum number of lines to return (default: all lines)."}},
                "required":["path"]}),
```

`execute`, after the existing `read_to_string`:

```rust
        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| (v as usize).max(1));
        let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
        let content = match (offset, limit) {
            (None, None) => content, // whole-file fast path, byte-identical
            (o, l) => {
                let first = o.unwrap_or(1);
                let lines: Vec<&str> = content.lines().collect();
                let n = lines.len();
                if first > n {
                    return Err(ToolError::InvalidArgs(format!(
                        "offset {first} is past the end of {path} ({n} lines)"
                    )));
                }
                let last = l.map_or(n, |l| (first + l - 1).min(n));
                if first == 1 && last == n {
                    content // limit covers the whole file: unchanged
                } else {
                    format!("[lines {first}–{last} of {n}]\n{}", lines[first - 1..last].join("\n"))
                }
            }
        };
        Ok(ToolOutput {
            content,
            display: None,
        })
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p agent-tools`
Expected: PASS (new + all existing fs tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/src/fs/read.rs
git commit -m "feat(tools): read_file offset/limit line pagination"
```

---

### Task 5: Config surface + frontend wiring (`agent-runtime-config`, `agent-cli`, `agent-server`)

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs`
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs`
- Modify: `agent/crates/agent-cli/src/main.rs` (~line 268)
- Modify: `agent/crates/agent-server/src/session.rs` (~lines 68, 270)

**Interfaces:**
- Consumes (Tasks 1, 3): `DEFAULT_MAX_TOOL_RESULT_BYTES`, `OffloadConfig.max_result_bytes`, 3-arg `context_tools`.
- Produces: `RuntimeConfig.max_tool_result_bytes: usize` (serde default, partial-merge aware); both frontends construct `CuratedContext` `.with_offload_config(...)` from it; `assemble.rs` threads it to `context_tools`.

- [ ] **Step 1: Write the failing tests** (in `runtime_config.rs`'s existing test module, following its serde/merge test style)

```rust
    #[test]
    fn max_tool_result_bytes_defaults_and_merges() {
        // Old on-disk file without the field → default.
        let c = RuntimeConfig::from_launch(
            "openai".into(),
            "http://x".into(),
            "m".into(),
            "native".into(),
            8192,
        );
        assert_eq!(c.max_tool_result_bytes, agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES);

        // A serialized config missing the field deserializes to the default.
        let mut v: serde_json::Value = serde_json::to_value(&c).unwrap();
        v.as_object_mut().unwrap().remove("max_tool_result_bytes");
        let parsed: RuntimeConfig = serde_json::from_value(v).unwrap();
        assert_eq!(parsed.max_tool_result_bytes, agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES);
    }
```

Also add a merge assertion following the file's existing partial-merge test (write a JSON file containing only `{"max_tool_result_bytes": 4096}` through the same load/merge path the neighboring tests use, and assert the field overrides while others keep base values). Note: `agent-runtime-config` already depends on `agent-core` (assemble.rs uses it), so the constant is importable; the test helper for the seeded config is whatever the module's existing tests use (`RuntimeConfig::from_launch(...)`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p agent-runtime-config runtime_config`
Expected: COMPILE ERROR — no field `max_tool_result_bytes`.

- [ ] **Step 3: Implement**

`runtime_config.rs`:

```rust
    #[serde(default = "default_max_tool_result_bytes")]
    pub max_tool_result_bytes: usize,
```

(place it after `context_limit`), plus:

```rust
fn default_max_tool_result_bytes() -> usize {
    agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES
}
```

- `PartialRuntimeConfig`: `max_tool_result_bytes: Option<usize>,`
- `merge`: `if let Some(v) = p.max_tool_result_bytes { self.max_tool_result_bytes = v; }`
- `from_launch`: `max_tool_result_bytes: default_max_tool_result_bytes(),`

`assemble.rs` — replace the Task-3 placeholder default with the config value:

```rust
    for t in agent_core::context_tools(
        parts.offload_store.clone(),
        parts.compact_flag.clone(),
        cfg.max_tool_result_bytes,
    ) {
        registry.register(t);
    }
```

`agent-cli/src/main.rs` (~268; the runtime config variable is `rt`):

```rust
    let mut ctx = CuratedContext::new(
        Message::system(built.system_prompt),
        offload_store,
        compact_flag,
    )
    .with_recall_budget(memory.recall_token_budget)
    .with_offload_config(agent_core::OffloadConfig {
        max_result_bytes: rt.max_tool_result_bytes,
        ..Default::default()
    });
```

`agent-server/src/session.rs` — `from_params` (capture the value BEFORE `config` moves into `RuntimeState::new` at ~line 54):

```rust
        let max_tool_result_bytes = config.max_tool_result_bytes;
```

then extend the `CuratedContext::new(...)` at ~68:

```rust
            .with_recall_budget(params.recall_token_budget)
            .with_offload_config(agent_core::OffloadConfig {
                max_result_bytes: max_tool_result_bytes,
                ..Default::default()
            }),
```

and in `set_workspace` (~270), read the CURRENT settings:

```rust
        .with_recall_budget(self.recall_budget)
        .with_offload_config(agent_core::OffloadConfig {
            max_result_bytes: self.runtime.settings_state().settings.max_tool_result_bytes,
            ..Default::default()
        });
```

(Known, accepted behavior: a settings change to the cap applies to new contexts — next workspace switch/session — not the live one; same as `recall_budget` today.)

Check `agent-server`'s wire/settings DTO: if `SettingsState`/the settings wire struct mirrors `RuntimeConfig` field-by-field, add `max_tool_result_bytes` there and to `web/src` types ONLY if the wire struct is exhaustive (grep `context_limit` in `agent-server/src/wire.rs` and `web/src` to see whether config crosses as a typed struct or as serde-transparent `RuntimeConfig`). If it serializes `RuntimeConfig` directly (serde default handles old clients), no wire/web change is needed.

- [ ] **Step 4: Run tests + build everything**

Run: `cargo test -p agent-runtime-config && cargo build && cargo test -p agent-server -p agent-cli`
Expected: PASS; whole workspace compiles with the new wiring.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config agent/crates/agent-cli/src/main.rs agent/crates/agent-server/src/session.rs
git commit -m "feat(config): max_tool_result_bytes — wire the ingestion cap into both frontends"
```

---

### Task 6: Full gate + spec status

**Files:**
- Modify: `docs/superpowers/specs/2026-07-01-tool-result-ingestion-cap-design.md` (Status line)

- [ ] **Step 1: Run the full CI gate**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: fmt, clippy, all agent tests, web typecheck + vitest — all green. Fix anything it flags (fmt/clippy issues from Tasks 1–5) before proceeding.

- [ ] **Step 2: Update the spec status**

Change the spec's `**Status:**` line to `Implemented (this plan: docs/superpowers/plans/2026-07-01-tool-result-ingestion-cap.md)`.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-07-01-tool-result-ingestion-cap-design.md
git commit -m "docs(spec): mark tool-result ingestion cap spec implemented"
```
