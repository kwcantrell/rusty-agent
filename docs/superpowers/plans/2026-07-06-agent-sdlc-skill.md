# agent-sdlc Skill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a Claude-facing skill at `.agents/skills/agent-sdlc/` teaching agents how to consume and maintain the OKF knowledge bundle at `docs/okf/agent-sdlc/`.

**Architecture:** Two markdown files following the repo's existing progressive-disclosure skill convention (`SKILL.md` always loads; `authoring.md` loads only when extending the bundle), plus a one-line cross-reference added to the existing `harness-engineering` skill. No code changes.

**Tech Stack:** Markdown skill files; validation via `python3 scripts/okf_check.py` and grep checks (no cargo/npm suites apply — this is doc-only work).

**Spec:** `docs/superpowers/specs/2026-07-06-agent-sdlc-skill-design.md`

## Global Constraints

- Skill lives in `.agents/skills/` (Claude-facing tree). NEVER touch `.agent/skills/` — that tree belongs to the runtime's own agent (CLAUDE.md "Two skill trees" gotcha).
- Do NOT modify any file under `docs/okf/agent-sdlc/` — bundle content is out of scope.
- Conventional commits: `type(scope): summary`.
- `python3 scripts/okf_check.py docs/okf/agent-sdlc` must print `OK` after every task (it should trivially, since no bundle file is touched — treat any failure as a stop-and-report).
- Commit after each task; do not push.

---

### Task 1: Create `SKILL.md`

**Files:**
- Create: `.agents/skills/agent-sdlc/SKILL.md`

**Interfaces:**
- Produces: `.agents/skills/agent-sdlc/SKILL.md` whose final section links to `authoring.md` (created in Task 2 — the dangling link is expected until then).

- [ ] **Step 1: Write the file**

Create `.agents/skills/agent-sdlc/SKILL.md` with exactly this content:

````markdown
---
name: agent-sdlc
description: >-
  Use for any question about how to build, evaluate, deploy, or operate AI
  agents — evals, tool design, context engineering, multi-agent decomposition,
  memory, human-in-the-loop gates, monitoring, harness design — or how agents
  should run a software lifecycle (spec-first workflows, verification-first
  coding). Routes to the source-backed knowledge bundle at docs/okf/agent-sdlc/
  (36 first-party sources, 23 cited concepts) for evidence behind
  agent-architecture decisions, specs, and reviews. Also use when extending or
  fixing that bundle (see authoring.md).
---

# agent-sdlc

A knowledge bundle of research findings on the software development lifecycle
for AI agents lives at `docs/okf/agent-sdlc/` (OKF v0.1). It compiles **36
first-party sources** — Anthropic, Google, LangChain, and peer-reviewed
research — into **23 concept pages** where every claim carries a citation.
Use it as the evidence base for any agent-shaped design decision: don't
re-derive best practices from memory when a cited concept already answers the
question.

"Agent SDLC" means two things, and the bundle covers both:

1. **The SDLC for *building* agent products** — evals, tools, context,
   memory, deployment, monitoring.
2. **The SDLC as *run by* agents** — spec-first workflows, human-in-the-loop
   gates, verification-first coding.

If the distinction matters to your task, read
`docs/okf/agent-sdlc/comparisons/two-meanings.md` first.

## Navigation

Start at `docs/okf/agent-sdlc/index.md`. Every directory has its own
`index.md` listing each node with a one-line description — those lines are
the routing signal. Scan the index, then open only the concepts you need.

| Directory | Contents | Reach for it when… |
|---|---|---|
| `practices/` | 12 named cross-org practices | you have a design question ("how should we do evals / tools / memory / multi-agent?") — the most useful entry point |
| `phases/` | 6 lifecycle stages, design → monitoring | you want to know what a stage involves or which practices belong to it |
| `perspectives/` | per-org syntheses (Anthropic, Google, LangChain, academia) | you want one org's coherent view, or to contrast orgs |
| `comparisons/` | cross-cutting comparisons | orientation — start with `two-meanings.md` |
| `sources/` | 36 provenance nodes: condensed notes per source, each with a `resource:` URL | verifying a claim, or going deeper than a concept |

## Link convention — read this or you will chase dead paths

Intra-bundle links are **bundle-root absolute**: `/practices/foo.md` means
`docs/okf/agent-sdlc/practices/foo.md`. They are not filesystem paths and
not repo-root paths.

## Trust model

- Every concept claim carries `[n]` citation markers resolving to a
  `# Citations` section that links into `sources/`. Concepts were verified
  against their sources (conformance clean as of 2026-07-06).
- Source files are condensed notes, not the original documents. For
  load-bearing decisions, follow the citation into the source file; its
  `resource:` URL is the ultimate ground truth.
- When you use bundle material in a spec, review, or design argument, **cite
  the bundle path** (e.g.
  `docs/okf/agent-sdlc/practices/eval-driven-development.md`) so the claim
  stays traceable.

## Relationship to harness-engineering

`.agents/skills/harness-engineering/` is the *playbook* layer — how to
audit, design, build, and eval this runtime's harness. This bundle is the
*evidence* layer behind those playbooks. Doing harness work? Load both: the
playbook for procedure, the bundle for source-backed justification.

## Extending the bundle

Adding a source, adding or editing a concept, or fixing links? Read
[authoring.md](authoring.md) first — the bundle is validated by
`scripts/okf_check.py` and has strict frontmatter, citation, and link rules.
````

- [ ] **Step 2: Verify frontmatter well-formedness**

Run:

```bash
python3 - <<'EOF'
import re, sys
text = open(".agents/skills/agent-sdlc/SKILL.md").read()
assert text.startswith("---\n"), "no frontmatter opener"
end = text.find("\n---\n", 4)
assert end != -1, "no frontmatter closer"
fm = text[4:end]
assert re.search(r"^name: agent-sdlc$", fm, re.M), "name missing/wrong"
assert re.search(r"^description: >-$", fm, re.M), "description missing"
print("OK")
EOF
```

Expected: `OK`

- [ ] **Step 3: Verify factual claims against the bundle**

The file asserts counts and paths. Check them:

```bash
ls docs/okf/agent-sdlc/sources/*.md | grep -v index.md | wc -l   # expect 36
ls docs/okf/agent-sdlc/{phases,practices,perspectives,comparisons}/*.md | grep -v index.md | wc -l   # expect 23
ls docs/okf/agent-sdlc/practices/*.md | grep -v index.md | wc -l  # expect 12
ls docs/okf/agent-sdlc/phases/*.md | grep -v index.md | wc -l     # expect 6
test -f docs/okf/agent-sdlc/comparisons/two-meanings.md && test -f docs/okf/agent-sdlc/practices/eval-driven-development.md && echo paths-ok
```

Expected: `36`, `23`, `12`, `6`, `paths-ok`. If any count differs, fix the
number in `SKILL.md` to match reality (the bundle is ground truth), then
re-run.

- [ ] **Step 4: Commit**

```bash
git add .agents/skills/agent-sdlc/SKILL.md
git commit -m "feat(skills): add agent-sdlc skill routing to the OKF knowledge bundle"
```

---

### Task 2: Create `authoring.md`

**Files:**
- Create: `.agents/skills/agent-sdlc/authoring.md`

**Interfaces:**
- Consumes: the `[authoring.md](authoring.md)` link at the end of Task 1's `SKILL.md` (this task resolves it).
- Produces: `.agents/skills/agent-sdlc/authoring.md`, the maintenance playbook.

- [ ] **Step 1: Write the file**

Create `.agents/skills/agent-sdlc/authoring.md` with exactly this content:

````markdown
# Authoring in the agent-sdlc bundle

Rules for adding or editing nodes in `docs/okf/agent-sdlc/` without breaking
validation. The checker, `scripts/okf_check.py`, is deliberately stricter
than the OKF spec — follow these shapes exactly.

## Frontmatter shapes

Every non-reserved `.md` file needs YAML frontmatter with a non-empty `type`.

`Source` nodes (`sources/`):

```yaml
---
type: Source
title: Demystifying evals for AI agents
description: Anthropic material on the SDLC of AI agents.
resource: https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents
org: Anthropic
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---
```

Concept nodes — `type` is one of `Practice` (practices/), `Lifecycle Phase`
(phases/), `Perspective` (perspectives/), `Comparison` (comparisons/):

```yaml
---
type: Practice
title: Eval-driven development
description: One-line summary; this is what directory indexes display and what agents route on — make it carry the whole idea.
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---
```

## YAML gotcha: inline lists only

The checker's frontmatter parser accepts only flat `key: value` lines and
**inline** lists — `tags: [a, b]`. Block-style lists fail validation:

```yaml
tags:            # ✗ FAILS okf_check
  - building-agents
```

## Citation rule

Every concept under `phases/`, `practices/`, `perspectives/`, or
`comparisons/` MUST end with a `# Citations` heading containing at least one
link that resolves into `/sources/`. Claims in the body carry `[n]` markers
pointing at that list.

**Verify claims against the source text before writing them into a
concept** — no unsupported assertions. If a claim needs a source the bundle
lacks, add the source node first.

## Link rule

All intra-bundle links are bundle-root absolute (`/practices/foo.md` =
`docs/okf/agent-sdlc/practices/foo.md`) and must resolve to an existing file
inside the bundle. No relative links, no links escaping the bundle.

## Reserved files

- `index.md` — no frontmatter. Exception: the bundle-root `index.md` may
  declare only `okf_version`.
- `log.md` — no frontmatter.

## Workflow for any change

1. Write or edit the node (frontmatter + cited body per the rules above).
2. Update the directory `index.md` (one line: link + description). If you
   added a directory or a major entry, update the root `index.md` too.
3. Add a dated entry to `log.md` describing the change.
4. Validate:

   ```bash
   python3 scripts/okf_check.py docs/okf/agent-sdlc
   ```

   Require `OK`. It exits 1 with one error per line otherwise — fix every
   line and re-run.
````

- [ ] **Step 2: Verify every rule in the file against the checker**

`authoring.md` paraphrases `scripts/okf_check.py`. Confirm the paraphrase is
faithful:

```bash
sed -n '1,25p' scripts/okf_check.py
grep -n 'RESERVED\|CITATION_DIRS\|okf_version' scripts/okf_check.py | head
```

Expected: the docstring lists the same four checks described in
`authoring.md` (frontmatter+type, reserved-file rules with the root
`okf_version` exception, resolving intra-bundle links, `# Citations` with ≥1
`/sources/` link for the four concept dirs), and `CITATION_DIRS = {"phases",
"practices", "perspectives", "comparisons"}`. If anything in `authoring.md`
contradicts the script, fix `authoring.md` (the script is ground truth).

- [ ] **Step 3: Confirm the bundle still validates and the SKILL.md link resolves**

```bash
python3 scripts/okf_check.py docs/okf/agent-sdlc
test -f .agents/skills/agent-sdlc/authoring.md && echo link-target-ok
```

Expected: `OK` and `link-target-ok`.

- [ ] **Step 4: Commit**

```bash
git add .agents/skills/agent-sdlc/authoring.md
git commit -m "feat(skills): add agent-sdlc authoring playbook"
```

---

### Task 3: Cross-reference from harness-engineering + final verification

**Files:**
- Modify: `.agents/skills/harness-engineering/SKILL.md:40-41` (the paragraph after the "Which playbook" table)

**Interfaces:**
- Consumes: `.agents/skills/agent-sdlc/SKILL.md` and `authoring.md` from Tasks 1–2 (must exist).

- [ ] **Step 1: Add the one-line pointer**

In `.agents/skills/harness-engineering/SKILL.md`, find:

```markdown
Read only the playbook the task needs (progressive disclosure). The shared
knowledge base below is common to all four.
```

Replace with:

```markdown
Read only the playbook the task needs (progressive disclosure). The shared
knowledge base below is common to all four. For source-backed research
evidence behind any playbook, consult the agent-sdlc knowledge bundle
(`docs/okf/agent-sdlc/`) via the `agent-sdlc` skill.
```

Make no other changes to that file.

- [ ] **Step 2: Verify the edit and overall state**

```bash
grep -n "agent-sdlc" .agents/skills/harness-engineering/SKILL.md
python3 scripts/okf_check.py docs/okf/agent-sdlc
git status --short
```

Expected: exactly one grep hit block (the new sentence, lines ~40-42); `OK`;
only `.agents/skills/harness-engineering/SKILL.md` modified.

- [ ] **Step 3: Behavioral spot-check**

Dispatch a fresh subagent (Agent tool, `Explore` or general-purpose) with
exactly this prompt:

> Read /home/kalen/rust-agent-runtime/.agents/skills/agent-sdlc/SKILL.md and
> follow it to answer: how should we seed an eval dataset for an agent, and
> how big should it be to start? Cite file paths for every claim.

Expected: the answer cites `docs/okf/agent-sdlc/practices/eval-driven-development.md`
(or the source it cites) and mentions starting with ~20-50 simple tasks drawn
from real agent failures. If the subagent instead answers from memory without
bundle paths, the SKILL.md routing text is too weak — strengthen the
Navigation/Trust sections and repeat.

*(If executing inside a subagent that cannot dispatch agents, leave this step
for the coordinating session and say so in your report.)*

- [ ] **Step 4: Commit**

```bash
git add .agents/skills/harness-engineering/SKILL.md
git commit -m "docs(skills): point harness-engineering at the agent-sdlc evidence bundle"
```
