# graphify-best-practices Skill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Author a single auto-firing skill that teaches an agent how to wield graphify well — when to reach for it, how to query it, how to read its outputs, and how to keep it cheap and fresh — complementing (not duplicating) the `/graphify` runbook.

**Architecture:** One self-contained `SKILL.md` at `.agents/skills/graphify-best-practices/SKILL.md`. Hybrid voice: a short prescriptive "non-negotiables" core followed by four pillar sections and a red-flags table. Zero bash/mechanics — every "how exactly" defers to the runbook. The deliverable is verified by checklist checks (mechanics-free, repo-agnostic, valid frontmatter, all sections present), not unit tests, because it is a documentation artifact.

**Tech Stack:** Markdown with YAML frontmatter, following the skill conventions already used by sibling skills in `.agents/skills/` (e.g. `llama-server`, `tauri`).

## Global Constraints

- **Location:** `.agents/skills/graphify-best-practices/SKILL.md` (single file, ~150–200 lines).
- **No `references/` subdir** — content stays in one file.
- **Zero mechanics:** no bash, no command pipelines, no flag syntax tables copied from the runbook. Name commands/flags in prose where needed (`--update`, `path`, `explain`, `save-result`) but defer all "how exactly" to the runbook.
- **Repo-agnostic:** no `rust-agent-runtime`, no project-specific file paths or examples.
- **Names graphify throughout** — that is the point of the skill.
- **Voice: hybrid** — prescriptive core + deeper techniques/gotchas layer.
- **Auto-fire frontmatter:** `description` triggers on querying/building/reading a knowledge graph, especially when `graphify-out/` exists; distinct from the runbook's own description so both co-activate.
- **Do not edit** the graphify runbook (`~/.claude/skills/graphify/SKILL.md`).
- Spec of record: `docs/superpowers/specs/2026-06-25-graphify-best-practices-skill-design.md`.

---

### Task 1: Author the graphify-best-practices SKILL.md

**Files:**
- Create: `.agents/skills/graphify-best-practices/SKILL.md`
- Test (verification script): inline bash checks shown in steps below (nothing committed for the checks themselves)

**Interfaces:**
- Consumes: nothing (no prior tasks).
- Produces: a complete `SKILL.md`. No downstream code depends on its symbols; downstream "consumers" are agents loading the skill.

- [ ] **Step 1: Write the verification checks (the failing test)**

Create a throwaway check script to assert the file's required properties. Run it first to confirm it fails (file absent). Save as `/tmp/check_graphify_skill.sh`:

```bash
#!/usr/bin/env bash
set -u
F=".agents/skills/graphify-best-practices/SKILL.md"
fail=0
check() { if eval "$2"; then echo "PASS: $1"; else echo "FAIL: $1"; fail=1; fi; }

check "file exists"                 "[ -f \"$F\" ]"
check "frontmatter opens"           "head -1 \"$F\" | grep -q '^---$'"
check "has name: graphify-best-practices" "grep -q '^name: graphify-best-practices$' \"$F\""
check "description mentions graphify"     "grep -A5 '^description:' \"$F\" | grep -qi 'graphify'"
check "description mentions graphify-out" "grep -A5 '^description:' \"$F\" | grep -q 'graphify-out'"
check "has Non-negotiables section"  "grep -qi '## Non-negotiables' \"$F\""
check "has When to reach for it"     "grep -qi 'reach for' \"$F\""
check "has Querying well pillar"     "grep -qi 'quer' \"$F\""
check "has Reading outputs pillar"   "grep -qi 'reading' \"$F\""
check "has Cheap & fresh pillar"     "grep -qiE 'cheap|fresh' \"$F\""
check "has Red flags table"          "grep -qi 'red flag' \"$F\""
check "mentions confidence tags"     "grep -q 'EXTRACTED' \"$F\" && grep -q 'INFERRED' \"$F\" && grep -q 'AMBIGUOUS' \"$F\""
check "mentions BFS and DFS"         "grep -q 'BFS' \"$F\" && grep -q 'DFS' \"$F\""
check "mentions vocabulary expansion" "grep -qi 'vocabular' \"$F\""
check "mentions --update"            "grep -q -- '--update' \"$F\""
check "mentions save-result"         "grep -q 'save-result' \"$F\""
check "mentions source_location"     "grep -q 'source_location' \"$F\""
# Negative checks — no mechanics, no repo references
check "no rust-agent-runtime ref"    "! grep -qi 'rust-agent-runtime' \"$F\""
check "no python -c bash blocks"     "! grep -q 'python3 -c' \"$F\" && ! grep -q 'graphify_python' \"$F\""
check "no fenced bash blocks"        "! grep -q '```bash' \"$F\""
check "length 120-260 lines"         "L=\$(wc -l < \"$F\"); [ \"\$L\" -ge 120 ] && [ \"\$L\" -le 260 ]"

[ \"$fail\" -eq 0 ] && echo \"ALL CHECKS PASSED\" || echo \"SOME CHECKS FAILED\"
exit \"$fail\"
```

- [ ] **Step 2: Run the checks to verify they fail**

Run: `bash /tmp/check_graphify_skill.sh`
Expected: `FAIL: file exists` (and subsequent checks fail), ending `SOME CHECKS FAILED`.

- [ ] **Step 3: Write the SKILL.md**

Create `.agents/skills/graphify-best-practices/SKILL.md` with this exact content:

````markdown
---
name: graphify-best-practices
description: >-
  Use when an agent is about to query, build, or read a graphify knowledge
  graph — especially when `graphify-out/` exists in the project. This is how to
  wield graphify *well*: when to reach for it, how to query it so the matcher
  actually hits, how to read its confidence tags and structure, and how to keep
  it cheap and fresh. Complements the `/graphify` runbook (which covers the
  mechanics); trigger alongside it whenever a codebase or corpus question could
  be answered from a graph.
---

# Using graphify well

graphify turns a corpus into a persistent, queryable knowledge graph. The
`/graphify` runbook covers the mechanics — commands, flags, the build pipeline.
**This skill is judgment:** how an agent should wield that graph to actually
work faster. The single highest-leverage idea: **when a graph already exists, a
question about how the code or corpus fits together is a graph query, not a
grep.**

## Non-negotiables

Discipline to not rationalize away:

- **Graph exists → query before grep.** If `graphify-out/graph.json` is present,
  a "how does X work / what connects to Y / where does Z flow" question is a
  graph query first. Manual search ignores a pre-built relational index.
- **Expand the query against the graph's own vocabulary first.** graphify's
  matcher is case-folded substring + IDF — no stemming, no synonyms, no
  cross-language matching. Your natural wording will silently miss. Pull the
  graph's vocabulary, pick only tokens that actually exist in it, and never
  invent tokens. (The runbook has the exact expansion step.)
- **An INFERRED or AMBIGUOUS edge is a lead, not a fact.** Verify at source
  before you act on it.
- **Cite `source_location`** when you state something the graph told you. For
  anything you're about to change, confirm against the live source.
- **`--update`, don't rebuild.** A full rebuild re-pays the extraction cost for
  the entire corpus; an update only touches what changed.
- **Zero hits means a vocabulary miss, not absence.** Re-expand and retry before
  concluding the thing isn't there.

## When to reach for it (and when not)

**Reach for the graph when:**
- The question is relational or structural: "what connects to X", "how does data
  flow from A to B", "what are the core abstractions here", "what depends on Y".
- You need cross-file or cross-document synthesis the grep can't assemble.
- You're onboarding to an unfamiliar codebase or corpus and will ask many such
  questions — the graph is the map you keep coming back to.

**Don't reach for it when:**
- You already know the exact file and symbol — just read it. A graph query for
  one known fact is slower and lossier than opening the file.
- You need the literal current source for something you're about to edit. The
  graph reflects the last build and can be stale; read the source.
- It's one-off trivia that a single grep answers outright.

## Querying well

- **Expand against the vocabulary first** (see Non-negotiables). This is the
  single biggest determinant of whether a query returns signal or noise.
- **Pick the traversal to the question.** BFS (the default) for "what surrounds
  X / give me broad context." DFS for "trace the path from X to Y" — following
  one chain deep rather than fanning out.
- **Use the right verb.** `path A B` answers "how are these two concepts
  connected"; `explain X` answers "what is this one node and what touches it".
  Don't write a verbose free-text query when one of these is the natural shape.
- **Seed from the report.** `GRAPH_REPORT.md` already computed the God Nodes,
  Surprising Connections, and Suggested Questions. Start from those entry points
  instead of guessing blind — they're the graph telling you where the signal is.
- **Budget the answer.** Cap the response when you want a tight result; raise the
  budget only when a first pass truncates something you actually need.
- **One concept per query.** A question spanning two subsystems is usually a
  `path` query between them, not one sprawling BFS.

## Reading the outputs

- **Respect the confidence tags.** EXTRACTED = structural and explicit
  (AST-derived — trust it as fact). INFERRED = the model judged it plausible
  (verify before acting). AMBIGUOUS = the graph itself is unsure (a lead, never
  ground truth). Never present INFERRED or AMBIGUOUS as established fact.
- **Cite `source_location`.** Every node carries where it came from. Quote it
  when you report a fact, and re-read live source before changing anything.
- **Read the structure, not just the hits.** God nodes are the core
  abstractions — where to start understanding a system. Communities are
  subsystems — the regions of the map. Surprising connections are
  cross-subsystem bridges — often the most interesting or most fragile coupling.
- **The graph only knows what it contains.** If a traversal doesn't have the
  answer, say so — don't backfill from training memory and present it as graph
  knowledge.

## Keeping it cheap and fresh

- **Code-only changes are free.** AST extraction is deterministic and costs no
  tokens; the LLM cost is only for docs, papers, and images. So re-running after
  a pure-code change is cheap — and `--update` re-extracts only what changed.
- **Don't blow away the cache.** Unchanged content files aren't re-extracted
  across runs; preserving `graphify-out/` keeps subsequent updates cheap.
- **Automate freshness on active repos.** The post-commit hook auto-rebuilds the
  AST on every commit (free); `graphify claude install` makes the graph
  always-on in sessions. Both skip doc/image changes — run `--update` by hand
  after those.
- **Close the feedback loop.** After answering a query, `save-result` writes the
  answer back into the corpus, so the next `--update` turns it into a node.
  Future queries get smarter at near-zero cost — compounding value.
- **Watch the spend.** `cost.json` tracks cumulative tokens; the report shows
  per-run cost. Glance at it before kicking off a large rebuild.
- **Know the guardrails.** The shrink-guard refuses to overwrite a larger graph
  with a smaller one — when a shrink is legitimate (you deleted files), force it
  rather than fighting it. HTML visualization warns past ~5000 nodes; let it
  aggregate to the community view instead of rendering every node.
- **Don't over-build.** A default run already produces the JSON and the report.
  Skip the Obsidian vault, wiki, and extra viz exports unless you'll actually
  use them.

## Red flags

These thoughts mean stop — you're about to waste effort or trust bad data:

| Thought | Reality |
|---------|---------|
| "I'll just grep for it" | A graph exists — query it first. |
| "Query returned nothing, so it's not there" | Vocabulary miss. Re-expand against the graph's tokens and retry. |
| "Rebuild to get fresh data" | `--update` — a rebuild re-pays extraction for the whole corpus. |
| "The graph says X, so X" | Check the confidence tag. Verify INFERRED/AMBIGUOUS at source. |
| "I need the exact current code" | Read the source — the graph reflects the last build and can be stale. |
| "Let me write one big query for this whole feature" | One concept per query; a two-subsystem question is a `path` query. |
````

- [ ] **Step 4: Run the checks to verify they pass**

Run: `bash /tmp/check_graphify_skill.sh`
Expected: every line `PASS:`, ending `ALL CHECKS PASSED`, exit 0.

If any check fails, fix the SKILL.md (not the check) and re-run until all pass.

- [ ] **Step 5: Commit**

```bash
git add .agents/skills/graphify-best-practices/SKILL.md
git commit -m "feat: add graphify-best-practices skill"
```

---

## Self-Review

**1. Spec coverage** — each spec section maps to plan content:
- Placement & shape (single file, ~150–200 lines, no references/) → Global Constraints + length check.
- Frontmatter / auto-fire → frontmatter in Step 3 + description checks in Step 1.
- Framing → "# Using graphify well" intro paragraph.
- Non-negotiables (6 rules) → "## Non-negotiables" section.
- Pillar 1 When to reach → "## When to reach for it".
- Pillar 2 Querying well → "## Querying well".
- Pillar 3 Reading outputs → "## Reading the outputs".
- Pillar 4 Cheap & fresh → "## Keeping it cheap and fresh".
- Red flags table → "## Red flags".
- Repo-agnostic / no mechanics → negative checks in Step 1.

**2. Placeholder scan** — the SKILL.md content is complete and literal; no TBD/TODO. The check script is the actual verification, not a placeholder.

**3. Type consistency** — N/A (no code symbols). Cross-checked that command names referenced (`--update`, `path`, `explain`, `save-result`, `graphify claude install`) match the runbook's actual vocabulary.
