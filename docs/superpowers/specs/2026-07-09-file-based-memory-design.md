# File-based memory — full switch (deepagents refactor, Phase 4A) — design

**Status:** BRAINSTORMED 2026-07-09, spec drafted; adversarial panel PENDING.
**Phase-4 decision round (owner, 2026-07-09, recorded here):** the bundle's three
Phase-4 judgment calls were decided before this brainstorm — **memory = GO, full
switch** (file-based memory replaces the vector store entirely; this spec);
**durable HITL = GO** (checkpointing foundation + resumable interrupts, sequenced
as Phase 4B, separate spec cycle); **interpreter/PTC = NO-GO / deferred** behind
an explicit eval precondition (build it only after an eval demonstrates
multi-tool orchestration overhead is a real bottleneck; no such eval exists and
the eval harness is itself parked, see
`[[context-evolve-needs-backend-migration]]`).
**Governing goal (owner):** adopt the deepagents agent-facing memory contract —
memory as transparent, human-auditable files, self-edited with the agent's
ordinary file tools, loaded index-first at run start, with explicit trust
framing — and **retire the vector fork completely** (no hybrid, no retrieval
layer kept).
**Knowledge base:** `docs/okf/deepagents-refactor/` (commit d997eec), esp.
`practices/memory-as-editable-files.md`. Memory-file format modeled on **OKF
v0.1** (github.com/GoogleCloudPlatform/knowledge-catalog `okf/SPEC.md`) — the
format this repo already authors bundles in and validates with
`scripts/okf_check.py` (owner decision).
**Live-source baseline:** commit 602ae5d (3B-2 merged, Phase 3 complete),
re-read 2026-07-09 during brainstorm (Explore-agent source map + spot reads).
All `file:line` anchors are orientation only — **locate quoted code by content
before editing.**
**Builds on:** Phase-1 middleware seam (707d7fd), Phase-2 backend seam
(71e23d1), Phase-3A loop middleware wave (cb6ddf0), Phase-3B registry/
permissions/stream (4cf682d, 68e846b, 3490590, 602ae5d). Preserves all
prior-phase invariants (§3).

## 0. Scope

**IN (built this cycle):**

- **Metadata-root rename** (owner decision, whole root incl. workspace dirs):
  `~/.agent/` → `~/.rusty-agent/` (sessions trace dir, fallback skills dir,
  new memories root) and `<workspace>/.agent/` → `<workspace>/.rusty-agent/`
  (skills registry primary dir), including this repo's checked-in
  `.agent/skills` tree, `scripts/skills_lint.py` expectations if it references
  the runtime dirs, and every doc reference (§2.2).
- **Memory store as OKF mini-bundles** under `~/.rusty-agent/memories/`:
  `global/` + `projects/<project-key>/`, each with a reserved-form `index.md`
  (always loaded), optional `log.md`, and one-fact-per-file concept docs with
  OKF frontmatter (§2.3).
- **`MemoryFilesMiddleware`** (agent-core) replacing `MemoryRecallMiddleware`
  in the same optional stack slot (gated by `cfg.memory`): loads both scope
  indexes via the `/memories/` backend route at `on_run_start`, refreshes in
  `after_tools`, injects through the existing (renamed) recall pinned-slot
  machinery with trust framing and an honest truncation pointer (§2.4).
- **Composite-backend mounts**: `/memories/global/` and `/memories/project/`
  as Host-backed routes; **read-only in children** via the Phase-2
  `ReadOnlyToTools` wrapper; read-write in the parent (§2.6).
- **Editing contract = ordinary file tools** over `/memories/` plus a static
  memory-discipline section in `compose_system_prompt` (parent-only) (§2.5).
- **Retirement**: delete the `agent-memory` crate (embedder, SQLite store,
  retriever, remember/recall/forget tools, `MemoryConfig`, `fastembed`/
  `rusqlite` deps); relocate `project_scope()`; retire the `Retriever` trait;
  rename the recall block machinery to the memory block (§2.7).
- Config: `memories_dir` override + `memory_index_budget` (default 1024);
  `cfg.memory: bool` keeps name and gate role (§2.7).
- Sweep: every reference to remember/recall/forget and `~/.agent` across
  code, tests, eval/soak harnesses, `config.example.toml`, and docs (§2.7).
- Tests incl. a `cfg.memory=false` byte-identical pin, `pinned_tokens()`
  lockstep, child read-only conformance, and an `#[ignore]` cross-run live
  soak (§6).

**OUT / deferred:**

- **No hybrid retrieval layer.** The owner rejected keeping the vector store
  under the file contract. If semantic retrieval is ever wanted again it is a
  new spec; nothing here reserves a seam for it beyond "memory is files."
- **No auto-migration of `~/.agent/`.** The memory DB is empty (verified
  2026-07-09: 0 rows); session traces are inert JSONL. Ship a one-line
  migration note (`mv ~/.agent ~/.rusty-agent`), nothing mechanized.
- **No code-level format validation.** Memory files are agent-written with
  ordinary tools; readers follow OKF consumer rules (tolerate unknown types,
  broken links, malformed optional fields). `okf_check.py` is NOT run against
  runtime memory (it validates repo doc bundles; memory is runtime data).
- **No child memory writes** (posture tightened vs today, deliberately —
  §2.6) and **no auto-load of indexes into child prompts** (3A quarantine
  preserved).
- **No new policy machinery.** Memory writes are ordinary `write_file` calls;
  `ToolIntent` already carries paths.
- **No semantic/similarity recall of any kind.** Finding memory is
  `read_file`/`grep`/`glob` over `/memories/` plus the always-loaded indexes.
- **Interpreter/PTC** stays NO-GO (decision round, header) — recorded here so
  the campaign memory can cite one artifact.

## 1. Problem

The runtime's long-term memory is the vector fork: `agent-memory` embeds
facts (fastembed BGE-Small ONNX), stores them as vector BLOBs in SQLite
(`~/.agent/memory.db`, project/global scoped), exposes bespoke
remember/recall/forget tools, and auto-injects top-k similar facts as a
512-token pinned recall block each run (`MemoryRecallMiddleware` →
`set_recall` → `recall_block`; middleware.rs:303, curated.rs:137,
context.rs:156). The deepagents comparison
(`comparisons/capability-gap-analysis.md`, "different fork") and the practice
doc (`practices/memory-as-editable-files.md`) identify what this fork costs:
memory is **opaque** (the agent can't see what exists, only what similarity
surfaces; the human can't audit or edit it without SQL), **uncacheable**
(recall content varies per input), and **API-shaped** rather than
substrate-shaped (three bespoke tools instead of the file tools the agent
already has, on the backend seam Phase 2 already built). The store is empty
in practice (0 rows after weeks of use) — the bespoke API failed to earn
adoption even from its own agent. The owner decided full switch: files in,
vectors out.

## 2. Design

### 2.1 Approaches considered

- **A — middleware + pinned block (CHOSEN):** `MemoryFilesMiddleware` reads
  scope indexes via `/memories/` and injects through the existing pinned-slot
  machinery. Live-refreshes after same-turn writes (`after_tools`); reuses
  pinned-token accounting; the calibration change stays in the one slot the
  eval already measures.
- **B — compose-time system-prompt block:** best prefix-cache behavior, but
  static per run (writes invisible until next run), bypasses pinned-token
  accounting, and moves the `cfg.memory` gate out of the loop into assembly.
  Rejected.
- **C — bespoke file-backed remember/forget tools:** contradicts the adopted
  contract (ordinary tools, transparent files). Rejected.
- **Load contract (owner):** index + on-demand (bounded prompt cost;
  agent navigates memory like files) over load-everything and budgeted
  load-all.
- **Format (owner):** OKF v0.1 mini-bundles over invented "structured-lite"
  or fully-freeform — same shape as structured-lite but backed by a spec the
  repo already uses, whose reserved `index.md` form *is* the index-and-hook
  list the load contract wants, and whose consumer rules spec the graceful
  degradation an agent-written store needs.

### 2.2 Storage layout & metadata-root rename

```
~/.rusty-agent/                      # home metadata root (was ~/.agent/)
├── memories/
│   ├── global/                      # OKF mini-bundle
│   │   ├── index.md                 #   reserved OKF index — always loaded
│   │   ├── log.md                   #   optional update history
│   │   └── <slug>.md                #   one memory per file
│   └── projects/<project-key>/      # same shape
├── sessions/                        # trace dir (was ~/.agent/sessions/)
└── skills/                          # fallback skills dir (was ~/.agent/skills)
```

- `<project-key>` = the **existing** scheme: SHA256(git toplevel ‖ canonical
  path) hex (`agent-memory/src/scope.rs:9` today; helper relocates, §2.7).
  Key scheme unchanged ⇒ project identities survive the refactor.
- **Mount mapping (agent-visible vs disk):** the composite mounts
  `/memories/global/` → `<memories_dir>/global/` and `/memories/project/` →
  `<memories_dir>/projects/<key>/`, with `<key>` resolved from the workspace
  at assembly. The agent never sees the hash — its project memory is always
  at the stable path `/memories/project/`.
- Workspace convention renames too (owner: whole root):
  `<workspace>/.agent/skills` → `<workspace>/.rusty-agent/skills`
  (`agent-skills/src/registry.rs:34-37` holds both defaults). This repo's own
  checked-in `.agent/` tree moves; docs and AGENTS.md references sweep.
- Trace default `~/.agent/sessions` → `~/.rusty-agent/sessions`
  (`agent-runtime-config/src/trace.rs:500`).
- No auto-migration (§0 OUT). The runtime creates missing directories lazily
  on first memory write; absence is never an error (§4).

### 2.3 Memory file format (OKF v0.1)

Each scope directory is a small OKF bundle:

- **`index.md`** — reserved OKF index form, **no frontmatter**: grouped
  bullet list, one line per memory, `* [Title](<slug>.md) - hook`. This is
  the only file the loader reads (§2.4). The hook line is the recall surface:
  it must carry enough for the agent to decide whether to `read_file` the
  node.
- **Memory nodes** — UTF-8 markdown, YAML frontmatter with required `type`
  plus recommended `title`, `description`, `tags`, `timestamp` (ISO 8601).
  Memory `type` vocabulary (producer-defined types are legal per OKF):
  `User` (who the user is), `Project` (ongoing work/constraints), `Feedback`
  (corrections/confirmed approaches, with why), `Reference` (pointers to
  external resources). Body freeform; cross-links between memories are
  ordinary markdown links (OKF: untyped directed edges, consumers tolerate
  broken links).
- **`log.md`** — optional, OKF log form (ISO-date headings, newest first).
  The discipline prompt mentions it as available, not required.
- **No code validation** (§0 OUT): the format is enforced by prompt
  discipline and tolerated-by-construction on read. The only hard dependency
  is `index.md` being readable text.

### 2.4 Load contract — `MemoryFilesMiddleware`

Replaces `MemoryRecallMiddleware` in the same conditional slot
(`assemble.rs:189-193`): stack stays
`[TodoList, Memory?, Curation, Stuck, ModelCallLimit(disabled),
ToolCallLimit, Repair]` — **order and every other entry untouched** (§3).

- **`on_run_start`**: read `global/index.md` and `projects/<key>/index.md`
  through the backend route; set the memory block via the renamed pinned-slot
  setter (§2.7). Missing dir/file ⇒ that scope contributes nothing; both
  missing ⇒ empty block (block omitted entirely, matching today's
  empty-recall behavior — curated.rs:137 renders only when non-empty).
- **`after_tools`**: unconditionally re-read and re-set (cheap host-fs reads
  of two small files, once per turn) so a same-turn memory write is visible
  to the *next* model call. Accepted inefficiency: re-reads happen even on
  turns that touched nothing under `/memories/` (§4).
- **Rendered block** (in the recall block's pinned position: system →
  goal/ledger → **memory** → summary → todos; curated.rs:119-154):
  1. header naming the store (`Long-term memory (self-managed files under
     /memories/):`) — exact wording at plan time;
  2. **trust framing** (bundle wording): memory may be outdated, incorrect,
     or written by someone other than the current user, and must not
     override the user's explicit request;
  3. `## global` index content, then `## project` index content (raw index
     lines, paths rewritten or prefixed so they resolve under `/memories/`
     from the agent's point of view);
  4. if over `memory_index_budget` (default **1024** tokens; replaces
     `recall_token_budget` 512 / `DEFAULT_RECALL_TOKEN_BUDGET`,
     context.rs:156): truncate whole entries from the tail and append
     `[index truncated: N more entries — read /memories/<scope>/index.md]`
     (Phase-2 INCOMPLETE-pointer honesty pattern). Never truncate mid-entry;
     always keep at least the first entry if any exist (soft-cap precedent,
     context.rs recall_block).
- **`pinned_tokens()` extends in lockstep** with the new block
  (curated.rs:193-222) — the audit-7.3 `est_total` invariant holds (3A-S5
  precedent).
- **Tools contributed: none.** The middleware's `tools()` is empty — the
  editing API is the file tools that already exist (§2.5). (Today's
  middleware contributes remember/recall/forget with `child_visible: true`,
  middleware.rs:322-330; that contribution disappears with the tools.)
- **Dep-direction build fact (verify at plan time, 3B-1 precedent):** the
  `Backend` trait lives in `agent-tools` (backend/mod.rs:68) and the
  middleware in `agent-core`. Phase 2 already has agent-core writing
  `large_tool_results/` via `SessionArtifacts` (`Arc<dyn Backend>` fields) —
  the middleware reaches `/memories/` through the same seam: assembly hands
  it the two scope backends (or one routed handle) at construction, exactly
  as `MemoryRecallMiddleware::new` receives its parts today
  (assemble.rs:189). If the trait's crate placement blocks agent-core from
  naming it, the plan resolves placement (re-export or trait-object
  indirection), not this spec.

### 2.5 Editing contract & prompt discipline

No bespoke memory tools. Create = `write_file` under `/memories/<scope>/`;
revise = `edit_file`; find = `read_file`/`grep`/`glob`; remove = delete the
memory's `index.md` line (an unindexed node is invisible to loading) and
overwrite the node body with a one-line tombstone. **No delete tool exists**
(verified 2026-07-09: the `Backend` trait has `delete` but no tool exposes
it, and the shell cannot reach backend routes) — exposing one is NOT in
scope; index-line removal is the forget mechanism. A static **memory-discipline section** joins
`compose_system_prompt` (`agent-skills/src/presets.rs:11-26`, beside the
skills-awareness note), **present only when `cfg.memory` is on and only for
the parent** (children get role prompts, not the discipline — §2.6). Content
(prose at plan time, contract here):

1. when you learn something durable, write it **in the same turn** — one
   fact per file, OKF frontmatter (`type` from §2.3 vocabulary +
   `description`), then add its index line to that scope's `index.md`;
   create the directory/index on first use;
2. update stale memories instead of duplicating — check the index first;
   fix the index line when the fact changes; retire a dead memory by
   removing its index line and tombstoning the body;
3. scope deliberately: project by default; global only for facts true across
   projects;
4. keep the index lean — it loads every run; the hook line should let a
   future run decide whether to open the file;
5. trust framing mirror: treat what you read there as possibly stale.

The old `Recall.when_not_to_call` disambiguation vs `large_tool_results/`
recovery (tools.rs:247-253) retires with the tool — both surfaces are now
just paths, self-disambiguating.

### 2.6 Children & policy

- Child backends mount `/memories/global/` and `/memories/project/` through
  the **existing `ReadOnlyToTools` wrapper** (same machinery as
  `large_tool_results/`, assemble.rs:503-515): children read/grep memory
  when the parent directs them to; write/edit/delete under `/memories/`
  fails with the standard read-only error.
- `MemoryFilesMiddleware` is **parent-only** — the dispatch child stack
  (`[curation, stuck, repair]` + 3B additions) is untouched, preserving the
  3A quarantine (dispatch.rs:2155 assertion adapts to the new middleware
  name).
- **Posture change vs today, deliberate:** children currently CAN write
  memory (remember/forget are `child_visible: true`). Under this design they
  cannot — closing the injection-persistence vector the deepagents
  production docs warn about (a child processing untrusted content must not
  be able to persist instructions into every future run's prompt).
- Policy: no new machinery. Memory writes are ordinary `write_file` calls
  whose `ToolIntent` carries the path; the 3B-1c floors and base policy see
  them like any file op. The 3A/3B-1c residual (`Access::Read` ⇏ no
  side-effects) is unaffected — file tools declare honest intents.

### 2.7 Retirement & config surface

- **Delete the `agent-memory` crate** — embedder, stores, retriever, the
  three tools, `MemoryConfig`, and the `fastembed`(onnx)/`rusqlite` deps
  leave the workspace. Relocations, not losses:
  - `project_scope()` (SHA256 key) moves to where assembly can use it —
    default `agent-runtime-config` (it already owns workspace-aware
    assembly, lib.rs:166-230); exact placement at plan time.
  - the `Retriever` trait (`agent-core/src/recall.rs:10-14`) retires with
    its only implementor.
  - the pinned-slot machinery is **repurposed and renamed** — `set_recall` /
    `recall_block` / `RECALL_HEADER` / `recall_budget` become the memory
    block equivalents (one slot, same position, new name/header). No
    parallel dead code, no second slot.
- **Config:** `cfg.memory: bool` (runtime_config.rs:161, default true)
  keeps its name; it gates the middleware + the prompt-discipline section.
  The `/memories/` mounts (parent rw, child ro) stay **unconditional** so a
  flag flip never strands files half-visible. New fields: `memories_dir`
  (override, default `~/.rusty-agent/memories`) and `memory_index_budget`
  (default 1024). All `MemoryConfig` knobs (db_path, thresholds, k values,
  model cache) retire.
- **Path-default renames ride along** (§2.2): trace dir, skills fallback,
  workspace skills dir, this repo's `.agent/` tree, `config.example.toml`,
  docs.
- **Sweep obligation (dedicated task at plan time):** every reference to
  remember/recall/forget and `~/.agent` across code, tests, eval/soak
  harnesses (`eval_context.rs`/`soak_live.rs` mint memory parts today),
  server/CLI surfaces, docs. The parked context-evolve ceilings already
  require re-measurement (`[[context-evolve-needs-backend-migration]]`);
  the recall-block→memory-block change joins that same debt — no new
  campaign, but the debt note updates.

## 3. Invariants (do-not-regress)

1. **Pinned-block order** stays `system → goal/ledger → memory → summary →
   todos` — the memory block occupies exactly the retired recall block's
   position (curated.rs:119-154); every other block renders byte-identical.
2. **`cfg.memory = false` ⇒ pinned assembly byte-identical** to the same
   config today (no recall block is rendered when recall is empty today, so
   the off-path must not change a byte). Pinned by test (§6).
3. **`pinned_tokens()` lockstep** with `pinned()` including the new block —
   the est_total audit invariant (3A-S5) holds.
4. **Child quarantine:** no memory middleware in child stacks; no index
   auto-load into child prompts; child `/memories/` access is read-only.
5. **Stack order and siblings untouched:** TodoList/Curation/Stuck/
   guardrails/Repair entries and their order unchanged; child stack
   unchanged apart from the mount.
6. **Calibrated token estimator, goal/ledger machinery, ToolIntent
   richness, refusal-on-degraded sandbox, first-class MCP** — untouched
   (campaign do-not-regress list).
7. **Project-key scheme unchanged** (SHA256 git-toplevel‖canonical-path).
8. **Backend conformance:** the new mounts pass the Phase-2 public
   conformance suite expectations for ReadOnlyToTools-wrapped routes.

## 4. Edge cases & accepted residuals

- **First run / missing store:** no dirs, no index ⇒ empty block, no error,
  no noise; discipline prompt covers creation. Lazy dir creation on first
  write (backend `write_file` semantics; verify Host creates parents at plan
  time, else the discipline prompt says to create the dir first).
- **Index rot** (file exists but index line missing, or line points at a
  deleted file): tolerated by OKF consumer rules; the agent repairs on
  encounter (discipline item 2). No code-level consistency check — accepted
  residual.
- **Unconditional `after_tools` re-read** even when no memory was touched:
  two small host-fs reads per turn — accepted inefficiency; optimizing
  (dirty-flag on `/memories/` writes) is follow-up fodder only if profiling
  ever cares.
- **Prefix-cache impact:** a mid-run index change re-renders the pinned
  region and invalidates the downstream prefix cache for that run — rare
  (only when memory is written mid-run) and identical in kind to today's
  goal/ledger mutations. Accepted.
- **Concurrent runs** writing the same scope's `index.md` can race
  (last-write-wins at host-fs level): pre-existing exposure class (SQLite
  serialized writes did better here) — accepted residual, single-user
  runtime; noted for any future multi-session work.
- **Tombstoned nodes leave residue** (forget = index-line removal + body
  tombstone, §2.5): stray files accumulate under the scope dir until a human
  or a future consolidation pass sweeps them. Accepted residual — invisible
  to loading, greppable on purpose.
- **Budget truncation hides tail entries** from the auto-loaded view — by
  design; the honesty pointer names the full index path. Entries are
  truncated whole (never mid-line).
- **`~/.agent` left behind** on machines that never run the migration `mv`:
  the runtime simply starts fresh (empty memory, new session dir). Accepted;
  the note in the release/README covers it.
- **Trust boundary:** memory files are a prompt-injection surface *for the
  parent that writes them* (self-written content re-enters every prompt).
  Mitigations: trust framing (§2.4), parent-only writes (§2.6). Same trust
  class as today's recall block; accepted.

## 5. What this slice deliberately does not solve

- **Semantic retrieval at scale.** If memory grows past what index+grep
  navigation handles, that's a future spec with fresh evidence (and the
  eval to show it) — not a reserved seam here.
- **Durable/checkpointed state (Phase 4B).** Memory files persist across
  runs but nothing here checkpoints *loop* state; 4B is a separate spec
  cycle.
- **Cross-machine / multi-user memory** (deepagents' org/assistant
  namespaces): single-user local runtime; out.
- **Background consolidation** (deepagents alternative to same-turn
  editing): discipline is same-turn; a consolidation pass is future work if
  the store measurably rots.
- **Memory for the runtime's own sub-agents** beyond read-only reference:
  child-written memory needs a provenance/approval story first (4B's HITL
  machinery may supply one).

## 6. Testing

Unit (agent-core):
- load: both scopes, one scope, neither (empty block ⇒ omitted), global-
  before-project ordering.
- budget: under/over `memory_index_budget`; whole-entry truncation; honesty
  pointer wording; at-least-first-entry soft cap.
- refresh: `after_tools` re-read makes a mid-run index write visible in the
  next `pinned()` render.
- `pinned_tokens()` lockstep with the new block (extend the existing
  lockstep test).
- trust-framing + header present when block non-empty.

Pins:
- **`cfg.memory=false` byte-identical pinned assembly** (invariant 2) —
  golden comparison against the pre-change rendering.
- Stack composition: memory slot present iff `cfg.memory`, order unchanged
  (extend assemble tests).
- Child stack contains no memory middleware (adapt dispatch.rs:2155
  assertion); child `/memories/` write ⇒ read-only error; child read ⇒ ok
  (conformance-suite style).
- remember/recall/forget absent from parent and child registries; dispatch
  schema carries no memory-tool references (sweep guard).

Config/paths:
- default-path tests for `memories_dir`, trace dir, skills fallback,
  workspace skills dir (all `~/.rusty-agent`/`.rusty-agent` forms).
- `config.example.toml` parses; `memories_dir` + `memory_index_budget`
  overrides honored.

Live:
- `#[ignore]` cross-run soak: run 1 is prompted to remember a fact (writes
  node + index line via ordinary tools); a **fresh** run 2's pinned block
  contains the index entry; run 2 can `read_file` the node.

Suite: full `bash scripts/ci.sh` (fmt, clippy, cargo test both workspaces'
legs, web) before merge, per campaign convention.

## 7. Success criterion

Memory persists across runs as human-auditable OKF files under
`~/.rusty-agent/memories/`, self-edited by the agent with its ordinary file
tools under prompt discipline, loaded index-first into the retired recall
block's exact pinned position with trust framing and honest truncation. The
vector fork (crate, deps, tools, DB) is fully gone; children read but never
write memory; `cfg.memory=false` remains byte-identical; all §3 invariants
hold; ci.sh green.

## Panel & review log

### 2026-07-09 — Brainstorm (owner decisions)

Phase-4 decision round: memory GO full-switch (4A, this spec); durable HITL
GO (4B, next cycle); interpreter/PTC NO-GO behind an eval precondition.
Design decisions, owner-selected: two-tier scoping under a renamed
`~/.rusty-agent` root (rename extends to workspace `.agent` dirs and
sessions); load contract = index + on-demand; children = read-only, no
auto-load; architecture = middleware + pinned block (A); format = OKF v0.1
mini-bundles. Verified during brainstorm: memory DB empty (0 rows), so no
data migration; middleware hook inventory supports on_run_start +
after_tools refresh.

*(Adversarial panel entry pending — 4 reviewers, distinct mandates, per
AGENTS.md; findings conflicting with the owner's full-switch mandate are
escalated to the gate, not silently adopted.)*
