# Instructions single-source & negative constraints — design

**Date:** 2026-07-02
**Status:** Approved (autonomous backlog-drain run; brief fixed by the 2026-07-01 deep
audit's Finding 1 + report Component 1)
**Cluster:** 1 of 6 in the 2026-07 residual-backlog drain (see `.superpowers/sdd/progress.md`)

## Problem

The deep audit's Component 1 found the runtime's *instructions layer* violates
"a single, versioned source of truth per agent role; no contradictory or stale rule files":

1. The coding-agent system prompt is a byte-identical 175-char string duplicated as two
   independently editable constants — `agent-server/src/daemon.rs:23 SYSTEM_PROMPT` and
   `agent-cli/src/main.rs:17 BASE_SYSTEM_PROMPT` — with 8 total use sites and no test
   preventing re-duplication.
2. The prompt is capabilities-only; the model is never told the rules it runs under
   (workspace confinement, sandbox/policy, secrets hygiene).
3. Six of eight `.agents/skills/` skills lack a "Do not use for…" negative-constraint
   block (llama-server and tauri have one; auto-drive-tauri, context-evolve,
   context-management, graphify-best-practices, harness-engineering, wayland do not).
4. context-evolve/SKILL.md hard-asserts "`cargo` is not on PATH by default — always
   `source ~/.cargo/env` first" while auto-drive-tauri/SKILL.md asserts "cargo/node on
   PATH — they are; do NOT `source ~/.cargo/env`". Direct contradiction; machine truth is
   CLAUDE.md's conditional form.
5. The wayland↔auto-drive-tauri cross-reference is one-directional: auto-drive-tauri
   names wayland, but wayland never deflects to auto-drive-tauri for this repo's own
   desktop app.
6. CLAUDE.md never distinguishes `.agents/skills/` (Claude-facing skills, this repo's
   authoring target) from the runtime's own skill registry dirs
   (`<workspace>/.agent/skills` + `~/.agent/skills`, per `agent-skills/src/registry.rs`)
   — easy to author into the wrong tree.

## Approaches considered

- **A (chosen): shared const in `agent-runtime-config` + re-export shim.** New
  `prompts.rs` module owns `pub const BASE_SYSTEM_PROMPT`; `daemon.rs` re-exports it as
  `SYSTEM_PROMPT` (its 5 in-crate use sites keep compiling unchanged); agent-cli imports
  it directly (it already depends on agent-runtime-config). Smallest diff, one source of
  truth, and the audit itself pointed here ("assemble.rs already owns
  base_system_prompt").
- **B: update all 8 sites to import `agent_runtime_config::BASE_SYSTEM_PROMPT`
  directly.** Cleaner naming (no alias), larger diff, churns 5 test sites for no
  behavioral gain. Rejected — the re-export is still a single source; the ratchet test
  guards text duplication, not aliasing.
- **C: make the prompt a config value (RuntimeConfig field with default).** Over-general:
  the prompt is a role identity, not an operator knob; a config default would *add* a
  place for drift. Rejected (YAGNI).

## Design

### 1. Shared prompt module (`agent-runtime-config/src/prompts.rs`)

```rust
/// The coding-agent role identity shared by every frontend (CLI, server/desktop).
/// Single source of truth — see the re-duplication ratchet test below.
pub const BASE_SYSTEM_PROMPT: &str = "…";
```

- Text = the existing 175-char prompt **plus a short negative-constraints clause**
  (one sentence group, conservative — this is a model-facing behavior change):

  > "Constraints: operate only inside the provided workspace; never attempt to bypass
  > the sandbox or the permission policy; never write secrets or credentials into
  > outputs, files, or command arguments."

- Re-exported from `lib.rs`. `daemon.rs` becomes
  `pub use agent_runtime_config::BASE_SYSTEM_PROMPT as SYSTEM_PROMPT;`; agent-cli's
  local const is deleted in favor of the import.
- Existing tests that assert `contains("local coding agent")` keep passing (prefix
  unchanged; the clause appends).

### 2. Re-duplication ratchet test

In `agent-runtime-config` (unit test in `prompts.rs`), following the repo's existing
enforcement-ratchet pattern (cf. the curated-confusable test): walk every `*.rs` file
under the repo's two workspaces (`agent/crates/**/src`, `agent/crates/**/tests`,
`src-tauri/src`), skipping `target/`, and assert the distinctive phrase
`"local coding agent"` appears **only** in `agent-runtime-config/src/prompts.rs`.
Re-exports don't repeat the text, so aliasing stays legal; pasting the prompt anywhere
else fails the build. Path root discovered via `CARGO_MANIFEST_DIR` (two `..` hops to
the repo root), tolerant of the src-tauri workspace being absent.

### 3. Skill negative-constraint blocks

Add a 1–3 line **"Do not use for"** block to each of the six skills missing one,
matching the llama-server/tauri house style (bold `**Do not**` bullet(s) near the top
usage section). Content per skill (drafted here, refined at implementation with the
writing-skills skill's guidance — these edits are prose-only, no frontmatter changes):

- **auto-drive-tauri**: not for generic Tauri development (→ tauri skill) or non-Tauri
  GUI automation (→ wayland skill); it drives THIS repo's app via the WS bridge.
- **context-evolve**: not for one-off context-management tuning or manual config edits
  (→ context-management skill); it is the eval-gated optimization *campaign* only.
- **context-management**: not for running the optimization campaign (→ context-evolve)
  and not documentation of this repo's runtime internals — it is generic
  context-window practice.
- **graphify-best-practices**: not a replacement for running `/graphify` itself; not
  for questions answerable by reading one known file — it guides graph *usage*.
- **harness-engineering**: not for generic Rust feature work in this repo, and its
  audit.md REPORTS ONLY — never use it to edit code.
- **wayland**: not for this repo's desktop app (→ auto-drive-tauri, which drives the
  WS bridge instead of the GUI); not for X11-only automation.

The wayland block doubles as the missing **deflection line** (finding 5).

### 4. Fact corrections

- **context-evolve/SKILL.md** cargo lines → CLAUDE.md's conditional form:
  "`source ~/.cargo/env` first if `cargo` isn't on PATH." (drop the "always"/"not on
  PATH by default" claims).
- **auto-drive-tauri/SKILL.md** keeps its intent but drops the unconditional
  contradiction: "cargo/node are on PATH here; only `source ~/.cargo/env` if a bare
  shell lacks it" (both the line-29 checklist item and the line-151 troubleshooting
  note).
- **CLAUDE.md**: one line in the repo-map or conventions section: `.agents/skills/` =
  Claude-facing skills for working ON this repo; the runtime's own agent loads skills
  from `<workspace>/.agent/skills` + `~/.agent/skills` — don't author into the wrong
  tree.

## Error handling

The ratchet test must fail loud with the offending file list. No runtime error paths
change (const swap only).

## Testing

- New: prompt re-duplication ratchet (source scan, both workspaces).
- New: unit assertion that `BASE_SYSTEM_PROMPT` contains both the identity prefix and
  the constraints clause (pins the clause against accidental deletion).
- Existing: `agent-server` runtime tests (`contains("local coding agent")`) and the
  full `scripts/ci.sh` gate must stay green — they exercise the re-export sites.
- Skill/doc edits: no runtime tests; verified by the ratchet not matching them (they
  never quote the prompt text) and by review.

## Out of scope (recorded residuals)

- Prompt-eval gate before system-prompt edits (report build-opportunity; the clause
  here is conservative and additive — if eval evidence ever shows regression, the
  context-evolve harness is the measuring ground).
- Skill-lint script (name↔dir, description, when-NOT presence) wired into tests —
  report build-opportunity, not required to close Finding 1.
- `_facts.md` single source for volatile machine facts (cargo PATH, llama port) —
  the conditional phrasing removes the contradiction without new structure.
- Inlining the skills catalog into the prompt (separate product decision).
