# Agent-Facing Repo Structure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure this repo's agent-facing surface per the `ai-agent-repo-structure` OKS bundle: cross-tool `AGENTS.md` files (root + per-surface), a human `README.md`, `CLAUDE.md` reduced to an `@AGENTS.md` import + Claude-only content, `.claude/skills/` symlinks so Claude Code discovers the 10 repo skills, a deterministic skills lint in CI, and committed project settings.

**Architecture:** Pure docs/config/symlink work — no Rust/TS code changes. Content is *redistributed* from the existing `CLAUDE.md` (single-source, no duplication): repo-wide facts → root `AGENTS.md`; per-surface commands/gotchas → `agent|web|src-tauri/AGENTS.md`; graphify → `CLAUDE.md`. One new Python lint script (`scripts/skills_lint.py`) with a hermetic test, wired into `scripts/ci.sh` following the existing `okf_check.py` pattern.

**Tech Stack:** Markdown, git symlinks, Python 3 stdlib (`re`, `pathlib`, `unittest` — no PyYAML; it is not installed), bash (`scripts/ci.sh`).

**Spec:** `docs/superpowers/specs/2026-07-07-agent-repo-structure-design.md`

## Global Constraints

- Root `AGENTS.md` < 100 lines; rewritten `CLAUDE.md` ~30 lines; every context file well under 200 lines.
- `CLAUDE.md` content is redistributed, not rewritten — preserve existing wording except where the split requires edits.
- `.agents/skills/` stays canonical; `.claude/skills/` holds ONLY relative symlinks (`../../.agents/skills/<name>`); never author content there.
- Skill frontmatter rules (lint enforces): frontmatter block first in file; `name` ≤64 chars of `[a-z0-9-]`, no "claude"/"anthropic", must equal the directory name; `description` non-empty, ≤1024 chars. (All 10 current skills already pass — verified 2026-07-07, max description 967 chars.)
- Conventional commits (`type(scope): summary`). Work on branch `feat/agent-repo-structure` off `main`.
- Out of scope: `.mcp.json`, `.claude/rules/`, any change to the runtime's own skill loading (`agent-skills/src/registry.rs`), `docs/okf/` and `docs/superpowers/` layouts.

---

### Task 1: Branch + per-surface AGENTS.md + CLAUDE.md stubs

Per-surface files come first so the root `AGENTS.md` (Task 2) can link to files that already exist.

**Files:**
- Create: `agent/AGENTS.md`, `agent/CLAUDE.md`
- Create: `web/AGENTS.md`, `web/CLAUDE.md`
- Create: `src-tauri/AGENTS.md`, `src-tauri/CLAUDE.md`

**Interfaces:**
- Produces: the three `<surface>/AGENTS.md` paths that root `AGENTS.md` (Task 2) links to.

- [ ] **Step 1: Create the branch**

```bash
git checkout -b feat/agent-repo-structure main
```

- [ ] **Step 2: Write `agent/AGENTS.md`**

Exact content (crate table copied verbatim from current root `CLAUDE.md`):

````markdown
# agent/ — Rust core (Cargo workspace)

The agent core. One of two Cargo workspaces in this repo (the other is
`src-tauri/`) — `cargo -p <crate>` here targets only these crates.

## Crates (`crates/`)

| crate | responsibility |
|-------|----------------|
| `agent-core` | agent loop, context manager, event model |
| `agent-model` | model client, tool-call protocols (native/prompted), inference types |
| `agent-tools` | shared tool vocabulary and the `Tool` trait |
| `agent-http` | outbound HTTP fetch tool; gates egress in-tool |
| `agent-mcp` | MCP client — connect to external MCP servers |
| `agent-memory` | long-term semantic memory (remember/recall/forget over a local vector store) |
| `agent-policy` | permission policy engine + approval channel |
| `agent-sandbox` | sandboxed tool/command execution |
| `agent-skills` | discover, load-on-demand, author, preload markdown skills |
| `agent-server` | daemon bridging the local agent to the Cloudflare Worker (browser UI backend) |
| `agent-cli` | terminal front-end binary |
| `agent-runtime-config` | shared loop wiring (tool registry, protocol picker, command lists) |

## Commands

```bash
cargo build                                  # whole workspace
cargo test -p <crate>                        # test one crate
cargo run -p agent-cli -- --backend claude-cli --model sonnet --workspace .   # run the CLI
AGENT_E2E_URL=… AGENT_E2E_MODEL=… cargo test -p agent-core --test e2e_sglang -- --ignored
```

## Config & docs

- `config.example.toml` — runtime config reference; `mcp.example.json` — MCP client config example.
- `docs/RUNNING.md` — full model-server setup (llama.cpp / SGLang / vLLM / Claude CLI).
- Session traces land in `~/.agent/sessions/<id>.jsonl` (disable with `"trace": false`
  in the runtime config).
````

- [ ] **Step 3: Write `web/AGENTS.md`**

Exact content:

````markdown
# web/ — browser SPA

React 19 / Vite / Tailwind single-page app (the Context Explorer UI). Reaches
the *local* agent through the Cloudflare Worker path (`agent-server`).

## Commands

```bash
npm test            # vitest
npm run typecheck
npm run build       # tsc -b && vite build
```
````

- [ ] **Step 4: Write `src-tauri/AGENTS.md`**

Exact content:

````markdown
# src-tauri/ — Tauri 2 desktop app

Wraps `agent-server` in a desktop shell. **A separate Cargo workspace** from
`agent/` — run cargo commands from this directory; `-p` from `agent/` cannot
reach these crates.

## Commands

From the **repo root**:

```bash
npm run desktop:dev
npm run desktop:build
```

## Gotchas

- CI (`scripts/ci.sh`) runs src-tauri clippy + tests only when GTK/WebKitGTK dev
  deps are present. Its fmt is never checked — src-tauri is hand-formatted by
  convention (compact hand-format, no `cargo fmt`).
- End-to-end GUI driving goes through WebDriver (tauri-driver + selenium) — see
  the `auto-drive-tauri` skill (`.agents/skills/auto-drive-tauri/`).
````

- [ ] **Step 5: Write the three CLAUDE.md stubs**

Each of `agent/CLAUDE.md`, `web/CLAUDE.md`, `src-tauri/CLAUDE.md` contains exactly one line (Claude Code auto-loads subdirectory `CLAUDE.md` on demand and resolves the import; it does not read `AGENTS.md` directly):

```markdown
@AGENTS.md
```

- [ ] **Step 6: Verify line counts and stub content**

Run: `wc -l agent/AGENTS.md web/AGENTS.md src-tauri/AGENTS.md && cat agent/CLAUDE.md web/CLAUDE.md src-tauri/CLAUDE.md`
Expected: every AGENTS.md < 60 lines; each stub prints exactly `@AGENTS.md`.

- [ ] **Step 7: Commit**

```bash
git add agent/AGENTS.md agent/CLAUDE.md web/AGENTS.md web/CLAUDE.md src-tauri/AGENTS.md src-tauri/CLAUDE.md
git commit -m "docs(agents): per-surface AGENTS.md + CLAUDE.md import stubs"
```

---

### Task 2: Root README.md, AGENTS.md, CLAUDE.md rewrite

**Files:**
- Create: `README.md`, `AGENTS.md`
- Modify: `CLAUDE.md` (full rewrite — import + Claude-only content)

**Interfaces:**
- Consumes: `agent/AGENTS.md`, `web/AGENTS.md`, `src-tauri/AGENTS.md` (Task 1 — link targets).
- Produces: root `AGENTS.md` referenced by every stub's `@AGENTS.md`; mentions `scripts/skills_lint.py` (created in Task 4 — forward reference within this branch is intentional).

- [ ] **Step 1: Write `README.md`**

Exact content:

````markdown
# rust-agent-runtime

A local-first LLM agent runtime. A Rust core drives a local model (llama.cpp /
SGLang / vLLM) or the Claude CLI through a tool/policy loop, exposed three ways:

- **Terminal CLI** (`agent/crates/agent-cli`)
- **Desktop app** (`src-tauri/` — Tauri 2)
- **Browser SPA** (`web/` — React 19 + Vite) that reaches your *local* agent
  through a Cloudflare Worker

## Quickstart

Rust core (needs a model server or the Claude CLI — see
[`agent/docs/RUNNING.md`](agent/docs/RUNNING.md)):

```bash
cd agent
cargo build
cargo run -p agent-cli -- --backend claude-cli --model sonnet --workspace .
```

Web UI: `cd web && npm install && npm run dev`

Desktop app (from repo root): `npm install && npm run desktop:dev`

## Layout

| path | what |
|------|------|
| `agent/` | Rust Cargo workspace — agent core, tools, policy, sandbox, memory, skills, server, CLI |
| `src-tauri/` | Tauri 2 desktop wrapper (its own Cargo workspace) |
| `web/` | React SPA (Context Explorer UI) |
| `docs/` | specs, plans, audits (`docs/superpowers/`), knowledge bundles (`docs/okf/`) |

Working on this repo with an AI agent? Start at [`AGENTS.md`](AGENTS.md).

## License

[MIT](LICENSE)
````

- [ ] **Step 2: Write root `AGENTS.md`**

Exact content (wording carried from current `CLAUDE.md` wherever possible):

````markdown
# rust-agent-runtime — agent guide

A local-first LLM agent runtime: a Rust core drives a local model (or the Claude CLI)
through a tool/policy loop, exposed three ways — a terminal CLI, a Tauri desktop app,
and a browser SPA that reaches your *local* agent via a Cloudflare Worker.

## Repo map

Three surfaces, one agent core — each surface has its own `AGENTS.md` with commands
and surface-local gotchas:

- **[`agent/`](agent/AGENTS.md)** — Rust Cargo workspace (the core); crate map inside.
- **[`src-tauri/`](src-tauri/AGENTS.md)** — Tauri 2 desktop app wrapping `agent-server`.
  Its own separate Cargo workspace.
- **[`web/`](web/AGENTS.md)** — React 19 / Vite / Tailwind SPA (the Context Explorer UI).
- **Cloud path** — `agent-server` dials a Cloudflare Worker so a browser can drive
  the local agent.

## How we work

Non-trivial work follows the superpowers SDLC — **don't jump straight to code**:

**brainstorm → spec (`docs/superpowers/specs/`) → plan (`docs/superpowers/plans/`) →
implement.** Small, obvious fixes can skip ahead, but design-bearing changes get a
spec first.

## Conventions

- **Conventional commits**: `type(scope): summary` (e.g. `fix(memory): …`), matching
  existing history.
- **Commit and push only when asked.** Branch off `main` for PRs.
- **Changes ship with tests.** Run the relevant suite (`cargo test` / `npm test`)
  before calling it done.

## CI gate

```bash
bash scripts/ci.sh   # okf check + skills lint + fmt + clippy + cargo test (agent/) + conditional src-tauri + web typecheck/vitest
```

Also runs as a pre-push hook — enable once per clone with
`git config core.hooksPath .githooks`.

## Gotchas

- **Two separate Cargo workspaces** — `agent/` and `src-tauri/`. `-p <crate>` must
  target the right one.
- **Three skill trees — author in the right one:**
  - `.agents/skills/` — canonical home of skills for working *on* this repo. Author here.
  - `.claude/skills/` — relative symlinks into `.agents/skills/` so Claude Code
    discovers them. Never author here; add a symlink for each new skill
    (`scripts/skills_lint.py` enforces both).
  - `<workspace>/.agent/skills` and `~/.agent/skills` — loaded by the runtime's *own*
    agent (`agent-skills/src/registry.rs`). Unrelated to working on this repo.
````

- [ ] **Step 3: Rewrite root `CLAUDE.md`**

Exact content — line 1 is the import; the Graphify section is carried **verbatim** from the current `CLAUDE.md` (it is Claude-only: graphify is a Claude skill and `graphify-out/` is gitignored):

````markdown
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
````

- [ ] **Step 4: Verify sizes and that no root-CLAUDE.md fact was dropped**

Run: `wc -l README.md AGENTS.md CLAUDE.md`
Expected: `AGENTS.md` < 100, `CLAUDE.md` < 40.

Then diff the OLD `CLAUDE.md` (from `git show main:CLAUDE.md`) section-by-section against the new file set. Every fact must appear in exactly one place:

| old CLAUDE.md section | new home |
|---|---|
| intro paragraph | root `AGENTS.md` intro |
| Repo map (surfaces) | root `AGENTS.md` § Repo map |
| Crate table | `agent/AGENTS.md` |
| Graphify first (incl. stale-graph gotcha) | root `CLAUDE.md` |
| How we work | root `AGENTS.md` |
| Commands: cargo | `agent/AGENTS.md` |
| Commands: web | `web/AGENTS.md` |
| Commands: desktop | `src-tauri/AGENTS.md` |
| Commands: ci.sh + hooksPath | root `AGENTS.md` § CI gate |
| Session traces | `agent/AGENTS.md` § Config & docs |
| RUNNING.md pointer | `agent/AGENTS.md` + `README.md` |
| Conventions (3 bullets) | root `AGENTS.md` § Conventions |
| Gotcha: two workspaces | root `AGENTS.md` § Gotchas |
| Gotcha: stale graph | root `CLAUDE.md` (graphify section) |
| Gotcha: two skill trees | root `AGENTS.md` § Gotchas (now *three* trees) |

Run: `git show main:CLAUDE.md | grep -oE '\`[^\`]+\`' | sort -u > /tmp/claude-1000/-home-kalen-rust-agent-runtime/cf4b3adb-8b94-450a-94e6-d17fbd9e8fa3/scratchpad/old-tokens.txt` and spot-check that each token (crate names, commands, paths) appears in one of the new files (`grep -rF "<token>" AGENTS.md CLAUDE.md README.md agent/AGENTS.md web/AGENTS.md src-tauri/AGENTS.md`).

- [ ] **Step 5: Commit**

```bash
git add README.md AGENTS.md CLAUDE.md
git commit -m "docs(agents): root AGENTS.md canonical, CLAUDE.md -> import + graphify, human README"
```

---

### Task 3: `.claude/skills/` symlinks + .gitignore hygiene

**Files:**
- Create: `.claude/skills/<name>` symlink for each of the 10 dirs under `.agents/skills/`
- Modify: `.gitignore` (append `.claude` local-state entries)

**Interfaces:**
- Produces: the symlink layout that `scripts/skills_lint.py` (Task 4) validates: `.claude/skills/<name> -> ../../.agents/skills/<name>` (relative, one per skill).

- [ ] **Step 1: Create the symlinks**

```bash
mkdir -p .claude/skills
for d in .agents/skills/*/; do
  n=$(basename "$d")
  ln -s "../../.agents/skills/$n" ".claude/skills/$n"
done
ls -l .claude/skills
```

Expected: exactly 10 symlinks (agent-sdlc, auto-drive-tauri, context-evolve, context-management, graphify-best-practices, harness-engineering, harness-evolve, llama-server, tauri, wayland), each showing `-> ../../.agents/skills/<name>`.

- [ ] **Step 2: Verify the links resolve**

Run: `for l in .claude/skills/*; do [ -f "$l/SKILL.md" ] || echo "BROKEN: $l"; done; echo done`
Expected: prints only `done`.

- [ ] **Step 3: Append local-state ignores to `.gitignore`**

The repo currently relies on the developer's *global* git ignore for `.claude/settings.local.json` — that does not travel with the repo. Append this block to `.gitignore`:

```gitignore
# Claude Code local state. NOTE: .claude/skills/ (symlinks) and
# .claude/settings.json ARE committed — never ignore .claude/ wholesale.
.claude/settings.local.json
.claude/scheduled_tasks.lock
.claude/worktrees/
```

- [ ] **Step 4: Verify git sees symlinks, not contents**

Run: `git add .claude/skills .gitignore && git status --short && git diff --cached --stat | tail -3`
Expected: 10 new `.claude/skills/<name>` entries (mode 120000 symlinks — each diffs as one line of link-target text, not file contents) plus `.gitignore`; nothing from `.agents/` re-staged.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(skills): expose .agents/skills to Claude Code via .claude/skills symlinks"
```

---

### Task 4: Skills lint (TDD) + ci.sh wiring

**Files:**
- Create: `scripts/skills_lint.py`
- Test: `scripts/test_skills_lint.py`
- Modify: `scripts/ci.sh` (add a leg after the okf check, lines 11–13)

**Interfaces:**
- Consumes: the symlink layout from Task 3.
- Produces: `skills_lint.lint(root: Path) -> list[str]` (list of violation strings, empty = pass) and a CLI (`python3 scripts/skills_lint.py [repo_root]`, exit 0/1). Follows the existing `okf_check.py`/`test_okf_check.py` pattern: stdlib-only, run directly by `ci.sh`.

- [ ] **Step 1: Write the failing test**

Create `scripts/test_skills_lint.py` — hermetic (builds its own temp tree; never touches the real repo):

```python
#!/usr/bin/env python3
"""Hermetic tests for scripts/skills_lint.py.

Run directly: python3 scripts/test_skills_lint.py
"""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import skills_lint

GOOD_FM = """---
name: {name}
description: >-
  Does a thing. Use when testing the linter.
---

# body
"""


class SkillsLintTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        (self.root / ".agents" / "skills").mkdir(parents=True)
        (self.root / ".claude" / "skills").mkdir(parents=True)

    def add_skill(self, name, fm=None, link=True):
        d = self.root / ".agents" / "skills" / name
        d.mkdir()
        (d / "SKILL.md").write_text(fm if fm is not None else GOOD_FM.format(name=name))
        if link:
            (self.root / ".claude" / "skills" / name).symlink_to(
                Path("../../.agents/skills") / name
            )

    def test_clean_tree_passes(self):
        self.add_skill("alpha")
        self.assertEqual(skills_lint.lint(self.root), [])

    def test_missing_symlink(self):
        self.add_skill("alpha", link=False)
        self.assertTrue(any("missing symlink" in e for e in skills_lint.lint(self.root)))

    def test_missing_skill_md(self):
        (self.root / ".agents" / "skills" / "alpha").mkdir()
        self.assertTrue(any("missing SKILL.md" in e for e in skills_lint.lint(self.root)))

    def test_no_frontmatter(self):
        self.add_skill("alpha", fm="# no frontmatter here\n")
        self.assertTrue(any("no frontmatter" in e for e in skills_lint.lint(self.root)))

    def test_name_mismatch(self):
        self.add_skill("alpha", fm=GOOD_FM.format(name="beta"))
        self.assertTrue(any("!= directory" in e for e in skills_lint.lint(self.root)))

    def test_forbidden_name_word(self):
        self.add_skill("claude-helper", fm=GOOD_FM.format(name="claude-helper"))
        self.assertTrue(any("'claude'/'anthropic'" in e for e in skills_lint.lint(self.root)))

    def test_overlong_description(self):
        fm = "---\nname: alpha\ndescription: " + "x" * 1100 + "\n---\n"
        self.add_skill("alpha", fm=fm)
        self.assertTrue(any("max 1024" in e for e in skills_lint.lint(self.root)))

    def test_block_scalar_description_measured_joined(self):
        # >- folds lines with spaces; 600+600 chars + joiner must exceed 1024
        fm = ("---\nname: alpha\ndescription: >-\n  " + "x" * 600
              + "\n  " + "y" * 600 + "\n---\n")
        self.add_skill("alpha", fm=fm)
        self.assertTrue(any("max 1024" in e for e in skills_lint.lint(self.root)))

    def test_stray_entry_in_mirror(self):
        self.add_skill("alpha")
        (self.root / ".claude" / "skills" / "ghost").symlink_to(
            Path("../../.agents/skills/ghost")
        )
        self.assertTrue(any("stray entry" in e for e in skills_lint.lint(self.root)))

    def test_missing_mirror_dir(self):
        shutil.rmtree(self.root / ".claude" / "skills")
        self.add_skill("alpha", link=False)
        self.assertTrue(any("directory missing" in e for e in skills_lint.lint(self.root)))


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `python3 scripts/test_skills_lint.py`
Expected: FAIL immediately with `ModuleNotFoundError: No module named 'skills_lint'`.

- [ ] **Step 3: Write `scripts/skills_lint.py`**

```python
#!/usr/bin/env python3
"""Lint the Claude-facing skills trees (.agents/skills canonical, .claude/skills mirror).

Checks:
  1. Every .agents/skills/<name>/SKILL.md starts with a YAML frontmatter block
     whose `name` is legal (<=64 chars of [a-z0-9-], no "claude"/"anthropic")
     and equals the directory name, and whose `description` is non-empty and
     <=1024 chars (block scalars measured as the space-joined text).
  2. Every .agents/skills/<name>/ has a symlink .claude/skills/<name> resolving
     to it; no stray or dangling entries under .claude/skills/.

Stdlib-only (no PyYAML on this machine); frontmatter is parsed with regexes that
cover the plain and block-scalar (>-, >, |, |-) description forms used here.

Usage: python3 scripts/skills_lint.py [repo_root]   # exit 0 clean, 1 violations
"""
import re
import sys
from pathlib import Path

FM_RE = re.compile(r"\A---\n(.*?)\n---\n", re.S)
NAME_RE = re.compile(r"^name:\s*(\S+)\s*$", re.M)
DESC_RE = re.compile(r"^description:\s*(.*)\n((?:[ ]{2,}.*\n?)*)", re.M)
NAME_LEGAL = re.compile(r"[a-z0-9-]{1,64}\Z")
BLOCK_MARKERS = (">-", ">", "|", "|-")


def _frontmatter(text):
    m = FM_RE.match(text)
    return m.group(1) if m else None


def _description(block):
    m = DESC_RE.search(block)
    if not m:
        return None
    first, rest = m.group(1).strip(), m.group(2)
    parts = [] if first in BLOCK_MARKERS else [first]
    parts += [ln.strip() for ln in rest.splitlines()]
    return " ".join(p for p in parts if p).strip()


def lint(root):
    errors = []
    canonical = root / ".agents" / "skills"
    mirror = root / ".claude" / "skills"
    skills = sorted(d for d in canonical.iterdir() if d.is_dir())

    for d in skills:
        rel = f".agents/skills/{d.name}"
        md = d / "SKILL.md"
        if not md.is_file():
            errors.append(f"{rel}: missing SKILL.md")
            continue
        block = _frontmatter(md.read_text(encoding="utf-8"))
        if block is None:
            errors.append(f"{rel}/SKILL.md: no frontmatter block at top of file")
        else:
            nm = NAME_RE.search(block)
            name = nm.group(1) if nm else ""
            if not NAME_LEGAL.fullmatch(name):
                errors.append(f"{rel}/SKILL.md: illegal or missing name {name!r}")
            elif "claude" in name or "anthropic" in name:
                errors.append(f"{rel}/SKILL.md: name may not contain 'claude'/'anthropic'")
            elif name != d.name:
                errors.append(f"{rel}/SKILL.md: name {name!r} != directory name {d.name!r}")
            desc = _description(block)
            if not desc:
                errors.append(f"{rel}/SKILL.md: missing or empty description")
            elif len(desc) > 1024:
                errors.append(f"{rel}/SKILL.md: description is {len(desc)} chars (max 1024)")
        link = mirror / d.name
        if not link.is_symlink():
            errors.append(f".claude/skills/{d.name}: missing symlink -> ../../{rel}")
        elif link.resolve() != d.resolve():
            errors.append(
                f".claude/skills/{d.name}: resolves to {link.resolve()}, expected {d.resolve()}"
            )

    if mirror.is_dir():
        expected = {d.name for d in skills}
        for entry in sorted(mirror.iterdir()):
            if entry.name not in expected:
                errors.append(
                    f".claude/skills/{entry.name}: stray entry (no matching .agents/skills/ dir)"
                )
    else:
        errors.append(".claude/skills/: directory missing")
    return errors


def main():
    root = (
        Path(sys.argv[1]).resolve()
        if len(sys.argv) > 1
        else Path(__file__).resolve().parent.parent
    )
    errors = lint(root)
    for e in errors:
        print(f"skills-lint: {e}", file=sys.stderr)
    n = sum(1 for d in (root / ".agents" / "skills").iterdir() if d.is_dir())
    print(f"skills-lint: {'FAIL' if errors else 'OK'} ({n} skills checked)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `python3 scripts/test_skills_lint.py`
Expected: `OK` — all 10 tests pass.

- [ ] **Step 5: Run the lint against the real repo**

Run: `python3 scripts/skills_lint.py; echo "exit=$?"`
Expected: `skills-lint: OK (10 skills checked)` and `exit=0` (all 10 skills pre-verified compliant; symlinks from Task 3).

- [ ] **Step 6: Wire into `scripts/ci.sh`**

In `scripts/ci.sh`, directly after the okf-check leg (currently lines 11–13), insert:

```bash
echo "==> skills lint"
python3 scripts/test_skills_lint.py
python3 scripts/skills_lint.py
```

- [ ] **Step 7: Run the full CI gate**

Run: `bash scripts/ci.sh`
Expected: `==> skills lint` leg prints `skills-lint: OK (10 skills checked)`; gate ends `CI gate passed.`

- [ ] **Step 8: Commit**

```bash
git add scripts/skills_lint.py scripts/test_skills_lint.py scripts/ci.sh
git commit -m "feat(ci): skills lint — frontmatter rules + .claude/skills symlink integrity"
```

---

### Task 5: Committed project settings

**Files:**
- Create: `.claude/settings.json`

**Interfaces:**
- Consumes: `.gitignore` entries from Task 3 (which document that this file IS committed).

- [ ] **Step 1: Write `.claude/settings.json`**

Exact content. Pattern syntax `"Tool(prefix *)"` follows the bundle's documented form. Read-only-git note: `git log/diff/show` accept `--output=<file>` (a known write vector — see memory/argscan work); accepted here because these rules only skip *prompts* for a trusted local workspace, they grant nothing the user couldn't approve interactively.

```json
{
  "$schema": "https://json.schemastore.org/claude-code-settings.json",
  "permissions": {
    "allow": [
      "Bash(cargo build *)",
      "Bash(cargo build)",
      "Bash(cargo test *)",
      "Bash(cargo test)",
      "Bash(cargo fmt *)",
      "Bash(cargo clippy *)",
      "Bash(npm test)",
      "Bash(npm run typecheck)",
      "Bash(npm run build)",
      "Bash(npx vitest run *)",
      "Bash(npx vitest run)",
      "Bash(bash scripts/ci.sh)",
      "Bash(python3 scripts/skills_lint.py)",
      "Bash(python3 scripts/test_skills_lint.py)",
      "Bash(python3 scripts/okf_check.py *)",
      "Bash(git status)",
      "Bash(git log *)",
      "Bash(git diff *)",
      "Bash(git show *)"
    ]
  }
}
```

- [ ] **Step 2: Validate JSON parses**

Run: `python3 -c "import json; json.load(open('.claude/settings.json')); print('valid')"`
Expected: `valid`.

- [ ] **Step 3: Verify it is tracked (not swallowed by .gitignore)**

Run: `git add .claude/settings.json && git status --short .claude/settings.json`
Expected: `A  .claude/settings.json` (staged; if git refuses or shows nothing, a `.gitignore` rule from Task 3 is wrong — fix there, not with `git add -f`).

- [ ] **Step 4: Commit**

```bash
git commit -m "chore(claude): committed project settings — \$schema + safe-command allowlist"
```

---

### Task 6: End-to-end verification

**Files:** none (verification only).

- [ ] **Step 1: Full CI gate on the finished branch**

Run: `bash scripts/ci.sh`
Expected: all legs green including `==> skills lint`; ends `CI gate passed.`

- [ ] **Step 2: Clean tree + branch review**

Run: `git status --short; git log --oneline main..HEAD`
Expected: empty status; 5 commits (Tasks 1–5).

- [ ] **Step 3: Live Claude Code discovery check (needs a fresh session — coordinate with the user)**

This cannot be verified from inside the implementing session (skills/context load at session start). Ask the user to open a fresh `claude` session in the repo and confirm:

1. The available-skills list now includes the 10 repo skills (e.g. `harness-engineering`, `graphify-best-practices`, `auto-drive-tauri`).
2. `/context` shows root `AGENTS.md` content (via the `CLAUDE.md` import) — the repo map / conventions / gotchas are present.
3. Reading or editing a file under `web/` pulls in `web/AGENTS.md` content (subdir `CLAUDE.md` auto-load + import).

If (1) fails: check symlink resolution (`ls -lL .claude/skills`). If (2) fails: check `@AGENTS.md` is line 1 of `CLAUDE.md` with no leading whitespace/BOM.

- [ ] **Step 4: Hand off**

Use superpowers:finishing-a-development-branch to merge/PR per the user's choice. (Per repo convention, push only when asked.)
