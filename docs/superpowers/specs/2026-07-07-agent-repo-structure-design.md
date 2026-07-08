# Agent-Facing Repo Structure — Design

**Date:** 2026-07-07
**Status:** Approved (brainstorm)
**Source of prescriptions:** `ai-agent-repo-structure` OKS bundle
(github.com/kwcantrell/okf-bundles, `oks/ai-agent-repo-structure/`), crawled in full
(6 areas, 17 concept files) on 2026-07-07.

## Problem

The repo predates the bundle's conventions and diverges from them in ways that have
real cost:

1. **Project skills are invisible.** The 10 skills in `.agents/skills/` are not
   discovered by Claude Code (confirmed empirically: none appear in a live session's
   available-skills list). They are reachable only by manual `Read`. The bundle
   prescribes `.claude/skills/<name>/SKILL.md` for project skills.
2. **No cross-tool entry point.** Only `CLAUDE.md` exists — no `AGENTS.md` (the open,
   cross-tool context-file standard) and no `README.md` for humans.
3. **No committed project settings.** `.claude/` holds only untracked local settings;
   no shared permission rules or `$schema`-validated settings travel with the repo.
4. **Monorepo layering unused.** Three surfaces (`agent/`, `web/`, `src-tauri/`) all
   route through the single root file; per-surface commands and gotchas sit at root.

Already compliant (unchanged): OKF bundles committed in-repo (`docs/okf/`), committed
pass/fail checks (`scripts/ci.sh`, `.githooks`, `scripts/okf_check.py`), root context
file under 200 lines and committed.

## Decisions (from brainstorm Q&A)

- **Audience: cross-tool.** `AGENTS.md` becomes the canonical agent context;
  `CLAUDE.md` shrinks to an `@AGENTS.md` import plus Claude-specific content.
- **Skills: keep `.agents/skills/` canonical, symlink from `.claude/skills/`.**
  Per-skill relative symlinks; zero duplication; both conventions served.
- **In scope:** root `README.md`, per-surface `AGENTS.md`, committed
  `.claude/settings.json`. **Out of scope:** `.mcp.json` (no repo-level MCP servers to
  share), `.claude/rules/` (root content too small to warrant it — YAGNI).
- **Content strategy: Approach A (layered).** Repo-wide facts at root; per-surface
  commands/gotchas in per-surface files; Claude-only material in `CLAUDE.md`.

## Design

### 1. Root context files

**`AGENTS.md`** (new, canonical, target <100 lines):

- Project one-liner and the three-surfaces repo map. The detailed crate table moves to
  `agent/AGENTS.md` (§2); root keeps only the surface-level map.
- SDLC: non-trivial work is brainstorm → spec (`docs/superpowers/specs/`) → plan →
  implement; small obvious fixes may skip ahead.
- Conventions: conventional commits (`type(scope): summary`), commit/push only when
  asked, changes ship with tests, CI gate `bash scripts/ci.sh` (also pre-push via
  `git config core.hooksPath .githooks`).
- Cross-cutting gotchas: two separate Cargo workspaces (`agent/`, `src-tauri/`); the
  three skill trees (§3); session traces in `~/.agent/sessions/<id>.jsonl`.

**`CLAUDE.md`** (rewritten, target ~30 lines):

- Line 1: `@AGENTS.md`.
- Claude-only section: Graphify-first (graph in `graphify-out/`, structural questions
  are graph queries before greps, vocabulary-expansion retry rule,
  EXTRACTED-vs-INFERRED evidence rule, `--update` never full rebuild,
  `graphify-best-practices` skill pointer). Claude-only because graphify is a Claude
  skill and `graphify-out/` is gitignored — cross-tool agents cannot use it.

**`README.md`** (new, human-focused): what the project is, the three surfaces,
per-surface quickstart, pointer to `agent/docs/RUNNING.md` for model-server setup, and
a pointer to `AGENTS.md` for agent-facing detail. No agent instructions.

### 2. Per-surface context

Each surface gets an `AGENTS.md` (nearest-file-wins for cross-tool agents) plus a
one-line `CLAUDE.md` stub containing exactly `@AGENTS.md`. Rationale for the stub:
Claude Code does not read `AGENTS.md` directly, but auto-loads subdirectory
`CLAUDE.md` files on demand when files there are touched — the stub bridges the two
conventions with zero drift.

| file | content |
|------|---------|
| `agent/AGENTS.md` | crate responsibility table (moved from root), cargo commands (`cargo build`, `cargo test -p <crate>`, `-p` targets this workspace only), agent-cli run line, e2e invocation (`AGENT_E2E_URL=… AGENT_E2E_MODEL=… cargo test -p agent-core --test e2e_sglang -- --ignored`), `config.example.toml` / `mcp.example.json` pointers, `docs/RUNNING.md` pointer |
| `web/AGENTS.md` | `npm test` (vitest), `npm run build` (`tsc -b && vite build`), `npm run typecheck`; stack facts: React 19, Vite, Tailwind |
| `src-tauri/AGENTS.md` | separate-Cargo-workspace warning, `npm run desktop:dev` / `desktop:build` (from repo root), GTK-deps conditional `ci.sh` leg, WebDriver e2e pointer (`auto-drive-tauri` skill) |

Root keeps only repo-wide truths; anything surface-specific lives with the surface.

### 3. Skills discovery fix

- Create `.claude/skills/` with a per-skill **relative** symlink for each of the 10
  skills in `.agents/skills/` (e.g. `.claude/skills/tauri -> ../../.agents/skills/tauri`).
  Symlinks are committed to git.
- `.agents/skills/` remains canonical: **author there, then add the symlink**. Never
  author under `.claude/skills/`.
- Frontmatter audit of all 10 `SKILL.md` files against the format rules: frontmatter
  block first in the file; `name` ≤64 chars, lowercase/digits/hyphens, must not
  contain "anthropic" or "claude"; `description` non-empty, ≤1024 chars, states what
  the skill does **and** when to use it. Malformed frontmatter loads with empty
  metadata (silently undiscoverable), so failures are fixed as part of this work.
- `AGENTS.md` documents the three trees: `.agents/skills/` (canonical, author here),
  `.claude/skills/` (symlinks only), `<workspace>/.agent/skills` + `~/.agent/skills`
  (the runtime's own agent — unrelated to working on this repo).

### 4. Deterministic skills lint

New `scripts/skills_lint.py`, wired into `scripts/ci.sh`:

- Every `.agents/skills/<name>/SKILL.md` parses with a valid frontmatter block and
  legal `name`/`description` per §3 rules.
- Every `.agents/skills/<name>/` has a matching `.claude/skills/<name>` symlink
  pointing at it; no dangling or stray entries under `.claude/skills/`.
- Exit non-zero with a per-violation message (pass/fail check per the bundle's
  determinism practice).

### 5. Committed project settings

New `.claude/settings.json` (committed):

- `"$schema": "https://json.schemastore.org/claude-code-settings.json"`.
- `permissions.allow` limited to obviously-safe project commands: cargo
  build/test/fmt/clippy, npm test/typecheck/build, `bash scripts/ci.sh`, read-only git
  (`status`/`log`/`diff`/`show`). No `deny`/`ask` rules, no hooks (the CI gate is
  already a git pre-push hook). `settings.local.json` remains personal/untracked.
- Note: project-scope allow rules take effect only in trusted workspaces — expected.

### 6. Out of scope / unchanged

`docs/okf/` and `docs/superpowers/` layouts; the runtime's own skill loading
(`agent-skills/src/registry.rs`); `graphify-out/` gitignore status; no `.mcp.json`;
no `.claude/rules/`. Existing CLAUDE.md content is redistributed, not rewritten —
wording preserved except where the split requires edits.

### 7. Risks

- **`@AGENTS.md` import or symlinked skills behave differently than documented.**
  Mitigation: empirical verification (below) is part of done, not an afterthought.
- **Windows checkouts don't materialize symlinks.** Accepted: Linux-first repo.
- **Future double-discovery** if Claude Code later reads `.agents/skills/` natively:
  symlinked duplicates of the same target are documented to load once; revisit if it
  ever double-lists.
- **Doc drift across more files.** Mitigated by single-source rule (root `CLAUDE.md`
  holds no duplicated facts, only the import + Claude-only content) and the lint for
  the skills tree.

### 8. Verification

1. `bash scripts/ci.sh` green, including the new skills lint.
2. Fresh Claude Code session in the repo: all 10 project skills appear in the
   available-skills list; root context shows AGENTS.md content (via `/context`);
   editing a file under `web/` picks up `web/` context.
3. `python3 scripts/okf_check.py` (existing) unaffected.
4. Existing test suites untouched (`cargo test`, `npm test` — no code changes).

## Implementation shape

Single branch off `main` (e.g. `feat/agent-repo-structure`), conventional commits,
roughly: (1) root files (AGENTS.md, CLAUDE.md rewrite, README.md), (2) per-surface
AGENTS.md + stubs, (3) `.claude/skills/` symlinks + frontmatter fixes, (4) skills
lint + ci.sh wiring, (5) `.claude/settings.json`. Each step leaves the repo in a
working state.
