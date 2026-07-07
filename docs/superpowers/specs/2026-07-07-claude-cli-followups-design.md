# Claude CLI Optimization Follow-ups — Design

**Date:** 2026-07-07
**Status:** Approved (brainstorm complete)
**Target:** `agent/crates/agent-model/src/claude_cli.rs`, `agent/crates/agent-cli/src/main.rs`, `agent/crates/agent-runtime-config/src/assemble.rs`
**Parent:** `docs/superpowers/specs/2026-07-07-claude-cli-optimization-design.md` (merged @ 84d117b); findings from its whole-branch review.

## Problem

Three residuals from the claude-cli optimization branch:

1. **Shared child-client session state.** `DispatchAgentTool` receives one
   `Arc<ClaudeCliClient>` built at assembly; nested dispatch clones
   `DispatchDeps` wholesale (`agent-core/src/dispatch.rs:415`), so **every
   descendant subagent at every depth shares one
   `Mutex<Option<SessionState>>`**. Concurrent siblings mutually clobber it —
   session reuse is defeated among them (each multi-round child falls back to
   full sends) — and two children whose transcripts share a strict-prefix
   relationship can `--resume` the same CLI session with different suffixes
   (contamination corner).
2. **agent-cli never validates.** `RuntimeConfig::validate()` is only called on
   the server's settings-apply path; a typo'd `--claude-effort` (or any bad
   knob) sails through CLI startup.
3. **Duplicated fresh-client block.** The claude-cli distinct-instance code
   from `5c9bf24` appears verbatim in both the compaction `or_else` and the
   child arm of `assemble.rs`.

A fourth follow-up — `graphify . --update` to ingest the merged branch — is an
ops action, **not part of this spec**; it runs directly after this work merges.

## Decisions taken during brainstorm

- Package items 1–3 as one small branch/spec; graphify runs separately.
- Item 1 goal: **correct AND fast for descendants** — concurrent subagents at
  any depth get working session reuse, and the contamination corner is closed.
- Approach chosen: **checkout-keyed session pool inside `ClaudeCliClient`**
  (Approach A). A `DispatchDeps` model-factory (Approach B) was declined: it
  churns the agent-core dispatch seam for a backend-specific problem and needs
  a factory story for openai. Accept-with-comment (Approach C) was declined by
  goal choice.
- The assemble.rs parent/child/compaction distinct instances from `5c9bf24`
  **stay** (belt-and-suspenders); the pool makes intra-tree sharing safe.
- Item 2: **hard exit** at startup on validation failure (matches server
  behavior; hardens all knobs, not just the claude ones).

## Section 1 — Session pool with checkout semantics

`ClaudeCliClient.state: Arc<Mutex<Option<SessionState>>>` becomes
`sessions: Arc<Mutex<Vec<SessionState>>>` with `const MAX_POOLED_SESSIONS:
usize = 8`.

**Plan (per `stream()` call):**
- `session_reuse: false` → exactly today's behavior: `FreshEphemeral`, no pool
  interaction.
- `session_reuse: true` → lock the pool; find entries whose `fingerprints` are
  a **strict prefix** of the incoming transcript's fingerprints; if several
  match, take the **longest**; **remove it from the pool** (checkout). Then the
  existing three-state logic applies unchanged:
  - matched entry not `persisted` → `FreshPersisted`;
  - matched entry `persisted` with a `session_id` and a non-assistant-only
    suffix → `Resume { session_id, suffix_start }`;
  - anything else (no match, no id, assistant-only suffix) → `FreshEphemeral`.

**Commit (successful exit only):** push the updated `SessionState` at the back
of the pool; if `len > MAX_POOLED_SESSIONS`, evict from the front (oldest).
Insertion order is the LRU order — re-inserted entries move to the back.

**Failure (parse error, io error, non-zero exit, wait error):** do **not**
re-insert. The checked-out entry stays dropped, so the loop's retry finds no
match and lands on a fresh full send. This replaces the previous
"reset state to None before `yield Err`" mechanism with equivalent semantics
(reset happens at checkout time, ahead of any yield — the unreachable-after-
yield hazard cannot recur).

**Why checkout closes the corner:** a concurrent second caller with the same
prefix finds the entry already checked out, matches nothing, and safely
degrades to `FreshEphemeral`. Two callers can never hold the same
`session_id` plan simultaneously. No lock is held across an await (lock,
match, remove, unlock — all synchronous).

**Bounded growth:** one-shot callers with reuse on (compaction, evals through
a reuse-enabled client) commit entries that never match again; the LRU cap
bounds the pool. No time-based eviction — 8 entries of `Vec<u64>` fingerprints
are negligible.

**Behavior compatibility:** a single sequential caller sees byte-identical
behavior to the current single-slot state (walk test, rewrite-reset test,
error-reset test, reuse-off test all pass unchanged).

## Section 2 — `rt.validate()` on the agent-cli path

After the `RuntimeConfig` is fully assembled from clap in `agent-cli`
(`runtime_config_from_cli` + any later mutation, i.e. the value handed to
`assemble_loop`), call `rt.validate()`. On `Err(msg)`: print
`error: {msg}` to stderr and exit with code 2. Placement must keep the check
unit-testable (validate the assembled config value; `main`'s exit path itself
is not under test).

## Section 3 — assemble.rs dedup helper

Fold the duplicated fresh-client construction into one local helper, e.g.
`fn fresh_claude_cli_client(cfg: &RuntimeConfig, claude_binary: &str,
api_key: Option<String>) -> Arc<dyn ModelClient>`, used by both the compaction
`or_else` and the child `None if backend == "claude-cli"` arm. Pure refactor;
no behavior change; the existing distinct-instance tests pin it.

## Section 4 — Tests and gate

New tests (styles follow the existing `claude_cli.rs` proc tests / assemble
tests):

- **Sibling interleave (proc test):** one reuse-on client; interleave calls of
  two transcript families A and B (A1, B1, A2, B2, A3, B3 where each *n+1*
  strictly extends *n*); both families must reach `--resume` on their third
  call — pinning that concurrent-ish siblings no longer clobber each other.
- **Checkout semantics (unit test):** plan the same extension twice without an
  intervening commit → first yields `Resume`/`FreshPersisted`, second yields
  `FreshEphemeral` (entry checked out).
- **Pool cap (unit test):** commit `MAX_POOLED_SESSIONS + n` unrelated
  transcripts; pool length stays at cap and the oldest entries are evicted.
- **CLI validate (unit test):** an assembled config with `claude_effort:
  Some("banana")` fails validation on the CLI path; a clean config passes.

All existing suites must pass unchanged. Gate: full `bash scripts/ci.sh`.
No live smoke — no flag, protocol, or prompt-surface change.

## Error handling summary

| failure | behavior |
|---|---|
| stream failure with a checked-out entry | entry stays dropped; retry lands fresh (same net semantics as today's reset) |
| concurrent same-prefix callers | second caller misses the pool → FreshEphemeral (safe, full send) |
| pool overflow | evict oldest (LRU by insertion order) |
| bad knob at CLI startup | stderr message + exit 2 |

## Out of scope

- `DispatchDeps` factory plumbing / any agent-core change (declined Approach B).
- openai client changes (stateless; sharing is harmless).
- Reverting the assemble.rs distinct-instance workaround (kept deliberately).
- graphify `--update` (ops action, runs post-merge).
