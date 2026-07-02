# Examples Context Type Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-skill worked exemplars — an `examples/` directory convention surfaced through the existing skill progressive-disclosure levels (L1 marker, L2 section with an imitate-don't-copy contract, L3 unchanged) — closing the deep audit's last absent capability.

**Architecture:** `Skill` gains an `examples: Vec<PathBuf>` subset of its bundled files (populated at load by an `examples/` path-component filter). `use_skill` renders a distinct `## Examples` section and switches all listings to skill-relative paths (the form `read_skill_file` consumes), stating the absolute skill dir once in the bundled-files header for script execution. `list_skills` gains an `[N examples]` suffix; `SKILLS_AWARENESS` gains one sentence; `create_skill` teaches the convention in prose only. Everything else (guard, L3 read, caps) is untouched.

**Tech Stack:** Rust, agent-skills crate only (+ one doc paragraph). Inline module tests (the crate's pattern; no tests/ dir).

**Spec:** `docs/superpowers/specs/2026-07-02-examples-context-type-design.md` (H1–H7).

## Global Constraints

- A skill without `examples/` behaves byte-identically at L1 and at L2 *except* for the H4 path-form change (absolute → relative + dir-in-header), which applies to all skills and is the only sanctioned output change.
- Static budget grows by exactly ONE sentence (the `SKILLS_AWARENESS` addition). No new tools, no injection, no new caps, no config changes.
- Path logic must compare path COMPONENTS via `strip_prefix(&skill.dir)`, never string prefixes (separator safety).
- All work in `agent/crates/agent-skills/` + one doc paragraph; nothing else changes. Run cargo from `/home/kalen/rust-agent-runtime/agent` (`source ~/.cargo/env` if missing).
- Conventional commits; TDD.

---

### Task 1: `Skill.examples` — model + registry population

**Files:**
- Modify: `agent/crates/agent-skills/src/skill.rs` (struct ~line 4)
- Modify: `agent/crates/agent-skills/src/registry.rs` (`load_skill` ~line 92)
- Test: `registry.rs` `mod tests`

**Interfaces:**
- Produces: `Skill.examples: Vec<PathBuf>` — ABSOLUTE paths (consistent with `files`), sorted, exactly the members of `files` whose first path component relative to `skill.dir` is the directory `examples`. `files` remains the superset (L3 reading unchanged). Task 2 renders both relative via `strip_prefix(&skill.dir)`.

- [ ] **Step 1: Write the failing tests**

In `registry.rs` `mod tests` (reuse the module's existing temp-skill helpers — it already writes `SKILL.md` + bundled files into tempdirs):

```rust
    #[test]
    fn examples_are_the_examples_dir_subset_of_files() {
        let dir = tempfile::tempdir().unwrap();
        let sd = dir.path().join("demo");
        std::fs::create_dir_all(sd.join("examples/nested")).unwrap();
        std::fs::create_dir_all(sd.join("references")).unwrap();
        std::fs::write(sd.join("SKILL.md"), "---\ndescription: d\n---\nbody").unwrap();
        std::fs::write(sd.join("examples/a.md"), "A").unwrap();
        std::fs::write(sd.join("examples/nested/b.md"), "B").unwrap();
        std::fs::write(sd.join("references/r.md"), "R").unwrap();
        // A plain FILE named "examples" elsewhere must not confuse the filter:
        std::fs::write(sd.join("notes.md"), "N").unwrap();

        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let skill = reg.find("demo").expect("skill loads");
        let rel: Vec<String> = skill
            .examples
            .iter()
            .map(|p| p.strip_prefix(&skill.dir).unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(rel, vec!["examples/a.md".to_string(), "examples/nested/b.md".to_string()]);
        // Superset pin: files still contains every bundled file incl. examples.
        assert!(skill.files.len() >= 4, "{:?}", skill.files);
        for e in &skill.examples {
            assert!(skill.files.contains(e));
        }
    }

    #[test]
    fn no_examples_dir_means_empty_examples_and_a_plain_file_named_examples_does_not_count() {
        let dir = tempfile::tempdir().unwrap();
        let sd = dir.path().join("plain");
        std::fs::create_dir_all(&sd).unwrap();
        std::fs::write(sd.join("SKILL.md"), "---\ndescription: d\n---\nbody").unwrap();
        std::fs::write(sd.join("examples"), "just a file named examples").unwrap();
        let reg = SkillRegistry::new(vec![dir.path().to_path_buf()], dir.path().to_path_buf());
        let skill = reg.find("plain").expect("skill loads");
        assert!(skill.examples.is_empty(), "{:?}", skill.examples);
        assert_eq!(skill.files.len(), 1); // the odd file is still bundled
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-skills examples_are ; cargo test -p agent-skills no_examples_dir`
Expected: compile error — `Skill` has no field `examples`.

- [ ] **Step 3: Implement**

`skill.rs` — add to `Skill` (after `files`, with a doc comment):

```rust
    /// Worked exemplars: the subset of `files` under the skill's `examples/`
    /// directory (spec 2026-07-02 Examples context type, H1). Absolute paths,
    /// sorted; `files` remains the superset so L3 reading is unchanged.
    pub examples: Vec<PathBuf>,
```

`registry.rs` `load_skill` — after `list_bundled_files`:

```rust
        let files = list_bundled_files(dir);
        // Component-wise filter (separator-safe): first component under the
        // skill dir must be the DIRECTORY `examples`.
        let examples: Vec<PathBuf> = files
            .iter()
            .filter(|p| {
                p.strip_prefix(dir)
                    .ok()
                    .and_then(|rel| rel.components().next())
                    .map(|c| c.as_os_str() == "examples" && p.parent() != Some(dir))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
```

Note the guard `p.parent() != Some(dir)`: a plain FILE named `examples` sitting directly in the skill dir has first component `examples` too — the parent check excludes it (only paths *inside* an `examples/` directory have a deeper parent). Alternative equivalent: require `rel.components().count() >= 2`. Pick one, keep the comment.

Populate the struct (`examples` already sorted since `files` is sorted and `filter` preserves order). Fix any other `Skill { .. }` literal constructions the compiler finds (tests) with `examples: vec![]`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-skills`
Expected: PASS (2 new + all 42 existing).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-skills
git commit -m "feat(skills): Skill.examples — the examples/ subset of bundled files"
```

---

### Task 2: Tool surfacing — use_skill section, relative paths, list_skills marker, create_skill prose

**Files:**
- Modify: `agent/crates/agent-skills/src/tools.rs` (`ListSkills` output ~line 58, `UseSkill` output ~lines 123-134, `CreateSkill` descriptions ~lines 161-186)
- Test: `tools.rs` `mod tests`

**Interfaces:**
- Consumes: `Skill.examples` (Task 1).
- Produces: the L2 output contract Task 3's e2e test asserts:

```
# Skill: <name>

<body>

## Examples (worked exemplars)
- examples/<rel>
...
Read one with read_skill_file and imitate its shape and conventions; do not copy content verbatim.

## Bundled files (dir: <abs skill dir>)
- <rel>
...
Read a bundled file with read_skill_file (paths above are relative, as it expects); run a bundled script with execute_command using the dir above.
```

(Examples section only when non-empty; bundled section EXCLUDES examples and only appears when the remainder is non-empty; `list_skills` lines become `- name: description [N examples]` when N > 0.)

- [ ] **Step 1: Write the failing tests**

In `tools.rs` `mod tests` (reuse the module's existing tempdir + registry + tool-execution helpers and its ToolCtx fixture):

```rust
    #[tokio::test]
    async fn use_skill_renders_examples_section_and_relative_bundled_paths() {
        // Build a temp skill: SKILL.md + examples/a.md + references/r.md
        // (mirror the module's existing use_skill test setup).
        let (reg, _tmp) = /* module helper or inline setup as in existing tests */;
        // ... write files: examples/a.md, references/r.md ...
        let out = /* execute UseSkill { name: "demo" } via the existing pattern */;
        let c = &out.content;
        // Examples section, relative path, contract line:
        assert!(c.contains("## Examples (worked exemplars)"), "{c}");
        assert!(c.contains("- examples/a.md"), "{c}");
        assert!(c.contains("imitate its shape and conventions"), "{c}");
        assert!(c.contains("do not copy content verbatim"), "{c}");
        // Bundled section: relative path, dir in header, examples EXCLUDED:
        assert!(c.contains("## Bundled files (dir: "), "{c}");
        assert!(c.contains("- references/r.md"), "{c}");
        let bundled = c.split("## Bundled files").nth(1).unwrap();
        assert!(!bundled.contains("examples/a.md"), "examples double-listed: {c}");
        // No absolute paths in the LISTS (the only absolute path is the dir header):
        let after_body = c.split("## Examples").nth(1).unwrap();
        for line in after_body.lines().filter(|l| l.starts_with("- ")) {
            assert!(!line.contains(_tmp.path().to_str().unwrap()), "absolute path leaked: {line}");
        }
    }

    #[tokio::test]
    async fn use_skill_without_examples_has_no_examples_section() {
        let out = /* execute UseSkill on a skill with only references/r.md */;
        assert!(!out.content.contains("## Examples"), "{}", out.content);
        assert!(out.content.contains("## Bundled files (dir: "), "{}", out.content);
    }

    #[tokio::test]
    async fn list_skills_marks_example_bearing_skills() {
        // Two temp skills: "withex" (has examples/a.md), "plain" (none).
        let out = /* execute ListSkills via the existing pattern */;
        assert!(out.content.contains("withex:") && out.content.contains("[1 examples]"), "{}", out.content);
        let plain_line = out.content.lines().find(|l| l.contains("plain:")).unwrap();
        assert!(!plain_line.contains("examples"), "{plain_line}");
    }

    #[tokio::test]
    async fn create_skill_examples_round_trip_surfaces_the_section() {
        // Author via the tool (files: [{path: "examples/sample.md", ...}]), then
        // consume via UseSkill — the authoring→consumption loop from the spec.
        // (Setup per the module's existing create_skill round-trip test.)
        let _created = /* CreateSkill { name: "authored", description, body,
                          files: [{"path": "examples/sample.md", "content": "EX"}] } */;
        let out = /* UseSkill { name: "authored" } via the same registry */;
        assert!(out.content.contains("## Examples (worked exemplars)"), "{}", out.content);
        assert!(out.content.contains("- examples/sample.md"), "{}", out.content);
    }

    #[test]
    fn create_skill_schema_teaches_the_examples_convention() {
        let t = CreateSkill { /* as existing tests construct it */ };
        let s = t.schema();
        let text = serde_json::to_string(&s.parameters).unwrap();
        assert!(text.contains("examples/"), "{text}");
        assert!(t.description().contains("examples/"), "{}", t.description());
    }
```

(The `/* ... */` markers mean: transcribe the setup from the module's OWN existing tests for the same tool — the harness already exists there; do not invent a new one. The assertions above are the contract, verbatim.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-skills use_skill_renders ; cargo test -p agent-skills list_skills_marks ; cargo test -p agent-skills create_skill_schema_teaches`
Expected: FAIL — no Examples section / absolute paths / no marker / no convention prose.

- [ ] **Step 3: Implement in `tools.rs`**

`ListSkills` line rendering (~line 58):

```rust
            let mark = if s.examples.is_empty() {
                String::new()
            } else {
                format!(" [{} examples]", s.examples.len())
            };
            // "- {name}: {description}{mark}"
```

`UseSkill` output assembly (~lines 123-134) — replace the bundled-files block:

```rust
        let rel = |p: &std::path::Path| {
            p.strip_prefix(&skill.dir)
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_else(|_| p.to_string_lossy().into_owned())
        };
        if !skill.examples.is_empty() {
            out.push_str("\n\n## Examples (worked exemplars)\n");
            for p in &skill.examples {
                out.push_str(&format!("- {}\n", rel(p)));
            }
            out.push_str(
                "Read one with read_skill_file and imitate its shape and conventions; \
                 do not copy content verbatim.",
            );
        }
        let others: Vec<&std::path::PathBuf> =
            skill.files.iter().filter(|p| !skill.examples.contains(p)).collect();
        if !others.is_empty() {
            out.push_str(&format!("\n\n## Bundled files (dir: {})\n", skill.dir.display()));
            for p in others {
                out.push_str(&format!("- {}\n", rel(p)));
            }
            out.push_str(
                "Read a bundled file with read_skill_file (paths above are relative, as it \
                 expects); run a bundled script with execute_command using the dir above.",
            );
        }
```

(Adapt to the function's actual string-building style — it may use `format!` chains; keep the exact section headers and contract sentences.)

`CreateSkill`: append to the tool `description()`: `" Put worked exemplars under examples/ — they surface to consumers as a distinct Examples section with imitate-don't-copy guidance."` and to the `files` param description: `"Files under examples/ are surfaced as worked exemplars."`

Update any existing `use_skill` tests that pinned absolute paths to the new relative form (they are the sanctioned H4 change) — do not weaken their other assertions.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-skills && cargo test -p agent-runtime-config`
Expected: PASS (incl. the assemble contract ratchets — the schema description changes keep required-param descriptions non-empty).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-skills
git commit -m "feat(skills): surface examples at L1/L2 — section with imitate contract, relative paths, list marker, authoring prose"
```

---

### Task 3: Awareness sentence + end-to-end flow test + doc paragraph

**Files:**
- Modify: `agent/crates/agent-skills/src/presets.rs` (`SKILLS_AWARENESS` ~line 4)
- Modify: `agent/crates/agent-skills/src/tools.rs` (e2e test in `mod tests`)
- Modify: the skills doc — locate with `grep -rln "skills_dirs\|use_skill" agent/docs/` and add one paragraph to the most fitting file (expected: `agent/docs/RUNNING.md` or a skills doc it links)

**Interfaces:**
- Consumes: Tasks 1–2 (the full surfaced pipeline).

- [ ] **Step 1: Write the failing tests**

`presets.rs` — extend the existing awareness-constant test:

```rust
    // in the existing test asserting SKILLS_AWARENESS content, add:
    assert!(SKILLS_AWARENESS.contains("worked examples"), "{SKILLS_AWARENESS}");
```

`tools.rs` `mod tests` — the L1→L2→L3 flow:

```rust
    #[tokio::test]
    async fn examples_flow_l1_marker_l2_section_l3_read() {
        // One temp skill "flow" with examples/sample.md containing "EXEMPLAR BODY".
        // (setup per the module's existing multi-tool tests)
        let l1 = /* ListSkills */;
        assert!(l1.content.contains("[1 examples]"), "{}", l1.content);
        let l2 = /* UseSkill { name: "flow" } */;
        assert!(l2.content.contains("- examples/sample.md"), "{}", l2.content);
        let l3 = /* ReadSkillFile { skill: "flow", path: "examples/sample.md" } */;
        assert!(l3.content.contains("EXEMPLAR BODY"), "{}", l3.content);
        // Confinement unchanged:
        let escape = /* ReadSkillFile { skill: "flow", path: "../escape" } */;
        assert!(escape.is_err(), "escape not rejected");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p agent-skills examples_flow ; cargo test -p agent-skills -- awareness`
Expected: FAIL — no "worked examples" in the constant (flow test may already pass after Task 2 except the constant-independent parts; the awareness assert fails).

- [ ] **Step 3: Implement**

`presets.rs` — append ONE sentence to `SKILLS_AWARENESS`:

```
Some skills bundle worked examples — when a loaded skill lists them, read a relevant one before producing your first artifact of that kind.
```

Doc paragraph (at the located skills doc), under the skills section:

```markdown
### Skill examples

Put worked exemplars under a skill's `examples/` directory. They surface as a
distinct "Examples" section when the skill is loaded (with guidance to imitate
their shape, not copy content), get an `[N examples]` marker in `list_skills`,
and are read on demand with `read_skill_file` — nothing is injected into the
prompt until the model asks.
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p agent-skills && cargo test -p agent-runtime-config`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-skills agent/docs
git commit -m "feat(skills): examples awareness note + end-to-end disclosure flow pin + docs"
```

---

### Task 4: Workspace sweep + spec cross-check

**Files:** none expected; fix fallout only.

- [ ] **Step 1: Full CI**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: green. Fix fallout minimally; commit as `chore: workspace sweep for examples context type`.

- [ ] **Step 2: Spec cross-check**

Re-read `docs/superpowers/specs/2026-07-02-examples-context-type-design.md` — verify H1–H7 + every Testing bullet landed or is explicitly out-of-scope; verify `git diff --stat main..HEAD` touches ONLY `agent/crates/agent-skills/`, `agent/docs/`, and `docs/superpowers/` (the spec's "zero changes outside agent-skills except docs" constraint). Report gaps; don't silently fix design-level ones.

- [ ] **Step 3: Commit (only if fixes were needed)**
