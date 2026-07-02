# Tool-Result Ingestion Cap — Design

**Date:** 2026-07-01
**Status:** Implemented (this plan: docs/superpowers/plans/2026-07-01-tool-result-ingestion-cap.md)
**Source:** Cluster 7 of the harness deep audit
(`docs/superpowers/audits/2026-07-01-harness-deep-audit.md`, Component 2 —
Tools, HIGH at `shell.rs:74; fs/read.rs:36-38; git.rs:85; mcp/tool.rs:91-125;
skills/tools.rs:306-308`; Top-10 fix #7), plus two folded build opportunities:
`read_file` offset/limit pagination (Component 2) and `context_recall`
pagination (forced by coherence, see Decisions). Anchors re-verified against
live `main` (`1784a60`) on 2026-07-01.

## Invariant

No tool-result message inside any built model request exceeds
`max_tool_result_bytes`. An oversized result is stored **whole** in the
offload store on the same maintenance pass that follows its arrival — before
the next model call — and its context message becomes a bounded preview plus
a recall marker carrying the offload id. Nothing is lost, only moved.

## Findings addressed (verified live)

1. **HIGH — unbounded ingestion at the loop drain.** Every tool result enters
   context at exactly one seam: `loop_.rs:505`
   (`ctx.append(Message::tool(id, name, content))`), on both the success path
   (`output.content`, `loop_.rs:485`) and the error path (`loop_.rs:502`).
   No cap exists anywhere in the loop. Per-tool status at HEAD:
   - `execute_command` — `shell.rs:98-99` formats full stdout+stderr;
     `captures_large_output_without_deadlock` (`shell.rs:165-181`) proves
     >100 KB flows through.
   - `read_file` — `fs/read.rs:55-57` whole-file `read_to_string`, no
     offset/limit params (schema at `fs/read.rs:33-35` exposes only `path`).
   - `git_*` — shared helper `git.rs:24-53` drains stdout unbounded
     (`git_diff` is the realistic large path).
   - MCP proxy — `mcp/tool.rs:100-118` joins all text blocks verbatim.
   - `use_skill` / `read_skill_file` — `skills/tools.rs:123-138, 338-344`
     load full bodies/files (write-side caps exist; read-side none).
   - Bounded already (precedents): `fetch_url` (2 MiB download /
     8 KiB return + `[truncated: N bytes]` marker, `http/content.rs:63-82`)
     and memory `recall` (`max_recall_chars`, `memory/tools.rs:397-418`).
2. **Offload is age-based only.** `select_offloads`
   (`offload_policy.rs:53-94`) protects the newest `keep_recent = 3` tool
   results unconditionally, so a giant result sits verbatim in the window for
   3+ tool calls — and `OffloadConfig` is never wired from `RuntimeConfig`
   (CLI `main.rs:268`, server `session.rs:68,270` use defaults).
3. **Spine-B MED, partially defused (folded consequence).** After cluster 5,
   a single oversized tool result forms an oversized turn-unit that the
   keep-≥1-unit floor must keep, guaranteeing an over-limit request. Capping
   ingestion bounds every tool message, so tool results can no longer create
   that state. The other half of the MED (compact-and-rebuild once on a
   context-overflow model error) remains open in the backlog.

Context confirmed during verification:

- `ctx.maintain(&deps)` runs at `loop_.rs:514`, immediately after the Phase-3
  appends and **before any subsequent model call** (`build()` at the top of
  the next iteration). A cap applied in `maintain` therefore yields the same
  model-visible guarantee as a cap at the loop drain.
- The loop does NOT hold the offload store; `CuratedContext` and
  `context_recall` share it via `LoopParts.offload_store` (`assemble.rs:30`).
- `AgentEvent::ToolResult` is emitted (with full output) *before* the append —
  observability and the JSONL trace keep full fidelity regardless of the cap.
- Production contexts are `CuratedContext` everywhere; `WindowContext` is a
  test/e2e fallback (cluster-5 precedent: it stays out of scope).
- `context_recall` (`context_tools.rs:54-72`) returns full stored content by
  design — unbounded re-ingestion by another name.

## Decisions

1. **Placement: eager pass as step (0) of `CuratedContext::maintain`,** not a
   literal cap at the loop drain and not N per-tool fixes. Rejected:
   per-tool caps (duplicated logic, misses MCP and every future tool — the
   audit's own argument); loop-drain cap (requires plumbing the offload store
   into `AgentLoop` and re-implementing placeholder/event/report logic that
   already lives in `curated.rs`/`offload_policy.rs`, for an identical
   model-visible guarantee). The raw string exists briefly in `history`
   between append and maintain, but no request is built in that interval.
2. **Truncate + eager offload with recall id** (audit's suggested shape), not
   lossy truncation à la `fetch_url`: the full content goes into the offload
   store; the window keeps a head preview + marker. The model sees something
   useful immediately and can page through the rest.
3. **Fold in `read_file` offset/limit** (line-based) — the natural affordance
   once whole-file reads are bounded, and an explicit audit build
   opportunity.
4. **Fold in `context_recall` pagination** (byte-offset) — without it the cap
   is incoherent: recalling a giant entry either re-floods the window or,
   once capped, could never reach past the first chunk.
5. **Uniform cap, no default exemptions.** `OffloadConfig.exclude_tools`
   already exists and the eager pass honors it (escape hatch, e.g. for
   `use_skill` if a >cap skill body must arrive whole), but the default list
   stays empty: the marker makes truncation visible and recall preserves
   access.
6. **Reuse `ContextEvent::Offloaded`** for the eager pass — id/bytes/tool
   describe exactly what happened; no new wire/web/CLI plumbing. Eager
   offloads also fold into `MaintReport.offloaded{,_bytes}`.
7. **Default `max_tool_result_bytes` = 16 KiB** (≈4 K tokens): large enough
   for real command output and mid-size files, small enough that even an
   8 K-token window survives one result. Tunable via `RuntimeConfig`.

## Section 1 — Eager selection + marker (`agent-core/src/offload_policy.rs`)

- `OffloadConfig` gains `max_result_bytes: usize` (doc: eager cap — any tool
  result larger than this is offloaded at ingestion regardless of age).
  Default = new `pub const DEFAULT_MAX_TOOL_RESULT_BYTES: usize = 16 * 1024;`.
- New `pub fn select_oversized(history: &[Message], config: &OffloadConfig)
  -> Vec<OffloadHit>` — pure/deterministic like `select_offloads`: every
  `Role::Tool` message with `content.len() > config.max_result_bytes`, in
  history order, skipping excluded tools and already-offloaded placeholders
  (`starts_with(PLACEHOLDER_PREFIX)`). **No `keep_recent` protection** — age
  is irrelevant to size. Reuses `classify` for the entry kind.
- New `pub fn truncation_marker(id: OffloadId, tool_name: &str, shown: usize,
  total: usize) -> String`:

  ```text
  \n[tool_result#{id} truncated: showing first {shown}B of {total}B from "{tool_name}" — continue with context_recall(id: {id}, offset: {shown})]
  ```

- New preview helper (private or `pub(crate)`): given `content` and `cap`,
  compute the preview budget as `cap` minus the rendered marker length
  (marker rendered with worst-case digit widths, i.e. `shown = total`), then
  truncate at a `char` boundary (the `is_char_boundary` walk-back pattern
  from `http/content.rs:63-82`). **Resulting message (preview + marker) is
  ≤ cap**, so the pass is idempotent: trigger is `len > cap`, output never
  re-triggers. `cap` smaller than the marker degrades to marker-only
  (saturating arithmetic), which is still bounded and recallable.

## Section 2 — Eager pass in `CuratedContext::maintain` (`agent-core/src/curated.rs`)

New step **(0)**, before the existing age-based offload step (a):

```rust
// (0) Ingestion cap — a fresh oversized result never reaches a model call.
for hit in select_oversized(&self.history, &self.config) { ... }
```

Per hit: store the FULL content (`self.store.put(hit.entry)`), replace
`history[idx].content` with `preview + truncation_marker(id, ...)`, bump
`report.offloaded` / `report.offloaded_bytes`, emit
`ContextEvent::Offloaded { id, bytes, tool }` — the same body as step (a),
extracted into a small private helper both steps call rather than duplicated.

Interplay with step (a), documented + tested, not special-cased: a capped
preview (~cap bytes, no longer starting with `PLACEHOLDER_PREFIX`) qualifies
for age-based offload once it leaves the `keep_recent` window, storing the
preview as a second small entry whose content still carries the marker to the
full entry. Lifecycle: fresh = preview + marker → stale = one-line
placeholder. The duplicate store entry is ≤ cap bytes and keeps recall
chains intact.

Role/`tool_call_id` are untouched (same as step (a)), so turn-atomicity and
the orphan invariant from cluster 5 are unaffected. `WindowContext` stays
uncapped (test-only fallback, no store).

## Section 3 — `context_recall` pagination (`agent-core/src/context_tools.rs`)

- `ContextRecallTool` gains a page budget: `context_tools(store, flag)`
  becomes `context_tools(store, flag, recall_page_bytes: usize)`; callers
  pass `max_tool_result_bytes` (assemble.rs is the only production call
  site, `assemble.rs:102`).
- Schema gains optional `offset` (integer ≥ 0, byte offset into the stored
  content, default 0) with a real description (contract ratchet).
- `execute`: slice `entry.content` from `offset` (walked back to a char
  boundary) for at most the page budget **minus** the continuation-marker
  length, so total output ≤ page budget and a recall page can never itself
  trip the ingestion cap. If content remains, append:

  ```text
  \n[bytes {start}–{end} of {total} — continue with context_recall(id: {id}, offset: {end})]
  ```

  `offset >= total` → `InvalidArgs` naming the valid range. Small entries
  (≤ budget) return verbatim, keeping the existing
  `recall_returns_full_content` behavior.
- Tool description updated to mention paging.

## Section 4 — `read_file` offset/limit (`agent-tools/src/fs/read.rs`)

Optional params, both described in the schema:

- `offset` — 1-based line number to start from (default 1).
- `limit` — max lines to return (default: all).

When either bounds the output, the content is a single header line
`[lines {first}–{last} of {n}]` followed by the sliced lines. `offset` past
EOF → `InvalidArgs` naming the file's line count.
Whole-file default behavior is unchanged for existing callers/tests. The
ingestion cap remains the byte-level backstop for pathological single lines.

## Section 5 — Config surface (`agent-runtime-config`)

- `RuntimeConfig` gains `max_tool_result_bytes: usize` with
  `#[serde(default = "default_max_tool_result_bytes")]` returning
  `agent_core::DEFAULT_MAX_TOOL_RESULT_BYTES`; mirrored in
  `PartialRuntimeConfig` + the on-disk merge, like `trace_max_mb`.
- `assemble.rs`: pass `cfg.max_tool_result_bytes` to `context_tools(...)`.
- Frontends wire the cap into their `CuratedContext` (the config is otherwise
  never applied — verified gap):
  - `agent-cli/src/main.rs` (~268) and `agent-server/src/session.rs`
    (~68, ~270): `.with_offload_config(OffloadConfig { max_result_bytes:
    cfg.max_tool_result_bytes, ..OffloadConfig::default() })`.

No CLI flag; the persisted config file is the tuning surface (matches
sandbox/trace knobs).

## Error handling & edge cases

- **Error results** (`ERROR: ...` content) are capped identically; `classify`
  marks the stored entry `OffloadKind::Error`.
- **Parallel tool calls:** several oversized results in one turn each get
  their own entry/preview on the same maintain pass.
- **Multi-byte UTF-8:** all truncation points walk back to char boundaries
  (preview, recall slice); no panics on CJK/emoji output.
- **`context_recall` of a capped entry** returns pages ≤ its budget by
  construction — no re-cap loop, no unreachable content.
- **Oversized `use_skill` body:** capped like everything else; the marker is
  visible and the body recallable. `exclude_tools` is the escape hatch if a
  deployment needs whole-body delivery.
- **Cap disabled/tuned:** a huge `max_tool_result_bytes` effectively disables
  the eager pass (trigger is strictly greater-than); no separate off switch.
- **Store growth:** entries are already RAM-only and session-scoped; the
  eager pass adds at most one full entry per oversized result (plus the ≤cap
  preview entry when later age-offloaded). Persisted/capped stores remain a
  separate backlog item (Spine-B low).

## Testing

- **`offload_policy.rs`:** `select_oversized` (oversized fresh result
  selected despite `keep_recent`; at-cap not selected; placeholder skipped;
  excluded tool skipped); preview+marker length ≤ cap incl. worst-case
  digits; char-boundary truncation on multi-byte content; idempotence
  (capping the capped output selects nothing); marker format carries id,
  totals, and the `context_recall(id:, offset:)` hint.
- **`curated.rs`:** maintain caps a fresh oversized tool result on the same
  pass — history message ≤ cap, store holds full content, `Offloaded`
  emitted, report counts it; second maintain is a no-op; later passes
  age-offload the preview to a placeholder and both store entries recall
  correctly; `tool_call_id`/role preserved (orphan checker still empty);
  build() under a small `model_limit` no longer carries an over-cap unit.
- **`context_tools.rs`:** paging (default offset returns first page +
  continuation marker; following the marker's offset walks to the end; total
  output ≤ budget each page; small entry verbatim with no marker;
  `offset >= total` → InvalidArgs; char-boundary slicing); updated
  `context_tools` signature; schema param described.
- **`fs/read.rs`:** offset/limit slicing + header line; defaults unchanged;
  offset past EOF error; schema params described.
- **`agent-runtime-config`:** serde default + partial-merge for
  `max_tool_result_bytes`; `assemble_loop` still registers `context_recall`.
- **Unchanged by design:** `shell.rs` `captures_large_output_without_deadlock`
  (tools still return full output; the cap is a context measure) and
  `AgentEvent::ToolResult` full-fidelity emission.
- Existing `e2e_context_management` / `stress_context_management` suites and
  the full `bash scripts/ci.sh` gate stay green.

## Files touched

- `agent/crates/agent-core/src/offload_policy.rs` — `max_result_bytes`,
  `DEFAULT_MAX_TOOL_RESULT_BYTES`, `select_oversized`, `truncation_marker`,
  preview helper; tests.
- `agent/crates/agent-core/src/curated.rs` — maintain step (0), shared
  offload-application helper; tests.
- `agent/crates/agent-core/src/context_tools.rs` — recall paging + signature;
  tests.
- `agent/crates/agent-tools/src/fs/read.rs` — offset/limit; tests.
- `agent/crates/agent-runtime-config/src/runtime_config.rs` + `assemble.rs` —
  config field, merge, `context_tools` call; tests.
- `agent/crates/agent-cli/src/main.rs`, `agent-server/src/session.rs` —
  `.with_offload_config(...)` wiring.
