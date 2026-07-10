# File-based memory — full switch (deepagents refactor, Phase 4A) — design

**Status:** PANEL-REVIEWED + **OWNER GATE CLOSED 2026-07-09**. Adversarial
panel (4 reviewers, distinct mandates): all **APPROVE-WITH-FIXES**; one
converged BLOCKER (src-tauri/web memory-admin surface) resolved at the gate
(E1 = retire). All fix-in-place findings FOLDED; gate decisions E1–E6 applied
(see Gate decisions + Panel & review log). **PLAN-READY** pending the
light-tier consistency read.
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
v0.1** (github.com/GoogleCloudPlatform/knowledge-catalog `okf/SPEC.md`) — OKF
*shape*, with deliberate memory-local divergences from the repo's doc-bundle
usage (§2.3).
**Live-source baseline:** commit 602ae5d (3B-2 merged, Phase 3 complete),
re-read 2026-07-09 during brainstorm (Explore source map) and re-verified by the
panel's assumptions reviewer (16 claims checked; divergences folded). All
`file:line` anchors are orientation only — **locate quoted code by content
before editing.**
**Builds on:** Phase-1 middleware seam (707d7fd), Phase-2 backend seam
(71e23d1), Phase-3A loop middleware wave (cb6ddf0), Phase-3B registry/
permissions/stream (4cf682d, 68e846b, 3490590, 602ae5d). Preserves all
prior-phase invariants (§3).

## Gate decisions (CLOSED 2026-07-09)

- **E1 — Desktop/web memory-admin surface: RETIRE.** The four src-tauri IPC
  commands (`memory_list` / `memory_update` / `memory_delete` /
  `memory_recall_preview`; src-tauri/src/lib.rs ~154-213, bridge.rs) and the
  web MemorySection CRUD panel (MemorySection.tsx + test, api.ts, types.ts
  `MemoryRow`/`ScoredRow`, ContextExplorer wiring) are deleted; the
  SettingsForm "remember/recall across sessions" label and
  ArchDetail/archFixture `recall_budget` references update. A read-only
  file-browser over `/memories/` is a possible later feature (§5), not this
  cycle.
- **E2 — Refresh cadence: DIRTY-FLAG.** The middleware re-reads the index
  only after a turn whose tool calls wrote under `/memories/` (§2.4). Same
  visible freshness as per-turn re-reading; prefix cache stays stable unless
  memory actually changes.
- **E3 — SPLIT: 4A-0 rename slice, then 4A-1 memory slice.** The
  metadata-root rename lands first as its own mechanical slice (allowlisted
  targets, full ci.sh green), then the memory work builds on it. One spec
  (this one), two plan slices.
- **E4 — Injection control: DISTINCT GLOBAL APPROVAL — moot this cycle via
  E5, recorded as a BINDING CONDITION on any future global tier.** The owner
  required more than trust-framing + ordinary write-Ask for global memory:
  any write/edit under a global memories route must trigger a distinct,
  explicit approval (never coalesced into ApproveAlways). E5 (project-only
  v0) removes the global surface entirely, resolving the blast-radius
  finding *by construction* for this cycle; the distinct-approval control is
  the recorded precondition a future `global/` tier must implement (§5).
- **E5 — Scoping: PROJECT-ONLY v0.** One scope: `projects/<project-key>/`.
  The `global/` tier is deferred until a real cross-project need lands, and
  arrives only together with the E4 control. (Panel found zero global facts
  in the old store; the owner weighed the demonstrated-need argument and
  chose to defer.) The on-disk layout keeps the `projects/` subdirectory so
  a future `global/` sits beside it without moving files.
- **E6 — Retire mechanism: UNINDEX, accepted.** No delete tool; removing a
  memory's index line hides it from loading; unindexed files remain
  greppable; true deletion is a human `rm`. Labeled honestly throughout
  (never "forget").

## 0. Scope

Split per E3 into two plan slices sharing this spec.

**IN — Slice 4A-0 (metadata-root rename, mechanical, lands first):**

- `~/.agent/` → `~/.rusty-agent/` and the runtime's workspace-dir convention
  `<workspace>/.agent/` → `<workspace>/.rusty-agent/`. **Allowlist sweep, not
  a blind search-replace** (panel: a naive `.agent`→`.rusty-agent` sed would
  corrupt the repo's unrelated `.agents/` and `.claude/` trees). Verified
  real targets (locate by content): `agent-skills/src/registry.rs` (both
  default dirs + 2 test asserts), `agent-runtime-config/src/trace.rs`
  (sessions default), `agent-server/src/session.rs` (independent hardcoded
  `.agent/skills` join), `agent-cli/src/main.rs` (`~/.agent/sessions` display
  string + doc comments), runtime-config test literals, `README.md`, the
  config example doc stub, docs. **Corrections vs the brainstorm draft:**
  this repo has NO checked-in `.agent/` tree (nothing to `git mv`), and
  `scripts/skills_lint.py` does not reference the runtime dirs (no-op).
- Guard test: zero `.agent` (non-`.agents`) literals remain in runtime code.
- One-line migration note (`mv ~/.agent ~/.rusty-agent`); no auto-migration.
- Full ci.sh green; merged before 4A-1 starts.

**IN — Slice 4A-1 (file-based memory + vector retirement):**

- **Memory store as an OKF-shaped mini-bundle** per project under
  `~/.rusty-agent/memories/projects/<project-key>/`: a reserved-form
  `index.md` (always loaded) and one-fact-per-file concept docs with OKF
  frontmatter (§2.3). No global tier (E5).
- **`MemoryFilesMiddleware`** (agent-core) replacing `MemoryRecallMiddleware`
  in the same optional stack slot (gated by `cfg.memory`): loads the project
  index via a **new middleware→backend read path** (net-new plumbing — no
  middleware holds a backend today, §2.4), injects through the existing
  (renamed) recall pinned-slot machinery with trust framing and **net-new
  honesty-pointer truncation** (§2.4). Refresh = dirty-flag (E2).
- **Composite-backend mount**: `/memories/project/` — **read-write to the
  parent's tools (net-new mount shape; today's mounts are read-only in BOTH
  parent and child), read-only in children** via the Phase-2
  `ReadOnlyToTools` wrapper (§2.6).
- **Policy mount-awareness (small, net-new):** the policy engine must
  recognize backend-routed virtual prefixes so `/memories/` reads auto-allow
  like workspace reads while writes keep the existing Ask (§2.6 — without
  this, every memory read prompts, or misresolves against the workspace
  root).
- **Editing contract = ordinary file tools** over `/memories/project/` plus
  a static memory-discipline section in the system prompt (parent-only,
  gated by `cfg.memory` — a `compose_system_prompt` signature change)
  (§2.5).
- **Atomic index writes:** HostBackend write moves to temp-file + rename (or
  at minimum for `/memories/` routes) to close the torn-read window (§2.4,
  panel fix).
- **Retirement**: delete the `agent-memory` crate (embedder, SQLite store,
  retriever, remember/recall/forget tools, `MemoryConfig`, `fastembed`/
  `rusqlite` deps); relocate `project_scope()`; retire the `Retriever`
  trait; rename the recall block machinery to the memory block; **retire the
  src-tauri IPC commands + web MemorySection per E1** (§2.7).
- Config (JSON — the runtime config loader is `serde_json`; the example file
  is a doc-only stub, §2.7): new optional `memories_dir` override (default
  `~/.rusty-agent/memories`; real consumer: test isolation). Index budget is
  a **const** (`DEFAULT_MEMORY_INDEX_BUDGET = 1024`), not a config field
  (panel trim).
- Sweep: every reference to remember/recall/forget across code, tests,
  eval/soak harnesses, config example, and docs — enumerated targets in
  §2.7.
- Tests incl. a `cfg.memory=false` byte-identical pin, `pinned_tokens()`
  lockstep, child read-only conformance, policy-decision pins for memory
  reads/writes, truncation honesty, dirty-flag cadence, and an `#[ignore]`
  cross-run live soak (§6).

**OUT / deferred:**

- **No global memory tier (E5).** Deferred until a demonstrated
  cross-project need; MUST arrive with the E4 distinct-approval control
  (§5). Layout reserves the spot (`memories/projects/` subdirectory).
- **No hybrid retrieval layer.** The owner rejected keeping the vector store
  under the file contract. If semantic retrieval is ever wanted again it is a
  new spec; nothing here reserves a seam for it beyond "memory is files."
- **No auto-migration of `~/.agent/`.** The memory DB is empty (verified
  2026-07-09: 0 rows); session traces are inert JSONL. One-line migration
  note only (4A-0).
- **No code-level format validation.** Memory files are agent-written with
  ordinary tools; readers follow OKF consumer rules (tolerate unknown types,
  broken links, malformed optional fields). `okf_check.py` is NOT run against
  runtime memory — and would **reject** these files by design (its
  `ALLOWED_TYPES` is the doc-authoring vocabulary; memory types are
  runtime-local, §2.3). Do not wire it up.
- **No child memory writes** (posture tightened vs today, deliberately —
  §2.6) and **no auto-load of the index into child prompts** (3A quarantine
  preserved).
- **No delete tool** (E6): removal = unindexing; true deletion is a human
  operation.
- **No semantic/similarity recall of any kind.** Finding memory is
  `read_file`/`grep` over `/memories/project/` plus the always-loaded index.
  (There is no `glob` tool — fs tools are read/write/edit/ls/grep; the
  brainstorm draft's `glob` mention was wrong.)
- **No `log.md`, no tombstone convention, no normative type vocabulary**
  (panel trims): update history is frontmatter `timestamp`s; retire =
  unindex; `type` is required by OKF shape but its values are suggested,
  not enforced (§2.3).
- **No memory UI** (E1): the tab + IPC commands retire; a read-only
  file-browser is possible later work (§5).
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
surfaces; the human can't audit or edit it without SQL), **API-shaped**
rather than substrate-shaped (three bespoke tools instead of the file tools
the agent already has, on the backend seam Phase 2 already built), and
**uncacheable** (recall content varies per input — a cost this design
recovers: with dirty-flag refresh (E2) the index block is static unless
memory is actually written). The store is empty in practice (0 rows after
weeks of use) — the bespoke API failed to earn adoption even from its own
agent. The owner decided full switch: files in, vectors out.

## 2. Design

### 2.1 Approaches considered

- **A — middleware + pinned block (CHOSEN):** `MemoryFilesMiddleware` reads
  the project index via `/memories/project/` and injects through the
  existing pinned-slot machinery. Reuses pinned-token accounting; the
  calibration change stays in the one slot the eval already measures;
  dirty-flag refresh keeps the block cache-stable.
- **B — compose-time system-prompt block:** best prefix-cache behavior, but
  static per run, bypasses pinned-token accounting, and moves the
  `cfg.memory` gate out of the loop into assembly. Rejected (E2's dirty-flag
  recovers the cache benefit inside approach A).
- **C — bespoke file-backed remember/forget tools:** contradicts the adopted
  contract (ordinary tools, transparent files). Rejected.
- **Load contract (owner):** index + on-demand (bounded prompt cost;
  agent navigates memory like files) over load-everything and budgeted
  load-all.
- **Format (owner):** OKF v0.1 shape over invented "structured-lite" or
  fully-freeform — backed by a spec the repo already authors in, whose
  reserved `index.md` form *is* the index-and-hook list the load contract
  wants, and whose consumer rules spec the graceful degradation an
  agent-written store needs. Memory-local divergences are deliberate and
  listed (§2.3).
- **Scoping (owner, gate E5):** project-only v0; the brainstormed two-tier
  layout survives on disk as the `projects/` subdirectory so a future
  `global/` (with the E4 control) is additive.

### 2.2 Storage layout & metadata-root rename (4A-0)

```
~/.rusty-agent/                      # home metadata root (was ~/.agent/)
├── memories/
│   └── projects/<project-key>/      # OKF-shaped mini-bundle (one per project)
│       ├── index.md                 #   reserved-form index — always loaded
│       └── <slug>.md                #   one memory per file
├── sessions/                        # trace dir (was ~/.agent/sessions/)
└── skills/                          # fallback skills dir (was ~/.agent/skills)
```

- `<project-key>` = the **existing** scheme (corrected per panel):
  `SHA256(git_toplevel(workspace) OR canonical_path)` hex — a **single**
  hashed value, the git toplevel when in a repo, else the canonical path
  (`agent-memory/src/scope.rs` today; helper relocates, §2.7). Scheme
  unchanged ⇒ project identities survive the refactor.
- **Mount mapping (agent-visible vs disk):** the composite mounts
  `/memories/project/` → `<memories_dir>/projects/<key>/`, with `<key>`
  resolved from the workspace at assembly. The agent never sees the hash —
  its memory is always at the stable path `/memories/project/`.
- Workspace convention renames too (owner: whole root):
  `<workspace>/.agent/skills` → `<workspace>/.rusty-agent/skills`. Real
  targets only (§0 allowlist) — there is no checked-in `.agent/` tree in
  this repo, and `.agents/` / `.claude/` are unrelated and MUST NOT be
  touched.
- Trace default `~/.agent/sessions` → `~/.rusty-agent/sessions`.
- No auto-migration (§0 OUT). HostBackend `write` calls
  `create_dir_all(parent)` (verified) so missing directories appear on first
  memory write; absence is never an error (§4). Stale-path note: a session
  started pre-upgrade keeps writing old-root traces until restarted; explicit
  old-root paths in a user's JSON config keep working (overrides are
  respected) — the rename changes defaults only.

### 2.3 Memory file format (OKF v0.1 shape, memory-local dialect)

The project scope directory is a small OKF-shaped bundle. **Deliberate
divergences from the repo's doc-bundle usage (panel: state them so nobody
"fixes" them):** memory `type` values are runtime-local (not
`okf_check.py`'s `ALLOWED_TYPES`); index lines use the memory-local form
below (not the doc bundles' absolute-path em-dash form); `okf_check.py` is
never pointed at memory (§0 OUT).

- **`index.md`** — reserved OKF index form, **no frontmatter**: bullet list,
  one line per memory, `* [Title](<slug>.md) - hook`. This is the only file
  the loader reads (§2.4). The hook line is the recall surface: it must
  carry enough for the agent to decide whether to `read_file` the node.
- **Memory nodes** — UTF-8 markdown, YAML frontmatter with required `type`
  plus recommended `title`, `description`, `tags`, `timestamp` (ISO 8601).
  `type` values are **suggested, not enforced** (panel trim — no code reads
  them): the discipline prompt suggests `User` / `Project` / `Feedback` /
  `Reference` as natural categories. Body freeform; cross-links between
  memories are ordinary markdown links (OKF: untyped directed edges,
  consumers tolerate broken links).
- **No code validation** (§0 OUT): the format is enforced by prompt
  discipline and tolerated-by-construction on read. The only hard dependency
  is `index.md` being readable text.

### 2.4 Load contract — `MemoryFilesMiddleware`

Replaces `MemoryRecallMiddleware` in the same conditional slot
(`assemble.rs:189-193`): stack stays
`[TodoList, Memory?, Curation, Stuck, ModelCallLimit(disabled),
ToolCallLimit, Repair]` — **order and every other entry untouched** (§3).

- **Net-new plumbing (panel correction — this is NOT reuse):** no middleware
  today holds a backend handle; `RunCx` has no backend field; today's
  injection goes through the `Retriever` port, and `SessionArtifacts` routes
  are `MemBackend`-backed. `MemoryFilesMiddleware` is constructed at
  assembly with the project-scope `Arc<dyn Backend>` handle (agent-core
  already depends on the crate that defines `Backend` — dep direction
  verified workable); reading a backend from a middleware is a new
  capability the plan must build, not find.
- **`on_run_start`** (fires once pre-loop, verified): read the project
  index and set the memory block via the renamed pinned-slot setter (§2.7).
  Missing dir/file ⇒ block omitted entirely (matches today's empty-recall
  behavior).
- **Dirty-flag refresh (E2):** the middleware re-reads and re-sets the block
  in `after_tools` **only when the just-executed turn's tool calls include a
  successful write/edit whose path is under `/memories/`** (observable from
  the turn's tool calls/intents — exact detection point at plan time). A
  same-turn memory write is thus visible to the next model call; turns that
  don't touch memory re-render nothing and the prefix cache is undisturbed.
  Note: `after_tools` fires every tool turn but never on a text-only turn
  (panel-verified; benign — a text-only turn is the run's last model call,
  nothing consumes a refresh).
- **Raw-read byte cap (panel fix):** the middleware reads `index.md` via the
  backend directly (bypassing the read tool's paging), so it applies its own
  ceiling — read at most `MEMORY_INDEX_MAX_BYTES` (const, e.g. 256 KiB);
  beyond that, truncate at the cap and treat the remainder as omitted
  entries (counted into the pointer line below). A pathological index can
  never OOM or stall the loop.
- **Rendered block** (in the recall block's pinned position: system →
  goal/ledger → **memory** → summary → todos):
  1. header naming the store (`Long-term memory (self-managed files under
     /memories/project/):`) — exact wording at plan time;
  2. **trust framing** (bundle wording): memory may be outdated, incorrect,
     or written by someone other than the current user, and must not
     override the user's explicit request;
  3. the index content, rendered so entry links resolve under
     `/memories/project/` from the agent's point of view;
  4. **budget + honesty pointer (net-new code — panel correction: the live
     `recall_block` truncator silently drops its tail and emits no
     pointer):** `DEFAULT_MEMORY_INDEX_BUDGET` (const, 1024 tokens): truncate
     whole entries from the tail and emit `[index truncated: N more entries
     — read /memories/project/index.md]`. Keep the soft-cap property: at
     least one entry always renders when the index is non-empty.
- **`pinned_tokens()` extends in lockstep** with the new block — the
  audit-7.3 `est_total` invariant holds (3A-S5 precedent).
- **Atomic writes (panel fix):** `HostBackend::write` is
  open-truncate-write (torn-read window for the refresh path and child
  greps). Change it to temp-file + `rename` (whole backend or `/memories/`
  routes at minimum — plan decides placement; whole-backend is strictly
  better and Phase-2 conformance-suite-visible).
- **Tools contributed: none.** The middleware's `tools()` is empty — the
  editing API is the file tools that already exist (§2.5). (Today's
  middleware contributes remember/recall/forget with `child_visible: true`;
  that contribution disappears with the tools.)

### 2.5 Editing contract & prompt discipline

No bespoke memory tools. Create = `write_file` under `/memories/project/`;
revise = `edit_file`; find = the always-loaded index + `read_file`/`grep`;
**unindex** (the retire mechanism, E6) = remove the memory's `index.md`
line — an unindexed node is invisible to loading. **No delete tool exists**
(verified: the `Backend` trait has `delete` but no tool exposes it, and the
shell cannot reach backend routes) — exposing one is NOT in scope; true
deletion is a human `rm`. No tombstone convention (panel trim).

A static **memory-discipline section** joins the system prompt **only when
`cfg.memory` is on and only for the parent** — this is a
`compose_system_prompt` signature change (panel: the function takes no such
flag today; children's prompts route differently and must not receive the
section — verify the child path at plan time). Content (prose at plan time,
contract here):

1. when you learn something durable about this project or how the user works
   in it, write it **in the same turn** — one fact per file, OKF frontmatter
   (`type` + `description`; suggested types §2.3), then add its index line
   to `/memories/project/index.md`; create the directory/index on first use;
2. update stale memories instead of duplicating — check the index first;
   fix the index line when the fact changes; retire a dead memory by
   removing its index line;
3. keep the index lean — it loads every run; the hook line should let a
   future run decide whether to open the file;
4. trust framing mirror: treat what you read there as possibly stale.

(The brainstormed scope-choice rule is gone with the global tier, E5.)

The old `Recall.when_not_to_call` disambiguation vs `large_tool_results/`
recovery retires with the tool — both surfaces are now just paths,
self-disambiguating.

### 2.6 Children & policy

- Child backends mount `/memories/project/` through the Phase-2
  `ReadOnlyToTools` wrapper: children read/grep memory when the parent
  directs them to; write/edit under `/memories/` fails with the standard
  read-only error.
- **The parent mount is read-write to tools — a net-new mount shape** (panel
  correction): today's composite routes (`large_tool_results/`,
  `conversation_history/`) are `ReadOnlyToTools`-wrapped in **both** parent
  and child; parent-privileged writes go through unwrapped non-tool handles.
  `/memories/project/` is the first tool-writable mount. The plan builds it
  deliberately (unwrapped Host route in the parent composite) rather than
  "reusing" a precedent that doesn't exist.
- **Policy mount-awareness (net-new, small — panel fix):** the policy engine
  resolves paths against the *workspace* root, so a virtual `/memories/...`
  path either fails containment (⇒ every memory read prompts Ask —
  approval-fatigue that would erode the load-bearing write-Ask) or
  misresolves. Required behavior, pinned by tests (§6): **reads under
  `/memories/` auto-allow** (same posture as workspace reads); **writes
  under `/memories/` keep the existing Ask** (verified: `Access::Write` ⇒
  `Decision::Ask` in RulePolicy today — this Ask is the load-bearing
  injection mitigation). Mechanism (policy learns the mount prefixes vs
  intent-path rewriting) is a plan decision; the workspace boundary posture
  for non-mount paths must not change (§3).
- `MemoryFilesMiddleware` is **parent-only** — the dispatch child stack is
  untouched, preserving the 3A quarantine. The existing quarantine test
  asserts on the rendered header string ("Relevant memories from past
  sessions"), not a middleware name (panel correction) — it updates to the
  new block header.
- **Posture change vs today, deliberate:** children currently CAN write
  memory (remember/forget are `child_visible: true`). Under this design they
  cannot — closing the injection-persistence vector for untrusted-content-
  processing children.
- The 3A/3B-1c residual (`Access::Read` ⇏ no side-effects) is unaffected —
  file tools declare honest intents.

### 2.7 Retirement & config surface

- **Delete the `agent-memory` crate** — embedder, stores, retriever, the
  three tools, `MemoryConfig`, and the `fastembed`(onnx)/`rusqlite` deps
  leave the workspace. Cargo dependents (panel-enumerated):
  `agent-runtime-config`, `agent-server`, **`src-tauri`** (E1: its four
  memory IPC commands are deleted, not migrated). Relocations, not losses:
  - `project_scope()` moves to where assembly can use it — default
    `agent-runtime-config`; exact placement at plan time.
  - the `Retriever` trait (`agent-core/src/recall.rs`) retires with its only
    implementor.
  - the pinned-slot machinery is **repurposed and renamed** — `set_recall` /
    `recall_block` / `RECALL_HEADER` / `recall_budget` become the memory
    block equivalents (one slot, same position, new name/header). No
    parallel dead code, no second slot. (Symbol churn across tests is
    accepted; do the rename as a mechanical final step of the memory slice.)
- **Sweep targets (panel-enumerated — the plan inherits this list, not a
  rediscovery):** `agent-tools/src/contract.rs` `CONFUSABLE_TOOLS`
  `"recall"` pair (its enforcement test trips if left); `agent-tools/src/fs/
  search.rs` grep `when_not_to_call` "use recall" prose;
  `agent-server/src/runtime.rs` tool-kind `"memory"` classification;
  `agent-runtime-config/src/lib.rs` imports + asserts;
  `agent-server/src/{setup,session,daemon}.rs`; `eval_context.rs` /
  `soak_live.rs` `MemoryParts`/`MemoryConfig` minting; src-tauri
  `bridge.rs`/`lib.rs` command registrations (E1); web
  `MemorySection.tsx`/`MemorySection.test.tsx`/`api.ts`/`types.ts`/
  `ContextExplorer.tsx` (E1), `SettingsForm.tsx` memory label,
  `ArchDetail`/`archFixture` `recall_budget`; README; `docs/`. Guard:
  registry test (tools absent) + grep-zero checks for the string targets.
- **Config (JSON, panel correction):** the runtime config is parsed with
  `serde_json` (`PartialRuntimeConfig` per-field merge); the example config
  file is a **doc-only stub** that never round-trips the loader.
  `cfg.memory: bool` keeps its name; it gates the middleware + the
  prompt-discipline section. The `/memories/project/` mounts (parent rw,
  child ro) stay **unconditional** so a flag flip never strands files
  half-visible. New config field: `memories_dir` (override, default
  `~/.rusty-agent/memories` — real consumer: test isolation).
  `DEFAULT_MEMORY_INDEX_BUDGET` and `MEMORY_INDEX_MAX_BYTES` are consts, not
  config (panel trim). All `MemoryConfig` knobs retire. The doc stub gains
  the memory fields; the loader test exercises JSON (§6).
- **Path-default renames are 4A-0** (§2.2, allowlist): trace dir, skills
  fallback, workspace skills dir, CLI display strings, README, docs.
- The parked context-evolve ceilings already require re-measurement
  (`[[context-evolve-needs-backend-migration]]`); the recall-block→memory-
  block change joins that same debt — no new campaign, but the debt note
  updates.

## 3. Invariants (do-not-regress)

1. **Pinned-block order** stays `system → goal/ledger → memory → summary →
   todos` — the memory block occupies exactly the retired recall block's
   position; every other block renders byte-identical.
2. **`cfg.memory = false` ⇒ pinned assembly byte-identical** to the same
   config today. Pinned by test (§6).
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
7. **Project-key scheme unchanged:** `SHA256(git_toplevel OR
   canonical_path)` — single value, not a concatenation (panel-corrected
   wording).
8. **Workspace-boundary policy posture unchanged for non-mount paths** —
   mount-awareness (§2.6) adds recognized virtual prefixes; it must not
   loosen containment anywhere else. New-test obligation (the Phase-2
   conformance suite is not assumed to cover arbitrary-prefix read-only
   mounts; panel correction).
9. **No-symlink-tool invariant (new, load-bearing — panel):** `/memories/`
   containment rests on "no tool can create a symlink/hardlink inside a
   mount" (path resolution chases links, and no tool plants them). Any
   future tool that can create links or extract archives into backend
   routes reopens mount-escape and MUST revisit this.

## 4. Edge cases & accepted residuals

- **First run / missing store:** no dirs, no index ⇒ empty block, no error,
  no noise; discipline covers creation; HostBackend `write` creates parent
  dirs (verified).
- **Index rot:** an index line pointing at a missing node = dead entry the
  agent repairs on encounter. **An orphaned node (file without an index
  line) is permanently invisible to loading** — "repair on encounter" never
  triggers for it (panel-corrected honesty; it remains greppable). No
  code-level consistency check; the live soak (§6) adds a
  node-count-vs-index-line-count observability assertion so rot is at least
  visible in soak runs.
- **Unindexed files remain greppable** (E6): unindexing hides memory from
  loading, not from search. Accepted; labeled honestly (never "forget").
- **Prefix-cache impact:** with dirty-flag refresh (E2) the pinned block
  re-renders only on turns that actually wrote memory — cache invalidation
  is write-gated, rare by design.
- **Concurrent sessions** writing the same project's `index.md`: torn reads
  are closed by atomic rename (§2.4), but **lost updates remain** (edit =
  read-modify-write; two concurrent sessions can drop one edit). Weaker
  than the SQLite it replaces; accepted for a single-user runtime, noted
  for any future multi-session work.
- **Budget truncation hides tail entries** from the auto-loaded view — by
  design; the pointer names the full index path; whole-entry truncation
  only.
- **`~/.agent` left behind** on machines that never run the migration `mv`:
  the runtime starts fresh. Accepted; release-note line covers it. Explicit
  old-root paths in user JSON configs keep working (defaults-only rename).
- **Trust boundary:** memory files are a prompt-injection persistence
  surface — a poisoned index line re-enters this project's every future
  prompt (project-confined by construction under E5; the old "same trust
  class as recall" claim was retracted at the panel). Mitigations:
  write-Ask (load-bearing, pinned by test §6), trust framing, parent-only
  writes. Residual accepted at the gate.

## 5. What this slice deliberately does not solve

- **A global memory tier (E5).** Deferred until a demonstrated cross-project
  need. **Binding condition (E4):** when added, writes/edits under the
  global route MUST trigger a distinct, explicit approval (never coalesced
  into ApproveAlways) — this is an owner-set precondition, not a suggestion.
  The disk layout (`memories/projects/`) already leaves room.
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
  the store measurably rots (the soak observability assertion, §4, is the
  early-warning).
- **Memory for the runtime's own sub-agents** beyond read-only reference:
  child-written memory needs a provenance/approval story first (4B's HITL
  machinery may supply one).
- **A memory file-browser UI** (E1 retired the CRUD tab): a later feature
  over `/memories/`, not this cycle.

## 6. Testing

Slice 4A-0 (rename): default-path tests for trace dir, skills fallback,
workspace skills dir (all `~/.rusty-agent`/`.rusty-agent` forms); guard
test: zero `.agent` (non-`.agents`) literals in runtime code; full ci.sh.

Slice 4A-1, unit (agent-core):
- load: index present / missing (empty block ⇒ omitted).
- budget: under/over `DEFAULT_MEMORY_INDEX_BUDGET`; whole-entry truncation;
  pointer wording; at-least-one-entry soft cap; `MEMORY_INDEX_MAX_BYTES`
  raw-read cap.
- dirty-flag cadence (E2): a turn with a `/memories/` write ⇒ next
  `pinned()` reflects the new index; a turn without ⇒ NO re-read
  (observable via a counting fake backend).
- `pinned_tokens()` lockstep with the new block (extend the existing
  lockstep test).
- trust-framing + header present when block non-empty.

Pins:
- **`cfg.memory=false` byte-identical pinned assembly** (invariant 2) —
  golden comparison against the pre-change rendering.
- Stack composition: memory slot present iff `cfg.memory`, order unchanged.
- Child stack contains no memory middleware (the existing quarantine test
  keys on the rendered header string — update it to the new header); child
  `/memories/project/` write ⇒ read-only error; child read ⇒ ok (new-test
  obligation, invariant 8).
- **Policy decisions pinned (net-new, §2.6):** parent read under
  `/memories/` ⇒ Allow; parent write under `/memories/` ⇒ Ask; a
  non-mount out-of-workspace path still fails containment exactly as today.
- remember/recall/forget absent from parent and child registries;
  grep-zero guards for the §2.7 string targets (CONFUSABLE_TOOLS pair
  removed with its test updated, when_not_to_call prose, runtime.rs kind).
- Atomic-write: backend conformance gains a torn-read regression test
  (concurrent read during write never observes partial content).

Config:
- JSON loader test: `memories_dir` override honored via
  `PartialRuntimeConfig` merge; the example doc stub updated (content-only —
  it does not round-trip the loader).

Surfaces (E1): src-tauri compiles with the crate and its four memory
commands gone; web typecheck + vitest green with MemorySection removed and
SettingsForm/ArchDetail references updated.

Live:
- `#[ignore]` cross-run soak: run 1 is prompted to remember a fact (writes
  node + index line via ordinary tools, passing the write-Ask); a **fresh**
  run 2's pinned block contains the index entry; run 2 can `read_file` the
  node. Soak also asserts node-count vs index-line-count match (rot
  observability, §4).

Suite: full `bash scripts/ci.sh` (fmt, clippy, both workspaces' legs, web)
before each slice's merge, per campaign convention.

## 7. Success criterion

Memory persists across runs as human-auditable OKF-shaped files under
`~/.rusty-agent/memories/projects/<key>/`, self-edited by the agent with its
ordinary file tools under prompt discipline, loaded index-first into the
retired recall block's exact pinned position with trust framing and honest
truncation pointers, refreshed only when memory is actually written. The
vector fork (crate, deps, tools, DB, desktop IPC commands, web panel) is
fully gone; children read but never write memory; memory writes prompt Ask
while reads auto-allow; `cfg.memory=false` remains byte-identical; all §3
invariants hold; ci.sh green on both slices.

## Panel & review log

### 2026-07-09 — Brainstorm (owner decisions)

Phase-4 decision round: memory GO full-switch (4A, this spec); durable HITL
GO (4B, next cycle); interpreter/PTC NO-GO behind an eval precondition.
Design decisions, owner-selected: scoping under a renamed `~/.rusty-agent`
root (rename extends to workspace `.agent` dirs and sessions); load
contract = index + on-demand; children = read-only, no auto-load;
architecture = middleware + pinned block (A); format = OKF v0.1
mini-bundles. Verified during brainstorm: memory DB empty (0 rows) ⇒ no data
migration; hook inventory supports on_run_start + after_tools; no delete
tool exists.

### 2026-07-09 — Adversarial spec panel (4 reviewers, distinct mandates) — all APPROVE-WITH-FIXES

Reviewers: requirements, assumptions (16 claims verified at live source),
failure & abuse, scope & simpler-design. Dispositions in three buckets per
AGENTS.md:

**Fixed in place (blockers/majors/minors folded):**
- Net-new framing corrections (assumptions F4/F5): parent-rw mount shape and
  middleware→backend handle are new capabilities, not reuse — today's mounts
  are read-only in both parent and child; no middleware reads a backend;
  today's injection rides the `Retriever` port (§2.4, §2.6).
- Truncation is net-new (failure M1): live `recall_block` silently drops its
  tail (no pointer); added budget + pointer + soft-cap contract (§2.4, §6).
  (The panel's per-scope starvation fix was subsumed by E5 project-only.)
- Unbudgeted raw index read (failure M2): added `MEMORY_INDEX_MAX_BYTES`
  ceiling (§2.4).
- Policy mount-awareness (failure): virtual `/memories/` paths misresolve
  against the workspace root; pinned read-Allow / write-Ask decisions +
  tests; confirmed write-Ask is the load-bearing injection mitigation
  (§2.6, §6).
- Torn reads (failure): HostBackend write → temp+rename (§2.4, §6).
- Phantom rename scope (scope/assumptions F12 + skills_lint): no checked-in
  `.agent/` tree; `skills_lint.py` unaffected; sweep is an allowlist with a
  do-not-touch-`.agents/`/`.claude/` guard + grep-zero test (§0, §2.2).
- Sweep enumeration (requirements M1): CONFUSABLE_TOOLS `"recall"`,
  grep `when_not_to_call` prose, runtime.rs tool-kind, session.rs hardcoded
  join, CLI display string, web SettingsForm/ArchDetail refs — all named
  (§2.7).
- Config is JSON (assumptions F15): loader is serde_json; the example config
  is a doc stub; tests target the JSON loader (§2.7, §6).
- `project_scope` description corrected to single-value hash (assumptions
  F9; §2.2, §3.7).
- `after_tools` fires only on tool turns (F1, benign — §2.4); quarantine
  test keys on header string (F6 — §2.6); `compose_system_prompt` needs a
  signature change (F14 — §2.5); no `glob` tool exists (F8 — §0, §2.5);
  conformance-suite claim downgraded to new-test obligation (requirements
  minor — §3.8); OKF divergences stated plainly so okf_check is never wired
  to memory (requirements minors — §2.3); orphaned-node invisibility stated
  honestly + soak observability assertion (failure minor — §4, §6);
  no-symlink-tool invariant recorded (failure minor — §3.9).
- Panel trims adopted: tombstone cut (unindex only), `log.md` cut, type
  vocabulary demoted to suggestion, `memory_index_budget` const-not-config
  (scope minors — §0, §2.3, §2.5, §2.7).

**Escalated to the owner gate:** E1 src-tauri/web surface fate (converged
BLOCKER); E2 refresh cadence (YAGNI/cacheability convergence); E3 rename
slice split; E4 injection blast-radius (global index > today's recall
exposure — "same trust class" claim retracted); E5 two-tier scoping; E6
forget-semantics honesty.

**Accepted as residual (minors):** lost updates on concurrent-session edits
(§4); unindexed-file greppability (§4, E6); symbol-rename churn (mechanical
final step, §2.7); type values unenforced (§2.3).

### 2026-07-09 — Owner gate (CLOSED)

**E1 = RETIRE** the memory tab + four IPC commands (delete, don't migrate;
file-browser is possible later work). **E2 = DIRTY-FLAG** refresh (re-read
only after a turn that wrote under `/memories/`). **E3 = SPLIT** into 4A-0
(rename) + 4A-1 (memory). **E4 = REQUIRE MORE: distinct global approval** —
made moot this cycle by E5 (no global surface); recorded in §5 as a binding
condition on any future global tier. **E5 = PROJECT-ONLY v0** (owner weighed
the demonstrated-need evidence — including live examples of global-shaped
facts from the assistant's own store — and chose to defer the tier).
**E6 = ACCEPT** unindex semantics. All decisions applied to this revision;
light-tier consistency read pending.
