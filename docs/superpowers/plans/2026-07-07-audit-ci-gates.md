# Audit Drain Cluster 1 — CI & Gates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close audit findings 11.1 (conditional src-tauri CI leg), 9.2+10.1 (okf_check gated in ci.sh), 10.2 (coverage number persisted to the Actions step summary), and 9.3 (four okf_check extensions) from `docs/superpowers/specs/2026-07-07-audit-drain-action-plan-design.md`.

**Architecture:** Extend `scripts/okf_check.py` first (TDD against `scripts/test_okf_check.py`, one check per task), verify the live bundle still passes, then wire the checker and a conditional src-tauri leg into `scripts/ci.sh`, and finally persist the coverage number in `.github/workflows/ci.yml`. Checker before gate, so the gate lands enforcing the finished checker.

**Tech Stack:** Python 3 stdlib (`re`, `pathlib`, `unittest`), bash, GitHub Actions YAML, cargo (two separate workspaces).

**Branch:** `feature/audit-ci-gates` off `main` (create via superpowers:using-git-worktrees at execution start).

## Global Constraints

- `source ~/.cargo/env` first if `cargo` is not on PATH.
- Two separate Cargo workspaces: `agent/` and `src-tauri/`. Never run `cargo fmt` on `src-tauri` (hand-formatted by convention — see CLAUDE.md).
- Conventional commits: `type(scope): summary`.
- The coverage job stays `continue-on-error: true` — it is a tracked number, never a gate (spec 2026-07-02 eval-flywheel §5).
- okf_check's frontmatter parser accepts only flat `key: value` lines and **inline** lists (`tags: [a, b]`) — keep all test fixtures in that shape.
- Code excerpts below were read from live source at `b243a6d`; if a hunk doesn't match, re-read the file before editing.
- Pre-verified facts: every bundle node is listed in its directory index, and every `[n]` marker resolves to a numbered Citations entry — so the extended checker is expected to pass the live bundle unchanged.

---

### Task 1: okf_check — require `resource:` on Source nodes (finding 9.3a)

**Files:**
- Modify: `scripts/okf_check.py` (frontmatter branch, currently lines 87-94)
- Test: `scripts/test_okf_check.py`
- Modify: `.superpowers/sdd/progress.md` (open the cluster ledger section)

**Interfaces:**
- Produces: an error string of the form `{rel}: Source node missing `resource` URL` — Task 2 restructures this same branch, Task 5 documents it.

- [ ] **Step 1: Open the cluster ledger section**

Append to `.superpowers/sdd/progress.md`:

```markdown

## audit-drain cluster 1 — CI & gates (feature/audit-ci-gates)

Spec: docs/superpowers/specs/2026-07-07-audit-drain-action-plan-design.md (Cluster 1).
Plan: docs/superpowers/plans/2026-07-07-audit-ci-gates.md.
Findings: 11.1 (src-tauri leg), 9.2+10.1 (okf leg), 10.2 (coverage persistence), 9.3 (okf_check extensions).
- STARTED <today's date>.
```

- [ ] **Step 2: Write the failing test**

In `scripts/test_okf_check.py`, add inside `class OkfCheckTest`:

```python
    def test_source_missing_resource_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/no_resource.md",
              "---\ntype: Source\ntitle: X\n---\n# Summary\nbody\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("no_resource.md" in e and "resource" in e for e in errs))
```

Also update `test_external_links_ignored` (it asserts a clean bundle, and its `ext.md` fixture is a Source with no `resource:` — it would start failing): replace its `write(...)` call with:

```python
        write(self.root, "sources/ext.md",
              "---\ntype: Source\nresource: https://example.com/ext\n---\n"
              "See [site](https://example.com/x) and [anchor](#schema)\n")
```

Likewise `test_broken_link_fails` and `test_citations_without_source_links_fail` write fixture nodes lacking `resource:`/vocabulary-valid types, but they assert with `any(...)`, so extra errors are harmless — leave them.

- [ ] **Step 3: Run test to verify it fails**

Run: `python3 scripts/test_okf_check.py -v 2>&1 | tail -5`
Expected: `test_source_missing_resource_fails ... FAIL` (the others pass).

- [ ] **Step 4: Implement the check**

In `scripts/okf_check.py`, the non-reserved frontmatter branch currently ends:

```python
                fm = parse_frontmatter(fm_text)
                if fm is None:
                    errors.append(f"{rel}: unparseable frontmatter")
                elif not str(fm.get("type", "")).strip():
                    errors.append(f"{rel}: missing or empty `type`")
```

Add one more branch:

```python
                fm = parse_frontmatter(fm_text)
                if fm is None:
                    errors.append(f"{rel}: unparseable frontmatter")
                elif not str(fm.get("type", "")).strip():
                    errors.append(f"{rel}: missing or empty `type`")
                elif (str(fm.get("type")).strip() == "Source"
                      and not str(fm.get("resource", "")).strip()):
                    errors.append(f"{rel}: Source node missing `resource` URL")
```

- [ ] **Step 5: Run the full suite to verify it passes**

Run: `python3 scripts/test_okf_check.py`
Expected: `OK` (10 tests).

- [ ] **Step 6: Commit**

```bash
git add scripts/okf_check.py scripts/test_okf_check.py .superpowers/sdd/progress.md
git commit -m "feat(okf): require resource: URL on Source nodes (audit 9.3a)"
```

---

### Task 2: okf_check — validate `type` against the authoring vocabulary (finding 9.3b)

**Files:**
- Modify: `scripts/okf_check.py` (module constants + the branch Task 1 touched)
- Test: `scripts/test_okf_check.py`

**Interfaces:**
- Consumes: the `elif Source/resource` branch from Task 1 (restructured here into an `else:` block).
- Produces: module constant `ALLOWED_TYPES` (a `set`); error form `{rel}: unknown `type` '...' (allowed: ...)`.

- [ ] **Step 1: Write the failing test**

Add to `scripts/test_okf_check.py` (this mirrors the audit's own demonstration that `type: Sorce` passes today):

```python
    def test_unknown_type_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/typo.md",
              "---\ntype: Sorce\nresource: https://example.com/t\n---\nbody\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("typo.md" in e and "Sorce" in e for e in errs))
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python3 scripts/test_okf_check.py -v 2>&1 | tail -5`
Expected: `test_unknown_type_fails ... FAIL`.

- [ ] **Step 3: Implement**

In `scripts/okf_check.py`, add a module constant next to `CITATION_DIRS` (vocabulary from `.agents/skills/agent-sdlc/authoring.md`):

```python
ALLOWED_TYPES = {"Source", "Practice", "Lifecycle Phase", "Perspective", "Comparison"}
```

Restructure the branch from Task 1 into:

```python
                fm = parse_frontmatter(fm_text)
                if fm is None:
                    errors.append(f"{rel}: unparseable frontmatter")
                elif not str(fm.get("type", "")).strip():
                    errors.append(f"{rel}: missing or empty `type`")
                else:
                    node_type = str(fm.get("type")).strip()
                    if node_type not in ALLOWED_TYPES:
                        errors.append(
                            f"{rel}: unknown `type` {node_type!r} "
                            f"(allowed: {', '.join(sorted(ALLOWED_TYPES))})")
                    if (node_type == "Source"
                            and not str(fm.get("resource", "")).strip()):
                        errors.append(f"{rel}: Source node missing `resource` URL")
```

- [ ] **Step 4: Run the full suite**

Run: `python3 scripts/test_okf_check.py`
Expected: `OK` (11 tests). Note `test_non_root_index_frontmatter_fails` writes `type: Index` but asserts with `any(...)` — the extra unknown-type error is harmless.

- [ ] **Step 5: Commit**

```bash
git add scripts/okf_check.py scripts/test_okf_check.py
git commit -m "feat(okf): validate node type against the authoring vocabulary (audit 9.3b)"
```

---

### Task 3: okf_check — resolve body `[n]` markers against Citations entries (finding 9.3c)

**Files:**
- Modify: `scripts/okf_check.py` (regex constants + the citations branch, currently lines 110-121)
- Test: `scripts/test_okf_check.py`

**Interfaces:**
- Produces: module constants `MARKER_RE`, `CITATION_ENTRY_RE`; error form `{rel}: citation marker(s) with no numbered Citations entry: [2]`.

- [ ] **Step 1: Write the failing test**

```python
    def test_unresolved_citation_marker_fails(self):
        valid_bundle(self.root)
        write(self.root, "practices/dangling.md",
              "---\ntype: Practice\n---\nClaim [1] and claim [2].\n\n"
              "# Citations\n1. [example](/sources/example.md)\n")
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("dangling.md" in e and "marker" in e for e in errs))
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python3 scripts/test_okf_check.py -v 2>&1 | tail -5`
Expected: `test_unresolved_citation_marker_fails ... FAIL`.

- [ ] **Step 3: Implement**

Add two regex constants next to the existing ones:

```python
MARKER_RE = re.compile(r"\[(\d+)\](?!\()")          # [3] but not [3](link)
CITATION_ENTRY_RE = re.compile(r"^\s*(\d+)\.\s", re.MULTILINE)
```

The citations branch currently reads:

```python
            else:
                section = body[m.end():]
                nxt = NEXT_HEADING_RE.search(section)
                if nxt:
                    section = section[:nxt.start()]
                cites = [t for t in iter_links(section) if t.startswith("/sources/")]
                if not cites:
                    errors.append(f"{rel}: # Citations has no /sources/ links")
```

Extend it (markers are collected only from the body *before* the Citations heading; entries only allow markers to resolve — unreferenced entries stay legal):

```python
            else:
                section = body[m.end():]
                nxt = NEXT_HEADING_RE.search(section)
                if nxt:
                    section = section[:nxt.start()]
                cites = [t for t in iter_links(section) if t.startswith("/sources/")]
                if not cites:
                    errors.append(f"{rel}: # Citations has no /sources/ links")
                markers = set(MARKER_RE.findall(body[:m.start()]))
                entries = set(CITATION_ENTRY_RE.findall(section))
                missing = sorted(markers - entries, key=int)
                if missing:
                    errors.append(
                        f"{rel}: citation marker(s) with no numbered Citations entry: "
                        + ", ".join(f"[{n}]" for n in missing))
```

- [ ] **Step 4: Run the full suite**

Run: `python3 scripts/test_okf_check.py`
Expected: `OK` (12 tests) — `valid_bundle`'s `practices/evals.md` has marker `[1]` and entry `1.`, so it stays clean.

- [ ] **Step 5: Commit**

```bash
git add scripts/okf_check.py scripts/test_okf_check.py
git commit -m "feat(okf): resolve body [n] markers against numbered Citations entries (audit 9.3c)"
```

---

### Task 4: okf_check — directory index must list every node (finding 9.3d)

**Files:**
- Modify: `scripts/okf_check.py` (new pass after the per-file loop, before `return errors` in `check_bundle`)
- Test: `scripts/test_okf_check.py`

**Interfaces:**
- Produces: error form `{dir}/index.md: does not list {node}.md`. Applies to every non-root `index.md`; a directory *without* an index.md is not checked (YAGNI — all five bundle dirs have one).

- [ ] **Step 1: Write the failing test**

```python
    def test_index_missing_node_fails(self):
        valid_bundle(self.root)
        write(self.root, "sources/unlisted.md", VALID_SOURCE)
        errs = okf_check.check_bundle(self.root)
        self.assertTrue(any("sources/index.md" in e and "unlisted.md" in e
                            for e in errs))
```

Also update `test_external_links_ignored` again (its clean-bundle assertion now needs `ext.md` listed): replace the whole method with:

```python
    def test_external_links_ignored(self):
        valid_bundle(self.root)
        write(self.root, "sources/ext.md",
              "---\ntype: Source\nresource: https://example.com/ext\n---\n"
              "See [site](https://example.com/x) and [anchor](#schema)\n")
        write(self.root, "sources/index.md",
              "# Sources\n- [example](/sources/example.md)\n- [ext](/sources/ext.md)\n")
        self.assertEqual(okf_check.check_bundle(self.root), [])
```

- [ ] **Step 2: Run test to verify it fails**

Run: `python3 scripts/test_okf_check.py -v 2>&1 | tail -6`
Expected: `test_index_missing_node_fails ... FAIL` (and `test_external_links_ignored ... ok`).

- [ ] **Step 3: Implement**

In `check_bundle`, insert before `return errors`:

```python
    for idx in md_files:
        if idx.name != "index.md" or idx.parent == root:
            continue
        _, idx_body = split_frontmatter(idx.read_text(encoding="utf-8"))
        listed = set()
        for target in iter_links(idx_body):
            if target.startswith("/"):
                listed.add((root / target.lstrip("/")).resolve())
            else:
                listed.add((idx.parent / target).resolve())
        for sib in sorted(idx.parent.glob("*.md")):
            if sib.name in RESERVED:
                continue
            if sib.resolve() not in listed:
                rel = idx.relative_to(root).as_posix()
                errors.append(f"{rel}: does not list {sib.name}")
```

- [ ] **Step 4: Run the full suite**

Run: `python3 scripts/test_okf_check.py`
Expected: `OK` (13 tests).

- [ ] **Step 5: Commit**

```bash
git add scripts/okf_check.py scripts/test_okf_check.py
git commit -m "feat(okf): require directory index.md to list every node (audit 9.3d)"
```

---

### Task 5: Live-bundle validation + document the checks and the human-duty boundary

**Files:**
- Modify: `scripts/okf_check.py` (module docstring only)
- Modify: `.agents/skills/agent-sdlc/authoring.md` (after the "Workflow for any change" section)
- Modify (only if the live run fails): `docs/okf/agent-sdlc/**/index.md` + `docs/okf/agent-sdlc/log.md`

**Interfaces:**
- Consumes: the four checks from Tasks 1-4.

- [ ] **Step 1: Run the extended checker against the live bundle**

Run: `python3 scripts/okf_check.py docs/okf/agent-sdlc`
Expected: `OK` (pre-verified: all nodes indexed, all markers resolve, all 36 Sources carry `resource:`, all types in vocabulary). If any error prints, it is a real bundle-conformance gap: fix the named index/node, add a dated entry to `docs/okf/agent-sdlc/log.md`, and re-run until `OK`.

- [ ] **Step 2: Update the checker docstring**

Replace the numbered "Checks:" list in the `scripts/okf_check.py` docstring with:

```
Checks:
1. every non-reserved .md file has parseable YAML frontmatter with a non-empty `type`
2. index.md files carry no frontmatter, except the bundle-root index.md, which may
   declare only `okf_version`; log.md carries no frontmatter
3. all intra-bundle markdown links resolve to existing files inside the bundle
4. every concept under phases/, practices/, perspectives/, comparisons/ has a
   `# Citations` section containing at least one resolving link into /sources/
5. `type` is one of the authoring vocabulary (Source, Practice, Lifecycle Phase,
   Perspective, Comparison)
6. every `type: Source` node carries a non-empty `resource:` URL
7. body `[n]` citation markers resolve to a numbered entry in # Citations
8. every non-root directory index.md lists every non-reserved node in its directory

NOT checked (human duty): whether a node's claims still match its live source —
semantic drift needs periodic human re-verification, recorded as a dated log.md entry.
```

- [ ] **Step 3: Add the human-duty note to authoring.md**

In `.agents/skills/agent-sdlc/authoring.md`, directly after the "Workflow for any change" numbered list, add:

```markdown

## What the checker does not catch

`okf_check.py` verifies structure only (frontmatter shape, type vocabulary,
`resource:` on Sources, link/citation/index integrity). Whether a node's claims
still match its live `resource:` is **semantic drift** — re-verify against the
source periodically and record the pass as a dated `log.md` entry.
```

- [ ] **Step 4: Verify suite + checker both green**

Run: `python3 scripts/test_okf_check.py && python3 scripts/okf_check.py docs/okf/agent-sdlc`
Expected: `OK` twice.

- [ ] **Step 5: Commit**

```bash
git add scripts/okf_check.py .agents/skills/agent-sdlc/authoring.md
git commit -m "docs(okf): document checks 5-8 and the semantic-drift human duty (audit 9.3)"
```

(Include any bundle index/log fixes from Step 1 in this commit and mention them in the message.)

---

### Task 6: Gate the OKF bundle in ci.sh (findings 9.2 + 10.1)

**Files:**
- Modify: `scripts/ci.sh`

**Interfaces:**
- Consumes: `scripts/test_okf_check.py` (unittest, exits non-zero on failure) and `scripts/okf_check.py <bundle>` (exits 1 on errors) from Tasks 1-5.
- Produces: an `okf bundle check` leg that Task 9's full-gate run exercises.

- [ ] **Step 1: Add the leg**

In `scripts/ci.sh`, insert after the `.cargo/env` line and before the `cargo fmt` block (first leg — it is sub-second, fail-fast):

```bash
echo "==> okf bundle check"
python3 scripts/test_okf_check.py
python3 scripts/okf_check.py docs/okf/agent-sdlc
```

- [ ] **Step 2: Verify syntax and the leg in isolation**

Run: `bash -n scripts/ci.sh && python3 scripts/test_okf_check.py && python3 scripts/okf_check.py docs/okf/agent-sdlc`
Expected: no syntax error, `OK` twice.

- [ ] **Step 3: Commit**

```bash
git add scripts/ci.sh
git commit -m "ci: gate the OKF bundle (test_okf_check + okf_check) in ci.sh (audit 9.2/10.1)"
```

---

### Task 7: Conditional src-tauri leg in ci.sh (finding 11.1)

**Files:**
- Modify: `scripts/ci.sh` (new leg + header comment rewrite)

**Interfaces:**
- Produces: a src-tauri leg that runs clippy `-D warnings` + `cargo test` when GTK/WebKitGTK dev deps are present (dev machines) and prints an explicit SKIPPED line when absent (GitHub runner) — preserving the original runner rationale. clippy `--all-targets` compiles the workspace, and `cargo test` builds it for real, so the spec's "build" is covered without a separate `cargo build`.

- [ ] **Step 1: Rewrite the header comment**

The header currently claims a blanket exclusion:

```bash
# Single source of truth for the CI gate — run by .githooks/pre-push and
# .github/workflows/ci.yml. src-tauri is intentionally excluded (GTK deps).
```

Replace with:

```bash
# Single source of truth for the CI gate — run by .githooks/pre-push and
# .github/workflows/ci.yml. src-tauri runs conditionally: it needs GTK/WebKitGTK
# dev deps (absent on the GitHub runner, present on dev machines). Its fmt is
# never checked — src-tauri is hand-formatted by convention (CLAUDE.md).
```

- [ ] **Step 2: Add the conditional leg**

Insert after the `cargo test` block (agent workspace) and before the web block:

```bash
if command -v pkg-config >/dev/null 2>&1 \
   && pkg-config --exists gtk+-3.0 webkit2gtk-4.1 2>/dev/null; then
  echo "==> src-tauri clippy + test"
  (cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings)
  (cd src-tauri && cargo test --workspace)
else
  echo "==> src-tauri: SKIPPED (GTK/WebKitGTK dev deps not found)"
fi
```

Note: the live-environment tests in `src-tauri/tests/` (gui_smoke, smoke_context_explorer) are `#[ignore]`d and llama_health is hermetic (wiremock), so `cargo test --workspace` is headless-safe.

- [ ] **Step 3: Verify the leg runs locally**

Run: `bash -n scripts/ci.sh` then, from the repo root:
`(cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings) && (cd src-tauri && cargo test --workspace)`
Expected: clippy clean, all non-ignored tests pass (first run compiles for several minutes; that is normal).

- [ ] **Step 4: Commit**

```bash
git add scripts/ci.sh
git commit -m "ci: conditional src-tauri clippy+test leg, skipped without GTK deps (audit 11.1)"
```

---

### Task 8: Persist the coverage number to the step summary (finding 10.2)

**Files:**
- Modify: `.github/workflows/ci.yml` (the `Coverage summary` step only)

**Interfaces:**
- Consumes: the existing `coverage` job (job-level `continue-on-error: true` — do not touch).

- [ ] **Step 1: Rewrite the coverage step**

Replace:

```yaml
      - name: Coverage summary
        working-directory: agent
        run: cargo llvm-cov --workspace --summary-only
```

with:

```yaml
      - name: Coverage summary
        working-directory: agent
        run: |
          cargo llvm-cov --workspace --summary-only | tee /tmp/llvm-cov-summary.txt
          {
            echo '### cargo llvm-cov (agent workspace)'
            echo '```'
            grep -E '^(Filename|TOTAL)' /tmp/llvm-cov-summary.txt
            echo '```'
          } >> "$GITHUB_STEP_SUMMARY"
```

- [ ] **Step 2: Verify the embedded script and YAML shape**

No PyYAML/actionlint on this machine, so: extract the run block into a scratch file and `bash -n` it, and confirm the YAML indentation matches the sibling steps by eye (2-space step members, `run: |` block scalar):

```bash
cat > /tmp/claude-cov-check.sh <<'EOF'
cargo llvm-cov --workspace --summary-only | tee /tmp/llvm-cov-summary.txt
{
  echo '### cargo llvm-cov (agent workspace)'
  echo '```'
  grep -E '^(Filename|TOTAL)' /tmp/llvm-cov-summary.txt
  echo '```'
} >> "$GITHUB_STEP_SUMMARY"
EOF
bash -n /tmp/claude-cov-check.sh
```

Expected: exit 0. Real-run confirmation happens on the first push to GitHub (the job is non-gating, so a mistake cannot block a merge; check the run's Summary tab shows the TOTAL line).

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: persist llvm-cov TOTAL to the Actions step summary (audit 10.2)"
```

---

### Task 9: Full gate, ledger close, and branch finish

**Files:**
- Modify: `.superpowers/sdd/progress.md` (cluster section status lines)

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Run the full gate (now including both new legs)**

Run: `bash scripts/ci.sh`
Expected: legs in order — okf bundle check (`OK` ×2), cargo fmt/clippy/test (agent), `==> src-tauri clippy + test` (this machine has the GTK deps), web typecheck + vitest — ending `CI gate passed.` The known-noise `act(...)` stderr warning from `App.tauri.test.tsx` is documented and fine.

- [ ] **Step 2: Update the ledger**

Append to the cluster section in `.superpowers/sdd/progress.md`:

```markdown
- All 4 findings implemented (9.3 a-d as okf_check checks 5-8 + 13-test suite; 9.2/10.1 okf leg; 11.1 conditional src-tauri leg; 10.2 step-summary persistence). Full ci.sh green including the new legs.
- BRANCH READY for finishing-a-development-branch.
```

```bash
git add .superpowers/sdd/progress.md
git commit -m "docs(sdd): cluster-1 ledger — all findings implemented, gate green"
```

- [ ] **Step 3: Finish the branch**

Invoke superpowers:finishing-a-development-branch (whole-branch review, then `--no-ff` merge to `main` on approval, branch deletion, ledger MERGED stamp with the merge commit hash).

- [ ] **Step 4: Post-merge re-stamps**

After the merge lands on `main`, append dated re-stamp entries for findings 11.1, 9.2+10.1, 10.2, and 9.3 to `.agents/skills/harness-engineering/audit.md`, matching the existing re-stamp format there (read the tail of the file first; one line per finding: date, finding id, one-line fix summary, merge commit). Commit as `docs(skills): re-stamp audit findings 11.1/9.2+10.1/10.2/9.3 (cluster 1 merged)`.

---

## Verification summary

| Finding | Proof it's closed |
|---|---|
| 9.3 | `test_okf_check.py` 13 tests green; live bundle `OK` under checks 5-8 |
| 9.2+10.1 | `bash scripts/ci.sh` runs the okf leg first and fails the gate on bundle errors |
| 11.1 | ci.sh runs src-tauri clippy+test on GTK machines, prints explicit SKIPPED otherwise |
| 10.2 | coverage job tees the summary and writes Filename/TOTAL lines to `$GITHUB_STEP_SUMMARY` (confirmed on first push) |
