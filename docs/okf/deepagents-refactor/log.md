# Log

- 2026-07-09 — **Update** — quoted colon-containing `title:`/`description:` frontmatter values in 1 practice file for the YAML-strict checker upgrade (PyYAML). Syntax-only; no semantic change. See docs/superpowers/specs/2026-07-09-tauri-okf-bundle-design.md (Deliverable 1, gate decision E3).
- 2026-07-08 — Final consistency + completeness review (independent
  read-only pass over the whole bundle, facts and design judgments out of
  scope). Verdict: no blockers; all 8 prior corrections propagated cleanly,
  numbers and links consistent, all sources cited. Four minor fixes applied:
  stuck-detection/re-ask detail added to the current-runtime perspective
  (was asserted only in the gap table), per-row provenance note + verdict
  legend added to the gap table, a Multimodal row added as **unassessed**
  (file tools' non-text behavior was not investigated — verify during the
  relevant spec), "never panic" in refactor-priorities re-tied to the
  documented error-convention claim, and a typed-subagent-stream bullet
  added to Phase 3. Known accepted gap: streaming has no dedicated practice
  node (covered by the gap row, both perspectives, and the Phase 3 bullet).
- 2026-07-08 — Adversarial verification pass (two independent skeptical
  agents: deepagents claims vs live docs/source; runtime claims vs live
  crate source). ~30 claims checked, 8 corrections applied: middleware tail
  order (tool-exclusion appended last in code; docstring disagrees),
  backend "never raise" softened to convention, QuickJS interpreter
  attributed to the satellite `langchain_quickjs` package,
  PatchToolCalls also covers dangling/cancelled calls; runtime side —
  no sqlite-vec (plain rusqlite + in-process cosine), recall budget default
  512 not 1024, dispatch tool has per-call `role`/`tools` args, and the loop
  has a built-in one-shot re-ask retry on malformed tool calls (gap row
  changed absent → partial). Judgment-layer claims in refactor-priorities.md
  were NOT verified here — they are design decisions reserved for the
  spec-phase adversarial panel, which must treat them as unvalidated input.

- 2026-07-08 — Bundle created. Research phase for the deepagents-style
  refactor of `agent/`: five sources (deepagents docs + source repo, two
  LangChain blog posts, the current runtime), two perspectives, nine
  practices, two comparisons (gap analysis + refactor sequencing).
  deepagents facts verified against docs pages and
  `libs/deepagents/deepagents/` source on 2026-07-08 (package pre-1.0 —
  expect drift; re-verify before implementation). Current-runtime facts
  verified against live source at commit 63401b1.
