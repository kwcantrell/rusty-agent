# Destroy Tier + Subcommand-Aware Allowlist Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close harness deep-audit Top-10 #9 and #10: destructive `git` subcommands must stop reaching auto-Allow, and destructive memory ops (`forget`, plus writing `remember`) must stop declaring `Access::Read`.

**Architecture:** Three small, ordered changes. (1) A third `Access::Destroy` variant whose policy floor is Ask — never auto-allowed by allowlist or workspace-boundary rules. (2) `is_auto_allowed` allowlist entries generalize from program-name equality to whitespace-token **prefix** matching, so `"git status"` allows only that subcommand and unknown subcommands fail safe to Ask; `default_allowlist()` swaps bare `git`/`cargo` for read-safe subcommand prefixes. (3) agent-memory re-tiers its three tools truthfully. No wire/web/Tauri changes: the approval wire carries no Access field and Destroy maps to Ask, which both surfaces already handle.

**Tech Stack:** Rust (Cargo workspace under `agent/` — NOT `src-tauri/`), existing crates only, no new dependencies.

**Spec:** `docs/superpowers/specs/2026-07-01-destroy-tier-subcommand-allowlist-design.md`

## Global Constraints

- Work from repo root `/home/kalen/rust-agent-runtime`; Rust commands need `source ~/.cargo/env` and run inside `agent/`.
- Conventional commits: `type(scope): summary`.
- Invariant (spec): no destructive operation reaches `Decision::Allow` without explicit user opt-in; unknown subcommands of exec-capable allowlisted programs fail safe to Ask; `Access::Destroy` can never be auto-allowed (floor = Ask; hard floor may still Deny).
- Hard-floor ordering unchanged: Deny beats Allow beats Ask.
- Out of scope (do NOT touch): MCP `Trust→Access` mapping, HTTP `HostDecision→Access` mapping, approval wire format, `ApproveAlways` persistence, cluster-8 follow-ups.
- Final gate: `bash scripts/ci.sh` green.

---

### Task 1: `Access::Destroy` variant + engine arms

**Files:**
- Modify: `agent/crates/agent-tools/src/types.rs:31-35` (the `Access` enum)
- Modify: `agent/crates/agent-policy/src/engine.rs:41-73` (`RulePolicy::check`)
- Test: `agent/crates/agent-policy/src/engine.rs` (tests module, lines 76-227)

**Interfaces:**
- Consumes: existing `Access` enum (`agent_tools::Access`), `RulePolicy`, `Decision`.
- Produces: `Access::Destroy` variant (later tasks: agent-memory declares it; engine semantics: Destroy → Ask everywhere, Deny if hard-floored).

- [ ] **Step 1: Write the failing tests**

Append inside the existing `mod tests` in `agent/crates/agent-policy/src/engine.rs` (note the existing `policy()` helper allowlists `ls`, `cat`, `git` and denylists `rm -rf /`, `sudo`; the `intent()` helper builds a `ToolIntent`):

```rust
    #[test]
    fn destroy_inside_workspace_still_asks() {
        // Destroy never participates in the Read-style inside-workspace auto-allow.
        assert!(matches!(
            policy().check(&intent(Access::Destroy, vec!["/work/a.txt"], None)),
            Decision::Ask
        ));
    }

    #[test]
    fn destroy_pathless_commandless_asks() {
        // Memory-shaped intent: a path-less, command-less `forget`.
        assert!(matches!(
            policy().check(&intent(Access::Destroy, vec![], None)),
            Decision::Ask
        ));
    }

    #[test]
    fn destroy_command_never_auto_allowed() {
        // "ls -la" is Allow for a Write intent (allowlisted); a Destroy intent skips the gate.
        assert!(matches!(
            policy().check(&intent(Access::Destroy, vec![], Some("ls -la"))),
            Decision::Ask
        ));
    }

    #[test]
    fn destroy_command_still_hits_hard_floor() {
        // Deny still beats Ask for Destroy-declared commands.
        assert!(matches!(
            policy().check(&intent(Access::Destroy, vec![], Some("sudo reboot"))),
            Decision::Deny(_)
        ));
    }
```

- [ ] **Step 2: Run tests to verify they fail to compile**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-policy destroy`
Expected: compile error — `no variant named Destroy found for enum Access`.

- [ ] **Step 3: Add the variant**

In `agent/crates/agent-tools/src/types.rs`, extend the enum (derives stay as-is — additive variant, old serialized values still decode):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Access {
    Read,
    Write,
    /// Irreversible destruction (e.g. deleting a stored record). Never auto-allowed:
    /// the policy floor for Destroy is Ask — no allowlist or workspace-boundary rule
    /// may return Allow for it. The hard floor can still Deny it.
    Destroy,
}
```

- [ ] **Step 4: Add the engine arms**

In `agent/crates/agent-policy/src/engine.rs`, `RulePolicy::check`, replace the command branch's auto-allow line and the access match:

```rust
        if let Some(cmd) = &intent.command {
            if let Some(reason) = crate::command::hard_floor_violation(cmd, &self.command_denylist)
            {
                return Decision::Deny(reason);
            }
            // Destroy-declared intents are never auto-allowed, even when the command
            // itself is allowlisted — the tier's floor is Ask.
            if intent.access != Access::Destroy
                && crate::command::is_auto_allowed(cmd, &self.command_allowlist)
            {
                return Decision::Allow;
            }
            return Decision::Ask;
        }
        // Otherwise judge by access + path boundary.
        match intent.access {
            Access::Read => {
                // Decide "inside workspace?" with the SAME resolver execute() uses, so the
                // approval gate and the execution guard can never disagree (resolve_in_workspace
                // collapses `.`/`..` before the boundary check). An escaping read -> Ask.
                let all_inside = intent
                    .paths
                    .iter()
                    .all(|p| resolve_in_workspace(&self.workspace, &p.to_string_lossy()).is_ok());
                if all_inside {
                    Decision::Allow
                } else {
                    Decision::Ask
                }
            }
            Access::Write => Decision::Ask,
            // Destroy never participates in any auto-allow; its floor is Ask.
            Access::Destroy => Decision::Ask,
        }
```

- [ ] **Step 5: Run the tests**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-policy && cargo build`
Expected: all agent-policy tests PASS (the 4 new + all existing); whole-workspace build succeeds (only `engine.rs` matches exhaustively on `Access`).

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-tools/src/types.rs agent/crates/agent-policy/src/engine.rs
git commit -m "feat(policy): Access::Destroy tier — floor is Ask, never auto-allowed"
```

---

### Task 2: Subcommand-aware (token-prefix) allowlist matching

**Files:**
- Modify: `agent/crates/agent-policy/src/command.rs:185-233` (`is_auto_allowed` + doc comments)
- Test: `agent/crates/agent-policy/src/command.rs` (tests module)

**Interfaces:**
- Consumes: nothing new.
- Produces: `is_auto_allowed(cmd: &str, allowlist: &[String]) -> bool` (signature unchanged) now treats each allowlist entry as a whitespace-split token prefix; one-word entries behave exactly as before. Task 3's `default_allowlist()` relies on this.

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` in `agent/crates/agent-policy/src/command.rs` (the module already has an `allow()` helper over a bare `ls`/`cat`/`git` allowlist; these tests use explicit lists):

```rust
    #[test]
    fn prefix_entries_gate_subcommands() {
        let al = vec![
            "git status".to_string(),
            "git log".to_string(),
            "cargo build".to_string(),
        ];
        assert!(is_auto_allowed("git status", &al));
        assert!(is_auto_allowed("git status --porcelain -b", &al));
        assert!(is_auto_allowed("git log --oneline -5", &al));
        assert!(is_auto_allowed("cargo build --release", &al));
        // Destructive / unlisted subcommands are not auto-allowed (audit Top-10 #9).
        assert!(!is_auto_allowed("git push --force", &al));
        assert!(!is_auto_allowed("git reset --hard", &al));
        assert!(!is_auto_allowed("git clean -fdx", &al));
        assert!(!is_auto_allowed("cargo publish", &al));
        // Bare program does not match when only prefix entries exist.
        assert!(!is_auto_allowed("git", &al));
        // A flag before the subcommand breaks the prefix — accepted over-ask.
        assert!(!is_auto_allowed("git -C /tmp status", &al));
    }

    #[test]
    fn prefix_entry_longer_than_command_does_not_match() {
        let al = vec!["git status --short".to_string()];
        assert!(!is_auto_allowed("git status", &al));
        assert!(is_auto_allowed("git status --short", &al));
    }

    #[test]
    fn one_word_entries_keep_legacy_program_match() {
        let al = vec!["ls".to_string()];
        assert!(is_auto_allowed("ls -la", &al));
        assert!(!is_auto_allowed("lsblk", &al)); // token equality, not substring
    }

    #[test]
    fn degenerate_entries_never_match() {
        let al = vec!["".to_string(), "   ".to_string()];
        assert!(!is_auto_allowed("ls", &al));
    }
```

- [ ] **Step 2: Run tests to verify the new ones fail**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-policy prefix_ -- --include-ignored && cargo test -p agent-policy degenerate_entries`
Expected: `prefix_entries_gate_subcommands` FAILS (bare-program matching allows nothing multi-word... specifically `is_auto_allowed("git status", &al)` is false today because the entry `"git status"` != program `git`). `one_word_entries_keep_legacy_program_match` and `degenerate_entries_never_match` may already pass — that is fine; they pin behavior.

- [ ] **Step 3: Implement prefix matching**

In `is_auto_allowed`, replace the final membership test (`allowlist.iter().any(|a| a == prog)`) — keep every guard above it untouched:

```rust
    let prog = &tokens[0];
    if prog.contains('/') {
        return false;
    }
    // Allowlist entries are whitespace-split token prefixes: a one-word entry matches
    // the program name alone (legacy behavior); a multi-word entry ("git status") also
    // pins the leading arguments, so exec-capable programs can expose only read-safe
    // subcommands. Unknown subcommands fail safe to Ask. Degenerate (empty) entries
    // never match.
    allowlist.iter().any(|entry| {
        let want: Vec<&str> = entry.split_whitespace().collect();
        !want.is_empty()
            && want.len() <= tokens.len()
            && want.iter().zip(tokens.iter()).all(|(w, t)| *w == t.as_str())
    })
```

Also update the function's doc comment (currently lines 185-187) to:

```rust
/// A command is auto-allowed only if it is a single simple command, free of shell-
/// significant characters, invokes an unqualified (no `/`) program name, and matches an
/// allowlist entry. Entries are whitespace-token prefixes: `"ls"` matches any `ls`
/// invocation, while `"git status"` matches only that subcommand — `git push` et al.
/// fall through to Ask. Unknown subcommands of exec-capable programs fail safe to Ask.
```

And append one sentence to the ACCEPTED RESIDUAL block (the comment ending "…and rely on the execution sandbox (agent-sandbox)." around line 220):

```rust
    // With prefix entries the DEFAULT allowlist no longer exposes bare `git`/`cargo`
    // (see agent-runtime-config::default_allowlist); the residual narrows to the
    // enumerated subcommands (`cargo build` still runs build scripts) and re-widens
    // only if a user adds a bare exec-capable entry back.
```

- [ ] **Step 4: Run the crate tests**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-policy`
Expected: PASS — new tests plus all existing (existing tests use one-word entries, whose behavior is unchanged).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-policy/src/command.rs
git commit -m "feat(policy): allowlist entries are token prefixes — subcommand-aware auto-allow"
```

---

### Task 3: `default_allowlist()` rework + symmetry pin

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/lib.rs:200-207` (`default_allowlist`)
- Test: `agent/crates/agent-runtime-config/src/runtime_config.rs` (tests module, near the existing `cli_default_config_does_not_over_deny_benign_catastrophe_names` test at ~line 828)

**Interfaces:**
- Consumes: Task 2's prefix-matching `agent_policy::is_auto_allowed`; `agent_policy::{RulePolicy, PolicyEngine, Decision}`; `agent_tools::{Access, ToolIntent}`.
- Produces: new `default_allowlist()` contents (used by all surfaces at fresh-config time).

- [ ] **Step 1: Write the failing tests**

Append inside the tests module of `agent/crates/agent-runtime-config/src/runtime_config.rs`. Check imports first: the module already calls `agent_policy::hard_floor_violation`; if `is_auto_allowed` / `RulePolicy` are not re-exported at the agent-policy crate root, add them to the existing `pub use` in `agent/crates/agent-policy/src/lib.rs` (mirror how `hard_floor_violation` is exported). If `agent-tools` is not already a dev-dependency of agent-runtime-config, add `agent-tools = { path = "../agent-tools" }` under `[dev-dependencies]` (check `[dependencies]` first — it is likely already there for registry assembly).

```rust
    #[test]
    fn default_allowlist_is_subcommand_aware_for_exec_capable_programs() {
        let al = crate::default_allowlist();
        // Read-safe subcommands stay frictionless.
        assert!(agent_policy::is_auto_allowed("git status --porcelain -b", &al));
        assert!(agent_policy::is_auto_allowed("git diff HEAD~1", &al));
        assert!(agent_policy::is_auto_allowed("git log --oneline -5", &al));
        assert!(agent_policy::is_auto_allowed("cargo test -p agent-core", &al));
        assert!(agent_policy::is_auto_allowed("ls -la", &al));
        // Destructive / unlisted forms are no longer auto-allowed (audit Top-10 #9).
        assert!(!agent_policy::is_auto_allowed("git push --force", &al));
        assert!(!agent_policy::is_auto_allowed("git reset --hard", &al));
        assert!(!agent_policy::is_auto_allowed("git clean -fdx", &al));
        assert!(!agent_policy::is_auto_allowed("git push", &al));
        assert!(!agent_policy::is_auto_allowed("git commit -m x", &al));
        assert!(!agent_policy::is_auto_allowed("cargo publish", &al));
        assert!(!agent_policy::is_auto_allowed("cargo install evil", &al));
    }

    #[test]
    fn execute_command_git_status_matches_git_status_tool_friction() {
        use agent_policy::{Decision, PolicyEngine, RulePolicy};
        let policy = RulePolicy {
            workspace: std::path::PathBuf::from("/work"),
            command_allowlist: crate::default_allowlist(),
            command_denylist: crate::default_denylist(),
        };
        // execute_command("git status …") — judged by the command branch.
        let via_shell = agent_tools::ToolIntent {
            tool: "execute_command".into(),
            access: agent_tools::Access::Write,
            paths: vec![],
            command: Some("git status --short --branch".into()),
            summary: "run".into(),
        };
        // git_status tool — judged by the access branch (Read, no paths, no command).
        let via_tool = agent_tools::ToolIntent {
            tool: "git_status".into(),
            access: agent_tools::Access::Read,
            paths: vec![],
            command: None,
            summary: "status".into(),
        };
        // Same operation, same friction on both routes (audit Tools-component asymmetry).
        assert!(matches!(policy.check(&via_shell), Decision::Allow));
        assert!(matches!(policy.check(&via_tool), Decision::Allow));
    }
```

- [ ] **Step 2: Run tests to verify failure**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config subcommand_aware`
Expected: FAIL — `git push --force` IS currently auto-allowed (bare `git` in defaults), so the negative assertions fire. The symmetry test passes before and after (pin).

- [ ] **Step 3: Rework the default**

In `agent/crates/agent-runtime-config/src/lib.rs`:

```rust
pub fn default_allowlist() -> Vec<String> {
    // Exec-capable programs (`git`, `cargo`) are exposed as subcommand prefixes only —
    // a bare entry would auto-allow destructive forms (`git push --force`,
    // `git reset --hard`, `git clean -fdx`). Unknown subcommands fail safe to Ask.
    // The cargo set still runs build scripts: the documented exec-vehicle residual.
    // Users may add a bare "git"/"cargo" entry back in command_allowlist to opt out.
    [
        "ls", "cat", "pwd", "echo", "grep", "find", "rg", "head", "tail", "wc",
        "git status", "git log", "git diff", "git show", "git blame",
        "git rev-parse", "git ls-files",
        "cargo build", "cargo check", "cargo test", "cargo fmt", "cargo clippy",
        "cargo metadata", "cargo tree",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}
```

- [ ] **Step 4: Run the crate tests**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-runtime-config`
Expected: PASS, including the pre-existing `cli_default_config_does_not_over_deny_benign_catastrophe_names` and round-trip tests.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/src/lib.rs agent/crates/agent-runtime-config/src/runtime_config.rs agent/crates/agent-policy/src/lib.rs
git commit -m "feat(config): default allowlist exposes git/cargo as read-safe subcommand prefixes"
```

(Drop the agent-policy path from `git add` if no re-export change was needed; add the Cargo.toml if a dev-dependency was added.)

---

### Task 4: Memory tool re-tiering

**Files:**
- Modify: `agent/crates/agent-memory/src/tools.rs:11-22` (helper + doc comment), `:125-128` (remember intent), `:267` (recall intent), `:338` (forget intent)
- Test: `agent/crates/agent-memory/src/tools.rs` (tests module)

**Interfaces:**
- Consumes: Task 1's `Access::Destroy`.
- Produces: `remember` → `Access::Write`, `recall` → `Access::Read`, `forget` → `Access::Destroy`; intents stay path-less/command-less so the engine judges purely by tier (remember/forget → Ask, recall → Allow).

- [ ] **Step 1: Write the failing test**

Append a test inside the existing tests in `agent/crates/agent-memory/src/tools.rs`. The file's test modules (see `recall_tests`/`forget_tests`/remember tests, lines ~422-801) already construct `Remember`/`Recall`/`Forget` with an in-memory store + test embedder + `MemoryConfig` — reuse that exact fixture pattern (same constructor calls the neighbouring tests use; do not invent a new fixture):

```rust
    #[test]
    fn memory_tools_declare_truthful_access_tiers() {
        // Construct remember/recall/forget with the same fixture the surrounding
        // tests use (in-memory store + test embedder + default MemoryConfig).
        let args = serde_json::json!({});
        let r = remember.intent(&args).unwrap();
        assert_eq!(r.access, Access::Write);
        assert!(r.paths.is_empty() && r.command.is_none());

        let q = recall.intent(&args).unwrap();
        assert_eq!(q.access, Access::Read);

        let f = forget.intent(&args).unwrap();
        assert_eq!(f.access, Access::Destroy);
        assert!(f.paths.is_empty() && f.command.is_none());
    }
```

(`remember`/`recall`/`forget` above are the fixture-built tool values; bind them however the neighbouring tests do. `Access` derives `PartialEq`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-memory memory_tools_declare`
Expected: FAIL — all three currently return `Access::Read`.

- [ ] **Step 3: Re-tier the intents**

Replace the `read_intent` helper and its doc comment (lines 11-22) with:

```rust
/// Memory intents are path-less and command-less, so `RulePolicy` judges them purely
/// by access tier: `recall` (Read) stays frictionless, while `remember` (Write —
/// upsert plus capacity eviction) and especially `forget` (Destroy — irreversible
/// record deletion) require approval. `summary` stays truthful for the audit log.
fn memory_intent(tool: &str, access: Access, summary: String) -> ToolIntent {
    ToolIntent {
        tool: tool.into(),
        access,
        paths: vec![],
        command: None,
        summary,
    }
}
```

Update the three call sites:

```rust
        Ok(memory_intent(
            "remember",
            Access::Write,
            "write to long-term memory store".into(),
        ))
```

```rust
        Ok(memory_intent(
            "recall",
            Access::Read,
            "search long-term memory".into(),
        ))
```

```rust
        Ok(memory_intent(
            "forget",
            Access::Destroy,
            "remove from long-term memory".into(),
        ))
```

- [ ] **Step 4: Run the crate tests**

Run: `source ~/.cargo/env && cd agent && cargo test -p agent-memory`
Expected: PASS — the new test plus all existing (existing tests call `execute()` directly and never consult the policy gate).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-memory/src/tools.rs
git commit -m "fix(memory): remember=Write, forget=Destroy — memory mutations are approval-gated (audit #10)"
```

---

### Task 5: Workspace sweep + full CI gate

**Files:**
- Possibly modify: any agent-core/agent-cli/agent-server test or fixture that drives `remember`/`forget` through `RulePolicy` + approval (approval fails closed, so such tests would now FAIL with a deny, not hang), or that assumes bare `git`/`cargo` in `default_allowlist()`.
- No planned production-code changes.

**Interfaces:**
- Consumes: everything above.
- Produces: green workspace + green `scripts/ci.sh`.

- [ ] **Step 1: Run the full workspace test suite**

Run: `source ~/.cargo/env && cd agent && cargo test`
Expected: identifies any fallout. Two anticipated classes:
1. Loop-level tests exercising memory tools through the policy gate → now `Decision::Ask` → denied by a missing/failing approval channel. Fix by attaching the same always-approve stub `ApprovalChannel` that neighbouring loop tests use (search `agent/crates/agent-core/src/loop_.rs` tests for an existing approve-all stub and reuse it), or by asserting the new Ask/Rejected outcome where the test's point IS the policy decision.
2. Tests asserting `default_allowlist()` contents or auto-allow of `git`/`cargo` forms now demoted to Ask → update the expectation to the new default (the demotion is the point of this cluster; do not re-add bare entries to make a test pass).

If `cargo test` is fully green with no fallout, record that and move on.

- [ ] **Step 2: Grep for stale assumptions**

Run: `grep -rn "default_allowlist\|Access::Read" agent/crates --include=*.rs | grep -vi test | grep -n "memory\|allowlist"` and skim: no non-test code may still assume memory ops are Read or that `git` is a bare allowlist entry. Also `grep -rn '"git"' agent/crates/*/src --include=*.rs` to catch hardcoded bare-git allowlists in fixtures/configs (engine.rs/command.rs test helpers keep theirs deliberately — legacy one-word behavior is still covered there).

- [ ] **Step 3: Run the CI gate**

Run: `bash scripts/ci.sh` (from repo root)
Expected: fmt + clippy + cargo test (agent/) + web typecheck/vitest all PASS.

- [ ] **Step 4: Commit any fallout fixes**

```bash
git add -A
git commit -m "test(core): adapt gated-memory and allowlist fixtures to Destroy tier + prefix allowlist"
```

(Skip if Step 1 produced no changes.)
