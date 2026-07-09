# Backend seam / virtual filesystem (deepagents refactor, Phase 2) — design

**Status:** APPROVED at spec-review gate 2026-07-08 (panel-reviewed;
escalations E1–E6 all resolved — decisions inline in §6 and in the Panel &
review log). **Governing goal, stated by the owner at the gate:** the
refactor's purpose is deepagents-style modularity — custom middleware,
filesystems, and extensions can be written to change runtime behavior. The
seam itself is the deliverable; dispositions below are argued against that
criterion. Next: implementation plan.
**Knowledge base:** `docs/okf/deepagents-refactor/` (commit d997eec). Design
judgments in `comparisons/refactor-priorities.md` are *unvalidated input* —
including the Phase-2/3/4 partition itself; the panel examined it as a claim
and the gate resolved it at E1.
**Live-source baseline:** commit a68e721, re-read 2026-07-08. All `file:line`
anchors are orientation only — locate quoted code by content before editing.
**Builds on:** Phase-1 middleware seam
(`docs/superpowers/specs/2026-07-08-middleware-seam-design.md`, merged
707d7fd) — this spec designs ON that seam and preserves its invariants (§3).

## 1. Problem

File access, context offloading, and evicted history are three unrelated
storage schemes today:

- File tools (`read_file`/`write_file`/`edit_file`/`list_directory`,
  `agent-tools/src/fs/`) operate directly on the host workspace via
  `resolve_in_workspace`. There is no seam to redirect file access anywhere
  else — no scratch space, no store-backed memory files later, no
  sandbox-backed files ever.
- Oversized/stale tool results are lifted into an id-keyed `OffloadStore`
  side table (`offload.rs`) and recovered through a bespoke `context_recall`
  tool with its own byte-paging grammar (`context_tools.rs`) — a second
  recall mechanism the model must learn alongside `read_file`.
- Compaction *destroys* the summarized span: `compact_old_span`
  (`curated.rs`) replaces assistant/tool chatter with the summary and the
  original bytes are gone — the one context artifact that is not
  agent-recoverable at all.

deepagents demonstrates the alternative (bundle:
`practices/filesystem-as-context-substrate.md`): one `BackendProtocol`
behind every file tool, with composite prefix routing, and *all*
context-management artifacts living on it as ordinary files —
`large_tool_results/{tool_call_id}` for evicted tool results,
`conversation_history/` for summarized-away history — recovered with the
file tools the model already knows. One recall grammar, one permission
surface, one portability seam.

## 2. Goals

- G1. A `Backend` trait in `agent-tools` with
  `ls / read / write / edit / glob / grep / delete` and **structured errors**
  (`FsError` enum) — hardening deepagents' error-return convention (now a
  documented directive in the live docs, though still structural-typing
  there) into a compile-time contract.
- G2. Three implementations: `HostBackend` (real disk rooted at the
  workspace, preserving today's symlink-safe containment),
  `MemBackend` (in-process map, session-scoped), and `CompositeBackend`
  (longest-prefix routing + `ls`/`glob`/`grep` aggregation).
- G3. File tools migrated onto the backend: the existing four become thin
  adapters over `ToolCtx.backend`, plus a new `grep` tool — the search half
  of the recovery surface that replaces `context_recall`. (An agent-facing
  `glob` tool was cut by the panel: placeholders cite exact paths, so
  recovery never needs discovery-by-pattern; the trait keeps the op. J8.)
- G4. `OffloadStore` folded into files: `CuratedContext` writes lifted tool
  results to `large_tool_results/…` and evicted spans (folded user units and
  compacted chatter) to `conversation_history/history.md` on the artifacts
  backend; placeholders, the ledger, and the compaction summary point at
  paths instead of ids. **The artifacts namespace is read-only to
  model-originated operations** (§5.2) — curation writes through a
  privileged handle, preserving today's exact integrity guarantee that no
  model action can forge or destroy offloaded content.
- G5. `context_recall` retired (superseded pins and every consumer of the
  retired surface enumerated in §5.6/§7). `context_compact` survives
  unchanged. *(Retirement ratified at the gate, E2.)*
- G6. Dispatch children get their own artifacts namespace (fresh
  `SessionArtifacts`, child-distinct artifact names) over the same workspace
  mount — the file-tools analog of today's per-child `InMemoryOffloadStore`.
- G7. Curation **selection logic** is unchanged given identical inputs:
  the offload/fold/compaction triggers, thresholds, `keep_recent`,
  high-water, monotone-prior guard, trivial-chatter skip, and all-or-nothing
  fold keep their exact shape. Honest scope (panel finding): the `read_file`
  source cap (§5.4) changes the *input distribution* — an oversized read
  result no longer exists to be selected by `select_oversized`, so offload
  event counts shift on read-heavy sessions — and the compaction
  write-failure posture is a declared change (§5.5, E4). "Bitwise cadence"
  claims are scoped to the selectors, not the whole pipeline.

### Non-goals (later phases, per bundle sequencing — E1 escalates the partition)

- **No sandbox-as-backend** (execute-derived fs ops) — scoped OUT as a spec
  judgment, see J2.
- **No multimodal file support** — the previously UNASSESSED gap is now
  assessed and explicitly excluded, see J3.
- No skills or memory files on the backend (Phase 4, with the memory
  judgment call); `read_skill_file` and the vector store are untouched.
- No agent-facing `glob` or `delete` tools (J8); the trait carries both ops
  per the protocol shape, but nothing exposes them to the model in Phase 2.
- No `execute` tool on backends, no `StoreBackend`/cross-session artifact
  persistence, no `virtual_mode` toggle (containment is always on).
- No change to `SandboxStrategy`, shell/git tools, MCP, policy engine
  internals, or the memory subsystem (one prose/test exception: the memory
  `recall` tool's disambiguation text names `context_recall` and must be
  rewritten, §5.6).

## 3. Do-not-regress invariants

Gap-analysis keep-invariants plus Phase-1 seam invariants:

| Invariant | How this design preserves it |
|---|---|
| Goal block + folded-facts ledger | Fold mechanics untouched (§5.5); the ledger's citation changes from `context_recall(id)` to `conversation_history/history.md` + a per-batch `## folded-{seq}` section anchor, keeping per-batch granularity (declared change, §5.5) |
| `ToolIntent` policy richness | File tools keep their intents; the new `grep` ships an `Access::Read` intent with its pattern/scope; `RulePolicy` gating unchanged. For artifact-prefix paths the policy check is honest-but-decorative: containment there is enforced by the backend itself (reads route to `MemBackend`, which has no escape surface; writes are refused outright) — see §5.9 |
| Offload-content integrity (implicit today: the store is model-unreachable) | Preserved explicitly: artifact prefixes are read-only to model-originated ops (§5.2); only curation's privileged handle writes them |
| Refusal-on-degraded sandbox | `SandboxStrategy` and its surfacing untouched; shell/git still execute through it |
| First-class MCP | Untouched |
| Calibrated token estimation | Untouched — curation still sees limits only via `RunCx` accessors |
| Middleware hook firing set (Phase 1) | `ContextCurationMiddleware` still acts only in `on_turn_end`/`after_final_reply`; no new hooks, no firing-set changes |
| `Maintained` marker semantics (Phase 1) | Unchanged |
| Child-stack composition `[curation, stuck]` (Phase 1) | Unchanged; the child curation instance binds to the child's artifacts backend instead of a child store (§5.7) |
| Parity-pin suite (Phase 1) | All stack-mechanics and cadence pins keep passing with assertion bodies unchanged; pins on the *retired* surface are superseded and enumerated in §7 |
| Caller-owned handle survival (LoopParts) | The session-stable artifacts backend replaces `offload_store` as the caller-owned handle with the same survival contract across loop rebuilds (§5.3) |

## 4. Alternatives considered

**A — Backend via `ToolCtx`, artifacts in caller-owned `MemBackend`s
(chosen; the artifacts-location half was ratified at the gate, E3).** The
backend handle rides `LoopConfig → ToolCtx` exactly like `sandbox` already
does; file tools stay stateless structs; children get a different composite
simply by the child loop carrying a different backend. Curation writes
artifacts through the same stores via privileged handles.

**B — deepagents-faithful `FilesystemMiddleware`** shipping backend-bound
tool instances as contributions. Rejected: with no node/wrap hooks it is a
tool bundle wearing a middleware costume, and construction-bound tools break
child routing — the base snapshot would hand children tools bound to the
*parent's* composite, so a child could never read its own artifacts;
fixing that means per-child tool re-instantiation and snapshot surgery.
`ToolCtx` is the loop-scoped dependency channel this runtime already uses
for exactly this shape (`sandbox`).

**C — Keep `context_recall`/`OffloadStore`, back it with files.** The
panel's scope reviewer showed the original rejection ("two recall grammars
during the transition") was circular — the second grammar only exists
because retirement introduces it. The honest statement: keeping the tool is
the materially smaller change (no read_file paging work, no wire change,
no consumer sweep), and the *reason* to retire anyway is the mandate's
uniformity goal (one recall grammar the model already knows, per the
deepagents evidence) — a goal, not a technical necessity. **Escalated to
the gate as E2** rather than recorded as a settled technical rejection.

**D — Host-disk artifacts** (session directory on disk instead of
`MemBackend`). The panel strengthened this alternative: it deletes the
`MemBackend`-for-artifacts routing, the reserved-prefix shadowing rules,
and the shell-coherence gap (sandboxed commands could grep artifacts) in
one stroke, at the cost of session litter/cleanup and losing
nothing-persists parity. The original rejection under-priced what
MemBackend+Composite carry. **Escalated to the gate as E3; resolved
MemBackend (§6).** Note the three
G2 implementations are mandated and get built either way; E3 only decides
where the *artifacts* route. The read-only-to-model guard (§5.2) applies
under either choice.

## 5. Design

New module: `agent/crates/agent-tools/src/backend.rs` (trait + errors +
`HostBackend` + `MemBackend` + `CompositeBackend` + the read-only guard).
`agent-core` already depends on `agent-tools` (verified: `agent-tools` has
zero `agent-*` dependencies, so no cycle), so `CuratedContext` can hold the
trait object without new dependency edges.

### 5.1 The trait

```rust
#[async_trait]
pub trait Backend: Send + Sync {
    /// Entries directly under `path`, name-sorted. Directories carry is_dir.
    async fn ls(&self, path: &str) -> Result<Vec<Entry>, FsError>;
    /// Whole-document read. Non-UTF-8 content is FsError::NotUtf8 (J3).
    /// (Deliberate divergence: deepagents' protocol read is paginated;
    /// this trait keeps the storage op whole-document and puts paging in
    /// the tool layer, §5.4.)
    async fn read(&self, path: &str) -> Result<String, FsError>;
    /// Create or overwrite, creating parents.
    async fn write(&self, path: &str, content: &str) -> Result<(), FsError>;
    /// Replace `old` (must occur exactly once) with `new`.
    /// Returns before/after so tools can render diffs. Provided default:
    /// read → check uniqueness → replacen → write. Backends may override.
    async fn edit(&self, path: &str, old: &str, new: &str)
        -> Result<Edited, FsError> { /* default impl */ }
    /// Paths matching a glob pattern (relative to the backend root).
    /// No agent-facing tool in Phase 2 (J8); kept per the protocol shape.
    async fn glob(&self, pattern: &str) -> Result<Vec<String>, FsError>;
    /// Regex search. `path` scopes to a file or prefix; None = everywhere.
    /// Hits are (path, line_no, line) and the result set is capped (§5.4).
    async fn grep(&self, pattern: &str, path: Option<&str>)
        -> Result<Vec<GrepHit>, FsError>;
    /// No agent-facing tool in Phase 2 (J8).
    async fn delete(&self, path: &str) -> Result<(), FsError>;
}

pub struct Entry { pub name: String, pub is_dir: bool }
pub struct Edited { pub before: String, pub after: String }
pub struct GrepHit { pub path: String, pub line: usize, pub text: String }

#[derive(Debug, Clone, thiserror::Error)]
pub enum FsError {
    NotFound(String),
    /// Containment violation (path escape / symlink escape) OR a
    /// model-originated mutation of the read-only artifacts namespace —
    /// maps to ToolError::Denied, preserving today's exact message strings
    /// for the containment cases.
    Denied(String),
    /// Exists but is not valid UTF-8 (binary): the honest error the current
    /// read_to_string path mislabels as NotFound (J3).
    NotUtf8(String),
    /// `edit` old-string matched 0 or >1 times (count carried in message).
    EditConflict(String),
    InvalidPath(String),
    Io(String),
}
```

(`FsError::Unsupported` was cut by the panel: with Host/Mem/Composite only,
it has no caller; the sandbox backend that would need it is out of scope,
J2.) Locked-across-await note for implementers: `MemBackend`'s interior
`Mutex` is a plain `std::sync::Mutex` whose guard must not be held across
an `.await` — its ops are synchronous under the hood and release before
returning.

Structured errors are the contract deepagents states as a directive but
cannot compile-enforce (Python `Protocol`). In Rust it is `Result` or
nothing; tools map `FsError → ToolError` in one place, preserving today's
user-visible strings where tests pin them
(`"path escapes workspace: {arg}"`, `"`old` matched {n} times"`, …).

Paths are `&str` virtual paths, not `Path`: they are model-provided strings
routed by prefix; only `HostBackend` ever touches the OS path type. No
host-fs assumption leaks into the trait, so an execute-derived sandbox
backend stays implementable later (J2). **Mount-location transparency
(E6):** a backend always receives paths relative to its own root — the
composite strips the mount prefix on the way in and re-prefixes result
paths (`ls`/`glob`/`grep` hits) on the way out. A custom backend written
against root-relative paths can therefore be mounted at any prefix without
modification; it never needs to know where it lives.

### 5.2 The three backends + the artifacts guard

**`HostBackend { root: PathBuf }`** — today's behavior relocated:
every op resolves through `resolve_in_workspace(root, path)`
(`fs/paths.rs` moves intact — symlink chasing, dangling-link rejection,
lexical normalization; its test suite keeps passing unchanged). `read` maps
`InvalidData` io errors to `NotUtf8` instead of the current
everything-is-NotFound mapping (declared change, J3). `glob`/`grep` walk
the root; the skip-set is exactly `.git/` (a stated default, not plan
whimsy), and reserved-prefix artifacts are never subject to any skip-set.

**`MemBackend`** — `Mutex<BTreeMap<String, String>>` keyed by
mount-relative path (E6 strip semantics). `BTreeMap` gives sorted
`ls`/`glob` for free. Unbounded, session-scoped, cheap to share by `Arc` —
the parity replacement for `InMemoryOffloadStore` (verified genuinely
unbounded today; parity, J5).

**`CompositeBackend { mounts: Vec<(String, Arc<dyn Backend>)>, default: Arc<dyn Backend> }`**
— route by **longest matching path prefix**, else `default`. Per E6 (gate
decision, reversing the panel-era draft): the composite **strips the mount
prefix** before delegating and **re-prefixes result paths** on the way out,
matching live deepagents' `backends/composite.py` (panel-verified) and —
the deciding argument under the modularity goal — keeping every mounted
backend mount-location-transparent. The draft's no-strip contract would
have made every future custom backend mount-aware; retrofitting stripping
after third-party backends exist would break their contract, so it lands
now, while zero backends exist. Two consequences the strip forces:
(1) `MemBackend` keys mount-relative paths, and (2) the two artifact
prefixes get **two separate `MemBackend` instances** — a single instance
mounted twice would merge namespaces after stripping (a `grep` under
`large_tool_results/` would surface `history.md` re-prefixed under the
wrong mount). The default mount is unstripped: `HostBackend` sees
workspace-relative paths exactly as today. `ls`/`glob`/`grep`
**aggregate**: the default's results union'd with every mount whose prefix
intersects the queried scope, re-prefixed per mount, deduped — and never
cross-namespace (pinned: a grep scoped to one prefix never surfaces
another mount's files).

**`ReadOnlyToTools` guard (new, load-bearing).** The composite's artifact
mounts wrap the two artifact `MemBackend`s in a guard that rejects `write` /
`edit` / `delete` with
`FsError::Denied("large_tool_results/ and conversation_history/ are
read-only records of offloaded context")`. Curation holds the **unwrapped**
`SessionArtifacts` handles and writes directly; tools only ever see the guarded
composite. Why this is load-bearing (panel blockers 1–2, Failure & abuse):
placeholders and the compaction pointer vouch for the provenance of what
they point at, and today's `OffloadStore` is unreachable by any model
action. Without the guard, one `write_file` into a placeholder's target —
by a confused model or a prompt-injected one — forges or destroys the only
copy of an evicted result while the placeholder still asserts it is
original tool output, with no trace. The guard preserves today's integrity
invariant exactly and makes §5.4's recoverability claim actually true.

**Reserved prefixes and shadowing:** the standard assembly mounts the
guarded artifacts backend at `large_tool_results/` and
`conversation_history/`. A real workspace directory with either name
becomes unreachable through file tools. Mitigations: `assemble_loop` logs a
one-time warning when the workspace contains a real entry with a reserved
name (diagnosable, not mysterious), and the tool prose documents the
reservation.

### 5.3 Handle plumbing — and the survival property

- `LoopConfig` gains `backend: Arc<dyn Backend>`; `ToolCtx` gains
  `backend: Arc<dyn Backend>` — built per call exactly where `sandbox`
  already is (`loop_.rs` ToolCtx construction; panel-verified the mirror is
  exact). `ToolCtx.workspace` stays: shell/git tools still need the OS path.
- `LoopParts.offload_store` is **replaced** by
  `artifacts: Arc<SessionArtifacts>`, a small struct holding the two
  per-mount stores (`results: Arc<MemBackend>`, `history: Arc<MemBackend>`
  — two instances per E6's strip semantics; concrete, not `dyn`, because
  curation needs the unwrapped privileged handles; the composite wraps them
  for tools). The survival contract carries over verbatim in the doc
  comment: *the caller owns it and passes the SAME handle across loop
  rebuilds (server settings change), so the conversation's offloaded
  artifacts survive.* `compact_flag` is unchanged. `assemble_loop` composes
  `CompositeBackend { [large_tool_results/ → ReadOnlyToTools(results), conversation_history/ → ReadOnlyToTools(history)], default: HostBackend(parts.workspace) }`
  fresh per assemble — the composite is derived state; only the artifacts
  handle is identity-bearing. Curation's privileged writes use
  mount-relative keys (`{seq}-{call}` into `results`, `history.md` into
  `history`); every string the *model* sees (placeholders, pointers, the
  ledger) carries the full virtual path.
- Honest blast-radius note (panel): `CuratedContext::new`'s middle
  parameter changes **type** (`Arc<dyn OffloadStore>` → `Arc<SessionArtifacts>`)
  — a breaking signature change at every construction site (assemble,
  dispatch, server runtime/session, CLI, and the test constructors in
  `curated.rs` / `compaction_routing.rs` / `soak_live.rs`), not a cosmetic
  arity note.
- Frontends: `agent-cli` builds one `SessionArtifacts` where it builds the
  store today; `agent-server`'s `Runtime` field and `offload_store()`
  accessor become the artifacts handle (same rebuild paths in
  `session.rs`).
- `ContextCurationMiddleware::new(artifacts, flag, max_result_bytes)` —
  same shape, store swapped for the privileged artifacts handle.

### 5.4 File tools on the backend

The four existing tools become adapters over `ctx.backend`, byte-identical
in output for every case that succeeds today, with two declared changes:

1. **`read_file` output is bounded by the ingestion cap, with a
   first-class paging contract.** The contract (this is normative, not plan
   detail — panel demanded the grammar be pinned):
   - Under-cap files: whole file by default, byte-identical to today
     (existing pins keep passing). Line-mode `offset`/`limit` params keep
     today's exact semantics (`offset` is ALWAYS a 1-based line number).
   - Over-cap output is truncated at `max_result_bytes` with a
     continuation marker. Normal multi-line files use the line-mode marker
     `[lines A–B of N — continue with read_file(path, offset: B+1)]`.
   - **Byte mode** (the `context_recall` paging machinery relocated): a new
     optional `byte_offset` parameter — a *distinct* param, never
     unit-punning `offset` — returns a raw byte slice from that offset,
     char-boundary-snapped on both ends exactly as `context_recall` and
     `capped_preview` do today, with **no `[lines …]` header**, and the
     marker `[bytes A–B of N — continue with read_file(path, byte_offset:
     B)]`. Continuation markers for artifact recovery and for
     single-line-over-cap content always emit byte mode. The normative
     property, pinned by a ported exact-bytes reassembly test: **following
     byte-mode markers from offset 0 reassembles any file exactly,
     byte-for-byte** — which is the capability bar `context_recall` sets
     and the retirement depends on. The char-boundary pins
     (`recall_slices_on_char_boundaries`,
     `capped_preview_respects_char_boundaries`) port as `read_file`
     byte-mode tests, not drop.
   - Reading an artifact path is capped identically; because the cap emits
     a byte-mode marker pointing at the **same** path, recovery is a single
     grammar with no artifact-copy churn and terminates trivially (offsets
     strictly increase toward a fixed total).
2. **Binary files return the honest error** (`NotUtf8` → a clear
   `ToolError`), not a fake `NotFound` (J3).

New tool (`Access::Read` intent carrying its pattern/scope):

- **`grep`** — `pattern` (+ optional `path` file-or-prefix scope); returns
  `path:line: text` hits, result-capped, with tool prose that names
  `large_tool_results/` and `conversation_history/` as the places to search
  offloaded context, and states that shell commands cannot see those
  prefixes (J5 coherence gap) and that they are read-only records.

`grep` is a child-visible base tool like the other file tools. The
agent-facing `glob` tool is cut (J8): placeholders cite exact paths, so
recovery is read+search, never discovery-by-pattern.

### 5.5 Curation writes files

`CuratedContext` holds `artifacts: Arc<SessionArtifacts>` (replacing
`store: Arc<dyn OffloadStore>`) plus a monotone per-instance sequence
counter. **Selection logic is untouched** (G7 as restated). What changes is
the sink:

- **Lift** (`lift()`, both ingestion-cap and age passes): write full content
  to `large_tool_results/{seq}-{sanitized tool_call_id}` (in a child
  context, `{dispatch-prefix}{seq}-…`, §5.7). The window message becomes
  `[tool_result offloaded to large_tool_results/{name}: {bytes}B {kind}
  from "{tool}" — read_file the path, or grep large_tool_results/ to
  search]`. The capped-preview marker points at the artifact path with a
  byte-mode continuation.
- **Placeholder grammar and idempotency (five lockstep sites, panel
  finding).** The idempotency literals stay as narrow as today's:
  selectors skip content starting with the exact literals
  `[tool_result offloaded` or `[tool_result truncated` (two literals
  replacing today's one `[tool_result#`; the earlier draft's loosening to
  bare `[tool_result` is withdrawn — it widened false-positive skips).
  The sites that must move in lockstep: `select_offloads`,
  `select_oversized`, `is_durable_placeholder_unit`, `placeholder_for`'s
  replacement string, and `capped_preview`/`truncation_marker` including
  the **degenerate marker-only output** (cap smaller than the marker),
  which must still start with a skip literal — the pathological-small-cap
  pin ports to the new grammar. Residual accepted (pre-existing in kind):
  a tool result that *echoes* a full placeholder line as its own prefix is
  skipped by the selectors — the same theoretical false positive today's
  prefix has; the read-only guard prevents the echo from ever being
  *stored back* over a real artifact.
- **Fold** (`fold_evicted_users`): verbatim originals **append** to
  `conversation_history/history.md` under a `## folded-{seq}` section; the
  ledger cites `conversation_history/history.md § folded-{seq}` per batch —
  path + section anchor, preserving today's per-batch citation granularity
  (panel finding: a bare rolling-file citation lost the ability to point at
  *which* batch holds a fact). All-or-nothing semantics keep their exact
  shape: extraction failure → nothing written, history intact; a backend
  **write failure aborts the fold the same way** (facts not added, units
  not removed).
- **Compaction** (`compact_old_span`): on a *committed* summary, the
  summarized span (rendered `[role] content`, one message per line — the
  same rendering the summarizer consumed) appends to
  `conversation_history/history.md` under `## compacted-{seq}`. This is the
  headline capability gain: the span deepagents preserves and today's
  runtime destroys. The pinned summary block gains a suffix line rendered
  by `pinned()` — *"Evicted transcripts: conversation_history/history.md —
  grep it for `## folded-N` / `## compacted-N` section headers, then
  read_file from the hit's line offset"* — tracked as a flag on the
  context, **not** stored inside `compaction_summary`: the pointer never
  enters the summarizer, so re-compaction can never paraphrase it away and
  the monotone-prior guard never sees it.
  **Failure posture (E4, escalated):** the spec's default is that a failed
  history write does **not** block the compaction commit (a broken
  artifacts write must not wedge window maintenance; today the span is
  destroyed unconditionally). Honesty guard (panel): on the first failed
  history write, the context sets a permanent `history_incomplete` flag —
  the pointer line thereafter reads *"Evicted transcripts (INCOMPLETE — at
  least one span failed to record): …"* — so the file never silently
  over-promises completeness. The alternative posture (abort the compaction
  like fold does) is the gate's call at E4.
- **Deep-recovery ergonomics (panel finding).** `history.md` grows
  unboundedly (parity with the unbounded store) while `read_file` is
  capped, so recovery of an early span late in a session must not require
  paging the whole file. The pinned recipe — `grep` the section header
  (grep hits carry line numbers) → `read_file(path, offset: hit_line)` —
  is asserted by a test that recovers a mid-file span from a multi-span
  history.md in exactly two tool calls. Rotation/size-bounding is a future
  knob, not Phase 2.
- **Eviction events**: `ContextEvent::Offloaded { id, bytes, tool }`
  becomes `{ path: String, bytes, tool }` (J9); `Compacted`/`Evicted`/
  `CompactionFailed` unchanged.

`offload.rs` (store trait + impl) is deleted; `OffloadEntry` loses its `id`
field and remains the selector currency in `offload_policy.rs`; `folded_ids`
becomes the per-batch section anchors cited by the ledger.

### 5.6 Tool-surface changes — full retired-surface enumeration

The panel required every consumer of `context_recall` / `OffloadStore` /
`Offloaded{id}` to be dispositioned, not just the obvious ones:

| Consumer | Disposition |
|---|---|
| `context_tools.rs` `ContextRecallTool` | Deleted; `context_tools()` ships only `ContextCompactTool` |
| `middleware.rs` `ContextCurationMiddleware::tools()` | Follows (compact only) |
| `offload.rs` (trait + store + tests) | Deleted |
| `dispatch.rs` `IMPLICIT_CHILD_TOOLS` const (`["context_recall", "context_compact"]`) | Drops `context_recall`; the allowlist contract keeps accepting `context_compact`; `grep` is an ordinary base tool needing no implicit-tool entry |
| `dispatch.rs` model-facing schema prose ("context tools (context_recall, context_compact) are always available") | Rewritten for the surviving tool |
| `agent-memory/src/tools.rs` `recall` `when_not_to_call` ("…use context_recall") + its pinned test | Rewritten to disambiguate against `read_file`/`grep`-based offload recovery; test re-pinned on the new prose (silent-rot hazard otherwise) |
| `agent-tools/src/contract.rs` `CONFUSABLE_TOOLS` + cluster doc | `context_recall` removed; decide whether `recall` vs `grep` needs a new cluster entry (plan detail) |
| `agent-server/runtime.rs` `CONTEXT_TOOLS` + `architecture()` classification + test | `context_recall` removed; `grep` classified with the file tools |
| `agent-cli/src/render.rs` `CE::Offloaded { id, … }` match arm | **Compile break** — re-rendered on `path` |
| `agent-server/wire.rs` Offloaded serialization + its pin | `id` → `path` |
| `web/src/state.ts` offloaded render (template literal reads `detail.id`, type-checks as `undefined` — silent break) | Reads `detail.path ?? detail.id` (the fallback keeps pre-Phase-2 session-trace replays rendering sanely) |
| `web/src/components/design/archFixture.ts` (`context_recall` fixture entry) | Updated |
| Tests: `soak_live.rs` (recall-driving soak), `compaction_routing.rs` (store construction), `dispatch_tool.rs`, `e2e_context_management.rs`, `stress_context_management.rs` | See §7 |
| `eval_context.rs` + context-evolve task drivers | **Migrated in wave 2** (E5, gate override of the draft's ignore-and-note posture): the harness compiles and runs against the `read_file`/`grep` recovery grammar. Guard-ceiling *re-measurement* stays parked until a context-evolve campaign resumes (a ceiling is a (config, rate) pair; see memory note) |

Registry surface after Phase 2: base = file tools (4 existing + grep) +
shell/git/artifact/http; middleware contributions = memory tools
(child-visible), `context_compact` (child-invisible). Snapshot-position
invariant from Phase 1 assembly is untouched.

### 5.7 Dispatch children

Where dispatch today builds a fresh `InMemoryOffloadStore` + flag per child
(`dispatch.rs`), it now builds a fresh `SessionArtifacts` + flag, composes
the child composite `{ artifact prefixes → ReadOnlyToTools(child stores),
default: the parent's HostBackend mount }`, and hands it to the child
`LoopConfig` and the child's `ContextCurationMiddleware`. Child artifact
names carry a path-sanitized form of the dispatch prefix the sink
attribution already mints (`sub{n}:` → `sub{n}-`), e.g.
`large_tool_results/sub1-{seq}-{call}` — making a
parent/child name collision structurally impossible. Consequences, pinned
by test:

- Child file tools read the same workspace as the parent (today's behavior —
  file tools were never isolated) and the child's **own** artifacts.
- Parent artifacts are invisible to the child and vice versa. **Panel
  hazard, pinned:** a child's final answer may cite its own
  `large_tool_results/…` paths; those paths look followable to the parent
  in a way opaque `context_recall` ids never did. The name prefixing
  guarantees the parent's read is a clean `NotFound` — never another
  tenant's bytes — and the dispatch tool's result prose states that child
  artifact paths are not parent-resolvable.
- Child stack stays `[ContextCuration, StuckDetection]` (Phase-1 invariant).

### 5.8 Events, wire, explorer

`wire.rs` translates the new `Offloaded { path, … }` shape; §5.6's table
names every renderer (CLI `render.rs` is a compile break; web `state.ts` is
a silent one — the panel caught that `detail.id` merely type-checks to
`undefined`). Pre-Phase-2 session traces replay with the `?? detail.id`
fallback rather than breaking. Snapshot categories are unchanged;
`ArchitectureSnapshot` tool classification updates per §5.6.

### 5.9 Policy interaction

No policy code changes. `ToolIntent.paths` still carry the raw model-given
strings; `RulePolicy` still resolves them with `resolve_in_workspace` for
the read-inside-workspace auto-allow (verified: Read/TrustedWrite
auto-allow on containment; Write always Asks; virtual artifact paths
resolve lexically inside the workspace, so reads auto-allow exactly as
today). Two honest clarifications the panel demanded:

- For artifact-prefix paths the policy check is **decorative**: the op
  never touches the filesystem the check reasons about. The real
  containment guarantees for those prefixes are the backend's — reads hit
  `MemBackend` (no escape surface), mutations are refused by the
  `ReadOnlyToTools` guard *below* the policy layer, regardless of what
  policy would have said. If E3 ever moves artifacts to disk, the
  disk-containment semantics must be re-derived — flagged, not assumed.
- The earlier draft's "nothing there is integrity-load-bearing" claim is
  withdrawn — it was wrong (placeholders vouch for provenance). The guard
  exists precisely because the namespace IS integrity-load-bearing.

The hard containment boundary moves with `resolve_in_workspace` into
`HostBackend` and is enforced for every op including the new
`grep`/`glob`/`delete`.

### 5.10 Implementation staging (panel recommendation, adopted)

Two independently-reviewable waves, so the uncontroversial seam lands
before the contested surface:

- **Wave 1 — pure seam, zero behavior change:** Backend trait + three
  impls + guard; file tools become adapters; `LoopConfig`/`ToolCtx`
  plumbing; whole existing suite green with construction-site-only diffs.
  No curation change, no retirement, no wire change.
- **Wave 2 — the substrate migration:** curation writes files, placeholder
  grammar, `read_file` paging contract + `grep` tool, `context_recall`
  retirement + consumer sweep (§5.6 table), wire/event change, children,
  and the eval-harness migration (E5).

## 6. Judgments and gate escalations

Escalations (panel findings that conflict with or reinterpret the mandate —
none silently adopted or dismissed; **all resolved at the 2026-07-08 gate**
under the owner's stated modularity goal):

- **E1 — Is the backend seam really Phase 2?** The scope reviewer's case:
  Phase 2's one user-facing gain (compaction-span preservation) is buildable
  on the existing store in ~20 lines; everything else is
  uniformity/portability plumbing whose consumers are Phase 3/4.
  **Decision: proceed as specced.** The reviewer priced the phase by
  user-facing payoff; under the modularity goal the pluggable seam *is* the
  payoff — the same shape as Phase 1's J1 resolution (full seam over
  extract-object). §5.10's staging addresses the surviving risk portion.
- **E2 — Retire `context_recall` at all?** The original alt-C rejection was
  circular (§4 C); retirement stands on the uniformity goal, not technical
  necessity. **Decision: retire.** The id-keyed tool couples recovery to
  one store implementation; the file-tool grammar gives every future custom
  backend offload recovery for free. Migration cost is one-time and priced
  in §5.6.
- **E3 — Artifacts on `MemBackend` vs host-disk (§4 D).**
  **Decision: MemBackend (spec default).** Under the modularity lens the
  composite is the extensibility surface, and routing artifacts through it
  gives the mount table an always-exercised consumer from day one — the
  seam lands tested-in-anger rather than idle until Phase 4. The
  shell-coherence gap stays a documented tool-prose note; the disk option
  remains a one-line mount swap later.
- **E4 — Compaction failure posture** when the history write fails.
  **Decision: commit-with-INCOMPLETE-marker (spec default).** Modularity
  raises the stakes in this direction: pluggable backends make write
  failures realistic (disk-full, remote-store timeout), and a mounted
  backend's failure must not wedge window maintenance — otherwise every
  custom backend is a liveness risk to the core loop. The permanent marker
  keeps the pointer honest under any mount.
- **E5 — Eval harness left broken.** The draft mandate said ignore the
  context-evolve harness and note it for later. **Decision: gate override —
  migrate the harness in wave 2** (§5.6, §5.10, §7). Phase 2 changes
  offload behavior (J6's input-distribution shift) while the harness is the
  instrument that measures it, and under the modularity goal the harness's
  long-term role grows (the regression guard when someone swaps a backend).
  Ceiling re-measurement stays parked (memory note
  `context-evolve-needs-backend-migration`).
- **E6 — Composite path contract (raised post-panel at the gate, from the
  modularity goal).** The panel-era draft passed full virtual paths to
  mounts, defended by Phase-2 circumstances. **Decision: strip+remap**
  (§5.1/§5.2) — mount-location transparency is the composable contract for
  third-party backends, it re-aligns with live deepagents, and it is
  cheapest now, before any backend or artifact-name format exists.
  Consequence absorbed: two `MemBackend` instances inside one
  `SessionArtifacts` handle (a single instance mounted twice would merge
  namespaces after stripping).

Judgments (panel-reviewed, held; numbering starts at J2 — the draft's J1,
"the partition is a claim", became escalation E1):

- **J2 — sandbox-as-backend is OUT.** Grounds (Assumptions reviewer
  verified all three): (1) the Docker sandbox bind-mounts the workspace RW
  at `/workspace` — sandboxed commands and host file tools already share
  one filesystem, so an execute-derived fs backend buys zero isolation here
  and adds a docker-exec round trip per file op; (2) deepagents derives
  fs-from-`execute()` because its sandbox providers are *remote* machines
  with their own disks (live docs: fs ops "remain identical", only
  `execute` is added) — a topology this runtime does not have; (3) the
  refusal-on-degraded posture would need re-derivation across seven ops.
  What Phase 2 owes the future: a trait with no host-fs assumptions (§5.1).
  **Gate amendment (modularity goal):** the "no host-fs assumptions"
  guardrail is load-bearing, not courtesy — and the §7 backend contract
  suite is declared a **public conformance surface**: the acceptance test a
  custom-backend author runs against their implementation.
- **J3 — multimodal files: assessed, excluded.** The UNASSESSED gap row is
  resolved: current behavior is text-only end-to-end (`read_to_string`;
  `ToolOutput.content: String`; `Message.content: String`). deepagents'
  multimodal `read_file` returns content *blocks*, which requires a
  message-model redesign across `agent-model`, both tool-call protocols,
  the claude-cli renderer, and the event wire — an own-phase-sized change
  orthogonal to the backend seam. Phase 2 ships the honest structured error
  (`FsError::NotUtf8`), and the gap-analysis row should move to "absent,
  deliberately deferred".
- **J4 — backend handle rides `ToolCtx`,** not middleware-bound tool
  instances (§4 B rationale; `sandbox` is the established precedent for
  loop-scoped tool deps; the ToolCtx mirror was panel-verified). **Gate
  clarification (modularity goal):** the filesystem extensibility point is
  the composite **mount table in `assemble_loop`** — extenders swap or add
  mounts; they do not write a filesystem middleware. Hardcoded mounts are
  correct while all mounts are internal; a config-driven mount table is
  the expected Phase-4 shape (§8).
- **J5 — artifacts live in caller-owned `MemBackend`s** (`SessionArtifacts`
  pair; ratified at E3). Parity with `InMemoryOffloadStore`, same
  one-identity-bearing-handle survival shape. Known coherence gap, accepted and documented in tool
  prose: sandboxed shell commands cannot see the artifact prefixes;
  dissolves under E3's disk option.
- **J6 — uniform offload + source-bounded `read_file`, not deepagents'
  fs-tool eviction exemption.** deepagents exempts seven fs tools from
  result eviction (live-verified). This spec instead bounds `read_file` at
  the source so fs results are small by construction and curation stays
  uniform. Declared consequence (G7 restated): the input distribution to
  `select_oversized` changes, which the broken-by-mandate eval harness
  would otherwise have measured — feeds E5.
- **J7 — one rolling `conversation_history/history.md`** with
  `## folded-{seq}` / `## compacted-{seq}` sections (a deliberate
  divergence from deepagents' per-thread files): a single stable path keeps
  the summary pointer constant. Per-instance file; children write to their
  own mount. Deep-recovery recipe pinned (§5.5). **Gate-added revisit
  trigger:** a store-backed `conversation_history/` mount (per-user/
  per-thread namespacing) would prefer keyed files over one rolling
  append-file — revisit alongside Phase 4's memory judgment (§8).
- **J8 — no agent-facing `glob` or `delete` tools.** `delete`: destructive
  surface with no Phase-2 consumer. `glob` (panel): no role in recovery —
  placeholders cite exact paths. Both trait ops remain (protocol shape;
  `MemBackend` GC and Phase-4 needs).
- **J9 — `Offloaded` event carries `path` instead of `id`.** Wire-format
  change to a coupling hotspot, shipped atomically with every in-repo
  consumer named in §5.6; old traces render via the `??` fallback.
- **J10 — `context_compact` and the compact-flag handle discipline are
  untouched.**

## 7. Testing

**Parity (assertion bodies unchanged):** the Phase-1 stack-mechanics and
cadence pins (`text_only_run_is_curated_at_exit`,
`tool_bearing_run_skips_the_exit_maintain`, hook-order/EndRun/RunState
suite, `child_stack_is_exactly_curation_and_stuck_detection…`); curation
guards (`maintain_is_idempotent`, `keep_recent_protects_newest_tool_results`,
`shrinking_summary_is_rejected_keeping_prior`,
`trivial_assistant_chatter_skips_the_summarizer`,
`explicit_request_bypasses_the_trivial_chatter_skip`,
`tiny_tool_bearing_span_still_compacts`,
`fold_extraction_failure_leaves_history_intact`, `fold_is_noop_when_users_fit`,
`ledger_is_capped_dropping_oldest_lines`, `ledger_rides_inside_the_goal_block`,
`ledger_survives_compaction_untouched`, goal-cap suite); the whole
`fs/paths.rs` symlink/containment suite; policy engine suite; dispatch
suite except the sites named below; `read_file` whole-file default for
under-cap files.

**Superseded pins (behavior deliberately changes — each maps to a §5
decision):**

- `context_tools.rs`: every `context_recall` test — the tool retires (G5).
  The paging-reassembly AND char-boundary properties port to `read_file`
  byte-mode tests (§5.4), they do not drop. `compact_sets_the_flag`
  survives.
- `offload.rs` store tests — module deleted (§5.5).
- `offload_policy.rs` + `curated.rs` assertions on placeholder/marker/ledger
  *strings* — same properties re-asserted against the path grammar,
  including the ported pathological-small-cap degenerate-marker pin
  (§5.5); selection-logic assertions unchanged.
- `e2e_context_management.rs::offload_then_recall_round_trips_through_the_loop`
  → replaced by an offload-then-`read_file` round trip (same exact-bytes
  property).
- `stress_context_management.rs` and `soak_live.rs` recall-driving
  content → re-driven via `read_file` byte-mode paging (the soak harness
  was missing from the earlier draft's list — panel finding).
- `compaction_routing.rs` — construction-site type updates only.
- `dispatch_tool.rs::allowlist_accepts_always_available_context_tools` —
  re-pinned on `context_compact`; `IMPLICIT_CHILD_TOOLS` shrinks (§5.6).
- `agent-memory/tools.rs` recall-prose test — re-pinned on the rewritten
  disambiguation (§5.6).
- `agent-server` `architecture` classification test, `contract.rs`
  name-cluster list, `agent-cli/render.rs` + `wire.rs` Offloaded arms,
  `web` state/fixture — per the §5.6 table.
- `eval_context.rs` — **migrated in wave 2** (E5 gate decision): the
  harness's recall-driving task machinery moves to the `read_file`/`grep`
  grammar so the phase does not land with its measurement instrument
  uncompilable. Ceiling re-measurement remains out of scope (memory note
  `context-evolve-needs-backend-migration` records that debt).

**New tests:**

- Backend contract suite run against all three impls (ls/read/write/edit/
  glob/grep/delete, structured-error cases incl. `NotUtf8`, containment
  `Denied`, `EditConflict`). Per J2's gate amendment this suite is the
  **public conformance surface** for custom-backend authors — written
  generic over `Arc<dyn Backend>` so a fourth implementation runs it
  unmodified. Composite: longest-prefix routing, **strip-on-entry /
  re-prefix-on-exit (E6)** incl. the mount-transparency property (a backend
  mounted at two different prefixes behaves identically), aggregation
  across mounts, **no cross-namespace leak** (a grep scoped to one prefix
  never surfaces another mount's files — the two-MemBackend consequence),
  reserved-prefix shadowing + the assembly-time shadow warning.
- **Guard:** model-originated `write_file`/`edit_file` (and trait-level
  `delete`) against artifact prefixes are `Denied`; curation's privileged
  handle still writes; an artifact's bytes are byte-identical after a
  denied overwrite attempt.
- Tool-over-backend parity: existing four tools byte-identical on under-cap
  host files.
- Offload → placeholder → `read_file` exact-bytes round trip (loop-level);
  byte-mode paged reassembly of a large artifact (ports
  `recall_pages_a_large_entry_to_completion`); byte-mode char-boundary pins
  (ports both boundary tests); a marker chain over an artifact terminates
  (offsets strictly increase).
- Placeholder idempotency under the new grammar: the two skip literals,
  never re-offloaded; degenerate marker-only capped preview never
  re-selected; a large result *echoing* a placeholder line documented as
  the accepted residual (test asserts current selector behavior so a future
  change is a conscious one).
- Compaction writes `## compacted-{seq}` AND the summary pointer appears;
  pointer survives re-compaction verbatim; a failed history write sets the
  permanent INCOMPLETE marker and still commits (E4).
- Fold cites `history.md § folded-{seq}`; fold aborts atomically on backend
  write failure.
- Deep-recovery recipe: grep section header → `read_file(offset:
  hit_line)` recovers a mid-file span from a multi-span history.md in two
  calls.
- Child isolation: child reads its own artifacts; parent `read_file` of a
  child-cited artifact path is `NotFound` (never cross-tenant bytes —
  prefix-named, §5.7); both share workspace files.
- `Offloaded { path }` wire round trip through `wire.rs`; web render uses
  `path ?? id` (old-trace replay).
- **Gate:** `bash scripts/ci.sh` green (includes web typecheck + vitest for
  the wire type change; NOTE: web typecheck alone does NOT catch the
  `state.ts` template-literal break — the render fallback needs a vitest
  assertion).

## 8. Open questions

- The disk-artifacts option (rejected at E3 for Phase 2) remains the
  documented future knob; taking it later reopens: artifacts dir location,
  gitignore, cleanup lifecycle, and re-derived containment (§5.9).
- Config-driven composite mount table (J4 gate clarification): expected
  Phase-4 shape once external mounts exist; hardcoded until then.
- `grep` engine for `HostBackend` (walk + `regex` crate vs shelling to
  `rg`): plan-level; the contract (capped, containment-checked, `.git/`
  skip, artifacts never skipped) is fixed here.
- Whether Phase 4's memory files reuse the artifacts backend or a
  store-backed mount — out of scope; the composite makes either a one-line
  mount change. Paired J7 revisit: a store-backed `conversation_history/`
  mount would prefer keyed per-thread files over the rolling `history.md`.

## Panel & review log

- **2026-07-08 — adversarial spec panel** (4 independent skeptical
  reviewers: Requirements, Assumptions incl. live deepagents drift check,
  Failure & abuse, Scope & simpler design; opus×4). Findings and
  dispositions:
  - **Blockers (fixed in place):** (1) model-writable artifacts namespace
    let one `write_file` forge or destroy the sole copy behind a recovery
    pointer — a genuine integrity regression vs the model-unreachable
    store; fixed with the `ReadOnlyToTools` guard + privileged curation
    handle (§5.2), and §5.9's "nothing integrity-load-bearing" claim
    withdrawn (two Failure findings, one root cause). (2) Retired-surface
    consumer enumeration was incomplete despite claiming exhaustiveness —
    `agent-memory` recall prose + pinned test, `contract.rs`
    CONFUSABLE_TOOLS, `dispatch.rs` IMPLICIT_CHILD_TOOLS + schema prose,
    `agent-cli/render.rs` (compile break), `web/state.ts` (silent
    `undefined` render), `archFixture.ts`, `soak_live.rs`,
    `compaction_routing.rs` all undispositioned; fixed with the §5.6 table
    (found independently by Requirements + Assumptions).
  - **Majors (fixed in place):** `read_file` paging respecified as a
    first-class byte-mode contract with exact-bytes reassembly + ported
    char-boundary pins — a line-based pager provably could not meet
    `context_recall`'s bar (§5.4); placeholder-prefix loosening withdrawn,
    five lockstep sites enumerated, degenerate-marker pin ported (§5.5);
    child artifact names dispatch-prefixed so parent reads of child-cited
    paths are structurally `NotFound` (§5.7); compaction pointer gains the
    permanent INCOMPLETE honesty marker (§5.5); deep-recovery recipe for
    the unbounded `history.md` specified + pinned (§5.5); ledger citation
    gains per-batch section anchors, restoring granularity (§5.5); G7's
    "bitwise" claim restated honestly against the source-cap input-
    distribution change (§2); `CuratedContext::new` change reframed as the
    type/signature break it is (§5.3); composite no-strip corrected from
    "matching deepagents" to a defended divergence — live deepagents
    strips + re-maps (§5.2); `glob` tool cut, `FsError::Unsupported` cut
    (J8, §5.1); two-wave staging adopted (§5.10); policy check on artifact
    prefixes documented as decorative (§5.9).
  - **Escalated to the gate (mandate-conflicting, not silently adopted):**
    E1 phase-partition/own-payoff, E2 retire-context_recall (alt-C
    rejection was circular), E3 MemBackend-vs-host-disk artifacts, E4
    compaction failure posture, E5 eval-harness-left-broken (§6).
  - **Minors (fixed):** trait `read` and `conversation_history` shapes
    reframed as deliberate deepagents divergences; structured-error
    convention wording updated (now a documented directive upstream);
    `.git/` skip-set stated + artifacts-never-skipped; shadow warning at
    assembly; old-trace replay fallback; `offset`-vs-`byte_offset`
    unit-punning eliminated.
  - **Minors (accepted as residual):** placeholder-echo false-positive
    skip (pre-existing in kind, now guard-bounded, asserted by test);
    deepagents' "timestamped sections / media extraction" sub-claims
    unverifiable live — not relied on.
  - **Clean bills:** dependency direction (no cycle), ToolCtx/LoopConfig
    mirror, Docker RW bind-mount (J2 grounds), policy gating of virtual
    paths, store unboundedness parity, no Phase-1-style attribution error
    (BackendProtocol genuinely lives in deepagents), eviction
    thresholds/exemption conventions match the bundle.
- **2026-07-08 — post-panel consistency read** (light tier, per AGENTS.md
  §"Post-gate edits get a consistency read"): stale-language purge,
  cross-section agreement, escalation-label parity, cross-references, and
  status-vs-log all CLEAN; one truncated sentence in §6 E4 fixed; the
  J2-start numbering annotated as intentional.
- **2026-07-08 — spec-review gate (Kalen): APPROVED, all escalations
  resolved.** The owner stated the governing goal at the gate — the
  refactor exists to provide deepagents-style modularity (custom
  middleware/filesystems/extensions change runtime behavior) — and every
  disposition below was argued against that criterion:
  - **E1 proceed as specced** (the seam is the payoff; same shape as
    Phase 1's J1). **E2 retire `context_recall`** (file-tool grammar gives
    any future backend recovery for free). **E3 MemBackend artifacts** (the
    composite gets an always-exercised consumer from day one). **E4
    commit-with-INCOMPLETE-marker** (a pluggable backend's failure must not
    wedge maintenance). **E5 gate override of the draft mandate: eval
    harness migrated in wave 2** (the phase must not land with its
    measurement instrument uncompilable); ceiling re-measurement stays
    parked.
  - **E6 (new at the gate): composite switched to strip+remap** — the
    panel-era no-strip contract was a Phase-2 simplification that would
    have made every third-party backend mount-aware; strip+remap restores
    mount-location transparency and re-aligns with live deepagents.
    Absorbed consequence: `SessionArtifacts` holds two `MemBackend`s (one
    per prefix) since stripping would merge a shared instance's namespaces.
  - **Judgment amendments:** J2 + §7 — the backend contract suite is a
    public conformance surface for custom-backend authors; J4 — the
    extensibility point is the `assemble_loop` mount table (config-driven
    table expected Phase 4, §8); J7 — store-backed keyed-files revisit
    trigger recorded (§8). J3/J5/J6/J8/J9/J10 held as written (J6 and J9
    additionally reinforced by the modularity lens).
- **2026-07-08 — post-decision consistency read** (light tier): E6 ripple
  (strip+remap / mount-relative keys / two-store SessionArtifacts), E5
  ripple, E1–E6 §6-vs-log coherence, child naming under strip semantics,
  and §8 dangling references all CLEAN; one stale E4 hedge in a §7 test
  description fixed, plus the `sub{n}:` → `sub{n}-` sanitization made
  explicit in §5.7.
- 2026-07-08 — implementation notes (plan/build): (1) `SessionArtifacts`
  fields shipped as `Arc<dyn Backend>` rather than concrete `Arc<MemBackend>`
  — still the privileged unwrapped handles §5.3 requires, and it lets the E4
  INCOMPLETE pin inject a failing history backend. (2) With `context_recall`
  retired, `ContextCurationMiddleware` needs only the compact flag —
  `new(flag)`, not §5.3's sketched `new(artifacts, flag, cap)`; the artifacts
  handle is consumed by `CuratedContext` alone. (3) `build_registry` gained a
  `max_read_bytes` parameter to thread the §5.4 read cap to `ReadFile`.
  (4) `LoopConfig` was NOT extended with a `backend` field (§5.3 sketched
  one): the default HostBackend is derived in `AgentLoop::new` from
  `config.workspace` and overridden via `with_backend`, keeping the wave-1
  config surface and its `Default` untouched. (5) host.rs's `FsError::NotUtf8`
  message was reworded to "is not valid UTF-8" (Task 6): the spec's
  parity-shim wording was unsatisfiable against the brief's own honest-error
  test; ruled within J3's declared-change umbrella (edit_file-on-binary
  wording changed, no consumer pinned the old text).
