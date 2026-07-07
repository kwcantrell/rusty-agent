# Full Harness + SDLC Audit — Design

**Date:** 2026-07-06
**Skills:** `harness-engineering` (`audit.md` playbook — procedure) + `agent-sdlc`
(`docs/okf/agent-sdlc/` — evidence layer)
**Predecessor:** `docs/superpowers/audits/2026-07-01-harness-deep-audit.md` — fully
closed 2026-07-02 (Top-10 fixed, both build opportunities built, backlog drained,
product-decision round adjudicated). This audit starts from that clean slate.

## Character

**REPORT ONLY.** No code is changed. Findings + ranked fixes; the human holds the
judgment gate. Every finding carries the four-field schema from `audit.md`:

```
severity:               high | med | low
file:line:              <repo-relative path>:<line or range>
violated principle:     <principle> — cited as a bundle path (docs/okf/agent-sdlc/...)
                        where a concept covers it; tier-tagged external source or
                        "first principles + runtime conventions" otherwise
concrete proposed fix:  one action the implementer should take
```

The citation discipline is the new element: the agent-sdlc bundle (36 sources,
23 concepts, conformance-clean 2026-07-06) replaces remembered blog posts as the
default evidence base. Auditors cite bundle paths so claims stay traceable.

Findings fixed and merged during the July close-out are **re-verified but not
re-reported** — they appear only in a "prior-state regressions" section if they
have re-opened. Product decisions the owner declined in the 2026-07-02 round
**may be re-proposed** when an auditor finds fresh evidence; the report may note
the prior decision for context, but it is not a bar.

## Scope — 11 dimensions

The original seven (re-run fresh, post-drain anchors) plus four covering surface
that has never been audited:

| # | Dimension | Anchors (open live at audit time — these drift) |
|---|-----------|--------------------------------------------------|
| 1 | Instructions & rule files | `agent/crates/agent-runtime-config/src/prompts.rs` (`BASE_SYSTEM_PROMPT` + re-dup ratchet), `CLAUDE.md`, dispatch `role` block injection, skill frontmatter/negative-constraint blocks |
| 2 | Tools | `agent-tools/src/tool.rs` + `types.rs` (`when_not_to_call` contract), registry duplicate handling, context/memory/http/mcp/skills tool prose, `dispatch_agent` schema |
| 3 | Sandboxes & execution | fail-closed sandbox + re-probe (`agent-tools`), `HostExecutor` env allow-list, sandbox-dev image + tri-state probe, MCP server spawning, CLI clap-default-shadowing gotcha |
| 4 | Orchestration & sub-agents | `agent-core/src/loop_.rs` (parallel dispatch, retry `ErrorClass`, stuck detection, graceful max_turns), `dispatch.rs` (depth, `ModelRef` routing, cancellation propagation, privilege inheritance) — the sub-agent capability as a finished whole, built after the last audit |
| 5 | Guardrails & policy | `agent-policy/src/engine.rs` + `command.rs` (hard floor, Layer A2, `/dev` redirect resolver), Destroy tier, prefix allowlist + `--output` arg-scan, post-exec validator (new, `5f41db5`), `agent-runtime-config` denylists |
| 6 | Observability | `TraceWriter` + child-record linkage, `SessionStats` subset counters, sub-agent attribution (`parent_id` chain), cost metering, ContextEvents, replayability of a failed turn |
| 7 | Context engineering (Spine B) | `context.rs`/`curated.rs` turn-atomic curation, ingestion cap, calibrated budgeting (`calib_ratio_micros`), Examples context type, offload/recall paging, memory recall block |
| 8 | Desktop/web design-tab harness | `src-tauri/` dev-server lifecycle (orphan-free teardown, pm/script whitelist, pipe draining), two-layer localhost URL guard (JS authoritative / Rust coarse), canvas/feedback/config surfaces, WebDriver e2e harness, `src-tauri` absent from `scripts/ci.sh` |
| 9 | Skills & knowledge layer | both skill trees (`.agents/skills/` Claude-facing vs runtime `.agent/skills` registry), OKF bundle integrity (`scripts/okf_check.py` run live), skill `description` routing overlap, graphify integration guidance |
| 10 | Eval & quality flywheel | eval harness (`eval_context.rs`, gold trajectories, denial capture), `policy_corpus.tsv` coverage vs current bypass classes, CI + coverage job, evolve-campaign residue (champion configs, paired-guard protocol) |
| 11 | Process (SDLC meaning 2) | the repo's own spec→plan→implement flow, review/HITL gates, verification-first practice, ledger/memory discipline — judged against `docs/okf/agent-sdlc/practices/` (spec-first, eval-driven development, HITL gates) |

Dimensions 1–7 use the checklists in `audit.md`; dimensions 8–11 derive their
checklists from the relevant bundle `practices/` pages plus runtime conventions
(the `audit.md` "thinly-sourced — judge locally" clause applies).

## Execution — multi-agent workflow (owner opted in)

One workflow run, two pipelined stages:

1. **Audit stage — 11 auditor agents in parallel.** Each auditor receives: its
   dimension's checklist, the anchor list, the relevant bundle concept paths,
   the prior audit's closed state (so it doesn't re-report), and a structured
   findings schema. Standing instructions: orient via `graphify-out/` if useful
   but **re-read live source before emitting any finding** (the graph may be
   stale); severities per the `audit.md` rubric; `EXTRACTED`-style discipline —
   no finding without a live file:line.
2. **Verify stage — one adversarial verifier per finding**, pipelined (a
   dimension's findings verify as soon as its auditor returns; no barrier).
   Verifier posture is default-refute: re-open the file, confirm the claim
   reproduces today at the cited line, sanity-check severity against the
   rubric. Refuted findings are dropped and logged; confirmed findings enter
   the report, with severity corrected if the verifier disagrees.

Estimated fleet: 11 auditors + ~25–40 verifiers (scales with findings volume).

## Deliverables

1. **Report:** `docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md` —
   executive summary, per-dimension findings (verified only), ranked **Top-10
   highest-leverage fixes** (component + file:line + one-line fix), a
   **prior-state regressions** section (anything closed in July that re-opened;
   expected empty), and a dropped-findings appendix (what verification refuted,
   one line each — auditability of the verify gate).
2. **Re-stamp:** a dated pointer note in `audit.md`'s example-findings section
   marking this report as the current findings snapshot (the playbook itself
   prescribes this on re-run; it is the playbook's own ledger, not code).

## Error handling

- An auditor that returns nothing (skip/terminal error) → its dimension is
  marked "NOT AUDITED" in the report, never silently omitted.
- A verifier that returns nothing → the finding enters the report flagged
  "unverified" rather than being dropped or silently promoted.
- Findings with stale line numbers at verify time are corrected, not dropped,
  when the underlying claim still holds.

## Testing / success criteria

- Every reported finding carries all four schema fields and survived (or is
  flagged as skipping) adversarial verification.
- Every dimension appears in the report with an explicit status: findings,
  clean, or NOT AUDITED.
- `scripts/okf_check.py` and `bash scripts/ci.sh` results are recorded in the
  report as ground truth for dimensions 9–10 (read-only runs; running the
  existing gates is observation, not code change).
- The report names zero edits to production code (REPORT ONLY held).

## Out of scope

- Fixing anything the audit finds (separate spec→plan→implement cycles, per
  finding cluster, after the owner triages the report).
- Re-running eval campaigns (context-evolve / harness-evolve) — dimension 10
  audits their artifacts and wiring, not their scores.
- The `web/` SPA's product UX beyond harness-relevant surfaces (URL guard,
  event rendering fidelity, attribution display).
