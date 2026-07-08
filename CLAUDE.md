@AGENTS.md

# Claude-specific

## Graphify first

A knowledge graph of this repo lives in `graphify-out/` (`graph.json`, `GRAPH_REPORT.md`).
It exists — use it.

- **A structural/relational question is a graph query before a grep.** "How does X work /
  what connects to Y / where does Z flow" → query the graph first. Manual search ignores a
  pre-built relational index.
- **Expand your query against the graph's own vocabulary first** — the matcher is case-folded
  substring + IDF, no synonyms. Zero hits means a vocabulary miss, not absence: re-expand and retry.
- **`EXTRACTED` = fact. `INFERRED`/`AMBIGUOUS` = a lead — verify at source.** Cite
  `source_location`; re-read live source before changing anything.
- **`graphify . --update`, never a full rebuild** (a rebuild re-pays extraction for the whole corpus).
  Seed from `GRAPH_REPORT.md`'s God Nodes and Suggested Questions.
- Full judgment lives in the `graphify-best-practices` skill — reach for it alongside any graph work.
- **The graph reflects the last build and can be stale.** Read live source before editing;
  `--update` after doc/image changes (code changes re-extract for free).
