# Durable HITL — checkpointing foundation + resumable interrupts (deepagents refactor, Phase 4B) — design

**Status:** BRAINSTORM-APPROVED (owner walked all five design sections
2026-07-10), **PRE-PANEL**. Adversarial spec panel per AGENTS.md § How we work
runs next; its synthesis arms the owner gate. Not planned, not implemented.

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
dependency; scope deliberately" — is honored by checkpointing exactly the two
points where a run can legally pause, not every transition.

**Live-source baseline:** commit 8b54fda (4A-1 merged, Phase 4A complete),
surveyed 2026-07-10 (Explore source map, 48 tool uses). All `file:line`
anchors are orientation only — **locate quoted code by content before
editing.**

**Builds on:** Phase-1 middleware seam (707d7fd), Phase-2 backend seam
(71e23d1), Phase-3A guardrails/repair/todos (cb6ddf0), Phase-3B registry/
permissions/structured-response/typed-stream (4cf682d, 68e846b, 3490590,
602ae5d), Phase-4A file-based memory + `~/.rusty-agent` root (d482572,
8b54fda). Preserves all prior-phase invariants (§3).

## Owner decisions (brainstorm, 2026-07-10)

- **D1 — Durability level: DAEMON-RESTART AT APPROVAL.** Pending approval +
  checkpoints persist to the session dir. A run parked at `Ask` survives
  frontend disconnect and daemon restart. No general mid-turn crash recovery
  beyond the `turn.json` fallback (§2.4); no time travel (versioned format
  leaves the door open, §5).
- **D2 — Decision vocabulary: EXISTING THREE + DENY-FEEDBACK.** Approve /
  ApproveAlways / Deny, with a new optional `feedback` string on Deny that
  becomes the tool-result text the model sees. deepagents' **edit** and
  **respond** decisions are deferred to a named follow-on (§5) — they add
  arg-editing UX and a synthetic-result executor path orthogonal to
  durability.
- **D3 — Child scope: ATTRIBUTION + CHILD CHECKPOINTS.** G4 fixed properly
  (ApprovalRequest carries delegation id / sub-agent name / depth) **and**
  children get the same checkpoint machinery, composed as a `sub{n}`-keyed
  tree under the parent's checkpoint; a restart resumes a parked child in
  place. (Owner chose the widest option over parent-only resume.)
- **D4 — Resume UX: ATTACH-TO-RESUME, BOTH SURFACES.** The daemon indexes
  parked runs from disk at startup; attaching to the session (desktop open,
  CLI reopen) re-emits the pending approval with attribution; answering
  resumes the loop. Parked runs are listable. No new resume command.
- **D5 — Architecture: TWO-TIER SNAPSHOTS** (approach A over trace-replay
  and a generic LangGraph-style checkpointer). Rolling end-of-turn
  checkpoint + parked checkpoint at the interrupt point; opt-in middleware
  snapshot hooks; everything non-serializable rebuilt fresh by
  `assemble_loop` on resume.

## 0. Scope

**IN:**
- `agent-core/src/checkpoint.rs`: versioned serde `Checkpoint` + tree
  storage under the session dir; atomic writes.
- `Middleware` trait: defaulted `snapshot`/`restore` methods; implementations
  for ToolCallLimit, ModelCallLimit, TodoList (persist) — Stuck/Repair reset.
- Loop park path at `Decision::Ask` (write-through `parked.json`), server
  channel auto-deny removal, resume-splice at the pre-execution point.
- SessionArtifacts dump/restore (`checkpoint/artifacts/`).
- Dispatch: child checkpoint tree + resume rebinding; ApprovalRequest
  attribution issued after the `sub{n}:` id rewrite (G4).
- Wire (additive): Deny `feedback`, `parked_runs` frame, approval re-emit on
  attach, `resumed` notice.
- Frontends: desktop modal attribution + parked banner + deny-feedback
  field + SubagentCard `waiting-approval` state; CLI parked listing +
  attributed re-prompt + feedback line.
- Tests incl. one live kill-restart WebDriver drive; full ci.sh.

**OUT (named, §5):** edit/respond decisions; time travel / checkpoint
history; mid-tool-execution checkpointing; general crash recovery for
non-parked runs beyond `turn.json`; declarative permission rules
(interrupt_on-style data config); server event replay buffer (3B-2
deferral, adjacent but separate); PII/retention policy for checkpoint
contents beyond what trace JSONL already implies.

## 1. Current state (survey 2026-07-10, baseline 8b54fda)

- `PolicyEngine::check` → `Decision::{Allow,Ask,Deny}`
  (agent-policy/src/engine.rs:31-33). On `Ask` the loop creates an
  `ApprovalRequest` and blocks on `approval.request(req)` under
  tokio::select with cancellation (agent-core/src/loop_.rs:1357-1388).
- Server channel `IpcApprovalChannel` holds `Mutex<HashMap<id,
  oneshot::Sender>>`, emits `ServerEvent::ApprovalRequest`, waits with a
  **300s timeout → auto-deny** (agent-server/src/approval.rs:14-66).
  CLI `TerminalApproval` blocks a stdin thread behind a gate mutex
  (agent-cli/src/approval.rs:61-88). Wire decisions are
  Approve/ApproveAlways/Deny (agent-server/src/wire.rs:69-76, 261-277).
- **Nothing about a pending approval is persisted.** Daemon death between
  emit and answer loses the run silently.
- Children share the parent's `Arc<dyn ApprovalChannel>`
  (dispatch.rs:404); the ApprovalRequest is issued **before** the
  `sub{n}:` id rewrite (dispatch.rs:722-724), so approval prompts carry a
  child-local id — the G4 attribution gap.
- Persistence today = trace JSONL (`~/.rusty-agent/sessions/
  {session_id}.jsonl`), which skips `Approval` and `RunStart` events and
  child deltas. **No resume path exists at any granularity.**
- Message history (`CuratedContext.history: Vec<Message>`) is cleanly
  serializable. Non-serializable/process-bound: middleware trait objects,
  `RunState`/`RunShared` (TypeId-keyed Any-boxes), CancellationToken,
  claude-cli session pooling. Since Phase 2, offloaded large tool results
  and evicted history spans live in **SessionArtifacts = two in-memory Mem
  backends** — lost on restart unless checkpointed.

## 2. Design

### 2.1 Checkpoint data model & storage

`agent-core/src/checkpoint.rs`:

```
Checkpoint {
  version: u32,                    // bump on breaking shape change
  session_id, run_id,
  subagent_path: Vec<String>,      // [] = parent; ["sub1"] etc.
  history: Vec<Message>,           // CuratedContext history
  goal: ..., todos: ...,           // pinned-block inputs
  middleware: Map<String, Value>,  // keyed by middleware name, opt-in
  turn: u64,
}
```

Two defaulted `Middleware` methods:
`fn snapshot(&self, ...) -> Option<serde_json::Value>` and
`fn restore(&self, Value, ...)`. Stateless middleware ignore them.
**Persist:** ToolCallLimit / ModelCallLimit tallies (a restart must not
refill a guardrail budget — abuse vector otherwise), TodoList (plan state).
**Reset:** StuckDetection, RepairMiddleware (their signal is the live
streak; resetting is honest). Memory index re-loads via the existing 4A
dirty-flag path — no snapshot needed. Child-only ResponseCapture: a
captured payload parks with the child's checkpoint.

Storage, all writes atomic temp+rename (4A-1 precedent, full-filename temp
names):

```
~/.rusty-agent/sessions/<session_id>/checkpoint/
  turn.json          # rolling end-of-turn snapshot, overwritten each turn
  parked.json        # exists only while an interrupt is parked
  artifacts/         # SessionArtifacts Mem-backend dump (§2.2)
  children/sub{n}/   # same layout, recursive (D3)
```

Rolling only — no checkpoint history, no time travel (v0).

### 2.2 SessionArtifacts dump/restore

The end-of-turn checkpoint dumps both SessionArtifacts Mem-backend file
trees to `checkpoint/artifacts/`; resume restores them into fresh Mem
backends. Without this, a resumed run's `[tool_result#N offloaded ...]`
pointers and `conversation_history/history.md` dangle and recall silently
breaks.

### 2.3 Park (write-through at Ask)

On `Decision::Ask`, before blocking, the loop writes `parked.json`:
in-flight assistant message, full parsed tool-call list, results of
already-settled siblings, pending call id, and the full `ApprovalRequest`
(with §2.6 attribution). Sibling tool calls execute concurrently; the park
write happens once in-flight siblings settle (already-approved work is not
cancelled). The server channel's 300s auto-deny is **removed**: an
unanswered Ask parks indefinitely. If the answer arrives on the live
channel, `parked.json` is deleted and the loop proceeds exactly as today —
parking is a write-through, not a detour; hot-path behavior is otherwise
unchanged (§3 pins).

### 2.4 Resume (attach-to-resume)

At daemon startup, sessions with `parked.json` are indexed as parked. On
frontend attach: re-run `assemble_loop` (fresh middleware stack, RunShared,
CancellationToken, model client — the non-serializable machinery rebuilt by
the code that already knows how), restore history + middleware snapshots +
artifacts, and re-enter the loop **at the pre-execution point of the parked
call**: settled sibling results are spliced in, not re-executed; the model
call is not re-issued. The pending approval re-emits with a fresh
correlation id and identical summary. Approve / deny-with-feedback then
flows the normal path. `ApproveAlways` grants persist in the checkpoint so
a resumed run doesn't re-ask what was already granted always.

**Crash windows.** Crash mid-turn with no Ask: resume falls back to
`turn.json` — the turn replays from its start, which may re-issue the model
call; tool side effects from the lost turn are the honest, stated cost.
Crash between park-write and emit: the park file is the source of truth;
re-emit on attach.

### 2.5 Child checkpoint tree (D3)

A child's checkpoints live under the parent's `children/sub{n}/`,
recursively (grandchildren compose). If the parked approval belongs to a
child, the resumed parent re-enters its dispatch tool call, which — instead
of building a fresh child — reconstructs the child loop from its checkpoint
and resumes it at its own parked point. From the parent checkpoint's view
the dispatch call is just an in-flight tool call whose result arrives when
the resumed child finishes. Child end-of-turn checkpoints ride the same
mechanism (the child loop is the same loop).

### 2.6 Approval attribution (G4)

`ApprovalRequest` gains `Option` origin fields: delegation id (the dispatch
call's on-wire id — the same key the 3B-2 subagent stream uses), registered
sub-agent name, depth. Issued **after** the `sub{n}:` rewrite so the wire
id is globally meaningful. Desktop modal: "Sub-agent *name* wants to run
…"; SubagentCard gains a `waiting-approval` state (no more looks-hung);
CLI prompt prefixes the child name. Parent approvals unchanged in shape.

### 2.7 Wire & surfaces (all additive)

- `Decision::Deny` gains optional `feedback: String` → becomes the
  tool-result text the model sees (today's deny is a bare refusal).
- New/extended frames: `parked_runs` (list on attach), approval re-emit on
  attach when a park exists (existing frame), `resumed` notice. No frame
  removed or reshaped — 3B-2's additive-frames discipline holds.
- Desktop/web: parked session renders its trace transcript + banner + the
  re-emitted modal; small optional feedback field on deny.
- CLI: session listing marks parked; reopening re-prompts inline with
  attribution + optional feedback line. Ctrl-C while an Ask is pending =
  "leave it parked" (the park file already exists; messaging only).

### 2.8 Sweep list (prose/tests that mention approval semantics)

Candidates to check during planning: approval-timeout wording in server
docs/config, wire.rs frame docs, web approval modal/reducer tests,
CLI help text for sessions, AGENTS.md surface docs, trace.rs comment on
skipped Approval events, `docs/okf/deepagents-refactor/` gap rows (HITL /
Durable execution flip to "partial/match" — bundle update is a follow-on,
not this cycle).

## 3. Invariants (do-not-regress)

1. **Non-parked runs byte-identical** except the removed 300s auto-deny:
   same events, same policy decisions, same pinned-block rendering. The
   end-of-turn checkpoint write is additive I/O only.
2. **`ToolIntent` policy richness untouched** — parking wraps the *outcome*
   of `check()`, never the engine. No policy rule changes.
3. **Approval security floor:** a resumed run re-parses config for its
   policy engine — config is live truth, conversation is checkpointed
   truth. A checkpoint can never smuggle a wider policy/floor than current
   config grants (3B-1c floors re-derive at resume).
4. **Additive wire protocol** (no removed/reshaped frames).
5. **Trace JSONL contract unchanged** (audit log stays an audit log;
   checkpoints are a separate artifact).
6. **Sibling non-re-execution on resume** (mutation-verified test).
7. **Guardrail tallies survive restart** (no budget refill).
8. **Child quarantine preserved** — resume rebuilds children through the
   same dispatch path (registry floors, tool allowlists, no memory recall
   in children).
9. **4A memory semantics untouched** (index-first pinned block, dirty-flag
   refresh; memory dir is not checkpointed — it's already durable).

## 4. Error handling

- Corrupt / version-mismatched / partial checkpoint: **refuse to resume**,
  surface honestly ("checkpoint unreadable; run cannot be resumed"), never
  silently start fresh over it. Versioned format makes migrations explicit.
- Parked run whose workspace or config changed: resumes with checkpointed
  conversation + fresh engine from current config (§3.3).
- Resume `assemble_loop` failure (e.g. model backend gone): surface as a
  normal run error on the attached frontend; park file retained.
- Checkpoint write failure at park time: fall back to today's live-only
  behavior for that Ask (log + event), never block the run on checkpoint
  I/O errors.

## 5. Deferred / future (named)

- **edit / respond decisions** (D2) — arg-editing UX + schema re-validation
  + synthetic-result executor path.
- **Time travel / checkpoint history** — the versioned format is the hook.
- **Declarative interrupt_on-style rules** (which tools ask, data not code).
- **Server replay buffer** for late-joining clients (3B-2 deferral).
- **Bundle gap-analysis row updates** after merge.
- **General mid-turn crash recovery** beyond `turn.json` replay.

## 6. Testing

- Checkpoint serde round-trip (conformance-style, versioned).
- Park write-through: Ask parks; live answer deletes park; non-parked-path
  byte-identity pinned.
- Resume-splice: settled siblings not re-executed (**mutation-verified** —
  the 3B-1b sever-test lesson).
- Guardrail tally persistence across a simulated restart.
- Child tree: parked child resumes in place; parent dispatch rebinds;
  grandchild composition.
- Artifacts restore: offload pointer readable after resume.
- Attribution: ApprovalRequest fields for child asks; web reducer tests for
  modal attribution + parked banner + waiting-approval card state.
- Deny-feedback reaches the model as tool-result text (both protocols).
- One **live kill-restart WebDriver drive** (3B-2 precedent): trigger Ask,
  kill daemon, restart, attach, approve-with-feedback, assert completion.
- Full `bash scripts/ci.sh` green.

## 7. Open questions for the panel

- Park-write ordering vs. concurrently-executing siblings: is
  "write after siblings settle" the right consistency point, or must the
  park file be written eagerly and patched as siblings land?
- `ApproveAlways` persistence scope: session checkpoint only, or does it
  interact with 3B-1c floors / 4A E4's distinct-global-approval condition?
- Child resume vs. `subagent_max_depth` and dispatch timeout caps: does a
  resumed child inherit its original deadline or get a fresh one?
- claude-cli session pooling: resume re-enrolls a fresh model session —
  any prompt-cache or delta-resume implications worth a knob?
- Does `turn.json` fallback risk double side effects severe enough to gate
  behind a confirm-on-attach prompt instead of auto-replay?

## Panel & review log

- **2026-07-10 — brainstorm:** owner walked D1–D5 and approved all five
  design sections (this document). Panel pending.
