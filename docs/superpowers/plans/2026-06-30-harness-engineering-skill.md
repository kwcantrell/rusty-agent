# harness-engineering Skill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Also required per authoring task:** consult `superpowers:writing-skills` for skill-file conventions (frontmatter shape, trigger-oriented `description`, progressive disclosure via on-demand playbooks) before writing each `.md`.

**Goal:** Build a single operational `harness-engineering` skill (router `SKILL.md` + four playbooks `audit.md` / `reference.md` / `build.md` / `eval.md`) that helps design, audit, build, and evaluate the LLM scaffolding in this `rust-agent-runtime`, anchored on the vendored Google *New SDLC With Vibe Coding* whitepaper.

**Architecture:** Mirrors the existing `context-evolve` skill shape — a router `SKILL.md` holding the shared, cited knowledge base (two whitepaper taxonomies + corroborating findings), plus four on-demand playbook files it routes to. The skill is **fully self-contained** and **operational** (it acts on this repo's real code); the auditor **reports findings + ranked fixes but never edits code**.

**Tech Stack:** Markdown skill files under `.agents/skills/`, YAML-ish frontmatter (block-scalar `description:`), verified against the runtime's real crate paths. No code compilation; "tests" are runnable shell checks (frontmatter structure, repo-path existence, required-content greps).

## Global Constraints

Copied verbatim from the spec (`docs/superpowers/specs/2026-06-30-harness-engineering-skill-design.md`) and the verification done during planning. Every task's requirements implicitly include this section.

- **ONE skill, FOUR playbooks** — not four skills. Shared knowledge base lives once, in `SKILL.md`.
- **Fully self-contained.** Cross-references `context-evolve` and `context-management` as living examples but does NOT depend on them.
- **Auditor never edits code** — emits `{ severity, file:line, violated principle + source, concrete proposed fix }` and a "top 3 highest-leverage fixes" tail. Human holds the judgment gate.
- **`build.md` is advisory** — supplies patterns, invokes the 80% problem, then hands off to `writing-plans` / TDD. It does NOT drive edits.
- **Primary spine = the vendored whitepaper**, read/quoted directly at `docs/superpowers/context/the-new-sdlc-with-vibe-coding-google-2026.pdf`. No network needed.
- **Durable principles carry a source URL + tier; dated snapshots are stamped "as of 2026-06" and segregated; refuted/contested claims appear only as cautions, never rules.**
- **Verified repo anchor paths use the `agent/` prefix** (e.g. `agent/crates/agent-core/src/loop_.rs`). The spec's anchor table omits `agent/`; the real files live under `agent/crates/...`. Use the prefixed, verified form. There is **no root `AGENTS.md`** — only root `CLAUDE.md`. The auditor audits whatever rule files actually exist.
- **Anchors are re-read at audit time, not hard-coded** — the playbook cites the file to open, not its current internals (which drift).
- Conventional commits: `docs(skill): …` scope. Commit after each task. Do not push (push only when the user asks).

---

## Verification helpers (used by multiple tasks)

Two dependency-free checks are reused below. They are defined here once; tasks reference them by name.

**CHECK-FM `<file>`** — frontmatter structural validity (no pyyaml on this box):

```bash
check_fm() {
  f="$1"
  head -1 "$f" | grep -qx -- '---' || { echo "FAIL: $f does not start with ---"; return 1; }
  # closing --- must exist on some later line
  awk 'NR>1 && $0=="---"{found=1; exit} END{exit !found}' "$f" || { echo "FAIL: $f has no closing ---"; return 1; }
  # name + description keys must appear inside the frontmatter block (before closing ---)
  awk 'NR>1 && $0=="---"{exit} {print}' "$f" | grep -q '^name:' || { echo "FAIL: $f frontmatter missing name:"; return 1; }
  awk 'NR>1 && $0=="---"{exit} {print}' "$f" | grep -q '^description:' || { echo "FAIL: $f frontmatter missing description:"; return 1; }
  echo "PASS: $f frontmatter valid"; return 0
}
```

**CHECK-PATHS `<file>`** — every backtick-quoted repo path cited in the file must exist (tries bare and `agent/`-prefixed):

```bash
check_paths() {
  f="$1"; miss=0
  # extract backtick-delimited tokens that look like repo paths: contain a slash and a known root
  grep -oE '`[^`]+`' "$f" | tr -d '`' \
    | grep -E '(^|/)(crates|\.agents|docs|src-tauri|web)/' \
    | sed -E 's/:[0-9].*$//' | sort -u \
    | while read -r p; do
        [ -e "$p" ] || [ -e "agent/$p" ] || { echo "MISS: $p (in $f)"; }
      done | tee /tmp/misspaths
  if [ -s /tmp/misspaths ]; then echo "FAIL: unresolved paths in $f"; return 1; fi
  echo "PASS: all repo paths in $f resolve"; return 0
}
```

Run both from the repo root (`/home/kalen/rust-agent-runtime`). Paste the function definition into the shell (or source a scratch file) before calling.

---

## Task 1: Router `SKILL.md` + shared knowledge base

This is the spine. It holds the frontmatter, the North-star framing, the routing map to the four playbooks, and the entire shared knowledge base (Spine A, Spine B, corroborating principles, dated snapshots, cautions, primary sources). All quotes/facts are drawn from the spec and the vendored PDF — reproduce them exactly.

**Files:**
- Create: `.agents/skills/harness-engineering/SKILL.md`
- Reference (read, do not edit): `docs/superpowers/specs/2026-06-30-harness-engineering-skill-design.md`, `docs/superpowers/context/the-new-sdlc-with-vibe-coding-google-2026.pdf` (pp. 6-10, 15-18, 26-35), `.agents/skills/context-evolve/SKILL.md` (style model)
- Test: inline shell (CHECK-FM + content greps below)

**Interfaces:**
- Produces: the skill directory `.agents/skills/harness-engineering/` and the file `SKILL.md`. Later tasks add `audit.md`, `reference.md`, `build.md`, `eval.md` as siblings. `SKILL.md`'s routing section names those four files by exact filename — later tasks must match those names.
- Consumes: nothing (first task).

- [ ] **Step 1: Write the failing verification script**

Create a scratch checker and run it (it will fail because the file doesn't exist yet). Paste CHECK-FM (from "Verification helpers") into the shell, then:

```bash
cd /home/kalen/rust-agent-runtime
F=.agents/skills/harness-engineering/SKILL.md
test -f "$F" && check_fm "$F" || echo "FAIL: $F missing"
```

- [ ] **Step 2: Run it to confirm it fails**

Run the Step 1 block. Expected: `FAIL: .agents/skills/harness-engineering/SKILL.md missing`.

- [ ] **Step 3: Create the directory and write frontmatter + North-star framing**

```bash
mkdir -p .agents/skills/harness-engineering
```

Write `SKILL.md` beginning with block-scalar frontmatter (mirror `context-evolve`'s `description: >-` style):

```markdown
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
```

- [ ] **Step 4: Add the routing map**

Add a "## Which playbook" section that routes the ask to exactly one of the four playbook files (use these exact filenames — later tasks depend on them):

```markdown
## Which playbook

| If the ask is… | Load |
|---|---|
| "audit / find gaps in / review the harness" | `audit.md` |
| "how should I …?" / a harness-design question | `reference.md` |
| "build / refactor <component>" (orchestration, retry, guardrails, tools) | `build.md` |
| "how do I eval this / isolate harness vs model / build trajectory evals" | `eval.md` |

Read only the playbook the task needs (progressive disclosure). The shared
knowledge base below is common to all four.
```

- [ ] **Step 5: Add Spine A — Harness Anatomy (six components)**

Add "## Spine A — Harness Anatomy" (whitepaper p27-28, Fig. 7). List all six components verbatim in intent; each item = principle (one line) + "what good looks like":

1. **Instructions & Rule Files** — who the agent is / what it's forbidden from (rule files, skill files, sub-agent prompts).
2. **Tools** — functions, MCP servers, APIs + the prose telling the model when/how to call them.
3. **Sandboxes & execution environments** — where code runs, what it can/can't reach.
4. **Orchestration logic** — sub-agent spawning, model routing, hand-offs, and the rules governing when each fires.
5. **Guardrails / Hooks** — deterministic code at lifecycle points (before tool call, after edit, before commit).
6. **Observability** — logs, traces, evals, cost/latency metering; "without it, no way to tell if the agent is drifting."

Then one line mapping these to the whitepaper's four SDLC phases (p29-30): Configuring → Running → Feedback Loop → Observing the Harness. Note: "`audit.md` walks all six against the runtime."

- [ ] **Step 6: Add Spine B — Context Engineering (six context types)**

Add "## Spine B — Context Engineering" (whitepaper p15). List the **six types of context**: Instructions, Knowledge, Memory, Examples, Tools, Guardrails — split into **static** (always loaded, expensive) vs **dynamic** (on-demand, pay-per-use). State that the static/dynamic boundary is "a first-class architectural decision, reviewed and versioned like code." Name **Agent Skills via progressive disclosure** (metadata at startup → full instructions on task match → deep reference only when needed) as the premier dynamic-context pattern, and note it maps directly to this repo's `.agents/skills/` model. Note: "drives `eval.md` + context guidance."

- [ ] **Step 7: Add corroborating principles + dated snapshots + cautions**

Add "## Corroborating durable principles" — each line carries source + tier:
- Workflows (predefined paths) vs agents (self-directed) — `eng-blog: Anthropic building-effective-agents`.
- Fewer, consolidated, token-efficient tools beat thin endpoint wrappers — `eng-blog: Anthropic writing-tools-for-agents`.
- Externalize & curate context; durable progress artifacts over a growing window/compaction — `eng-blog: Anthropic long-running-agents`.
- Decompose & delegate — orchestrator-workers, sub-agents-as-tools, one-feature-at-a-time — `eng-blog: Anthropic` + `paper: COMPASS / AOrchestra`.
- Judgment over generation; the **80% problem** (AI does ~80%; the 20% — edge cases, error handling, integration, subtle correctness — needs human judgment) — `whitepaper p34`.
- Co-design harness with eval; isolate harness contribution from model (the "conflation problem"); evals via labelled datasets, scoring rubrics, LM judges — `paper: arXiv 2503.16416` + `whitepaper p15`.
- **Conductor vs Orchestrator** modes (real-time/synchronous/in-IDE vs async/high-level/multi-agent) — `whitepaper p31-33`.

Add "## Dated snapshots (as of 2026-06 — illustrative, not authoritative)": Terminal-Bench 2.0 best ≈63% / frontier <65%; Top 30→Top 5 harness-only; LangChain +13.7 pts; COMPASS "up to +20%" (single unreplicated paper); specific model names (GPT-5.2, Gemini 3 Pro, Claude Opus 4.5).

Add "## Cautions (contested / refuted — warnings, NOT rules)": "Vibe coding must be replaced by explicit specifications" was **refuted 0-3**; durable position is "structure scales and amplifies culture." AOrchestra "introduces the sub-agent-as-tools paradigm" drew a dissent — it *automates* a pre-existing trend.

- [ ] **Step 8: Add primary sources block**

Add "## Primary sources" listing every URL from the spec's source list (these are the greppable citations the checklist points at):

```markdown
- **Google — The New SDLC With Vibe Coding** (PRIMARY SPINE, vendored PDF): https://www.kaggle.com/whitepaper-the-new-SDLC-with-vibe-coding
- Google — Spec-Driven Production-Grade Development: https://www.kaggle.com/whitepaper-spec-driven-production-grade-development-in-the-age-of-vibe-coding
- Anthropic — Building Effective Agents: https://www.anthropic.com/research/building-effective-agents
- Anthropic — Writing Tools for Agents: https://www.anthropic.com/engineering/writing-tools-for-agents
- Anthropic — Effective Harnesses for Long-Running Agents: https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents
- Anthropic — Effective Context Engineering: https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- COMPASS: https://arxiv.org/pdf/2510.08790
- AOrchestra: https://arxiv.org/pdf/2602.03786
- Survey on Evaluation of LLM-based Agents (conflation problem): https://arxiv.org/html/2503.16416v2
- Terminal-Bench 2.0: https://arxiv.org/html/2601.11868v1
```

Vendored copy: `docs/superpowers/context/the-new-sdlc-with-vibe-coding-google-2026.pdf`.

- [ ] **Step 9: Run the verification and confirm it passes**

Paste CHECK-FM into the shell, then:

```bash
cd /home/kalen/rust-agent-runtime
F=.agents/skills/harness-engineering/SKILL.md
check_fm "$F"
echo "--- required content ---"
grep -q 'Agent = Model + Harness' "$F" && echo "OK north-star" || echo "MISS north-star"
grep -q 'configuration failures' "$F" && echo "OK lead-quote" || echo "MISS lead-quote"
for p in audit.md reference.md build.md eval.md; do grep -q "$p" "$F" && echo "OK routes $p" || echo "MISS routes $p"; done
grep -qi 'Harness Anatomy' "$F" && echo "OK spineA" || echo "MISS spineA"
grep -qi 'static' "$F" && grep -qi 'dynamic' "$F" && echo "OK spineB static/dynamic" || echo "MISS spineB"
grep -qi 'progressive disclosure' "$F" && echo "OK progressive-disclosure" || echo "MISS progressive-disclosure"
grep -q '80% problem' "$F" && echo "OK 80pct" || echo "MISS 80pct"
grep -qi 'conflation problem' "$F" && echo "OK conflation" || echo "MISS conflation"
grep -qi 'refuted' "$F" && echo "OK cautions" || echo "MISS cautions"
for u in anthropic.com/research/building-effective-agents anthropic.com/engineering/writing-tools arxiv.org/pdf/2510.08790 arxiv.org/html/2503.16416 kaggle.com/whitepaper-the-new-SDLC; do grep -q "$u" "$F" && echo "OK src $u" || echo "MISS src $u"; done
```

Expected: all `OK`, `PASS: … frontmatter valid`. Fix any `MISS` before continuing.

- [ ] **Step 10: Commit**

```bash
git add .agents/skills/harness-engineering/SKILL.md
git commit -m "docs(skill): harness-engineering router + shared knowledge base (whitepaper spines, cited)"
```

---

## Task 2: `audit.md` — operational auditor

The teeth of the skill. Walks the six Harness Anatomy components against the runtime, emits cited findings + ranked fixes, **never edits code**.

**Files:**
- Create: `.agents/skills/harness-engineering/audit.md`
- Reference: verified anchor paths (below), `SKILL.md` (Spine A)
- Test: inline shell (CHECK-PATHS + content greps)

**Interfaces:**
- Consumes: `SKILL.md`'s Spine A component list and the "audit.md walks all six" contract.
- Produces: a documented finding schema `{ severity, file:line, violated principle + source, concrete proposed fix }` and a "top 3 highest-leverage fixes" output contract that the success criteria check against.

- [ ] **Step 1: Write the failing verification**

```bash
cd /home/kalen/rust-agent-runtime
F=.agents/skills/harness-engineering/audit.md
test -f "$F" || echo "FAIL: $F missing"
```

- [ ] **Step 2: Run it to confirm it fails**

Expected: `FAIL: .agents/skills/harness-engineering/audit.md missing`.

- [ ] **Step 3: Write the auditor procedure + non-negotiable boundary**

Write `audit.md` with a top banner stating the boundary verbatim:

```markdown
# audit.md — harness auditor

**REPORTS ONLY. NEVER EDITS CODE.** Emit findings + ranked fixes; the human holds
the judgment gate (research principle: judgment over generation).

For each of the six Harness Anatomy components (Spine A in `SKILL.md`):
1. **Open and re-read** the anchored file(s) live — do not trust remembered internals; anchors drift.
2. Judge conformance against that component's checklist items + the corroborating principles.
3. Emit a finding: `{ severity, file:line, violated principle + source, concrete proposed fix }`.

Close with **"Top 3 highest-leverage fixes"** — ranked, each naming the component, the file:line, and the one-line fix.
```

- [ ] **Step 4: Add the verified anchor table**

Add the component→anchor map using **verified `agent/`-prefixed paths** (these all exist as of 2026-06-30; the playbook opens them fresh at audit time):

```markdown
| Harness component | Runtime anchor(s) to open |
|---|---|
| 1. Instructions & Rule Files | `.agents/skills/`, root `CLAUDE.md`, sub-agent prompts (no root `AGENTS.md` exists — audit whatever rule files are present) |
| 2. Tools | `agent/crates/agent-tools/src/tool.rs`, `agent/crates/agent-tools/src/types.rs`, `agent/crates/agent-core/src/context_tools.rs` |
| 3. Sandboxes & execution | tool-execution path in `agent/crates/agent-tools/`, `agent/crates/agent-server/src/runtime.rs` |
| 4. Orchestration logic | `agent/crates/agent-core/src/loop_.rs` (incl. parallel tool calls), `agent/crates/agent-model/src/protocol.rs` |
| 5. Guardrails / Hooks | permission/limit checks across `agent-core`/`agent-server`; `agent/crates/agent-runtime-config/src/lib.rs` |
| 6. Observability | logging/metering across `agent-server`/`agent-core`; offload/eval surfaces |
| (Context engineering — Spine B) | `agent/crates/agent-core/src/context.rs`, `agent/crates/agent-core/src/context_tools.rs`, `agent/crates/agent-core/src/offload.rs`, `agent/crates/agent-core/src/offload_policy.rs` |
```

- [ ] **Step 5: Add the "thinly-sourced → judge locally" clause + severity rubric**

Add a paragraph: components the external research thinly sourced (error-recovery/retry, guardrails/permission tiers, parallel tool execution) are audited **against the runtime's own existing patterns** (e.g. `agent/crates/agent-core/src/loop_.rs` already runs parallel tool calls; config limits in `agent/crates/agent-runtime-config/src/lib.rs`) as the local reference — marked "judge from first principles + this runtime's conventions," NOT asserting external authority. Add a 3-level severity rubric (e.g. `high` = correctness/safety gap, `med` = leverage/efficiency gap, `low` = polish) so findings rank consistently.

- [ ] **Step 6: Run the verification and confirm it passes**

Paste CHECK-PATHS into the shell, then:

```bash
cd /home/kalen/rust-agent-runtime
F=.agents/skills/harness-engineering/audit.md
check_paths "$F"
grep -qi 'NEVER EDITS' "$F" && echo "OK boundary" || echo "MISS boundary"
grep -qi 'Top 3' "$F" && echo "OK top3" || echo "MISS top3"
grep -q 'severity' "$F" && grep -q 'file:line' "$F" && grep -q 'proposed fix' "$F" && echo "OK finding-schema" || echo "MISS finding-schema"
n=$(grep -cE '^\| [0-9]\.' "$F"); [ "$n" -ge 6 ] && echo "OK six components ($n)" || echo "MISS components ($n)"
grep -qi 'parallel tool' "$F" && echo "OK local-ref clause" || echo "MISS local-ref clause"
```

Expected: `PASS: all repo paths in … resolve` and all `OK`. Fix any `MISS`.

- [ ] **Step 7: Commit**

```bash
git add .agents/skills/harness-engineering/audit.md
git commit -m "docs(skill): harness-engineering audit playbook — six-component walk, verified anchors, report-only"
```

---

## Task 3: `reference.md` — advisor / lookup

Answers a harness-design question from the two spines + corroborating principles, always distinguishing a durable principle from a dated specific, and surfacing the relevant caution when a question touches contested ground.

**Files:**
- Create: `.agents/skills/harness-engineering/reference.md`
- Reference: `SKILL.md` (both spines, cautions, sources)
- Test: inline shell (content greps)

**Interfaces:**
- Consumes: `SKILL.md`'s vocabulary (`Agent = Model + Harness`, six components, static/dynamic context, Conductor vs Orchestrator) and the cautions list.
- Produces: a stable answer contract (durable-vs-dated tagging) referenced by the success criteria.

- [ ] **Step 1: Write the failing verification**

```bash
cd /home/kalen/rust-agent-runtime
F=.agents/skills/harness-engineering/reference.md
test -f "$F" || echo "FAIL: $F missing"
```

- [ ] **Step 2: Run it to confirm it fails**

Expected: `FAIL: … reference.md missing`.

- [ ] **Step 3: Write the advisor procedure**

Write `reference.md`:

```markdown
# reference.md — harness advisor

Given a harness-design question, answer from the two spines + corroborating
principles in `SKILL.md`, with citations. For every claim, tag it:

- **[DURABLE]** — a principle with a source URL + tier. Safe to build on.
- **[DATED as of 2026-06]** — a specific number/model/benchmark. Verify before relying.

Frame answers in the whitepaper's vocabulary: `Agent = Model + Harness`, the six
Harness Anatomy components, static vs dynamic context, Conductor vs Orchestrator
working modes. When a question touches contested ground, surface the relevant
**Caution** (e.g. "specs must replace vibe coding" was refuted 0-3) rather than
asserting it as a rule.
```

- [ ] **Step 4: Add worked examples**

Add 2-3 short worked Q→A examples that model the tagging and vocabulary, e.g.:
- Q: "Should I split this into sub-agents?" → cite orchestrator-workers / sub-agents-as-tools [DURABLE], name the Conductor-vs-Orchestrator tradeoff, note COMPASS "+20%" is [DATED, single unreplicated paper].
- Q: "Should we mandate specs over vibe coding?" → surface the refuted-0-3 caution; durable position is "structure scales and amplifies culture."
- Q: "How many tools should this agent expose?" → "fewer, consolidated, token-efficient tools" [DURABLE: writing-tools-for-agents].

- [ ] **Step 5: Run the verification and confirm it passes**

```bash
cd /home/kalen/rust-agent-runtime
F=.agents/skills/harness-engineering/reference.md
grep -q 'DURABLE' "$F" && grep -qi 'DATED' "$F" && echo "OK tagging" || echo "MISS tagging"
grep -qi 'Conductor' "$F" && grep -qi 'Orchestrator' "$F" && echo "OK modes" || echo "MISS modes"
grep -qi 'refuted' "$F" && echo "OK caution" || echo "MISS caution"
grep -q 'Agent = Model + Harness' "$F" && echo "OK vocab" || echo "MISS vocab"
```

Expected: all `OK`. Fix any `MISS`.

- [ ] **Step 6: Commit**

```bash
git add .agents/skills/harness-engineering/reference.md
git commit -m "docs(skill): harness-engineering reference playbook — durable-vs-dated advisor"
```

---

## Task 4: `build.md` — build-or-refactor driver (advisory)

Supplies named patterns for building/refactoring a harness component, invokes the 80% problem, then **hands off to `writing-plans` / TDD**. Does not drive edits.

**Files:**
- Create: `.agents/skills/harness-engineering/build.md`
- Reference: `SKILL.md` (corroborating principles), `superpowers:writing-plans` / `superpowers:test-driven-development` (handoff targets)
- Test: inline shell (content greps)

**Interfaces:**
- Consumes: `SKILL.md`'s corroborating principles (the named patterns) and the 80% problem.
- Produces: an explicit handoff contract to `superpowers:writing-plans` + TDD (checked by grep).

- [ ] **Step 1: Write the failing verification**

```bash
cd /home/kalen/rust-agent-runtime
F=.agents/skills/harness-engineering/build.md
test -f "$F" || echo "FAIL: $F missing"
```

- [ ] **Step 2: Run it to confirm it fails**

Expected: `FAIL: … build.md missing`.

- [ ] **Step 3: Write the advisory driver**

Write `build.md`:

```markdown
# build.md — build/refactor advisor (advisory only)

**This playbook advises; it does not edit.** It supplies patterns, marks where
human judgment must stay in the loop, then hands off to the normal
`superpowers:writing-plans` → `superpowers:test-driven-development` flow.

When building/refactoring a harness component, reach for the matching pattern:

| Building… | Pattern (source in `SKILL.md`) |
|---|---|
| sub-agent orchestration | orchestrator-workers; sub-agents-as-tools; one-feature-at-a-time |
| long-horizon execution | externalized progress artifact over a growing window/compaction |
| tool surface | fewer, consolidated, token-efficient tools; not thin endpoint wrappers |
| context loadout | static vs dynamic split; progressive-disclosure skills |
| workflow vs agent choice | predefined code paths (workflow) vs self-directed (agent) |

**The 80% problem:** AI gets ~80%; the remaining 20% — edge cases, error
handling, integration, subtle correctness — needs human judgment. Name, in the
plan, exactly where that 20% lives for this component and keep a human on it.

**Handoff:** stop here and invoke `superpowers:writing-plans` to turn the chosen
patterns into a step-by-step plan, then implement under
`superpowers:test-driven-development`. This playbook does not drive edits itself.
```

- [ ] **Step 4: Run the verification and confirm it passes**

```bash
cd /home/kalen/rust-agent-runtime
F=.agents/skills/harness-engineering/build.md
grep -qi 'advis' "$F" && grep -qi 'does not edit' "$F" && echo "OK advisory" || echo "MISS advisory"
grep -q '80% problem' "$F" && echo "OK 80pct" || echo "MISS 80pct"
grep -q 'writing-plans' "$F" && echo "OK handoff" || echo "MISS handoff"
grep -qi 'orchestrator-workers' "$F" && grep -qi 'sub-agents-as-tools' "$F" && echo "OK patterns" || echo "MISS patterns"
grep -qi 'progressive-disclosure' "$F" && echo "OK static/dynamic" || echo "MISS static/dynamic"
```

Expected: all `OK`. Fix any `MISS`.

- [ ] **Step 5: Commit**

```bash
git add .agents/skills/harness-engineering/build.md
git commit -m "docs(skill): harness-engineering build playbook — advisory patterns, 80% problem, writing-plans handoff"
```

---

## Task 5: `eval.md` — eval co-design (self-contained)

How to build trajectory/process evals that isolate harness contribution from model capability (the conflation problem). References `context-evolve` as a living instance but stands alone.

**Files:**
- Create: `.agents/skills/harness-engineering/eval.md`
- Reference: `SKILL.md` (conflation principle, source arXiv 2503.16416), `.agents/skills/context-evolve/SKILL.md` (living example)
- Test: inline shell (CHECK-PATHS + content greps)

**Interfaces:**
- Consumes: `SKILL.md`'s conflation-problem principle + source.
- Produces: a self-contained eval procedure; cross-references `context-evolve` by its real skill path.

- [ ] **Step 1: Write the failing verification**

```bash
cd /home/kalen/rust-agent-runtime
F=.agents/skills/harness-engineering/eval.md
test -f "$F" || echo "FAIL: $F missing"
```

- [ ] **Step 2: Run it to confirm it fails**

Expected: `FAIL: … eval.md missing`.

- [ ] **Step 3: Write the eval co-design procedure**

Write `eval.md` covering, self-contained:

```markdown
# eval.md — harness eval co-design

**Goal: isolate the harness's contribution from the model's (the "conflation
problem," arXiv 2503.16416).** A score gain means nothing if you can't say whether
the harness or the model earned it. Hold the model fixed; vary only the harness.

- **Outcome vs process scoring.** Outcome = did the final state pass? Process =
  did the trajectory take the right steps? Harness quality shows up in *process*.
- **Reference-based trajectory comparison** against a gold trajectory: exact /
  partial / unordered / subset match, chosen to fit how strict the step order is.
- **The agentic eval loop:** a while-loop wrapping alternating LLM + tool calls,
  driving a frozen task to termination under one harness config, emitting a
  machine-checkable result line.
- **Correctness-gated / token-tiebreak promotion:** correctness is a hard gate;
  among correctness-preserving configs, prefer fewer total tokens. Never trade a
  pass for tokens.

**Living instance:** `.agents/skills/context-evolve/` already runs exactly this
loop (`eval_context` → RunResult lines → `eval_gate`, correctness-gated /
token-tiebreak) on the context subsystem. Read it as a concrete example — but this
playbook stands alone and applies to any harness component.
```

- [ ] **Step 4: Run the verification and confirm it passes**

```bash
cd /home/kalen/rust-agent-runtime
F=.agents/skills/harness-engineering/eval.md
check_paths "$F"
grep -qi 'conflation' "$F" && echo "OK conflation" || echo "MISS conflation"
grep -qi 'outcome' "$F" && grep -qi 'process' "$F" && echo "OK outcome/process" || echo "MISS outcome/process"
grep -qi 'trajectory' "$F" && echo "OK trajectory" || echo "MISS trajectory"
grep -qi 'correctness-gated' "$F" && echo "OK gate" || echo "MISS gate"
grep -q 'context-evolve' "$F" && echo "OK cross-ref" || echo "MISS cross-ref"
```

Expected: `PASS` + all `OK`. Fix any `MISS`.

- [ ] **Step 5: Commit**

```bash
git add .agents/skills/harness-engineering/eval.md
git commit -m "docs(skill): harness-engineering eval playbook — conflation problem, trajectory evals, correctness-gated"
```

---

## Task 6: Whole-skill integration check + memory update

Final gate: verify the five files hang together (every cited repo path resolves, cross-references name real skills, all four playbooks are routed and present), then update the resume memory.

**Files:**
- Modify (only if the check finds a defect): any of the five skill files
- Modify: `/home/kalen/.claude/projects/-home-kalen-rust-agent-runtime/memory/harness-engineering-skill.md` and its `MEMORY.md` index line
- Test: inline shell (whole-skill sweep)

**Interfaces:**
- Consumes: all five files from Tasks 1-5.
- Produces: a green whole-skill report; updated resume memory pointing at the built skill.

- [ ] **Step 1: Run the whole-skill sweep**

Paste CHECK-FM and CHECK-PATHS into the shell, then:

```bash
cd /home/kalen/rust-agent-runtime
D=.agents/skills/harness-engineering
echo "--- all files present ---"
for f in SKILL.md audit.md reference.md build.md eval.md; do test -f "$D/$f" && echo "OK $f" || echo "MISS $f"; done
echo "--- frontmatter (router) ---"
check_fm "$D/SKILL.md"
echo "--- repo paths resolve in every file ---"
for f in "$D"/*.md; do check_paths "$f"; done
echo "--- router routes all four playbooks ---"
for p in audit.md reference.md build.md eval.md; do grep -q "$p" "$D/SKILL.md" && echo "OK routes $p" || echo "MISS routes $p"; done
echo "--- cross-referenced skills exist ---"
for s in context-evolve context-management; do test -d ".agents/skills/$s" && echo "OK skill $s" || echo "MISS skill $s"; done
```

Expected: all `OK` / `PASS`. If any `MISS`/`FAIL`, fix the offending file, re-run this step, then commit the fix with `docs(skill): fix harness-engineering …` before proceeding.

- [ ] **Step 2: Manual self-review against the spec**

Open the spec (`docs/superpowers/specs/2026-06-30-harness-engineering-skill-design.md`) and confirm each Success Criterion has a home:
- Triggers on harness asks + routes → SKILL.md description + routing map (Task 1).
- `audit.md` produces cited ranked gap report, no edits → Task 2 boundary + finding schema + top-3.
- Durable principles carry source+tier; dated stamped; refuted as cautions → Task 1 Steps 7-8.
- Usable with no dependency on `context-evolve`/`context-management` → cross-refs are illustrative only (Tasks 3,5).

Note any gap in the commit message if a fix is needed; otherwise proceed.

- [ ] **Step 3: Update the resume memory**

Rewrite `/home/kalen/.claude/projects/-home-kalen-rust-agent-runtime/memory/harness-engineering-skill.md` to reflect completion: change status to "BUILT — skill live at `.agents/skills/harness-engineering/` (SKILL.md + audit/reference/build/eval); plan at `docs/superpowers/plans/2026-06-30-harness-engineering-skill.md`." Keep the settled-design and source-anchor notes. Update the `description:` frontmatter to drop "RESUME POINT — next step writing-plans."

Update its one-line pointer in `MEMORY.md` to match the new hook.

- [ ] **Step 4: Final commit**

```bash
cd /home/kalen/rust-agent-runtime
git add .agents/skills/harness-engineering/
git commit -m "docs(skill): harness-engineering — whole-skill integration verified" --allow-empty
```

(The `--allow-empty` covers the case where Steps 1-2 found no defects to fix; the memory files live outside the repo and are not committed.)

---

## Self-Review (performed by the plan author)

**1. Spec coverage.** Every spec section maps to a task:
- North-star framing → Task 1 Step 3. Purpose/thesis → Task 1.
- Scope decisions 1-4 (one skill/four playbooks; self-contained; auditor no-edit; build advisory) → Global Constraints + Tasks 1,2,4.
- Structure & placement (5 files) → Tasks 1-5, one file each.
- Shared knowledge base (Spine A, Spine B, corroborating, snapshots, cautions, sources) → Task 1 Steps 5-8.
- Four playbooks (audit/reference/build/eval) → Tasks 2-5.
- "What this is NOT" → enforced by Task 2 boundary (no auto-fix), Task 4 (advisory), cross-refs illustrative.
- Success criteria → Task 6 Step 2.
- Source recovery (vendored PDF) → referenced in Tasks 1, Global Constraints.
- Open follow-ups (thinly-sourced → judge locally) → Task 2 Step 5.

**2. Placeholder scan.** No "TBD"/"add appropriate…"/"similar to Task N". Each authoring step lists concrete required content and a runnable check. Prose is authored by the implementer from the spec + PDF, but every *required element* (quotes, sources, anchor paths, schema, patterns) is spelled out and grep-verified.

**3. Type consistency.** Filenames are consistent everywhere: `SKILL.md`, `audit.md`, `reference.md`, `build.md`, `eval.md`. The finding schema `{ severity, file:line, violated principle + source, concrete proposed fix }` is stated identically in the Global Constraints and Task 2. Anchor paths use the verified `agent/`-prefixed form throughout. The two verification helpers (CHECK-FM, CHECK-PATHS) are defined once and referenced by name.
