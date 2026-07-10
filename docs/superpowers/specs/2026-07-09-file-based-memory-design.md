# File-based memory — full switch (deepagents refactor, Phase 4A) — design

**Status:** PANEL-REVIEWED 2026-07-09. Adversarial panel (4 reviewers, distinct
mandates): all **APPROVE-WITH-FIXES**; one converged BLOCKER (src-tauri/web
memory-admin surface unaccounted for → gate E1). All fix-in-place findings
FOLDED (see Panel & review log). **OWNER GATE PENDING:** E1–E6 below.
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

## Gate escalations (owner decisions required before plan)

- **E1 — Desktop/web memory-admin surface fate (BLOCKER).** `src-tauri`
  exposes `memory_list` / `memory_update` / `memory_delete` /
  `memory_recall_preview` IPC commands returning `agent_memory` types
  (src-tauri/src/lib.rs ~154-213, bridge.rs), consumed by the web
  MemorySection CRUD panel (web/src/explorer/MemorySection.tsx, api.ts,
  types.ts, + SettingsForm "remember/recall across sessions" label,
  ArchDetail/archFixture `recall_budget`). Crate deletion cannot land without
  deciding: **(a)** retire the memory tab + the four IPC commands (web panel
  removed; recommend for 4A — smallest honest cut; a file-browser view can be
  a later feature), or **(b)** migrate MemorySection to a read-only file view
  over `/memories/` (new UI feature — scope growth this cycle).
  **Recommendation: (a).**
- **E2 — Refresh cadence.** Panel converged (scope-YAGNI + requirements-
  cacheability + failure-cost): the brainstorm's unconditional `after_tools`
  re-read pays two host-fs reads + prefix-cache invalidation every tool turn
  to serve an event (mid-run memory write) that is rare by design. Options:
  **(a)** keep unconditional per-turn re-read (as brainstormed); **(b)**
  dirty-flag — re-read only after a turn whose tool calls wrote under
  `/memories/` (same visible freshness, cache-stable in the common case);
  **(c)** run-start-only (cache-perfect; mid-run writes invisible until next
  run). **Recommendation: (b).** §2.4 is written cadence-neutral with the
  chosen option to be inlined at gate close.
- **E3 — Split the metadata-root rename into its own slice.** The rename
  (§2.2) is mechanical, spans both workspaces + docs, and has zero logical
  dependency on the memory redesign; campaign precedent (3B split) argues for
  **4A-0 (rename, land + ci.sh green first)** then **4A-1 (memory)**.
  **Recommendation: split.** (Panel also flags, for conscious ownership: the
  rename is cosmetic — its value is branding consistency, not behavior.)
- **E4 — Injection blast-radius acceptance.** A poisoned `global/index.md`
  line loads into **every future run of every project** — strictly larger
  exposure than today's similarity-gated 512-token recall (the spec's earlier
  "same trust class" claim was FALSE and is retracted). Standing mitigations:
  every memory write already hits `Decision::Ask` (verified:
  `Access::Write` ⇒ Ask in RulePolicy), trust framing, parent-only writes.
  Proposed additional prompt-level mitigation (folded in §2.5, cuttable at
  gate): discipline instructs **global writes only on explicit user request**;
  same-turn auto-writes go to project scope. **Decision: accept the residual
  with these mitigations, or require more (e.g. unconditional human
  confirmation phrasing for global index edits)?**
- **E5 — Two-tier scoping (weak finding, raised for honesty).** Panel found
  zero evidence any global fact has ever existed (store empty; only test
  seeds). Options: keep two-tier (owner mandate; nearly free; "user prefers
  X"-class facts are genuinely cross-project) or ship project-only and add
  `global/` when a real cross-project fact appears.
  **Recommendation: keep two-tier.**
- **E6 — "Forget" semantics honesty.** Real deletion is impossible with
  ordinary tools (no delete tool; shell can't reach backend routes) — the
  mechanism is **unindexing** (drop the index line; §2.5), and unindexed
  files remain greppable. Relabeled throughout (no "forget" claims). Options:
  accept (recommended; human `rm` covers true deletion) or add a delete tool
  (new scope, new abuse surface). **Recommendation: accept.**

## 0. Scope

**IN (built this cycle; E3 may split it into 4A-0 + 4A-1):**

- **Metadata-root rename** (owner decision, whole root incl. workspace dirs):
  `~/.agent/` → `~/.rusty-agent/` and the runtime's workspace-dir convention
  `<workspace>/.agent/` → `<workspace>/.rusty-agent/`. **Allowlist sweep, not
  a blind search-replace** (panel: a naive `.agent`→`.rusty-agent` sed would
  corrupt the repo's unrelated `.agents/` and `.claude/` trees). Verified
  real targets (locate by content): `agent-skills/src/registry.rs` (both
  default dirs + 2 test asserts), `agent-runtime-config/src/trace.rs`
  (sessions default), `agent-server/src/session.rs` (independent hardcoded
  `.agent/skills` join), `agent-cli/src/main.rs` (`~/.agent/sessions` display
  string + doc comments), runtime-config test literals, `README.md`,
  config example, docs. **Corrections vs the brainstorm draft:** this repo
  has NO checked-in `.agent/` tree (nothing to `git mv`), and
  `scripts/skills_lint.py` does not reference the runtime dirs (no-op).
  Guard test: zero `.agent` (non-`.agents`) literals remain in runtime code.
- **Memory store as OKF-shaped mini-bundles** under
  `~/.rusty-agent/memories/`: `global/` + `projects/<project-key>/`, each
  with a reserved-form `index.md` (always loaded) and one-fact-per-file
  concept docs with OKF frontmatter (§2.3).
- **`MemoryFilesMiddleware`** (agent-core) replacing `MemoryRecallMiddleware`
  in the same optional stack slot (gated by `cfg.memory`): loads both scope
  indexes via a **new middleware→backend read path** (net-new plumbing —
  no middleware holds a backend today, §2.4), injects through the existing
  (renamed) recall pinned-slot machinery with trust framing, **per-scope
  budgets, and net-new honesty-pointer truncation** (§2.4). Refresh cadence
  per gate E2.
- **Composite-backend mounts**: `/memories/global/` and `/memories/project/`
  — **read-write to the parent's tools (net-new mount shape; today's mounts
  are read-only in BOTH parent and child), read-only in children** via the
  Phase-2 `ReadOnlyToTools` wrapper (§2.6).
- **Policy mount-awareness (small, net-new):** the policy engine must
  recognize backend-routed virtual prefixes so `/memories/` reads auto-allow
  like workspace reads while writes keep the existing Ask (§2.6 — without
  this, every memory read prompts, or misresolves against the workspace
  root).
- **Editing contract = ordinary file tools** over `/memories/` plus a static
  memory-discipline section in the system prompt (parent-only, gated by
  `cfg.memory` — a `compose_system_prompt` signature change) (§2.5).
- **Atomic index writes:** HostBackend write moves to temp-file + rename (or
  at minimum for `/memories/` routes) to close the torn-read window the
  refresh path can hit (§2.4, panel fix).
- **Retirement**: delete the `agent-memory` crate (embedder, SQLite store,
  retriever, remember/recall/forget tools, `MemoryConfig`, `fastembed`/
  `rusqlite` deps); relocate `project_scope()`; retire the `Retriever`
  trait; rename the recall block machinery to the memory block; **resolve
  the src-tauri/web surface per gate E1** (§2.7).
- Config (JSON — the runtime config loader is `serde_json`;
  `config.example.toml` is a doc-only stub, §2.7): new optional
  `memories_dir` override (default `~/.rusty-agent/memories`; needed for
  test isolation). Index budget is a **const** (`DEFAULT_MEMORY_INDEX_BUDGET
  = 1024`), not a config field (panel trim — nothing tunes it).
- Sweep: every reference to remember/recall/forget and `~/.agent` across
  code, tests, eval/soak harnesses, config example, and docs — enumerated
  targets in §2.7.
- Tests incl. a `cfg.memory=false` byte-identical pin, `pinned_tokens()`
  lockstep, child read-only conformance, policy-decision pins for memory
  reads/writes, per-scope truncation, and an `#[ignore]` cross-run live
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
  runtime memory — and would **reject** these files by design (its
  `ALLOWED_TYPES` is the doc-authoring vocabulary; memory types are
  runtime-local, §2.3). Do not wire it up.
- **No child memory writes** (posture tightened vs today, deliberately —
  §2.6) and **no auto-load of indexes into child prompts** (3A quarantine
  preserved).
- **No delete tool** (§2.5, gate E6): removal = unindexing; true deletion is
  a human operation.
- **No semantic/similarity recall of any kind.** Finding memory is
  `read_file`/`grep` over `/memories/` plus the always-loaded indexes.
  (There is no `glob` tool — fs tools are read/write/edit/ls/grep; the
  brainstorm draft's `glob` mention was wrong.)
- **No `log.md`, no tombstone convention, no normative type vocabulary**
  (panel trims): update history is frontmatter `timestamp`s; forget =
  unindex; `type` is required by OKF shape but its values are suggested,
  not enforced (§2.3).
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
recovers to the degree gate E2 chooses: a static-per-run index block is
prefix-cacheable; refresh-on-write invalidates only when memory actually
changes). The store is empty in practice (0 rows after weeks of use) — the
bespoke API failed to earn adoption even from its own agent. The owner
decided full switch: files in, vectors out.

## 2. Design

### 2.1 Approaches considered

- **A — middleware + pinned block (CHOSEN):** `MemoryFilesMiddleware` reads
  scope indexes via `/memories/` and injects through the existing pinned-slot
  machinery. Reuses pinned-token accounting; the calibration change stays in
  the one slot the eval already measures; refresh cadence per E2.
- **B — compose-time system-prompt block:** best prefix-cache behavior, but
  static per run, bypasses pinned-token accounting, and moves the
  `cfg.memory` gate out of the loop into assembly. Rejected (E2 option (b)
  recovers most of the cache benefit inside approach A).
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

### 2.2 Storage layout & metadata-root rename

```
~/.rusty-agent/                      # home metadata root (was ~/.agent/)
├── memories/
│   ├── global/                      # OKF-shaped mini-bundle
│   │   ├── index.md                 #   reserved-form index — always loaded
│   │   └── <slug>.md                #   one memory per file
│   └── projects/<project-key>/      # same shape
├── sessions/                        # trace dir (was ~/.agent/sessions/)
└── skills/                          # fallback skills dir (was ~/.agent/skills)
```

- `<project-key>` = the **existing** scheme (corrected per panel):
  `SHA256(git_toplevel(workspace) OR canonical_path)` hex — a **single**
  hashed value, the git toplevel when in a repo, else the canonical path
  (`agent-memory/src/scope.rs` today; helper relocates, §2.7). Scheme
  unchanged ⇒ project identities survive the refactor.
- **Mount mapping (agent-visible vs disk):** the composite mounts
  `/memories/global/` → `<memories_dir>/global/` and `/memories/project/` →
  `<memories_dir>/projects/<key>/`, with `<key>` resolved from the workspace
  at assembly. The agent never sees the hash — its project memory is always
  at the stable path `/memories/project/`.
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

Each scope directory is a small OKF-shaped bundle. **Deliberate divergences
from the repo's doc-bundle usage (panel: state them so nobody "fixes" them):**
memory `type` values are runtime-local (not `okf_check.py`'s
`ALLOWED_TYPES`); index lines use the memory-local form below (not the doc
bundles' absolute-path em-dash form); `okf_check.py` is never pointed at
memory (§0 OUT).

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
  assembly with two `Arc<dyn Backend>` scope handles (agent-core already
  depends on the crate that defines `Backend` — dep direction verified
  workable); reading a backend from a middleware is a new capability the
  plan must build, not find.
- **`on_run_start`** (fires once pre-loop, verified): read both scope
  indexes and set the memory block via the renamed pinned-slot setter
  (§2.7). Missing dir/file ⇒ that scope contributes nothing; both missing ⇒
  block omitted entirely (matches today's empty-recall behavior).
- **Refresh** (gate E2): if (a)/(b), the refresh rides `after_tools` — which
  fires **every tool turn but never on a text-only turn** (panel-verified;
  benign: a text-only turn is the run's last model call, nothing consumes a
  refresh). Option (b) re-reads only when the turn's tool calls touched
  `/memories/`.
- **Raw-read byte cap (panel fix):** the middleware reads `index.md` via the
  backend directly (bypassing the read tool's paging), so it applies its own
  ceiling — read at most `MEMORY_INDEX_MAX_BYTES` (const, e.g. 256 KiB) per
  index; beyond that, truncate at the cap and treat the remainder as
  omitted entries (counted into the pointer line below). A pathological
  index can never OOM or stall the loop.
- **Rendered block** (in the recall block's pinned position: system →
  goal/ledger → **memory** → summary → todos):
  1. header naming the store (`Long-term memory (self-managed files under
     /memories/):`) — exact wording at plan time;
  2. **trust framing** (bundle wording): memory may be outdated, incorrect,
     or written by someone other than the current user, and must not
     override the user's explicit request;
  3. `## global` index content, then `## project` index content, rendered so
     entry links resolve under `/memories/<scope>/` from the agent's point
     of view;
  4. **per-scope budgets (panel fix — cross-scope starvation):**
     `DEFAULT_MEMORY_INDEX_BUDGET` (const, 1024 tokens total) is split as an
     independent per-scope ceiling (512 each; an under-budget scope donates
     leftover to the other). A fat global index can never evict the project
     index. Within a scope: truncate whole entries from the tail and emit
     `[index truncated: N more entries — read /memories/<scope>/index.md]`.
     **The pointer and per-scope split are NET-NEW code** — the live
     `recall_block` truncator silently drops its tail and knows nothing of
     scopes (panel correction; the brainstorm draft wrongly cited it as
     precedent). Keep the soft-cap property: at least one entry per
     non-empty scope always renders.
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

No bespoke memory tools. Create = `write_file` under `/memories/<scope>/`;
revise = `edit_file`; find = the always-loaded indexes + `read_file`/`grep`;
**unindex** (the retire mechanism, gate E6) = remove the memory's `index.md`
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

1. when you learn something durable, write it **in the same turn** — one
   fact per file, OKF frontmatter (`type` + `description`; suggested types
   §2.3), then add its index line to that scope's `index.md`; create the
   directory/index on first use;
2. update stale memories instead of duplicating — check the index first;
   fix the index line when the fact changes; retire a dead memory by
   removing its index line;
3. scope deliberately: **project by default; write to global only when the
   user explicitly asks for a cross-project memory** (E4 mitigation —
   same-turn auto-writes never target global);
4. keep the index lean — it loads every run; the hook line should let a
   future run decide whether to open the file;
5. trust framing mirror: treat what you read there as possibly stale.

The old `Recall.when_not_to_call` disambiguation vs `large_tool_results/`
recovery retires with the tool — both surfaces are now just paths,
self-disambiguating.

### 2.6 Children & policy

- Child backends mount `/memories/global/` and `/memories/project/` through
  the Phase-2 `ReadOnlyToTools` wrapper: children read/grep memory when the
  parent directs them to; write/edit under `/memories/` fails with the
  standard read-only error.
- **Parent mounts are read-write to tools — a net-new mount shape** (panel
  correction): today's composite routes (`large_tool_results/`,
  `conversation_history/`) are `ReadOnlyToTools`-wrapped in **both** parent
  and child; parent-privileged writes go through unwrapped non-tool handles.
  `/memories/` is the first tool-writable mount. The plan builds it
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
  injection mitigation, E4). Mechanism (policy learns the mount prefixes vs
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
  `agent-runtime-config`, `agent-server`, **`src-tauri`** (gate E1 decides
  the desktop/web surface). Relocations, not losses:
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
  `bridge.rs`/`lib.rs` (E1); web `MemorySection.tsx`/`api.ts`/`types.ts`/
  `MemorySection.test.tsx`/`ContextExplorer.tsx` (E1), `SettingsForm.tsx`
  memory label, `ArchDetail`/`archFixture` `recall_budget`; README;
  `docs/`. Guard: registry test (tools absent) + grep-zero checks for the
  string targets.
- **Config (JSON, panel correction):** the runtime config is parsed with
  `serde_json` (`PartialRuntimeConfig` per-field merge); `config.example.toml`
  is a **doc-only stub** that never round-trips the loader. `cfg.memory:
  bool` keeps its name; it gates the middleware + the prompt-discipline
  section. The `/memories/` mounts (parent rw, child ro) stay
  **unconditional** so a flag flip never strands files half-visible. New
  config field: `memories_dir` (override, default `~/.rusty-agent/memories`
  — real consumer: test isolation). `DEFAULT_MEMORY_INDEX_BUDGET` and
  `MEMORY_INDEX_MAX_BYTES` are consts, not config (panel trim). All
  `MemoryConfig` knobs retire. The doc stub gains the memory fields; the
  loader test exercises JSON (§6).
- **Path-default renames ride along** (§2.2, allowlist): trace dir, skills
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
- **Prefix-cache impact:** per gate E2 — option (b) invalidates the prefix
  cache only on turns that actually wrote memory; option (a) accepts
  index-mtime-independent re-render each turn (content-stable unless
  written, so cache impact is write-gated either way).
- **Concurrent sessions** writing the same scope's `index.md`: torn reads
  are closed by atomic rename (§2.4), but **lost updates remain** (edit =
  read-modify-write; two concurrent sessions can drop one edit). Weaker
  than the SQLite it replaces; accepted for a single-user runtime, noted
  for any future multi-session work.
- **Budget truncation hides tail entries** from the auto-loaded view — by
  design; per-scope pointers name the full index path; whole-entry
  truncation only; per-scope budgets prevent cross-scope starvation (§2.4).
- **`~/.agent` left behind** on machines that never run the migration `mv`:
  the runtime starts fresh. Accepted; release-note line covers it. Explicit
  old-root paths in user JSON configs keep working (defaults-only rename).
- **Trust boundary (E4):** memory files are a prompt-injection persistence
  surface — and the global index has a strictly larger blast radius than
  the similarity-gated recall it replaces (loads into every run of every
  project). Mitigations: write-Ask (load-bearing, pinned by test §6), trust
  framing, parent-only writes, global-only-on-explicit-request discipline.
  Residual accepted at gate E4.

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
  the store measurably rots (the soak observability assertion, §4, is the
  early-warning).
- **Memory for the runtime's own sub-agents** beyond read-only reference:
  child-written memory needs a provenance/approval story first (4B's HITL
  machinery may supply one).
- **A memory file-browser UI** (if E1 = retire): a later feature over
  `/memories/`, not this cycle.

## 6. Testing

Unit (agent-core):
- load: both scopes, one scope, neither (empty block ⇒ omitted),
  global-then-project ordering.
- budgets: per-scope ceilings + donation; whole-entry truncation; pointer
  wording; at-least-one-entry-per-scope soft cap; **cross-scope starvation
  pin** (fat global cannot evict project entries); `MEMORY_INDEX_MAX_BYTES`
  raw-read cap.
- refresh (per E2): a mid-run index write is visible in the next `pinned()`
  render; if (b), a no-memory-write turn does NOT re-read (observable via a
  counting fake backend).
- `pinned_tokens()` lockstep with the new block (extend the existing
  lockstep test).
- trust-framing + header present when block non-empty.

Pins:
- **`cfg.memory=false` byte-identical pinned assembly** (invariant 2) —
  golden comparison against the pre-change rendering.
- Stack composition: memory slot present iff `cfg.memory`, order unchanged.
- Child stack contains no memory middleware (the existing quarantine test
  keys on the rendered header string — update it to the new header); child
  `/memories/` write ⇒ read-only error; child read ⇒ ok (new-test
  obligation, invariant 8).
- **Policy decisions pinned (net-new, §2.6):** parent read under
  `/memories/` ⇒ Allow; parent write under `/memories/` ⇒ Ask; a
  non-mount out-of-workspace path still fails containment exactly as today.
- remember/recall/forget absent from parent and child registries;
  grep-zero guards for the §2.7 string targets (CONFUSABLE_TOOLS pair
  removed with its test updated, when_not_to_call prose, runtime.rs kind).
- Atomic-write: backend conformance gains a torn-read regression test
  (concurrent read during write never observes partial content).

Config/paths:
- default-path tests for `memories_dir`, trace dir, skills fallback,
  workspace skills dir (all `~/.rusty-agent`/`.rusty-agent` forms); guard
  test: zero `.agent` (non-`.agents`) literals in runtime code.
- JSON loader test: `memories_dir` override honored via
  `PartialRuntimeConfig` merge; `config.example.toml` doc stub updated
  (content-only — it does not round-trip the loader).

Surfaces (per E1): src-tauri compiles with the crate gone; web typecheck +
vitest green with MemorySection retired-or-migrated.

Live:
- `#[ignore]` cross-run soak: run 1 is prompted to remember a fact (writes
  node + index line via ordinary tools, passing the write-Ask); a **fresh**
  run 2's pinned block contains the index entry; run 2 can `read_file` the
  node. Soak also asserts node-count vs index-line-count match (rot
  observability, §4).

Suite: full `bash scripts/ci.sh` (fmt, clippy, both workspaces' legs, web)
before merge, per campaign convention.

## 7. Success criterion

Memory persists across runs as human-auditable OKF-shaped files under
`~/.rusty-agent/memories/`, self-edited by the agent with its ordinary file
tools under prompt discipline, loaded index-first into the retired recall
block's exact pinned position with trust framing, per-scope budgets, and
honest truncation pointers. The vector fork (crate, deps, tools, DB) is
fully gone, including the E1-resolved desktop/web surface; children read but
never write memory; memory writes prompt Ask while reads auto-allow;
`cfg.memory=false` remains byte-identical; all §3 invariants hold; ci.sh
green.

## Panel & review log

### 2026-07-09 — Brainstorm (owner decisions)

Phase-4 decision round: memory GO full-switch (4A, this spec); durable HITL
GO (4B, next cycle); interpreter/PTC NO-GO behind an eval precondition.
Design decisions, owner-selected: two-tier scoping under a renamed
`~/.rusty-agent` root (rename extends to workspace `.agent` dirs and
sessions); load contract = index + on-demand; children = read-only, no
auto-load; architecture = middleware + pinned block (A); format = OKF v0.1
mini-bundles. Verified during brainstorm: memory DB empty (0 rows) ⇒ no data
migration; hook inventory supports on_run_start + after_tools; no delete
tool exists.

### 2026-07-09 — Adversarial spec panel (4 reviewers, distinct mandates) — all APPROVE-WITH-FIXES

Reviewers: requirements, assumptions (16 claims verified at live source),
failure & abuse, scope & simpler-design. Dispositions in three buckets per
AGENTS.md:

**Fixed in place (blockers/majors/minors folded into this revision):**
- Net-new framing corrections (assumptions F4/F5): parent-rw mount shape and
  middleware→backend handle are new capabilities, not reuse — today's mounts
  are read-only in both parent and child; no middleware reads a backend;
  today's injection rides the `Retriever` port (§2.4, §2.6).
- Truncation is net-new (failure M1): live `recall_block` silently drops its
  tail (no pointer); added per-scope budgets + pointers + starvation pin
  (§2.4, §6).
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
- Config is JSON (assumptions F15): loader is serde_json;
  `config.example.toml` is a doc stub; tests target the JSON loader (§2.7,
  §6).
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

**Escalated to the owner gate (decisions pending, top of file):** E1
src-tauri/web surface fate (converged BLOCKER); E2 refresh cadence
(YAGNI/cacheability convergence; recommendation dirty-flag); E3 rename
slice split (decomposition + cosmetic-cost honesty); E4 injection
blast-radius acceptance (global index > today's recall exposure — "same
trust class" claim retracted; discipline mitigation added §2.5.3, cuttable);
E5 two-tier scoping (weak finding vs owner mandate; recommendation keep);
E6 forget-semantics honesty (relabeled unindex; accept vs delete tool).

**Accepted as residual (minors):** lost updates on concurrent-session edits
(§4); unindexed-file greppability (§4, E6); symbol-rename churn (mechanical
final step, §2.7); enum-free type values unenforced (§2.3).
