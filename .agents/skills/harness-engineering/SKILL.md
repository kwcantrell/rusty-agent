---
name: harness-engineering
description: >-
  Use to design, audit, build, or evaluate the *harness* around this runtime's
  LLM — the agent loop, tool design & prose, sub-agent orchestration, sandboxes,
  guardrails/permissions/hooks, observability, error recovery, and long-horizon
  execution. Invoke when asked to audit/improve/design the agent harness, reason
  about context engineering, or co-design harness evals. Routes to one of four
  playbooks based on the ask.
---

# harness-engineering

**Agent = Model + Harness.** The harness is "the scaffolding wrapped around the
model that lets it actually finish something" and accounts for **~90% of agent
behavior vs ~10% for the model** (whitepaper Fig. 7). Leading principle:

> "Most agent failures, examined honestly, are configuration failures."

Harness work is the highest-leverage, most-attributable investment: on Terminal
Bench 2.0 one team moved a coding agent **from outside the Top 30 to the Top 5 by
changing only the harness, no model change** (whitepaper p31); a LangChain study
raised score **+13.7 points** by tweaking only system prompt, tools, and
middleware around a fixed model. This skill makes that work systematic for
`rust-agent-runtime`.

## Which playbook

| If the ask is… | Load |
|---|---|
| "audit / find gaps in / review the harness" | `audit.md` |
| "how should I …?" / a harness-design question | `reference.md` |
| "build / refactor <component>" (orchestration, retry, guardrails, tools) | `build.md` |
| "how do I eval this / isolate harness vs model / build trajectory evals" | `eval.md` |

Read only the playbook the task needs (progressive disclosure). The shared
knowledge base below is common to all four.

## Spine A — Harness Anatomy

The whitepaper's six concrete harness components (p27-28, Fig. 7). Each maps to a
lifecycle phase and a concrete presence in this runtime.

1. **Instructions & Rule Files** — who the agent is / what it's forbidden from
   (rule files, skill files, sub-agent prompts). *What good looks like:* a single,
   versioned source of truth per agent role; no contradictory or stale rule files.

2. **Tools** — functions, MCP servers, APIs + the prose telling the model when/how
   to call them. *What good looks like:* each tool has a clear name, tight
   description, and explicit "when NOT to call" guidance; no thin endpoint wrappers
   when a consolidated tool will do.

3. **Sandboxes & execution environments** — where code runs, what it can/can't
   reach. *What good looks like:* execution is isolated by default; capabilities
   are explicitly granted, not ambient.

4. **Orchestration logic** — sub-agent spawning, model routing, hand-offs, and the
   rules governing when each fires. *What good looks like:* explicit routing rules;
   no "ask the model to decide" unless the decision is truly judgment-gated.

5. **Guardrails / Hooks** — deterministic code at lifecycle points (before tool
   call, after edit, before commit). *What good looks like:* hooks are fast,
   side-effect-free validators; they block bad actions, not delay good ones.

6. **Observability** — logs, traces, evals, cost/latency metering; "without it, no
   way to tell if the agent is drifting." *What good looks like:* every tool call
   and model turn is logged with enough context to replay and diagnose.

These six components map across the whitepaper's four SDLC phases (p29-30):
**Configuring → Running → Feedback Loop → Observing the Harness.** `audit.md`
walks all six against the runtime.

## Spine B — Context Engineering

The whitepaper's six types of context (p15): **Instructions, Knowledge, Memory,
Examples, Tools, Guardrails** — split into:

- **Static** (always loaded, expensive): Instructions, baseline Knowledge,
  core Guardrails. Load only what every session needs; no dead weight.
- **Dynamic** (on-demand, pay-per-use): Memory recalls, per-task Examples,
  situational Tools, progressive-disclosure skill bodies.

The static/dynamic boundary is "a first-class architectural decision, reviewed and
versioned like code." Blurring it is the most common context bloat path.

The premier dynamic-context pattern is **Agent Skills via progressive disclosure**:
metadata at startup → full instructions on task match → deep reference only when
needed. This maps directly to this repo's `.agents/skills/` model and is the
concrete application of Spine B to this runtime. Drives `eval.md` + context
guidance.

## Corroborating durable principles

Each carries source + tier. These deepen Spines A and B; they are the stable
findings across the research corpus.

- **Workflows vs. agents** (predefined code paths vs. self-directed) — know which
  mode you're in before designing orchestration.
  `tier: eng-blog` — Anthropic: Building Effective Agents

- **Fewer, consolidated, token-efficient tools** beat thin endpoint wrappers —
  combine related read operations; avoid per-endpoint MCP tools.
  `tier: eng-blog` — Anthropic: Writing Tools for Agents

- **Externalize & curate context; durable progress artifacts** over relying on a
  growing window or compaction to save you.
  `tier: eng-blog` — Anthropic: Effective Harnesses for Long-Running Agents

- **Decompose & delegate** — orchestrator-workers, sub-agents-as-tools,
  one-feature-at-a-time; resist monolithic agent loops.
  `tier: eng-blog` — Anthropic; `tier: paper` — COMPASS / AOrchestra

- **Judgment over generation; the 80% problem** — AI does ~80%; the 20% (edge
  cases, error handling, integration, subtle correctness) needs human judgment.
  The harness must surface the 20%, not hide it.
  `tier: whitepaper p34` — Google: The New SDLC With Vibe Coding

- **Co-design harness with eval; isolate harness contribution from model** (the
  "conflation problem"). Evals checked by labelled datasets, scoring rubrics, LM
  judges. Without isolation you cannot attribute improvements.
  `tier: paper` — arXiv 2503.16416; `tier: whitepaper p15`

- **Conductor vs. Orchestrator** working modes — real-time/synchronous/in-IDE
  (Conductor) vs. async/high-level/multi-agent (Orchestrator). Match the mode to
  the task's latency and human-involvement requirements.
  `tier: whitepaper p31-33` — Google: The New SDLC With Vibe Coding

## Dated snapshots (as of 2026-06 — illustrative, not authoritative)

Benchmark numbers and model names drift. These are illustrative reference points,
not durable facts. Segregated here to avoid mixing into durable principles.

- Terminal-Bench 2.0 best config ≈ 63%; frontier agents < 65%.
- Harness-only leverage: Top 30 → Top 5 (whitepaper p31); LangChain +13.7 pts.
- COMPASS "up to +20%" (single, unreplicated arXiv paper — treat with caution).
- Specific model names cited at time of writing: GPT-5.2, Gemini 3 Pro,
  Claude Opus 4.5. These will be superseded.

## Cautions (contested / refuted — warnings, NOT rules)

- **"Vibe coding must be replaced by explicit specifications"** was **refuted 0-3**
  in the design review. The durable position is "structure scales and amplifies
  culture," not that informal coding is invalid per se. Do not cite the refuted
  form as a principle.

- **AOrchestra "introduces the sub-agent-as-tools paradigm"** drew a dissent — it
  *automates* a pre-existing trend, it did not originate it. Cite accordingly.

## Primary sources

- **Google — The New SDLC With Vibe Coding** (PRIMARY SPINE, vendored PDF): https://www.kaggle.com/whitepaper-the-new-SDLC-with-vibe-coding
- Google — Spec-Driven Production-Grade Development: https://www.kaggle.com/whitepaper-spec-driven-production-grade-development-in-the-age-of-vibe-coding
- Anthropic — Building Effective Agents: https://www.anthropic.com/research/building-effective-agents
- Anthropic — Writing Tools for Agents: https://www.anthropic.com/engineering/writing-tools-for-agents
- Anthropic — Effective Harnesses for Long-Running Agents: https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents
- Anthropic — Effective Context Engineering: https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- COMPASS (context management): https://arxiv.org/pdf/2510.08790
- AOrchestra (dynamic sub-agents): https://arxiv.org/pdf/2602.03786
- Survey on Evaluation of LLM-based Agents (conflation problem): https://arxiv.org/html/2503.16416v2
- Terminal-Bench 2.0: https://arxiv.org/html/2601.11868v1

Vendored copy: `docs/superpowers/context/the-new-sdlc-with-vibe-coding-google-2026.pdf`.
