# Harness Deep Audit — rust-agent-runtime

**Date:** 2026-07-01
**Method:** `harness-engineering` skill (`audit.md` playbook) — six Harness Anatomy
components + Context Engineering (Spine B) + an Eval/quality-flywheel dimension,
each fanned out to a dedicated auditor reading live source on branch
`fix/hard-floor-position-aware-denylist`.
**Reference spine:** Google — *The New SDLC With Vibe Coding* (Fig. 7 harness
anatomy, p27-30; context engineering p15-18; the 80% problem p34); Anthropic
engineering blogs (Writing Tools for Agents, Effective Harnesses, Effective
Context Engineering, Building Effective Agents); arXiv 2503.16416 (conflation).

**REPORT ONLY.** No code was changed. Findings carry `severity / file:line /
violated principle / concrete fix`. Line numbers drift — re-open before acting.

Prior fixes (this week) are NOT re-reported: parallel-dispatch isolation, the
tool `when_not_to_call` contract, the guardrails denylist hardening +
position-aware Layer A2 + approval timeout. Those are confirmed still in place.

---

## Executive summary

The runtime is **well above baseline** for agentic engineering: a real
correctness-gated, model-pinned eval loop with sealed oracles (`eval_context.rs`
+ `.agents/skills/context-evolve/`), ~455 tests with no untested crate, specs
that genuinely become tests, and a strong Docker sandbox profile and command
policy engine. The gaps cluster in four places:

1. **Sandbox is a *conditional* boundary that defaults off.** `sandbox_mode`
   defaults to `"auto"`, which silently degrades to unconfined **host**
   execution whenever Docker is absent — the common case for a "local-first"
   runtime. On that path the host executor **inherits the full parent
   environment** (leaking `AGENT_API_KEY`) and has **no resource limits**. This
   also invalidates the sandbox as the documented mitigation for the accepted
   exec-vehicle catastrophe residual.
2. **Observability is UI-only and ephemeral.** No tool call emits a terminal
   status/duration; failed calls emit *nothing*; nothing is persisted on any
   surface, so a failed turn cannot be replayed or harvested into an eval.
3. **No CI.** Two deterministic suites were purpose-built and labelled
   "CI-runnable" and have never had a CI to run in; the hottest-changing
   component (policy, 14 commits/7 days) is guarded only by human memory.
4. **Loop robustness edges.** Retries don't classify failures (a 400 is retried
   3×), context eviction/compaction can orphan a tool message and 400 the turn
   mid-session, approval waits can't be cancelled, and three terminal paths
   never emit `Done`.

Two capability categories are **absent entirely** and are the biggest *build*
opportunities: **sub-agent / decompose-and-delegate** orchestration, and the
**Examples** context type (few-shot / reference patterns).

### Top 10 highest-leverage fixes (ranked, severity × leverage)

| # | Sev | Component | file:line | One-line fix |
|---|-----|-----------|-----------|--------------|
| 1 | high | Sandbox | `agent-tools/src/sandbox.rs:106` | `env_clear()` + explicit allow-list env in `HostExecutor` — stops `AGENT_API_KEY` leaking into every child on the default path |
| 2 | high | Sandbox | `runtime_config.rs:106`; `agent-cli/src/main.rs:125` | Default `sandbox_mode` to `enforce` (or refuse exec-capable tools when `degraded.is_some()`) — make isolation real by default |
| 3 | high | Observability | `loop_.rs:331-352` | Emit one `ToolResult{id,name,status,duration_ms,content}` for **every** resolved call at the Phase-3 drain (today only `Ok` emits; failures are silent) |
| 4 | high | Eval | `.github/workflows/` (absent) | Add CI: `cargo fmt --check` + `clippy` + `cargo test` (agent/) + `web` typecheck/vitest on every PR |
| 5 | high | Context | `context.rs:147-155`; `curated.rs:134-141,174-184` | Make eviction + compaction **turn-atomic** so a `Role::Tool` message is never kept without its parent `tool_calls` (otherwise a mid-session 400) |
| 6 | high | Observability | `session.rs:30-43`; `agent-cli/src/main.rs:235` | Add a JSONL session-trace sink (tee in `assemble_loop`) — nothing is persisted today; failed turns can't be replayed |
| 7 | high | Tools | `shell.rs:74`; `fs/read.rs:36`; `mcp/tool.rs:91` | Cap tool-result ingestion (truncate + eager offload); `execute_command`/`read_file`/MCP return unbounded output into the window |
| 8 | med | Orchestration | `loop_.rs:132-152` | Classify `ModelError` before retrying — abort on `Status{4xx}`/`Decode`, retry only transport/stream/timeout |
| 9 | med | Guardrails | `command.rs:184-223` | Make the allowlist subcommand-aware: `git push --force`/`reset --hard`/`clean -fdx` are **auto-Allowed** today (destructive → Allow, not even Ask) |
| 10 | med | Guardrails | `agent-memory/src/tools.rs:11-16` | Give `remember`/`forget` `Access::Write` — a destructive `forget` (deletes a record) is currently auto-allowed as `Read` |

---

## Component 1 — Instructions & Rule Files

```
severity: med
file:line: agent-server/src/daemon.rs:23-25 ; agent-cli/src/main.rs:15-17
violated principle: single versioned source of truth per role — whitepaper p27-28
concrete proposed fix: hoist one pub const BASE_SYSTEM_PROMPT into agent-runtime-config (assemble.rs already owns base_system_prompt) and import from both binaries; add a unit test asserting both reference it.
```
Byte-identical coding-agent prompt duplicated across two crates (verified
identical), and re-used in 5 more sites in agent-server. Two independently
editable copies of one role identity = the drift the principle forbids.

```
severity: med
file:line: agent-server/src/daemon.rs:23-25
violated principle: instructions state what the agent is FORBIDDEN from — p27-28
concrete proposed fix: add a short forbidden clause to the shared prompt (stay in workspace; never bypass sandbox/policy; never write secrets to outputs).
```
The identity prompt is capabilities-only; all prohibition lives in the mechanical
policy engine and the model is never told the rules it runs under.

```
severity: med
file:line: .agents/skills/context-evolve/SKILL.md:29 (vs auto-drive-tauri/SKILL.md:29)
violated principle: no contradictory rule files — p27-28
concrete proposed fix: update context-evolve to CLAUDE.md's conditional "source ~/.cargo/env if cargo isn't on PATH"; it hard-asserts the opposite of auto-drive-tauri (verified: cargo IS on PATH on this machine).
```

```
severity: med
file:line: .agents/skills/wayland/SKILL.md:3-17 (vs auto-drive-tauri/SKILL.md:3-10)
violated principle: unambiguous non-overlapping routing metadata — p27-28
concrete proposed fix: add a deflection line to the wayland skill ("for THIS repo's desktop app, load auto-drive-tauri; drive the WS bridge, not the GUI"). Cross-ref is one-directional today.
```

```
severity: low
file:line: wayland/SKILL.md ; harness-engineering/SKILL.md ; context-management/SKILL.md ; context-evolve/SKILL.md (frontmatter)
violated principle: skills state when-NOT, not only capabilities — p27-28
concrete proposed fix: add a 1-3 line "Do not use for…" block to each (3 of 8 skills already have one: llama-server, tauri, graphify-best-practices).
```

```
severity: low
file:line: CLAUDE.md (whole) vs agent-skills/src/registry.rs:18-19
violated principle: rule files complete for onboarding — p27-28
concrete proposed fix: add one CLAUDE.md line distinguishing .agents/skills/ (Claude-facing) from the runtime's own .agent/skills registry dirs — easy to author into the wrong tree.
```

**Build opportunities:** shared prompt module (`agent-runtime-config/prompts.rs`)
with a re-duplication CI guard · skill-lint script (name↔dir, description present,
when-NOT line) wired into tests · single `_facts.md` for volatile machine facts
(cargo PATH, llama port, model name) referenced by skills · prompt-eval gate
reusing the context-evolve frozen-task harness before any system-prompt edit.

---

## Component 2 — Tools

```
severity: high
file:line: shell.rs:74 ; fs/read.rs:36-38 ; git.rs:85 ; mcp/tool.rs:91-125 ; skills/tools.rs:306-308
violated principle: token-efficient tool results (truncation/pagination) — Anthropic Writing Tools for Agents
concrete proposed fix: add a loop-level ToolOutput cap (truncate + eager offload with a recall id). Today execute_command formats full stdout+stderr (a test asserts >100 KB flows through), read_file reads whole files, MCP joins verbatim; the offloader only lifts results older than keep_recent=3, so a giant result sits verbatim 3+ turns. Only fetch_url caps correctly (2 MiB / 8 KiB).
```

```
severity: med
file:line: registry.rs:13-15 ; mcp/manager.rs:42-53
violated principle: unambiguous tool names / collision handling
concrete proposed fix: make ToolRegistry::register detect duplicate names (warn+suffix or reject) and dedupe namespaced MCP names across servers. register is a bare HashMap::insert (last write silently wins); "git hub" and "git.hub" both sanitize to git_hub and shadow each other unlogged.
```

```
severity: med
file:line: assemble.rs:197-211,271-306 ; mcp/tool.rs:55-61
violated principle: contract enforcement for ALL tools — spec 2026-06-30 intent
concrete proposed fix: run required_params_missing_description on MCP schemas at connect and add remember/forget to the contract test. The ratchet test uses mcp_tools:vec![] and memory_tools:vec![], so MCP-proxied tools and remember/forget are never contract-checked anywhere.
```

```
severity: med
file:line: shell.rs:15 ; git.rs:63,78,93
violated principle: fewer consolidated tools; when-NOT for overlaps — Anthropic
concrete proposed fix: give execute_command when_not_to_call steering to read_file/list_directory/git_* siblings, and fold the three git wrappers into one `git` tool. Note the policy asymmetry: git_status is Access::Read (auto-allow) but execute_command("git status") is Access::Write (Ask) — same op, different friction, no prose to disambiguate.
```

```
severity: low
file:line: agent-memory/src/tools.rs:220-231
violated principle: non-empty param descriptions — Anthropic
concrete proposed fix: describe forget's id and query (both bare {"type":"string"}, neither required, so the ratchet is vacuous for this tool).
```

```
severity: low
file:line: agent-memory/src/tools.rs:74-77,85-88,176-178 ; render.rs:27-44 ; mcp/tool.rs:51-53
violated principle: tight descriptions / schema as source of truth
concrete proposed fix: move remember/recall arg lists into per-param descriptions (tags/scope/k undescribed); state render's per-kind requirements; cap+lint MCP description length (injected verbatim into every request).
```

**Build opportunities:** loop-level result cap + eager offload · registration-time
contract gate (one enforcement point covering MCP) · `read_file` offset/limit
pagination · dedicated bounded rg-backed `search` tool (today search = unbounded
`execute_command` at Write friction) · MCP lint-at-connect surfaced in
`ServerStatus` · consolidated `git` subcommand tool.

---

## Component 3 — Sandboxes & Execution

Docker profile itself is strong (`--network none`, `--read-only`, `--cap-drop
ALL`, `no-new-privileges`, non-root, mem/cpu/pids/tmpfs limits, mount validation
blocking `/`, docker socket, `~/.ssh|.aws|.gnupg|…`). The problem is it is almost
never *guaranteed* to be used.

```
severity: high
file:line: runtime_config.rs:106 (default_sandbox_mode -> "auto") ; strategy.rs:57-64 ; agent-cli/src/main.rs:125
violated principle: execution isolated by default — whitepaper p27
concrete proposed fix: default to "enforce" (fail-closed) OR refuse exec-capable tools when sandbox_descriptor().degraded.is_some(). VERIFIED: default is "auto", which degrades to HostExecutor (unconfined host, network=true) whenever Docker is unavailable. This also nullifies the sandbox as the documented mitigation for the exec-vehicle catastrophe residual (agent-policy/src/command.rs:213-214 names it explicitly).
```

```
severity: high
file:line: agent-tools/src/sandbox.rs:106
violated principle: capabilities explicit, not ambient; secrets filtered from child env — p28-29
concrete proposed fix: env_clear() before .envs(&spec.env) in HostExecutor, building an explicit allow-list (PATH etc.). VERIFIED: cmd.args(&spec.args).current_dir(&spec.cwd).envs(&spec.env) with no env_clear — tokio inherits the FULL parent env, and execute_command passes empty spec.env, so a degraded execute_command("printenv AGENT_API_KEY") exfiltrates the key (read via std::env::var at setup.rs:37 / cli main.rs:178, never removed). Docker path is clean.
```

```
severity: med
file:line: agent-tools/src/sandbox.rs:104-117 (host fallback)
violated principle: resource limits on executed commands — p29-30
concrete proposed fix: apply setrlimit via a pre_exec hook on the host executor (mem/pids/fsize), or gate exec tools off when degraded. Limits (memory/cpus/pids/tmp) are honored ONLY on the Docker path — a degraded run can fork-bomb or fill disk with only a wall-clock timeout.
```

```
severity: med
file:line: agent-core/src/loop_.rs:420-421
violated principle: fail-closed default — p27
concrete proposed fix: treat a None LoopConfig.sandbox as a hard error, not a silent Arc::new(HostExecutor). VERIFIED: unwrap_or_else(|| Arc::new(HostExecutor)). Latent (both frontends set Some today) but fail-OPEN.
```

```
severity: low
file:line: agent-runtime-config/src/lib.rs:228-246 (current_uid_gid) -> docker.rs:37 ; agent-mcp/src/transport.rs:38
violated principle: least privilege / workspace confinement — p28-30
concrete proposed fix: current_uid_gid() falls back to 0:0 (root-in-container) on `id` failure — fail or pick a fixed non-zero uid. Separately, MCP servers spawn with cwd = current_dir(), not the configured workspace, so their RW root can differ from the fs tools' confinement.
```

**Verified clean:** fetch_url SSRF/egress gating (DNS-pin, per-hop redirect
re-validation), fs read/write/edit symlink+`..`+absolute confinement, Docker
hardening asserts, mount validation.

**Build opportunities (the top two are the fix):** fail-closed exec under
degradation (turn the existing `SandboxDegraded` *signal* into *enforcement*) ·
`env_clear()` + allow-list env (≈2 lines, closes a live leak) · rlimit host caps
· make `enforce` the default or print a loud startup warning when unconfined.

---

## Component 4 — Orchestration Logic

Verified solid: `max_parallel_tools` (0 → `DEFAULT_MAX_PARALLEL_TOOLS=8`,
documented, no unlimited footgun); one tool message per `tool_call_id`
end-to-end (`buffer_unordered` + `normalize_tool_call_ids`, tested);
hand-off/aggregation has a no-silent-drop backstop.

```
severity: med
file:line: loop_.rs:132-152
violated principle: retry only retryable failures — first principles
concrete proposed fix: classify ModelError before retrying — retry Http/Stream/Timeout/Process, abort on Status{4xx} and Decode. Today a 400 (auth, bad request, context overflow) is re-sent verbatim 3× before failing.
```

```
severity: med
file:line: loop_.rs:117-118, 138-149
violated principle: no duplicated/unretracted output — first principles
concrete proposed fix: buffer tokens until stream completes, or emit a retraction event before re-streaming. A mid-stream failure + retry shows the user partial text then a duplicate full response.
```

```
severity: med
file:line: loop_.rs:237-243, 251-254, 143-145
violated principle: every terminal path emits a terminal event — first principles
concrete proposed fix: emit AgentEvent::Done (with an Error/Aborted stop reason) on the max_tokens-truncation, protocol-repair-exhausted, and retry-exhausted paths. Frontends keyed on Done never see termination on these three endings (browser stream hangs).
```

```
severity: med
file:line: loop_.rs:376-426 (esp. 408-411)
violated principle: cancellation observable at every await — first principles
concrete proposed fix: race the approval await against cancel.cancelled() (tokio::select!) and treat cancel as deny+stop. Ctrl-C has no effect while an approval prompt is pending — the run wedges until answered.
```

```
severity: low
file:line: loop_.rs:29-52 (#[derive(Default)] LoopConfig)
violated principle: documented default constants must be the actual defaults
concrete proposed fix: hand-impl Default so stream_idle_timeout=DEFAULT_STREAM_IDLE_TIMEOUT, tool_timeout nonzero, max_turns>=1. Today Default yields zero timeouts + max_turns=0 (runs zero turns → instant BudgetExhausted); widespread in tests, one copy-paste from prod.
```

```
severity: low
file:line: agent-model/src/protocol.rs:18-33 ; loop_.rs:191-372
violated principle: graceful degradation / loop robustness — Building Effective Agents + June audit backlog
concrete proposed fix: (a) turn one call's bad-args into a per-call error instead of failing the whole turn (one malformed call discards N-1 good ones); (b) add repeated-identical-call detection (same tool+args N turns → nudge or abort) so a stuck model burns 2-3 turns, not all 25; (c) exponential backoff w/ jitter + honor Retry-After (today linear 100ms×attempt, ~600ms total — inadequate vs rate-limited cloud).
```

**Build opportunities:** **sub-agent / decompose-and-delegate** — none exists; a
`spawn_agent` tool building a child `AgentLoop` with fresh window + restricted
registry + own budget is a natural fit (everything is already `Arc<dyn>`) ·
**intelligent model routing** (whitepaper p42) — single model per session today,
even compaction uses the expensive session model; first customer is
`run_compaction` · graceful max_turns landing (one tools-disabled wrap-up
completion) · promote `max_parallel_tools` into `RuntimeConfig` · loop-health
telemetry (stuck-model signal).

---

## Component 5 — Guardrails / Hooks / Policy

Verified correct: deterministic pre-exec hook on **every** tool call on both
surfaces (order policy→approval→execute); approval fails closed (CLI + IPC deny
on timeout / no reader); hard floor covers `rm -rf`, `dd of=/dev/*`, forkbomb,
`mkfs*`, glued operators, `$()`/backtick/subshell/group/env-prefix (tested).
Accepted exec-vehicle residual acknowledged, not re-reported.

```
severity: med
file:line: agent-memory/src/tools.rs:11-16, 93-95, 233-235
violated principle: explicit read/write/destroy tiers — whitepaper p28-30
concrete proposed fix: give remember/forget Access::Write (destructive forget especially). A helper deliberately declares memory ops Read so RulePolicy auto-allows them — a prompt-injected forget silently deletes a long-term-memory record with zero approval.
```

```
severity: med
file:line: agent-policy/src/command.rs:184-223 ; runtime-config/src/lib.rs:170-173
violated principle: permission tiers, not flat allow/deny — first principles
concrete proposed fix: make the allowlist subcommand-aware for exec-capable programs. is_auto_allowed checks only tokens[0]; `git` is allowlisted, so `git push --force`, `git reset --hard`, `git clean -fdx` reach Decision::Allow — destructive history loss reaching Allow, not merely the documented Ask residual. `cargo` (build scripts) has the same property.
```

```
severity: med
file:line: agent-tools/src/lib.rs (Access enum) ; agent-policy/src/engine.rs:44-55
violated principle: explicit read/write/destroy tiers — p28-30
concrete proposed fix: add an Access::Destroy tier (or ToolIntent.destructive). The engine is a 2-state fold (Read→Allow-if-inside, Write→Ask); overwriting a tracked file is indistinguishable from a scratch write, and git_commit (git add -A + commit) sits in the same tier as a one-line note.
```

```
severity: low
file:line: agent-core/src/loop_.rs (gate→execute path — no post-exec hook) ; agent-policy/src/engine.rs:49-52 ; agent-mcp/src/tool.rs:63-77
violated principle: post-exec hooks; fast side-effect-free decision hook; floor covers all vehicles — p28-30
concrete proposed fix: (a) add an optional post-execution validator hook (after edit/write/commit) — none exists; (b) the read-boundary decision hook calls resolve_in_workspace which does FS I/O (canonicalize/read_link, ≤40 hops) at decision time — defer to execute() which already re-checks; (c) note that Trust::Allow MCP tools bypass the floor entirely and Trust::Ask ones can never be Denied.
```

**Observations (fail-safe but noted):** `shutdown`/`reboot`/`chmod -R 777 /`/
`curl|sh`/`> /dev/sda` are not floored but reach Ask (not auto-allowed) — though
`dd of=/dev/sda` is Denied while `echo x > /dev/sda` only reaches Ask (redirection
asymmetry). `ApproveAlways` is a no-op distinction vs `Approve` (nothing
persists it) — safe but misleads the operator.

**Build opportunities:** Destroy tier (closes the three med findings together) ·
subcommand-aware allowlist for git/cargo · post-execution validator hook ·
split the read boundary check into pure-lexical decision + FS-resolving execute ·
redirection-to-block-device handler for `dd` parity.

---

## Component 6 — Observability

```
severity: high
file:line: loop_.rs:282-284, 312, 343-349
violated principle: replay/diagnose a failed turn; drift detection — p28/p30
concrete proposed fix: emit ToolResult{id,name,status:ok|denied|error|timeout|panic,content} for EVERY resolved call at the Phase-3 drain. Today only Resolved::Ok emits; gate rejections and tool errors are silent, so every failed call shows a ToolStart with no paired terminal event — tool-error rate is uncomputable.
```

```
severity: high
file:line: loop_.rs:302 (execute) ; event.rs:21 ; wire.rs:23-28
violated principle: latency metering — p30
concrete proposed fix: capture std::time::Instant around execute_isolated and add duration_ms to ToolResult; likewise time one_completion for turn latency. VERIFIED zero timing capture anywhere in agent-core/agent-tools.
```

```
severity: high
file:line: agent-server/src/session.rs:30-43 ; sink.rs:23-26 ; agent-cli/src/main.rs:235-241
violated principle: enough context to replay a failed turn — p28
concrete proposed fix: add a JsonlEventSink teeing every AgentEvent (+turn index, timestamps) to ~/.agent/sessions/<id>.jsonl in both frontends. Nothing is persisted on either surface today; offload store is in-memory; claude-cli passes --no-session-persistence. A failed turn can only be diagnosed by re-running.
```

```
severity: med
file:line: wire.rs:119 ; agent-cli/src/render.rs:81 (emitters curated.rs:163,205,216)
violated principle: "quietly drifting" — p28
concrete proposed fix: add ServerEvent::Context and forward all three ContextEvent variants (CompactionFailed at minimum). Both UIs drop AgentEvent::Context, so neither ever learns the window was truncated or compaction failed.
```

```
severity: med
file:line: agent-model/src/claude_cli.rs:152-183
violated principle: token cost metering — p30
concrete proposed fix: parse the CLI "result" event usage (input/output tokens, total_cost_usd — present in stream-json). claude-cli sessions report ServerUsage 0/0 today; total_cost_usd is the ONLY dollar signal anywhere in the system.
```

```
severity: med
file:line: agent-model/src/openai.rs:205-209 ; types.rs:75 ; loop_.rs:377,345 ; event.rs:20-21
violated principle: reasoning-spend metering; trace correlation — p28/p30
concrete proposed fix: parse reasoning_tokens/cached_tokens from completion_tokens_details and thread through ServerUsage; add the tool_call id to ToolStart/ToolResult so parallel same-tool calls are start→result correlatable.
```

```
severity: low
file:line: agent crates (0 cost-metering hits) ; sink.rs:23-26
violated principle: cost/drift aggregates — p28/p30
concrete proposed fix: accumulate per-session totals (cumulative tokens, tool calls, tool-error count, turns) exposed via a session_stats query; buffer events in a bounded ring when no subscriber so pre-subscribe events aren't lost.
```

**Note:** project-memory "message_tokens ignores reasoning + tool_calls" is
**stale/fixed** (context.rs:13-23 counts both, tested). Remaining gap is
heuristic estimate vs server truth (see Context #2).

**Build opportunities:** structured per-call terminal event (one drain change →
tool-error/denial-rate for free) · JSONL trace sink (one tee in assemble_loop
covers CLI+server+desktop) · claude-cli usage/cost parsing · SessionStats
accumulator feeding a web Context-Explorer drift dashboard · forward
ContextEvent to the UI (explains token drops on the existing chart).

---

## Spine B — Context Engineering

Best-in-audit area (explicit static/dynamic boundary, spec'd; user turns survive
compaction verbatim; running summary carries forward; offload idempotent with
recallable placeholders; true 3-level skill disclosure with per-call re-scan =
no staleness).

```
severity: high
file:line: context.rs:147-155 ; curated.rs:134-141, 174-184
violated principle: transcript coherence under curation — Anthropic Effective Harnesses
concrete proposed fix: evict/split at turn boundaries — treat an assistant tool_calls message + its Role::Tool results as one atomic unit. Both build() impls walk history per-message newest-first; a kept tool message whose parent was evicted is serialized with tool_call_id and OpenAI-compat servers 400 it — exactly when the window is fullest.
```

```
severity: high
file:line: context.rs:9-11 with loop_.rs:196-228, curated.rs:167-172
violated principle: token accounting vs ground truth — whitepaper p16; project memory
concrete proposed fix: feed ServerUsage prompt_tokens (already parsed/emitted) into MaintCtx as a calibration for the high-water gate + eviction budget, instead of chars/4. estimate_tokens undercounts code (~3.2 c/tok) and severely undercounts CJK/emoji → compaction fires too late → real overflow.
```

```
severity: med
file:line: loop_.rs:140-151, 196-216
violated principle: graceful degradation mid-session — Effective Harnesses
concrete proposed fix: on a context-overflow model error, force a maintain() compaction and rebuild once before the generic retry counts a fatal attempt. Also build() keeps ≥1 message even if it alone exceeds budget → a single oversized tool result guarantees an unfixable over-limit request.
```

```
severity: med
file:line: loop_.rs:175-180
violated principle: dynamic context re-evaluated per task — whitepaper p15
concrete proposed fix: call ctx.set_recall(lines) unconditionally (incl. empty) so a turn retrieving nothing clears the prior recall block. Contexts persist across REPL/session runs, so a stale recall block stays pinned. Retrieval also runs once per run against the raw first user_input, never refreshed across tool loops.
```

```
severity: med
file:line: context.rs:144-155 ; curated.rs:131-141
violated principle: no silent loss — Effective Context Engineering
concrete proposed fix: emit ContextEvent::Evicted (count+est tokens) and/or route the evicted tail through the offload store. Plain window eviction silently omits messages — no event, no placeholder, invisible to model and user (offload/compaction both emit events; eviction doesn't).
```

```
severity: low
file:line: skills/presets.rs:10-25 ; presets.rs:4-5 & tools.rs:48-61 ; offload.rs:43-55 ; snapshot.rs:57-64
violated principle: static context budgeted; progressive-disclosure L1; durable artifacts; faithful observability
concrete proposed fix: (a) log composed system-prompt token estimate + warn past a fraction of context_limit — compose_system_prompt inlines full preset bodies (≤64KB each, ~16K tokens on an 8192 limit) uncapped; (b) optionally inline the list_skills catalog into the prompt so matching is passive not model-initiative; (c) add a persisted OffloadStore behind the existing trait seam + cap growth (all session state is RAM-only, lost on restart); (d) compute snapshot "memory" segment from the 512-token-capped recall_block, not the sum of all lines.
```

**Examples context type is absent entirely** — no few-shot / reference-pattern /
exemplar mechanism anywhere.

**Build opportunities:** Examples type (per-skill `examples/` bundled exemplars
surfaced via the existing read_skill_file L3 path — cheapest: skills already
support bundled files, only a convention + prompt note missing) ·
server-usage-calibrated budgeting · turn-atomic curation helper shared by
eviction + compaction · durable session artifacts (persisted offload +
transcript checkpoint) · static-context budget gate at compose/assemble time.

---

## Dimension — Eval & Quality Flywheel

Far above baseline: a real correctness-gated/token-tiebreak eval loop
(`agent-runtime-config/src/eval/` + `tests/eval_context.rs` + `eval_gate` CLI),
sealed hidden oracles (`.agents/skills/context-evolve/tasks/`), two-sided task
admissibility, ~455 tests (no untested crate), specs that carry `## Testing`
sections and become tests, and harness-vs-model isolation solved for the context
subsystem (model pinned via `AGENT_E2E_MODEL`, only `CandidateConfig` varies).

```
severity: high
file:line: .github/workflows/ (absent — no CI, no git hooks, no Makefile/justfile)
violated principle: continuous quality flywheel — verify on every change (whitepaper); CLAUDE.md "changes ship with tests" is human-enforced only
concrete proposed fix: add a GitHub Actions workflow: cargo fmt --check + clippy + cargo test (agent/) + cd web && npm run typecheck && npm test on every PR. e2e_robustness.rs and e2e_context_management.rs already self-describe as "Deterministic, CI-runnable" and have never had a CI to run in.
```

```
severity: med
file:line: agent-runtime-config/src/eval/result.rs:6-10
violated principle: trajectory/process evals — own playbook eval.md:60-63; arXiv 2503.16416 §3
concrete proposed fix: add trajectory: Vec<{tool,args}> to RunResult (captured from ToolStart in the eval sink) + a gold-trajectory subset comparator. Today `turns` is the only process signal and the gate ignores it; no LM-judge or rubric anywhere.
```

```
severity: med
file:line: agent-core/src/event.rs:13-32 ; agent-runtime-config/tests/eval_context.rs:172-174, 24/55
violated principle: failure logging → evals (monitor step); conflation coverage
concrete proposed fix: (a) persist trajectories (opt-in transcript sink) so real-usage failures become eval tasks — nothing writes a session JSONL today; (b) widen CandidateConfig beyond context knobs (base_system_prompt hardcoded, protocol pinned native) to prompt/protocol/tool-description variants so the same gate attributes the harness changes that actually churned most; (c) SafeApproval.denied is accumulated then never read — emit it.
```

```
severity: med
file:line: agent-policy/src/command.rs (and denylist tests)
violated principle: regression suite guarding the hottest component (14 policy commits/7 days)
concrete proposed fix: add a data-driven adversarial corpus (one file of command → expected {Allow|Ask|Deny}, incl. all closed bypasses) as a table test, so new wrapping tricks are one-line additions; optionally proptest the tokenizer.
```

```
severity: low
file:line: agent-cli (7 tests) ; no coverage tooling repo-wide
violated principle: measure blind spots
concrete proposed fix: add cargo llvm-cov to the new CI job — makes agent-cli render/approval and sandbox docker-strategy thinness a tracked number.
```

```
severity: low
file:line: .agents/skills/context-evolve/program.md / MEMORY.md campaign state
violated principle: flywheel cadence — paused mid-loop
concrete proposed fix: the campaign's own log lists an open discriminator (locked-portmap 4/10 vs favorable 5/5) as priority #1 since 2026-06-29; schedule the next iteration or the flywheel stops after v1→v2.
```

**Build opportunities (ranked):** CI workflow (amplifies everything; two suites
built for it already) · trajectory capture + comparator (unlocks process scoring;
the portmap diagnosis likely needs exactly this) · adversarial denylist corpus
table test · session transcript sink (closes monitor→evaluate) · widen
CandidateConfig to prompt/protocol/tool vars · coverage in CI · resume the
context-evolve campaign.

---

## Cross-cutting themes

1. **"Local-first" and "isolated-by-default" are in tension and isolation lost.**
   The single most important fix cluster: Sandbox #1/#2/#3 + the env leak. The
   accepted exec-vehicle policy residual is only acceptable *if* the sandbox is a
   real backstop — by default it isn't.
2. **The harness can't see itself.** No persisted trace, no tool-failure event,
   no duration, no CI. Observability #1-3 + Eval CI + trajectory capture are one
   coherent investment: make every run replayable and every failure harvestable.
3. **Two-state permission model.** A Destroy tier (Guardrails) + subcommand-aware
   allowlist close a real auto-Allow-destructive gap (`git push --force`, memory
   `forget`) and are the highest-value guardrail work.
4. **Two whole capability categories are missing:** sub-agents and Examples.
   Both are greenfield build opportunities the architecture already accommodates.
