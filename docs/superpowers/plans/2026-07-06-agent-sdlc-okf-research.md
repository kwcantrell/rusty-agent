# Agent-SDLC Research → OKF Bundle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (recommended for this plan — most tasks orchestrate the Workflow tool, which only the main loop can call) to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Research "SDLC for AI agents" (both meanings, compared) from allowlisted first-party sources and ship the findings as an OKF v0.1 bundle at `docs/okf/agent-sdlc/`, validated by a committed checker.

**Architecture:** A staged pipeline — ground in the real OKF spec, build a deterministic conformance checker (TDD), then Workflow-orchestrated discovery (haiku) → inline curation → extraction (haiku) → inline taxonomy design → synthesis writers (opus, comparison on fable) → adversarial verification (sonnet) → finalize and commit. Intermediates live in the session scratchpad; only the bundle and checker are committed.

**Tech Stack:** Python 3 stdlib (checker + tests via `unittest`), Claude Code `Workflow`/`Agent` tools, `WebFetch`/`WebSearch`.

**Spec:** `docs/superpowers/specs/2026-07-06-agent-sdlc-okf-research-design.md`

## Global Constraints

- **Source allowlist (enforced at curation):** hosts ending in `anthropic.com`, `docs.claude.com`, `cloud.google.com`, `developers.googleblog.com`, `research.google`, `blog.langchain.dev`, `blog.langchain.com`, `docs.langchain.com`, `arxiv.org`, `deepmind.google`, `openai.com` (research/blog paths only), `microsoft.com` (`/en-us/research` paths only). Nothing else, ever.
- **Depth:** 25–40 curated sources; both meanings (`building-agents`, `agent-run-sdlc`) covered; explicit comparison concept.
- **Model tiering:** discovery + extraction = `haiku` (effort `low`); synthesis writers = `opus` (comparison concept = `fable`), effort `high`; verification = `sonnet`, effort `high`; taxonomy/curation = main loop inline.
- **OKF v0.1 conformance:** every non-reserved `.md` has YAML frontmatter with non-empty `type`; `index.md`/`log.md` reserved (index files: no frontmatter, except bundle-root `index.md` which declares only `okf_version: "0.1"`); root-relative cross-links; sourced claims cited under `# Citations`. Frontmatter uses **inline list syntax only** (`tags: [a, b]`).
- **Scratch dir:** `$SCRATCH` = the executing session's scratchpad directory; all intermediates under `$SCRATCH/agent-sdlc-research/`. Define `WORK=$SCRATCH/agent-sdlc-research` once and reuse.
- **Fixed date string** `2026-07-06` for all `timestamp:` fields (Workflow scripts cannot call `Date.now()`; agents receive the date in their prompts).
- **Commits:** conventional commits; commit only at the steps that say so.
- `python3` runs from repo root: `/home/kalen/rust-agent-runtime`.

---

### Task 1: Ground in the real OKF spec

**Files:**
- Create: `$WORK/conformance-notes.md` (scratchpad, not committed)

**Interfaces:**
- Produces: `$WORK/conformance-notes.md` — verbatim conformance rules + any deltas from the Global Constraints above. Task 2 reads it before finalizing checker rules.

- [ ] **Step 1: Create the work dir**

```bash
mkdir -p "$WORK"/{notes,}
```

- [ ] **Step 2: Fetch the authoritative spec**

WebFetch `https://raw.githubusercontent.com/GoogleCloudPlatform/knowledge-catalog/main/okf/SPEC.md` with prompt: "Reproduce all normative rules verbatim: conformance criteria, frontmatter fields, reserved filenames and their exact semantics, link rules, citation conventions, versioning/okf_version rules."

- [ ] **Step 3: Fetch one sample bundle for style**

WebFetch `https://github.com/GoogleCloudPlatform/knowledge-catalog/tree/main/okf/samples` (follow one sample, e.g. the GA4 bundle's root `index.md` and one concept file) with prompt: "Show the exact frontmatter and body structure of the root index.md and one concept document."

- [ ] **Step 4: Write conformance notes**

Write `$WORK/conformance-notes.md` containing: (a) the verbatim rules from Step 2, (b) the sample's index/concept structure from Step 3, (c) a short "deltas" section listing any place where this plan's Global Constraints or the Task 2 checker rules disagree with the spec. If there are deltas, adjust the Task 2 test/impl code accordingly **before** starting Task 2 (the checker must encode the real spec, not this plan's guess).

---

### Task 2: OKF conformance checker (TDD)

**Files:**
- Create: `scripts/okf_check.py`
- Test: `scripts/test_okf_check.py`

**Interfaces:**
- Produces: `python3 scripts/okf_check.py <bundle-dir>` — exit 0 + `OK` on pass; exit 1 with one error per line on failure. Also importable: `okf_check.check_bundle(path) -> list[str]`. Tasks 6–9 run this.

- [ ] **Step 1: Write the failing tests**

Create `scripts/test_okf_check.py`:

```python
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import okf_check

VALID_SOURCE = """---
type: Source
title: Example
resource: https://example.com/post
---
# Summary
A claim.
"""


def write(root, rel, text):
    p = Path(root) / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding="utf-8")


def valid_bundle(root):
    write(root, "index.md",
          '---\nokf_version: "0.1"\n---\n# Bundle\n- [example](/sources/example.md)\n')
    write(root, "sources/index.md", "# Sources\n- [example](/sources/example.md)\n")
    write(root, "sources/example.md", VALID_SOURCE)
    write(root, "practices/evals.md",
          "---\ntype: Practice\ntitle: Evals\ntags: [building-agents]\n---\n"
          "Body claim [1].\n\n# Citations\n1. [example](/sources/example.md)\n")


class OkfCheckTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def test_valid_bundle_passes(self):
        valid_bundle(self.root)
        self.assertEqual(okf_check.check_bundle(self.root), [])

    def test_missing_type_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/no_type.md", "---\ntitle: X\n---\nbody\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("no_type.md" in e and "type" in e for e in errs))

    def test_missing_frontmatter_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/bare.md", "just a body\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("bare.md" in e and "frontmatter" in e for e in errs))

    def test_broken_link_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/bad_link.md",
              "---\ntype: Source\n---\nSee [missing](/sources/nope.md)\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("broken link" in e for e in errs))

    def test_external_links_ignored(self):
        valid_bundle(self.root)
        write(self.root, "sources/ext.md",
              "---\ntype: Source\n---\nSee [site](https://example.com/x) and [anchor](#schema)\n")
        self.assertEqual(okf_check.check_bundle(self.root), [])

    def test_non_root_index_frontmatter_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/index.md", "---\ntype: Index\n---\n# Sources\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("sources/index.md" in e for e in errs))

    def test_root_index_extra_keys_fail(self):
        valid_bundle(self.root)
        write(self.root, "index.md", '---\nokf_version: "0.1"\ntype: Bundle\n---\n# B\n')
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("okf_version" in e for e in errs))

    def test_missing_citations_fails(self):
        valid_bundle(self.root)
        write(self.root, "practices/uncited.md",
              "---\ntype: Practice\n---\nA sourced claim with no citations.\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("uncited.md" in e and "Citations" in e for e in errs))

    def test_citations_without_source_links_fail(self):
        valid_bundle(self.root)
        write(self.root, "practices/selfcite.md",
              "---\ntype: Practice\n---\nClaim.\n\n# Citations\n1. [me](/practices/evals.md)\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("selfcite.md" in e for e in errs))


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `python3 scripts/test_okf_check.py`
Expected: FAIL with `ModuleNotFoundError: No module named 'okf_check'`

- [ ] **Step 3: Implement the checker**

Create `scripts/okf_check.py`:

```python
#!/usr/bin/env python3
"""Validate an OKF v0.1 bundle (the subset of the spec this repo's bundles use).

Checks:
1. every non-reserved .md file has parseable YAML frontmatter with a non-empty `type`
2. index.md files carry no frontmatter, except the bundle-root index.md, which may
   declare only `okf_version`; log.md carries no frontmatter
3. all intra-bundle markdown links resolve to existing files inside the bundle
4. every concept under phases/, practices/, perspectives/, comparisons/ has a
   `# Citations` section containing at least one resolving link into /sources/

Frontmatter is parsed with a minimal flat parser: `key: value` and `key: [a, b]`
lines only (bundles produced by this repo use inline list syntax).

Usage: python3 scripts/okf_check.py <bundle-dir>
Exits 0 and prints OK on success; exits 1 with one error per line otherwise.
"""
import re
import sys
from pathlib import Path

RESERVED = {"index.md", "log.md"}
CITATION_DIRS = {"phases", "practices", "perspectives", "comparisons"}
LINK_RE = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")
KV_RE = re.compile(r"^([A-Za-z_][\w-]*):\s*(.*)$")
CITATIONS_HEADING_RE = re.compile(r"^#{1,3}\s+Citations\s*$", re.MULTILINE)
NEXT_HEADING_RE = re.compile(r"^#{1,3}\s+\S", re.MULTILINE)


def split_frontmatter(text):
    """Return (frontmatter_text or None, body)."""
    if not text.startswith("---\n"):
        return None, text
    end = text.find("\n---\n", 4)
    if end == -1:
        return None, text
    return text[4:end], text[end + 5:]


def parse_frontmatter(fm_text):
    """Return a dict, or None if any non-blank line fails to parse."""
    data = {}
    for line in fm_text.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        m = KV_RE.match(line)
        if not m:
            return None
        key, val = m.group(1), m.group(2).strip()
        if val.startswith("[") and val.endswith("]"):
            data[key] = [v.strip().strip("\"'") for v in val[1:-1].split(",") if v.strip()]
        else:
            data[key] = val.strip("\"'")
    return data


def iter_links(text):
    for target in LINK_RE.findall(text):
        if target.startswith(("http://", "https://", "mailto:", "#")):
            continue
        yield target.split("#")[0]


def check_bundle(root):
    root = Path(root).resolve()
    errors = []
    md_files = sorted(root.rglob("*.md"))
    if not md_files:
        return [f"{root}: no .md files found"]
    for path in md_files:
        rel = path.relative_to(root).as_posix()
        text = path.read_text(encoding="utf-8")
        fm_text, body = split_frontmatter(text)

        if path.name in RESERVED:
            if fm_text is not None:
                if rel == "index.md":
                    fm = parse_frontmatter(fm_text)
                    if fm is None or set(fm) != {"okf_version"}:
                        errors.append(
                            f"{rel}: bundle-root index.md frontmatter may declare only okf_version")
                else:
                    errors.append(f"{rel}: {path.name} must not have frontmatter")
        else:
            if fm_text is None:
                errors.append(f"{rel}: missing frontmatter")
            else:
                fm = parse_frontmatter(fm_text)
                if fm is None:
                    errors.append(f"{rel}: unparseable frontmatter")
                elif not str(fm.get("type", "")).strip():
                    errors.append(f"{rel}: missing or empty `type`")

        for target in iter_links(body):
            if target.startswith("/"):
                resolved = (root / target.lstrip("/")).resolve()
            else:
                resolved = (path.parent / target).resolve()
            try:
                resolved.relative_to(root)
            except ValueError:
                errors.append(f"{rel}: link escapes bundle: {target}")
                continue
            if not resolved.exists():
                errors.append(f"{rel}: broken link: {target}")

        parts = Path(rel).parts
        if path.name not in RESERVED and parts and parts[0] in CITATION_DIRS:
            m = CITATIONS_HEADING_RE.search(body)
            if not m:
                errors.append(f"{rel}: missing # Citations section")
            else:
                section = body[m.end():]
                nxt = NEXT_HEADING_RE.search(section)
                if nxt:
                    section = section[:nxt.start()]
                cites = [t for t in iter_links(section) if t.startswith("/sources/")]
                if not cites:
                    errors.append(f"{rel}: # Citations has no /sources/ links")
    return errors


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    errors = check_bundle(sys.argv[1])
    for e in errors:
        print(e)
    if errors:
        print(f"FAIL: {len(errors)} error(s)")
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
```

If Task 1's conformance notes recorded deltas (e.g. different reserved-file semantics), encode the spec's version, adjusting tests first, then implementation.

- [ ] **Step 4: Run tests to verify they pass**

Run: `python3 scripts/test_okf_check.py`
Expected: `OK` (9 tests passed)

- [ ] **Step 5: Commit**

```bash
git add scripts/okf_check.py scripts/test_okf_check.py
git commit -m "feat(scripts): OKF v0.1 bundle conformance checker"
```

---

### Task 3: Discovery workflow

**Files:**
- Create: `$WORK/candidates.json`

**Interfaces:**
- Produces: `$WORK/candidates.json` — `{"candidates": [{url, title, org, date?, meanings: ["building-agents"|"agent-run-sdlc", ...], relevance}]}`. Task 4 consumes it.

- [ ] **Step 1: Run the discovery workflow**

Invoke `Workflow` with this script:

```js
export const meta = {
  name: 'agent-sdlc-discovery',
  description: 'Sweep Anthropic/Google/LangChain/academic sources for agent-SDLC material',
  phases: [{ title: 'Discover', detail: '4 parallel family sweeps (haiku)' }],
}
const CANDIDATES = {
  type: 'object', required: ['candidates'],
  properties: { candidates: { type: 'array', items: {
    type: 'object',
    required: ['url', 'title', 'org', 'meanings', 'relevance'],
    properties: {
      url: { type: 'string' }, title: { type: 'string' }, org: { type: 'string' },
      date: { type: 'string' },
      meanings: { type: 'array', items: { enum: ['building-agents', 'agent-run-sdlc'] } },
      relevance: { type: 'string' },
    },
  } } },
}
const COMMON = `You are finding first-party sources on the software development lifecycle (SDLC) for AI agents, in BOTH senses: (a) building-agents — how teams design, prototype, eval, test, deploy, and monitor agent systems; (b) agent-run-sdlc — how AI coding agents participate in or drive the SDLC (spec-first workflows, plan-execute-review loops, human gates). Use WebSearch to find pages and WebFetch to confirm each page exists and is published by the named org (or is a peer-reviewed paper). Return 10-20 candidates as structured output. Include the publication date when visible. Exclude secondary blogs, Medium, and press coverage.`
const FAMILIES = [
  { key: 'anthropic', prompt: COMMON + `\nFamily: Anthropic. Sweep anthropic.com/engineering, anthropic.com/research, docs.claude.com. Seed topics: building effective agents; Claude Code best practices; how we built our multi-agent research system; writing effective tools for agents; agent evals; context engineering.` },
  { key: 'google', prompt: COMMON + `\nFamily: Google. Sweep cloud.google.com/blog, developers.googleblog.com, research.google. Seed topics: AgentOps; Agent Development Kit (ADK); agent evaluation; MLOps for generative AI / agents; Vertex AI Agent Builder guidance; agent2agent (A2A) protocol.` },
  { key: 'langchain', prompt: COMMON + `\nFamily: LangChain. Sweep blog.langchain.dev, blog.langchain.com, docs.langchain.com. Seed topics: agent engineering; plan-and-execute agents; evaluating agents with LangSmith; LangGraph reliability patterns; context engineering; ambient agents.` },
  { key: 'academic', prompt: COMMON + `\nFamily: peer-reviewed + major-lab research only (arxiv.org, deepmind.google, openai.com research, microsoft.com/en-us/research). Seed topics: LLM agent surveys; agent benchmarks (SWE-bench, AgentBench, tau-bench); AI software engineering lifecycle studies; agentic coding workflow papers.` },
]
phase('Discover')
const results = await parallel(FAMILIES.map(f => () =>
  agent(f.prompt, { label: 'discover:' + f.key, schema: CANDIDATES, model: 'haiku', effort: 'low' })))
return { candidates: results.filter(Boolean).flatMap(r => r.candidates) }
```

- [ ] **Step 2: Save the result**

Write the workflow's return value to `$WORK/candidates.json`. Sanity check: `python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(len(d['candidates']))" "$WORK/candidates.json"` — expect roughly 40–80. If any family returned null (agent died), re-run just that family with a fresh single `agent()` workflow before proceeding.

---

### Task 4: Curation (inline, main loop)

**Files:**
- Create: `$WORK/sources.json`

**Interfaces:**
- Consumes: `$WORK/candidates.json` (Task 3 shape).
- Produces: `$WORK/sources.json` — `{"selected": [{slug, url, title, org, date, meanings, relevance}], "reserve": [same shape]}` with 25–40 selected. Tasks 5–7 consume `selected`; Task 5 pulls replacements from `reserve`.

- [ ] **Step 1: Dedup and enforce the allowlist**

Read `$WORK/candidates.json`. Dedup by normalized URL (strip trailing `/`, query strings, `www.`). Drop every candidate whose host/path is not on the Global Constraints allowlist. This is a judgment-free mechanical filter — do it inline, listing what was dropped and why.

- [ ] **Step 2: Select 25–40 with balanced coverage**

Selection criteria, in order: (1) both meanings covered with ≥8 sources each; (2) all three orgs represented with ≥5 sources each; (3) ≥4 academic/peer-reviewed; (4) prefer canonical/primary pieces over derivative posts; (5) prefer dated, stable URLs. Assign each a kebab-case slug prefixed by org (e.g. `anthropic-building-effective-agents`, `google-agentops-intro`, `arxiv-swe-bench`). Keep the next ~10 best as `reserve`.

- [ ] **Step 3: Write and report**

Write `$WORK/sources.json`. Report to the user: counts per org and per meaning, plus the dropped-at-allowlist list. If selected < 25, return to Task 3 and run one more targeted discovery sweep for the underrepresented family before proceeding.

---

### Task 5: Extraction workflow

**Files:**
- Create: `$WORK/notes/<slug>.json` (one per selected source)

**Interfaces:**
- Consumes: `$WORK/sources.json` `selected` array.
- Produces: `$WORK/notes/<slug>.json` — `{slug, url, title, org, date, meanings, claims: [{id, claim, quote}], practices: [string], phases: [string], terms: [string]}`. Tasks 6–8 consume these.

- [ ] **Step 1: Run the extraction workflow**

Invoke `Workflow` with `args: {"sources": <selected array>, "notesDir": "<absolute $WORK/notes>"}` and this script:

```js
export const meta = {
  name: 'agent-sdlc-extract',
  description: 'Fetch each curated source and write structured extraction notes',
  phases: [{ title: 'Extract', detail: 'one haiku agent per source' }],
}
const RESULT = {
  type: 'object', required: ['slug', 'ok'],
  properties: { slug: { type: 'string' }, ok: { type: 'boolean' },
    nClaims: { type: 'number' }, error: { type: 'string' } },
}
phase('Extract')
const results = await pipeline(args.sources, (s) =>
  agent(
`Fetch ${s.url} with WebFetch (retry once on failure). Extract every distinct claim relevant to the SDLC of AI agents — either sense: (a) building/testing/evaling/deploying/monitoring agent systems, (b) AI coding agents driving the development lifecycle. Typically 5-15 claims.
Write the notes as JSON to ${args.notesDir}/${s.slug}.json with exactly this shape:
{"slug": "${s.slug}", "url": "${s.url}", "title": "${s.title}", "org": "${s.org}", "date": "${s.date || ''}", "meanings": ${JSON.stringify(s.meanings)}, "claims": [{"id": "c1", "claim": "<one factual claim, own words>", "quote": "<short VERBATIM supporting quote from the page>"}], "practices": ["<named practice, e.g. eval-driven development>"], "phases": ["<lifecycle phase touched, e.g. evaluation, deployment>"], "terms": ["<key term>"]}
Rules: quotes must be verbatim; no claims beyond the page; if both fetches fail, write nothing and return ok=false with the error. Return ok=true and nClaims when written.`,
    { label: 'extract:' + s.slug, schema: RESULT, model: 'haiku', effort: 'low' }))
return { results: results.filter(Boolean) }
```

- [ ] **Step 2: Handle failures via the reserve**

For every result with `ok=false` (or a missing notes file — verify with `ls "$WORK/notes" | wc -l`): report it visibly, pick a same-family replacement from `reserve` in `$WORK/sources.json`, move it into `selected`, and run a single-source rerun of the same workflow with `args.sources` = just the replacements. Repeat until `selected` count (25–40) is met or the reserve is exhausted; if the count drops below 25, say so explicitly rather than proceeding silently.

- [ ] **Step 3: Update sources.json**

Rewrite `$WORK/sources.json` so `selected` reflects reality (replacements swapped in, hard failures removed). Spot-check two notes files by reading them: quotes present, claims sensible.

---

### Task 6: Taxonomy, bundle skeleton, and source concepts

**Files:**
- Create: `$WORK/taxonomy.json`, `$WORK/gen_sources.py`
- Create: `docs/okf/agent-sdlc/index.md`, `docs/okf/agent-sdlc/log.md`, `docs/okf/agent-sdlc/{sources,phases,practices,perspectives,comparisons}/index.md`, `docs/okf/agent-sdlc/sources/<slug>.md` (one per selected source)

**Interfaces:**
- Consumes: all `$WORK/notes/*.json`.
- Produces: `$WORK/taxonomy.json` — `{"concepts": [{slug, path, type, title, outline, tags: [..], noteFiles: [abs paths], related: [bundle-relative paths like "comparisons/two-meanings.md"], model: "opus"|"fable"}]}`. Task 7 consumes it verbatim.

- [ ] **Step 1: Design the taxonomy (deep reasoning, inline)**

Read every `$WORK/notes/*.json` (they are small). Merge the `phases`/`practices` vocabularies across sources into a final concept list:

- `phases/` — expect roughly 5–8 (e.g. design/scoping, prototyping, evaluation, testing, deployment, monitoring-agentops); merge synonyms.
- `practices/` — expect roughly 8–15 named practices, each backed by ≥2 sources where possible.
- `perspectives/` — exactly 4: `google.md`, `anthropic.md`, `langchain.md`, `academia.md`.
- `comparisons/` — at least `two-meanings.md` (building-agents vs agent-run-sdlc: where they differ, where they converge); add `comparisons/` siblings only if the evidence forces one.

For each concept record: `type` (`Lifecycle Phase` / `Practice` / `Perspective` / `Comparison`), a 2–4 sentence `outline` of what the file should argue, the `noteFiles` that evidence it, `related` cross-links, `tags` (subset of `[building-agents, agent-run-sdlc]`), and `model` (`fable` for `comparisons/two-meanings.md`, `opus` for everything else). Write `$WORK/taxonomy.json`.

- [ ] **Step 2: Generate source concept files mechanically**

Create `$WORK/gen_sources.py`:

```python
#!/usr/bin/env python3
"""Generate OKF source concept files from extraction notes. Deterministic; no model."""
import json
import sys
from pathlib import Path

notes_dir, out_dir, date = sys.argv[1], sys.argv[2], sys.argv[3]
out = Path(out_dir)
out.mkdir(parents=True, exist_ok=True)
for nf in sorted(Path(notes_dir).glob("*.json")):
    n = json.loads(nf.read_text(encoding="utf-8"))
    published = f" (published {n['date']})" if n.get("date") else ""
    lines = [
        "---",
        "type: Source",
        f"title: {n['title']}",
        f"description: {n['org']} material on the SDLC of AI agents.",
        f"resource: {n['url']}",
        f"org: {n['org']}",
        f"tags: [{', '.join(n.get('meanings', []))}]",
        f"timestamp: {date}T00:00:00Z",
        "---",
        "",
        "# Summary",
        "",
        f"First-party material by {n['org']}{published}. Key claims extracted below;"
        f" the live document is the authority.",
        "",
        "# Key claims",
        "",
    ]
    lines += [f"- {c['claim']}" for c in n.get("claims", [])]
    lines.append("")
    (out / f"{n['slug']}.md").write_text("\n".join(lines), encoding="utf-8")
    print(n["slug"])
```

Run: `python3 "$WORK/gen_sources.py" "$WORK/notes" docs/okf/agent-sdlc/sources 2026-07-06`
Expected: one slug per line, count == selected count.

- [ ] **Step 3: Write the skeleton indexes**

Write `docs/okf/agent-sdlc/index.md`:

```markdown
---
okf_version: "0.1"
---
# Agent SDLC — knowledge bundle

Research findings on the software development lifecycle for AI agents, in both
senses of the phrase, compiled from first-party sources (Google, Anthropic,
LangChain, peer-reviewed research). See [the comparison](/comparisons/two-meanings.md)
for how the two senses relate.

- [Sources](/sources/index.md) — the research sources (provenance nodes)
- [Lifecycle phases](/phases/index.md)
- [Practices](/practices/index.md)
- [Perspectives](/perspectives/index.md) — per-org syntheses
- [Comparisons](/comparisons/index.md)
```

Write `docs/okf/agent-sdlc/log.md`:

```markdown
# Change log

## 2026-07-06

- Initial bundle: research sweep of Google, Anthropic, LangChain, and
  peer-reviewed sources; see /sources/index.md for the full list.
```

Write each subdirectory `index.md` (no frontmatter) listing its concept files as root-relative links — for `sources/index.md` generate the list with:

```bash
ls docs/okf/agent-sdlc/sources/*.md | grep -v index.md | sed 's|docs/okf/agent-sdlc||;s|\(.*\)/\(.*\)\.md|- [\2](\1/\2.md)|'
```

For `phases/`, `practices/`, `perspectives/`, `comparisons/`, list the paths from `$WORK/taxonomy.json` (the files don't exist yet — the checker is not run in this task).

- [ ] **Step 4: Commit the scaffold**

```bash
git add docs/okf/agent-sdlc
git commit -m "docs(okf): scaffold agent-sdlc bundle with source concepts"
```

---

### Task 7: Synthesis workflow

**Files:**
- Create: `docs/okf/agent-sdlc/phases/*.md`, `practices/*.md`, `perspectives/*.md`, `comparisons/*.md` (per taxonomy)

**Interfaces:**
- Consumes: `$WORK/taxonomy.json` `concepts`, `$WORK/notes/*.json`.
- Produces: every concept file listed in the taxonomy, OKF-conformant, every claim cited to `/sources/<slug>.md`.

- [ ] **Step 1: Run the synthesis workflow**

Invoke `Workflow` with `args: {"concepts": <taxonomy concepts array>, "bundleDir": "<absolute path to docs/okf/agent-sdlc>", "date": "2026-07-06"}` and this script:

```js
export const meta = {
  name: 'agent-sdlc-synthesize',
  description: 'Write OKF concept files from taxonomy + extraction notes',
  phases: [{ title: 'Write', detail: 'one writer per concept (opus; comparison on fable)' }],
}
const RESULT = {
  type: 'object', required: ['path', 'ok'],
  properties: { path: { type: 'string' }, ok: { type: 'boolean' }, error: { type: 'string' } },
}
phase('Write')
const results = await pipeline(args.concepts, (c) =>
  agent(
`Write the OKF concept file ${args.bundleDir}/${c.path}.
Concept: "${c.title}" (type: ${c.type}). Outline to develop: ${c.outline}
Evidence: FIRST read these extraction-notes files: ${c.noteFiles.join(' , ')}
Rules:
- YAML frontmatter, inline lists only: type: ${c.type}; title: ${c.title}; description: <one sentence>; tags: [${c.tags.join(', ')}]; timestamp: ${args.date}T00:00:00Z
- Every claim in the body must come from the notes. Cite inline with numbered refs like [1], and end the file with a "# Citations" heading listing them as numbered markdown links to root-relative source paths, e.g. "1. [Building Effective Agents](/sources/anthropic-building-effective-agents.md)".
- Cross-link related concepts inline with root-relative links: ${c.related.map(r => '/' + r).join(' , ')}
- No claims beyond the evidence; no filler. Dense, neutral, readable prose, 300-700 words of body.
Write the file, then return ok=true with its path.`,
    { label: 'write:' + c.slug, phase: 'Write', schema: RESULT, model: c.model, effort: 'high' }))
return { results: results.filter(Boolean) }
```

- [ ] **Step 2: Verify completeness and re-run gaps**

Compare written files against the taxonomy: `ls docs/okf/agent-sdlc/{phases,practices,perspectives,comparisons}/*.md`. Any concept missing or returned `ok=false` gets a single-concept rerun (same script, `args.concepts` = the gaps). Read `comparisons/two-meanings.md` yourself end-to-end — it is the centerpiece; if it doesn't genuinely compare the two senses, rewrite it inline (you are the fable tier).

- [ ] **Step 3: Commit**

```bash
git add docs/okf/agent-sdlc
git commit -m "docs(okf): add agent-sdlc phase/practice/perspective/comparison concepts"
```

---

### Task 8: Verification workflow + conformance

**Files:**
- Modify: any concept file with unsupported claims (fixed in place)

**Interfaces:**
- Consumes: all bundle concept files, `$WORK/notes/*.json`, `scripts/okf_check.py`.
- Produces: a verified bundle — checker passes, every claim traced.

- [ ] **Step 1: Run the verification workflow**

Invoke `Workflow` with `args: {"concepts": <taxonomy concepts array>, "bundleDir": "<absolute path to docs/okf/agent-sdlc>", "notesDir": "<absolute $WORK/notes>"}`:

```js
export const meta = {
  name: 'agent-sdlc-verify',
  description: 'Trace every claim in every concept file to its cited source',
  phases: [{ title: 'Verify', detail: 'one sonnet skeptic per concept file' }],
}
const VERDICT = {
  type: 'object', required: ['path', 'ok', 'edits'],
  properties: { path: { type: 'string' }, ok: { type: 'boolean' },
    checkedClaims: { type: 'number' },
    edits: { type: 'array', items: { type: 'object',
      required: ['claim', 'action', 'reason'],
      properties: { claim: { type: 'string' },
        action: { enum: ['reworded', 'removed', 'kept'] },
        reason: { type: 'string' } } } } },
}
phase('Verify')
const results = await pipeline(args.concepts, (c) =>
  agent(
`Adversarially verify ${args.bundleDir}/${c.path}. For EVERY claim in the body:
1. Find its citation and open the matching notes file in ${args.notesDir} (notes filename = source slug from the /sources/ link).
2. Check the claim is actually supported by that source's claims/quotes. For 2-3 of the strongest claims, also WebFetch the live source URL (the "url" field in the notes) and confirm the quote exists.
3. Fix the file in place: reword any over-claimed statement down to what the source supports, or delete it (and its citation entry if now orphaned; renumber). Do NOT add new claims.
Report every edit. A claim with no citation at all counts as unsupported.`,
    { label: 'verify:' + c.slug, phase: 'Verify', schema: VERDICT, model: 'sonnet', effort: 'high' }))
return { results: results.filter(Boolean) }
```

Summarize the edits to the user (file, action, reason). If any file lost most of its claims, decide inline whether the concept still stands; if not, delete the file, remove it from the relevant `index.md`, and note the drop in `log.md` under `## 2026-07-06`.

- [ ] **Step 2: Run the conformance checker**

Run: `python3 scripts/okf_check.py docs/okf/agent-sdlc`
Expected: `OK`. Fix every reported error inline (broken links from renumbering are the likely culprits) and re-run until clean.

- [ ] **Step 3: Commit**

```bash
git add docs/okf/agent-sdlc
git commit -m "docs(okf): verify agent-sdlc claims against sources; conformance clean"
```

---

### Task 9: Finalize

**Files:**
- Modify: `docs/okf/agent-sdlc/index.md`, subdirectory `index.md` files, `log.md`

**Interfaces:**
- Consumes: the whole bundle + `scripts/okf_check.py`.
- Produces: the shipped bundle; acceptance criteria all green.

- [ ] **Step 1: Reconcile the indexes**

Regenerate every subdirectory `index.md` against the files that actually exist (Task 8 may have dropped concepts). Give the root `index.md` a final read: does its navigation reflect the real bundle? Add one sentence per section describing what's inside.

- [ ] **Step 2: Acceptance sweep**

Verify each acceptance criterion from the spec, reporting evidence for each:

```bash
python3 scripts/okf_check.py docs/okf/agent-sdlc          # expect: OK
ls docs/okf/agent-sdlc/sources/*.md | grep -vc index      # expect: 25-40
python3 scripts/test_okf_check.py                          # expect: OK
```

Plus by inspection: both meanings covered (grep `tags:` across concepts — both tags appear in non-source concepts); `comparisons/two-meanings.md` exists and compares; every source URL is on the allowlist (re-check `$WORK/sources.json` hosts).

- [ ] **Step 3: Final commit and wrap-up**

```bash
git add docs/okf/agent-sdlc
git commit -m "docs(okf): finalize agent-sdlc bundle indexes and log"
```

Report to the user: bundle stats (files per directory, source count per org/meaning), the checker output, and the two out-of-scope follow-ups from the spec (graphify ingestion; serving the bundle to the runtime's own agent).
