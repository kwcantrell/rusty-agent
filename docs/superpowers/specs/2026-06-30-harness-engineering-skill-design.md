# Design: `harness-engineering` skill

**Date:** 2026-06-30
**Status:** Approved (design) — pending implementation plan
**Author:** brainstormed with Claude Code, seeded by a verified deep-research report on AI
harness engineering, then re-anchored on the fetched primary source (Google's *The New
SDLC With Vibe Coding* whitepaper, May 2026, Osmani/Saboo/Kartakis).

## North-star framing (from the whitepaper, fetched primary source)

The skill's organizing thesis is the whitepaper's central equation:

> **Agent = Model + Harness**

The harness is "the scaffolding wrapped around the model that lets it actually finish
something" and, per the whitepaper's Figure 7, accounts for **~90% of agent behavior vs.
~10% for the model**. The motivating quote the skill leads with:

> "Most agent failures, examined honestly, are configuration failures."

Quantified harness leverage (whitepaper, p31, fetched primary source): on Terminal Bench
2.0 one team moved a coding agent **from outside the Top 30 to the Top 5 by changing only
the harness, no model change**; a separate LangChain study raised score by **+13.7 points**
by tweaking only system prompt, tools, and middleware around a fixed model. This is the
*why* of the whole skill: harness work is the highest-leverage, most-attributable
investment, and this skill makes it systematic for this runtime.

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
  SKILL.md       router + shared knowledge base (whitepaper spines + corroborating findings, cited)
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

The knowledge base is **anchored on the whitepaper's two named taxonomies** (authoritative,
fetched primary source), with the deep-research findings as corroboration. Each checklist
item carries the principle (one line), its source + tier, and a one-line "what good looks
like."

### Spine A — Harness Anatomy (drives `audit.md`)
The whitepaper's "What's in the harness" — the **six concrete harness components**
(whitepaper p28; Figure 7 "Harness Anatomy", p27):
1. **Instructions & Rule Files** — who the agent is / what it's forbidden from. (`AGENTS.md`,
   `CLAUDE.md`, `GEMINI.md`, skill files, sub-agent prompts.)
2. **Tools** — functions, MCP servers, APIs + the prose telling the model when/how to call them.
3. **Sandboxes & execution environments** — where code runs, what it can/can't reach.
4. **Orchestration logic** — sub-agent spawning, model routing, hand-offs, the rules governing when each fires.
5. **Guardrails / Hooks** — deterministic code at lifecycle points (before tool call, after edit, before commit).
6. **Observability** — logs, traces, evals, cost/latency metering; "without it, no way to tell if the agent is drifting."

These map cleanly across the **four SDLC phases** the whitepaper names (p29–30): Configuring
the Harness → Running the Harness → Feedback Loop → Observing the Harness. `audit.md` walks
all six components and reports gaps.

### Spine B — Context Engineering (drives `eval.md` + context guidance)
The whitepaper's **six types of context** (p15): Instructions, Knowledge, Memory, Examples,
Tools, Guardrails — split into **static** (always loaded, expensive) vs **dynamic** (on-demand,
pay-per-use). The static/dynamic boundary is "a first-class architectural decision, reviewed
and versioned like code." The premier dynamic-context pattern is **Agent Skills** via
**progressive disclosure** (metadata at startup → full instructions on task match → deep
reference only when needed), which directly maps to this repo's own `.agents/skills/` model.

### Corroborating durable principles (deep-research findings)
Used to deepen Spines A/B, each with source + tier:
- Workflows (predefined code paths) vs. agents (self-directed) — `eng-blog: Anthropic building-effective-agents`.
- Fewer, consolidated, token-efficient tools beat thin endpoint wrappers — `eng-blog: Anthropic writing-tools-for-agents`.
- Externalize & curate context; durable progress artifacts over relying on a growing window/compaction — `eng-blog: Anthropic long-running-agents`.
- Decompose & delegate — orchestrator-workers, sub-agents-as-tools, one-feature-at-a-time — `eng-blog: Anthropic` + `paper: COMPASS / AOrchestra`.
- Judgment over generation; the **80% problem** (AI does ~80%; the 20% — edge cases, error handling, integration, subtle correctness — needs human judgment) — `whitepaper p34`.
- Co-design harness with eval; isolate harness contribution from model (the "conflation problem"); evals checked by labelled datasets, scoring rubrics, LM judges — `paper: arXiv 2503.16416` + `whitepaper p15`.
- **Conductor vs. Orchestrator** working modes (real-time/synchronous/in-IDE vs. async/high-level/multi-agent) — `whitepaper p31–33`.

### Dated Snapshots (stamp "as of 2026-06", illustrative not authoritative)
- Terminal-Bench 2.0 best config ≈ 63%; frontier agents < 65%.
- Harness-only leverage: Top 30 → Top 5 (whitepaper); LangChain +13.7 pts.
- COMPASS "up to +20%" (single, unreplicated arXiv paper).
- Specific model names (GPT-5.2, Gemini 3 Pro, Claude Opus 4.5) and benchmark numbers.

### Cautions (contested / refuted — warnings, NOT rules)
- "Vibe coding must be replaced by explicit specifications" was REFUTED 0-3. The durable
  position is "structure scales and amplifies culture," not that informal coding is invalid.
- AOrchestra "introduces the sub-agent-as-tools paradigm" drew a dissent — it *automates* a
  pre-existing trend.

### Primary sources (cite these in the checklist)
- **Google — The New SDLC With Vibe Coding** (PRIMARY SPINE, fetched PDF, May 2026): https://www.kaggle.com/whitepaper-the-new-SDLC-with-vibe-coding
- Google — Spec-Driven Production-Grade Development: https://www.kaggle.com/whitepaper-spec-driven-production-grade-development-in-the-age-of-vibe-coding
- Anthropic — Building Effective Agents: https://www.anthropic.com/research/building-effective-agents
- Anthropic — Writing Tools for Agents: https://www.anthropic.com/engineering/writing-tools-for-agents
- Anthropic — Effective Harnesses for Long-Running Agents: https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents
- Anthropic — Effective Context Engineering: https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- COMPASS (context management): https://arxiv.org/pdf/2510.08790
- AOrchestra (dynamic sub-agents): https://arxiv.org/pdf/2602.03786
- Survey on Evaluation of LLM-based Agents (conflation problem): https://arxiv.org/html/2503.16416v2
- Terminal-Bench 2.0: https://arxiv.org/html/2601.11868v1

**Note:** the whitepaper was fetched and read directly (10MB PDF, pages 1-10, 15-18, 26-35);
its claims are quoted from the source body, not search summaries. The earlier spec's caveat
about Kaggle pages being unfetchable no longer applies to this whitepaper.

## The four playbooks

### `audit.md` — operational auditor
Walks **the six Harness Anatomy components** (Spine A) against the runtime. For each
component:
1. Inspect the anchored file(s) in this repo.
2. Judge conformance against that component's checklist items + corroborating principles.
3. Emit a finding: `{ severity, file:line, violated principle + source, concrete proposed fix }`.

**Reports only — never edits.** Output is a ranked gap report ending with "top 3
highest-leverage fixes." The six components map to this runtime as follows (anchors verified
to exist as of 2026-06-30; the playbook re-reads them at audit time rather than hard-coding
internals, which drift):

| Harness component | Runtime anchor |
|---|---|
| 1. Instructions & Rule Files | `.agents/skills/**`, repo `CLAUDE.md`/`AGENTS.md`, sub-agent prompts |
| 2. Tools | `crates/agent-tools/src/tool.rs`, `crates/agent-tools/src/types.rs`, `crates/agent-core/src/context_tools.rs` |
| 3. Sandboxes & execution | tool execution path in `crates/agent-tools/**`, `crates/agent-server/src/runtime.rs` |
| 4. Orchestration logic | `crates/agent-core/src/loop_.rs` (incl. parallel tool calls), `crates/agent-model/src/protocol.rs` |
| 5. Guardrails / Hooks | permission/limit checks across `agent-core`/`agent-server`; `crates/agent-runtime-config/src/lib.rs` |
| 6. Observability | logging/metering across `agent-server`/`agent-core`; offload/eval surfaces |
| (Context engineering, Spine B) | `crates/agent-core/src/{context,context_tools,offload,offload_policy}.rs` |

Components the deep-research thinly sourced (error-recovery/retry, guardrails/permission
tiers, parallel tool execution) are audited **against the runtime's own existing patterns**
(e.g. `loop_.rs` already runs parallel tool calls) as the local reference, marked
"judge from first principles + this runtime's conventions" rather than asserting external
authority.

### `reference.md` — advisor / lookup
Given a harness-design question, answer from the two spines + corroborating principles with
citations, always distinguishing a **durable principle** from a **dated specific**. Frames
answers using the whitepaper's vocabulary (`Agent = Model + Harness`, the six components,
static vs. dynamic context, Conductor vs. Orchestrator modes). Surfaces the relevant
"Caution" when a question touches contested ground.

### `build.md` — build-or-refactor driver (advisory)
When building/refactoring a component (e.g. sub-agent orchestration, retry/guardrails),
supply the relevant named patterns (orchestrator-workers, sub-agents-as-tools,
one-feature-at-a-time, externalized progress artifact, consolidated tools, static/dynamic
context split, progressive-disclosure skills). Explicitly invokes the **80% problem** to mark
where human judgment must stay in the loop. **Advises, then hands off** to the normal
`writing-plans` / TDD flow. Does not drive edits itself.

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

## Source recovery
The whitepaper PDF (authoritative spine) is **vendored in-repo** at
`docs/superpowers/context/the-new-sdlc-with-vibe-coding-google-2026.pdf` (~10 MB). The
implementation plan reads/quotes it directly — no network needed. Sections used: pp. 6-10
(intro/agent loop), 15-18 (context engineering), 26-35 (harness engineering + roles).
Original source: Google Drive id `1IR7CddF_2FyQo_PdfBNTaEA50EGiVt2r`
(`https://drive.google.com/uc?export=download&id=1IR7CddF_2FyQo_PdfBNTaEA50EGiVt2r`).

## Open follow-ups (not blocking this skill)
The deep-research thinly covered error-recovery/retry, guardrails/permission tiers, and
parallel tool execution. These are now handled by auditing **against the runtime's own
existing patterns** as the local reference (e.g. `loop_.rs`'s parallel tool calls, config
limits), marked "judge from first principles + this runtime's conventions" rather than
fabricating external authority. The whitepaper names a companion "Day-3" paper on *Context
Engineering: Sessions, Skills & Memory* — a candidate future source if the context spine
needs deepening.
