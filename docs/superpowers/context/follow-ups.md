# Review Follow-ups Ledger (project-wide)

Durable, committed home for review findings from each subagent-driven-development cycle.
**ALL project review follow-ups are saved here** — this is the single source of truth.
Populated at the end of every `/implement-subsystem` cycle (the `.superpowers/sdd/` progress
ledger is gitignored scratch — findings live HERE). Convention: each item has a file:line ref,
a status (**Open** / **Accepted (won't-fix)** / **Resolved**), and a one-line reason. This file
is the project-wide index, including the claude-cli backend items below.

---

## claude-cli backend (standing)

Standing items for the `ClaudeCliClient` inference backend (the prior per-backend spike doc has
been folded into this ledger, which is now their home). For backend design detail see the spec
`2026-06-23-claude-cli-inference-backend-design.md` + its plan.

- **Per-turn idle (inter-chunk) stream timeout** — `agent/crates/agent-core` (`one_completion`) —
  **Resolved**. Wraps stream-open + each chunk in `tokio::time::timeout` → retryable
  `ModelError::Timeout`. Config: `LoopConfig.stream_idle_timeout` (default 120s) /
  `--stream-timeout-secs`. Covers SGLang + claude-cli. Spec
  `2026-06-23-agent-loop-stream-timeout-design.md` (merged 2026-06-23).
- **Rate-limit strategy for the 5-hour subscription cap** (P2) — **Open**. Detect a
  `rate_limit_event` → typed `ModelError` + backoff before running sustained loops.
- **Pin the subprocess CWD** (P2) — **Open**. `Command::current_dir` to an empty scratch dir so
  project-local hooks in the launch dir can't load. Small, self-contained.
- **Guard `BARE_SYSTEM_PROMPT` acceptance** (P3) — **Open**. Add an `#[ignore]`-gated real-CLI
  test so a future guardrail change doesn't break it silently.

## 2026-06-23 memory-system

Subsystem #4 (Vector / Long-Term Memory), tools-first slice. New `agent-memory` crate: `Embedder`
trait (in-process fastembed/ONNX `BGESmallENV15` 384-dim behind default feature `onnx`; deterministic
`StubEmbedder` for tests) + `MemoryStore` trait (`SqliteStore` single file `~/.agent/memory.db` +
`InMemoryStore`) + three tools `remember`/`recall`/`forget`. Per-project (hashed git-toplevel) + global
scoping, dedup-on-write, LRU cap, relevance threshold, budgeted recall. **Zero `agent-core` changes** —
attaches via the `Tool` seam + `build_memory` wiring into both `agent-cli` and `agent-server` (mirrors the
`mcp_tools` injection so memory survives a live settings-reconfigure) behind `--memory`/`--memory-db`/
`--memory-model-dir`. Spec `2026-06-23-memory-system-design.md`, plan `2026-06-23-memory-system.md`.
Branch `worktree-feat+memory-system` (commits `840c7e3..35e58c7`, 14 commits). Built subagent-driven (12 tasks).
Final whole-branch review (opus): **Ready to merge — Yes** (no Critical, no Important). All 5 binding
constraints verified against source: zero agent-core churn; best-effort isolation (tool errors → `ToolError`,
construction failure → memory disabled, never aborts); `Access::Read` auto-allow confirmed in `RulePolicy`
(approval-gating deferred, documented); SQL-enforced scope isolation for recall+dedup (and now by-id forget);
offline-safe `StubEmbedder` test discipline. Gates at tip: `cargo test --workspace` **252 passed / 0 failed /
9 ignored**; `clippy --all-targets -D warnings` clean. **Spec DoD validated against the real model**: live
`#[ignore]` `paraphrase_recall_across_reopen` passes — paraphrase query retrieves a stored memory across a
fresh-process reopen (11.8s incl. model load).

### Resolved during the cycle
- **Embedding-result `.unwrap()` panic surface** (Task 7 review, Important) — `agent/crates/agent-memory/src/tools.rs` — **Resolved** (`9e29b70`). `embed(...).into_iter().next().unwrap()` could panic on an empty embedder result, violating "memory tools never panic"; added a shared `first_embedding(vectors) -> Result<Vec<f32>, ToolError>` helper used by all three tools.
- **`forget` by-id deleted across scope** (final whole-branch review, Minor→hardened) — `agent/crates/agent-memory/src/tools.rs` (`Forget::execute` by-id branch) — **Resolved** (`35e58c7`). `delete(id)` matched on primary key alone, so a by-id forget was not scope-isolated like recall/dedup. Now get-then-check-`ScopeFilter::ProjectAndGlobal`-then-delete; not-found and out-of-scope collapse to the same `NotFound` (no cross-project id-existence probing). Subsumes the empty-string-id nit (`{"id":""}` → NotFound). Regression test `forget_by_id_refuses_other_project_scope`.

### Open (Minor — deferred to ledger by the final review; all cosmetic or single-user-bounded)
- **FastEmbedEmbedder holds the model mutex during blocking ONNX inference** — `agent/crates/agent-memory/src/embedder.rs` — **Open**. Reactor-blocking under concurrent load (spec defers `spawn_blocking`); `model.lock().unwrap()` poison-panics (idiomatic). The one item that could bite under future concurrent load; fine at single-user, low-call-volume scale.
- **`render_hits` budget uses strict `>`** — `agent/crates/agent-memory/src/tools.rs` — **Open**. Output can exceed `max_recall_chars` by the truncation-marker length (~38B); bounded + tested (`<= max + 64`), context-safe. Ambiguous in spec whether the marker counts against budget.
- **`rank` "one-time-ish warning" comment is misleading** — `agent/crates/agent-memory/src/store.rs` — **Open**. The dimension-mismatch `tracing::warn!` fires per row per query; consider debug-level or a per-query dedup, and fix the comment.
- **`SqliteStore` hygiene** — `agent/crates/agent-memory/src/store.rs` — **Open**. `SELECT *` loads the unused `dim` column; `0600` perms set with `let _ =` (silently fails on a foreign-owned file); the `"global"` literal is duplicated between `MemoryScope::kind()` and `row_to_record`'s round-trip.
- **`config` defaults test asserts 8/13 fields** — `agent/crates/agent-memory/src/config.rs` — **Open**. `max_tags`/`max_tag_len`/`candidate_warn_threshold`/`db_path`/`model_cache_dir` are not round-trip-asserted; `default_db_path` uses `$HOME` (Linux/macOS; not Windows — acceptable for local-first).
- **`parse_tags` silently drops non-string JSON tag values** — `agent/crates/agent-memory/src/tools.rs` — **Open**. Reasonable, but undocumented in the tool schema.
- **`scope.rs` `to_string_lossy` → lossy hash on non-UTF-8 paths** — `agent/crates/agent-memory/src/scope.rs` — **Open**. Theoretical (legal-but-exotic on Linux); stable but lossy project key.
- **`Embedder::dim()` is never called** — `agent/crates/agent-memory/src/embedder.rs` — **Open**. Dimension-mismatch handling relies on `cosine()` → NaN; `dim()` is part of the seam contract but currently dead. Flagged for the ledger.
- **README has no explicit prompt-injection/threat note** — `agent/crates/agent-memory/README.md` — **Open**. The residual prompt-injection-persistence risk is in the spec but not surfaced at the crate; a two-line "Security/threat model" note would make the accepted residual risk discoverable.
- **Cosmetics** — **Open**. `cosine` index-loop vs `zip` + `format!` per-dim alloc in `StubEmbedder` (test stub); `agent-runtime-config` Cargo.toml `agent-memory` line indent; `build_memory` clones `Option<PathBuf>` args; the `build_memory_enabled` test asserts the three names present rather than `len()==3`; a redundant `use std::process::Command` in `scope.rs` tests; memory tools registered before skills (last-write-wins registry, but `remember`/`recall`/`forget` don't collide with skill tool names).

### Deferred scope (intentional, from spec §1 "Out of scope")
- **Automatic `RetrievingContextManager` (silent top-K injection) + the async `ContextManager::build` core refactor** — Open — the headline follow-on; deliberately deferred so the async-seam change lands on its own atop a proven store.
- **Auto-ingestion** (end-of-session salient-fact capture) — Open — pairs with the auto-retrieval slice.
- **RuntimeConfig persistence + browser/Settings UI for memory** — Open — flags-only this slice (mirrors the skills→skills-runtime-config precedent; avoids a browser save wiping an un-round-tripped field).
- **Approval-gated memory writes** — Open — `Access::Read` auto-allow this slice; flipping to `Access::Write` is a one-line change.
- **LanceDB / ANN backend** — Open — `MemoryStore` trait keeps it a swap; exact brute-force is correct at single-user scale (a `candidate_warn_threshold` logs when scale would warrant ANN).
- **HTTP `/v1/embeddings` Embedder** — Open — `Embedder` trait keeps it a swap; in-process fastembed shipped.
- **Re-embedding / migration on embedding-model change** — Open — dimension-mismatched rows go inert with a one-time notice rather than being re-embedded.

## 2026-06-23 os-sandboxing-docker

Subsystem #2 (top hardening priority). New `agent-sandbox` crate (Docker `docker run` impl) behind a
`SandboxStrategy` trait in `agent-tools` (`HostExecutor` default + `DockerSandbox`); `execute_command` and
MCP stdio servers launch through it. Tri-state mode (off/auto/enforce; default auto = degrade-to-host-with-warning,
enforce = fail-closed→`ToolError::Denied`). Hardened `docker run` (network none, read-only rootfs + tmpfs,
cap-drop ALL, no-new-privileges, --user, mem/cpu/pids/fsize limits, workspace bind at /workspace). Config via
`RuntimeConfig.sandbox_*` (serde-default, no-silent-wipe) + `--sandbox-*` flags on both binaries; Settings/web
wiring deferred. Spec `2026-06-23-os-sandboxing-docker-design.md`, plan `2026-06-23-os-sandboxing-docker.md`.
Branch `feat/os-sandboxing-docker` (commits `784b4b5..54fa126`, 17 commits). Built subagent-driven (12 tasks).
Final whole-branch review (opus): **Ready to merge — With fixes** (2 cross-task Important, both fixed; no Critical).
Gates at tip: `cargo test --workspace` 225 passed / 0 failed / 7 ignored; `clippy --all-targets -D warnings` clean.
**DoD validated against real Docker 29.4.0 + debian:stable-slim**: all 5 ignore-gated escape proofs pass
(host-FS isolation, read-only rootfs, network off, workspace writable as host-uid, workspace host-owned); zero
container leaks (`docker ps -a` clean after).

### Resolved during the cycle
- **Approval posture misreported the degraded reality** (final review, Important) — `agent/crates/agent-core/src/loop_.rs` (`run_tool` `Decision::Ask`) — **Resolved** (`227ec8d`). In `auto` mode with Docker absent a command runs on the host (full network, no isolation) but the prompt said `(sandbox: docker, network off)`. Now: when `describe().degraded.is_some()`, the posture reads `(sandbox: docker unavailable->host, network on)`. Regression test `degraded_posture_shows_unavailable_and_network_on`.
- **Service (MCP) containers leaked stopped records** (final review, Important) — `agent/crates/agent-sandbox/src/docker.rs` (`docker_run_args` Service branch) — **Resolved** (`227ec8d`). Service launched with `-i` but no `--rm`, and teardown only `docker kill`s (never `docker rm`), so stopped containers accumulated with a PID-reuse name-collision tail risk. Service now also gets `--rm` (docker auto-removes on exit/kill; kill-by-name still works). Test renamed `service_keeps_stdin_open_and_rm`.
- **Pipe-buffer deadlock in `execute_command`** (Task 2, Critical) — `agent/crates/agent-tools/src/shell.rs` — **Resolved** (`4971bd2`). Plan's example drained stdout/stderr after `wait()`; restored concurrent drain via `tokio::join!(child.wait(), read_out, read_err)`. Regression test `captures_large_output_without_deadlock` (~220 KiB).
- **Workspace compile break between tasks** (Task 2, Critical) — 5 cross-crate `ToolCtx` literals — **Resolved** (`4971bd2`). Non-optional `ToolCtx.sandbox` left agent-core + 4 test helpers uncompilable; added `HostExecutor` stopgaps (agent-core's replaced by the config-driven strategy in Task 3 / `895c536`).
- **`$HOME`-root reject compared a raw (un-canonicalized) home** (Task 5, Important) — `agent/crates/agent-sandbox/src/mounts.rs` — **Resolved** (`1d7f192`). A symlinked `$HOME` prefix could dodge the reject; now canonicalizes home before compare. Also clarified the `/var/run`->`/run` symlink aliasing. Regression test `rejects_symlinked_home_root`.
- **Escape-proof test bugs surfaced by real-Docker validation** — `agent/crates/agent-sandbox/tests/escape.rs` — **Resolved** (`54fa126`). `workspace_is_writable` used `--user 0:0` but `--cap-drop ALL` removes `CAP_DAC_OVERRIDE`, so container-root cannot write a host-user-owned bind mount (the failure actually *proved* the hardening) — switched to the real host uid:gid. `cannot_read_etc_shadow` was misconceived (read the container's own throwaway `/etc/shadow`, not a host secret) — replaced with `host_filesystem_is_not_visible` (unmounted host secret unreachable).
- **Drop backstop `docker kill` printed spurious "No such container" on every `--rm` completion** — `agent/crates/agent-tools/src/sandbox.rs` (`SandboxedChild::Drop`) — **Resolved** (`54fa126`). Fire-and-forget `docker kill` inherited stderr; now nulls stdout/stderr. The async `kill()` path already captured via `.output()`.
- **Task-4 test gaps on security flags** — `agent/crates/agent-sandbox/src/docker.rs` (tests) — **Resolved** (`c0a890e`). Added assertions for `--memory/--cpus/--pids-limit/--tmpfs`, the `--ulimit fsize` positive path, and `--cap-add` absence.
- **`kill()` dual-kill + `extra_rw` boundary undocumented** (final review fold-ins) — `sandbox.rs` / `docker.rs` — **Resolved** (`227ec8d`). Added the dual-kill comment (docker kill stops the container; start_kill reaps the local `docker run` client) and a note that `--sandbox-extra-rw` widens the writable boundary beyond `/workspace`.

### Open (Minor — deferred to ledger by the final review)
- **`StdioTransport::Drop` uses `self.child.lock().unwrap()`** — `agent/crates/agent-mcp/src/transport.rs` (`Drop`) — **Open**. Double-panic risk on a poisoned mutex; matches the pre-existing pattern and Drop now only `take()`s (no await/kill), so the window is tiny. `if let Ok(g) = ...` would close it.
- **Cosmetics in the MCP transport** — `agent/crates/agent-mcp/src/transport.rs`/`manager.rs` — **Open**. Redundant `spec.env.clone().into_iter().collect()` (plain `.clone()` suffices); fully-qualified `std::sync::Arc<...>` vs a `use` alias.
- **`current_uid_gid` yields `":"` on pathological empty `id` output; no isolated `enforce` `build_sandbox` test** — `agent/crates/agent-runtime-config/src/lib.rs` — **Open**. No panic; bad `--user` arg only if `id -u`/`id -g` returned empty. Enforce denial is covered at the strategy layer (`enforce_denies_when_unavailable`).
- **`build_loop` smoke tests assert no-panic only; agent-cli `sbcfg` is a carrier-only `RuntimeConfig`** — `agent/crates/agent-server/src/runtime.rs`, `agent/crates/agent-cli/src/main.rs` — **Open**. Readability/coverage; both paths are correct and exercised.
- **`~user` (other-user tilde) mounts fall through to canonicalize→err** — `agent/crates/agent-sandbox/src/mounts.rs` — **Open**. Rejected, but via not-found rather than explicit policy; the socket-reject unit test likewise passes via not-found when Docker is absent (test-quality, not a security gap).
- **Task-1 TDD RED was procedurally weak** (single-commit delivery) — `agent/crates/agent-tools/src/sandbox.rs` — **Open**. Process note only; no code impact.

### Deferred scope (intentional, from spec §10)
- **Settings-panel/web UI + `AgentEvent::SandboxNotice` wire round-trip** — Open — flags + disk only this slice (avoids a browser save wiping an un-round-tripped field, per the skills-config precedent).
- **Per-approval / per-command network & path grants** — Open — global `sandbox_network` toggle this slice; per-call grants would expand the ApprovalChannel/wire/web surface.
- **Auto `docker pull`, devcontainer/Dockerfile detection, image-build caching** — Open — deferred; a missing image surfaces as a clear `docker run` error (auto degrades, enforce denies).
- **Landlock/seccomp/firejail strategies** — Open — the `SandboxStrategy` trait keeps them pluggable.
- **Redirect-caches-into-workspace build ergonomics** — Open — strict FS grants + `extra_rw` config knob this slice.
- **OS-confining the in-process file tools** — Open — would require sandboxing the agent process itself (a different, larger effort); they keep the logical `resolve_in_workspace` guard.

## 2026-06-23 skills-runtime-config-persistence

Persist `skills_dirs`/`active_skills` into `RuntimeConfig` + full browser Settings capability (daemon
disk+wire round-trip, live-apply mid-session, discovered-skills picker). One deliberate additive core
change (`ContextManager::set_system`); `agent-model`/`agent-tools`/`agent-policy`/`agent-skills`
internals and `agent-cli` untouched. Branch `feat/skills-runtime-config` (commits `ca31368..ebb72b5`).
Final whole-branch review (opus): **Ready to merge — Yes**; no Critical/Important. All four load-bearing
invariants verified against source: no-silent-wipe (`#[serde(default)]` + web `...form` spread + UI edit);
single-core-change discipline; strict-wire/lenient-startup validation split; no-cross-lock next-turn
concurrency (`apply()` touches only std mutexes; `set_system` only inside the per-turn task holding `ctx`).
Gates at tip: Rust 199 tests + clippy `-D warnings` clean; web 47 vitest + build clean.

Review follow-ups:
- **`compose_system_prompt` error was silently swallowed in `build_loop`** — `agent/crates/agent-server/src/runtime.rs` (`build_loop`) — **Resolved** (commit `ebb72b5`). Was `.unwrap_or_else(|_| base)`; now `match` + `tracing::error!` then fall back to base. Deliberately NOT `expect()`: `build_loop` runs at startup via the lenient `RuntimeState::new`, which must never panic. Path is unreachable today (presets pre-filtered against `scan()`); the log surfaces a future pre-filter/compose divergence.
- **`wire.test.ts` "parses a settings_state frame" inline fixture omitted `discovered_skills`** — `web/test/wire.test.ts` — **Resolved** (commit `ebb72b5`). Passed only because `parseInbound` is a loose cast; added `discovered_skills: []` so the fixture matches the real wire shape.
- **`state_body` runs a filesystem `scan()` on every `SettingsGet`** — `agent/crates/agent-server/src/runtime.rs` (`state_body`) — **Accepted**. Panel-scoped, human-driven; not a hot path. Deliberate per-request rescan keeps the discovered-skills list fresh. Would matter only if `state_body` were ever called from a ping/reconnect loop.
- **`apply_rejects_unknown_active_skill_without_swapping` asserts loop ptr unchanged but not `current_system_prompt()`** — `agent/crates/agent-server/src/runtime.rs` (test mod) — **Open**. Behavior is correct (the `return Err` precedes any `system_prompt` swap), so this is a test-fidelity gap only; a one-line `assert_eq!` on the prompt would make it a complete non-mutation proof.
- **`settings_state_includes_discovered_skills` test uses `try_recv()`** — `agent/crates/agent-server/src/runtime.rs` (test mod) — **Accepted**. Correct while `handle` posts its reply synchronously before the test reads; would need a timed `recv()` only if the reply ever became async.

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

## 2026-06-23 sampling-thinking-settings

Spec: [`../specs/2026-06-23-sampling-thinking-settings-design.md`](../specs/2026-06-23-sampling-thinking-settings-design.md) ·
Plan: [`../plans/2026-06-23-sampling-thinking-settings.md`](../plans/2026-06-23-sampling-thinking-settings.md).
Seven inference controls end-to-end (5 optional sampling params + `enable_thinking` + `preserve_thinking`) plus a
distinct reasoning channel (captured from `delta.reasoning_content` AND inline `<think>…</think>` via a streaming
`ThinkingSplitter`, surfaced as `AgentEvent::Reasoning`/`WireEvent::Reasoning`, re-wrapped into or stripped from
history per `preserve_thinking`). Built subagent-driven (8 tasks). Final whole-branch review (opus): **Ready to
merge — With fixes (one process step; no code defects).** No Critical, no Important. Cross-cutting properties
verified clean: claude-cli ignores all 7 (`ClaudeCliClient::stream` reads only `messages`); the
`CompletionRequest::Default → enable_thinking=false` is genuinely unreachable in production (the only non-test
construction, `loop_.rs` `run()`, sets it explicitly from `LoopConfig`); the splitter is constructed fresh per
`stream()` and runs only on the live stream, so a `preserve_thinking=true` `<think>` re-sent in history is inert
(never re-routed to reasoning, never leaks into a later answer); answer text stays reasoning-free so tool-call
parsing is unaffected. The Cloudflare Worker is unchanged (relays opaquely). This slice intentionally modified the
core crates `agent-model`/`agent-core` (additive: new optional fields + enum variants) — accepted per spec §9, since
sampling and reasoning capture *are* the model/loop's job. Final: workspace tests green (incl. 2 splitter/SSE tests
added post-review), clippy `-D warnings` clean; web 45 tests + build green.

### Resolved during the cycle
- **`reasoning_content` SSE path + unterminated-`<think>` flush were coded-correct but untested** — `agent/crates/agent-model/src/openai.rs` (test mod) — Resolved (commit `5a860cf`). The spec §10 list requires "reasoning_content delta path produces `Chunk::Reasoning`"; the original splitter tests covered only the inline-`<think>` branch. Added `streams_reasoning_content_separately` (wiremock SSE with a `reasoning_content` delta → asserts reasoning and answer accumulate on separate channels) and `splitter_flushes_unterminated_think` (an unterminated `<think>` at stream end flushes its buffer as reasoning, not lost/leaked). Raised by the final whole-branch review.

### Accepted (Minor, won't-fix now — backfill candidates)
- **Prompted-protocol tool-call fence inside `<think>` would be swallowed** — `agent/crates/agent-model/src/openai.rs` (`ThinkingSplitter`) interacting with `prompted.rs` parse — Open. If a model under `PromptedJsonProtocol` ever wrapped its ` ```tool_call ` fence inside `<think>…</think>`, the splitter would route the fence to the reasoning channel and `parse` would see a final answer (tool call lost). Model-misbehavior, not a pipeline bug; the live Qwen3 target uses the **native** protocol (`delta.tool_calls`), which is unaffected. Fix: a one-line comment near the splitter noting the assumption that tool calls don't appear inside `<think>`.
- **Interleaved `reasoning → token → reasoning` renders multiple "Thinking" blocks** — `web/src/state.ts` (`reduceFrame` `"reasoning"` case) — Open. The reducer folds into the last item only when `last.kind === "reasoning"`, so an answer token between two reasoning runs splits them into two collapsible blocks straddling the answer. Cosmetic; coalescing adjacent reasoning is a small reducer change if observed in practice with the live backend.
- **`save_then_load_over_round_trips_all_fields` not extended to all 4 new sampling fields** — `agent/crates/agent-runtime-config/src/runtime_config.rs` (test mod) — Open. The new `sampling_round_trips_and_partial_file_keeps_base` test does save+reload `top_k` (+ both bools), but `top_p`/`min_p`/`presence_penalty`/`repeat_penalty` are never round-tripped. Serde is mechanical so the risk is low; the "all fields" name is now a mild misnomer.
- **`CompletionRequest::Default` gives `enable_thinking=false` vs the spec's "default true"** — `agent/crates/agent-model/src/types.rs` (`CompletionRequest`) — Accepted. Verified inert: the only non-test construction (`loop_.rs` `run()`) sets `enable_thinking` explicitly from `LoopConfig` (whose default is `true` via `RuntimeConfig`'s `default_true`, and the CLI sets `!cli.no_thinking`). A doc-comment noting "always populated from `LoopConfig` in production" would prevent a future direct-`Default` send from silently disabling thinking.
- **Cosmetics** — Accepted: `PartialRuntimeConfig` merge arms re-wrap `if let Some(v) = p.top_p { self.top_p = Some(v); }` instead of `self.top_p = p.top_p` (`runtime_config.rs`); the `body()` `top_p` test uses an epsilon tolerance of `1e-6`, ~100× wider than the actual `f32(0.8)` rounding error of ~1.2e-8 (`openai.rs`, no false-pass risk); `import React, { useState }` added to type a `ChangeEvent` where `import type React` would suffice (`web/src/components/SettingsPanel.tsx`).

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
