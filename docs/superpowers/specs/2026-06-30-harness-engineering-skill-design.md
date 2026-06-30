# Design: `harness-engineering` skill

**Date:** 2026-06-30
**Status:** Approved (design) — pending implementation plan
**Author:** brainstormed with Claude Code, seeded by a verified deep-research report on AI harness engineering

## Purpose

A single operational skill, `harness-engineering`, that helps design, audit, build, and
evaluate the scaffolding around an LLM in this `rust-agent-runtime`. It distills a
verified body of 2024–2026 research/best-practice into a cited checklist and four
invocable playbooks. It is modeled on the existing `context-evolve` skill shape
(a router `SKILL.md` plus playbook files), and is **operational** — it acts on this
runtime's actual code — not a passive reference essay.

## Scope decisions (settled during brainstorming)

1. **Packaging:** one skill, four playbooks (NOT four separate skills). Rationale: the
   four modes share one knowledge base; separate skills would duplicate the checklist
   and create four competing trigger descriptions.
2. **Boundary with existing skills:** **fully self-contained** (user's explicit choice).
   It gives complete guidance including context-window eval/optimization. It
   *cross-references* `context-evolve` (live correctness-gated/token-tiebreak campaign)
   and `context-management` as concrete living examples, but does not depend on them.
3. **Auditor teeth:** **findings + ranked fixes, no auto-edit.** Each finding cites the
   violated principle + source, points at file/line, rates severity, and proposes a
   concrete fix. The human holds the judgment gate (aligns with the research's
   "judgment over generation" principle).
4. **`build.md` is advisory:** it supplies patterns and hands off to the normal
   `writing-plans` / TDD flow. It does NOT drive edits itself (unlike `context-evolve`'s
   `train.md`).

## Structure & placement

```
.agents/skills/harness-engineering/
  SKILL.md       router + the shared verified checklist (the research, distilled & cited)
  audit.md       operational auditor playbook
  reference.md   advisor / lookup playbook
  build.md       build-or-refactor driver playbook (advisory)
  eval.md        eval co-design playbook (self-contained)
```

`SKILL.md` frontmatter:
- `name: harness-engineering`
- `description:` triggers on requests to audit/improve/design the agent harness — the
  agent loop, tool design, sub-agent orchestration, guardrails/permissions, error
  recovery, or long-horizon execution. Routes to the right playbook based on the ask.

## The shared knowledge base (lives once, in `SKILL.md`)

The 24 adversarially-verified findings from the research report, distilled into a
**checklist** organized by the four areas. Each checklist item carries:
- the principle (one line),
- its source URL + tier (`paper/whitepaper` vs `eng-blog`),
- a one-line "what good looks like."

The checklist is split into two registers, because the research explicitly flagged some
facts as fast-dating:

- **Durable Principles** (safe to encode as rules):
  1. Workflows (predefined code paths) vs. agents (self-directed) — know which you're building.
  2. Fewer, consolidated, token-efficient tools beat many thin endpoint wrappers.
  3. Externalize & curate context (durable progress artifacts; don't rely on a growing window or compaction alone).
  4. Decompose & delegate — one feature at a time; orchestrator-workers; sub-agents-as-tools.
  5. Judgment over generation — review gates, specs, evaluation are the scarce skill.
  6. Co-design the harness with its eval; isolate harness contribution from model capability (the "conflation problem").

- **Dated Snapshots** (record with an "as of 2026-06" stamp, treat as illustrative not authoritative):
  - Terminal-Bench 2.0 best config ≈ 63%; frontier agents < 65%.
  - COMPASS "up to +20%" accuracy figure (single, unreplicated arXiv paper).
  - Specific model names (GPT-5.2, Gemini 3 Pro, Claude Opus 4.5) and benchmark numbers.

- **Cautions** (contested / refuted — record as warnings, NOT rules):
  - "Vibe coding must be replaced by explicit specifications" was REFUTED 0-3. The durable
    position is "structure scales and amplifies culture," not that informal coding is invalid.
  - AOrchestra "introduces the sub-agent-as-tools paradigm" drew a dissent — it *automates*
    a pre-existing trend.

### Primary sources (cite these in the checklist)
- Anthropic — Building Effective Agents: https://www.anthropic.com/research/building-effective-agents
- Anthropic — Writing Tools for Agents: https://www.anthropic.com/engineering/writing-tools-for-agents
- Anthropic — Effective Harnesses for Long-Running Agents: https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents
- Anthropic — Effective Context Engineering: https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- COMPASS (context management): https://arxiv.org/pdf/2510.08790
- AOrchestra (dynamic sub-agents): https://arxiv.org/pdf/2602.03786
- Survey on Evaluation of LLM-based Agents (conflation problem): https://arxiv.org/html/2503.16416v2
- Terminal-Bench 2.0: https://arxiv.org/html/2601.11868v1
- Google/Kaggle — The New SDLC with Vibe Coding: https://www.kaggle.com/whitepaper-the-new-SDLC-with-vibe-coding
- Google/Kaggle — Spec-Driven Production-Grade Development: https://www.kaggle.com/whitepaper-spec-driven-production-grade-development-in-the-age-of-vibe-coding

## The four playbooks

### `audit.md` — operational auditor
Walks the checklist against the runtime. For each item:
1. Inspect the anchored file(s) in this repo.
2. Judge conformance.
3. Emit a finding: `{ severity, file:line, violated principle + source, concrete proposed fix }`.

**Reports only — never edits.** Output is a ranked gap report ending with "top 3
highest-leverage fixes." Codebase anchors (verified to exist as of 2026-06-30):
- Agent loop: `crates/agent-core/src/loop_.rs`
- Tool design: `crates/agent-tools/src/tool.rs`, `crates/agent-tools/src/types.rs`
- Context mgmt: `crates/agent-core/src/{context,context_tools,offload,offload_policy}.rs`
- Model protocol: `crates/agent-model/src/protocol.rs` (+ `openai.rs`, `claude_cli.rs`, `prompted.rs`)
- Server/runtime: `crates/agent-server/src/runtime.rs`, `wire.rs`
- Config: `crates/agent-runtime-config/src/lib.rs`

The playbook instructs reading these at audit time (it does not hard-code their internals,
which drift).

### `reference.md` — advisor / lookup
Given a harness-design question, answer from the checklist with citations, always
distinguishing a **durable principle** from a **dated specific**. Surfaces the relevant
"Caution" when a question touches contested ground.

### `build.md` — build-or-refactor driver (advisory)
When building/refactoring a component (e.g. sub-agent orchestration, retry/guardrails),
supply the relevant named patterns (orchestrator-workers, sub-agent-as-tools,
one-feature-at-a-time, externalized progress artifact, consolidated tools). **Advises,
then hands off** to the normal `writing-plans` / TDD flow. Does not drive edits itself.

### `eval.md` — eval co-design (self-contained)
How to build trajectory/process evals that isolate harness contribution from model
capability (the conflation problem): outcome vs. process scoring, reference-based
trajectory comparison (exact/partial/unordered/subset), the agentic eval loop
(while-loop wrapping alternating LLM + tool calls), and correctness-gated/token-tiebreak
promotion. References `context-evolve` as a concrete living instance of this pattern but
stands alone.

## What this skill deliberately is NOT
- Not auto-fixing (no code edits from the auditor).
- Not a framework or runtime dependency.
- Not a duplicate of `context-evolve`'s campaign loop — for context-window *tuning* it
  points there.
- No telemetry, no scoring rubric beyond severity ranking (YAGNI).

## Success criteria
- `harness-engineering` triggers on harness-design/audit asks and routes to the right playbook.
- Running `audit.md` against the current runtime produces a cited, ranked gap report that
  points at real files and proposes concrete fixes — without editing code.
- The checklist's durable principles each carry a source URL + tier; dated snapshots are
  stamped and segregated; refuted/contested claims appear as cautions.
- The skill is usable with no dependency on `context-evolve` / `context-management`, while
  cross-referencing them where relevant.

## Open follow-ups (not blocking this skill)
The research thinly covered error-recovery/retry, guardrails/permission tiers, and parallel
tool execution. `audit.md` and `build.md` will include checklist stubs for these marked
"under-sourced — judge from first principles + the runtime's own patterns," rather than
fabricating authority.
