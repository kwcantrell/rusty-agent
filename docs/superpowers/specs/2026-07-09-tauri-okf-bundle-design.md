# Tauri v2 OKF bundle — design

**Date:** 2026-07-09
**Status:** Draft — pending adversarial panel + owner gate
**Branch:** `worktree-tauri-okf-bundle` (worktree forked from local `main` @ 5cabef5)

## Goal

Ship a general-reference OKF v0.1 knowledge bundle on Tauri v2 at `docs/okf/tauri/`,
weighted toward **best practices and testing**, built from v2.tauri.app plus targeted
ecosystem sources, verified with the house two-shape review, and discoverable via a
Claude-facing companion skill. As an enabling first step, upgrade `scripts/okf_check.py`
to real YAML parsing (PyYAML via uv) and close two existing validation gaps.

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
| Companion | Yes — `.agents/skills/tauri/` consume-guide + `.claude/skills` symlink |
| Isolation | Dedicated worktree off local `main`; the primary checkout (another agent's live branch) is never touched |

## Deliverable 1 — checker upgrade (lands first)

`scripts/okf_check.py`:

1. **PEP 723 inline-script metadata** (`# /// script` block: `requires-python`, `pyyaml`
   dependency) so `uv run scripts/okf_check.py <bundle>` is self-contained.
2. **Frontmatter parsing via `yaml.safe_load`**, replacing the minimal flat parser. The
   inline-list-only house constraint is retired; any valid YAML frontmatter parses.
   Existing bundles (inline lists are valid YAML) must keep passing unchanged.
3. **Vocabulary gap closed:** `Capability` added to `ALLOWED_TYPES`, `capabilities` added
   to `CITATION_DIRS` — the existing `claude-cli-headless/capabilities/` directory is
   currently outside both.
4. **`scripts/test_okf_check.py`** updated for the new parser; new cases for block-style
   YAML lists and `type: Capability`. It imports `okf_check` directly, so it gets its own
   PEP 723 block (pyyaml) and runs via `uv run` as well.

`scripts/ci.sh` okf leg:

5. Both okf calls switch `python3` → `uv run`.
6. Validation extends from `docs/okf/agent-sdlc` only to **every** bundle directory under
   `docs/okf/` (glob over immediate subdirectories). Pre-flight: run the upgraded checker
   against `claude-cli-headless` and `deepagents-refactor`; any failures are fixed in this
   campaign before the ci.sh change lands.

**New toolchain dependency:** ci.sh and the pre-push hook now require `uv` (installed
here: 0.9.26; uv fetches pyyaml on first run). Documented in the commit message and
AGENTS.md CI-gate section if wording there implies pure-python3.

## Deliverable 2 — the bundle

### Layout

```
docs/okf/tauri/
├── index.md            # entry point: bundle purpose, concept map, Tauri version stamp; frontmatter: okf_version: 0.1 only
├── log.md              # dated entries, newest first, bold action words (**Creation**, **Review**, ...)
├── sources/            # ~35–45 snapshots, one per allowlisted URL
├── capabilities/       # what Tauri v2 is/does: process model, IPC surface, windowing, plugin system, webview, mobile story (~6–8 files)
├── practices/          # the five deep areas as actionable practice files (~10–14 files)
└── comparisons/        # e.g. events-vs-channels, mock-IPC-vs-WebDriver, v1-vs-v2 security model (~3–4 files)
```

No `perspectives/` or `phases/` unless synthesis surfaces a genuine need (YAGNI).
Per-directory `index.md` files list every non-reserved node (checker-enforced).

### Conventions (OKF SPEC v0.1, upstream)

- **Frontmatter fields in spec priority order:** `type`, `title`, `description`,
  `resource` (sources: the live URL), `tags`, `timestamp` (ISO 8601, set at
  fetch/synthesis time). Block-style YAML lists permitted (new checker).
- **Types:** `Source`, `Capability`, `Practice`, `Comparison`.
- **Links:** bundle-relative absolute form (`/sources/<slug>.md`) — spec-recommended and
  citation-checker-enforced.
- **Citations:** numbered `[1] [Title](url)` entries under `# Citations`; body `[n]`
  markers must resolve (checker-enforced). Every normative claim in a concept file
  carries a marker resolving to an allowlisted source.
- **Sources are faithful condensations**, not verbatim mirrors: normative claims and
  load-bearing code examples preserved; navigation/marketing prose dropped. Provenance
  (`resource` + `timestamp`) makes the live original recoverable.
- **Version stamping:** Tauri stable version at fetch date recorded in root `index.md`
  and implied per-source by `timestamp`.

## Deliverable 3 — companion skill

`.agents/skills/tauri/SKILL.md` (+ relative symlink `.claude/skills/tauri` →
`../../.agents/skills/tauri`, satisfying `scripts/skills_lint.py`): a consume-guide
modeled on `agent-sdlc`'s SKILL.md — when to reach for the bundle, `index.md` as entry
point, the citation-trust rule (bundle claims are point-in-time; re-check live docs
before acting on version-sensitive details). Maintenance mechanics: one short section
pointing at `.agents/skills/agent-sdlc/authoring.md` conventions; **no** second
authoring.md.

## Research & verification pipeline (Approach A — curation-first waves)

Sub-agent tiers per house rule: light (haiku/sonnet) for fetch-and-compare and
mechanical snapshots; heavy (opus/fable) for synthesis and adversarial judgment.

- **Wave 0 — scout & curate.** One agent maps the v2.tauri.app navigation tree (Start,
  Core Concepts, Security, Develop, Distribute, Test, Plugins, Reference); one sweeps the
  ecosystem for the thin areas (tauri-driver, WebdriverIO Tauri service, `@tauri-apps/api`
  mocks, maintainer blogs/GitHub discussions on security/testing/perf). Output: ~60
  candidates (URL, section, one-line why). The orchestrator curates to the ~35–45
  allowlist, each entry tagged **deep** or **survey**, checked against the five deep
  areas + mobile-survey scope. The allowlist is the contract for all later waves.
- **Wave 1 — snapshot.** Parallel light-tier agents, one per allowlisted URL →
  `/sources/<slug>.md` per the conventions above.
- **Wave 2 — synthesize.** Heavy-tier agents write capabilities/practices/comparisons,
  citing only allowlisted sources.
- **Wave 3 — adversarial fact-verification.** Light-tier refuters, one per concept file,
  re-fetch the *live* cited URLs and attempt to refute each claim (v1 behavior stated as
  v2, stale API names, over-claims). Calibrated skeptical. Orchestrator applies fixes as
  a wave.
- **Wave 4 — consistency & completeness.** One heavy-tier reviewer over the finished
  bundle: cross-file contradictions, coverage gaps vs agreed scope, index completeness,
  stale post-fix language. May demand source additions (recall backstop for
  curation-first). Both verification waves land as dated `log.md` entries.

Sub-agent prompts state that any `file:line` anchors are orientation only — locate
quoted content by content (house rule).

## Delivery order on the branch

1. Checker upgrade + ci.sh + tests + old-bundle pre-flight fixes (bundle validated by the
   new checker from its first commit).
2. Bundle waves 0–4 (committed in reviewable increments — allowlist, sources, concepts,
   fix waves).
3. Companion skill + symlink.
4. Final: `bash scripts/ci.sh` green.

Conventional commits throughout; no push without an explicit ask.

## Acceptance criteria

- `uv run scripts/okf_check.py docs/okf/tauri` exits 0; same for the three existing
  bundles; `uv run scripts/test_okf_check.py` and `scripts/skills_lint.py` pass;
  `bash scripts/ci.sh` green end-to-end.
- Source count within 35–45; every deep area has ≥2 practice files; mobile covered at
  survey depth (2–4 sources, ≥1 capability file section).
- `log.md` carries dated entries for creation, the adversarial fact-verification wave
  (with disposition counts), and the consistency/completeness pass.
- Root `index.md` states the Tauri version researched and the fetch window.

## Non-goals

- No repo-local (`src-tauri/`) experience folded into the bundle (owner decision — corpus
  is docs + ecosystem only).
- No `perspectives/`/`phases/` directories speculatively.
- No graphify ingestion, no serving to the runtime's own agent (parked follow-ups, same
  as agent-sdlc).
- No mobile deep-dive (Xcode/Gradle setup minutiae, store distribution).
- No general-purpose third-party OKF validation beyond what the PyYAML upgrade naturally
  grants; house checks stay stricter than spec.

## Risks & mitigations

- **Scout misses a load-bearing source** → Wave 4 reviewer explicitly empowered to demand
  additions; curation happens against the nav tree, not search results alone.
- **Doc drift / version churn** → version stamp + per-source timestamps; checker docstring
  already names semantic drift a human duty via dated log entries.
- **uv absent on some machine running ci.sh** → fails fast with a clear command-not-found;
  uv is a one-line install and already the repo owner's tooling.
- **Ecosystem source quality** → allowlist curation applies a provenance bar (official
  org repos, maintainer posts, established project docs — not random blog spam).
- **Old bundles fail the extended ci.sh glob** → pre-flight step fixes them before the
  glob lands; the glob commit is atomic with those fixes.

## Panel & review log

*(to be filled after the adversarial panel and owner gate)*
