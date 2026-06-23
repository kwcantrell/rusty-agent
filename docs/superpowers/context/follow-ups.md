# Review Follow-ups Ledger (project-wide)

Durable, committed home for review findings from each subagent-driven-development cycle.
Populated at the end of every `/next-spec` subsystem cycle (the `.superpowers/sdd/` progress
ledger is gitignored scratch — findings live HERE). Convention mirrors
[`claude-cli-inference.md`](./claude-cli-inference.md) → "Follow-ups / known limitations":
each item has a file:line ref, a status (**Open** / **Accepted (won't-fix)** / **Resolved**),
and a one-line reason. The per-backend claude-cli list remains the source of truth for
claude-cli detail; this file is the project-wide index.

---

## 2026-06-23 skills-subsystem

New `agent-skills` crate: Claude-Code-style skill packages (discover, load-on-demand, author, presets),
attaching only via the `Tool` seam + binary wiring. Built subagent-driven (11 tasks). Final whole-branch
review (opus): **Ready to merge — Yes.** No Critical, no Important. All binding constraints verified clean
(core crates agent-core/model/tools/policy untouched; no wire/web/cloud/RuntimeConfig changes; no process
spawning — bundled scripts run via the existing `execute_command` + policy + approval; lexical guards;
`runtime.rs` unchanged — skill tools ride the existing `mcp_tools` slice → registered per `build_loop`
rebuild, surviving settings-reconfigure). Security chain (slug → guard → validate-before-write →
approval-gated Write) verified airtight. 180 workspace tests pass, clippy `-D warnings` clean.

### Deferred scope (intentional, from the spec's "Deferred")
- **Persisting `skills_dirs`/`active_skills` into `RuntimeConfig`** + a browser Settings UI to edit them — Open — deferred to the Settings capability cycle. Persisting now (without matching web round-trip support) would let a browser settings-save silently wipe skill config, since `WireBody::SettingsUpdate` carries a full `RuntimeConfig`. `--skills-dir`/`--skill` are launch flags only this cycle.
- **Sub-agent skills** (a skill that spawns a constrained sub-`AgentLoop`) — Open — needs nested-agent machinery (event streaming, approval propagation, context budgeting, recursion limits); composes later as a different execution strategy over this same registry.
- **A dedicated skill-script runner / OS sandboxing** — Open — execution stays on the existing `execute_command` seam; os-sandboxing is subsystem #2.

### Resolved during the cycle
- **Path guard accepted any path when the base normalized to empty** — `agent/crates/agent-skills/src/guard.rs` (`resolve_in_dir`) — Resolved (commit `9f200f9`). `starts_with("")` is always true; added an empty-base rejection + `rejects_empty_base_dir` test. (Only Important finding of the cycle, raised in the Task 2 review.) Not reachable in current wiring, but a security-boundary guard must be robust regardless of caller.
- **`read_skill_file` missing-file→`NotFound` branch untested** — `agent/crates/agent-skills/src/tools.rs` (test mod) — Resolved (commit `c8fd7aa`). Brief-required error branch; added `read_skill_file_missing_file_is_not_found`.
- **Redundant non-dev `tokio` dependency + empty `--skills-dir ""` → relative writable root** — `agent/crates/agent-skills/Cargo.toml` + `src/registry.rs` (`from_config`) — Resolved (commit `8c09513`, from the final-review cleanup). Dropped the unused `[dependencies]` tokio (only `#[tokio::test]` needs it, covered by dev-deps; build confirms it was redundant); `from_config` now filters empty/whitespace `--skills-dir` entries (falling through to defaults) + `from_config_ignores_empty_skills_dir_entries` test. (Surfaced by the final whole-branch review.)

### Accepted (won't-fix)
- **`BASE_SYSTEM_PROMPT` (agent-cli) differs by ~1 space from the prior inline literal** — `agent/crates/agent-cli/src/main.rs` (`BASE_SYSTEM_PROMPT`) — Accepted. A whitespace normalization, not a regression; zero model-behavior impact (the new form is objectively cleaner). The spec's "no behavioral prompt change" wording was aspirational given the original literal's `\`-continuation artifact.

### Accepted (Minor, won't-fix now — backfill candidates)
- **`scan()` + `list_bundled_files` swallow non-`NotFound` `read_dir` errors silently** — `agent/crates/agent-skills/src/registry.rs` (`scan`, `list_bundled_files`) — Open. A root/subdir that exists but is unreadable (permissions) is treated identically to a missing one — no log. `scan` *does* `tracing::warn!` on malformed skills; only the IO-error path is silent. A matching `tracing::warn!` would aid ops debugging. Non-blocking.
- **`SKILL.md` CRLF body leaves a trailing `\r` per line** — `agent/crates/agent-skills/src/skill.rs` (`parse_skill_md` body assembly) — Open. `str::lines()` strips `\n` not `\r`; a CRLF-authored skill body keeps `\r` per line. Cosmetic; affects only CRLF-authored skills. Also: `parse_skill_md` tolerates leading blank lines before the opening `---` fence (more permissive than the brief; non-defect).
- **`create_skill` can orphan the target dir on a mid-write I/O fault** — `agent/crates/agent-skills/src/tools.rs` (`CreateSkill::execute`) — Open. If `create_dir_all` succeeds but the `SKILL.md` write fails (disk full / permissions), the empty dir remains and the no-overwrite guard then refuses retries until it's removed manually. NOT an input-validation hole (validate-before-write is airtight; bad *input* never writes). `scan()` skips the malformed dir so it's visible/recoverable. A cleanup-on-failure guard or an err-message note would help. v1-acceptable.
- **`UseSkill` NotFound error path calls `scan()` twice** — `agent/crates/agent-skills/src/tools.rs` (`UseSkill::execute`) — Open. `find()` scans, then the error branch scans again to list available names. Error-only path; scan is cheap. Also: the message renders a trailing `"Available: "` when the registry is empty — guard with `"none"`.
- **Coverage gaps** — `agent-skills` — Open. Untested: `intent()` bad-name (identical code path is tested via `execute()`); the 2-root default structure (only `writable_root` asserted, not that `~/.agent/skills` is added); a no-`name`-frontmatter skill (accepted using the dir name); `daemon_roundtrip.rs` uses the default `SYSTEM_PROMPT` rather than an overridden string to prove the param reaches `WindowContext`. Low-risk; load-bearing paths covered.
- **Cosmetics** — Accepted: `presets.rs` uses `format!`+`push_str` where `write!` (or chained `push_str`) avoids an allocation; a couple of test helpers return `Arc<SkillRegistry>` wider than the `&SkillRegistry` needed; one weak `contains("greeter")` assertion (could pin `"## Skill: greeter"`); `agent-runtime-config/src/lib.rs` import ordering places `agent_skills` after `agent_tools` (rustfmt would sort it before).

---

## 2026-06-23 — Settings capability (browser-driven live daemon reconfiguration)

Spec: [`../specs/2026-06-23-settings-capability-design.md`](../specs/2026-06-23-settings-capability-design.md) ·
Plan: [`../plans/2026-06-23-settings-capability.md`](../plans/2026-06-23-settings-capability.md) ·
Merged to `main` at `6133ff4`. Final whole-branch review (opus): "Ready to merge, with fixes" — all findings Minor, no Critical/Important.

### Accepted (won't-fix / deferred)
- **`effective_denylist` dedup is O(n²)** — `agent/crates/agent-runtime-config/src/runtime_config.rs` (`effective_denylist`). Uses `Vec::contains` per insert; fine for config-size lists, not a hot path. Reason: not worth an `IndexSet` for a handful of entries. (Reviewed again in the 2026-06-23 follow-up pass — deliberately kept.)
- **`SettingsPanel` keeps its opened snapshot if the server pushes a new `settings_state` while open** — `web/src/components/SettingsPanel.tsx` (`useState(settings)` init, no re-sync effect). Reason: closing and reopening the panel refreshes; standard modal-form pattern; not a spec requirement. The naive re-sync effect would clobber in-progress edits; a non-destructive "settings changed — reload?" banner is the correct fix and out of scope for the follow-up pass. (Reviewed again 2026-06-23 — deliberately kept.)

### Resolved (2026-06-23 follow-up pass, commit `6be8bb5`)
Four of the originally-accepted Minors resolved directly (TDD; 5 new Rust tests, 2 new web tests; `cargo test --workspace` + clippy `-D warnings` + web 42 tests + `npm run build` all green).
- **`validate()` now independently rejects `claude-cli` + non-`prompted`** — `agent/crates/agent-runtime-config/src/runtime_config.rs` (`validate`). Defense-in-depth for a future caller that skips `normalized()`; the happy path still normalizes first so it never fires there.
- **`load_over` splits `ErrorKind::NotFound` from other read errors** — same file. A missing file stays silent; an unreadable or malformed config now falls back to the launch base *and* warns to stderr, via an extracted, unit-tested pure `resolve_load` classifier.
- **`App.tsx` inline `import("./wire").RuntimeSettings` hoisted** — `web/src/App.tsx`. Now a top-level `import type { Decision, RuntimeSettings }`.
- **Hard-floor / user-denylist overlap now flagged** — `web/src/components/SettingsPanel.tsx`. A live "Redundant — already in the hard floor: …" note lists denylist entries already covered by the floor.

### Resolved (during the cycle, kept for context)
- **Daemon `user_input` arm lost integration coverage** after the forced `daemon_roundtrip.rs` rewrite → added a model-free `user_input` smoke test (fail-fast base_url + then `settings_get`→`settings_state` proves the read loop survived). Commit `c9a6e5a`.
- **Settings gear not disabled when daemon offline** (spec §7) → `StatusBar` gained `settingsDisabled?`; `App` passes `!(connected && state.online)`. Commit `b72c1fb`.
- **Daemon catch-all stamped the session id for every unhandled frame** → scoped stamp + `handle()` to `SettingsGet | SettingsUpdate`, `_ => {}` otherwise. Commit `b72c1fb`.
- **`settings_state` wire test didn't round-trip-deserialize** → added deserialize + `matches!` assertion mirroring the error half. Commit `b72c1fb`.
- **Duplicate `import` in `web/test/wire.test.ts`** (caused `tsc TS2300`, blocking `npm run build`) → merged the imports. Commit `45709d6`.

---

## 2026-06-23 mcp-client

- Streamable HTTP / remote MCP transport (+ auth) — Open — deferred; `McpTransport` seam is ready.
- MCP resources & prompts — Open — no core seam yet; deferred.
- EventSink/UI MCP server-status — Open — would add an AgentEvent variant (core touch); deferred until a UI consumer exists.
- Browser-side MCP management via Settings inbound channel — Open — pairs with the deferred Settings capability.
- OS-sandboxed MCP server processes — Open — MCP servers are untrusted code; synergy with os-sandboxing primer.

### Review findings (subagent-driven build, 9 tasks; final whole-branch review: Ready to merge — Yes)

**Resolved during the cycle**
- **Concurrent connect silently dropped a panicking task** — `agent/crates/agent-mcp/src/manager.rs` (`futures_join_all`) — Resolved (commit `d7fc3c8`). The `if let Ok(v) = h.await` drain discarded `JoinError`; now a `match` logs the panic via `tracing::error!` so degradation is never silent. (Only Important finding of the cycle.)

**Accepted (Minor, won't-fix now)**
- **`McpTool::execute` honors `ctx.timeout` but not `ctx.cancel`** — `agent/crates/agent-mcp/src/tool.rs` (`execute`); spec §3.4 mentions cancel — Accepted. The agent loop hands every tool a fresh `CancellationToken` that is never fired (dormant for native tools too, e.g. `shell.rs`), so wiring `select!` on cancel is YAGNI today; spec mention is aspirational. Revisit if the loop ever fires cancellation.
- **`connect_mcp` returns `McpManager` (not `Result`); a malformed/unreadable explicit `--mcp-config` degrades silently** — `agent/crates/agent-runtime-config/src/lib.rs` (`connect_mcp`) — Accepted. Consistent with the spec's "warn and disable rather than abort" stance; warns via `eprintln!` (could be `tracing::warn!` for consistency).
- **Test-coverage gaps in normalization/branches** — `agent-mcp` — Accepted. Unasserted/untested: multi-part `content[]` text join, non-text-content `[… omitted]` fallback, `list_tools` `description` field, `McpClient::close`, config "unreadable" branch. Load-bearing paths are covered by the hermetic suite + the live DoD test (14 tools); these are low-risk normalization branches. Backfill candidate.
- **`McpManager::from_parts` (test-only) bypasses status sort; `summary_line` "error" fallback only reachable via it** — `agent/crates/agent-mcp/src/manager.rs` — Accepted. Test helper only; no production reachability.
- **Cosmetics** — Accepted: redundant `text.clone()` in `execute` success path (`tool.rs`); `notify` always emits `params: {}` (`client.rs`); verbose fully-qualified type annotation in `agent-server/src/main.rs`; CLI `let _ = &mcp_manager;` keep-alive is a no-op-for-lifetime (binding already lives to end-of-`main`; comment overstates the mechanism) (`agent-cli/src/main.rs`).

---

## 2026-06-23 http-tool

New `agent-http` crate: read-only `fetch_url` web-fetch tool. Built subagent-driven (6 tasks).
Final whole-branch review (opus): **Ready to merge — Yes.** No Critical, no Important. All three
security invariants (zero core change; non-overridable SSRF floor; DNS-rebinding pin) verified
against the real core crates + `RulePolicy`. Live DoD validated against the real qwen3.6-35b-a3b
model (allowlisted no-prompt fetch; metadata-IP hard-denied even when allowlisted; non-allowlisted
approval prompt).

### Deferred scope (intentional, from the spec's "Out of scope")
- In-session response caching — Open — deferred; not needed for the slice.
- Headless browser (Playwright/chromiumoxide) — Open — separate, larger follow-up.
- POST / custom headers / general `http_request` — Open — `fetch_url` is GET-only by design.
- Overriding the SSRF floor for an explicitly-allowed private host — Open — floor is non-overridable
  in this slice; an opt-in escape hatch (e.g. for an internal docs host) is a future config addition.

### Resolved during the cycle
- **Redirect security boundaries were coded-correct but untested** — `agent/crates/agent-http/src/tool.rs` (test mod) — Resolved (commit `d0efc86`). Added `redirect_to_non_http_scheme_is_denied` (302→`file://` Location → `Denied`) and `too_many_redirects_is_failed` (7-hop chain → `Failed`). (Only Important finding of the cycle; raised in the Task 4 review.)

### Accepted (Minor, won't-fix now)
- **Per-hop redirect timeout multiplies the overall budget** — `agent/crates/agent-http/src/tool.rs` (`execute`, the per-hop `reqwest::Client::builder().timeout(ctx.timeout)`) — Open. Each redirect hop builds a fresh client with the FULL `ctx.timeout`, so a 5-hop chain can run up to ~6× the intended wall-clock before failing; the spec §5 says "overall timeout." Bounded externally by `ctx.cancel` (no hang risk). Partly a plan-level omission. Fix: compute a single `deadline = Instant::now() + ctx.timeout` before the loop and set each hop's timeout to the remaining budget (error if exhausted). Worth doing; non-blocking.
- **No test exercises SSRF re-validation on a redirect hop to a blocked IP** — `agent/crates/agent-http/src/tool.rs` (test mod) — Open. The spec DoD lists "redirect chain re-validation"; the per-hop `guard.check` is the identical path proven by `strict_guard_blocks_loopback_target` at hop 0, but no test shows a mid-chain redirect→blocked-IP being denied. Hard to test cleanly (the mock server is itself on loopback, requiring the permissive guard to reach). Backfill with a second mock or a stub resolver.
- **`human()` displays "2048.0 KB" instead of "2.0 MB" at the 2 MiB cap** — `agent/crates/agent-http/src/tool.rs` (`human`) — Accepted. Display-only; KB is accurate, just unrolled. Add an MB arm when convenient.
- **Truncation marker reworded from the plan's literal** — `agent/crates/agent-http/src/content.rs` (`truncate`) — Accepted. The plan's verbose marker (~82 chars) would have exceeded the plan's OWN `<= MAX_RETURN + 64` test bound — a plan-internal contradiction the implementer resolved by shortening to `[truncated: <n> bytes downloaded]`; spec intent (signal truncation + size) preserved. An explanatory comment at the marker would close the loop.
- **Cancellation maps to `ToolError::Timeout`** — `agent/crates/agent-http/src/tool.rs` (`resolve`/`execute`/`read_capped` select arms) — Accepted. `ctx.cancel` firing and a network timeout both surface as `Timeout`; the error taxonomy has no distinct cancel variant (matches the plan and the rest of the codebase). Revisit only if upstream retry logic must distinguish them.
- **Cosmetics** — Accepted: redundant `use agent_tools::Access;` in the `tool.rs` test module (harmless — explicit `use` shadows the `super::*` glob, compiles clean); content-type binary-refusal message shows the bare mime (`unknown` when empty) without quotes; a `agent-runtime-config` registry test could use a one-line "always registered; `NetworkPolicy` gates which hosts are permitted" comment.
