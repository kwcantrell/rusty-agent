# Skills Subsystem — Design

**Status:** approved design (brainstorming → spec). Next step: `writing-plans`.
**Date:** 2026-06-23
**Attaches via:** `Tool` + `ToolRegistry` (new `agent-skills` crate) + agent-cli/agent-server wiring.
**Core crates touched:** none (agent-core / agent-model / agent-tools / agent-policy stay frozen — same bar held by #1 http-tool, #5, and #6).

---

## 1. Purpose & scope

Bring **skills** — Claude-Code-style reusable capability packages — to the Rust agent. A *skill* is a directory containing a `SKILL.md` (YAML frontmatter + markdown body) and optional bundled files/scripts. The agent **discovers** available skills, learns about them cheaply (name + when-to-use), and **loads a skill's full body on demand** (progressive disclosure) when it decides the skill is relevant.

This subsystem is the *foundation* of a layered family the user described ("all of the above"). The layers and their disposition:

| Layer | Disposition this cycle |
|---|---|
| Load-on-demand skill packages (discovery, progressive disclosure, on-demand load) | **In scope** — the foundation. |
| Prompt/system **presets** | **In scope** — a preset is a skill preloaded at startup. Folds in for nearly free. |
| **Agent-authoring** of skills (`create_skill`) | **In scope** — one additive tool + the same discovery path. |
| **Sub-agent skills** (a skill that spawns a constrained sub-`AgentLoop`) | **Deferred** — needs nested-agent machinery (nested event streaming, approval propagation, context budgeting, recursion limits). Composes cleanly later as a different skill *execution strategy* on top of this registry. |

### The one rule that contains the blast radius

The user chose the broadest payload — skills may bundle **executable scripts**. To keep that from dragging in the deferred os-sandboxing (#2) and sub-agent cycles, this subsystem obeys a single invariant:

> **The skills subsystem never executes anything.** It only *discovers, parses, surfaces, and authors*. When the agent runs a bundled script, it does so through the **already-built `execute_command` tool** → the **existing command allow/deny policy** → the **existing approval gate**.

A bundled script is therefore *no more privileged than the user typing that command*: it still hits approval and is bounded by the same guardrails that exist today. The `agent-skills` crate adds **zero** new execution authority. This was confirmed against the code: `ExecuteCommand` runs `sh -c <command>` with `cwd = workspace` (`crates/agent-tools/src/shell.rs:29-30`) and does **not** path-guard the command's arguments, so a script living outside the workspace runs by passing an absolute path, gated solely by the command policy + approval on the intent's `command` string.

### Out of scope (explicitly)

- No dedicated skill-script *runner* inside `agent-skills` (that is effectively building part of os-sandboxing #2).
- No OS-level isolation; the core-review caveat is carried verbatim — **the command policy is a guardrail, not a sandbox**, and path guards are lexical (no symlink resolution).
- No sub-agent execution.
- No modification of the four core crates.

---

## 2. Architecture

### New crate: `agent-skills`

Mirrors the `agent-http` precedent (the http-tool #1 subsystem added `FetchUrl` in a *new* crate that depends on `agent-tools`' `Tool` trait and implements it). `agent-skills` does the same: it depends on `agent-tools` for the `Tool`/`ToolSchema`/`ToolIntent`/`ToolCtx`/`ToolOutput`/`ToolError` types and implements `Tool` for each skill tool. The four core crates are untouched.

Registration follows the existing wiring: skill tools are constructed at startup and registered into the `ToolRegistry` via `build_registry` (`crates/agent-runtime-config/src/lib.rs:60`), and threaded into the daemon the same way `mcp_tools` are (`DaemonParams.mcp_tools` → `build_loop`, `crates/agent-server/src/runtime.rs:115-152`).

### Components

**`Skill`** — the parsed in-memory model of one skill:
- `name: String` — from frontmatter; must match the directory slug.
- `description: String` — the "when to use" line surfaced in the catalog.
- `body: String` — the markdown body (everything after frontmatter), returned by `use_skill`.
- `dir: PathBuf` — the skill's own directory (absolute).
- `files: Vec<PathBuf>` — manifest of bundled files (everything in `dir` other than `SKILL.md`), absolute paths.

**`SkillRegistry`** — discovery + lookup:
- Holds the ordered list of **skill roots** and a designated **writable root** (for authoring).
- `scan() -> Vec<Skill>` — walks each root for `*/SKILL.md`, parses frontmatter + body, builds the file manifest. **Dedupes by name with precedence: earlier root wins** (project-local beats user-global). Malformed skills are skipped with a logged warning, never fatal.
- Designed to be **cheap and re-scanned on every tool call**. This is the central design decision (see §6): the *tool surface* is fixed and small, while the *skill catalog* is dynamic, so a skill authored mid-session via `create_skill` appears on the very next `list_skills`/`use_skill` with **no loop rebuild** — sidestepping the registry-immutability constraint (the `ToolRegistry` is immutable once `Arc`-wrapped, `crates/agent-tools/src/registry.rs`).

The skill tools each hold an `Arc<SkillRegistry>` (the registry is config — the roots — not the catalog; the catalog is recomputed per call).

---

## 3. The tools (the stable surface)

Four tools, each implementing `agent_tools::Tool`, following the `ReadFile` template (`crates/agent-tools/src/fs/read.rs`): a struct holding the `Arc<SkillRegistry>`, `name`/`description` literals, an inline `json!` schema, an `intent()` declaring access for the policy engine, and an async `execute()`.

### `list_skills`
- **Args:** none.
- **Behavior:** `scan()`, return the catalog as text — one line per skill: `name: description`. No bodies, no file contents.
- **Intent:** `Access::Read`, no paths, summary `"list available skills"`.
- **Why:** discovery that always reflects current state, including freshly authored skills.

### `use_skill`
- **Args:** `{ "name": string }`.
- **Behavior:** `scan()`, find the named skill (error if absent), return: the markdown **body**, followed by a **bundled-file manifest** (absolute paths), followed by a short usage hint — "Read a bundled reference with `read_skill_file`; run a bundled script with `execute_command`." The result lands in context as the normal tool-result `Message::tool(...)` appended by the loop (`crates/agent-core/src/loop_.rs:153`). **No changes to `ContextManager` or the loop** — progressive disclosure falls out of the existing tool-result path.
- **Intent:** `Access::Read`, `paths: [skill.dir]`, summary `"load skill <name>"`.

### `read_skill_file`
- **Args:** `{ "skill": string, "path": string }` (`path` relative to the skill dir).
- **Behavior:** resolve `path` **within the named skill's own directory**, read it, return contents. Read-only. Lets the model inspect a bundled script/reference *before* running it.
- **Guard:** a lexical containment check confined to `skill.dir` — same discipline as `resolve_in_workspace` (`crates/agent-tools/src/fs/paths.rs`), applied to the skill dir instead of the workspace. This deliberately does **not** weaken the workspace guard: bundled files live outside the workspace, so routing them through `read_file` would either fail the workspace guard or require loosening it. A dedicated, separately-scoped read path is cleaner. Symlink resolution is out of scope (consistent with the core caveat).
- **Intent:** `Access::Read`, `paths: [resolved path]`, summary `"read skill file <skill>/<path>"`.

### `create_skill`
- **Args:** `{ "name": string, "description": string, "body": string, "files"?: [{ "path": string, "content": string }] }`.
- **Behavior:** write `<writable-root>/<slug>/SKILL.md` (frontmatter from `name`+`description`, body from `body`) plus any optional bundled `files` under that dir. The writable root is the project-local `<workspace>/.agent/skills` (created if absent).
- **Safety:**
  - `name` is sanitized to a slug; reject path traversal, absolute paths, separators, and empty/oversize names.
  - Bundled `files[].path` are confined to the new skill's dir (same lexical guard as `read_skill_file`).
  - Refuse to overwrite an existing skill of the same name unless an explicit overwrite is requested (default: refuse, return a clear error).
  - Enforce a size cap on `body` and each file's `content`.
- **Intent:** `Access::Write`, `paths: [<writable-root>/<slug>]`, summary `"create skill <name>"`. (So authoring is itself gated by the policy/approval engine — writing a skill is a `Write` intent.)

---

## 4. Presets

A **preset** is a skill **preloaded at startup** instead of loaded on demand.

- New repeatable flag `--skill <name>` on both `agent-cli` (`crates/agent-cli/src/main.rs` `Cli`) and `agent-server` (`crates/agent-server/src/main.rs` `Run`).
- At startup, the wiring resolves each named skill via the `SkillRegistry`, reads its body, and **concatenates the bodies after the base system prompt** before constructing the context — i.e. before `WindowContext::new(Message::system(...))` at the CLI site (`crates/agent-cli/src/main.rs:103`) and the daemon `SYSTEM_PROMPT` site (`crates/agent-server/src/daemon.rs:30-32,58`).
- This is **agent-cli / agent-server wiring only — the core stays untouched** (`ContextManager` exposes no post-construction system mutation, and we don't add one; we build the system message with presets already folded in).
- Persistence: presets may be stored as `active_skills` in `RuntimeConfig` so they survive daemon reconnect (mirroring how `http_allow_hosts` is persisted).

A preset and an on-demand skill are the *same artifact* — the only difference is whether the body is injected at startup or via `use_skill`. This is what makes "presets" nearly free given the foundation.

---

## 5. Discovery & configuration

### Roots
- `<workspace>/.agent/skills` — project-local, **writable** (authoring target).
- `~/.agent/skills` — user-global, read-only, shared across projects.
- **Precedence:** project-local wins over user-global on a name conflict (mirrors Claude Code's project+personal split).

### Flags (both binaries)
- `--skills-dir <path>` (repeatable) — override/add skill roots. Mirrors the `--allow-host` / `--mcp-config` pattern.
- `--skill <name>` (repeatable) — preload as a preset (§4).

### System-prompt awareness
Append a short static line to the base system prompt (at the CLI + daemon construction sites) noting that skills are available and to call `list_skills` to see them. This makes the model aware without dumping the whole catalog upfront — the catalog can grow and change, so `list_skills` is the authoritative, always-current view.

### Persisted config (`RuntimeConfig`, `crates/agent-runtime-config/src/runtime_config.rs`)
Add `skills_dirs` and `active_skills`. Each needs the field on the struct (`:11-24`), the `PartialRuntimeConfig` mirror (`:28-41`), a `from_launch` default (`:45-58`), and a `merge` rule (`:111-124`). This lets the (future) Settings capability edit skill roots/presets live, and lets presets survive reconnect.

---

## 6. Approaches considered

**Chosen — meta-tools + dynamic registry.** A small fixed set of tools (`list_skills` / `use_skill` / `read_skill_file` / `create_skill`), each re-scanning the skill catalog per call. The tool *surface* is stable; the *catalog* is dynamic. Authored skills appear instantly on the next `list_skills`; no loop rebuild is required.

**Rejected — each skill is its own registered `Tool`.** Would surface every skill as a first-class tool (`skill_<name>`) whose `execute` returns the body. Two problems: (1) it pollutes the tool list and the prompted-protocol preamble as skills grow (every skill's description ships every turn — defeats progressive disclosure); (2) the `ToolRegistry` is immutable once `Arc`-wrapped (`crates/agent-tools/src/registry.rs`), so an agent-authored skill could not appear without a full loop rebuild. The meta-tool approach avoids both.

---

## 7. Error handling & safety

- **Malformed `SKILL.md`** (invalid YAML, missing `name`/`description`, name≠dir-slug) → skipped from the catalog with a logged warning; the scan never crashes.
- **`create_skill`** → slug sanitization rejects path traversal / absolute paths / separators; refuses to overwrite an existing skill unless explicitly allowed; size caps on body + each bundled file. The write goes through a `Write` intent, so policy/approval still gate it.
- **`read_skill_file` / `create_skill` file writes** → lexical containment confined to the relevant skill dir; symlink resolution explicitly out of scope (consistent with the core's lexical-guard caveat).
- **Bundled-script execution** → not handled here at all; flows through `execute_command` + command allow/deny policy + approval. Carries the verbatim caveat: **command policy is a guardrail, not a sandbox; OS isolation deferred to #2**.
- **Missing/empty skill roots** → treated as "no skills"; `list_skills` returns an empty catalog cleanly.

---

## 8. Testing

**Unit (inline `#[cfg(test)]` in `agent-skills`, per the codebase idiom):**
- Frontmatter + body parsing (valid, missing fields, malformed YAML → skipped).
- `scan()` discovery across multiple roots + **name-conflict precedence** (project wins).
- Slug sanitization in `create_skill` (rejects traversal, absolute paths, separators, oversize).
- `read_skill_file` containment guard (rejects escapes from the skill dir).
- `create_skill` round-trip (write → next `scan()` sees it → `use_skill` returns the body + manifest).
- Tool plumbing: `name`/`schema`/`intent` access classification for each tool.

**End-to-end (against the live local model, per `agent/docs/RUNNING.md`):**
Author a skill via `create_skill` → confirm it appears in the next `list_skills` → `use_skill` loads its body → the model reads a bundled script via `read_skill_file` and runs it via `execute_command`, hitting the approval gate. Validates the full discovery → author → load → execute loop without any new execution authority.

**Quality gates:** `cargo test --workspace` green; `cargo clippy --all-targets -- -D warnings` clean (the standing bar held by every prior slice).

---

## 9. What this preserves

- **Core untouched** — new crate only; agent-core/model/tools/policy unchanged (held by #1, #5, #6).
- **Every seam reused** — `Tool`/`ToolRegistry` for the surface, the existing tool-result path for progressive disclosure, `execute_command` + policy + approval for execution, `build_registry` + `DaemonParams.mcp_tools` wiring for registration, `RuntimeConfig` for persistence.
- **Deferred work stays deferred** — sub-agent skills and os-sandboxing are not pulled forward; both compose cleanly on top of this foundation later.
