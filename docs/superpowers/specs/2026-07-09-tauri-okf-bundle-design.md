# Tauri v2 OKF bundle — design

**Date:** 2026-07-09
**Status:** Panel-reviewed — pending owner gate (two escalated decisions, §Panel log)
**Branch:** `worktree-tauri-okf-bundle` (worktree forked from local `main` @ 5cabef5)

## Goal

Ship a general-reference OKF v0.1 knowledge bundle on Tauri v2 at `docs/okf/tauri/`,
weighted toward **best practices and testing**, built from v2.tauri.app plus targeted
ecosystem sources, verified with the house two-shape review, and discoverable via a
Claude-facing companion skill. As an enabling first step, upgrade `scripts/okf_check.py`
to real YAML parsing (PyYAML via uv) and close existing validation gaps.

## Consumers & motivation

Primary consumer: Claude agents working in this repo — both future `src-tauri/` work and
general Tauri questions. The bundle is a *general* v2 reference (owner decision), not
scoped to what `src-tauri/` currently uses. It will plausibly feed future specs, so it
gets spec-input verification rigor now rather than retroactively.

## Scope decisions (owner-confirmed)

| Decision | Choice |
| --- | --- |
| Purpose | General Tauri v2 reference, emphasis on best practices + testing |
| Corpus | v2.tauri.app spine + ecosystem sources where upstream is thin (testing, security, perf); **no** repo-local experience folded in |
| Scale | ~35–45 allowlisted sources (agent-sdlc weight class) |
| Deep areas | Testing; security model; IPC & app architecture; distribution & updates; performance & footprint |
| Mobile | Survey depth only (2–4 sources) |
| Verification | Full two-shape: adversarial per-claim refuters + consistency/completeness pass, both logged |
| Companion | Yes — shape escalated to gate (E1 in §Panel log): existing `.agents/skills/tauri/` skill collides |
| Isolation | Dedicated worktree off local `main`; the primary checkout (another agent's live branch) is never touched |

## Deliverable 1 — checker upgrade (lands first)

`scripts/okf_check.py`:

1. **PEP 723 inline-script metadata** (`# /// script` block: `requires-python`, `pyyaml`
   dependency) so `uv run scripts/okf_check.py <bundle>` is self-contained.
2. **Frontmatter parsing via `yaml.safe_load`**, replacing the minimal flat parser.
   Implementation notes: `yaml.YAMLError` → parse error; non-dict result → parse error;
   checks must tolerate YAML-native scalar types (e.g. `timestamp:` parses to a
   `datetime`, not a string). Docstring updated (usage line, retired flat-parser
   paragraph).
3. **Marker scanning gains code awareness:** fenced code blocks and inline code spans
   are stripped before `MARKER_RE` runs. Without this, code-heavy content false-fails
   (`claude-cli-headless/capabilities/partial-message-streaming.md` trips today on
   `` `content[0].text` `` once `capabilities` is citation-checked, and a Tauri bundle
   is dense with `args[0]`-shaped snippets).
4. **Vocabulary:** `Capability` added to `ALLOWED_TYPES`, `capabilities` added to
   `CITATION_DIRS`. This serves the **new** bundle only — existing
   `claude-cli-headless/capabilities/` files are typed `Practice`, which remains valid;
   **no retyping of old-bundle files in this campaign.**
5. **`scripts/test_okf_check.py`** updated for the new parser (own PEP 723 block, runs
   via `uv run`); new cases: block-style YAML lists parse, `type: Capability` accepted,
   datetime-typed `timestamp`, YAMLError/non-dict handling, code-span marker exclusion.

Pre-flight — run the **upgraded** checker on all three existing bundles. Known findings
(panel-verified):

6. **10 frontmatter blocks fail strict YAML** (unquoted colons in `title:`/`description:`
   values — 9 in `agent-sdlc`, 1 in `deepagents-refactor`). Fix: mechanical,
   value-preserving quoting only. Each edited bundle gets a dated `log.md` entry
   (**Update** — frontmatter quoted for YAML-strict checker; no semantic change). These
   are syntax-only edits to reviewed artifacts; no re-review beyond the log entry.
7. The `partial-message-streaming.md` false marker is resolved by point 3 (checker fix),
   not by editing that bundle. Any other pre-flight failure is fixed at the artifact
   with the same value-preserving discipline, never by loosening the checker.

Invocation & doc sweep (same commit series):

8. `scripts/ci.sh` okf leg: `python3` → `uv run`; validation extends from
   `docs/okf/agent-sdlc` only to **every** immediate subdirectory of `docs/okf/` (glob).
9. **`.github/workflows/ci.yml`:** add `astral-sh/setup-uv` (with cache) to the gate
   job, in the **same atomic commit** as the ci.sh switch — uv is not preinstalled on
   `ubuntu-latest`, so without this every CI run fails at the first leg.
10. Sweep every documented `python3 scripts/okf_check.py` invocation to `uv run`:
    `.agents/skills/agent-sdlc/SKILL.md`, `.agents/skills/agent-sdlc/authoring.md`
    (also retire its "block-style tags ✗ FAILS okf_check" rule — see conventions below),
    and `.claude/settings.json`'s Bash allowlist entry (else every check run
    permission-prompts). AGENTS.md CI-gate wording checked, updated if it implies
    pure-python3.

**New toolchain dependency:** ci.sh and the pre-push hook now require `uv` (installed
here: 0.9.26). First-ever offline run on a fresh machine fails until uv's cache is warm
— accepted residual; CI uses setup-uv's cache.

## Deliverable 2 — the bundle

### Layout

```
docs/okf/tauri/
├── index.md            # entry point: bundle purpose, concept map, Tauri version stamp + fetch window + staleness tripwire; frontmatter: okf_version: 0.1 only
├── log.md              # dated entries, newest first, bold action words (**Creation**, **Review**, ...)
├── sources/            # ~35–45 snapshots, one per allowlisted URL
├── capabilities/       # what Tauri v2 is/does: ~6 files, one per named topic — process model, IPC surface, windowing, plugin system, webview, mobile (dedicated capabilities/mobile.md); more only if synthesis surfaces a distinct capability
├── practices/          # the five deep areas as actionable practice files; granularity at synthesis's discretion (see acceptance criteria)
└── comparisons/        # e.g. events-vs-channels, mock-IPC-vs-WebDriver, v1-vs-v2 security model (~3–4 files)
```

No `perspectives/` or `phases/` unless synthesis surfaces a genuine need (YAGNI).
Per-directory `index.md` files list every non-reserved node (checker-enforced).

### Conventions (OKF SPEC v0.1, upstream)

- **Frontmatter fields in spec priority order:** `type`, `title`, `description`,
  `resource` (sources: the live URL), `tags`, `timestamp`. Values containing colons are
  quoted. **Inline-list YAML remains house authoring style** — the PyYAML parser
  tolerates block style, but that tolerance is an implementation detail, not a
  convention change.
- **`timestamp` keeps upstream semantics** — ISO 8601 of *last meaningful change* (it
  bumps when a fix wave edits the file). Fetch provenance lives in a separate `fetched:`
  extension field on sources (ISO date of snapshot) plus the fetch window in root
  `index.md`. Extension fields are spec-sanctioned (consumers preserve unknown keys).
- **Types:** `Source`, `Capability`, `Practice`, `Comparison`.
- **Links:** bundle-relative absolute form (`/sources/<slug>.md`) — spec-recommended and
  citation-checker-enforced.
- **Citations:** numbered `[1] [Title](url)` entries under `# Citations`; body `[n]`
  markers must resolve (checker-enforced). Every normative claim in a concept file
  carries a marker resolving to an **allowlisted** source; citing off-allowlist URLs is
  a Wave 4 defect (checker can't enforce this — Wave 4 greps for non-`/sources/`
  citation targets).
- **Sources are faithful condensations**, not verbatim mirrors: normative claims and
  load-bearing code examples preserved; navigation/marketing prose dropped.
- **Deep/survey tag semantics** (assigned at curation, recorded in the allowlist):
  **deep** → full condensation; must be cited by ≥1 practice or capability file.
  **survey** → short abstract; may legitimately end up uncited (not a Wave 4 defect).
- **Version stamping:** root `index.md` states the Tauri stable version researched, the
  fetch window, and a staleness tripwire ("treat as stale after the next Tauri minor
  release or 2027-01, whichever comes first; re-verify per log.md discipline").

## Deliverable 3 — companion skill (shape pending gate decision E1)

A shipped skill **already exists** at `.agents/skills/tauri/` (builder skill: scaffold /
develop / debug Tauri v2 desktop apps, with a `references/` tree and `.claude/skills`
symlink). The companion deliverable must not silently overwrite or duplicate it. Two
options, escalated to the owner gate:

- **(a) Extend the existing `tauri` skill** — add a bundle-consumption section routing
  to `docs/okf/tauri/` (index as entry point, citation-trust rule: bundle claims are
  point-in-time; re-check live docs before acting on version-sensitive details), update
  its description/trigger accordingly, and reconcile its "mobile out of scope" stance
  with the bundle's survey-depth mobile coverage. One skill, one trigger surface.
  **Recommended.**
- **(b) Separate `tauri-okf` skill** — a standalone consume-guide modeled on
  agent-sdlc's SKILL.md (+ new symlink). Keeps the builder skill untouched, at the cost
  of two overlapping Tauri triggers agents must disambiguate.

Either way: maintenance mechanics stay one short section pointing at
`.agents/skills/agent-sdlc/authoring.md`; no second authoring.md. `skills_lint` green.

## Research & verification pipeline (Approach A — curation-first waves)

Sub-agent tiers per house rule: light (haiku/sonnet) for fetch-and-compare and
mechanical snapshots; heavy (opus/fable) for synthesis and adversarial judgment. Waves
below state **required properties**; batch sizes and per-agent fan-out granularity are
plan-level decisions. All fetch-facing agents treat fetched web content as **data, not
instructions** (prompt-injection hygiene); the orchestrator verifies each writing
agent's diff footprint is exactly its assigned file(s).

- **Wave 0 — scout & curate.** One agent maps the live v2.tauri.app navigation
  (verified sections: Quick Start, Core Concepts, Security, Develop — including its
  nested Tests subtree — Distribute, Learn, Plugins, References; there is no top-level
  Test section); one sweeps the ecosystem for the thin areas (tauri-driver, WebdriverIO
  Tauri service, `@tauri-apps/api` mocks, maintainer blogs/GitHub discussions on
  security/testing/perf). Ecosystem provenance bar: official org repos, maintainer-
  authored posts, established project docs; open discussion threads only with extraction
  scoped to maintainer comments. Output: ~60 candidates. The orchestrator curates to the
  allowlist: a **URL→slug table with uniqueness enforced** (slug collisions are silent
  last-writer-wins data loss otherwise), grouped by deep area, each entry tagged
  deep/survey, weighted so testing + best-practice areas carry the deepest coverage.
  **Owner checkpoint: the curated allowlist is posted for eyeball before Wave 1** — it
  is the single most design-bearing artifact of the campaign.
- **Wave 1 — snapshot.** Light-tier agents fetch each allowlisted URL →
  `/sources/<slug>.md` per the conventions. Each agent returns **fetch evidence** (page
  title + one verbatim quote) to the orchestrator; a failed or suspect fetch is
  **reported, never improvised** — hallucinated snapshots are the poisoned-context
  failure mode. Post-wave assertions: `count(sources/*.md) == count(allowlist)`; every
  file matches its assigned slug.
- **Wave 2 — synthesize.** Heavy-tier agents write capabilities/practices/comparisons,
  citing only allowlisted sources.
- **Wave 3 — adversarial fact-verification.** Light-tier refuters re-fetch the **live**
  cited URLs and attempt to refute each claim. Refuter discipline: **read-only**
  reports; forbidden from using `/sources/` snapshots as evidence; every verdict backed
  by a quoted excerpt from the live fetch. While a live page is open, the refuter also
  **spot-checks the corresponding snapshot's fidelity** (closing the evidence-layer gap:
  snapshots are what citations resolve to). Verdict taxonomy: CONFIRMED /
  REFUTED-SYNTHESIS (concept misstates source) / SOURCE-CHANGED (live page moved on
  since snapshot → re-snapshot + `timestamp` bump, not a synthesis defect) /
  UNVERIFIABLE. **UNVERIFIABLE normative claims are never silently retained** — removed,
  or explicitly marked unverified in the concept file. Claims resting **solely on a
  single ecosystem source** are cross-checked against official docs where coverage
  overlaps; where it doesn't, the concept file flags them as single-source. Fix
  discipline: refuters supply exact corrected text + live-source quote; the orchestrator
  applies **verbatim**; any fix beyond verbatim substitution re-enters a scoped refuter
  check.
- **Wave 4 — consistency & completeness.** One heavy-tier reviewer over the finished
  bundle: cross-file contradictions, coverage gaps vs agreed scope (judged against the
  allowlist), index completeness, stale post-fix language, off-allowlist citation grep.
  May demand source additions (recall backstop). **Loop rule: any post-Wave-3 source or
  concept addition gets a scoped refuter pass before the final log entry** — otherwise
  the least-verified content enters last. Both verification waves land as dated `log.md`
  entries with disposition counts.

Sub-agent prompts state that any `file:line` anchors are orientation only — locate
quoted content by content (house rule).

## Delivery order on the branch

1. Checker upgrade + pre-flight quoting fixes + ci.sh + ci.yml + tests + doc/allowlist
   sweep (bundle validated by the new checker from its first commit).
2. Bundle waves 0–4, committed in reviewable increments, **sources before the concepts
   that cite them** and directory indexes kept current per commit. Residual (accepted):
   mid-campaign commits may transiently fail the full checker between waves; the
   pre-push hook makes this moot since nothing is pushed mid-campaign.
3. Companion skill work per gate decision E1.
4. Final: `bash scripts/ci.sh` green.

Conventional commits throughout; no push without an explicit ask.

## Acceptance criteria

- `uv run scripts/okf_check.py docs/okf/tauri` exits 0; same for the three existing
  bundles; `uv run scripts/test_okf_check.py` and `scripts/skills_lint.py` pass;
  `bash scripts/ci.sh` green end-to-end; ci.yml carries the uv setup step.
- Source count within 35–45. The allowlist shows per-area grouping with testing and the
  best-practice areas carrying the deepest tags — the owner's stated emphasis is visible
  in the corpus, not just the prose.
- Each of the five deep areas is covered at **practice depth** (actionable guidance, not
  description), judged by the Wave 4 reviewer against the allowlist; file granularity is
  synthesis's call. Testing coverage must include mock-IPC, WebDriver/tauri-driver, and
  CI guidance. Mobile: dedicated `capabilities/mobile.md` at survey depth.
- `log.md` carries dated entries for creation, the adversarial fact-verification wave
  (with verdict-taxonomy disposition counts), and the consistency/completeness pass.
- Root `index.md` states Tauri version, fetch window, and the staleness tripwire.

## Non-goals

- No repo-local (`src-tauri/`) experience folded into the bundle (owner decision — corpus
  is docs + ecosystem only).
- No retyping or semantic editing of existing bundles (pre-flight quoting is
  syntax-only).
- No `perspectives/`/`phases/` directories speculatively.
- No graphify ingestion, no serving to the runtime's own agent (parked follow-ups, same
  as agent-sdlc).
- No mobile deep-dive (Xcode/Gradle setup minutiae, store distribution).
- No general-purpose third-party OKF validation beyond what the PyYAML upgrade naturally
  grants; house checks stay stricter than spec.

## Risks & residuals

- **Scout misses a load-bearing source** → Wave 4 reviewer empowered to demand additions
  (with the refuter loop rule); curation happens against the live nav tree; owner
  eyeballs the allowlist.
- **Doc drift / version churn** → version stamp, `fetched:` provenance, SOURCE-CHANGED
  verdict path, staleness tripwire.
- **Confidently-wrong ecosystem source** → provenance bar + single-source cross-check /
  flagging (fidelity-verification alone can't catch this).
- **uv first-run offline / PyPI blip** → accepted residual locally; CI cached via
  setup-uv.
- **Mid-campaign checker-red commits** → accepted residual (see delivery order).
- **Checker can't enforce allowlist-only citations** → Wave 4 grep; accepted as a
  process check rather than a checker feature.

## Panel & review log

### 2026-07-09 — adversarial spec panel (4 reviewers: requirements, assumptions, failure & abuse, scope/YAGNI)

Panel calibrated skeptical per AGENTS.md. Orchestrator independently verified the
cross-reviewer contradiction (does `claude-cli-headless` pass the current checker? — it
does; its capability files are typed `Practice`) and the skill-collision claim (real).

**Blockers/majors fixed in place:**

- yaml.safe_load breaks 10 existing frontmatter blocks (unquoted colons; 9 agent-sdlc,
  1 deepagents-refactor) — pre-flight scope corrected to all three bundles, mechanical
  quoting + log entries specified (assumptions: BLOCKER).
- ci.yml has no uv → setup-uv step, atomic with ci.sh switch (assumptions + failure:
  MAJOR).
- `python3 okf_check` invocation sweep: agent-sdlc SKILL.md/authoring.md, docstring,
  settings.json allowlist (assumptions: MAJOR).
- MARKER_RE false-positives on inline code → code-span/fence exclusion in checker
  (assumptions: MAJOR).
- Snapshot fidelity never verified + hallucinated-fetch risk → fetch evidence, no
  improvised snapshots, refuter snapshot spot-checks, count/footprint assertions
  (requirements + failure: MAJOR).
- Wave 4 additions bypass refutation → loop rule (failure: MAJOR).
- Fix-wave error injection → verbatim-fix discipline (failure: MAJOR).
- No UNVERIFIABLE policy → never-silently-retained rule (failure: MAJOR).
- Ecosystem fidelity-vs-truth → single-source cross-check/flagging (failure: MAJOR).
- Prompt-injection surface → data-not-instructions rule, maintainer-comment scoping,
  read-only refuters, diff-footprint checks (failure: MAJOR).
- Slug collisions → URL→slug table with uniqueness + post-wave assertions (failure:
  MAJOR).
- Emphasis had no acceptance criterion + per-area file quota was spec-invented →
  acceptance reworked to outcome-shaped coverage with allowlist-visible weighting and a
  concrete testing-content floor (requirements MAJOR + scope MAJOR, reconciled).

**Escalated to the owner gate:**

- **E1 — companion-skill collision** with the shipped `.agents/skills/tauri/` builder
  skill: extend it (recommended) vs separate `tauri-okf` skill. *(decision: pending)*
- **E2 — allowlist owner checkpoint** adopted into the pipeline (owner eyeballs the
  curated allowlist before Wave 1); owner may instead delegate curation. *(decision:
  pending)*
- **E3 — pre-flight edits touch reviewed bundles** (mechanical frontmatter quoting in
  agent-sdlc + deepagents-refactor, with log entries): flagged for owner awareness since
  those artifacts already passed review. *(decision: pending)*

**Minors accepted/fixed as noted:**

- Fixed: timestamp semantics (upstream last-change + `fetched:` extension); deep/survey
  tag semantics defined (also resolves orphan-source ambiguity); mobile deliverable
  disambiguated to `capabilities/mobile.md`; nav-tree list corrected against the live
  site (no top-level Test section; Learn added); old-bundle no-retype fence; capability
  count fenced to ~6 named topics; fan-out granularity demoted to plan level;
  inline-list authoring style retained (parser tolerance ≠ convention); SOURCE-CHANGED
  vs REFUTED-SYNTHESIS verdict split; staleness tripwire; datetime-coercion +
  YAMLError test cases; off-allowlist citation grep; refuter snapshot-shortcut ban;
  sources-before-concepts commit ordering.
- Accepted residual: mid-campaign checker-red commits; uv offline first-run; checker not
  enforcing allowlist-only citations (process check instead); no automated staleness
  tripwire beyond the dated line.
