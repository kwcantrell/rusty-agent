# Examples context type — per-skill worked exemplars

**Date:** 2026-07-02
**Cluster:** harness deep audit, missing capability #2 — the **Examples** context
type (Spine B: Instructions, Knowledge, Memory, **Examples**, Tools, Guardrails),
"absent entirely — no few-shot / reference-pattern / exemplar mechanism
anywhere." This is the audit's LAST open item.
**Audit sketch (followed):** "per-skill `examples/` bundled exemplars surfaced
via the existing read_skill_file L3 path — cheapest: skills already support
bundled files, only a convention + prompt note missing."

## Invariant

Examples are **dynamic context, model-initiative, pay-per-use** — exactly the
house progressive-disclosure pattern. The static budget grows by ONE sentence
(the awareness note); everything else is loaded on demand through the existing
L1→L2→L3 machinery. A skill without an `examples/` directory behaves
byte-identically to today.

## Verified live-source facts the design rests on

- `Skill { name, description, body, dir, files }` — `files` is every non-SKILL.md
  file in the skill dir, absolute paths, sorted (`agent-skills/src/skill.rs:4-11`,
  `registry.rs:112-124`). Nested dirs (`references/`, `tasks/`) already exist as
  informal conventions; nothing collides with `examples/` (grep-verified absent).
- L1 = `list_skills` (re-scans per call; emits `- name: description`,
  `tools.rs:58`). L2 = `use_skill` (body + "## Bundled files" section listing
  **absolute** paths + read/run guidance, `tools.rs:123-134`). L3 =
  `read_skill_file(skill, path)` — takes a **relative** path, confined by
  `resolve_in_dir` (no absolute, no `..`; `guard.rs`), read uncapped
  (`tools.rs:281-345`).
- The absolute-vs-relative mismatch: L2 lists absolute paths while L3 consumes
  relative ones — the model must strip the dir prefix today.
- `compose_system_prompt` = base + `SKILLS_AWARENESS` (2 sentences) + full
  preset bodies (`presets.rs:4-25`). No skills catalog in the prompt; discovery
  is model-initiative.
- `create_skill` writes SKILL.md + bundled `files: [{path, content}]` (≤32
  files, ≤256 KB each, confined) — `examples/foo.md` already WORKS as a file
  path today; nothing teaches the convention (`tools.rs:143-278`).
- Oversized L3 reads are already bounded window-side by the 16 KiB tool-result
  ingestion cap (eager offload + recall marker) — no new caps needed.
- Tests are inline per module (42 cases); assemble-level contract ratchets cover
  registered skill tools.

## Decisions

- **H1 — Convention: `examples/` inside the skill directory.** Files under
  `<skill>/examples/` are that skill's worked exemplars (reference
  patterns / few-shot artifacts to imitate). `Skill` gains
  `examples: Vec<PathBuf>` — the subset of bundled files whose path (relative
  to the skill dir) starts with `examples/` — populated during load;
  `files` keeps containing them too (L3 reading is unchanged). Rejected:
  frontmatter-declared example lists (drifts from disk; the directory IS the
  declaration).
- **H2 — L2 surfacing: a distinct `## Examples` section in `use_skill`.** When
  `examples` is non-empty, `use_skill` emits, BEFORE the bundled-files section:

  ```
  ## Examples (worked exemplars)
  - examples/<file>
  ...
  Read one with read_skill_file and imitate its shape and conventions;
  do not copy content verbatim.
  ```

  and the generic "## Bundled files" section **excludes** the examples (no
  double-listing). Rejected: leaving examples in the flat file list (the whole
  point is that exemplars carry a distinct usage contract — imitate, don't
  copy — which a bare path can't say).
- **H3 — L1 + prompt awareness.** `list_skills` lines gain a suffix when a
  skill has examples: `- name: description [N examples]`. `SKILLS_AWARENESS`
  gains ONE sentence: "Some skills bundle worked examples — when a loaded
  skill lists them, read a relevant one before producing your first artifact
  of that kind." (The static-budget cost of this capability is exactly this
  sentence.)
- **H4 — Relative paths at L2 (folded friction fix).** `use_skill` lists BOTH
  examples and bundled files as **skill-relative** paths (the exact form
  `read_skill_file` consumes), with the guidance line updated to say so.
  Today's absolute paths force the model to strip the directory prefix.
  Because the existing guidance also says "run a bundled script with
  execute_command" — which resolves from the workspace cwd and therefore
  NEEDS an absolute path — the skill directory is stated ONCE in the section
  header ("## Bundled files (dir: <abs skill dir>)") so script execution
  keeps its root while reads use the listed relative form. This changes
  existing L2 output — affected tests updated, behavior otherwise identical.
- **H5 — Authoring: prose, not machinery.** `create_skill` keeps its `files`
  array unchanged; its tool description and the `files` param description gain
  the convention: put worked exemplars under `examples/` — they surface as a
  distinct section to consumers. Rejected: a dedicated `examples` arg
  (duplicate of `files` with a path prefix).
- **H6 — No new tools, no injection.** Rejected alternatives, recorded:
  (a) a dedicated `get_examples`/`search_examples` tool — `read_skill_file`
  already covers retrieval and "fewer consolidated tools" governs; (b)
  automatic per-task example injection into the prompt (Memory-retriever
  style) — spends static budget, needs relevance machinery, and inverts the
  model-initiative pattern the whole skills system is built on. Revisit (b)
  only with eval evidence that model-initiative loading under-triggers.
- **H7 — No new caps.** L3 reads stay uncapped tool-side; the window is
  protected by the existing ingestion cap. Example file counts are bounded by
  the existing 32-file bundle cap at authoring time (filesystem-authored
  skills are user-owned, as today).

## Sections

**1 — Model + registry (agent-skills/src/skill.rs, registry.rs).** `Skill.examples`
field; populated in `load_skill` by filtering the collected bundle on the
`examples/` prefix (path-relative, separator-safe); sorted like `files`.

**2 — Tool surfacing (tools.rs).** `use_skill`: relative-path rendering, the
`## Examples` section per H2, exclusion from bundled list, updated guidance.
`list_skills`: `[N examples]` suffix per H3. `create_skill`: description +
`files` param prose per H5. `read_skill_file`: unchanged (its when-not prose
already steers correctly).

**3 — Prompt (presets.rs).** The one-sentence H3 addition to `SKILLS_AWARENESS`.

**4 — E2E + docs.** An end-to-end test driving the full flow through the tools
against a temp skill with `examples/`: L1 shows the marker → L2 shows the
section with relative paths → L3 reads an example (and confinement still
rejects escapes). One short paragraph documenting the convention in
`agent/docs/RUNNING.md`'s skills section (or the nearest existing skills doc —
locate at implementation time).

## Error handling & edge cases

- `examples/` empty or absent → `Skill.examples` empty → zero output change
  (byte-identical L1/L2; pinned).
- A file literally named `examples` (not a dir) → it's just a bundled file
  (prefix filter matches `examples/`-the-directory only).
- Nested subdirs under `examples/` → included (prefix match), listed with
  their full relative paths.
- Path separators on non-Unix: compute the relative path via
  `strip_prefix(&skill.dir)` and compare components, not string prefixes.
- `list_skills` marker only when count > 0 — no `[0 examples]` noise.
- Preset-inlined skills (compose_system_prompt) do NOT inline examples —
  presets inline the body only, unchanged; a preset skill's examples remain
  L3-loadable like any other (document in the spec only; no code).

## Testing

- skill/registry: `examples` populated for `examples/*.md` (incl. a nested
  subdir file), empty otherwise; a plain file named `examples` not treated as
  a dir; `files` still contains everything (superset pin).
- tools: `use_skill` — section present with relative paths + guidance line,
  absent (and bundled list byte-identical-to-relative-form) when no examples;
  bundled list excludes examples; `list_skills` — `[N examples]` suffix
  present/absent; `create_skill` — schema description mentions `examples/`;
  a create_skill round-trip writing `examples/sample.md` then `use_skill`
  showing the section (authoring→consumption loop).
- presets: awareness constant contains the examples sentence (existing
  constant test extended).
- e2e (tools-level, temp dirs): L1 marker → L2 section → L3 read of the
  example content; `read_skill_file("../escape")` still Denied.
- Contract ratchets (assemble): required-param descriptions stay green with
  the updated schemas.
- `bash scripts/ci.sh` green. Zero changes outside agent-skills except docs
  (no wire/web/config/loop changes at all).

## Files touched

- `agent/crates/agent-skills/src/skill.rs`, `registry.rs`, `tools.rs`,
  `presets.rs` (+ their inline tests).
- One skills-related doc paragraph (location resolved at implementation).

## The 20% (human-judgment points)

The usage-contract wording (imitate-don't-copy) and the one-sentence awareness
budget are the judgment calls — both are cheap to revise with eval evidence.
Whether model-initiative loading actually triggers often enough is an open
empirical question (H6's revisit clause); the context-evolve harness is the
natural place to measure it later.

## Out of scope (recorded residuals)

- Automatic example retrieval/injection (H6b — revisit with eval evidence).
- A repo-shipped example corpus for the runtime's own registry dirs (the
  runtime ships no built-in skills; users author their own — the capability is
  the mechanism + convention, exercised by tests).
- Example-aware eval tasks in the context-evolve harness (natural follow-up).
- L3 read caps (window already protected by the ingestion cap).
