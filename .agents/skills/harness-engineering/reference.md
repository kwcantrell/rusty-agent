# reference.md — harness advisor

Given a harness-design question, answer from the two spines + corroborating
principles in `SKILL.md`, with citations. For every claim, tag it:

- **[DURABLE]** — a principle with a source URL + tier. Safe to build on.
- **[DATED as of 2026-06]** — a specific number, model, or benchmark. Verify before relying.

Frame answers in the whitepaper's vocabulary: `Agent = Model + Harness`, the six
Harness Anatomy components, static vs dynamic context, Conductor vs Orchestrator
working modes. When a question touches contested ground, surface the relevant
**Caution** (e.g. "specs must replace vibe coding" was refuted 0-3) rather than
asserting it as a rule.

Do not restate the full knowledge base here — point into `SKILL.md`'s Spine A,
Spine B, corroborating principles, dated snapshots, cautions, and primary sources.

---

## How to use this playbook

1. **Read the question** and identify which Harness Anatomy component(s) it touches
   (Spine A) or whether it's a context-shape question (Spine B).
2. **Find the relevant durable principle(s)** in `SKILL.md`'s corroborating
   principles section. Each carries a source URL + tier.
3. **Check the dated snapshots** — if the answer requires a number, model name, or
   benchmark, pull it from that section and stamp it [DATED as of 2026-06].
4. **Check the cautions** — if the question's framing sounds like a contested
   claim, surface the caution before answering. Do not silently promote a refuted
   position.
5. **Tag every claim** with [DURABLE] or [DATED as of 2026-06]. Do not leave claims
   untagged.
6. **Name the relevant mode** — Conductor (real-time / synchronous / in-IDE) or
   Orchestrator (async / high-level / multi-agent) — when the question is about
   orchestration or agent topology.

---

## Worked examples

### Q1: Should I split this task into sub-agents?

**Answer:**

`Agent = Model + Harness`. Decomposing a task into sub-agents is an orchestration
decision, touching Harness Anatomy component 4 (Orchestration logic). Two durable
findings apply:

- **[DURABLE]** Decompose and delegate — orchestrator-workers, sub-agents-as-tools,
  one-feature-at-a-time; resist monolithic agent loops.
  Source: Anthropic — Building Effective Agents (tier: eng-blog); COMPASS /
  AOrchestra (tier: paper).

- **[DURABLE]** Prefer the **Orchestrator** working mode (async, high-level,
  multi-agent) when the task has high latency tolerance and requires autonomous
  parallel execution. Prefer **Conductor** (real-time, synchronous, in-IDE) when
  the task needs tight human-in-the-loop feedback.
  Source: The New SDLC With Vibe Coding, whitepaper p31-33 (tier: whitepaper).

When citing coordination gains from sub-agent decomposition:

- **[DATED as of 2026-06]** COMPASS reports "up to +20%" from context-management
  improvements. This is a single, unreplicated arXiv paper — treat as a directional
  signal, not a reliable benchmark. See `SKILL.md` Dated Snapshots.

**Caution:** AOrchestra "introduces the sub-agent-as-tools paradigm" is disputed —
it *automates* a pre-existing trend, it did not originate it. Cite accordingly
(see `SKILL.md` Cautions).

---

### Q2: Should we mandate specs over vibe coding for all harness work?

**Answer:**

**Caution — contested ground:** the claim that "vibe coding must be replaced by
explicit specifications" was **refuted 0-3** in the design review for this skill.
Do not assert it as a rule.

The durable position, from the same whitepaper corpus, is:

- **[DURABLE]** "Structure scales and amplifies culture." Structured processes
  (specs, checklists, formal reviews) raise the ceiling for teams that already
  work carefully; they do not substitute for judgment, and informal coding is not
  inherently invalid.
  Source: The New SDLC With Vibe Coding, whitepaper (tier: whitepaper); Spec-Driven
  Production-Grade Development (tier: whitepaper) — as corroboration, not mandate.

The practical guidance: use specs when the harness decision is design-bearing or
high-blast-radius (e.g. static/dynamic context boundary, tool schema changes,
orchestration topology). Skip the ceremony for low-stakes, obviously-correct fixes.
— *[SYNTHESIZED guidance: drawn from the sources cited above, no single source]*
See `SKILL.md` Spine B for the static/dynamic boundary as "a first-class
architectural decision, reviewed and versioned like code."

---

### Q3: How many tools should this agent expose?

**Answer:**

Tools are Harness Anatomy component 2 (see `SKILL.md` Spine A). The durable finding:

- **[DURABLE]** Fewer, consolidated, token-efficient tools beat thin endpoint
  wrappers. Combine related read operations; avoid per-endpoint MCP tools; give
  each tool a clear name, tight description, and explicit "when NOT to call"
  guidance.
  Source: Anthropic — Writing Tools for Agents (tier: eng-blog).

In the context of `Agent = Model + Harness`: every tool consumes static context
budget (Spine B). A proliferation of thin tools erodes the context window and
degrades the model's ability to select correctly. Consolidation is a form of
context engineering.

There is no authoritative "right number" — the benchmark is: can the model
reliably choose the correct tool, given only the descriptions, without ambiguity?
If not, consolidate or sharpen descriptions before adding more tools.
— *[SYNTHESIZED guidance: drawn from the sources cited above, no single source]*

Numbers like "< N tools per agent" or "MCP server tool limits" are **[DATED as of
2026-06]** and tied to specific model context windows that change with each release.
Do not hard-code a count as a rule; re-evaluate when model/context changes.

---

## Quick-reference: tag map

| Claim type | Tag |
|---|---|
| Named principle with source URL + tier | **[DURABLE]** |
| Benchmark number, model name, score delta | **[DATED as of 2026-06]** |
| Contested or refuted claim | Surface the **Caution** from `SKILL.md`; do not promote |

---

## See also

- `SKILL.md` — full knowledge base (Spine A, Spine B, corroborating principles,
  dated snapshots, cautions, primary source URLs)
- `audit.md` — when the task is "find gaps in the harness"
- `build.md` — when the task is "build or refactor a harness component"
- `eval.md` — when the task is "design evals to isolate harness vs model"
