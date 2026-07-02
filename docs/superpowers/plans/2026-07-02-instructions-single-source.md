# Instructions Single-Source & Negative Constraints Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One shared, ratchet-guarded coding-agent system prompt (with a negative-constraints clause) plus "Do not use for…" blocks and fact corrections across the `.agents/skills/` rule files.

**Architecture:** New `prompts.rs` module in `agent-runtime-config` owns `pub const BASE_SYSTEM_PROMPT`; `agent-server/src/daemon.rs` re-exports it as `SYSTEM_PROMPT` (5 in-crate use sites unchanged); `agent-cli` imports it. A source-scan ratchet test fails the build if the prompt text is pasted anywhere else. Skill/CLAUDE.md edits are prose-only.

**Tech Stack:** Rust (cargo workspace under `agent/`), markdown skill files.

**Spec:** `docs/superpowers/specs/2026-07-02-instructions-single-source-design.md`

## Global Constraints

- Two Cargo workspaces: `agent/` and `src-tauri/`. All `cargo` commands here run from `agent/`. If `cargo` is missing: `source ~/.cargo/env`.
- Conventional commits: `type(scope): summary`.
- `bash scripts/ci.sh` (repo root) must stay green at cluster end.
- The prompt's identity prefix `"You are a local coding agent."` must not change — server tests assert `contains("local coding agent")`.
- Skill-file edits follow the existing house style for negative constraints (bold `**Do not**` bullet block, cf. `llama-server/SKILL.md:32-34`, `tauri/SKILL.md:28-30`).

---

### Task 1: Shared prompt const + re-duplication ratchet

**Files:**
- Create: `agent/crates/agent-runtime-config/src/prompts.rs`
- Modify: `agent/crates/agent-runtime-config/src/lib.rs:7-8` (add module + re-export)
- Modify: `agent/crates/agent-server/src/daemon.rs:23-25` (const → re-export)
- Modify: `agent/crates/agent-cli/src/main.rs:6-9,17-19` (import; delete local const)

**Interfaces:**
- Produces: `agent_runtime_config::BASE_SYSTEM_PROMPT: &str` (also re-exported as `agent_server::daemon::SYSTEM_PROMPT`). Task 2 does not consume it; nothing else changes.

- [ ] **Step 1: Write the failing ratchet test**

Create `agent/crates/agent-runtime-config/src/prompts.rs`:

```rust
//! Model-facing prompt text shared by every frontend (CLI, server/desktop).
//!
//! Single source of truth for the coding-agent role identity. The ratchet
//! test below fails the build if the identity sentence is pasted into any
//! other `.rs` file in either workspace — re-export instead of copying.

/// Base system prompt for the coding agent. Frontends pass this to
/// `LoopParts.base_system_prompt` / `DaemonParams.system_prompt`.
pub const BASE_SYSTEM_PROMPT: &str = "You are a local coding agent. Use the provided tools to \
inspect and modify the workspace. Think step by step. When the task is complete, reply with a \
summary and no tool call. Constraints: operate only inside the provided workspace; never \
attempt to bypass the sandbox or the permission policy; never write secrets or credentials \
into outputs, files, or command arguments.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins both halves of the prompt: role identity and the negative-
    /// constraints clause added by the 2026-07-02 instructions cluster.
    #[test]
    fn prompt_contains_identity_and_constraints() {
        assert!(BASE_SYSTEM_PROMPT.starts_with("You are a local coding agent."));
        assert!(BASE_SYSTEM_PROMPT.contains("Constraints: operate only inside"));
        assert!(BASE_SYSTEM_PROMPT.contains("never write secrets or credentials"));
    }

    /// Re-duplication ratchet: the identity sentence may exist in exactly one
    /// .rs file — this one. (`contains("local coding agent")` assertions in
    /// tests are fine; the needle includes the "You are a" prefix they lack.)
    #[test]
    fn prompt_text_is_not_duplicated_anywhere() {
        const NEEDLE: &str = "You are a local coding agent";
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("repo root");
        let mut offenders = Vec::new();
        for root in ["agent/crates", "src-tauri/src"] {
            let dir = repo_root.join(root);
            if dir.exists() {
                scan(&dir, NEEDLE, &mut offenders);
            }
        }
        assert!(
            offenders.is_empty(),
            "system prompt text duplicated outside prompts.rs — re-export \
             agent_runtime_config::BASE_SYSTEM_PROMPT instead: {offenders:?}"
        );
    }

    fn scan(dir: &std::path::Path, needle: &str, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                scan(&path, needle, offenders);
            } else if path.extension().is_some_and(|e| e == "rs")
                && !path.ends_with("agent-runtime-config/src/prompts.rs")
                && std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains(needle)
            {
                offenders.push(path.display().to_string());
            }
        }
    }
}
```

In `agent/crates/agent-runtime-config/src/lib.rs`, after the `mod assemble;` block (line 7-8), add:

```rust
pub mod prompts;
pub use prompts::BASE_SYSTEM_PROMPT;
```

- [ ] **Step 2: Run the ratchet to verify it fails**

Run: `cd agent && cargo test -p agent-runtime-config prompt_text_is_not_duplicated -- --nocapture`
Expected: FAIL — offenders list contains `agent-server/src/daemon.rs` and `agent-cli/src/main.rs` (both still hold the old consts).

- [ ] **Step 3: Swap the two duplicated consts to the shared one**

In `agent/crates/agent-server/src/daemon.rs`, replace lines 23-25 (the whole `pub const SYSTEM_PROMPT` definition) with:

```rust
/// Re-export of the shared role prompt — single source of truth lives in
/// `agent_runtime_config::prompts`.
pub use agent_runtime_config::BASE_SYSTEM_PROMPT as SYSTEM_PROMPT;
```

In `agent/crates/agent-cli/src/main.rs`:
- Delete lines 17-19 (the local `const BASE_SYSTEM_PROMPT` definition).
- Add `BASE_SYSTEM_PROMPT` to the existing `agent_runtime_config` import list (lines 6-9), keeping alphabetical order:

```rust
use agent_runtime_config::{
    assemble_loop, backend_name_is_valid, build_memory_full, build_model, build_sandbox,
    default_allowlist, default_denylist, LoopParts, RuntimeConfig, BASE_SYSTEM_PROMPT,
};
```

(The use site at `main.rs:252`, `base_system_prompt: BASE_SYSTEM_PROMPT.to_string()`, compiles unchanged. The five `crate::daemon::SYSTEM_PROMPT` sites in agent-server compile unchanged via the re-export.)

- [ ] **Step 4: Run the crate tests + both dependent crates**

Run: `cd agent && cargo test -p agent-runtime-config prompts:: && cargo test -p agent-server && cargo test -p agent-cli`
Expected: PASS — both prompts tests green (ratchet now finds zero offenders); agent-server runtime tests (`contains("local coding agent")`) still green with the appended constraints clause.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/prompts.rs agent/crates/agent-runtime-config/src/lib.rs agent/crates/agent-server/src/daemon.rs agent/crates/agent-cli/src/main.rs
git commit -m "feat(runtime-config): single shared system prompt + constraints clause + re-dup ratchet"
```

---

### Task 2: Skill negative-constraint blocks, fact corrections, CLAUDE.md line

**Files:**
- Modify: `.agents/skills/auto-drive-tauri/SKILL.md:22,29,151`
- Modify: `.agents/skills/context-evolve/SKILL.md:19,27-29`
- Modify: `.agents/skills/context-management/SKILL.md:~17` (after intro paragraph)
- Modify: `.agents/skills/graphify-best-practices/SKILL.md:~21` (after intro paragraph)
- Modify: `.agents/skills/harness-engineering/SKILL.md:~27` (before "## Which playbook")
- Modify: `.agents/skills/wayland/SKILL.md:~30` (after the two-jobs list)
- Modify: `CLAUDE.md` (Gotchas section)

**Interfaces:** none (prose only). Line numbers are approximate — locate by the quoted anchor text, which is exact.

- [ ] **Step 1: auto-drive-tauri — Do-not block + drop the unconditional cargo claims**

After the "Pixel-driving the window is the **last resort** here (see §This machine for why)." paragraph (line ~22), insert:

```markdown
**Do not** use this skill for: generic Tauri v2 development (→ `tauri` skill);
automating GUI apps outside this repo on Wayland (→ `wayland` skill); or
pixel-driving when a WS-bridge rung is available — the bridge is the point.
```

Replace the prerequisite bullet (line 29):
`- **cargo / node on PATH** — they are; do NOT \`source ~/.cargo/env\`.`
with:
`- **cargo / node on PATH** — normally already true on this machine; only \`source ~/.cargo/env\` if a bare shell lacks \`cargo\`.`

Replace the troubleshooting bullet (line 151):
`- \`source ~/.cargo/env\` → unnecessary; cargo is on PATH.`
with:
`- \`source ~/.cargo/env\` → only needed when \`cargo\` is missing from PATH (normally it isn't here).`

- [ ] **Step 2: context-evolve — conditional cargo form + Do-not block**

Replace the build-harness bullet's command (line ~28):
`\`source ~/.cargo/env && cd agent && cargo build -p agent-runtime-config --tests --bins\``
with:
`\`cd agent && cargo build -p agent-runtime-config --tests --bins\``

Replace the following bullet (line ~29):
`- \`cargo\` is not on PATH by default — always \`source ~/.cargo/env\` first.`
with:
`- If \`cargo\` isn't on PATH, \`source ~/.cargo/env\` first (CLAUDE.md's conditional form).`

After the three-playbooks bullet list (after the `program.md` bullet, line ~19), insert:

```markdown
**Do not** use this skill for one-off context tuning or manual config edits
(→ `context-management` skill), nor for any change that would skip the eval
gate — it exists only for the full hypothesize → eval → gate campaign loop.
```

- [ ] **Step 3: context-management — Do-not block**

After the intro paragraph ending "This skill is judgment: how to work *with* that machinery." insert:

```markdown
**Do not** use this skill to run the optimization campaign (→ `context-evolve`)
or as internals documentation for the runtime's Rust context code — it is
generic judgment for working with the curating context manager at run time.
```

- [ ] **Step 4: graphify-best-practices — Do-not block**

After the intro paragraph ending "…is a graph query, not a grep.**" insert:

```markdown
**Do not** use this skill in place of the `/graphify` runbook (mechanics —
commands, flags, builds — live there), and don't reach for the graph when the
answer is one known file: a direct read beats a graph query for single-fact
lookups.
```

- [ ] **Step 5: harness-engineering — Do-not block**

Immediately before the "## Which playbook" heading, insert:

```markdown
**Do not** use this skill for ordinary feature work in this repo — only for
the harness layer itself. And never let `audit.md` edit code: it REPORTS ONLY;
the human holds the judgment gate.
```

- [ ] **Step 6: wayland — Do-not block doubling as the auto-drive-tauri deflection**

After the numbered two-jobs list ("1. **Writing** … 2. **Driving** …") and its "The two are linked…" paragraph, insert:

```markdown
**Do not** use this skill for THIS repo's own desktop app — load
`auto-drive-tauri` instead and drive its WebSocket bridge, not the GUI. Also
not for X11-only automation (there `xdotool` still works; this skill exists
for Wayland's constraints).
```

- [ ] **Step 7: CLAUDE.md — skill-tree disambiguation line**

In the `## Gotchas` section of the repo-root `CLAUDE.md`, add a bullet:

```markdown
- **Two skill trees** — `.agents/skills/` is Claude-facing (skills for working *on*
  this repo). The runtime's own agent loads skills from `<workspace>/.agent/skills`
  and `~/.agent/skills` (`agent-skills/src/registry.rs`). Don't author into the
  wrong tree.
```

- [ ] **Step 8: Verify the contradiction is gone and blocks are present**

Run: `grep -rn "not on PATH by default\|they are; do NOT" .agents/skills/ ; grep -c "^\*\*Do not\*\*" .agents/skills/*/SKILL.md`
Expected: first grep empty (exit 1); second shows ≥1 for each of the six edited skills (llama-server/tauri use their own existing style and may show 0 here — that's fine, they already have blocks).

- [ ] **Step 9: Commit**

```bash
git add .agents/skills CLAUDE.md
git commit -m "docs(skills): negative-constraint blocks, cargo-PATH fact fix, wayland deflection, skill-tree gotcha"
```

---

### Task 3: Cluster verification — full CI gate

- [ ] **Step 1: Run the full gate**

Run: `bash scripts/ci.sh`
Expected: fmt + clippy + cargo test (agent/) + web typecheck/vitest all green.

- [ ] **Step 2: No commit** (nothing should change; if ci.sh dirties anything, stop and report).
