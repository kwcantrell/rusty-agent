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

**Do not** use this skill in place of the `/graphify` runbook (mechanics —
commands, flags, builds — live there), and don't reach for the graph when the
answer is one known file: a direct read beats a graph query for single-fact
lookups.

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
