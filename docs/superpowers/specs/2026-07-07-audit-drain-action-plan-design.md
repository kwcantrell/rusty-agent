# 2026-07-06 Audit Drain — Action Plan (Triage Spec)

**Date:** 2026-07-07
**Input:** `docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md` (41 confirmed findings:
18 med, 23 low, 0 high; commit `04ae150`).
**Role of this doc:** the owner-adjudicated triage for the whole backlog. Each cluster below is
executed as its own branch/cycle; this spec is the single source for what ships, in what
grouping, and in what order. Per-cluster design questions (where any remain) get short
per-cluster specs; mechanical clusters go straight to a plan.
**Precedent:** the 2026-07-02 backlog drain (cluster-based, ledgered, re-stamped) — this round
repeats that shape.

## Decisions (owner-adjudicated 2026-07-07)

- **Scope:** full drain — all 41 findings, no parked remainder.
- **Structure:** boundary-based clusters (grouped by shared code seam, not audit dimension).
- **Verdicts:** **ship all 39 distinct fixes; zero declines.** Two dedups: finding 2.3 ≡ 4.5
  (one dispatch-description fix), finding 9.2 ≡ 10.1 (one okf_check CI leg).
- **In-fix decisions (flagged and resolved):**
  - 5.2 → **warn** (not hard-reject) when `command_allowlist` gains a shell interpreter.
  - 11.4 → **record the docs-on-main exception** in CLAUDE.md (with its compensating control)
    rather than requiring branches for docs-only campaigns.
  - 3.5 → **ship** the single-shared-sandbox refactor (LoopParts accepts the Arc).
  - 9.3 → **full** okf_check extension list (resource: on Sources, type vocabulary,
    [n]-marker resolution, index coverage), riding the same branch that gates the checker.
- **No collision with the six DECLINED-BY-OWNER items** from the July product-decision round:
  2.2 stays prose-only (does not re-open git consolidation); 3.5 touches sandbox plumbing,
  not the declined persisted-OffloadStore/live-trace items.

## Cluster map (execution order)

Finding IDs refer to the audit report's numbering. Line numbers there drift — every plan
re-opens live source before acting.

### Cluster 1 — CI & gates *(first: later clusters run under the strengthened gate)*

| Finding | Sev | Fix |
|---|---|---|
| 11.1 | med | Conditional src-tauri leg in `scripts/ci.sh`: build + clippy -D warnings + test, skipped when GTK/WebKitGTK dev deps absent (GitHub-runner rationale preserved). fmt stays excluded (documented hand-format convention). |
| 9.2 + 10.1 | med | `python3 scripts/test_okf_check.py && python3 scripts/okf_check.py docs/okf/agent-sdlc` leg in ci.sh. |
| 10.2 | low | Coverage job writes the llvm-cov summary to `$GITHUB_STEP_SUMMARY` (still `continue-on-error`, never a gate). |
| 9.3 | low | Extend okf_check.py: require `resource:` on `type: Source` nodes; validate `type` against the authoring.md vocabulary; resolve body `[n]` markers against Citations; check each directory index.md lists every node. Document that semantic-claim drift stays a human duty. |

Branch: `feature/audit-ci-gates`. Straight to plan (mechanical).

### Cluster 2 — MCP seam *(three audit dimensions converged here)*

| Finding | Sev | Fix |
|---|---|---|
| 2.1 | med | Lint MCP schemas at connect (`connect_one`/`McpManager::connect`): run `required_params_missing_description` + description emptiness/length checks per RawTool; surface via `tracing::warn` and a `ServerStatus` field. Warn-don't-reject, matching duplicate-name posture. |
| 5.1 | med | Decouple MCP auto-allow from the declared Access tier so `Trust::Allow` mutations still trip the post-exec validator (e.g. intent-level `mutating: true`, or a tier RulePolicy auto-allows but the validator trigger counts as Write). |
| 3.1 | med | `docker_run_args` emits name-only `-e KEY` with the value set on the docker client process env (or a 0600 `--env-file`); stops broadcasting MCP secrets in world-readable argv. |

Branch: `feature/audit-mcp-seam`. Gets a short per-cluster spec first — 5.1's encoding choice
has real design surface (new tier vs. intent flag) and 3.1 must keep `HOME=/tmp` behavior.

### Cluster 3 — Design-tab hardening *(src-tauri; after Cluster 1 so the new leg gates it)*

| Finding | Sev | Fix |
|---|---|---|
| 8.1 | med | `DevServerManager::start`: canonicalize `cand.dir` and require workspace containment before spawning; reject otherwise. (The spec's Security section already promises this.) |
| 8.3 | med | Add `frame-src 'self' http://localhost:* http://127.0.0.1:*` to the desktop CSP; verify the auto-dev-server canvas renders under the production (non-dev) CSP. |
| 8.4 | low | Anchor the dev-server URL host check (exact `localhost`/`127.0.0.1` up to port/path boundary, or reuse `validate_local_url`), restoring two-layer parity. |
| 8.5 | low | Restrict Image artifact `src` to `data:` URIs or validate the host — the browser SPA path has no CSP fallback. |
| 8.2 | low | PR_SET_PDEATHSIG via `pre_exec` so the dev server dies with a SIGKILLed/aborted app. |
| 8.6 | low | SIGTERM, bounded grace wait, then SIGKILL only if still alive; fix the "backstop" doc comment. |

Branch: `feature/audit-design-tab-hardening`. Straight to plan. Gotchas from memory apply:
src-tauri is its own workspace, compact hand-format (no cargo fmt).

### Cluster 4 — Sub-agent composition *(largest cluster; the seams between July sub-specs)*

| Finding | Sev | Fix |
|---|---|---|
| 4.1 | med | Thread `tool_description_overrides` into dispatch: `DispatchDeps.description_overrides` filled from cfg in assemble.rs, cloned into nested deps; `reg.set_description_overrides(...)` on child registries. Pin: child `schemas()` shows the override. Restores the seam spec's uniformity claim; unblocks description-variant eval candidates. |
| 4.2 | med | Optional `context_limit` (and `max_tokens`) on `ModelRef`, inherit-on-None; apply to `child_config.model_limit` where `max_turns` is already overridden; same for a routed compaction model's MaintCtx limit. |
| 4.3 | med | Keep `BUDGET_WRAP_UP_PROMPT` out of durable history (replace/drop after the wrap-up completion, or self-expiring wording). Pin: two-run test — next run's build contains no tools-disabled instruction. |
| 4.4 | med | Timeout and `Ok(Err(e))` dispatch arms return the child's partial transcript from `sink.summary()` with a loud prefix, mirroring the budget-exhaustion posture. Pin: timed-out child's captured text reaches the parent. |
| 2.3 + 4.5 | low | Depth-aware `dispatch_agent` description (DispatchDeps carries depth/max_depth): "minus dispatch_agent itself" only when `depth >= max_depth`. |

Branch: `feature/audit-subagent-composition`. Short per-cluster spec first — 4.3's removal
mechanics (mutating persistent context safely) and 4.2's inheritance semantics deserve a page.

### Cluster 5 — Context, trace & config guards

| Finding | Sev | Fix |
|---|---|---|
| 7.1 | med | Cap the pinned goal block at a token budget in `set_goal` (mirror `DEFAULT_RECALL_TOKEN_BUDGET`; first N tokens + ellipsis marker; full input stays in history). Pin: over-window first paste does not permanently wedge the session. |
| 7.2 | med | `max_tool_result_bytes` floor in `RuntimeConfig::validate()` (reject 0 or clamp, mirroring the max_parallel_tools zero-guard); optional warn when the cap's est. tokens exceed a fraction of context_limit. |
| 6.1 | med | Record run inputs in the trace: run-start event with user input (None-mapped in `server_event_from` for old-SPA wire compat) + composed system prompt hash/override set once per run. |
| 6.2 | low | Terminal `trace_disabled` marker record (reason cap\|io_error) before `inner.w = None`, with reserved headroom so it always fits. |
| 7.3 | low | Snapshot gains the folded-facts ledger tokens (ledger segment, or fold into the goal segment to match `pinned()`), so est_total matches injection. |
| 3.3 | low | CLI validates sandbox_mode: `rt.validate()` in agent-cli main (exit 2) or clap `value_parser(["off","auto","enforce"])`. |
| 5.2 | low | `validate()` **warns** when `command_allowlist` gains a shell interpreter / exec vehicle as leading token (bash/sh/zsh/dash/ksh/eval/xargs/env) — mechanizes the command.rs KNOWN LIMITATION prose. |
| 5.3 | low | `policy_corpus.tsv` gains `ask` rows for the documented /dev residuals: `tee /dev/sda`, `cp x /dev/sda`, `$DEV` expansion, cwd-relative redirect. |

Branch: `feature/audit-context-config-guards`. Short per-cluster spec first — 6.1 adds a trace
record type (wire-compat surface) and 7.1 touches the champion-v4 context spine (re-run the
context-evolve guard sweep after; ceilings are validated as (config, rate) pairs).

### Cluster 6 — Prose, skills & ledger sweep *(one branch of small one-file edits)*

| Finding | Sev | Fix |
|---|---|---|
| 1.1 | med | House-style **Do not** block in `.agents/skills/agent-sdlc/SKILL.md`; narrow the frontmatter overlap with harness-engineering. |
| 2.2 | med | `when_not_to_call()` on ExecuteCommand steering to read_file/list_directory/git_*; add execute_command to `CONFUSABLE_TOOLS`. Prose only — git consolidation stays declined. |
| 9.1 | med | Rewrite auto-drive-tauri's L0/L1-hybrid paragraph to reference a live test (bridge.rs died in `474b7af`). |
| 2.4 | low | Recurse `required_params_missing_description` into array-items object schemas; describe render's columns/rows with their `(kind=table)` requirement. |
| 2.5 | low | Drop the duplicated "Args: …" sentences from remember/recall/forget base descriptions. |
| 3.2 | low | Fix the `build_sandbox` doc comment: auto mode is fail-closed, not degrade-to-host. |
| 3.4 | low | Parity test: default-parsed CLI sandbox fields == runtime-config `default_*` fns (or derive clap defaults from them). |
| 3.5 | low | Build the sandbox once in agent-cli main; `loop_config_from` accepts the Arc instead of calling `build_sandbox` again. |
| 9.4 | low | Trim the four longest MEMORY.md index lines to one-sentence pointers (detail already lives in the topic files). Keep one-clause guardrails like "don't re-propose" in the index. |
| 10.3 | low | Reword the stale `favorable_disables_curation` comment (ingestion cap IS genome axis 8). |
| 11.2 | low | Append the MERGED close-out line to the auto-dev-server-canvas ledger section (`dfec8b7`). |
| 11.3 | low | Rename the five completed non-archived ledgers to `*.archive.md` with a one-line merged stamp. |
| 11.4 | low | **Record the docs-on-main exception** (and its compensating control: whole-campaign review before any push) in CLAUDE.md "How we work". |

Branch: `feature/audit-polish-sweep`. Straight to plan. 9.4 edits live outside the repo
(Claude memory dir) — do it in the same cycle but it is not part of the branch/commits.

## Process (per cluster)

1. Branch off `main` (worktree per superpowers convention).
2. Per-cluster spec only where flagged (Clusters 2, 4, 5); otherwise straight to
   writing-plans.
3. Implement with TDD; every fix lands with the regression test its verifier entry implies.
4. Policy-touching fixes (5.1, 5.2, 5.3) extend `policy_corpus.tsv` / the corpus runner, not
   ad-hoc tests.
5. `bash scripts/ci.sh` green before merge; Cluster 3 also runs the src-tauri leg Cluster 1
   introduces. Per-task subagent review + whole-branch review + `--no-ff` merge, as always.
6. Bookkeeping per cluster: ledger section in `.superpowers/sdd/progress.md` (opened at start,
   MERGED stamp at close), then dated re-stamps for its findings in
   `.agents/skills/harness-engineering/audit.md`.

**Close-out (after all six):** final audit.md re-stamp; memory file
`harness-sdlc-audit-2026-07.md` flips TRIAGE PENDING → DRAINED/CLOSED with per-cluster merge
commits; MEMORY.md index line updated (within its 9.4-trimmed budget).

## Ordering rationale & sizing

CI first (cheap; gates the rest — notably the src-tauri leg covers Cluster 3's code), then the
two security-adjacent clusters (MCP seam, design-tab) land in the first half, then sub-agent
composition (largest: four med fixes in dispatch/loop internals), context/config guards, and
the sweep last. Rough effort: C1 small · C6 small-but-wide · C2, C3, C5 medium · C4 largest.
A mid-drain pause after C3 still leaves all security-relevant items shipped.

## Out of scope

- Re-opening any of the six DECLINED-BY-OWNER July items (git consolidation, persisted
  OffloadStore, catalog inlining, live-trace toggle, lexical/FS split, sub-agent extras).
- The three refuted findings (Appendix A of the audit) — no action.
- New audit dimensions or fresh auditing; this round only drains the 2026-07-06 report.
