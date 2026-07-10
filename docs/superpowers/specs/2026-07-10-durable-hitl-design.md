# Durable HITL — checkpointing foundation + resumable interrupts (deepagents refactor, Phase 4B) — design

**Status:** PANEL-REVIEWED + **OWNER GATE CLOSED 2026-07-10**. Adversarial
panel (4 reviewers, distinct mandates, all APPROVE-WITH-FIXES): converged
blockers (session-identity foundation; two-phase gate reality) and all
fix-in-place findings FOLDED; gate decisions E1–E6 applied (see Gate
decisions + Panel & review log). **PLAN-READY** pending the light-tier
consistency read + owner spec review.

**Governing goal (owner, from the Phase-4 decision round 2026-07-09):**
durable HITL = **GO** — a checkpointing foundation + resumable interrupts, so
a run parked on an approval survives frontend disconnect **and daemon
restart**, for parent **and child** loops, and resumes in place when a
frontend attaches and answers. Folds in the 3B-2 G4 residual (child-approval
attribution).

**Knowledge base:** `docs/okf/deepagents-refactor/` — esp.
`practices/declarative-guardrails.md` (interrupt-driven HITL: approve/edit/
reject/respond over durable interrupts, checkpointer mandatory, resume via
explicit decisions) and `comparisons/capability-gap-analysis.md` (Durable
execution: **absent**; HITL: **partial**). The priorities doc's warning —
"durable *resumable* interrupts imply checkpointing — a large LangGraph-shaped
dependency; scope deliberately" — is honored by checkpointing exactly one
point: where a run legally parks (E1).

**Live-source baseline:** commit 8b54fda (4A-1 merged, Phase 4A complete),
surveyed 2026-07-10 (Explore source map) and re-verified by the panel's four
reviewers (every §1 claim checked at source; two blockers and two mechanism
mislocations corrected — see log). All `file:line` anchors are orientation
only — **locate quoted code by content before editing.**

**Builds on:** Phase-1 middleware seam (707d7fd), Phase-2 backend seam
(71e23d1), Phase-3A guardrails/repair/todos (cb6ddf0), Phase-3B registry/
permissions/structured-response/typed-stream (4cf682d, 68e846b, 3490590,
602ae5d), Phase-4A file-based memory + `~/.rusty-agent` root (d482572,
8b54fda). Preserves all prior-phase invariants (§3).

## Owner decisions (brainstorm, 2026-07-10)

- **D1 — Durability level: DAEMON-RESTART AT APPROVAL.** A run parked at
  `Ask` survives frontend disconnect and daemon restart. **No mid-run crash
  recovery for non-parked runs** (a crash while no Ask is parked loses the
  run, exactly as today — reaffirmed at the gate by E1); no time travel
  (versioned format leaves the door open, §5).
- **D2 — Decision vocabulary: EXISTING THREE + DENY-FEEDBACK.** Approve /
  ApproveAlways / Deny, with a new optional feedback string on the human's
  Deny that becomes the tool-result text the model sees. deepagents'
  **edit** and **respond** decisions are deferred to a named follow-on (§5).
  Deny-feedback rides the surfaces slice (E4).
- **D3 — Child scope: ATTRIBUTION + CHILD CHECKPOINTS.** G4 fixed properly
  (approval requests carry delegation id / sub-agent name / depth) **and**
  children get the same checkpoint machinery, composed as a `sub{n}`-keyed
  tree under the parent's checkpoint; a restart resumes a parked child in
  place. (Owner chose the widest option over parent-only resume; E6a caps
  the live-drive depth.)
- **D4 — Resume UX: ATTACH-TO-RESUME, BOTH SURFACES.** The daemon indexes
  parked runs from disk at startup; attaching to the session (desktop open,
  CLI reopen) re-emits pending approvals with attribution; answering
  resumes the loop. Parked runs are listable. No new resume verb — but the
  CLI's session list/reopen substrate is net-new build (E3, slice 4B-2).
- **D5 — Architecture: PARK-POINT SNAPSHOTS** (originally "two-tier"; the
  end-of-turn tier was cut at the gate, E1). One checkpoint, written only
  when an Ask parks; explicit serialization of the few stateful pieces;
  everything non-serializable rebuilt fresh by `assemble_loop` on resume.

## Gate decisions (CLOSED 2026-07-10)

- **E1 — `turn.json` CUT; checkpoints are park-time-only.** Three reviewers
  converged: the rolling end-of-turn checkpoint mainly served mid-turn crash
  recovery, which D1 puts OUT, while adding hot-path writes to every run,
  every-turn artifact dumps, a GC problem, a parked-vs-turn precedence
  ambiguity, and a double-side-effect replay hazard. v0 writes a checkpoint
  **only at park**. Ordinary runs gain no checkpoint I/O at all (§3.1).
- **E2 — ApproveAlways persistence STRUCK; resumed runs re-ask.** No grant
  store exists at any layer today (`ApproveAlways` is byte-identical to
  `Approve` at the only decision site — one-shot allow, remembered nowhere).
  Checkpoints never carry standing approvals. A real always-grant table
  (live semantics first, then durability, honoring 4A-E4's
  distinct-global-approval condition) is a named §5 deferral.
- **E3 — CLI stays in scope, in its own slice.** D4's "both surfaces"
  honored, with the honest accounting that CLI session list/reopen is a
  net-new surface (single-shot process, no session concept today). Desktop
  proves the mechanism in 4B-1; CLI ships in 4B-2.
- **E4 — THREE SLICES, one spec.** 4B-0 session-descriptor foundation →
  4B-1 durability core → 4B-2 surfaces (deny-feedback + parked-run UX +
  CLI reopen). Each slice merges green before the next (4A precedent).
- **E5 — Timeout semantics: PARK BY DEFAULT + CONFIG AUTO-DENY KNOB.**
  Server channel: an unanswered Ask parks indefinitely by default; a new
  optional config knob (deny after N seconds) restores fail-closed for
  headless/eval callers. CLI interactive prompt: its independent 300s
  timeout now **parks-and-exits** ("run parked; reopen to answer") instead
  of denying.
- **E6 — (a) Grandchild depth: recursion implemented, depth-1 asserted
  live.** `subagent_max_depth` defaults to 1, so the live kill-restart
  drive exercises parent+child only; grandchild composition is covered by
  unit tests. **(b) Checkpoint integrity: HARDENED THIS CYCLE.** Beyond the
  folded-in mitigations (re-derived summaries, refuse-on-corrupt, file
  modes, tally clamps), checkpoint files carry an **HMAC** keyed from a
  daemon-local secret (§2.2); a failed MAC = corrupt = refuse to resume.

## 0. Scope

Split per E4 into three plan slices sharing this spec.

**IN — Slice 4B-0 (session-descriptor foundation, lands first):**
- Durable, restart-stable **session identity owned by the server session**
  (not the trace sink): per-session directory
  `~/.rusty-agent/sessions/<session_id>/` with `descriptor.json`
  (session id, workspace path, created-at, config provenance), written at
  run start, dirs `0o700` / files `0o600`.
- `TraceWriter` (and the CLI) **consume** the session id instead of minting
  it (`mint_session_id`'s `{secs}-{pid}` moves behind the descriptor);
  trace JSONL filename/shape unchanged otherwise.
- Startup index: the scan capability (`scan_descriptors`, tested +
  exported). *Owner-ratified narrowing (2026-07-10, 4B-0 whole-branch
  gate): the daemon-startup caller that rebuilds the session→workspace
  map lands in 4B-1 with attach-to-resume, its only consumer.*
- Daemon-local secret file for E6b HMAC (created on first use, `0o600`).

**IN — Slice 4B-1 (durability core):**
- `agent-core/src/checkpoint.rs`: versioned serde `Checkpoint`, park-time
  write, HMAC manifest, atomic temp+rename, refuse-on-corrupt.
- serde derives on `Message` (net-new — they do not exist today, §1).
- `CuratedContext` **restore constructor** (net-new seam) covering full
  pinned state (§2.2).
- Park path inside the Phase-1 gate loop; server-channel auto-deny removal
  (+ E5 knob); park-deletion as the answer commit point.
- Resume: startup parked index, attach re-emit, re-derived approval
  display (§2.4), gate-loop re-entry at the parked index.
- SessionArtifacts dump/restore at park time (Backend-trait-level).
- Child checkpoint tree + resume rebinding (D3); approval attribution via
  a wrap-at-dispatch attributing channel (§2.6); wire `ApprovalRequest`
  frame gains optional origin fields.
- Desktop/web: modal attribution + SubagentCard `waiting-approval` state
  (minimal; full parked-run UX is 4B-2).
- Tests incl. the live kill-restart WebDriver drive (new-pid daemon).

**IN — Slice 4B-2 (surfaces):**
- Deny-feedback: `ApprovalResponse::Deny` gains the optional feedback
  string; wire `Decision::Deny` carries it (**breaks the enum's `Copy`
  derive** — every match site across wire.rs, agent-server, agent-cli,
  src-tauri, web fans out); feedback splices into the tool-result text.
- `parked_runs` list frame; `approval_resolved` broadcast (retracts stale
  prompts on other surfaces); `resumed` notice; web parked banner + deny
  feedback field.
- CLI: session listing (parked marked), reopen-with-re-prompt,
  timeout parks-and-exits (E5).

**OUT (named, §5 where deferred):** edit/respond decisions; ApproveAlways
grant store (E2); time travel / checkpoint history; mid-tool-execution
checkpointing; **any crash recovery for non-parked runs** (E1); declarative
interrupt_on-style permission rules; server event replay buffer (3B-2
deferral, adjacent but separate); checkpoint retention policy beyond
delete-on-completion (§2.3); tamper resistance beyond §2.2's HMAC +
mitigations — a local attacker who can read the daemon-local secret is
same-trust-domain, **accepted threat, recorded not silent**; PII/retention
policy for checkpoint contents beyond what trace JSONL already implies.

## 1. Current state (survey 2026-07-10, baseline 8b54fda; panel-verified)

**Approval flow.** `PolicyEngine::check` → `Decision::{Allow,Ask,Deny}`
(agent-policy/src/engine.rs:31-33). The loop's turn is **strictly
two-phase** (loop_.rs ~988-1065): Phase 1 gates every parsed tool call
**sequentially** — `gate_tool` awaits `approval.request(...)` inline, and
its doc comment states "sequential by design so approval prompts never
overlap" (~loop_.rs:1302-1388); only after the whole batch is gated does
Phase 2 execute the approved set concurrently (`buffer_unordered`).
**Consequences:** at the moment an Ask blocks, *no sibling in the batch has
executed*; at most one Ask pends per loop at a time; concurrent Asks happen
only **across loops** (parent/child sub-trees — the CLI serializes those
prompts with a gate mutex, agent-cli/src/approval.rs "spec D12").
`ApproveAlways` is handled **byte-identically to `Approve`** at the only
decision site (~loop_.rs:1385) — no grant table exists at any layer.

**Channels.** `ApprovalRequest { intent, display }` has **no id field**
(engine.rs:18-21); the wire correlation id is minted *inside*
`IpcApprovalChannel::request` as `c{counter}` (agent-server/src/
approval.rs:42), held in `Mutex<HashMap<id, oneshot::Sender>>`, **300s
timeout → auto-deny** (approval.rs:58-63; constant `APPROVAL_TIMEOUT` in
session.rs:26). `ApprovalResponse::Deny` is a unit variant; the wire
`Decision` enum derives `Copy` (wire.rs:261-277). CLI `TerminalApproval`
has its **own independent 300s → deny** (agent-cli/src/approval.rs).
`Session::approve` resolves the shared pending map by id — first responder
wins, second is a silent no-op; `set_event_out` **replaces** the single
subscriber slot (session.rs:89-90), so a second attached frontend clobbers
the first's event stream.

**Children.** Children share the parent's `Arc<dyn ApprovalChannel>`
(dispatch.rs:844; field at :404). The `sub{n}:` id rewrite lives in
`SubagentSink::emit` and touches **only ToolStart/ToolResult** event ids
(dispatch.rs:198,214); child `AgentEvent::Approval` events fall into the
sink's `other` arm and are never forwarded typed. The grandchild
`id_prefix` assignment (~dispatch.rs:722-724) is unrelated to approvals.
**The G4 gap, precisely:** (a) `ApprovalRequest` carries no origin fields;
(b) the channel-minted `c{n}` id cannot tie back to a delegation; (c) child
approval activity is invisible in the typed subagent stream.

**Persistence & identity.** `session_id` exists **only inside the trace
sink**, minted as `{secs}-{pid}` (agent-runtime-config/src/trace.rs:162-167)
— it embeds the dead daemon's PID, is minted fresh per CLI launch, and is
absent entirely when `trace: false` (build_trace returns None). Sessions
are **flat** `sessions/{id}.jsonl` files; `Session` has no id field and its
`workspace` is an in-memory `Mutex<PathBuf>` reset on switch — **nothing
persists a session→workspace binding**. Trace files are deliberately
`0o600` (trace.rs:43-50) — the file-mode precedent checkpoints must match.
Trace JSONL skips `Approval`/`RunStart` events and child deltas. **No
resume path exists at any granularity.**

**Serializability.** `Message` (agent-model/src/types.rs:12) derives only
`Debug, Clone` — **no serde**; `ToolCall`/`Role` do derive serde. So
history is serde-*ready*, not serde-*done* (one derive line = 4B-1 work).
`CuratedContext::new` hard-codes empty history with **no restore seam**,
and holds more pinned state than messages: `compaction_summary`,
`folded_facts`, `folded_sections`, `memory_index`, `seq`,
`history_has_spans`/`incomplete` flags, todos handle. Guardrail tallies
live in `RunShared` (`Arc<Mutex<HashMap<TypeId, Box<dyn Any>>>>`,
middleware.rs:91): `ToolCallCount(pub usize)` and the **private**
`ModelCallCount(usize)` — the *container* is non-serializable, but the
individual counters are extractable in-crate via `RunShared::with`; the
middleware structs themselves hold only caps, no tallies. Torn-counter
doc'd property: counters may only over-count (fail-safe direction).
Non-serializable/process-bound: middleware trait objects,
CancellationToken, claude-cli per-client session pooling.
`SessionArtifacts` = two `Arc<dyn Backend>` (MemBackend by default,
artifacts.rs:9-21); the Backend trait exposes ls/read/write/glob — but
`glob` is capped at 500 results, so tree walks must use recursive `ls`.
`subagent_max_depth` defaults to **1** (runtime_config.rs:315-317).

## 2. Design

### 2.1 Session descriptor (Slice 4B-0)

Session identity moves from the trace sink to the server session: a stable
id (no PID component) minted at session creation, a per-session directory
`~/.rusty-agent/sessions/<session_id>/`, and `descriptor.json` recording
id, workspace path, created-at, and config provenance — written at run
start, before any checkpoint could exist. `TraceWriter` and the CLI take
the id as input. The startup index scans descriptors to rebuild the
session→workspace map, which resume needs to re-derive policy floors,
memory scope, and skills from **current** config against the **recorded**
workspace (§3.3). Dirs `0o700`, files `0o600` (including atomic-rename
temp files — created `0o600`, never mode-widened by rename).

### 2.2 Checkpoint data model & integrity (Slice 4B-1)

`agent-core/src/checkpoint.rs`:

```
Checkpoint {
  version: u32,                    // bump on breaking shape change
  session_id, run_id,
  subagent_path: Vec<String>,      // [] = parent; ["sub1"] etc.
  context: CuratedContextState,    // messages + goal + compaction_summary
                                   //  + folded_facts + folded_sections
                                   //  + seq + history flags + todos
  guardrails: { tool_calls: u64, model_calls: u64 },  // from RunShared
  turn: u64,
  parked: {                        // the interrupt point (§2.3)
    assistant_message,             // the turn's model output
    tool_calls: Vec<ToolCall>,     // full parsed batch
    gate_outcomes: Vec<GateOutcome>, // decisions for calls before the
                                   //  parked index (Ready / Rejected)
    parked_index: usize,
    request: ApprovalRequest,      // id-less; intent + display + origin
  },
}
```

**Explicit serialization, no trait extension.** Only three stateful pieces
persist — the two guardrail tallies (read in-crate via `RunShared::with`;
`ModelCallCount` becomes `pub(crate)`) and the todo list (via its handle,
inside `CuratedContextState`). `StuckDetection` and `RepairMiddleware`
reset on resume (their signal is the live streak). The `Middleware` trait
is **unchanged** — the persistence set stays an auditable three-item list
(serves §3.8). Memory index re-loads via the existing 4A dirty-flag path.
A child's captured `ResponseCapture` payload, if any, parks inside that
child's own checkpoint.

**Integrity (E6b).** The checkpoint directory carries a manifest with an
HMAC-SHA256 over the checkpoint payload(s), keyed from the daemon-local
secret file (4B-0). Failed MAC ⇒ treated exactly as corrupt (§4). This,
plus §2.4's re-derived display, closes forged-grant / forged-tally /
swapped-args tampering by anyone who cannot read the secret; an attacker
who can read `~/.rusty-agent/secret` is same-trust-domain (accepted, §0).

**Storage** (all writes atomic temp+rename, full-filename temp names,
`0o600`):

```
~/.rusty-agent/sessions/<session_id>/
  descriptor.json                  # 4B-0
  checkpoint/
    parked.json + manifest         # exist only while parked (E1)
    artifacts/                     # SessionArtifacts dump (§2.3)
    children/sub{n}/               # same layout, recursive (D3)
```

### 2.3 Park (inside the Phase-1 gate loop)

When `gate_tool` receives `Decision::Ask`, **before** blocking on the
channel, the loop writes the checkpoint: full context state, guardrail
tallies, the turn's parsed batch, gate outcomes already decided for earlier
calls in the batch (Ready/Rejected — none has *executed*; Phase 2 hasn't
started), the parked index, and the id-less `ApprovalRequest` with origin
fields. The SessionArtifacts trees are dumped at the same moment via
recursive `ls` + `read` (Backend-trait-level, backend-agnostic; not
`glob`, which caps at 500). Then it blocks as today.

- **Answer on the live channel:** deleting `parked.json` (+manifest) is the
  **atomic commit point** of the answer — resolve first deletes the park,
  then the loop proceeds; a resolve that finds no park file is a stale
  duplicate and is a no-op. Hot-path behavior is otherwise unchanged.
- **Timeouts (E5):** the server channel's 300s auto-deny is removed; an
  unanswered Ask parks indefinitely. A new optional config knob restores
  auto-deny-after-N-seconds for headless callers (the park file is still
  written first; auto-deny deletes it like any answer). The CLI's
  interactive timeout parks-and-exits with a reopen hint.
- **Checkpoint write failure:** fall back to today's live-only behavior for
  that Ask (log + event); never block the run on checkpoint I/O errors.
- Ordinary (non-Ask) runs perform **zero** checkpoint I/O (E1, §3.1).

### 2.4 Resume (attach-to-resume)

At daemon startup, the sessions index marks sessions whose checkpoint dir
holds a valid `parked.json`. On frontend attach:

1. **Verify** MAC + version; on failure refuse to resume and surface
   honestly ("checkpoint unreadable; run cannot be resumed") — never
   silently start fresh over it, never guess.
2. **Rebuild machinery** via `assemble_loop` against the descriptor's
   workspace and **current** config — fresh middleware stack, RunShared,
   CancellationToken, model client (a fresh claude-cli session enrolls; the
   old pool is gone — delta-resume/cache implications are a follow-on
   knob, §5). Policy engine and 3B-1c floors re-derive from current config:
   **config is live truth, conversation is checkpointed truth** (§3.3).
3. **Restore state**: the new `CuratedContext` restore constructor rebuilds
   the full pinned context (messages, compaction ledger, folded sections,
   seq, todos); guardrail tallies restore with a **monotonic clamp** — a
   restored tally may never be lower than what the checkpointed history
   implies (preserves the over-count-only fail-safe; below-implied ⇒
   corrupt ⇒ refuse); artifacts restore into fresh Mem backends.
4. **Re-derive the approval display from the stored args** through the same
   `tool.intent()` path used live — the persisted `display`/`summary` is
   advisory and never shown from disk, so what the human approves is
   provably what will run (closes the see-benign/run-hostile TOCTOU).
5. **Re-emit** pending approvals — all parked loops in the tree (§2.5),
   each with attribution — under fresh channel-minted correlation ids.
   **Daemon-alive reattach:** if the in-memory oneshot for a parked Ask is
   still live (the daemon never died), the existing pending entry is
   re-emitted as-is — no second id is minted beside a live one.
6. On answer: park deletion commits (§2.3); the loop re-enters the Phase-1
   gate loop **at the parked index** — earlier gate outcomes are reused,
   not re-prompted (including across restart: no double-gate); calls after
   the parked index gate normally (a later Ask parks again); Phase 2 then
   executes the approved set fresh. Nothing executed pre-park, so nothing
   can double-execute. `ApproveAlways` answered before the restart is
   **not** remembered (E2): a resumed run re-asks.

Multiple frontends: `approval_resolved` broadcasts retract the prompt on
other attached surfaces (4B-2); the first answer wins by the commit-point
rule, later answers are no-ops.

### 2.5 Child checkpoint tree (D3)

A child's checkpoint lives under the parent's `children/sub{n}/`,
recursively (grandchildren compose the same way; E6a scopes live-drive
assertions to depth 1). A parent blocked in Phase 2 awaiting a dispatch
result is *not* parked — only loops blocked at a gate park; since parallel
dispatches run concurrently, **several children can be parked at once**,
and attach re-emits every parked Ask in the tree. On resume, the parent
re-enters its turn; its dispatch call, finding a child checkpoint,
reconstructs the child loop from it (same restore path, recursively)
instead of building a fresh child, and the child resumes at its own parked
index. From the parent's view the dispatch call is an in-flight tool call
whose result arrives when the resumed child finishes. **Child deadlines:**
a resumed child gets a **fresh** dispatch timeout (a park can outlast any
deadline; the human attach+approve in the loop makes clock-reset abuse
moot — recorded in §7, child-deadline row).

### 2.6 Approval attribution (G4)

`ApprovalRequest` gains optional origin fields: **delegation id** (the
dispatch call's on-wire id — `id_prefix + call_id`, the same key the 3B-2
subagent stream joins on), **sub-agent name** (registered name or
`general-purpose`), **depth**. Mechanism: dispatch wraps the shared
channel in an **attributing decorator** (`AttributingApprovalChannel`,
wrap-at-dispatch — the 3B-1c `child_policy` precedent) that stamps origin
onto every request a child issues; no id-rewrite path is involved (the
`sub{n}:` sink rewrite never touches approvals — §1). The wire
`ApprovalRequest` frame carries the origin fields; desktop modal renders
"Sub-agent *name* wants to run …" and the SubagentCard (keyed by
delegation id) shows a `waiting-approval` state; CLI prompts prefix the
child name. Parent approvals are unchanged in shape (fields absent).

### 2.7 Wire & surfaces (all additive; Slice 4B-2 unless noted)

- `ApprovalRequest` frame: origin fields (4B-1, needed by the live drive).
- Deny-feedback: `ApprovalResponse::Deny` gains an optional feedback
  string (the *policy* `Decision::Deny(String)` reason in agent-policy is
  a different, pre-existing type — untouched); the wire `Decision::Deny`
  carries it, **dropping the enum's `Copy` derive** — the match-site
  fan-out across wire.rs, agent-server, agent-cli, src-tauri, and web is
  enumerated in the plan. Feedback becomes the denial tool-result text the
  model sees (today's bare refusal remains the no-feedback rendering).
- New frames: `parked_runs` (list on attach), `approval_resolved`
  (broadcast retraction), `resumed` notice. No frame removed or reshaped —
  3B-2's additive-frames discipline holds.
- Desktop/web: parked session renders its trace transcript + banner + the
  re-emitted modal(s); optional feedback field on deny.
- CLI: session listing marks parked; reopen re-prompts inline with
  attribution + feedback line; interactive timeout parks-and-exits (E5).
  Ctrl-C with an Ask pending = "left parked" messaging (the park already
  exists).

### 2.8 Sweep list (prose/tests that mention approval semantics)

Candidates to check during planning: approval-timeout wording in server
docs/config (+ new E5 knob docs), wire.rs frame docs, web approval
modal/reducer tests, CLI help text, AGENTS.md surface docs, trace.rs
comment on skipped Approval events, config.example.json, the
`docs/okf/deepagents-refactor/` gap rows (HITL / Durable execution flip to
"partial/match" — bundle update is a follow-on, not this cycle).

## 3. Invariants (do-not-regress)

1. **Runs that never hit Ask are byte-identical** — zero checkpoint I/O on
   the non-Ask path (E1); the only behavior deltas anywhere are the removed
   auto-deny (E5) and optional origin fields on approval frames.
2. **`ToolIntent` policy richness untouched** — parking wraps the *outcome*
   of `check()`, never the engine. No policy rule changes.
3. **Config is live truth, conversation is checkpointed truth.** Resume
   re-derives policy engine + 3B-1c floors from current config against the
   descriptor's workspace; a checkpoint can never widen policy, floors,
   grants (E2), or tallies (monotonic clamp, §2.4 restore step 3).
4. **Approval display integrity:** what the human sees on resume is
   re-derived from the stored args via `tool.intent()` — never a trusted
   stored string.
5. **Additive wire protocol** (no removed/reshaped frames; `Copy` loss on
   the wire `Decision` enum is source-level, not wire-level).
6. **Trace JSONL contract unchanged** (audit log stays an audit log;
   checkpoints are a separate artifact). TraceWriter consumes the 4B-0 id;
   file naming/shape/`0o600` unchanged.
7. **No double-gate, no pre-park execution:** earlier gate outcomes in the
   parked batch are reused on resume, never re-prompted; no tool in the
   batch executes before the park (Phase 2 hadn't started) —
   mutation-verified.
8. **Guardrail tallies survive restart** (no budget refill; clamp
   direction preserves the over-count-only fail-safe).
9. **Child quarantine preserved** — resume rebuilds children through the
   same dispatch path (registry floors, tool allowlists, no memory recall
   in children, ToolCallLimit, ResponseCapture precedence).
10. **4A memory semantics untouched** (index-first pinned block, dirty-flag
    refresh; the memory dir is not checkpointed — it's already durable).

## 4. Error handling

- Corrupt / MAC-failed / version-mismatched / partial checkpoint: **refuse
  to resume**, surface honestly, never silently start fresh over it, never
  fall back to guessing. Versioned format makes future migrations explicit.
- Crash windows: before the park write ⇒ run lost (D1, today's behavior);
  during the park write ⇒ temp+rename leaves either no park or a complete
  one, and the MAC catches torn trees; between write and emit ⇒ park file
  is the source of truth, re-emit on attach; during resume ⇒ the park file
  persists until the commit point, so resume is re-runnable; crash after
  answer-commit (park deleted) but before the turn completes ⇒ run lost
  from that point (D1 — no mid-run recovery).
- Resume `assemble_loop` failure (model backend gone, workspace missing):
  surface as a normal run error on the attached frontend; park retained.
- Checkpoint write failure at park time: degrade to live-only for that Ask
  (log + event); never block the run on checkpoint I/O.
- Stale descriptor (workspace deleted/moved): refuse resume with the path
  named; the parked entry stays listed so the user can see why.

## 5. Deferred / future (named)

- **edit / respond decisions** (D2) — arg-editing UX + schema re-validation
  + synthetic-result executor path.
- **ApproveAlways grant store** (E2) — live always-grant semantics first,
  then checkpoint persistence; must honor 4A-E4's distinct-approval
  condition for any global scope.
- **Time travel / checkpoint history** — the versioned format is the hook.
- **Mid-run crash recovery for non-parked runs** (E1 cut; would need the
  replay-hazard design the gate declined this cycle).
- **Declarative interrupt_on-style rules** (which tools ask, data not code).
- **Server replay buffer** for late-joining clients (3B-2 deferral).
- **claude-cli delta-resume/cache knob** for resumed sessions (§2.4
  rebuild step 2).
- **Checkpoint retention/GC** beyond delete-on-completion, if parked trees
  ever accumulate in practice.
- **Bundle gap-analysis row updates** after merge.

## 6. Testing

- Checkpoint serde round-trip + MAC verify/reject + version-mismatch
  refuse (unit).
- Park write-through: Ask parks (file exists, `0o600`, complete); live
  answer deletes park before proceeding (commit point); resolve-without-
  park is a no-op; non-Ask runs write nothing (E1 pin).
- Resume-splice: earlier gate outcomes reused, no re-prompt, no pre-park
  execution — **mutation-verified** (invariant §3.7; the 3B-1b sever-test
  lesson: single-turn script shapes matter).
- Guardrail tally persistence + monotonic clamp (below-implied ⇒ refuse).
- Tamper: edited args ⇒ re-derived display shows the real command; edited
  payload ⇒ MAC refuses.
- Child tree: parked child resumes in place, parent dispatch rebinds;
  multiple children parked at once, all re-emitted; grandchild composition
  (unit-level per E6a).
- Attribution: origin fields populated for child asks, absent for parent;
  web reducer tests for modal attribution + `waiting-approval` card state
  + parked banner.
- Deny-feedback reaches the model as tool-result text (native + prompted
  protocols).
- Cross-surface: sequential attach — surface A answers, a later-attached
  surface B sees no re-emitted prompt and an `approval_resolved` was
  emitted; cross-process first-answer-wins is enforced by the 4B-2
  `resume.lock` claim + the answer commit point. *(Amended at the 4B-2
  plan gate, 2026-07-10: the single-subscriber event slot makes literally
  simultaneous frontends unrepresentable — owner-ratified; see the 4B-2
  plan's Panel & review log.)* Daemon-alive reattach reuses the live
  pending id (no duplicate).
- E5: headless knob auto-denies after N and deletes the park; CLI timeout
  parks-and-exits.
- One **live kill-restart WebDriver drive** (3B-2 precedent): trigger a
  child Ask, kill the daemon, restart (**genuinely new pid**), attach,
  observe attributed modal, approve, assert the run completes (depth-1 per
  E6a).
- 4B-0: descriptor written at run start; startup index rebinds workspace;
  trace consumes the id (filename parity pinned).
- Full `bash scripts/ci.sh` green per slice.

## 7. Resolved questions (were §7 open questions, pre-panel)

- **Park-write vs. siblings:** dissolved — the gate loop is sequential;
  the real requirement is persisting *gate outcomes* so a resumed batch
  never re-prompts (§2.3, invariant §3.7).
- **ApproveAlways scope:** resolved by E2 — checkpoints carry no standing
  approvals; grant store deferred.
- **Child deadline:** fresh timeout on resume; human-gated attach makes
  clock-reset abuse moot (§2.5).
- **claude-cli session pooling:** fresh enrollment on resume; delta-resume
  knob deferred (§5).
- **turn.json replay risk:** moot — turn.json cut (E1).

## Panel & review log

- **2026-07-10 — brainstorm:** owner walked D1–D5 and approved all five
  design sections (pre-panel draft committed d5c8313).
- **2026-07-10 — adversarial panel** (4 opus reviewers, distinct mandates:
  Requirements / Assumptions / Failure & abuse / Scope & simpler design;
  all APPROVE-WITH-FIXES, none REJECT).
  **Blockers/majors FIXED IN PLACE:**
  - Session identity/workspace binding doesn't exist at baseline (trace-
    owned `{secs}-{pid}`, trace-gated, flat JSONL) → net-new 4B-0
    foundation slice (Requirements-B1 + Scope-B1, independently converged).
  - §2.3's concurrency model contradicted the two-phase gate loop
    ("approval prompts never overlap") — no sibling executes before a park;
    park schema rewritten to gate-outcomes + parked-index; "settled sibling
    results" deleted; multiple-Asks-per-turn dissolved into the real
    cross-loop multi-park case (Failure-F1 + Assumptions-M4, converged;
    also retires Requirements-B2).
  - `Message` has no serde derives (claimed "cleanly serializable");
    `CuratedContext` needs a restore seam + full pinned-state coverage
    (compaction ledger, folded sections, seq) (Assumptions-B1, m5).
  - Guardrail tallies live in `RunShared`, not middleware structs; spec now
    serializes the three concrete states directly in-crate, `Middleware`
    trait unchanged (Assumptions-B2 + Scope-m6).
  - G4 mechanism mis-located: no approval id exists; `sub{n}:` rewrite is
    sink-side only; attribution redesigned as a wrap-at-dispatch
    attributing channel with explicit origin fields (Requirements-M2 +
    Assumptions-M3 + Failure-F9).
  - TOCTOU: approval display re-derived from stored args on resume; park
    deletion = atomic answer commit point; daemon-alive reattach reuses the
    live pending id; `approval_resolved` retraction frame (Failure-F2(1),
    F5 + Requirements-M4).
  - File modes `0o600`/`0o700` incl. temp files (trace precedent)
    (Failure-F6); tally monotonic clamp (Failure-F10); artifacts dump via
    recursive `ls` not 500-capped `glob`, Backend-trait-level
    (Assumptions-m6 + Requirements-m2); deny-feedback retargeted to
    `ApprovalResponse`/wire `Decision` with the `Copy`-loss fan-out named
    (Scope-m5); child deadline resolved fresh-on-resume (Failure-F11);
    test rows added: new-pid restart, multi-park tree, cross-surface
    first-answer-wins (Requirements-m1).
  **ESCALATED TO THE GATE (decisions above):** turn.json overbuild + replay
  hazard + write amplification/GC (Scope-M2/M3 + Failure-F3/F4/F7 → E1
  CUT); ApproveAlways persistence without a grant store + forged-grant
  abuse (Requirements-M1 + Assumptions-m7 + Failure-F2(2) → E2 STRIKE);
  CLI substrate is net-new vs D4 (Requirements-M3 + Scope-M4 → E3 keep,
  own slice); slice split (Scope-M4 → E4 three slices); auto-deny removal
  vs headless flows + CLI's independent timeout (Failure-F8 +
  Requirements-m3 → E5 park default + knob); grandchild depth cost
  (Scope escalation → E6a depth-1 live); checkpoint tamper resistance
  (Failure-F2(4) → E6b HMAC hardening this cycle, residual beyond it
  accepted explicitly).
  **MINORS ACCEPTED AS RESIDUAL:** local attacker who can read the
  daemon-local secret is same-trust-domain (§0 OUT); grandchild live-drive
  coverage capped at depth 1 (E6a); parked runs hold their session slot
  until answered or auto-denied via the E5 knob (inherent to parking).
- **2026-07-10 — owner gate:** E1–E6 CLOSED (recorded above).
- **2026-07-10 — 4B-0 whole-branch gate:** reviewer escalated one spec-IN
  narrowing (startup index shipped as unwired capability); owner RATIFIED
  — wiring moves to 4B-1 (§0 updated in place).
- **2026-07-10 — light-tier consistency read (sonnet):** CLEAN on all
  substance (no stale E1/E2 language, dispositions match normative
  sections, slice assignments consistent); 4 mechanical citation fixes
  applied (§2.4 list-item pseudo-headings ×2, §3.7→§3.8 miscite, orphan
  "OQ3" label). Owner spec review pending.
- **2026-07-10 — 4B-2 plan gate (owner):** two plan-review escalations
  decided — **P1** cancel-while-parked retains the park on BOTH surfaces
  (fixes a verified baseline bug where the cancel arm cleared the park;
  parks are cleared only by answers — a server-side behavior change,
  accepted); **P2** §6's cross-surface row amended in place from
  "two attached frontends" to sequential-attach (single-subscriber slot;
  cross-process first-answer-wins = the 4B-2 `resume.lock` + answer
  commit point; multi-subscriber broadcast stays deferred). Also folded
  into 4B-2 per the 4B-1 merge-gate dispositions: resumed-run trace
  attribution + in-life failed-resume retry. Details in
  `docs/superpowers/plans/2026-07-10-durable-hitl-4b2.md`.
