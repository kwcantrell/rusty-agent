# graphify-best-practices skill — design

**Date:** 2026-06-25
**Status:** Approved (design phase)

## Goal

Create a skill that teaches an agent how to *wield* graphify well — when to
reach for it, how to query it, how to read its outputs, and how to keep it cheap
and fresh. It is a **best-practices / judgment** layer that complements the
existing `/graphify` runbook (`~/.claude/skills/graphify/SKILL.md`) rather than
duplicating its mechanics.

Non-goals: restating any bash, command syntax, or pipeline internals. Every
"how exactly" defers to the runbook. The skill names graphify throughout
(that is the whole point) but stays **repo-agnostic** — no rust-agent-runtime
references, no project-specific examples.

## Placement & shape

- **Location:** `.agents/skills/graphify-best-practices/SKILL.md`
- **Single file**, ~150–200 lines. No `references/` subdir — the content is
  principle-dense, not mechanics-dense, so it stays in one focused file. (If it
  later outgrows one file, a `references/` split is the escape hatch, but we do
  not start there.)
- **Voice: hybrid** — a short prescriptive core (non-negotiables) plus a deeper
  techniques/gotchas layer.

## Frontmatter (activation)

Auto-fire. The `description` triggers whenever an agent is about to **query,
build, or read** a knowledge graph — especially when `graphify-out/` exists —
so it rides alongside the runbook without explicit invocation. It must be
distinct enough from the runbook's own description that both can co-activate:
the runbook is "how to run graphify," this is "how to use graphify well."

We do **not** edit the graphify runbook (the user chose auto-fire over a
runbook cross-link). True complement, no upstream edits.

## Structure

### 1. Framing (2–3 sentences)
The graph is a pre-built relational index over a corpus. This skill is how to
wield it; the mechanics (commands, flags, pipeline) live in the `/graphify`
runbook. Lead with the single highest-leverage idea: when a graph exists, a
question about how the code/corpus fits together is a *graph query*, not a
grep.

### 2. Non-negotiables (prescriptive core, ~6 rules)
The discipline an agent should not rationalize away:
- **Graph exists → query before grep.** A question about how the code/corpus
  works is a graph query first; falling back to manual search ignores a
  pre-built index.
- **Always expand the query against the graph's own vocabulary first.** The
  matcher is case-folded substring + IDF — no stemming, no synonyms, no
  cross-language. Natural-language wording silently misses. Pick only tokens
  that exist in the graph; never invent tokens.
- **An INFERRED or AMBIGUOUS edge is a lead, not a fact.** Verify at source
  before acting on it.
- **Cite `source_location`** when stating a fact from the graph; verify against
  live source for anything you will change.
- **`--update`, don't rebuild.** A full rebuild re-pays for the whole corpus.
- **0 hits = vocabulary miss, not absence.** Re-expand; don't conclude the thing
  doesn't exist.

### 3. Pillar — When to reach for it (and when not)
- **Yes:** relational/structural questions ("what connects to X", "how does data
  flow from A to B", "what are the core abstractions", "what depends on Y"),
  cross-file or cross-document synthesis, onboarding to an unfamiliar
  codebase/corpus where you will ask many such questions.
- **No:** you already know the exact file/symbol (just read it — a graph query
  for one known fact is slower and lossy); you need the literal current source
  for something you will act on (the graph can be stale — verify at source);
  one-off trivia grep answers in a single shot.

### 4. Pillar — Querying well
- Expand against vocabulary first (principle stated here; exact command in the
  runbook).
- **BFS (default)** for "what surrounds X / broad context"; **DFS** for "trace
  the path from X to Y."
- Use **`path A B`** for the connection between two named concepts and
  **`explain X`** for a single node's neighborhood — don't write a verbose query
  when one of these is the right shape.
- **Seed queries from `GRAPH_REPORT.md`** — its God Nodes, Surprising
  Connections, and Suggested Questions are pre-computed entry points; start there
  instead of guessing blind.
- Use **`--budget`** to keep answers tight; raise it only when a first pass
  truncates something you need.
- One concept per query; a two-subsystem question is often a `path` query, not
  one big BFS.

### 5. Pillar — Reading outputs
- **Confidence tags:** EXTRACTED = structural/explicit (AST-derived, trust as
  fact); INFERRED = LLM-judged (plausible, verify before acting); AMBIGUOUS =
  the graph itself is unsure (a lead, never ground truth). Never present
  INFERRED/AMBIGUOUS as fact.
- **Cite `source_location`**; verify live source before changing anything.
- Read the **structure**, not just hits: god nodes = core abstractions (where to
  start understanding); communities = subsystems (the map's regions); surprising
  connections = cross-subsystem bridges (often the interesting or risky
  coupling).
- The graph answers only from what it contains. If it lacks the info, say so —
  don't fill gaps from training memory.

### 6. Pillar — Cheap & fresh
- **Code-only changes are free** (deterministic AST); semantic (LLM) cost is
  only for docs/papers/images. `--update` re-extracts only what changed.
- **Preserve the semantic cache** — unchanged content files aren't re-extracted
  across runs; don't blow it away.
- For an actively-developed repo, the **post-commit hook** (auto AST-rebuild,
  free) and/or `graphify claude install` keep the graph fresh without manual
  runs. Run `--update` manually after doc/image changes (the hook ignores
  those).
- **Close the feedback loop:** after answering a query, `save-result` writes the
  answer back so the next `--update` turns it into a node — future queries get
  smarter. Cheap and compounding.
- Watch `cost.json` for cumulative spend; the report shows per-run tokens.
- **Know the guardrails:** the shrink-guard refuses to overwrite a bigger graph
  with a smaller one (use `--force` for a legitimate shrink, e.g. after deleting
  files); HTML viz warns past 5000 nodes (use community aggregation). Don't
  fight these blindly.
- **Don't over-build:** skip `--obsidian` / `--wiki` / viz exports unless you'll
  use them; a default run already gives JSON + report.

### 7. Red flags table (superpowers house style)
Rationalization → reality. Examples:
- "I'll just grep for it" → a graph exists; query first.
- "Query returned nothing, so it's not there" → vocabulary miss; re-expand.
- "Rebuild to get fresh data" → `--update`; rebuild re-pays for the whole corpus.
- "The graph says X, so X" → check the confidence tag; verify INFERRED/AMBIGUOUS
  at source.
- "I need the exact current code" → read the source; the graph can be stale.

## Success criteria

- An agent reading this skill changes its default behavior: queries an existing
  graph before manual search, expands against vocabulary, respects confidence
  tags, and reaches for `--update` over rebuild.
- Zero mechanics duplicated from the runbook; every "how" defers to it.
- Repo-agnostic; names graphify throughout.
- Self-contained in one `SKILL.md`.
