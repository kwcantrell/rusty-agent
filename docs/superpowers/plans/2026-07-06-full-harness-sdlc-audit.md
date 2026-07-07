# Full Harness + SDLC Audit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Exception:** Task 3 (the Workflow invocation) MUST run in the main session — the Workflow tool cannot be nested inside a dispatched subagent. Inline execution is the natural fit for this plan.

**Goal:** Produce a verified, REPORT-ONLY 11-dimension harness+SDLC audit at `docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md`, per `docs/superpowers/specs/2026-07-06-full-harness-sdlc-audit-design.md`.

**Architecture:** Two read-only ground-truth gate runs feed an 11-auditor / per-finding-adversarial-verifier Workflow (pipelined, no barrier); the orchestrator synthesizes the structured results into the report and re-stamps `audit.md`. No production code changes anywhere.

**Tech Stack:** Claude Code Workflow tool (owner opted in during brainstorming), `harness-engineering` skill playbooks, `agent-sdlc` OKF bundle as citation layer.

## Global Constraints

- **REPORT ONLY.** No task edits production code. The only files created/modified: the report, the `audit.md` re-stamp note, scratchpad artifacts.
- Every reported finding carries all four schema fields (`severity`, `file:line`, `violated principle + source`, `concrete fix`) and a verification verdict (or an explicit `unverified` flag).
- Every dimension appears in the report with status `findings` | `clean` | `NOT AUDITED` — never silently omitted.
- Declined 2026-07-02 product decisions MAY be re-proposed on fresh evidence; note the prior decision for context.
- Prior July-closed findings are re-verified, not re-reported; regressions go to a dedicated report section.
- Scratchpad: `/tmp/claude-1000/-home-kalen-rust-agent-runtime/744cbc2e-418b-4a5c-af90-effff4811528/scratchpad` (referred to as `$SCRATCH` below).

---

### Task 1: Ground-truth gate runs (read-only)

**Files:**
- Create: `$SCRATCH/audit-gates.md` (scratch artifact, feeds dimensions 9–10 and the report)

**Interfaces:**
- Produces: `$SCRATCH/audit-gates.md` with two fenced sections, `## okf_check` and `## ci.sh`, each containing exit status + tail of output. Task 4 embeds these verbatim in the report's "Ground truth" section.

- [ ] **Step 1: Run the OKF conformance checker**

```bash
cd /home/kalen/rust-agent-runtime && python3 scripts/okf_check.py docs/okf/agent-sdlc; echo "exit=$?"
```

Expected: conformance clean (it was clean as of 2026-07-06). Record full output.

- [ ] **Step 2: Run the CI gate**

```bash
cd /home/kalen/rust-agent-runtime && source ~/.cargo/env && bash scripts/ci.sh; echo "exit=$?"
```

Expected: PASS (fmt + clippy + cargo test in `agent/` + web typecheck/vitest). This takes minutes — run in background and collect. Record exit status + the summary tail (last ~30 lines). If it fails, that is itself audit evidence — record it, do not fix anything.

- [ ] **Step 3: Write both results to `$SCRATCH/audit-gates.md`**

```markdown
# Ground-truth gate runs — 2026-07-06

## okf_check
exit=0
<output>

## ci.sh
exit=0
<summary tail>
```

No commit (scratch artifact).

---

### Task 2: Author the audit workflow script

**Files:**
- Create: `$SCRATCH/audit-workflow.mjs` (passed to Workflow via `scriptPath`; scratch, not committed)

**Interfaces:**
- Produces: a Workflow script whose return value is an array of 11 objects `{dim, title, status, prior_state, findings[]}` where each finding is `{severity, file_line, principle, source, fix, evidence, prior_decision?, verdict: {verdict, severity, file_line, reason} | null}`. Task 4 consumes this exact shape.

- [ ] **Step 1: Write the complete script below to `$SCRATCH/audit-workflow.mjs`**

```javascript
export const meta = {
  name: 'harness-sdlc-audit',
  description: 'REPORT-ONLY 11-dimension harness+SDLC audit with per-finding adversarial verification',
  phases: [
    { title: 'Audit', detail: 'one auditor per dimension, live-source findings' },
    { title: 'Verify', detail: 'one default-refute verifier per finding' },
  ],
}

const FINDINGS_SCHEMA = {
  type: 'object',
  required: ['status', 'prior_state', 'findings'],
  properties: {
    status: { type: 'string', enum: ['findings', 'clean'] },
    prior_state: {
      type: 'string',
      description: 'One paragraph: are the previously-closed July fixes touching this dimension still in place? Name any regression explicitly (a regression is ALSO a finding).',
    },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        required: ['severity', 'file_line', 'principle', 'source', 'fix', 'evidence'],
        properties: {
          severity: { type: 'string', enum: ['high', 'med', 'low'] },
          file_line: { type: 'string', description: 'repo-relative path:line or path:start-end, read live this session' },
          principle: { type: 'string', description: 'the violated principle, one sentence' },
          source: { type: 'string', description: 'a docs/okf/agent-sdlc/... path when a concept covers it; else a tier-tagged external source; else "first principles + runtime conventions"' },
          fix: { type: 'string', description: 'one concrete action the implementer should take' },
          evidence: { type: 'string', description: 'short live code/doc excerpt demonstrating the gap' },
          prior_decision: { type: 'string', description: 'ONLY if this re-proposes a 2026-07-02 declined product decision: name which one' },
        },
      },
    },
  },
}

const VERDICT_SCHEMA = {
  type: 'object',
  required: ['verdict', 'severity', 'file_line', 'reason'],
  properties: {
    verdict: { type: 'string', enum: ['confirmed', 'refuted'] },
    severity: { type: 'string', enum: ['high', 'med', 'low'], description: 'your independent severity call per the audit.md rubric' },
    file_line: { type: 'string', description: 'the corrected file:line if the cited one drifted; else repeat it' },
    reason: { type: 'string', description: 'one or two sentences: why confirmed or refuted' },
  },
}

const PREAMBLE = `You are one auditor in an 11-dimension REPORT-ONLY audit of the harness in
/home/kalen/rust-agent-runtime. You NEVER edit any file — you read and report. Method:

1. Read .agents/skills/harness-engineering/SKILL.md (Spine A + B, severity framing), then the
   checklist named in your brief below — either a numbered section of
   .agents/skills/harness-engineering/audit.md, or the derived checklist in the brief itself.
   Apply audit.md's severity rubric (high = correctness/safety, med = leverage/efficiency,
   low = polish) and its "thinly-sourced — judge locally" clause where the brief says so.
2. Citation discipline: read docs/okf/agent-sdlc/index.md and docs/okf/agent-sdlc/practices/index.md;
   for each finding, fill 'source' with the bundle concept path (docs/okf/agent-sdlc/...) that backs
   the violated principle when one exists. NOTE: links INSIDE bundle pages are bundle-root absolute
   (/practices/foo.md means docs/okf/agent-sdlc/practices/foo.md). If no concept covers it, tier-tag
   an external source; if judging locally, write "first principles + runtime conventions".
3. Prior state: the 2026-07-01 deep audit is fully closed. Skim
   docs/superpowers/audits/2026-07-01-harness-deep-audit.md (exec summary + your component's section)
   and the dated "Re-stamp note" paragraphs in audit.md. Do NOT re-report anything marked fixed —
   instead spot-check that the fixes touching your dimension are still in live source and summarize
   in 'prior_state'. A regression goes in 'prior_state' AND as a finding.
4. Evidence: you may orient via graphify-out/GRAPH_REPORT.md, but the graph can be stale — every
   finding must come from source you read live THIS session, with a real file:line and a short
   excerpt in 'evidence'. No finding without live evidence.
5. Accepted residuals documented in specs/re-stamp notes are not findings unless you can show the
   acceptance rationale no longer holds. Product decisions DECLINED on 2026-07-02 (git-tool
   consolidation, persisted OffloadStore, list_skills catalog inlining, live trace toggle,
   lexical/FS split of the read-boundary hook, sub-agent extras) MAY be re-proposed given fresh
   evidence — name the prior decision in 'prior_decision'.
6. Emptiness is a valid result: status 'clean' with an evidenced prior_state beats invented findings.

Your final answer is ONLY the StructuredOutput call. Your dimension brief:

`

const DIMENSIONS = [
  {
    key: 'instructions', title: 'Instructions & rule files',
    brief: `Checklist: audit.md section "1. Instructions & Rule Files".
Anchors: agent/crates/agent-runtime-config/src/prompts.rs (BASE_SYSTEM_PROMPT + the re-duplication
ratchet test), CLAUDE.md, every .agents/skills/*/SKILL.md frontmatter description + "Do not" block,
the dispatch role-block injection in agent/crates/agent-core/src/dispatch.rs (role arg -> child
system prompt). Check: single source of truth held since bc8934e; negative constraints present and
current; no contradictions between CLAUDE.md, skills, and the runtime prompt; role blocks bounded
and versioned.`,
  },
  {
    key: 'tools', title: 'Tools',
    brief: `Checklist: audit.md section "2. Tools".
Anchors: agent/crates/agent-tools/src/tool.rs + types.rs (when_not_to_call contract, Access tiers in
schemas), agent/crates/agent-core/src/context_tools.rs, the dispatch_agent tool schema in
agent-core/src/dispatch.rs, memory tools (agent-memory), http fetch tool (agent-http), mcp tool
bridging (agent-mcp), skills tools (agent-skills). Check: every registered tool's description/
when_not_to_call is tight and current (several tools gained args since the contract landed);
duplicate-name warn behavior; no thin wrappers a consolidated tool covers; required-param
descriptions non-empty (including tools added after the enforcement ratchet).`,
  },
  {
    key: 'sandbox', title: 'Sandboxes & execution',
    brief: `Checklist: audit.md section "3. Sandboxes & execution environments".
Anchors: the sandbox strategy + fail-closed refusal + self-healing re-probe in agent/crates/agent-tools/,
HostExecutor env_clear + six-var allow-list, sandbox-image/ (Dockerfile, noexec /tmp, smoke.sh),
the agent-sandbox-dev default image + tri-state image probe (merged e60710e), MCP server spawn
(cwd = workspace, skip-under-refusal), CLI clap defaults vs runtime-config defaults (known shadowing
gotcha class — check sandbox knobs specifically), agent/crates/agent-server/src/runtime.rs sandbox
wiring. Check: isolation still fail-closed on every exec path added since July 1 (dispatch children,
dev-server?— no, that is dimension 8); capabilities explicit; egress gated per-tool.`,
  },
  {
    key: 'orchestration', title: 'Orchestration & sub-agents',
    brief: `Checklist: audit.md section "4. Orchestration logic", PLUS the sub-agent capability as a
finished whole (built after the last audit ran its component 4: merges af4dd14, 0224383, d19625b).
Anchors: agent/crates/agent-core/src/loop_.rs (parallel dispatch + isolation, ErrorClass retry,
overflow compact-and-rebuild, stuck-model nudge/abort, graceful max_turns wrap-up),
agent/crates/agent-core/src/dispatch.rs (depth/subagent_max_depth, tools allowlist transitive
narrowing, ModelRef routing incl. compaction_model, cancellation propagation, privilege inheritance
— child holds parent's exact policy/approval/sandbox Arcs), agent/crates/agent-model/src/protocol.rs.
Parallel tool calls + config limits are thinly-sourced: judge from first principles + runtime
conventions. Check the seams BETWEEN the July clusters (e.g. wrap-up completion x dispatch x
retry x stream-retry interactions) — each shipped reviewed alone, the composition did not.`,
  },
  {
    key: 'guardrails', title: 'Guardrails & policy',
    brief: `Checklist: audit.md section "5. Guardrails / Hooks".
Anchors: agent/crates/agent-policy/src/engine.rs + command.rs (hard floor, position-aware Layer A2,
resolved_dev_suffix + both /dev-redirect layers), Destroy tier enforcement (both guard sites),
prefix allowlist + git --output arg-scan, the post-exec validator (decision-round cluster D, merge
5f41db5 — new since audit.md's checklist was written: audit its hook placement, side-effect-freedom,
and bypass surface from first principles), agent/crates/agent-runtime-config/src/lib.rs denylists,
approval channel (timeout, cancellation race fix, sub-agent prompt serialization),
agent-runtime-config/tests/policy_corpus.{rs,tsv} (is the corpus current vs the newest bypass
classes?). Check: deterministic pre-exec order (policy -> approval -> execute) holds on every path
including dispatch children and the post-exec validator's addition.`,
  },
  {
    key: 'observability', title: 'Observability',
    brief: `Checklist: audit.md section "6. Observability".
Anchors: TraceWriter (JSONL sessions, 0600, 64MB cap/keep-50, record_child interleaving),
SessionStats (subagent subset counters, turns=max semantics), sub-agent attribution chain
(ToolCtx.call_id -> parent_id across event/wire/trace, id-first web correlation), ServerUsage/cost
metering (claude-cli total_cost_usd, cache-token folding; mixed-backend conflation residual),
ContextEvents (Evicted dedup, OverflowRecovery, StreamRetry), agent/crates/agent-server/src/runtime.rs
EventSink. Check: can a failed turn — including a failed CHILD turn at depth 2 — be replayed from
the trace alone? Is anything emitted to the UI but absent from the trace, or vice versa?`,
  },
  {
    key: 'context', title: 'Context engineering (Spine B)',
    brief: `Checklist: the Spine B anchors row of audit.md's anchor table + SKILL.md Spine B.
Anchors: agent/crates/agent-core/src/context.rs + curated.rs (turn-unit helpers, turn-atomic
eviction/compaction, keep-floor), ingestion cap (max_result_bytes eager offload, recall paging),
calibrated budgeting (calib_ratio_micros EMA, effective_model_limit at the four sites), the Examples
context type (skills examples/ convention, L1/L2 surfacing), offload.rs + offload_policy.rs,
memory recall block (unconditional set_recall). Static/dynamic boundary review: what loads every
session vs on-demand; the compose-time quarter-window warn. Check interactions: calibration x
eviction x eager offload x overflow recovery composed correctness; offload store growth (RAM-only)
under the new eager path.`,
  },
  {
    key: 'design-tab', title: 'Desktop/web design-tab harness',
    brief: `No audit.md section exists — derived checklist; judge from first principles + runtime
conventions + bundle concepts on sandboxing/guardrails where they apply.
Anchors: src-tauri/src/ (dev-server manager: detect/rank scripts, single-process invariant,
orphan-free teardown / process-group death, both-pipes-drained-to-EOF, ANSI URL parse, server-side
package-manager/script whitelist), the two-layer localhost URL guard (web/src urlGuard — JS
authoritative, userinfo/@-authority bypass closed — and the Rust coarse layer in agent-tools'
kind=url render), the canvas/feedback/pin surfaces (web/src), Tauri command boundary (which
commands are exposed, input validation, capability scoping in src-tauri config), WebDriver e2e
harness (src-tauri/tests gui_smoke) as the verification layer.
Checklist:
- [ ] Dev-server spawn is gated server-side (whitelist enforced in Rust, not only in the SPA); no
      argument smuggling through script names or package-manager choice.
- [ ] Teardown kills the whole process group on every exit path (tab close, app quit, restart,
      panic); no orphan or zombie path.
- [ ] The localhost guard's two layers agree (guard parity); no URL form reaches the iframe that
      the JS layer would reject; non-localhost egress impossible from the canvas.
- [ ] Tauri command surface is minimal and validated; no command trusts SPA-provided paths/URLs
      without server-side checks.
- [ ] src-tauri is absent from scripts/ci.sh — what actually gates it (fmt/clippy/test/gui_smoke),
      and is that gate documented and runnable?`,
  },
  {
    key: 'skills-knowledge', title: 'Skills & knowledge layer',
    brief: `No audit.md section exists — derived checklist; cite bundle concepts on progressive
disclosure / context engineering where they apply.
Anchors: .agents/skills/ (all 10 Claude-facing skills), the runtime skill registry
agent/crates/agent-skills/src/registry.rs (loads <workspace>/.agent/skills + ~/.agent/skills — the
two-trees gotcha), docs/okf/agent-sdlc/ (indexes, link convention, citations), scripts/okf_check.py
(conformance result comes from the Task-1 gate run — do not re-run; read the checker source to
judge what it does NOT check), graphify-out/ + CLAUDE.md's graphify-first guidance,
.agents/skills/agent-sdlc/authoring.md.
Checklist:
- [ ] Skill frontmatter descriptions route unambiguously — no two skills claim overlapping triggers
      a model would confuse (check especially harness-engineering vs agent-sdlc vs
      graphify-best-practices).
- [ ] No skill references files/flags/paths that no longer exist (staleness sweep).
- [ ] The two skill trees are impossible to conflate given current docs (CLAUDE.md gotcha +
      create_skill authoring path).
- [ ] OKF bundle: conformance clean (Task-1), intra-bundle links follow the bundle-root-absolute
      convention, sources/ nodes carry resource: URLs; what does okf_check.py structurally miss
      (e.g. semantic claim drift)?
- [ ] Progressive-disclosure hygiene: skill bodies load on demand; nothing bulky leaked into
      always-loaded surfaces (CLAUDE.md size, MEMORY.md index one-line rule).`,
  },
  {
    key: 'eval-flywheel', title: 'Eval & quality flywheel',
    brief: `No audit.md numbered section — derive from the prior audit's Eval dimension + bundle
practices (eval-driven development). Conformance vs the runtime's own eval conventions counts as
"first principles + runtime conventions".
Anchors: agent/crates/agent-core (eval_context.rs / eval harness: RunResult trajectory/denials/
gold_matched, TaskSpec.gold_trajectory, sealed oracles, model pinning),
agent-runtime-config/tests/policy_corpus.{rs,tsv} (86+ rows — coverage vs bypass classes closed
SINCE it landed: /dev-redirect waves, post-exec validator), scripts/ci.sh + .githooks/pre-push +
.github/workflows/ (incl. the continue-on-error llvm-cov job — has it produced a real run?),
.agents/skills/context-evolve + harness-evolve (paired-guard protocol, champion configs pinned,
"(config, rate) pair" rule), the ci.sh result from Task-1.
Checklist:
- [ ] The eval harness's neutralized knobs (ingestion cap pinned off, unbounded L2 listings) are
      documented and still intentional; nothing new silently depends on them.
- [ ] policy_corpus rows exist for every bypass class closed since the corpus landed.
- [ ] CI actually gates what the repo believes it gates (compare ci.sh contents vs CLAUDE.md claims
      vs Actions workflows; src-tauri exclusion is dimension 8's item — here check agent/ + web/).
- [ ] Gold-trajectory support has at least one real consumer or a dated plan (diagnostic-only was
      the landing posture — is that still the recorded intent?).
- [ ] Eval results land somewhere actionable (files/ledgers), not only in past session transcripts.`,
  },
  {
    key: 'process', title: 'Process — the SDLC as run by agents (meaning 2)',
    brief: `No audit.md section — judge the repo's own development process against the bundle's
meaning-2 practices. Read docs/okf/agent-sdlc/comparisons/two-meanings.md FIRST, then the
practices/ index and the spec-first / human-in-the-loop / verification-first concept pages; cite
them as sources.
Evidence base: docs/superpowers/specs/ + plans/ + audits/ (density and dating), git log since
2026-06-25 (sample 5+ merged feature branches: did each carry spec -> plan -> implementation ->
review evidence? conventional commits held?), .superpowers/sdd/progress.md ledgers, CLAUDE.md's
"How we work" contract, the memory-index discipline visible in
docs/superpowers/audits/2026-07-01-harness-deep-audit.md re-stamp trail.
Checklist:
- [ ] Spec-first holds for design-bearing changes (sample the July merges; a fix-wave commit
      without a spec is fine, a feature without one is a finding).
- [ ] HITL gates are real: owner adjudication points are recorded (product-decision round), reviews
      happen before merge (whole-branch review notes), REPORT-ONLY boundaries respected.
- [ ] Verification-first: changes ship with tests; ci gate is enforced (hooksPath configured?
      check .githooks wiring is opt-in-per-clone and whether that is documented honestly).
- [ ] Accepted-residual bookkeeping: residuals are dated, findable, and re-surfaced (the re-stamp
      trail) rather than lost.
- [ ] Anti-pattern scan: process theater (specs written after code), stale ledgers, or unclosed
      loops the bundle's practices warn about.`,
  },
]

function auditPrompt(d) {
  return PREAMBLE + `Dimension ${d.title} (key: ${d.key}).\n\n` + d.brief
}

function verifyPrompt(d, f) {
  return `You are an adversarial verifier in a REPORT-ONLY audit of /home/kalen/rust-agent-runtime.
You NEVER edit files. Your default posture is REFUTE — a finding survives only if the evidence
holds up when you re-derive it yourself. Finding (dimension "${d.title}"):

severity: ${f.severity}
file:line: ${f.file_line}
principle: ${f.principle}
source: ${f.source}
proposed fix: ${f.fix}
evidence: ${f.evidence}

Procedure:
1. Open ${f.file_line.split(':')[0]} live and locate the cited code/doc. If the line drifted but the
   claim holds, correct file_line and continue — drift alone is not refutation.
2. Re-derive the claim: does the gap actually exist in today's source? Check for a fix, guard, test,
   or documented accepted-residual the auditor missed (search specs/ and audit.md re-stamp notes for
   the topic). An accepted residual with a still-valid rationale = refuted.
3. Judge severity independently against audit.md's rubric (high = correctness/safety,
   med = leverage/efficiency, low = polish). Disagreement with the auditor is fine.
4. If the finding is unfalsifiable, vague, or its fix is not one concrete action -> refuted, say why.
Return ONLY the StructuredOutput call.`
}

log('11 auditors fanning out; each dimension verifies as soon as its auditor returns')

const results = await pipeline(
  DIMENSIONS,
  d => agent(auditPrompt(d), { label: 'audit:' + d.key, phase: 'Audit', schema: FINDINGS_SCHEMA }),
  async (res, d) => {
    if (!res) return { dim: d.key, title: d.title, status: 'NOT_AUDITED', prior_state: null, findings: [] }
    const verified = await parallel(res.findings.map(f => () =>
      agent(verifyPrompt(d, f), { label: 'verify:' + d.key + ':' + f.file_line, phase: 'Verify', schema: VERDICT_SCHEMA })))
    log(d.key + ': ' + res.findings.length + ' findings, ' +
        verified.filter(v => v && v.verdict === 'confirmed').length + ' confirmed')
    return {
      dim: d.key, title: d.title, status: res.status, prior_state: res.prior_state,
      findings: res.findings.map((f, i) => ({ ...f, verdict: verified[i] ?? null })),
    }
  },
)

return results
```

- [ ] **Step 2: Sanity-check the script parses**

Re-read the file once; confirm: `meta` is a pure literal, no TypeScript annotations, no `Date.now()`/`Math.random()`, `pipeline` stage 2 handles `res === null` (NOT_AUDITED) and `verified[i] === null` (unverified flag), and phase labels match `meta.phases` titles exactly (`Audit`, `Verify`).

No commit (scratch artifact).

---

### Task 3: Run the workflow (MAIN SESSION ONLY)

**Files:**
- Create: `$SCRATCH/audit-results.json` (the workflow's return value, saved verbatim)

**Interfaces:**
- Consumes: `$SCRATCH/audit-workflow.mjs` from Task 2.
- Produces: `$SCRATCH/audit-results.json` — the 11-element array shape defined in Task 2's Interfaces block. Task 4 reads it.

- [ ] **Step 1: Invoke the Workflow tool**

Call `Workflow` with `{scriptPath: "$SCRATCH/audit-workflow.mjs"}` (owner opted into multi-agent orchestration during brainstorming — recorded in the spec's Execution section). Note the returned `runId` for resume.

- [ ] **Step 2: Monitor to completion**

The workflow runs in background; completion arrives as a task notification. On failure/partial: resume with `{scriptPath, resumeFromRunId}` — completed auditor calls return cached, only the broken tail re-runs. Do not hand-rerun auditors outside the workflow.

- [ ] **Step 3: Save the return value verbatim to `$SCRATCH/audit-results.json` and verify shape**

Check: exactly 11 elements; every element has `dim`, `title`, `status`, `findings`; count findings by `verdict.verdict` (`confirmed` / `refuted` / `null` = unverified). Record the counts — they feed the report's appendix.

No commit (scratch artifact).

---

### Task 4: Synthesize the report

**Files:**
- Create: `docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md`

**Interfaces:**
- Consumes: `$SCRATCH/audit-results.json` (Task 3), `$SCRATCH/audit-gates.md` (Task 1).
- Produces: the committed audit report (Task 6 commits it).

- [ ] **Step 1: Write the report with exactly this structure**

```markdown
# Harness + SDLC Audit — rust-agent-runtime

**Date:** 2026-07-06
**Method:** harness-engineering `audit.md` playbook (procedure) + agent-sdlc OKF bundle
(evidence layer) — 11 dimensions, one auditor each, every finding adversarially verified
(default-refute) before inclusion. Workflow run id: <runId>.
**Predecessor:** docs/superpowers/audits/2026-07-01-harness-deep-audit.md (fully closed 2026-07-02).
**REPORT ONLY.** No code was changed. Line numbers drift — re-open before acting.

## Ground truth
<verbatim okf_check + ci.sh results from $SCRATCH/audit-gates.md>

## Executive summary
<3-6 paragraphs: overall posture, where the confirmed findings cluster, prior-state verdict>

### Top 10 highest-leverage fixes
| # | Sev | Dimension | file:line | One-line fix |
<ranked severity x leverage, confirmed findings only; fewer than 10 rows if fewer qualify — do not pad>

## Prior-state regressions
<every regression named in any prior_state or finding; write "None found — all July fixes verified in place." if empty>

## Findings by dimension
### <n>. <title> — <status>
**Prior state:** <prior_state paragraph>
<one block per confirmed finding: severity / file:line (verifier-corrected) / violated principle +
source / concrete proposed fix / evidence excerpt / prior_decision note if present>
<unverified findings appear here flagged **[unverified — verifier unavailable]**>

## Appendix A — dropped findings
<one line per refuted finding: dimension, file:line, claim, verifier's refutation reason>

## Appendix B — dimension/verification stats
<per dimension: findings raised / confirmed / refuted / unverified; total agents used>
```

Rules while writing: severity in the report = the VERIFIER's severity when it differs (note the disagreement inline); `NOT_AUDITED` dimensions get their section with that status and no findings; re-proposed product decisions keep their `prior_decision` note visible.

- [ ] **Step 2: Verify report invariants**

Check against the spec's success criteria: every finding has 4 schema fields + verdict flag; all 11 dimensions present with explicit status; gate results embedded; zero production-code edits made. Fix inline.

No commit yet (Task 6 commits).

---

### Task 5: Re-stamp `audit.md`

**Files:**
- Modify: `.agents/skills/harness-engineering/audit.md` (the example-findings/re-stamp section — the playbook's own ledger, which it prescribes updating on re-run; not production code)

**Interfaces:**
- Consumes: the report path from Task 4.

- [ ] **Step 1: Append a dated re-stamp note**

After the last existing re-stamp note (currently the 2026-07-02 product-decision-round note, near the end of the re-stamp trail), append:

```markdown
Re-stamp note (2026-07-06, full harness+SDLC audit): a fresh 11-dimension audit was run — the
original seven components plus sub-agent orchestration (as a finished whole), the desktop/web
design-tab harness, the skills/OKF knowledge layer, and a process dimension (the SDLC as run by
agents, per docs/okf/agent-sdlc/comparisons/two-meanings.md). Every finding was adversarially
verified before inclusion. **The current findings snapshot is
`docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md`** — it supersedes both this section's
inline list and the 2026-07-01 report. Spec:
`docs/superpowers/specs/2026-07-06-full-harness-sdlc-audit-design.md`.
```

- [ ] **Step 2: Verify the note landed after the final existing note and nothing else changed**

Run: `git diff --stat .agents/skills/harness-engineering/audit.md` — expect exactly one file, additions only.

No commit yet (Task 6 commits).

---

### Task 6: Commit deliverables

**Files:**
- Commit: `docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md`, `.agents/skills/harness-engineering/audit.md`

- [ ] **Step 1: Confirm the working tree contains ONLY the two deliverables**

Run: `git status --porcelain`
Expected: exactly the report (new) and audit.md (modified). Anything else = REPORT-ONLY was violated — stop and investigate before committing.

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/audits/2026-07-06-harness-sdlc-audit.md .agents/skills/harness-engineering/audit.md
git commit -m "docs(audits): 2026-07-06 full harness+SDLC audit (11 dims, verified findings) + audit.md re-stamp"
```

- [ ] **Step 3: Report to the owner**

Surface: Top-10 table, regression verdict, per-dimension one-liners, dropped-findings count, and the reminder that triage/fixes are separate spec→plan cycles per cluster (the owner holds the judgment gate).
