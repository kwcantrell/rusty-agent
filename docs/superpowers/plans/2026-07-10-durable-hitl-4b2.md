# Durable HITL Slice 4B-2 — Surfaces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The durable-HITL machinery grows its product surfaces: a human Deny
can carry feedback the model sees, parked runs are visible (web banner,
`parked_runs` frame) and answerable from either surface (`approval_resolved`
retraction, `resumed` notice), and the CLI gains session list/reopen with a
timeout that parks-and-exits instead of denying (E5).

**Architecture:** Deny-feedback threads one optional string through the whole
decision path — `ApprovalResponse::Deny { feedback }` (agent-policy) → wire
`Decision` (compat serde, `Copy` dropped) → durable `answer.json` (feedback
MAC-bound) → a shared `denial_content()` renderer in the loop. The three new
frames are additive `ServerEvent` variants emitted from the existing attach
scan (`parked_runs`), `IpcApprovalChannel::resolve`/`retract_external_for`
(`approval_resolved`), and `start_resume` (`resumed`). The CLI wires the
4B-1 `Checkpointer` into its own loop (making CLI runs park-capable), adds a
`sessions` subcommand, and reopens a parked session through the same
scan → answer → restore → `resume_with_cancel` sequence the server uses.

**Tech Stack:** Rust (agent/ workspace: agent-policy, agent-core,
agent-server, agent-cli, agent-runtime-config), serde/serde_json, tokio,
clap; React/TS + vitest (web/); src-tauri passthrough (no bridge changes);
one live CLI park→reopen drive against the local llama server.

**Spec:** `docs/superpowers/specs/2026-07-10-durable-hitl-design.md` §0
(Slice 4B-2), §2.3–§2.4 (answer commit), §2.7 (wire & surfaces), §3, §4, §6.
Baseline: main @ 577b40d (4B-1 merged). All `file:line` anchors are
orientation only — **locate quoted code by content before editing** (repo
convention).

## Plan-level refinements (recorded for the plan review + owner)

The spec's mechanisms are honored; these are implementation-level decisions
the spec left open, plus two merge-gate deferrals folded in:

1. **Wire `Decision` keeps byte-identical encoding for the no-feedback
   case.** Spec §3.5 rules the `Copy` loss "source-level, not wire-level",
   so `Deny { feedback: Option<String> }` gets a hand-written serde impl:
   deserialize accepts BOTH the legacy bare string `"deny"` and the new
   `{"deny":{"feedback":"…"}}` object; serialize emits the bare string when
   feedback is `None` (byte parity with today) and the object only when
   feedback is present. `"approve"`/`"approve_always"` are untouched.
2. **`answer.json` MAC covers the feedback text.** Feedback becomes
   tool-result text the model reads; an unMAC'd string would let a same-host
   attacker without the secret inject model-visible content — exactly the
   E6b forged-grant class. The MAC formula changes to cover
   `(approve, feedback, manifest MAC)`. Consequence: an `answer.json`
   written by a pre-4B-2 daemon fails the new MAC after an upgrade ⇒
   `take_answer` returns `None` ⇒ the ask re-prompts live. Benign (a re-ask,
   never a wrong resume), **accepted residual** — the crash-after-commit
   window an answer file survives is seconds-to-minutes and a daemon upgrade
   inside it is vanishingly rare.
3. **`approval_resolved` is scoped to the product's real topology.** The
   server has ONE subscriber slot (`set_event_out` replaces it); two
   *simultaneously* attached frontends do not exist. The frame retracts
   stale prompts on the currently-attached surface (answer arrived from the
   CLI's cross-process reopen, from the E5 auto-deny knob, or from a
   pre-reattach answer); cross-process first-answer-wins is enforced by
   refinement 11's resume lock plus the `answer.json`/park-deletion commit
   point (spec §2.3). **⚠ ESCALATED TO THE OWNER GATE (arch review
   finding 9):** this reinterprets spec §6's "two attached frontends" test
   row from *simultaneous* to *sequential* attach — pinned at channel
   level as: attach A → answer → attach B sees no re-emitted prompt AND an
   `approval_resolved` was emitted. The literal simultaneous-two-GUIs row
   cannot exist under the single-slot architecture; owner ratification of
   the reinterpretation is requested at the gate.
4. **CLI park-and-exit = print hint, flush the trace, then
   `std::process::exit(0)`.** The park is durably written by the loop
   BEFORE it blocks on the channel (spec §2.3), so at the moment the
   terminal prompt times out the disk state is already correct; exiting
   without unwinding is what guarantees nothing on the exit path deletes
   it. Guarded on durable wiring: if the CLI built no `Checkpointer` (no
   HOME/secret), the timeout keeps today's deny. Three hazards named by
   the arch review (finding 6), all handled in Task 9: (a) the exit can
   fire from a **child** loop's ask — safe because the child always
   `write_park`s (which flushes the ancestor dispatch-kind snapshots)
   before it can block on the prompt, and the D12 gate mutex means at most
   one prompt is in flight; (b) buffered trace lines would be lost —
   Task 9 flushes the `TraceWriter` before exiting; (c) MCP child
   processes / sandbox containers are orphaned by the no-unwind exit —
   **accepted residual, recorded not silent** (the CLI process is
   terminating either way; a non-`--rm` container leak is possible).
4b. **Cancel-while-parked retains the park (baseline bug fix, owner
   escalation).** Both reviewers confirmed at live source that the cancel
   arm calls `clear_park()` unconditionally (~loop_.rs:1419-1421), so
   Ctrl-C/cancel while an Ask pends DELETES the park — directly
   contradicting spec §2.7's "Ctrl-C with an Ask pending = 'left parked'
   (the park already exists)". Task 9 fixes the site: `clear_park` fires
   only for real channel answers (approve / deny / E5 auto-deny), never
   for cancellation. **⚠ ESCALATED TO THE OWNER GATE:** the same code path
   serves the SERVER, so a desktop-side deliberate cancel while an ask
   pends now also leaves the run parked (it appears in the next attach's
   `parked_runs` list and can be resumed or left to rot). The alternative
   — clear-on-cancel — would break the spec's CLI Ctrl-C story; retention
   is spec-aligned but is a 4B-1 server behavior change the owner should
   see.
5. **Two 4B-1 merge-gate deferrals land here** (dispositions recorded in the
   4B-1 plan's review log; they extend spec §0's 4B-2 bullet list):
   (a) **resumed-run trace attribution** — `build_resume_loop` stops sharing
   the daemon's own trace handle and builds a per-descriptor `TraceWriter`
   for the resumed session id (`TraceWriter::create` opens append-mode, so
   reopening a prior session's JSONL appends correctly); (b) **in-life
   failed-resume retry** — the sticky `resuming` guard is removed at the
   spawned resume task's tail on failure (park retained), so the next
   attach re-prompts instead of demanding a daemon restart.
6. **CLI reopen drives the resumed run to completion, then exits.** No
   post-resume REPL continuation this slice (the restored context lives in
   the resumed loop; wiring it back into a fresh REPL is a named follow-on).
7. **`parked_runs` is a snapshot-on-attach** built from the same
   `scan_parked` call `spawn_parked_reemit` already makes (4B-1 header note
   8 anticipated exactly this); sessions currently in the `resuming` set are
   excluded (they are being resumed, not waiting).
8. **`resumed` fires at resume start** (immediately after the `resuming`
   guard commits and the active slot is claimed) — the moment the parked
   entry stops being answerable *for this attempt*, so the banner row
   drops. If the resume then FAILS, the park is retained and becomes
   answerable again (Task 5): within the still-attached session the user
   sees the error frame but no banner row until the next attach's
   `parked_runs` snapshot re-adds it — **accepted residual** (arch review
   finding 4), the error frame names the session.
9. **Web keeps a single `pendingApproval`.** Multiple re-emitted asks
   (several parked children) overwrite last-wins, exactly as in 4B-1;
   answering one triggers resume, and surviving asks re-ask live from the
   resumed tree. A prompt queue is out of scope (accepted residual).
10. **The CLI reopen driver accepts ~30 lines of duplication** with
    `session.rs start_resume`'s restore sequence rather than extracting a
    shared abstraction across the Session/CLI seam this slice (the 4B-0
    descriptor-block duplication note already tracks consolidation
    appetite; a shared helper would have to parameterize sink, error
    surface, and loop assembly — three axes for two callers).
11. **Cross-process resume exclusivity: an O_EXCL `resume.lock` claim on
    the checkpoint dir** (arch review BLOCKER, finding 3). The 4B-1
    `resuming` guard and `active` slot live in ONE daemon's heap; the CLI
    reopen is a separate OS process, so nothing at baseline prevents a
    desktop-daemon resume and a CLI-reopen resume from BOTH driving
    `resume_with_cancel` over the same checkpoint tree — `clear_park` is a
    remove, not a claim, and each driver holds its own in-memory
    `Checkpoint` + decision, so park deletion does not stop the loser:
    approved host-effecting tools would execute TWICE. Fix (Task 5
    primitive, both drivers honor it): `checkpoint::claim_resume(dir)`
    creates `<checkpoint>/resume.lock` with `create_new(true)` (O_EXCL) —
    success = you own the resume; `AlreadyExists` = refuse with "session
    <id> is being resumed elsewhere". Released explicitly on resume
    failure (parks retained ⇒ retry must be able to re-claim); removed
    implicitly by the success-path `remove_dir_all`. The daemon claims in
    `start_resume` (right after the `resuming` guard); the CLI claims
    BEFORE prompting the human (no wasted answer). **Stale-lock residual
    (recorded):** a SIGKILLed holder leaves the lock and the session
    refuses resume until it is removed by hand; fail-closed is correct for
    a human-gated rare path — an age-check or `--force` is a named
    follow-on. Side effect: this lock also bounds the Task 6 trace-file
    aliasing case (two writers on one JSONL requires two concurrent
    resumes, now impossible) and closes the arch review's findings 5/7 by
    construction. A late `commit_answer` by a non-claiming surface leaves
    a stale `answer.json` in a tree that either gets reaped on success or
    auto-consumed on the next attach — it was a genuine human answer;
    accepted.

## Global Constraints

- **Additive wire protocol** (spec §3.5): no frame removed or reshaped; new
  `ServerEvent` variants + optional fields only; wire `Decision` keeps
  byte-identical encoding when feedback is absent (refinement 1).
- **E1 / spec §3.1:** zero checkpoint I/O on non-Ask paths is untouched —
  this slice adds no checkpoint writes anywhere new (the CLI gains the same
  park-at-Ask-only behavior the server has).
- **E2:** no standing approvals; nothing in this slice persists an
  ApproveAlways.
- **Policy engine untouched** (spec §3.2): the *policy*
  `Decision::Deny(String)` reason type in agent-policy is pre-existing and
  NOT this feature — deny-feedback rides `ApprovalResponse`/wire `Decision`
  only (spec §2.7).
- **Trace JSONL contract unchanged** (spec §3.6): naming/shape/`0o600`
  identical; refinement 5a changes only WHICH session's file a resumed
  run's events land in.
- Checkpoint files stay `0o600`, dirs `0o700`, atomic temp+rename with
  FULL-filename temp names.
- Refuse-on-corrupt: MAC failure ⇒ refuse/re-ask, never guess (spec §4).
- Two Cargo workspaces: all `cargo` commands run in `agent/`. Web commands
  run in `web/`. `-p <crate>` must target the right workspace.
- agent-server tests that touch `$HOME`: use the real-HOME secret-touch
  pattern, NOT env mutation (4B-1 gotcha: `$HOME` env mutation races ~15
  parallel `metadata_root()` readers).
- Child checkpoint accessors (`load_child`/`load_child_answer`/
  `child_artifact_dump`) are called on the PARENT checkpointer (4B-1
  gotcha 1).
- Conventional commits `type(scope): summary`. Full `bash scripts/ci.sh`
  green before merge.

---

### Task 0: Branch

**Files:** none (git only)

- [ ] **Step 1: Create the feature branch off current main**

```bash
cd /home/kalen/rust-agent-runtime
git checkout main && git pull --ff-only 2>/dev/null; git checkout -b feature/durable-hitl-4b2
```

Expected: branch `feature/durable-hitl-4b2` at 577b40d (or later main).

---

### Task 1: `ApprovalResponse::Deny { feedback }` + shared `denial_content` renderer

**Files:**
- Modify: `agent/crates/agent-policy/src/engine.rs` (enum),
  `agent/crates/agent-core/src/loop_.rs` (deny arms + helper),
  `agent/crates/agent-server/src/approval.rs` (2 construction sites +
  waiter-adjacent matches),
  `agent/crates/agent-server/src/session.rs` (answer-waiter match →
  temporary bridge),
  `agent/crates/agent-cli/src/approval.rs` (stdin construction sites)
- Test: inline tests in `engine.rs`, `loop_.rs`, `approval.rs` (cli)

**Interfaces:**
- Produces (Tasks 2, 3, 7, 9, 10 rely on):

```rust
// agent-policy/src/engine.rs — Copy and Eq are DROPPED (String field):
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalResponse {
    Approve,
    ApproveAlways,
    Deny { feedback: Option<String> },
}

// agent-core/src/loop_.rs — the ONE renderer for human denials (live arm,
// resume splice, dispatch rebind all call it; Task 2 threads the stored
// feedback into it):
pub(crate) fn denial_content(feedback: Option<&str>) -> String {
    match feedback {
        Some(f) => format!("ERROR: {}", ToolError::Denied(format!("user declined — {f}"))),
        None => format!("ERROR: {}", ToolError::Denied("user declined".into())),
    }
}
```

- [ ] **Step 1: Write the failing tests**

`agent-core/src/loop_.rs` tests (mirror the existing park/approval test
rigs — they use `CuratedContext` + a scripted approval channel; locate the
4B-1 park tests by content, e.g. the test asserting a Deny produces a
`ToolStatus::Denied` result):

```rust
    #[test]
    fn denial_content_no_feedback_is_byte_identical_to_today() {
        // Pins the pre-4B-2 literal exactly (ToolError::Denied display is
        // "denied: {0}"):
        assert_eq!(denial_content(None), "ERROR: denied: user declined");
    }

    #[test]
    fn denial_content_with_feedback_appends_the_text() {
        assert_eq!(
            denial_content(Some("use the staging DB instead")),
            "ERROR: denied: user declined — use the staging DB instead"
        );
    }

    #[tokio::test]
    async fn live_deny_with_feedback_reaches_the_tool_result() {
        // Rig: ScriptedModel emits one tool call whose policy decision is
        // Ask (reuse the 4B-1 park-test rig's asking policy + CuratedContext);
        // approval channel = a scripted ApprovalChannel returning
        //   ApprovalResponse::Deny { feedback: Some("wrong host".into()) }.
        // Run one turn; assert the Role::Tool message content for that call
        // == denial_content(Some("wrong host")) and the emitted ToolResult
        // event carries ToolStatus::Denied.
    }

    #[tokio::test]
    async fn live_deny_without_feedback_matches_pre_4b2_content() {
        // Same rig, Deny { feedback: None } → content is EXACTLY
        // "ERROR: denied: user declined" (byte-parity pin for spec §3.1's
        // "only behavior deltas" list).
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core denial`
Expected: COMPILE ERROR (`denial_content` absent; `Deny` is a unit variant).

- [ ] **Step 3: Implement the enum change + renderer + full fan-out**

1. `agent-policy/src/engine.rs`: change the enum as in Interfaces. **Both
   `Copy` AND `Eq` are dropped** (the `String` field forbids `Copy`; `Eq`
   is verified-unused — nothing keys a map on the type or derives `Eq`
   around it; comparisons are `assert_eq!`/`matches!`, `PartialEq`
   suffices). Do not re-add `Eq`.
2. `agent-core/src/loop_.rs`: add `denial_content` (near `GateOutcome`,
   locate by content) and rewrite the human-denial sites. **The live
   NeedsApproval arm is NOT a match on the response** (both reviewers
   verified): it is a `tokio::select!` that collapses the response to a
   bool with `matches!` and then derives the reason string separately
   (~loop_.rs:1409-1435). Restructure it to CAPTURE the response and
   preserve the distinct `"run cancelled"` string:

```rust
        // was: let allowed = tokio::select! { … => matches!(resp, …) };
        let resp = tokio::select! {
            _ = cancel.cancelled() => ApprovalResponse::Deny { feedback: None },
            resp = self.approval.request(req) => resp,
        };
        let allowed = matches!(
            resp,
            ApprovalResponse::Approve | ApprovalResponse::ApproveAlways
        );
        // …existing plumbing; in the !allowed branch:
        let content = if cancel.is_cancelled() {
            // cancel-deny keeps its own message — denial_content is the
            // HUMAN-decline family only (reviewer B1: do not regress this).
            format!("ERROR: {}", ToolError::Denied("run cancelled".into()))
        } else {
            let feedback = match &resp {
                ApprovalResponse::Deny { feedback } => feedback.as_deref(),
                _ => None,
            };
            denial_content(feedback)
        };
```

   - the resume-splice denial (`parked_decision` false → today's hardcoded
     `ToolError::Denied("user declined".into())`, ~loop_.rs:1316-1323)
     calls `denial_content(None)` for now (Task 2 threads the stored
     feedback).
   - any other `"user declined"` literal:
     `grep -rn '"user declined"' agent/crates/agent-core/src` — every hit
     routes through the helper. The `"run cancelled"` literal stays.
3. Sweep every `ApprovalResponse` construction/match site to the new shape
   (`ApprovalResponse::Deny` → `ApprovalResponse::Deny { feedback: None }`
   at construction; `ApprovalResponse::Deny =>` → destructuring match):

```bash
grep -rn "ApprovalResponse" agent/crates --include=*.rs | grep -v target
```

   Known sites (verified at 577b40d; the grep is authoritative — note the
   fan-out spans SIX crates/dirs, not four):
   - `agent-server/src/approval.rs` timeout + no-subscriber arms (~:173,
     :175): `ApprovalResponse::Deny { feedback: None }` (an auto-deny has
     no human feedback).
   - `agent-server/src/session.rs` answer-waiter: the match that converts a
     response into `commit_answer(ask, approve, key)`'s bool — **temporary
     bridge**: `Deny { .. } => false`, discarding feedback (Task 2 threads
     it; keeps each task compiling, 4B-0 T4 precedent — leave a
     `// 4B-2 Task 2 threads feedback` comment).
   - `agent-server/src/wire.rs` `From<Decision>` impl: map
     `Decision::Deny => ApprovalResponse::Deny { feedback: None }`
     (Task 3 reshapes the wire side).
   - `agent-cli/src/approval.rs`: FIVE sites — `stdin_prompt` returns
     (~:27, :32), timeout/join arms (~:88, :91), and the
     `matches!(resp, Deny)` in the timeout test (~:134).
   - `agent-policy/src/engine.rs` test (~:374).
   - agent-core TEST channels: scripted approval channels in `loop_.rs`
     tests (~:4355, :4455, :8813, :8893), `dispatch.rs` (~:2762 — a
     `(Deny, false)` tuple), and `tests/dispatch_tool.rs` (~:549, :1740,
     :1798, :1861 — `reply:` fields).
   - **`agent-runtime-config` integration tests** (easy to miss — a
     different crate): `tests/e2e_robustness.rs` (~:191),
     `tests/memory_files_soak.rs` (~:58), `tests/eval_context.rs` (~:72),
     `tests/soak_live.rs` (~:72).

- [ ] **Step 4: Run the whole agent workspace to verify**

Run: `cd agent && cargo test`
Expected: PASS — the full-workspace run is deliberate (reviewer M3): the
`agent-runtime-config` integration tests construct `ApprovalResponse` and
would otherwise break silently until final CI. No existing assertion
weakened — auto-deny tests still see a Deny, now with `feedback: None`.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-policy agent/crates/agent-core agent/crates/agent-server agent/crates/agent-cli agent/crates/agent-runtime-config
git commit -m "feat(policy): ApprovalResponse::Deny carries optional feedback + shared denial renderer (4B-2)"
```

---

### Task 2: Durable answer carries feedback — `Answer` MAC extension + splice threading

**Files:**
- Modify: `agent/crates/agent-core/src/checkpoint.rs` (`Answer`,
  `write_answer`, `take_answer`, `load_child_answer`, `answer_mac`, new
  `ParkedAnswer`),
  `agent/crates/agent-core/src/loop_.rs` (`ResumeTurn.parked_decision`,
  `PreDecided`, splice arm),
  `agent/crates/agent-core/src/dispatch.rs` (rebind passes the answer
  through),
  `agent/crates/agent-server/src/resume.rs` (`commit_answer`),
  `agent/crates/agent-server/src/session.rs` (waiter threads feedback —
  removes the Task-1 bridge)
- Test: inline tests in `checkpoint.rs`, `loop_.rs`

**Interfaces:**
- Consumes: Task 1 (`ApprovalResponse::Deny { feedback }`,
  `denial_content`).
- Produces (Tasks 4, 10 rely on):

```rust
// agent-core/src/checkpoint.rs:
/// A durably committed approval decision (spec §2.3 answer commit point).
#[derive(Debug, Clone, PartialEq)]
pub struct ParkedAnswer {
    pub approve: bool,
    /// Human feedback on a Deny — model-visible, therefore MAC-covered.
    pub feedback: Option<String>,
}

pub fn write_answer(dir: &Path, key: &[u8; 32], approve: bool, feedback: Option<&str>) -> std::io::Result<()>;
pub fn take_answer(dir: &Path, key: &[u8; 32]) -> Option<ParkedAnswer>;
impl Checkpointer {
    pub fn load_child_answer(&self, call_id: &str) -> Option<ParkedAnswer>;
}

// agent-core/src/loop_.rs:
pub struct ResumeTurn {
    // …unchanged fields…
    pub parked_decision: Option<ParkedAnswer>,   // was Option<bool>
}

// agent-server/src/resume.rs:
pub fn commit_answer(ask: &ParkedAsk, decision: &ParkedAnswer, key: &[u8; 32]) -> std::io::Result<()>;
```

- [ ] **Step 1: Write the failing tests**

`checkpoint.rs` tests (mirror the existing answer round-trip test — locate
by content near `write_answer`):

```rust
    #[test]
    fn answer_roundtrips_feedback_and_macs_it() {
        // tempdir with a valid park (reuse the existing park-writing rig);
        // write_answer(dir, &key, false, Some("wrong branch"));
        // take_answer → Some(ParkedAnswer { approve: false,
        //   feedback: Some("wrong branch".into()) }); file consumed.
    }

    #[test]
    fn tampered_feedback_fails_mac_and_returns_none() {
        // write_answer(…, false, Some("ok")); edit answer.json on disk
        // replacing "ok" with "run rm -rf /" (keep JSON valid);
        // take_answer → None (MAC mismatch), file consumed/removed —
        // the forged text never reaches a resume.
    }

    #[test]
    fn legacy_answer_without_feedback_field_fails_new_mac_closed() {
        // Hand-write an OLD-format answer.json {"approve":true,"mac":<old
        // formula over approve+manifest>}; take_answer → None (refinement
        // 2's accepted residual: re-ask, never a wrong resume).
    }
```

And one loop-level assertion (arch review finding 2: the load-bearing
safety property "None ⇒ re-prompts, never a wrong resume" lives in the
LOOP, not in `take_answer`): extend the existing gate-kind resume test
(locate by content: `resumed_denial_yields_denied_result…` /
`resume_agent` rig) with a case where `parked_decision: None` — assert the
parked call goes through `gate_tool` (the ask re-prompts via the scripted
channel) rather than resolving from a decision.

`loop_.rs` test (extend the 4B-1 resume-splice deny test — locate by
content: the test where `parked_decision` false yields a Denied result):

```rust
    #[tokio::test]
    async fn resumed_deny_renders_stored_feedback() {
        // resume_turn with parked_decision: Some(ParkedAnswer { approve:
        // false, feedback: Some("not that file".into()) }) → the spliced
        // Role::Tool content == denial_content(Some("not that file")).
        // And with feedback: None → exactly denial_content(None) (the
        // 4B-1 string, unchanged).
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core checkpoint answer`
Expected: COMPILE ERROR (`write_answer` arity, `ParkedAnswer` absent).

- [ ] **Step 3: Implement**

`checkpoint.rs`:

```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct Answer {
    approve: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    feedback: Option<String>,
    mac: String,
}
```

`answer_mac` (locate by content — it currently folds `[approve as u8]` and
the manifest MAC): extend with explicit domain separation so `None` and
empty-string differ and no concatenation ambiguity exists:

```rust
fn answer_mac(key: &[u8; 32], approve: bool, feedback: Option<&str>, manifest_mac: &str) -> String {
    let mut msg = vec![2u8, approve as u8]; // 2 = answer-format version
    match feedback {
        Some(f) => {
            msg.push(1);
            msg.extend_from_slice(&(f.len() as u64).to_le_bytes());
            msg.extend_from_slice(f.as_bytes());
        }
        None => msg.push(0),
    }
    msg.extend_from_slice(manifest_mac.as_bytes());
    hex(&hmac_sha256(key, &msg)) // the real helpers (checkpoint.rs ~:77/:101);
                                 // there is no hmac_sha256_hex
}
```

(Reviewer-verified domain separation: the old message is
`[approve] ‖ manifest_hex` — the new no-feedback message
`[2, approve, 0] ‖ manifest_hex` differs in length AND first byte, so no
cross-formula reinterpretation exists in either direction.)

`write_answer`/`take_answer`: thread `feedback` through; `take_answer`
returns `ParkedAnswer` on MAC success. `Checkpointer::load_child_answer`
returns `Option<ParkedAnswer>` (it already appends
`children/<sanitized id>` — do not double-nest, 4B-1 gotcha 1).

`loop_.rs`: `ResumeTurn.parked_decision`/`PreDecided.parked_decision`
become `Option<ParkedAnswer>`; `Checkpoint::resume_turn(decision)`
signature follows; the consume arm:

```rust
        let ans = p.parked_decision.clone().unwrap();
        if ans.approve {
            self.gate_preapproved(call, cancel)
        } else {
            // …existing emit…
            GateOutcome::Rejected {
                id: call.id,
                name: call.name,
                content: denial_content(ans.feedback.as_deref()),
            }
        }
```

`dispatch.rs`: the rebind site (`ck.load_child_answer(&ctx.call_id)` →
`chk.resume_turn(decision)`, ~dispatch.rs:995-999) compiles through
unchanged semantics — the `Option<ParkedAnswer>` flows where
`Option<bool>` did.

Known `parked_decision` fan-out beyond the defs (compile-driven, reviewer
m7): consumers at ~loop_.rs:1141 and :1294-1305, plus FIVE test sites
(~loop_.rs:9317, :9374, :9437, :9493, :9622) — a bool literal becomes
`Some(ParkedAnswer { approve: …, feedback: None })` and a bool assertion
becomes `.approve`.

`resume.rs`: `commit_answer(ask, decision: &ParkedAnswer, key)` calls
`checkpoint::write_answer(&ask.dir, key, decision.approve,
decision.feedback.as_deref())`.

`session.rs` answer-waiter: replace the Task-1 bridge — map the channel's
response to a `ParkedAnswer`:

```rust
        let decision = match resp {
            ApprovalResponse::Approve | ApprovalResponse::ApproveAlways => {
                ParkedAnswer { approve: true, feedback: None } // E2: plain approve
            }
            ApprovalResponse::Deny { feedback } => ParkedAnswer { approve: false, feedback },
        };
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core && cargo test -p agent-server`
Expected: PASS, including every pre-existing answer/splice/rebind test
updated to the new types with assertions preserved (a bool assertion
becomes `.approve`).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core agent/crates/agent-server
git commit -m "feat(checkpoint): durable answers carry MAC-bound deny feedback (4B-2)"
```

---

### Task 3: Wire `Decision` — feedback payload, `Copy` dropped, byte-compatible serde

**Files:**
- Modify: `agent/crates/agent-server/src/wire.rs` (`Decision` + `From`
  impl), `web/src/wire.ts` (`Decision` type), `web/src/socket.ts` (no code
  change expected — verify passthrough), `src-tauri` (verify-only: the
  `approve` command deserializes the new shape via serde)
- Test: inline tests in `wire.rs`; `web/test/socket.test.ts`

**Interfaces:**
- Consumes: Task 1 (`ApprovalResponse::Deny { feedback }`).
- Produces (Task 7 relies on):

```rust
// agent-server/src/wire.rs — Copy is DROPPED; serde is hand-written
// (refinement 1: bare "deny" stays valid on the wire both directions):
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Approve,
    ApproveAlways,
    Deny { feedback: Option<String> },
}
```

```typescript
// web/src/wire.ts:
export type Decision =
  | "approve"
  | "approve_always"
  | "deny"
  | { deny: { feedback?: string } };
```

- [ ] **Step 1: Write the failing tests**

`wire.rs` tests (next to the existing `From<Decision>` test, ~:873):

```rust
    #[test]
    fn decision_serde_accepts_legacy_bare_deny_and_new_object_form() {
        let d: Decision = serde_json::from_str("\"deny\"").unwrap();
        assert_eq!(d, Decision::Deny { feedback: None });
        let d: Decision = serde_json::from_str(r#"{"deny":{"feedback":"use staging"}}"#).unwrap();
        assert_eq!(d, Decision::Deny { feedback: Some("use staging".into()) });
        let d: Decision = serde_json::from_str(r#"{"deny":{}}"#).unwrap();
        assert_eq!(d, Decision::Deny { feedback: None });
        let d: Decision = serde_json::from_str("\"approve_always\"").unwrap();
        assert_eq!(d, Decision::ApproveAlways);
    }

    #[test]
    fn decision_serialize_is_byte_identical_when_no_feedback() {
        assert_eq!(serde_json::to_string(&Decision::Approve).unwrap(), "\"approve\"");
        assert_eq!(
            serde_json::to_string(&Decision::Deny { feedback: None }).unwrap(),
            "\"deny\""
        );
        assert_eq!(
            serde_json::to_string(&Decision::Deny { feedback: Some("x".into()) }).unwrap(),
            r#"{"deny":{"feedback":"x"}}"#
        );
    }

    #[test]
    fn deny_feedback_crosses_into_approval_response() {
        let r: ApprovalResponse = Decision::Deny { feedback: Some("nope".into()) }.into();
        assert_eq!(r, ApprovalResponse::Deny { feedback: Some("nope".into()) });
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-server wire`
Expected: FAIL/compile error (derived serde rejects the object form; `Copy`
conflicts with the `String` field).

- [ ] **Step 3: Implement**

Replace the derives with manual serde on `Decision`:

```rust
impl serde::Serialize for Decision {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Decision::Approve => s.serialize_str("approve"),
            Decision::ApproveAlways => s.serialize_str("approve_always"),
            Decision::Deny { feedback: None } => s.serialize_str("deny"),
            Decision::Deny { feedback: Some(f) } => {
                use serde::ser::SerializeMap;
                #[derive(serde::Serialize)]
                struct Fb<'a> { feedback: &'a str }
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("deny", &Fb { feedback: f })?;
                m.end()
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for Decision {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Tag(String),
            Obj { deny: DenyBody },
        }
        #[derive(serde::Deserialize)]
        struct DenyBody {
            #[serde(default)]
            feedback: Option<String>,
        }
        match Wire::deserialize(d)? {
            Wire::Tag(t) => match t.as_str() {
                "approve" => Ok(Decision::Approve),
                "approve_always" => Ok(Decision::ApproveAlways),
                "deny" => Ok(Decision::Deny { feedback: None }),
                other => Err(serde::de::Error::unknown_variant(
                    other,
                    &["approve", "approve_always", "deny"],
                )),
            },
            Wire::Obj { deny } => Ok(Decision::Deny { feedback: deny.feedback }),
        }
    }
}
```

`From<Decision> for ApprovalResponse`: `Decision::Deny { feedback } =>
ApprovalResponse::Deny { feedback }`. Then sweep for `Copy` reliance:
`grep -rn "Decision" agent/crates/agent-server/src src-tauri/src --include=*.rs`
— the src-tauri `approve` command (`src-tauri/src/lib.rs` ~:59) moves the
value once into `session.approve(&id, decision)`; no clone needed. Web
`wire.ts` gains the union member above (socket.ts already forwards
`o.decision` verbatim into `invoke("approve", …)` — no change).

`web/test/socket.test.ts`: add one case — sending
`{ kind: "approval_response", id: "c0", decision: { deny: { feedback: "why" } } }`
invokes `approve` with that exact decision object (mirrors the existing
"routes approval_response to the approve command" test).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-server && cd ../web && npx vitest run test/socket.test.ts`
Expected: PASS. Also `cd src-tauri && cargo check` if GTK deps are present
(ci.sh's conditional leg covers it otherwise).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server/src/wire.rs web/src/wire.ts web/test/socket.test.ts
git commit -m "feat(wire): Decision::Deny carries feedback with legacy-compatible serde (4B-2)"
```

---

### Task 4: New frames — `parked_runs`, `approval_resolved`, `resumed`

**Files:**
- Modify: `agent/crates/agent-server/src/wire.rs` (3 variants + DTO),
  `agent/crates/agent-server/src/approval.rs` (`resolve` +
  `retract_external_for` emit `approval_resolved`),
  `agent/crates/agent-server/src/session.rs` (`spawn_parked_reemit` emits
  `parked_runs`; `start_resume` emits `resumed`)
- Test: inline tests in `approval.rs`, `session.rs`

**Interfaces:**
- Consumes: `scan_parked`, the `resuming` set, `EventSlot`.
- Produces (Task 7 relies on — wire shapes, snake_case tagged):

```rust
// wire.rs — additive ServerEvent variants:
    /// Attach-time snapshot of PRIOR sessions parked on approvals (4B-2).
    ParkedRuns { runs: Vec<ParkedRunDto> },
    /// An approval was answered/retracted somewhere else — drop the prompt.
    ApprovalResolved { id: String },
    /// A parked session's resume has started; its banner row should drop.
    Resumed { session_id: String },

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParkedRunDto {
    pub session_id: String,
    pub workspace: String,
    pub created_ms: u64,
    /// Unanswered asks awaiting a human (0 with error set ⇒ unresumable).
    pub asks: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
```

- [ ] **Step 1: Write the failing tests**

`approval.rs` tests (the `Captured` EventOut rig exists in this file since
4B-1 — reuse it):

```rust
    #[tokio::test]
    async fn resolve_emits_approval_resolved_to_the_attached_surface() {
        // subscriber attached; spawn request() → capture the frame id;
        // resolve(id, Approve) → Captured's LAST frame is
        // ServerEvent::ApprovalResolved { id } (emitted after the maps
        // clear, so a reemit_pending after it sends nothing).
    }

    #[tokio::test]
    async fn retract_external_sweeps_and_emits_resolved_per_id() {
        // register_external twice under group "200-bbbbbbbb";
        // retract_external_for("200-bbbbbbbb") → two ApprovalResolved
        // frames, pending/pending_frames/external_groups all empty.
    }

    #[tokio::test]
    async fn cross_surface_first_answer_wins_and_second_attach_sees_retraction() {
        // Spec §6 row, scoped per refinement 3: attach A; request();
        // resolve via A. Attach B (new Captured) → reemit_pending sends
        // NOTHING to B, and B's history contains no live prompt; the
        // resolve-time ApprovalResolved went to the then-attached surface.
    }
```

`session.rs` test (extend the 4B-1 `attach_reemits_parked_ask…` rig by
content):

```rust
    #[tokio::test]
    async fn attach_sends_parked_runs_snapshot() {
        // Same planted PRIOR session as the 4B-1 attach test (descriptor
        // "100-aaaaaaaa" + gate park). set_event_out(Captured) → among the
        // captured frames is ParkedRuns whose single ParkedRunDto has
        // session_id "100-aaaaaaaa", asks == 1, error == None, workspace ==
        // the planted descriptor's path. A session present in `resuming`
        // must NOT appear (insert it into the set first in a second case).
    }
```

For `resumed`: extend the existing start_resume-path test (locate by
content — the 4B-1 test that drives an answer through commit and asserts
the resumed run) to assert a `ServerEvent::Resumed { session_id }` frame is
captured before the resumed run's first event.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-server`
Expected: COMPILE ERROR (variants absent).

- [ ] **Step 3: Implement**

`wire.rs`: add the three variants + DTO exactly as in Interfaces (additive;
existing decoders ignore unknown frames).

`approval.rs`:
- `resolve`: after removing from the three maps and before sending on the
  oneshot, emit to the slot if attached:

```rust
        if let Some(out) = self.slot.lock().unwrap().clone() {
            out.send(ServerEvent::ApprovalResolved { id: id.to_string() });
        }
```

- `retract_external_for`: after the sweep loop, emit one
  `ApprovalResolved { id }` per retracted id (collect ids first — the locks
  are already scoped that way).

`session.rs`:
- Add the generic slot-send helper first (reviewer m2 — only the private,
  Error-hardcoding `emit_error` exists today; mirror its body):

```rust
    fn send_event(&self, ev: ServerEvent) {
        if let Some(out) = self.slot.lock().unwrap().clone() {
            out.send(ev);
        }
    }
```

  (Optionally re-express `emit_error` through it — same lock pattern.)
- `spawn_parked_reemit`: the scan is currently consumed inline
  (`for parked in scan_parked(…)`) — hoist it to a binding so the snapshot
  and the re-emit loop share ONE scan (reviewer m3):
  `let parked_list = crate::resume::scan_parked(&root, &key, sess.runtime.session_id());`
  then build and send the snapshot BEFORE the per-session re-emit loop:

```rust
            let resuming = sess.resuming.lock().unwrap().clone();
            let runs: Vec<ParkedRunDto> = parked_list.iter()
                .filter(|p| !resuming.contains(&p.descriptor.session_id))
                .map(|p| ParkedRunDto {
                    session_id: p.descriptor.session_id.clone(),
                    workspace: p.descriptor.workspace.display().to_string(),
                    created_ms: p.descriptor.created_ms,
                    asks: p.asks.iter().filter(|a| !a.answered).count() as u32,
                    error: p.errors.first().cloned(),
                })
                .collect();
            if !runs.is_empty() {
                sess.send_event(ServerEvent::ParkedRuns { runs });
            }
```

  (One scan feeds both the snapshot and the re-emit loop — never scan
  twice.)
- `start_resume`: immediately after the `resuming` guard insert succeeds
  and the `active` slot is claimed, emit
  `ServerEvent::Resumed { session_id: sid.to_string() }` (refinement 8).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server
git commit -m "feat(server): parked_runs snapshot + approval_resolved retraction + resumed notice frames (4B-2)"
```

---

### Task 5: Resume exclusivity lock (cross-process) + in-life failed-resume retry

**Files:**
- Modify: `agent/crates/agent-core/src/checkpoint.rs`
  (`claim_resume`/`release_resume`),
  `agent/crates/agent-server/src/session.rs` (`start_resume`: claim wiring
  + guard-clear-on-failure)
- Test: inline tests in `checkpoint.rs`, `session.rs`

**Interfaces:**
- Consumes: the `resuming: Arc<Mutex<HashSet<String>>>` guard (4B-1).
- Produces (Task 10's CLI driver relies on — refinement 11, the review
  BLOCKER fix):

```rust
// agent-core/src/checkpoint.rs:
/// Exclusive cross-process claim on resuming this checkpoint tree
/// (refinement 11). O_EXCL create of <dir>/resume.lock: Ok(true) = claimed,
/// Ok(false) = another process holds it, Err = I/O trouble (treat as not
/// claimed). The success path's remove_dir_all reaps it; a FAILED resume
/// must release_resume so a retry can claim.
pub fn claim_resume(dir: &Path) -> std::io::Result<bool>;
pub fn release_resume(dir: &Path);
```

- Also produces: a failed resume no longer requires a daemon restart to
  retry — the park is retained (4B-1 behavior) AND the next attach
  re-prompts.

- [ ] **Step 1: Write the failing tests**

`checkpoint.rs`:

```rust
    #[test]
    fn claim_resume_is_exclusive_and_releasable() {
        // tempdir: claim_resume → Ok(true); second claim_resume →
        // Ok(false); release_resume; claim_resume → Ok(true) again.
        // File mode 0o600 (match the tree's discipline).
    }
```

`session.rs`:

```rust
    #[tokio::test]
    async fn failed_resume_clears_the_guard_so_the_next_attach_reprompts() {
        // NET-NEW test — no failed-resume test exists at baseline
        // (reviewer M2: the bounce literal at session.rs ~:304 is NOT
        // test-pinned; do not hunt for one). Plant a PRIOR parked session
        // (the attach_reemits… rig), then sabotage the resume attempt
        // DETERMINISTICALLY so resume_with_cancel/assembly errors —
        // candidate levers, implementer picks the one that fails reliably:
        //   (a) delete the descriptor's workspace dir AFTER the ask was
        //       re-emitted (spawn_parked_reemit's is_dir refusal already
        //       ran) but BEFORE answering;
        //   (b) corrupt the artifacts dump so restore errors.
        // Drive: attach → answer → resume fails (error frame captured).
        // THEN assert: sess.resuming does NOT contain the session id;
        // checkpoint::claim_resume(&dir) succeeds (the failed attempt
        // RELEASED the lock); and a SECOND set_event_out(Captured2)
        // re-emits the parked ask instead of bouncing.
    }

    #[tokio::test]
    async fn resume_refuses_when_another_process_holds_the_lock() {
        // Plant the parked session; pre-create resume.lock via
        // checkpoint::claim_resume (simulating a concurrent CLI reopen).
        // attach → answer → an Error frame naming "being resumed
        // elsewhere" is emitted, the resumed run never starts, the park
        // and lock are untouched.
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core claim_resume && cargo test -p agent-server failed_resume`
Expected: COMPILE ERROR (`claim_resume` absent).

- [ ] **Step 3: Implement**

`checkpoint.rs`:

```rust
pub fn claim_resume(dir: &Path) -> std::io::Result<bool> {
    std::fs::create_dir_all(dir)?; // dir exists whenever a park exists; cheap guard
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    match opts.open(dir.join("resume.lock")) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn release_resume(dir: &Path) {
    let _ = std::fs::remove_file(dir.join("resume.lock"));
}
```

`session.rs start_resume`: right after the `resuming` guard insert (and its
existing active-slot check), claim the lock — refuse without starting when
another process holds it:

```rust
        match agent_core::checkpoint::claim_resume(&root_dir) {
            Ok(true) => {}
            _ => {
                self.emit_error(format!(
                    "session {sid}: is being resumed elsewhere (another daemon or a CLI reopen); \
                     if that process crashed, remove {}/resume.lock",
                    root_dir.display()
                ));
                self.resuming.lock().unwrap().remove(sid);
                return;
            }
        }
```

In the spawned task, the `Err` arm currently emits the error and leaves the
guard set. At the task tail, AFTER `*sess.active.lock().unwrap() = None;`
(ordering: the active slot must be free before a retry can claim it):

```rust
            Err(e) => {
                sess.emit_error(format!("session {sid}: resumed run failed: {e}"));
                failed = true;
            }
        }
        *sess.active.lock().unwrap() = None;
        if failed {
            // In-life retry (4B-1 merge-gate deferral): the park was
            // retained; releasing lock + guard lets the next attach
            // re-prompt (refinement 11 makes the retry re-claim).
            agent_core::checkpoint::release_resume(&root_dir);
            sess.resuming.lock().unwrap().remove(&sid);
        }
```

(On SUCCESS neither fires: `remove_dir_all` reaps the lock with the tree,
and the id staying in `resuming` is harmless — nothing is left to
re-prompt. The existing active-conflict early-return at ~session.rs:310
already removes the guard on its own path; the two removals are on
disjoint exits — the new test's interleaving covers it.)

Update the bounce error text at ~session.rs:304 to
`"resume already in progress; answer again after it finishes"` — the
"restart the daemon to retry" wording becomes false with this task. The
literal is NOT test-pinned (reviewer M2); update the source only. The
guard still holds for the WHOLE attempt, so same-process double-resume
stays impossible; the lock extends that exclusivity across processes.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core && cargo test -p agent-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/checkpoint.rs agent/crates/agent-server/src/session.rs
git commit -m "feat(checkpoint): cross-process resume.lock + in-life failed-resume retry (4B-2)"
```

---

### Task 6: Resumed-run trace attribution — per-descriptor TraceWriter

**Files:**
- Modify: `agent/crates/agent-server/src/runtime.rs` (`build_resume_loop`
  builds the resumed session's trace), `agent/crates/agent-server/src/session.rs`
  (caller passes the resumed session id — it already has `sid`)
- Test: inline test in `runtime.rs` or `session.rs`

**Interfaces:**
- Consumes: `agent_runtime_config::build_trace(cfg, session_id)` (appends:
  `TraceWriter::create` opens `append(true)`).
- Produces: resumed-run events land in the RESUMED session's
  `<sessions_root>/<id>.jsonl`, not the daemon's own.

- [ ] **Step 1: Write the failing test**

```rust
    #[tokio::test]
    async fn resumed_run_traces_into_its_own_session_file() {
        // params with trace_dir tempdir + trace: true. Plant PRIOR parked
        // session "100-aaaaaaaa" (4B-1 attach-test rig). Drive attach →
        // answer → resume to completion. Assert:
        //   <trace_dir>/100-aaaaaaaa.jsonl EXISTS and gained records
        //     (line count grew past whatever the plant wrote — the resumed
        //     run's events);
        //   the daemon's OWN <trace_dir>/<own_id>.jsonl did NOT gain the
        //     resumed run's tool events (snapshot its length before the
        //     answer, compare after).
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-server trace`
Expected: FAIL (resumed events land in the daemon's own file).

- [ ] **Step 3: Implement**

`build_resume_loop` gains the resumed id and swaps the trace handle:

```rust
    pub fn build_resume_loop(
        &self,
        resumed_session_id: &str,
        workspace: &Path,
        checkpoint: Arc<agent_core::Checkpointer>,
        artifacts: &Arc<SessionArtifacts>,
        todos: &agent_core::TodoHandle,
        compact_flag: &Arc<AtomicBool>,
    ) -> BuiltLoop {
        let cfg = self.config.lock().unwrap().clone();
        // Trace attribution (4B-1 merge-gate deferral): a resumed session's
        // events append to ITS OWN jsonl, not the resuming daemon's.
        let trace = agent_runtime_config::build_trace(&cfg, resumed_session_id);
        build_loop(
            &cfg,
            // …unchanged args…
            &trace,               // was &self.trace
            &Some(checkpoint),
        )
    }
```

TWO callers update (reviewer m8): the production site in `session.rs`
(~:190) — note `sid` is bound AFTER the call there, so pass
`&parked.descriptor.session_id` (or hoist the `sid` binding above the
call) — and the `build_resume_loop` test in `runtime.rs` (~:1008). Trace
contract note: naming/shape/`0o600` unchanged (spec §3.6); the appended
records' `seq` counter restarts from a fresh `TraceWriter` — recorded as
an accepted residual (audit readability only; the JSONL is append-ordered
regardless). File-aliasing by two concurrent writers is bounded by the
Task 5 resume lock (two concurrent resumes of one session are now
impossible — arch review finding 7).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-server`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server
git commit -m "fix(server): resumed runs trace into their own session file (4B-2)"
```

---

### Task 7: Web — Decision feedback field, parked banner, retraction + resumed handling

**Files:**
- Modify: `web/src/wire.ts` (3 inbound frame kinds + `ParkedRun` type —
  `Decision` landed in Task 3), `web/src/socket.ts` (`toInbound` routes the
  3 new server events), `web/src/state.ts` (state + reducer),
  `web/src/components/ApprovalPrompt.tsx` (feedback field),
  `web/src/App.tsx` (banner mount), 
- Create: `web/src/components/ParkedBanner.tsx`
- Test: `web/src/state.parked.test.ts`,
  `web/src/components/ApprovalPrompt.test.tsx` (extend),
  `web/src/components/ParkedBanner.test.tsx`

**Interfaces:**
- Consumes: Task 3 (`Decision` union), Task 4 (frame shapes).
- Produces:

```typescript
// wire.ts:
export interface ParkedRun {
  session_id: string;
  workspace: string;
  created_ms: number;
  asks: number;
  error?: string;
}
// Inbound union gains:
//  | { v; session_id; kind: "parked_runs"; runs: ParkedRun[] }
//  | { v; session_id; kind: "approval_resolved"; id: string }
//  | { v; session_id; kind: "resumed"; resumed_session_id: string }

// state.ts — ConversationState gains:
//  parkedRuns: ParkedRun[];
```

- [ ] **Step 1: Write the failing tests**

`web/src/state.parked.test.ts` (mirror `state.approval.test.ts`'s helpers):

```typescript
import { describe, expect, it } from "vitest";
import { initialState, reduce } from "./state";
import type { Inbound } from "./wire";

const runs = [{ session_id: "100-aaaaaaaa", workspace: "/w", created_ms: 5, asks: 1 }];
const parkedFrame = { v: 1, session_id: "s", kind: "parked_runs", runs } as Inbound;

describe("parked runs + retraction + resumed", () => {
  it("stores the parked_runs snapshot", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: parkedFrame });
    expect(s.parkedRuns).toEqual(runs);
  });

  it("approval_resolved clears a matching pending approval and card flags", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", id: "c9", kind: "approval_request", summary: "run x",
    } as Inbound });
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", kind: "approval_resolved", id: "c9",
    } as Inbound });
    expect(s.pendingApproval).toBeNull();
  });

  it("approval_resolved with a different id leaves the prompt alone", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", id: "c9", kind: "approval_request", summary: "run x",
    } as Inbound });
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", kind: "approval_resolved", id: "c8",
    } as Inbound });
    expect(s.pendingApproval?.id).toBe("c9");
  });

  it("resumed drops the banner row", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: parkedFrame });
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", kind: "resumed", resumed_session_id: "100-aaaaaaaa",
    } as Inbound });
    expect(s.parkedRuns).toEqual([]);
  });
});
```

`ApprovalPrompt.test.tsx` additions:

```typescript
  it("deny with feedback sends the object decision", () => {
    const onDecide = vi.fn();
    render(<ApprovalPrompt approval={approval} onDecide={onDecide} />);
    fireEvent.change(screen.getByPlaceholderText(/optional feedback/i), {
      target: { value: "use staging" },
    });
    fireEvent.click(screen.getByText(/^3\./).closest("button") ?? screen.getByText("No"));
    expect(onDecide).toHaveBeenCalledWith({ deny: { feedback: "use staging" } });
  });

  it("deny with empty feedback sends the legacy string", () => {
    const onDecide = vi.fn();
    render(<ApprovalPrompt approval={approval} onDecide={onDecide} />);
    fireEvent.keyDown(window, { key: "3" });
    expect(onDecide).toHaveBeenCalledWith("deny");
  });
```

`ParkedBanner.test.tsx`:

```typescript
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ParkedBanner } from "./ParkedBanner";

describe("ParkedBanner", () => {
  it("lists parked runs with ask counts and marks unresumable ones", () => {
    render(<ParkedBanner
      runs={[
        { session_id: "100-aaaaaaaa", workspace: "/w", created_ms: 5, asks: 2 },
        { session_id: "200-bbbbbbbb", workspace: "/x", created_ms: 6, asks: 0, error: "checkpoint unreadable" },
      ]}
      onDismiss={() => {}}
    />);
    expect(screen.getByText(/100-aaaaaaaa/)).toBeTruthy();
    expect(screen.getByText(/2 approvals? waiting/i)).toBeTruthy();
    expect(screen.getByText(/checkpoint unreadable/)).toBeTruthy();
  });

  it("dismisses", () => {
    const onDismiss = vi.fn();
    render(<ParkedBanner runs={[{ session_id: "1", workspace: "/w", created_ms: 5, asks: 1 }]} onDismiss={onDismiss} />);
    fireEvent.click(screen.getByLabelText("Dismiss"));
    expect(onDismiss).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd web && npx vitest run src/state.parked.test.ts src/components/ParkedBanner.test.tsx src/components/ApprovalPrompt.test.tsx`
Expected: FAIL (types/component/reducer arms absent).

- [ ] **Step 3: Implement**

`wire.ts`: types as in Interfaces. `socket.ts` `toInbound`: route the three
server-event types to the three new frame kinds (mirror how
`approval_request` is lifted out of the generic `event` arm — locate by
content in `toInbound`; the `resumed` server event's field is `session_id`
inside the payload, mapped to `resumed_session_id` on the frame to avoid
colliding with the envelope's `session_id`).

`state.ts`:
- `ConversationState` gains `parkedRuns: ParkedRun[]` (`initialState`: `[]`).
- `reduceFrame` new arms — NOTE the top-level frame kinds are an
  **`if (frame.kind === …)` chain, not a `switch`** (reviewer m6; only the
  nested `event` payload switches). Add guards in the same style:

```typescript
  if (frame.kind === "parked_runs") {
    return { ...state, parkedRuns: frame.runs };
  }
  if (frame.kind === "approval_resolved") {
    // id-guarded: a retraction for an overwritten OLD id must not clear a
    // newer prompt (arch review finding 8 verified this guard sufficient).
    if (state.pendingApproval?.id !== frame.id) return state;
    return {
      ...state,
      pendingApproval: null,
      items: state.items.map((it) =>
        isToolItem(it) && it.subagent?.waitingApproval
          ? { ...it, subagent: { ...it.subagent, waitingApproval: false } }
          : it,
      ),
    };
  }
  if (frame.kind === "resumed") {
    return {
      ...state,
      parkedRuns: state.parkedRuns.filter((r) => r.session_id !== frame.resumed_session_id),
    };
  }
```

- add a `dismiss_parked_banner` action (`parkedRuns: []`), mirroring
  `dismiss_sandbox_banner`.

`ApprovalPrompt.tsx`: add local `feedback` state + input rendered between
the command display and the buttons:

```tsx
  const [feedback, setFeedback] = useState("");
  const decide = (d: Decision) =>
    onDecide(d === "deny" && feedback.trim() ? { deny: { feedback: feedback.trim() } } : d);
  // …
  <input
    value={feedback}
    onChange={(e) => setFeedback(e.target.value)}
    placeholder="optional feedback if denying"
    className="mb-1 w-full rounded px-2 py-1 text-sm"
    style={{ background: "var(--surface-raised)", border: "1px solid var(--border)", color: "var(--cli-text)" }}
  />
```

All three OPTION buttons and the 1/2/3 key handler route through `decide`
(only deny picks up feedback). Guard the key handler: keys typed while the
feedback input is focused must NOT trigger decisions (check
`document.activeElement`).

`ParkedBanner.tsx` (mirror `SandboxBanner`'s structure/`role="alert"`/
dismiss; color: `--accent` + label per the no-`--cli-warn` convention):
one row per run — `Parked run {session_id} · {workspace} — {asks}
approval(s) waiting` or the error text; `data-testid="parked-banner"`.

`App.tsx`: mount after `SandboxBanner`:

```tsx
      {state.parkedRuns.length > 0 && (
        <ParkedBanner runs={state.parkedRuns} onDismiss={() => dispatch({ type: "dismiss_parked_banner" })} />
      )}
```

- [ ] **Step 4: Run tests + typecheck**

Run: `cd web && npx tsc --noEmit && npx vitest run`
Expected: PASS (full web suite — the `Decision` union change may surface
`onDecide` typing in existing tests; fix types, not assertions).

- [ ] **Step 5: Commit**

```bash
git add web/src
git commit -m "feat(web): deny feedback field + parked-runs banner + approval_resolved/resumed handling (4B-2)"
```

---

### Task 8: CLI `sessions` subcommand + `sessions list`

**Files:**
- Modify: `agent/crates/agent-cli/src/main.rs` (clap subcommand + dispatch),
  `agent/crates/agent-cli/Cargo.toml` (add `agent-server` path dep — needed
  by Tasks 8/10 for `resume::scan_parked`),
  `AGENTS.md` (one surface line, see step 3)
- Test: inline tests in `main.rs`

**Interfaces:**
- Consumes: `scan_descriptors`, `session_dir`, `sessions_root`,
  `checkpoint::has_park`.
- Produces (Task 10 relies on): the `Command::Sessions` clap scaffold.

```rust
#[derive(clap::Subcommand)]
enum Command {
    /// Inspect and reopen past sessions.
    Sessions {
        #[command(subcommand)]
        cmd: SessionsCmd,
    },
}

#[derive(clap::Subcommand)]
enum SessionsCmd {
    /// List sessions, newest first; parked runs are marked.
    List,
    /// Reopen a parked session: re-prompt its pending approval and resume.
    Reopen { session_id: String },
}
```

- [ ] **Step 1: Write the failing tests**

`main.rs` tests (mirror the existing clap parse tests, e.g.
`sandbox_mode_flag_parsed`):

```rust
    #[test]
    fn sessions_list_parses() {
        let cli = Cli::parse_from(["agent", "sessions", "list"]);
        assert!(matches!(
            cli.command,
            Some(Command::Sessions { cmd: SessionsCmd::List })
        ));
    }

    #[test]
    fn sessions_reopen_parses_with_id() {
        let cli = Cli::parse_from(["agent", "sessions", "reopen", "100-aaaaaaaa"]);
        assert!(matches!(
            cli.command,
            Some(Command::Sessions { cmd: SessionsCmd::Reopen { ref session_id } })
                if session_id == "100-aaaaaaaa"
        ));
    }

    #[test]
    fn bare_invocation_still_parses_with_no_subcommand() {
        let cli = Cli::parse_from(["agent"]);
        assert!(cli.command.is_none()); // REPL default unchanged
    }

    #[test]
    fn list_sessions_marks_parked() {
        // tempdir root; write two descriptors via write_descriptor
        // ("100-aaaaaaaa", "200-bbbbbbbb"); under 200's dir create
        // checkpoint/parked.json (any bytes — has_park is an existence
        // check). let rows = list_sessions(root.path());
        // assert rows[0].session_id == "200-bbbbbbbb" (newest first) and
        // rows[0].parked; assert !rows[1].parked.
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-cli sessions`
Expected: COMPILE ERROR (`command` field absent).

- [ ] **Step 3: Implement**

`Cli` gains `#[command(subcommand)] command: Option<Command>` (all ~70
existing flags stay top-level; `None` ⇒ today's REPL path, so bare `agent`
is unchanged). Early in `main`, before the REPL setup:

```rust
    if let Some(Command::Sessions { cmd }) = &cli.command {
        return run_sessions_cmd(cmd, &cli).await;
    }
```

`list_sessions` is a pure helper (testable without HOME):

```rust
struct SessionRow {
    session_id: String,
    workspace: PathBuf,
    created_ms: u64,
    parked: bool,
}

fn list_sessions(root: &Path) -> Vec<SessionRow> {
    agent_runtime_config::scan_descriptors(root)
        .into_iter()
        .map(|d| {
            let parked = agent_core::checkpoint::has_park(
                &agent_runtime_config::session_dir(root, &d.session_id).join("checkpoint"),
            );
            SessionRow { session_id: d.session_id, workspace: d.workspace, created_ms: d.created_ms, parked }
        })
        .collect()
}
```

`run_sessions_cmd` for `List`: derive the root exactly like the run path
does (build the flag-derived `RuntimeConfig` via the existing
`runtime_config_from_cli`, then `sessions_root(&cfg)`; error exit 2 if
None), print one line per row:
`{session_id}  {created_ms}  {workspace}  [PARKED]` (raw ms — no new date
dep), footer hint `reopen a parked session: agent sessions reopen <id>`.

`Cargo.toml`: add `agent-server = { path = "../agent-server" }` (verified
cycle-free: agent-server does not depend on agent-cli). `cargo tree -p
agent-cli -i agent-server` should show only the new edge.

`AGENTS.md`: in the CLI surface bullet (locate the 4B-0 session-descriptor
line by content), append: `agent sessions list|reopen <id>` — parked-run
listing and reopen (4B-2).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-cli && cargo build -p agent-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-cli AGENTS.md agent/Cargo.lock
git commit -m "feat(cli): sessions subcommand with parked-aware list (4B-2)"
```

---

### Task 9: CLI durability — checkpointer wiring, park-and-exit timeout, Ctrl-C messaging

**Files:**
- Modify: `agent/crates/agent-cli/src/main.rs` (checkpointer + Ctrl-C
  message), `agent/crates/agent-cli/src/approval.rs` (timeout arm)
- Test: inline tests in `approval.rs`; park-retention pin in
  `agent-core/src/loop_.rs` tests

**Interfaces:**
- Consumes: `Checkpointer::new(dir, key, session_id)`,
  `load_or_create_secret`, `metadata_root`, Task 1's response shape.
- Produces: CLI runs park at Ask like server runs; the interactive timeout
  parks-and-exits (E5); Ctrl-C with an Ask pending prints "left parked".

- [ ] **Step 1: Write the failing tests**

`approval.rs`:

```rust
    #[tokio::test]
    async fn timeout_with_durable_park_prints_hint_and_exits() {
        // NOTE (reviewer m9): no `new_for_test` exists — TerminalApproval
        // today has `new`, a #[cfg(test)] `with_prompt`, and a Default
        // impl, ALL of which gain the new field. Build the rig from the
        // test constructor this task adds (with_prompt + park_exit
        // setter): 1ms timeout, a prompt that sleeps 500ms, park_exit:
        // Some(ParkExit { session_id: "100-aaaaaaaa".into(), exit:
        // Box::new(captured_fn) }) — the exit hook is injected for tests
        // (production installs std::process::exit). Drive request();
        // assert the exit hook fired with code 0 and the message line
        // contains "run parked" and "agent sessions reopen 100-aaaaaaaa".
    }

    #[tokio::test]
    async fn timeout_without_durable_wiring_keeps_denying() {
        // park_exit: None (no checkpointer was built) → today's behavior:
        // returns ApprovalResponse::Deny { feedback: None } (pins the
        // guard in refinement 4).
    }
```

`loop_.rs` (the load-bearing pin from refinement 4b — **this is a definite
baseline bug fix, not a maybe**; reviewer M1 verified the cancel arm calls
`ck.clear_park()` unconditionally at ~loop_.rs:1419-1421, under the comment
"Auto-deny (E5 knob) and cancel-deny commit the same way"):

```rust
    #[tokio::test]
    async fn cancelled_while_parked_retains_the_park_file() {
        // Reuse the 4B-1 park-test rig (CuratedContext + asking policy +
        // checkpointer on a tempdir; locate by content:
        // ask_parks_before_blocking…) with an approval channel that never
        // answers (pending forever). Drive run_with_cancel; once
        // has_park(dir) turns true (poll with a short timeout), cancel the
        // token; await the run's exit. THEN assert has_park(dir) is STILL
        // true — cancellation must not consume or clear a parked ask
        // (it is the CLI Ctrl-C / frontend-disconnect story, spec §2.7).
        // WILL FAIL at baseline. Fix site: the clear_park() in the cancel
        // branch at ~loop_.rs:1419-1421 — gate the clear on a REAL channel
        // answer (approve / deny / E5 auto-deny), never on
        // cancel.is_cancelled(). An auto-deny IS an answer and must still
        // clear (keep the existing auto-deny pin green).
    }
```

This fix also changes the SERVER's deliberate-cancel semantics (same code
path) — that consequence is the refinement 4b owner escalation; record the
disposition in the review log at the gate.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-cli approval && cargo test -p agent-core cancelled_while_parked`
Expected: cli COMPILE ERROR (`ParkExit` absent); the core pin FAILS
(clear-on-cancel is a verified baseline bug — the fix lands in this task).

- [ ] **Step 3: Implement**

`main.rs` — build the checkpointer where `checkpoint: None` sits today
(~:294; locate by content), after the descriptor write so `session_id` and
`sessions_root` are in scope:

```rust
    // Durable parks (4B-2): CLI runs park at Ask exactly like server runs.
    // Degrades to live-only (None) without HOME/secret — never fails boot.
    let checkpoint = agent_runtime_config::metadata_root()
        .and_then(|meta| agent_runtime_config::load_or_create_secret(&meta).ok())
        .and_then(|key| {
            let root = agent_runtime_config::sessions_root(&rt)?;
            Some(agent_core::Checkpointer::new(
                agent_runtime_config::session_dir(&root, &session_id).join("checkpoint"),
                key,
                session_id.clone(),
            ))
        });
```

and pass `checkpoint: checkpoint.clone()` in `LoopParts`.

`approval.rs` — inject the park-exit capability:

```rust
pub struct ParkExit {
    pub session_id: String,
    /// Test seam; production = Box::new(|code| std::process::exit(code)).
    pub exit: Box<dyn Fn(i32) + Send + Sync>,
}
```

`TerminalApproval` gains `park_exit: Option<ParkExit>` — updating ALL
construction points (reviewer m9): `new`, the `#[cfg(test)] with_prompt`,
and the `Default` impl (all build the struct literal; `Default` keeps
`None`), plus a `with_park_exit` production constructor. `ParkExit` also
carries the trace handle for the pre-exit flush (refinement 4 hazard b):

```rust
pub struct ParkExit {
    pub session_id: String,
    /// Flushed before exit — process::exit skips Drop, and buffered trace
    /// tail-loss on every park-exit would be a real audit gap.
    pub trace: Option<Arc<agent_runtime_config::TraceWriter>>,
    /// Set ONLY by the reopen driver (Task 10): the checkpoint dir whose
    /// resume.lock must be released before exit — process::exit skips
    /// Drop, and a leaked lock would refuse the NEXT reopen.
    pub release_lock: Option<PathBuf>,
    /// Test seam; production = Box::new(|code| std::process::exit(code)).
    pub exit: Box<dyn Fn(i32) + Send + Sync>,
}
```

The timeout arm becomes:

```rust
            Err(_elapsed) => {
                if let Some(park) = &self.park_exit {
                    eprintln!(
                        "\nApproval unanswered for {}s — run parked; answer later with:\n  agent sessions reopen {}",
                        self.timeout.as_secs(), park.session_id
                    );
                    if let Some(t) = &park.trace {
                        t.flush(); // add a pub flush() if TraceWriter lacks one
                    }
                    if let Some(dir) = &park.release_lock {
                        agent_core::checkpoint::release_resume(dir); // reopen only
                    }
                    (park.exit)(0);
                }
                eprintln!("\nApproval timed out; denying.");
                ApprovalResponse::Deny { feedback: None }
            }
```

(The park file already exists on disk — the loop wrote it before blocking,
spec §2.3; exiting without unwinding is the retention guarantee,
refinement 4. This can fire from a CHILD loop's ask — safe: the child
`write_park`s (flushing ancestor snapshots) before it can block on the
prompt, and the D12 gate mutex means no second prompt is concurrently in
flight. MCP/sandbox orphaning on exit = named accepted residual.
`(park.exit)(0)` never returns in production; in tests the hook records
and the fall-through Deny is the harmless test-path return.)

`main.rs` wires it only when durable:

```rust
    let approval = TerminalApproval::with_park_exit(checkpoint.as_ref().map(|_| ParkExit {
        session_id: session_id.clone(),
        trace: trace.clone(),
        release_lock: None, // ordinary runs hold no resume lock
        exit: Box::new(|code| std::process::exit(code)),
    }));
```

Ctrl-C arm (~:338, locate by content) gains the parked message — check the
checkpointer's disk state, not in-memory flags:

```rust
    _ = tokio::signal::ctrl_c() => {
        cancel.cancel();
        if checkpoint.as_ref().is_some_and(|c| agent_core::checkpoint::has_park(c.dir())) {
            eprintln!("\n^C — a pending approval was left parked; answer later with:\n  agent sessions reopen {session_id}");
        } else {
            eprintln!("\n^C cancelling…");
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-cli && cargo test -p agent-core && cargo test -p agent-server`
Expected: PASS (the agent-server sweep guards against accidental
`end_turn`/`clear_park` changes if the core pin forced a fix).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-cli agent/crates/agent-core
git commit -m "feat(cli): durable parks + timeout parks-and-exits + Ctrl-C left-parked messaging (4B-2 E5)"
```

---

### Task 10: CLI `sessions reopen` — inline re-prompt with feedback + resume driver

**Files:**
- Modify: `agent/crates/agent-server/src/resume.rs` (extract
  `scan_parked_session`), `agent/crates/agent-cli/src/main.rs` (reopen
  arm), `agent/crates/agent-cli/src/approval.rs` (feedback line on deny)
- Test: inline tests in `resume.rs`, `approval.rs`

**Interfaces:**
- Consumes: Tasks 1, 2 (`ParkedAnswer`, `commit_answer`), 8 (scaffold +
  agent-server dep), 9 (checkpointer pattern); `CuratedContext` restore +
  `resume_with_cancel` + `Checkpoint::resume_turn` (4B-1);
  `build_trace` (per-descriptor, Task 6's mechanism).
- Produces:

```rust
// agent-server/src/resume.rs — extracted per-session scanner; scan_parked
// becomes a thin loop over it (behavior-identical):
pub fn scan_parked_session(
    root: &Path,
    key: &[u8; 32],
    descriptor: agent_runtime_config::SessionDescriptor,
) -> Option<ParkedSession>;   // None ⇔ no park under the session dir
```

- [ ] **Step 1: Write the failing tests**

`resume.rs`:

```rust
    #[test]
    fn scan_parked_session_finds_the_single_session_tree() {
        // Reuse the existing scan test's planting rig for ONE session:
        // descriptor "200-bbbbbbbb" + root dispatch-kind + one gate-kind
        // child. scan_parked_session(root, &key, descriptor) → Some with
        // 1 ask; a descriptor with no checkpoint dir → None.
        // And: scan_parked's existing tests still pass unchanged (it now
        // delegates).
    }
```

`approval.rs`:

```rust
    #[test]
    fn deny_prompt_collects_optional_feedback() {
        // prompt_for_answer_with_reader over a Cursor stdin of "n\nuse staging\n"
        // → ParkedAnswer { approve: false, feedback: Some("use staging".into()) };
        // "n\n\n" → feedback None; "y\n" → approve true, no feedback read.
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-server resume && cargo test -p agent-cli approval`
Expected: COMPILE ERROR (functions absent).

- [ ] **Step 3: Implement**

`resume.rs`: lift the per-descriptor body of `scan_parked` (the
`has_park` check + `ParkedSession` construction + `walk`) into
`scan_parked_session`; `scan_parked` iterates descriptors, skips
`own_session_id`, and calls it. Pure refactor — existing tests must pass
untouched.

`approval.rs`: a reusable reader-parameterized prompt (the reopen path is
synchronous — no gate mutex needed, one ask at a time):

```rust
/// Reopen-path prompt: y/n/always, then an optional feedback line on deny.
pub fn prompt_for_answer_with_reader<R: std::io::BufRead>(
    summary: &str,
    who: &str,
    mut input: R,
) -> ParkedAnswer {
    print!("\n\x1b[35mAllow:\x1b[0m {who}{summary} ? [y]es / [n]o / [a]lways: ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if input.read_line(&mut line).is_err() {
        return ParkedAnswer { approve: false, feedback: None };
    }
    match line.trim().to_lowercase().as_str() {
        "y" | "yes" | "a" | "always" => ParkedAnswer { approve: true, feedback: None }, // E2: always = plain approve
        _ => {
            print!("Feedback for the agent (optional, Enter to skip): ");
            let _ = std::io::stdout().flush();
            let mut fb = String::new();
            let _ = input.read_line(&mut fb);
            let fb = fb.trim();
            ParkedAnswer {
                approve: false,
                feedback: (!fb.is_empty()).then(|| fb.to_string()),
            }
        }
    }
}
```

`main.rs` reopen arm (`run_sessions_cmd`, `Reopen { session_id }`):

1. Config + root exactly like `List`; find the descriptor by id in
   `scan_descriptors(&root)` (missing ⇒ `eprintln!` + exit 2). Workspace
   dir missing ⇒ refuse naming the path (spec §4), parks retained, exit 2.
2. `load_or_create_secret(&metadata_root)` (wrong-length = the loud
   InvalidData error, surface + exit 2);
   `let Some(parked) = agent_server::resume::scan_parked_session(&root, &key, descriptor) else { eprintln!("session {id}: nothing parked"); exit 2 }`.
   Any `parked.errors` ⇒ print each `checkpoint unreadable; run cannot be
   resumed (…)` and exit 2 (never resume over corruption, spec §4).
3. **Claim the resume lock BEFORE prompting the human** (refinement 11 —
   no wasted answer): `agent_core::checkpoint::claim_resume(&parked.root_dir)`;
   on `Ok(false)`/`Err` print `session {id} is being resumed elsewhere
   (another daemon may hold it); if that process crashed, remove
   {root_dir}/resume.lock` and exit 2. From here on, EVERY error exit in
   this command releases the lock (`release_resume`) — success reaps it
   with the tree.
4. Assemble the loop FIRST (display derivation needs it): mirror
   `session.rs start_resume`'s restore sequence (locate by content: the
   block between the guard insert and `resume_with_cancel`; copy its order
   exactly — the functions are all public to agent-cli, reviewer-verified:
   `restore_artifacts`/`load_artifact_dump`, `CuratedContext::restore`,
   `verify_tally_floor`): fresh `SessionArtifacts`/todos/compact-flag →
   artifacts restore from the root checkpoint's dump → `CuratedContext`
   restore from the root checkpoint → assemble via the CLI's OWN
   `assemble_loop` call (reuse the existing `LoopParts` block in `main.rs`
   with: `workspace = descriptor.workspace`, `approval = TerminalApproval`
   (WITH park-exit wired to THIS session id AND
   `release_lock: Some(parked.root_dir.clone())` — a later Ask in the
   resumed run parks again, and the exit must free the resume lock for the
   next reopen), `checkpoint = Some(Checkpointer::new(<that session's
   checkpoint dir>, key, descriptor.session_id.clone()))`,
   `trace = build_trace(&cfg, &descriptor.session_id)` (per-descriptor,
   refinement 5a), sink = `TerminalSink`).
5. For the first UNANSWERED ask (CLI answers one; the resumed tree re-asks
   the rest live — same shape as the server's first-answer-wins):
   **re-derive the display from stored args** via
   `built.loop_.derive_intent(&call.name, &call.args)` — reviewer m4:
   `derive_intent` is ALREADY `pub` on `AgentLoop` (~loop_.rs:216, the
   exact helper the server attach path calls at ~session.rs:230); no
   pub-ification, no duplication (spec §3.4: derive-not-stored on every
   surface). Attribution prefix from `ask.origin` (same format as the live
   prompt). Prompt via
   `prompt_for_answer_with_reader(summary, &who, std::io::stdin().lock())`.
6. `agent_server::resume::commit_answer(&ask, &answer, &key)` — the durable
   commit (crash after this point auto-resumes on next attach, spec §2.3) —
   then
   `loop.resume_with_cancel(&mut ctx, root_chk.resume_turn(answer_opt), token)`
   where `answer_opt` is `Some(answer)` if the answered ask was the ROOT
   park, `None` if it was a child's (the child rebind consumes its own
   `answer.json` — Task 2 / 4B-1 dispatch path).
7. Ctrl-C during the resumed run: same select! + parked messaging as
   Task 9 (lock note: leave the lock in place on Ctrl-C-park? NO —
   `release_resume` before exiting, the run is parked, not resuming; the
   park must be claimable by the next reopen). On success the checkpoint
   tree is gone (loop/dispatch reap, lock included); print the stats line;
   exit 0. On error: print `resumed run failed: {e}`, parks retained,
   `release_resume`, exit 1.

- [ ] **Step 4: Run tests + build**

Run: `cd agent && cargo test -p agent-server -p agent-cli && cargo build -p agent-cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server agent/crates/agent-cli
git commit -m "feat(cli): sessions reopen — inline re-prompt with deny feedback + tree resume (4B-2)"
```

---

### Task 11: Live drive — CLI park → reopen → complete against the local model

**Files:**
- Create: `agent/scripts/reopen_drive.exp` (or scratchpad-local if the
  repo prefers not to commit it — implementer's call, note in ledger)
- Test: manual live drive, evidence pasted in the ledger

The 4B-1 lesson stands: the live drive caught a real abort-on-attach bug
unit tests structurally missed. 4B-2's genuinely new integration surface is
the CLI reopen path (real binary, real stdin, real model, real park files).

- [ ] **Step 1: Preconditions**

- Local llama server up (`docker start llama-agent` if stopped; it has
  `--restart no` — see memory note). Verify: `curl -s localhost:8080/health`.
- Build: `cd agent && cargo build -p agent-cli --release` (prebuilt binary —
  HOME-redirect breaks rustup for `cargo run`, 4B-0 gotcha).

- [ ] **Step 2: Park via Ctrl-C**

Drive with an expect script (stdin is interactive; pipes won't do SIGINT
mid-prompt):

```tcl
#!/usr/bin/expect -f
# reopen_drive.exp — park a CLI run at an approval, then die.
set timeout 120
set env(HOME) $::env(DRIVE_HOME)
spawn ./target/release/agent --workspace $::env(DRIVE_WS) --base-url http://localhost:8080
expect ">"
send "run the command: touch /tmp/4b2-drive-marker\r"
expect -re {Allow:.*\[y\]es}
# Ask is on screen ⇒ the park is already written. Kill without answering:
exec kill -INT [exp_pid]
expect eof
```

Run:

```bash
export DRIVE_HOME=$(mktemp -d) DRIVE_WS=$(mktemp -d)
expect agent/scripts/reopen_drive.exp
```

Assert on disk: exactly one
`$DRIVE_HOME/.rusty-agent/sessions/<id>/checkpoint/parked.json` exists;
note the `<id>`. Also assert the "left parked" hint printed.

- [ ] **Step 3: Reopen, answer with feedback-deny first, then approve**

```bash
SID=<id from step 2>
# 3a: deny with feedback — the model must SEE the feedback text:
printf 'n\nuse /tmp/other-marker instead\n' | HOME=$DRIVE_HOME ./target/release/agent sessions reopen $SID
```

Expected: the re-derived prompt shows the REAL command (`touch
/tmp/4b2-drive-marker`); the resumed run continues and the model's next
turn visibly reacts to the feedback (transcript shows it retrying with
`/tmp/other-marker` or acknowledging). The run then either completes or
asks again — approve any follow-up ask interactively (`y`).

Assert after exit: checkpoint dir for `$SID` is GONE (completed tree
reaped); `$DRIVE_HOME/.rusty-agent/sessions/$SID.jsonl` (trace) gained the
resumed events (grep the feedback string — per-descriptor trace,
refinement 5a).

- [ ] **Step 4: Record evidence**

Paste the transcript tail + the three disk assertions into the execution
ledger (`.superpowers/sdd/progress.md`). If any step fails: STOP, treat as
a product bug (systematic-debugging), fix, re-drive.

---

### Task 12: Sweep, full CI, branch finish

**Files:**
- Modify (as the sweep finds): `agent/config.example.toml`
  (reviewer-verified: no `.json` variant exists; the E5 knob text at
  ~lines 21-24 gains the CLI parks-and-exits sentence),
  `agent/crates/agent-cli` help/about strings,
  `agent/crates/agent-server/src/wire.rs` frame doc comments,
  `docs/superpowers/specs/2026-07-10-durable-hitl-design.md` §0 (one-line
  ratification note: refinement-5 deferrals landed in 4B-2 — pending owner
  sign-off at the merge gate)
- Test: `bash scripts/ci.sh`

- [ ] **Step 1: Sweep prose that this slice made stale**

```bash
grep -rn "parks-and-exits is 4B-2\|4B-2 scope\|restart the daemon to retry" agent/crates --include=*.rs
grep -rn "approval" agent/config.example.* docs/AGENTS.md AGENTS.md 2>/dev/null
```

Every hit that describes pre-4B-2 behavior is updated (the
`DEFAULT_TERMINAL_APPROVAL_TIMEOUT` doc comment in `agent-cli/approval.rs`
is a known one). The `docs/okf/deepagents-refactor/` gap rows stay
untouched (spec §2.8: bundle update is a follow-on).

- [ ] **Step 2: Full CI**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: GREEN (okf check, skills lint, fmt, clippy, agent tests,
conditional src-tauri, web typecheck + vitest).

- [ ] **Step 3: Commit the sweep**

```bash
git add -A
git commit -m "docs(sweep): de-stale approval-timeout prose + config example for 4B-2 surfaces"
```

- [ ] **Step 4: Whole-branch review + merge**

Per repo convention: whole-branch review (fresh reviewer, spec §3
invariants + cross-task seams at the live tip), fix wave if needed, then
`git merge --no-ff feature/durable-hitl-4b2` onto main with a green ci.sh
at the merged tree. **Commit/merge/push only with owner go-ahead.**

---

## Self-review notes (author, 2026-07-10)

**Spec coverage (slice 4B-2 items, spec §0 + §2.7 + merge-gate deferrals):**
- Deny-feedback on `ApprovalResponse` + wire `Decision` `Copy` loss + match
  fan-out ✅ T1+T3 (fan-out enumerated from a live-source survey: the wire
  `From` impl, server auto-deny arms, CLI stdin arms, policy test, waiter).
- Feedback splices into tool-result text ✅ T1 (`denial_content`, live) +
  T2 (durable answers, resume splice, child rebind).
- `parked_runs` / `approval_resolved` / `resumed` frames ✅ T4 (additive;
  built on the existing attach scan per 4B-1 header note 8).
- Web parked banner + deny feedback field ✅ T7.
- CLI session listing (parked marked) ✅ T8; reopen-with-re-prompt ✅ T10;
  timeout parks-and-exits (E5) ✅ T9; Ctrl-C "left parked" ✅ T9.
- Merge-gate deferrals: resumed-run trace attribution ✅ T6; in-life
  failed-resume retry ✅ T5; stale-prompt retraction ✅ T4.
- Spec §6 rows owned: deny-feedback reaches the model ✅ T1/T2 units + T11
  live (native protocol; prompted-protocol rendering shares the same
  tool-result content path — the content string is protocol-independent);
  cross-surface first-answer-wins + retraction ✅ T4; E5 CLI rows ✅ T9;
  full ci.sh ✅ T12.

**Invariant mapping (spec §3):** §3.1 T1's byte-parity pins (no-feedback
deny content identical; no new checkpoint I/O anywhere — CLI parks only at
Ask); §3.2 policy engine untouched (T1 explicitly leaves
`Decision::Deny(String)` alone); §3.3 reopen re-derives config/floors by
assembling through the CLI's current-config path against the descriptor
workspace (T10); §3.4 reopen re-derives display via the already-pub
`AgentLoop::derive_intent`, never the stored string (T10 step 5); §3.5
refinement 1's byte-compat serde + additive frames (T3/T4 tests); §3.6
trace contract unchanged, only attribution fixed (T6); §3.8/§3.9/§3.10
untouched by construction (no tally, child-assembly, or memory changes —
T2 changes the answer's *payload type* only); §3.7 is STRENGTHENED twice:
the T9 clear-on-cancel fix (a verified baseline bug was deleting parks on
cancel, refinement 4b) and the T5 resume lock (cross-process double-resume
would have double-executed approved tools, refinement 11) — both changes
escalated/recorded, not silent.

**Type-consistency check:** `ParkedAnswer { approve, feedback }` defined T2,
consumed T4 (waiter), T10 (CLI prompt + commit_answer); `denial_content`
defined T1, consumed T2; `Decision::Deny { feedback }` T3, consumed T7 (TS
mirror union); `ParkedRunDto`/`ParkedRun` T4/T7 field-identical
(session_id/workspace/created_ms/asks/error); `claim_resume`/
`release_resume` defined T5, consumed T5 (daemon) + T9 (`ParkExit.
release_lock`) + T10 (CLI claim/release); `scan_parked_session` T10,
also available to T8's lister (T8 deliberately uses the cheaper
`has_park`); `ParkExit { session_id, trace, release_lock, exit }` T9,
reused T10 (reopen wires session id + lock dir); `Command`/`SessionsCmd`
T8, consumed T10; `send_event` defined T4, available to T5's error paths.

**Deliberately NOT in this slice (spec §5 + refinements):** edit/respond
decisions; ApproveAlways grant store; multi-subscriber broadcast/replay
buffer (refinement 3); web prompt queue (refinement 9); post-resume CLI
REPL (refinement 6); shared Session/CLI resume-driver abstraction
(refinement 10); OKF bundle gap-row updates.

## Panel & review log

- **2026-07-10 — plan review** (two reviewers per the SDLC's "lighter
  adversarial pass when design decisions leak into the plan": coverage/
  decomposition/buildability + adversarial architecture, both opus, both
  live-source-verified at 577b40d): **both APPROVE-WITH-FIXES — all
  findings FOLDED IN.**
  **Blocker fixed in place:** cross-process double-resume — the 4B-1
  `resuming` guard is per-daemon-heap and the CLI reopen is a separate
  process; nothing prevented two drivers concurrently executing the same
  approved batch (double host side effects). Fix = O_EXCL
  `checkpoint/resume.lock` claimed by both drivers (refinement 11, Task 5
  primitive + Task 10 CLI wiring); it also subsumes the trace-aliasing and
  retry-race findings.
  **Majors fixed in place:** (1) the live deny arm is a `select!` that
  collapses the response to a bool (both reviewers converged) — Task 1 now
  shows the real capture-the-response shape and preserves the distinct
  `"run cancelled"` string; (2) cancel-while-parked `clear_park()` at
  ~loop_.rs:1419 is a VERIFIED baseline bug, not a maybe — Task 9 fixes
  the exact site (clear only on real answers) with the server-semantics
  consequence escalated (refinement 4b); (3) Task 5's premise of an
  existing failed-resume test was false — rewritten as net-new with
  concrete failure levers; (4) Task 1's fan-out missed the entire
  `agent-runtime-config` test crate + agent-core test channels — list
  completed, verify step widened to the full workspace; (5) `process::exit`
  hazards (trace tail-loss, child-channel exit, MCP/sandbox orphans) —
  flush + lock-release added to `ParkExit`, residuals named.
  **Minors folded:** `Eq` drop stated; `hex(&hmac_sha256(…))` (no
  `hmac_sha256_hex`); `send_event` helper is net-new; `parked_list`
  hoisting; `derive_intent` already pub on `AgentLoop` (Task 10 reordered
  to assemble-before-prompt); reducer is an if-chain not a switch;
  `build_resume_loop` has two callers; `config.example.toml` resolved;
  constructor naming aligned; legacy-answer test now also asserts the
  loop-level re-prompt.
  **ESCALATED TO THE GATE — both DECIDED by owner 2026-07-10:**
  - **P1 (refinement 4b): RETAIN ON CANCEL, both surfaces.**
    Cancel-while-parked retains the park on the server too (desktop
    deliberate-cancel leaves a resumable parked run listed on next
    attach). Spec-aligned (§2.7 pins the CLI Ctrl-C story on this path);
    the 4B-1 server behavior change is accepted — one uniform rule, parks
    are cleared only by answers (approve / deny / E5 auto-deny).
  - **P2 (refinement 3): SEQUENTIAL-ATTACH REINTERPRETATION RATIFIED.**
    Spec §6's "two attached frontends" row is amended to sequential-attach
    (attach A → answer → attach B sees no prompt + `approval_resolved`);
    the single-subscriber slot makes the literal simultaneous case
    unrepresentable, and cross-process first-answer-wins is delivered by
    the resume lock + answer commit point. Multi-subscriber broadcast
    stays deferred (adjacent to the 3B-2 replay-buffer deferral). Spec §6
    updated in place with a ratification note.
  **Accepted residuals (recorded):** stale `resume.lock` after a SIGKILLed
  resume holder refuses resume until removed by hand (fail-closed;
  age-check/`--force` = named follow-on); legacy-format `answer.json`
  fails the new MAC after an upgrade ⇒ benign live re-ask; failed-resume
  banner row absent until next attach; trace `seq` restart on append;
  MCP/sandbox orphaning on park-exit; web single-prompt overwrite
  (refinement 9).
