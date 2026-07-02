# Small-Residuals Sweep Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the 13 accumulated small residuals (S1-S13 in the spec) in four crate-grouped tasks.

**Architecture:** No new subsystems. Each item is a bounded fix specified in `docs/superpowers/specs/2026-07-02-small-residuals-sweep-design.md` — THE SPEC IS THE REQUIREMENTS DOCUMENT; every task below names its spec items and the implementer reads those sections verbatim. Line anchors are approximate; live source governs.

**Tech Stack:** Rust (workspace `agent/`), web (vitest/tsc).

## Global Constraints

- Run cargo from `agent/` (`source ~/.cargo/env` if missing); `cargo fmt` touched crates before every commit; conventional commits.
- Run any full-workspace test/ci command with STDIN CLOSED (`< /dev/null`) until S13 lands (known wedge in agent-cli approval tests under open stdin).
- No wire changes anywhere in this cluster.
- TDD per item where a test is specified; pure refactors (S10) rely on existing suites staying green.

---

### Task 1: agent-core context items — S5, S10, S11

**Files:** `agent/crates/agent-core/src/curated.rs` (S5 dedup key, S10 pinned_tokens helper), `agent/crates/agent-core/src/snapshot.rs` + wherever `recall_block` lives (`context.rs`) for S11; tests in each.

Steps: for each item, read its spec section; write/extend the named failing test; implement; `cargo test -p agent-core` full crate; fmt.
Commit: `fix(core): sweep S5/S10/S11 — Evicted dedup key (messages,est_tokens), non-cloning pinned_tokens, snapshot memory segment uses capped recall block`

### Task 2: model/loop items — S4, S6

**Files:** `agent/crates/agent-model/src/types.rs` + `openai.rs` (Status.retry_after; parse header), `agent/crates/agent-model/src/claude_cli.rs` (env_remove + cache fold + fixture test), `agent/crates/agent-core/src/loop_.rs` (jittered_backoff + Retry-After honor + test bound changes).

Steps: spec §S4/§S6 verbatim; note ALL `ModelError::Status` construction/match sites across both crates need the new field (`..` where irrelevant — grep first); the paused-clock exact-700ms pin becomes the `[700ms, 875ms]` bound; new 429-with-Retry-After virtual-sleep test; claude_cli usage-fold fixture test. Full `cargo test -p agent-model && cargo test -p agent-core` + `cargo build -p agent-server -p agent-cli`.
Commit: `feat(model+core): sweep S4/S6 — Retry-After + jittered backoff; claude-cli drops AGENT_API_KEY and folds cache tokens into prompt_tokens`

### Task 3: tools/skills/config/sandbox items — S1, S3, S7, S8, S9, S12

**Files:** `agent/crates/agent-runtime-config/src/trace.rs` (S1) + `assemble.rs` (S12), `agent/crates/agent-skills/src/tools.rs` (S3), `agent/crates/agent-sandbox/src/strategy.rs` (S7), `agent/crates/agent-tools/src/registry.rs` (S8), `agent/crates/agent-memory/src/tools.rs` (S9); tests in each.

Steps: per spec sections; each item's named test first where specified. Suites: `cargo test -p agent-runtime-config -p agent-skills -p agent-sandbox -p agent-tools -p agent-memory` (all < /dev/null unnecessary here — no agent-cli — but harmless).
Commit: `fix(sweep): S1/S3/S7/S8/S9/S12 — trace 0600, L2 listing truncation, enforce copy, dup-tool warn, memory param descs, prompt budget warn`

### Task 4: cli/web items — S2, S13

**Files:** delete `web/src/components/ToolCall.tsx` (S2; then `cd web && npm run typecheck && npx vitest run`), `agent/crates/agent-cli/src/approval.rs` (S13 — read the impl first; prefer the existing `with_prompt` seam; verify the fixed test passes with an OPEN-pipe stdin: `cargo test -p agent-cli approval < <(sleep 300) &` style check or equivalent — the point is no dependence on process stdin).

Commit: `fix(cli+web): sweep S2/S13 — delete dead ToolCall.tsx; hermetic approval timeout test (no real-stdin dependence)`

### Task 5: Cluster gate

- [ ] `bash scripts/ci.sh < /dev/null` (background; ~10+ min cold) → green. No commit expected.
