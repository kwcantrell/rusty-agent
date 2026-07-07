# Harness + SDLC Audit — rust-agent-runtime

**Date:** 2026-07-06
**Method:** `harness-engineering` `audit.md` playbook (procedure) + `agent-sdlc` OKF bundle
(evidence layer) — 11 dimensions, one auditor each, every finding adversarially verified
(default-refute) before inclusion. Workflow run id: `wf_5f1cb5a5-295` (55 agents: 11 auditors,
44 verifiers).
**Spec:** `docs/superpowers/specs/2026-07-06-full-harness-sdlc-audit-design.md`
**Predecessor:** `docs/superpowers/audits/2026-07-01-harness-deep-audit.md` (fully closed 2026-07-02).
**REPORT ONLY.** No code was changed. Line numbers drift — re-open before acting.

Severities shown are the **verifier's** independent call; where the auditor disagreed the
original is noted inline. Refuted findings appear only in Appendix A.

---

## Ground truth

### okf_check

```
$ python3 scripts/okf_check.py docs/okf/agent-sdlc
OK
exit=0
```

Bundle conformance clean (frontmatter, citations, intra-bundle links).

### ci.sh

```
$ bash scripts/ci.sh
[... fmt + clippy + cargo test (agent/) + web typecheck + vitest ...]
 Test Files  65 passed (65)
      Tests  295 passed (295)
   Duration  12.08s
CI gate passed.
exit=0
```

Full gate green: rustfmt, clippy, `cargo test` across the `agent/` workspace,
web typecheck, and 295 vitest tests across 65 files. One known-noise stderr:
an `act(...)` warning from `App.tauri.test.tsx` (the documented vi.mock
file-scope leak; test passes).


---

## Executive summary

This is the first audit of this runtime with **zero high-severity findings**. The 2026-07-01
deep audit's entire surface was re-verified in live source by every dimension's auditor and
**no regression was found anywhere** — turn-atomic curation, the fail-closed sandbox, the
position-aware hard floor, retry classification, sub-agent privilege inheritance, the
observability spine, and the instructions single-source ratchet all held. Of 44 findings
raised, 41 survived adversarial verification (18 med, 23 low); the 3 refutations were two
documented accepted-residuals the auditor missed and one misread of the sequential approval
gate — the verify stage earned its keep in both directions.

The confirmed findings cluster in four places:

1. **MCP is the softest seam.** MCP-proxied tool schemas are the one tool class that reaches
   the model with no contract lint at all (tools #1); a `Trust::Allow` MCP tool is encoded
   `Access::Read`, making trusted-MCP *mutations* both auto-allowed and invisible to the new
   post-exec validator (guardrails #1); and MCP server secrets are passed as `docker run -e
   KEY=VALUE` argv, world-readable in `/proc/<pid>/cmdline` for the container's lifetime
   (sandbox #1). Three dimensions independently converged on the same boundary.

2. **Sub-agent composition gaps.** The capability shipped as three individually-reviewed
   sub-specs, but the seams *between* clusters were never reviewed as a whole: the post-drain
   description-override seam never reaches child registries (orchestration #1), a routed child
   model inherits the primary's context window for budgeting (orchestration #2), a child
   killed by wall-clock timeout discards its entire partial transcript (orchestration #4), and
   the budget wrap-up prompt pollutes durable history (orchestration #3).

3. **The new surfaces carry classic first-audit findings.** The design-tab harness trusts an
   SPA-provided path at the dev-server spawn boundary (design-tab #1), the desktop CSP never
   grants `frame-src` for the live-preview iframe (design-tab #3), and one URL check
   prefix-matches its host (design-tab #4). The knowledge layer's conformance checker is not
   in any CI leg (found independently by two dimensions), and one skill steers agents at a
   test deleted in `474b7af`.

4. **CI legs that lag the repo's growth.** `src-tauri` remains outside `scripts/ci.sh`
   (process #1), the OKF bundle gate is manual (eval-flywheel #1 / skills #2), and the
   coverage number lives only in expiring Actions logs.

Nothing here invalidates the July close-out: these are new-surface gaps, cross-cluster
composition seams, and drift that post-dates the fixes — exactly what a re-audit exists to
catch. Triage and fixes are separate spec→plan cycles; the owner holds the judgment gate.

### Top 10 highest-leverage fixes

| # | Sev | Dimension | file:line | One-line fix |
|---|-----|-----------|-----------|--------------|
| 1 | med | Sandbox | `agent/crates/agent-sandbox/src/docker.rs:73-77` | Pass MCP env secrets as name-only `-e KEY` + client-process env (or a 0600 `--env-file`) — stop broadcasting them in world-readable argv |
| 2 | med | Design-tab | `src-tauri/src/devserver.rs:176-199` | Canonicalize the SPA-provided dir and require workspace containment before spawning the dev server |
| 3 | med | Guardrails | `agent/crates/agent-mcp/src/tool.rs:69-74` + `loop_.rs:828-834` | Decouple MCP auto-allow from the declared tier so `Trust::Allow` mutations still trip the post-exec validator |
| 4 | med | Tools | `agent/crates/agent-mcp/src/tool.rs:61-67` | Lint MCP schemas at connect (empty descriptions, undescribed required params) — warn-don't-reject |
| 5 | med | Orchestration | `agent/crates/agent-runtime-config/src/runtime_config.rs:16-29` | Give `ModelRef` an optional `context_limit`/`max_tokens` so routed children budget against their own window |
| 6 | med | Orchestration | `agent/crates/agent-core/src/dispatch.rs:468-486` | Return the child's partial transcript (`sink.summary()`) on timeout/failure instead of a bare error |
| 7 | med | Context | `agent/crates/agent-core/src/curated.rs:206-210` | Cap the pinned goal block at a token budget (mirror `DEFAULT_RECALL_TOKEN_BUDGET`) |
| 8 | med | Observability | `agent/crates/agent-core/src/loop_.rs:460` | Record run inputs (user prompt + system-prompt hash/override set) in the trace — replay and eval harvest need them |
| 9 | med | Process | `scripts/ci.sh:2-3` | Add a conditional `src-tauri` leg (build + clippy + test; skip when GTK dev deps absent) |
| 10 | med | Eval + Skills | `scripts/ci.sh:9-19` | Gate the OKF bundle: `test_okf_check.py` + `okf_check.py` leg in ci.sh (raised independently by two dimensions) |

## Prior-state regressions

**None found — all July fixes verified in place.** Every dimension's auditor re-checked the
fixes touching its surface in live source; each `prior_state` paragraph below records what was
spot-checked. One evolution worth noting (not a regression): the tool-description override
seam DECLINED-FOR-NOW on 2026-07-02 was subsequently built (`a0bbf0d`) — its revisit trigger
evidently fired — and correctly preserves the `when_not_to_call` fold (pinned by test).

---

## Findings by dimension


### 1. Instructions & rule files — findings

**Prior state:** The 2026-07-02 instructions-cluster fixes (bc8934e) are all still in live source and verified this session: BASE_SYSTEM_PROMPT lives only in agent-runtime-config/src/prompts.rs:9-13 with the negative-constraints clause (workspace confinement, no sandbox/policy bypass, no secrets), re-exported at agent-server/src/daemon.rs:25 (`pub use agent_runtime_config::BASE_SYSTEM_PROMPT as SYSTEM_PROMPT`); both pin tests pass live (`cargo test -p agent-runtime-config --lib prompts` → 2 passed, including the re-duplication ratchet scanning agent/crates, src-tauri/src, src-tauri/tests with the repo-root vacuous-pass guard at prompts.rs:41-44). All 9 skills that existed at fix time still carry their "Do not" blocks; the context-evolve↔auto-drive-tauri cargo-PATH contradiction remains eliminated both ways (context-evolve/SKILL.md:33 and auto-drive-tauri/SKILL.md:37,235 both use CLAUDE.md's conditional form); the wayland→auto-drive-tauri deflection is present (wayland/SKILL.md:30); CLAUDE.md's Gotchas still distinguish .agents/skills/ from the runtime's .agent/skills trees (CLAUDE.md:99-102). Sub-agent prompts remain versioned and role-specific: SUBAGENT_PREAMBLE is a named const (dispatch.rs:15-18), the child system prompt is composed from the single-source prompt (assemble.rs:250-253), and the role arg is bounded at MAX_ROLE_CHARS=2000 with type/emptiness validation (dispatch.rs:20-21,322-336); other inline system strings (compactor.rs COMPACTION_SYSTEM/EXTRACTION_SYSTEM, claude_cli.rs warmup) are named, role-specific consts. No regression of any closed fix. The single finding is post-fix drift: the agent-sdlc skill added 2026-07-06 (b60fad7) does not follow the Do-not house style the cluster ratcheted in.


**Finding 1.1** — `med`

- **file:line:** `.agents/skills/agent-sdlc/SKILL.md:3-11`
- **violated principle:** Skill rule files carry negative constraints and unambiguous, non-overlapping frontmatter descriptions — descriptions are the routing signal, and a broadly-triggering skill without a forbidden zone creates ambiguous decision points.
- **source:** docs/okf/agent-sdlc/practices/tool-design-as-engineering.md
- **concrete proposed fix:** Add the house-style '**Do not**' block to agent-sdlc/SKILL.md (e.g. 'Do not use for doing harness design/build/audit work in this repo — that is the harness-engineering playbooks; this bundle is the evidence layer behind them. Do not edit bundle files without reading authoring.md / running scripts/okf_check.py.'), and consider narrowing the 'Use for any question about…' opener that overlaps harness-engineering's frontmatter.
- **evidence:** Frontmatter: 'Use for any question about how to build, evaluate, deploy, or operate AI agents — evals, tool design, context engineering, … harness design'. grep for 'Do not|Don't|Never' over the file returns zero hits — it is the only skill of 10 in .agents/skills/ without a negative-constraints block, breaking the house style the 2026-07-02 instructions cluster (bc8934e re-stamp: 'all 8 skills now carry a Do not block … house style') established. Its trigger overlaps harness-engineering's ('Use to design, audit, build, or evaluate the *harness*…'); the body's 'Relationship to harness-engineering' section is positive guidance ('load both'), not a boundary, and only visible after the skill body loads.
- **verifier:** Re-derived: agent-sdlc is the only skill of 10 in .agents/skills/ without a '**Do not**' negative-constraints block (case-sensitive grep = 0 hits), breaking the house style commit bc8934e explicitly established from a prior audit Finding 1; its frontmatter lists 'harness design' verbatim overlapping harness-engineering's trigger, and the disambiguation exists only in the post-load body. No spec or re-stamp documents this as an accepted residual — the skill just post-dates the house-style pass. Routing-precision/leverage issue, not correctness: med.

### 2. Tools — findings

**Prior state:** All July fixes touching the Tools dimension are still in live source: the `when_not_to_call` contract and marker fold (agent-tools/src/tool.rs:13-15, registry.rs:56-59), the required-param description ratchet (contract.rs:25-45) enforced over the whole assembled registry (assemble.rs:671-684), the CONFUSABLE_TOOLS coverage ratchet with `recall` enforced in agent-memory (assemble.rs:697-729; agent-memory/src/tools.rs:730-782), the 16 KiB ingestion cap (offload_policy.rs:10), `context_recall` byte-offset paging (context_tools.rs:8-121), `read_file` offset/limit with described params (fs/read.rs:33-37), duplicate-name warn with pinned last-wins (S8, registry.rs:19-26 + test at 298-307), memory optional-param descriptions (S9, agent-memory/src/tools.rs:117-119,262-263,336-338), the `use_skill` 50-file listing cap (S10, agent-skills/src/tools.rs:15,149), the dispatch_agent schema contract test (agent-core/tests/dispatch_tool.rs:460), and memory tiering remember=Write/recall=Read/forget=Destroy (agent-memory/src/tools.rs). No regressions. One notable evolution: the tool-description override seam that was DECLINED-FOR-NOW on 2026-07-02 has since been built (commit a0bbf0d, registry.rs:39-46) wired through RuntimeConfig and the eval genome — its revisit trigger evidently fired; the override correctly preserves the when_not_to_call fold (pinned by test at registry.rs:321-347), so it is consistent with the contract, not a regression.


**Finding 2.1** — `med`

- **file:line:** `agent/crates/agent-mcp/src/tool.rs:61-67 (schema passthrough); agent/crates/agent-mcp/src/manager.rs:44-57 (no lint at connect)`
- **violated principle:** Contract enforcement must cover ALL tools the model sees — MCP-proxied schemas are injected verbatim into every request with no description or required-param check anywhere.
- **source:** docs/okf/agent-sdlc/practices/tool-design-as-engineering.md
- **concrete proposed fix:** In connect_one/McpManager::connect, run required_params_missing_description (and a description length/emptiness lint) on each RawTool's schema, surfacing violations via tracing::warn and a field on ServerStatus (warn-don't-reject, matching duplicate-name posture).
- **evidence:** McpTool::schema(): `parameters: self.input_schema.clone()` with `description: self.description.clone()` — raw server prose straight to the model; manager.rs just `tools.extend(server_tools)`. The enforcement test in assemble.rs uses `mcp_tools: vec![]`, so no MCP tool is ever contract-checked. This is the unresolved MCP half of the 2026-07-01 audit's Tools finding 3 (the memory half was fixed); it appears in no ACCEPTED-BY-DESIGN list, cluster, or product decision — it fell through the drain triage.
- **verifier:** Re-derived in live source: McpTool::schema() injects raw server description/input_schema verbatim, connect_one/McpManager::connect apply no contract lint, and the assemble.rs ratchet uses mcp_tools: vec![] so MCP schemas are never checked anywhere (grep confirms no required_params_missing_description call in agent-mcp). The backlog drain fixed only the memory half (Sweep S9) and dup-name warn (S8); the 2026-06-30 spec's 'out of scope' note covers static tests only and the 2026-07-01 audit later re-raised connect-time lint as the fix — no fix, accepted residual, or product decision covers it, so the gap fell through the drain as claimed. Med per rubric: reliability/leverage gap in tool steering, not a safety-boundary violation.

**Finding 2.2** — `med`

- **file:line:** `agent/crates/agent-tools/src/shell.rs:19-21; agent/crates/agent-tools/src/contract.rs:14-21`
- **violated principle:** Overlapping tools need explicit 'when NOT to call' steering — execute_command subsumes read_file/list_directory/git_status/git_diff yet carries no exclusion prose and is absent from CONFUSABLE_TOOLS.
- **source:** docs/okf/agent-sdlc/practices/tool-design-as-engineering.md
- **concrete proposed fix:** Add when_not_to_call() to ExecuteCommand steering to read_file/list_directory/git_* for operations those tools do directly, and add execute_command to CONFUSABLE_TOOLS so the ratchet pins it.
- **evidence:** description(): "Run a shell command in the workspace directory." — no siblings named, no when_not_to_call impl. The 2026-07-01 MED's fix text included this steering; only its friction-asymmetry half (git_status vs `git status`) was fixed in the permissions cluster, and this half appears in no accepted-residual list. A `cat`/`ls` via shell is Access::Write (Ask for non-allowlisted forms) and skips per-tool path policy — prose is the cheap fix without re-opening the declined git consolidation.
- **verifier:** Live source confirms execute_command has a bare description, no when_not_to_call() impl, and is absent from CONFUSABLE_TOOLS despite the fold+ratchet infrastructure existing; the audit re-stamp shows only the friction-asymmetry half of the original 2026-07-01 MED was fixed, and the declined-by-owner item was git consolidation (a different action), so the steering half is neither fixed nor an accepted residual. Med is right: it's a tool-selection leverage/efficiency gap, not a correctness/safety defect.

**Finding 2.3** — `low`

- **file:line:** `agent/crates/agent-core/src/dispatch.rs:246-253 vs dispatch.rs:407-425`
- **violated principle:** Tool descriptions must stay current with behavior — dispatch_agent's prose predates the sub-spec #3 depth feature and now contradicts both the code and its own tools-arg description.
- **source:** docs/okf/agent-sdlc/practices/tool-design-as-engineering.md
- **concrete proposed fix:** Qualify the description sentence, e.g. "(minus dispatch_agent itself, unless nesting depth allows — then children may dispatch their own)", so it is true at every depth configuration.
- **evidence:** description(): "same permissions and tools as you (minus dispatch_agent itself)" — but at dispatch.rs:414 `if nested_allowed && nested_named { reg.register(... DispatchAgentTool ...) }` grants the child dispatch_agent BY DEFAULT (no allowlist needed) whenever subagent_max_depth > 1, and the tools-arg prose ("Include dispatch_agent to let the sub-agent dispatch its own") describes only the allowlist path. Accurate only under the default max_depth=1.
- **verifier:** Re-derived: description() unconditionally states the child gets tools "minus dispatch_agent itself", but dispatch.rs:411-425 registers a nested DispatchAgentTool in the child registry by default (nested_named is true when no allowlist is given) whenever depth < max_depth, so the sentence is false for any subagent_max_depth > 1; the tools-arg prose (line 277) covers only the allowlist path. Spec G7/G9 (2026-07-02-subagent-advanced-dispatch-design.md) updated the allowlist prose and appended the fan-out sentence but never amended the main description, and no accepted-residual entry covers it. Severity stays low: default_subagent_max_depth() is 1 (runtime_config.rs:215-217), so the description is accurate under the default config and the gap is doc-accuracy polish, not correctness/safety.

**Finding 2.4** — `low`

- **file:line:** `agent/crates/agent-skills/src/tools.rs:225-232; agent/crates/agent-tools/src/contract.rs:25-45; agent/crates/agent-tools/src/render.rs:81-82` *(auditor cited `agent/crates/agent-skills/src/tools.rs:224-231; agent/crates/agent-tools/src/contract.rs:25-45; agent/crates/agent-tools/src/render.rs:80-82`)*
- **violated principle:** Poka-yoke argument shapes: the required-param-description ratchet inspects only top-level `required`, so nested required params and de-facto per-kind required params escape enforcement undescribed.
- **source:** docs/okf/agent-sdlc/practices/tool-design-as-engineering.md
- **concrete proposed fix:** Recurse required_params_missing_description into array `items` object schemas (create_skill files[].path/content currently bare), and describe render's columns/rows params with their `(kind=table)` requirement, matching the lang/mime pattern.
- **evidence:** create_skill files.items: `"path": {"type": "string"}, "content": {"type": "string"} ... "required": ["path", "content"]` — both undescribed yet required; the checker reads only `params.get("required")` at the top level. render's `"columns": {"type": "array", ...}` and `"rows"` carry no description at all while sibling params state their kind requirement.
- **verifier:** Re-derived from live source: the ratchet reads only top-level `required`/`properties`, create_skill's files.items requires undescribed path/content, and render's columns/rows are undescribed with only sibling params carrying (kind=...) annotations. No fix, test, or documented accepted-residual found in specs/, the audit report, or the sdd ledger; the related prior audit low ("state render's per-kind requirements") was only partially addressed. Polish-tier: advisory descriptions, no correctness/safety impact.

**Finding 2.5** — `low`

- **file:line:** `agent/crates/agent-memory/src/tools.rs:106-109,244-247,326-329` *(auditor cited `agent/crates/agent-memory/src/tools.rs:106-109,244-246,326-329`)*
- **violated principle:** Single source of truth for argument prose — remember/recall/forget still duplicate their arg lists in the base description alongside the (now present) per-param descriptions, a drift-prone redundancy the 2026-07-01 LOW asked to eliminate.
- **source:** docs/okf/agent-sdlc/practices/tool-design-as-engineering.md
- **concrete proposed fix:** Drop the "Args: ..." sentences from the three base descriptions now that every param carries its own description (the prior LOW's fix was applied half-way: params described, duplicate prose retained).
- **evidence:** remember description(): "... Args: text (required), tags (optional string array), scope ('project'|'global', default project)." while the schema separately describes text/tags/scope; recall: "... Args: query (required), k (optional)."; forget: "Args: either id (exact) or query (...)". Two copies of the same contract per tool in every request.
- **verifier:** Live source confirms all three base descriptions still carry "Args: ..." prose (lines 108, 246, 327) while every parameter now has its own schema description (text/tags/scope, query/k, id/query) — two independently editable copies of each contract per request. The backlog-drain ledger (.superpowers/sdd/progress-2026-07-backlog-drain.archive.md) shows only "optional-param descriptions (forget id/query, recall k)" was shipped; no spec, re-stamp, or accepted-residual note covers retaining the duplicate prose, so the prior LOW's "move arg lists into per-param descriptions" fix was indeed applied half-way. Polish-tier: drift-prone redundancy plus minor per-request token cost, no correctness or leverage impact.

### 3. Sandboxes & execution — findings

**Prior state:** All 2026-07-01 sandbox-cluster fixes are still live and verified this session: auto-mode fail-closed refusal with actionable opt-out copy (strategy.rs:212-228 — the old degrade-to-host arm is gone, and the previously accepted "enforce refusals lack actionable copy" residual has since been closed too, both arms now name "start Docker / sandbox_mode=off"); the self-healing single-flighted 2s-bounded re-probe (strategy.rs:70-98, enforce never re-probes); HostExecutor env_clear + six-var allow-list with spec.env winning (sandbox.rs:129-145, leak test intact); LoopConfig.sandbox as a required non-Option field with the documented test-only HostExecutor Default (loop_.rs:109/117-143, accepted residual unchanged and its rationale still holds); MCP servers spawned with cwd=workspace and skipped loudly under refusal (transport.rs:40-46, manager.rs:59-61); current_uid_gid nobody fallback never 0:0 (lib.rs:326-355). The re-stamp's flagged "new residual for the next audit pass" — claude_cli.rs spawning the model backend with AGENT_API_KEY in env — has been FIXED since: the spawn now carries .env_remove("AGENT_API_KEY") (claude_cli.rs, "The CLI authenticates via its own subscription… must not inherit the runtime's model API key"). The e60710e dev-image work is live: agent-sandbox-dev:latest default with tri-state ImageProbeOutcome (Exists/Missing/Indeterminate) and explicit-image-never-substituted fallback (runtime_config.rs:224-226, strategy.rs:26-146, lib.rs:265-279); sandbox-image/Dockerfile + smoke.sh exercise the exact hardening flags including noexec /tmp. Exec paths added since July 1 remain fail-closed: the post-exec validator (5f41db5) runs through the injected sandbox and Skips under refusal (loop_.rs:1238-1270), and dispatch children share the parent's sandbox Arc via child_config = loop_config.clone() (assemble.rs:237, dispatch.rs:214-216 invariant). Egress stays gated per-tool (fetch_url NetworkPolicy allowlist, sandbox_network default false → --network none). No regressions found; the five findings above are new gaps, mostly polish plus one med secret-exposure path on the docker MCP route.


**Finding 3.1** — `med`

- **file:line:** `agent/crates/agent-sandbox/src/docker.rs:73-77`
- **violated principle:** Secrets must be granted to a sandboxed workload privately, not broadcast — passing env values as docker-run argv makes MCP server credentials world-readable on the host (/proc/<pid>/cmdline) for the whole session and persists them in `docker inspect`.
- **source:** docs/okf/agent-sdlc/phases/testing-and-safety.md
- **concrete proposed fix:** In docker_run_args, emit name-only `-e KEY` and set the value on the docker client process env (cmd.env in DockerSandbox::spawn_docker), or write a 0600 --env-file; keep `HOME=/tmp` as-is (non-secret).
- **evidence:** docker.rs: `for (k, v) in &spec.env { a.push("-e".into()); a.push(format!("{k}={v}")); }` — fed from agent-mcp/src/transport.rs:42 `env: spec.env.clone()` (mcp.json `env` conventionally carries API keys, config.rs:22); Service-kind MCP containers keep the `docker run` client — and its argv — alive for the session. HostExecutor by contrast sets env on the child (sandbox.rs:145), not argv, so the docker path is strictly more exposed.
- **verifier:** Re-derived end-to-end: docker_run_args emits -e KEY=VAL into the docker client argv (docker.rs:73-77), spawn_docker (strategy.rs:180-186) passes it verbatim with no env passthrough, and MCP Service servers (transport.rs:42, ProcKind::Service) keep that client — and its world-readable /proc cmdline — alive for the session while mcp.json env conventionally carries API keys; HostExecutor (agent-tools/src/sandbox.rs:145) sets env privately via cmd.envs, so the docker path is strictly more exposed and no fix/test/accepted-residual exists. Med fits: real secret-leak channel but needs a co-resident local user on a single-user local-first runtime; fix is one concrete action (name-only -e + cmd.env on the client).

**Finding 3.2** — `low`

- **file:line:** `agent/crates/agent-runtime-config/src/lib.rs:284`
- **violated principle:** Documentation of a safety boundary must match its behavior — a doc comment claiming auto mode 'degrades to host' invites a future maintainer to 'restore' the fail-open behavior the 2026-07-01 fail-closed fix removed.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Rewrite the build_sandbox doc line to: `anything else (e.g. "auto") → DockerSandbox in Mode::Auto (fail-closed: refuses exec while Docker is unavailable, re-probes on each launch)`.
- **evidence:** lib.rs:284 doc comment: `/// - anything else (e.g. \"auto\") → DockerSandbox in Mode::Auto (degrades to host).` — but strategy.rs:218-227 (live) refuses: `// auto: fail closed. The old degrade-to-host arm is gone` and returns SandboxError::Unavailable.
- **verifier:** The doc comment still claims auto mode "degrades to host", but strategy.rs:212-227 fail-closes (returns SandboxError::Unavailable, with tests asserting auto+degraded refuses launch); the 2026-07-01 fail-closed plan updated the event.rs doc but missed this line, and no accepted-residual covers it. Behavior is correct, so this is documentation polish only — low per the rubric.

**Finding 3.3** — `low`

- **file:line:** `agent/crates/agent-cli/src/main.rs:204-205 (with agent-runtime-config/src/lib.rs:289-293)`
- **violated principle:** Config affecting an isolation boundary should be validated at the edge on every frontend — the CLI never calls RuntimeConfig::validate(), and build_sandbox's `_ => Mode::Auto` catch-all silently maps a typo'd --sandbox-mode (e.g. 'enfore') to auto instead of erroring, while the server rejects it (runtime.rs:143 cfg.validate()).
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Either call `rt.validate()` in agent-cli main and exit 2 on error (parity with backend_name_is_valid handling), or constrain the flag with clap `value_parser(["off", "auto", "enforce"])`.
- **evidence:** lib.rs build_sandbox: `let mode = match cfg.sandbox_mode.as_str() { "off" => return Arc::new(HostExecutor), "enforce" => Mode::Enforce, _ => Mode::Auto };` — main.rs builds rt via runtime_config_from_cli and goes straight to `build_sandbox(&rt)` with no validate() call; runtime_config.rs:385 has the check but only the server path reaches it. Direction-safe (auto is fail-closed) but a user asking for enforce silently gets re-probing auto semantics.
- **verifier:** Re-derived from live source: agent-cli main.rs builds RuntimeConfig and calls build_sandbox with no validate() call and no clap value_parser on --sandbox-mode (main.rs:128-129), while build_sandbox's `_ => Mode::Auto` catch-all (lib.rs:289-293) silently maps a typo'd mode to Auto; the server path does reject it via cfg.validate() (runtime.rs:143 → runtime_config.rs:385-390). No accepted residual covers this — prior sandbox audit work addressed defaults/degradation, not CLI edge validation. Low: direction is fail-safe (Auto still attempts Docker) but a misspelled 'enforce' silently drops the fail-if-Docker-absent guarantee; fix is concrete and small.

**Finding 3.4** — `low`

- **file:line:** `agent/crates/agent-cli/src/main.rs:128-150 (vs agent-runtime-config/src/runtime_config.rs:218-244)`
- **violated principle:** One source of truth per default — every sandbox knob default is hand-duplicated between clap attributes and runtime_config default_* fns (the documented clap-shadowing gotcha class); they match today but nothing asserts parity, so bumping a server-side default silently leaves the CLI behind.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Add a parity test asserting Cli::parse_from(["agent-cli"]) sandbox fields equal RuntimeConfig::from_launch defaults (or derive the clap default_value strings from the shared default_* fns, as already done for DEFAULT_SANDBOX_IMAGE).
- **evidence:** main.rs: `#[arg(long, default_value = "2g")] sandbox_memory` / `default_value = "1g"` tmp_size etc. mirror runtime_config.rs `fn default_sandbox_memory() -> String { "2g".into() }` / `default_sandbox_tmp_size() … "1g"`; the existing test `sandbox_defaults` (main.rs:332) re-hardcodes the literals rather than comparing against the runtime-config fns. Only sandbox_image is derived from the shared constant.
- **verifier:** Re-derived: clap defaults ("auto","2g","2",512,"1g") are hand-duplicated against private default_sandbox_* fns, runtime_config_from_cli (main.rs:20-32) copies clap defaults into RuntimeConfig so drift would silently shadow server-side defaults, the sandbox_defaults test (main.rs:332) re-hardcodes literals instead of asserting parity, and no spec/audit accepted-residual covers it (only sandbox_image derives from the shared DEFAULT_SANDBOX_IMAGE constant). Low is correct: defaults match today, so this is latent maintenance hygiene, not a live correctness defect.

**Finding 3.5** — `low`

- **file:line:** `agent/crates/agent-cli/src/main.rs:205 (with agent-runtime-config/src/assemble.rs:116)`
- **violated principle:** One isolation boundary should have one authoritative state — the CLI constructs two independent DockerSandbox instances (one for MCP connect, a second inside loop_config_from), each paying its own startup probe (up to 2s + image probe) and holding a separate availability cache, so the degraded posture surfaced to the user (SandboxDegraded / approval summary) reflects only the loop's instance.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Build the sandbox once in main and pass the Arc into assemble_loop via LoopParts (have loop_config_from accept an Arc<dyn SandboxStrategy> instead of calling build_sandbox itself).
- **evidence:** main.rs:205 `let sandbox = build_sandbox(&rt);` (used only for connect_mcp) then main.rs:238 assemble_loop → assemble.rs:203 loop_config_from → assemble.rs:116 `sandbox: build_sandbox(cfg)` — a second DockerSandbox with its own RwLock<Availability> cache and its own DockerSandbox::probe() at construction. Both fail closed, so this is cost/observability drift, not a safety hole.
- **verifier:** Re-derived from live source: main.rs:205 builds a DockerSandbox used only for connect_mcp, then assemble_loop→loop_config_from (assemble.rs:116) builds a second one — each paying DockerSandbox::probe() (2s timeout) plus an image probe and holding its own RwLock&lt;Availability&gt; cache, with LoopParts offering no way to pass the first instance in. No fix or accepted-residual note found (follow-ups.md covers other build_sandbox items only); both instances fail closed, so it is cost/observability drift, not a safety hole — low per the rubric.

### 4. Orchestration & sub-agents — findings

**Prior state:** All July fixes touching orchestration were re-verified in live source this session and are intact: ErrorClass-classified retry including the claude-cli `Process` overflow arm (agent-model/src/types.rs:222-243), jittered exponential backoff + integer-seconds Retry-After (loop_.rs:61-72, 382-393), once-per-turn overflow compact-and-rebuild with StreamRetry retraction, OverflowRecovery event and Usage re-emit (loop_.rs:511-545), fatal/second-overflow terminal paths all emitting Done (loop_.rs:546-556), approval-wait racing cancellation with gate-entry short-circuit (loop_.rs:1063-1073, 1133-1139), per-call invalid-tool-call isolation with id normalization and the Length-truncation guard (loop_.rs:608-630, 740-764, 1367-1401), stuck-model nudge/abort with OpenAI-compat message ordering (loop_.rs:32-33, 636-680, 983-989), graceful tools-disabled max_turns wrap-up (loop_.rs:1008-1051), `max_parallel_tools` promoted to RuntimeConfig with 0→DEFAULT_MAX_PARALLEL_TOOLS resolution (assemble.rs:117, loop_.rs:791-795), panic/timeout-isolated parallel dispatch with the 2x backstop (loop_.rs:1333-1359), and the Phase-3 no-silent-drop backstop (loop_.rs:869-887). The finished sub-agent capability (af4dd14/0224383/d19625b) is also intact: exact policy/approval/sandbox Arc inheritance (assemble.rs:239-262, dispatch.rs:450-458), structural depth-1 no-recursion plus in-tool defense (dispatch.rs:393-395), transitive monotonically-narrowing tools allowlist (dispatch.rs:397-424), ModelRef routing for subagent and compaction models with claude-cli→prompted protocol defaulting (assemble.rs:207-262), child-token cancellation (dispatch.rs:466-471), and SubagentSink forwarding/capture with StreamRetry segment retraction (dispatch.rs:110-196). No regressions found; the findings above are composition gaps between individually-reviewed clusters (notably the post-drain description-override seam a0bbf0d never reaching child registries), not regressions of closed fixes.


**Finding 4.1** — `med`

- **file:line:** `agent/crates/agent-core/src/dispatch.rs:385-406` *(auditor cited `agent/crates/agent-core/src/dispatch.rs:385-405`)*
- **violated principle:** Tool descriptions steer agent behavior like prompts and must be applied consistently everywhere the tool is presented — a configured description override that silently applies to the parent loop but not to sub-agent loops splits the tool vocabulary and contaminates any eval that varies descriptions.
- **source:** docs/okf/agent-sdlc/practices/tool-design-as-engineering.md
- **concrete proposed fix:** Thread the override map into dispatch: add `description_overrides: HashMap<String,String>` to `DispatchDeps` (filled from `cfg.tool_description_overrides` in assemble.rs, cloned into nested deps) and call `reg.set_description_overrides(...)` on the child registry built in `DispatchAgentTool::execute`; pin with a test that a child's `schemas()` shows the override.
- **evidence:** dispatch.rs builds the child registry bare: `let mut reg = ToolRegistry::new(); … reg.register(t.clone());` — `set_description_overrides` is never called (overrides live on the registry, not the tools: registry.rs:48-59 applies them in `schemas()`). assemble.rs:265 applies `cfg.tool_description_overrides` only to the parent registry, and the seam spec explicitly claims "a registry-level override applies uniformly to parent and (via its own registry) child loops" (2026-07-02-tool-description-override-seam-design.md) — the child half was never implemented. The seam shipped in `a0bbf0d` AFTER the dispatch snapshot commit `affd765`; the composition was never reviewed together. The eval genome consumes it (eval/config.rs:155), so a description-variant candidate would measure parent-only behavior while children keep base prose — the exact harness-isolation conflation the eval flywheel exists to prevent.
- **verifier:** Re-derived: overrides live on ToolRegistry (registry.rs:53-55, applied in schemas()) and are set only on the parent registry (assemble.rs:265); the child registry built in DispatchAgentTool::execute is bare and set_description_overrides is never called in agent-core, contradicting the seam spec's uniformity claim (seam commit a0bbf0d never touched dispatch.rs, and no doc records this as an accepted residual). Med because nothing live breaks today, but the eval genome (eval/config.rs:155) consuming the knob means any description-variant candidate measures parent-only behavior — an eval-integrity/leverage gap, not a correctness/safety one.

**Finding 4.2** — `med`

- **file:line:** `agent/crates/agent-runtime-config/src/runtime_config.rs:16-29 (and assemble.rs:237-238)`
- **violated principle:** Config limits must travel with the resource they bound: a routed sub-agent model with a different context window inherits the primary's `model_limit`, so child budgeting is computed against the wrong window.
- **source:** first principles + runtime conventions (audit.md thinly-sourced clause: config-based limits judged locally)
- **concrete proposed fix:** Add optional `context_limit` (and `max_tokens`) to `ModelRef`, inherit-on-None like its other fields, and apply it to `child_config.model_limit` in assemble.rs where `max_turns` is already overridden; same for the compaction model's MaintCtx limit if routed.
- **evidence:** `ModelRef` carries only `{backend, base_url, model, claude_binary, protocol}` — no window field. assemble.rs clones the parent config for children changing only turns: `let mut child_config = loop_config.clone(); child_config.max_turns = cfg.subagent_max_turns;` so `model_limit: cfg.context_limit` (the primary's window) governs every child build/maintain site. If `subagent_model` routes to a smaller-window backend, the child systematically over-builds; the safety nets are shrink-only calibration capped at 4x (loop_.rs:243, and the ratio resets to 1.0 per child loop) and once-per-turn overflow recovery — a second overflow in a turn is fatal by design (types.rs class + loop_.rs:546-552). Sub-spec #3's residual list covers tokenizer conflation and $0 cost but not window inheritance.
- **verifier:** Re-derived: ModelRef has no window field and assemble.rs clones the parent LoopConfig for children overriding only max_turns, so a routed child inherits the primary's model_limit; the shrink-only [1.0,4.0] calibration (loop_.rs:230-243) and once-per-turn overflow recovery (second overflow fatal, loop_.rs:546-552) cannot compensate for a genuinely smaller child window, and the advanced-dispatch spec's recorded residuals (lines 249-256) do not cover window inheritance. Med fits: degraded efficiency/reliability of an optional routing feature, not a safety boundary.

**Finding 4.3** — `med`

- **file:line:** `agent/crates/agent-core/src/loop_.rs:38-40,1012-1013`
- **violated principle:** Curate the context to only true, high-signal instructions: the budget wrap-up user message ('tools are disabled for the remainder of this run') is appended to the persistent context and survives into subsequent runs of the same session, where it is a stale, false capability statement the model may imitate.
- **source:** docs/okf/agent-sdlc/practices/context-engineering.md
- **concrete proposed fix:** Keep the wrap-up prompt out of durable history: after the wrap-up completion, replace the appended `BUDGET_WRAP_UP_PROMPT` user message with a neutral marker (or drop it, keeping only the assistant summary), or reword it to be explicitly self-expiring ('until the next user message'); pin with a two-run test asserting the next run's build contains no tools-disabled instruction.
- **evidence:** `ctx.append(Message::user(BUDGET_WRAP_UP_PROMPT));` before the wrap-up completion — the append is unconditional and never removed, even when the completion errors and is swallowed (loop_.rs:1044-1046), so the instruction persists with no reply. This codebase has already measured the failure mode of models imitating visible history patterns: the maintain-cadence comment at loop_.rs:704-711 records memory-roster regressing 10/10→6/10 because 'the model imitates the visible ack-without-tool-call pattern'. The runtime-knobs spec's cross-run analysis covered only dangling tool_call ids ('no dangling tool_call ids in persistent history'), not instruction staleness — an unreviewed seam between cluster A and cross-run context persistence.
- **verifier:** Re-derived: the wrap-up user message ("tools are disabled for the remainder of this run") is appended unconditionally at loop_.rs:1013 into the session-persistent context (CLI reuses one CuratedContext across REPL turns, main.rs:267/295; server reuses self.ctx, session.rs:117), is never removed even when the wrap-up completion errors (1044-1046), has no test or spec coverage for cross-run staleness (runtime-knobs spec Part 2 only covers dangling tool_call ids), and the repo's own comment at loop_.rs:699-711 documents measured model imitation of stale history patterns. Med, not high: self-scoping wording and a narrow trigger path make it an effectiveness/leverage risk rather than a correctness/safety violation.

**Finding 4.4** — `med`

- **file:line:** `agent/crates/agent-core/src/dispatch.rs:468-486`
- **violated principle:** Reduce information loss on agent hand-off — a sub-agent's partial results should reach the coordinator rather than being discarded; a child killed by the wall-clock timeout (or a fatal model error) loses its entire captured transcript even though the capture is sitting in the sink.
- **source:** docs/okf/agent-sdlc/practices/multi-agent-decomposition.md
- **concrete proposed fix:** On the timeout and `Ok(Err(e))` arms, return the tool result built from `sink.summary()` with a loud prefix note ('[sub-agent timed out after Ns / failed: e — partial transcript follows]') instead of a bare `ToolError::Timeout`/`Failed`, mirroring the budget-exhaustion posture; pin with a test that a timed-out child's captured text reaches the parent.
- **evidence:** `Err(_elapsed) => { child_cancel.cancel(); return Err(ToolError::Timeout); }` and `Ok(Err(e)) => return Err(ToolError::Failed{…})` — both return before `let s = sink.summary();`, so up to `subagent_timeout` (default 600s) of captured child work is dropped and the parent must retry blind. This is now inconsistent with the July runtime-knobs cluster, which deliberately gave the TURN-budget exhaustion path a real summary hand-off ('a child hitting subagent_max_turns now hands its parent a real summary', pinned by budget_exhausted_child_wrap_up_summary_reaches_parent) — the wall-clock budget path got no equivalent when the two clusters composed. The dispatch-core spec D8 specifies the Err return but never adjudicates discarding the capture.
- **verifier:** Live source confirms both the timeout arm (line 469-472) and Ok(Err) arm (473-478) return bare ToolErrors before sink.summary() at line 488, discarding the partial transcript the SubagentSink demonstrably accumulates (Token events -> segments, summary() concat fallback); the budget-exhaustion path hands a real summary (pinned by budget_exhausted_child_wrap_up_summary_reaches_parent, tests/dispatch_tool.rs:330) while no spec (D8, runtime-knobs) or audit residual adjudicates the timeout-path discard. Med fits the rubric: wasted up-to-600s of child work and blind parent retries is an efficiency/leverage loss, not a correctness/safety break.

**Finding 4.5** — `low`

- **file:line:** `agent/crates/agent-core/src/dispatch.rs:246-253` *(auditor cited `agent/crates/agent-core/src/dispatch.rs:247-253`)*
- **violated principle:** Tool prose must not contradict actual capability: the static description says the child gets your tools 'minus dispatch_agent itself', which is false whenever `subagent_max_depth > 1` (the child then gets a nested dispatch_agent), and it contradicts the `tools` parameter prose in the same schema that explains including dispatch_agent for nesting.
- **source:** docs/okf/agent-sdlc/practices/tool-design-as-engineering.md
- **concrete proposed fix:** Make the description depth-aware at construction (DispatchDeps already carries depth/max_depth — format the description string, or store two variants) so it says 'minus dispatch_agent' only when `depth >= max_depth`.
- **evidence:** description(): "The sub-agent has the same permissions and tools as you (minus dispatch_agent itself)" — but at dispatch.rs:414-424 the child registry registers a nested `DispatchAgentTool` whenever `depth < max_depth && nested_named`, and the `tools` param description (line 277) says "Include dispatch_agent to let the sub-agent dispatch its own (only meaningful when nesting depth allows)". The main description was written for the depth-1 default and was not updated when sub-spec #3 (d19625b) added depth.
- **verifier:** Re-derived from live source: description() statically says "minus dispatch_agent itself", yet with subagent_max_depth > 1 (top-level depth=1, nested_allowed at line 360) the child registry registers a nested DispatchAgentTool at lines 414-424, and the tools-param prose at line 277 contradicts the main description in the same schema; no spec or audit note accepts this residual. Low severity: stale prose under non-default config, no behavioral/safety impact since the depth guard is enforced structurally.

### 5. Guardrails & policy — findings

**Prior state:** All July guardrails fixes were re-verified in live source this session and are intact — no regressions. Hard floor: three-layer design with position-aware Layer A2 boundary scan (agent-policy/src/command.rs:300-334, `command_boundary_programs` + `program_name_is_catastrophic`), structural rm/dd handlers, and the whitespace-stripped substring backstop; `HARD_FLOOR_DENYLIST = ["rm -rf /", ":(){", "dd if="]` (runtime_config.rs:11) mirrored by `default_denylist()` (lib.rs:255-263) and unioned via `effective_denylist()` (runtime_config.rs:396-404). Both /dev-redirect layers live (`redirect_catastrophe_in_argv` command.rs:212, `raw_redirect_catastrophe` command.rs:237) over the shared lexical `resolved_dev_suffix` resolver (command.rs:149-171), with the dd `of=` handler sharing it (command.rs:95-104). Destroy tier enforced at both guard sites (engine.rs:52 command-branch `access != Access::Destroy && is_auto_allowed`, engine.rs:77 non-command `Destroy => Ask`). Prefix allowlist with read-safe git/cargo entries (lib.rs:215-254) and the git log/diff/show `--output`/`-o` arg-scan (command.rs:440-450). Post-exec validator (decision-round cluster D) is live and conforms to its spec: once-per-turn, after Phase-3 tool-result append and before the stuck-nudge (loop_.rs:922-978), sink-free `run_validator` via `sh -c` through the loop's sandbox with cancel/timeout select and 4 KiB char-boundary cap, best-effort Skipped semantics (loop_.rs:1238-1314); dispatch children inherit it via the cloned LoopConfig. Approval channels: TerminalApproval 300 s timeout + serialization gate for parallel sub-agent prompts (agent-cli/src/approval.rs:9,17,75) with the hermetic timeout test; IpcApprovalChannel denies on timeout/absent slot (agent-server/src/approval.rs:51-64); the approval-vs-cancel race fix (`tokio::select!` in gate_tool's Ask arm, loop_.rs:1133-1139) and gate-entry cancel short-circuit are in place, and the deterministic policy→approval→execute order holds on every path including dispatch children (dispatch.rs shares the parent policy/approval Arcs into the same AgentLoop gate). The policy corpus (policy_corpus.{rs,tsv}) runs the real RulePolicy + ExecuteCommand::intent + default lists and includes the newest closed classes (/dev normalization rows, git --output rows). The carried `memory` PartialRuntimeConfig mirror fix is intact (runtime_config.rs:452-454) alongside new mirrored fields (post_tool_validators, tool_description_overrides). The four findings above are new interactions/gaps (validator trigger tier-fidelity in both directions, an unenforced allowlist-interpreter invariant, and unpinned accepted-residual corpus rows), not regressions of closed work.


**Finding 5.1** — `med`

- **file:line:** `agent/crates/agent-mcp/src/tool.rs:69-75 + agent/crates/agent-core/src/loop_.rs:825-834` *(auditor cited `agent/crates/agent-mcp/src/tool.rs:69-74 + agent/crates/agent-core/src/loop_.rs:828-834`)*
- **violated principle:** Guardrails should be layered defense-in-depth; encoding MCP Trust::Allow as Access::Read makes a mutating trusted MCP tool both auto-allowed AND invisible to the post-exec validator — the one post-hoc guardrail added since that encoding was accepted.
- **source:** docs/okf/agent-sdlc/practices/human-in-the-loop-gates.md
- **concrete proposed fix:** Decouple the auto-allow decision from the declared tier for MCP tools (e.g. an intent-level `mutating: true` for Trust::Allow tools, or a tier that RulePolicy auto-allows but the validator trigger counts as Write) so trusted MCP mutations still trigger post-execution validation; the Trust→Access 'zero policy change' acceptance predates the validator and its rationale no longer prices in this exemption.
- **evidence:** tool.rs: `// Trust is encoded onto the policy's Read/Write axis (zero policy change): … Trust::Allow => Access::Read, Trust::Ask => Access::Write` — an MCP tool that writes files/state under Trust::Allow yields Access::Read, so `turn_mutated` stays false and configured validators never run for its edits.
- **verifier:** Re-derived end-to-end: McpTool maps Trust::Allow to Access::Read (tool.rs:72-75), intent.access is threaded via ReadyCall (loop_.rs:1099) into turn_mutated (loop_.rs:828-834) which counts only Write/Destroy, so trusted MCP mutations never trigger configured post-exec validators. The 'accepted LOW' residual in the 2026-07-01 destroy-tier spec predates the 2026-07-02 validator design, which re-accepts only the execute_command over-trigger and is silent on this under-trigger — the acceptance rationale was approval-posture-only and was never re-priced against the new consumer of Access. Med, not high: the validator is opt-in and advisory (never a gate), so this is a defense-in-depth/leverage gap rather than a direct safety breach.

**Finding 5.2** — `low`

- **file:line:** `agent/crates/agent-runtime-config/src/runtime_config.rs:327-392 + agent/crates/agent-policy/src/command.rs:391-394`
- **violated principle:** A hard floor that 'denies even if a user would approve it' should not be silently disarmable by one config edit whose danger is documented only in a source comment; invariants deserve enforcement, not prose.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** In `RuntimeConfig::validate()` (or at settings_update/CLI load), warn — or reject with an explicit opt-out — when `command_allowlist` gains a shell interpreter or exec-vehicle as a leading token (bash/sh/zsh/dash/ksh/eval/xargs/env), enforcing the command.rs KNOWN LIMITATION guidance mechanically.
- **evidence:** command.rs: 'Do not add shell interpreters or exec-capable arg runners (bash/sh/zsh/dash/eval/xargs) to command_allowlist.' — but validate() checks backend/protocol/sampling/sandbox_mode only; a user (or the wire-editable settings surface) adding "bash" silently auto-allows `bash -c "sudo …"`, hollowing the hard floor with no warning anywhere.
- **verifier:** validate() checks backend/protocol/sampling/sandbox_mode only and is the sole gate on the wire-editable apply() path, so adding "bash" to command_allowlist silently auto-allows `bash -c "sudo …"` (one-word prefix match + quoted-token blind spot verified in command.rs). The 2026-07-01 hard-floor spec accepted the command-time scanner limitation with prose guidance only; mechanical validate-time enforcement was never considered or declined, so the gap stands. Low: default config safe, residual documented, requires deliberate config edit — hardening, not an active defect.

**Finding 5.3** — `low`

- **file:line:** `agent/crates/agent-runtime-config/tests/policy_corpus.tsv:39-52`
- **violated principle:** The adversarial corpus should pin every documented bypass class — including accepted Ask-not-Deny residuals — so a future allowlist or classifier change cannot silently degrade them to Allow.
- **source:** docs/okf/agent-sdlc/practices/eval-driven-development.md
- **concrete proposed fix:** Add one-line `ask` rows for the /dev-redirect spec's documented out-of-scope classes: variable expansion (`echo x > $DEV/sda`, `dd of=$DEV`), non-redirect write vehicles (`tee /dev/sda`, `cp x /dev/sda`), and cwd-relative (`cd /dev && echo x > sda`), so their fail-safe Ask posture is regression-pinned through the real engine path.
- **evidence:** The corpus's /dev class covers deny rows (redirects, normalization, dd) and safe-sink asks (`cmd 2>/dev/null`, `/dev/shm/f`), but has no row for `tee /dev/sda`, `$DEV` expansion, or the cwd-relative form — the exact residuals the 2026-07-02 dev-redirect spec documents as 'reach Ask not Deny'; a user later allowlisting `tee` (or a SHELL_SIGNIFICANT edit dropping `$`) would flip them to Allow with no test failing.
- **verifier:** Verified live: the corpus's /dev class has no rows for tee/cp write vehicles, $DEV variable expansion, or the cwd-relative redirect, and no other test in the agent/ workspace pins them, despite the 2026-07-02 dev-redirect spec explicitly recording them as Ask-not-Deny residuals; the spec's out-of-scope note documents the behavior but does not pin it, so an allowlist/classifier change could silently flip these to Allow. No current defect — pure regression-net polish — so severity stays low.

### 6. Observability — findings

**Prior state:** All 2026-07-01/02 observability fixes were re-verified in live source this session and remain in place, with no regressions: every resolved tool call emits a terminal ToolResult with status+duration (agent/crates/agent-core/src/event.rs:88-96; emit sites loop_.rs:890/905/960); the JSONL TraceWriter is default-on, 0600 at creation (agent-runtime-config/src/trace.rs:43-49 + test :513-522), 64MB cap (:106) and keep-50 retention (:37,:145-161); record_child interleaves child transcripts into the same file under one seq counter with sub ordinal + parent_id (:84-127, test :452-469); SessionStats has the subagent subset counters and parent-only turns=max semantics (agent-core/src/stats.rs:59-71); the attribution chain (ToolCtx.call_id -> parent_id) is intact across event/wire/trace (event.rs:80-95, agent-server/src/wire.rs:43-61, trace.rs:201-218) and chains transitively at depth 2 via id_prefix (agent-core/src/dispatch.rs:414-424,443); the web reducer is still id-first (web/src/state.ts:171 parentId); claude-cli parses total_cost_usd and folds cache_read/cache_creation into prompt_tokens (agent-model/src/claude_cli.rs:185-205); ContextEvents reach wire and trace including Evicted dedup now keyed on (messages, est_tokens) (agent-core/src/curated.rs:44,591-597 — the small-residuals-sweep fix is live), OverflowRecovery and StreamRetry; ObservedSink (stats fold + trace + forward) is wired for all three frontends via assemble_loop (assemble.rs:196-200) with session-stable handles on the server (agent-server/src/runtime.rs:43-45). A failed CHILD turn — including depth 2 — IS replayable from the trace alone (forwarded tool events + tapped Token/Error/Done, and the child's input prompt is captured in the dispatch ToolStart args). Accepted residuals confirmed still standing and not re-reported: mixed-backend token/cost conflation ($0 for non-claude-cli children), tool_time_ms double-counting child tool time inside the dispatch duration, child approval wire prompts unattributed, pre-subscribe event drop on the server (query-on-attach design), zero-valued subagent stats fields always serialized.


**Finding 6.1** — `med`

- **file:line:** `agent/crates/agent-core/src/loop_.rs:460 (also agent-runtime-config/src/trace.rs:176-242, agent-server/src/session.rs:116-117, agent-cli/src/main.rs:295)` *(auditor cited `agent/crates/agent-core/src/loop_.rs:460 (also agent-runtime-config/src/trace.rs:180-242, agent-server/src/session.rs:117, agent-cli/src/main.rs:295)`)*
- **violated principle:** A trace must log enough context to replay and diagnose a failed turn without re-running the session — and an eval dataset harvested from traces needs the user prompt alongside the trajectory.
- **source:** docs/okf/agent-sdlc/practices/trajectory-evaluation.md (a dataset should contain the user prompt, reference/generated trajectory, and response); docs/okf/agent-sdlc/phases/monitoring-and-operations.md (production traces enable post-hoc evaluation)
- **concrete proposed fix:** Record run inputs in the trace: emit a run-start event carrying the user input (mapped to None in server_event_from for old-SPA wire compat, recorded by ObservedSink/ChildTraceTap), and record the composed system prompt (or a hash + override/skill set) once per run — the server recomposes it per turn at session.rs:116 so it is otherwise unrecoverable.
- **evidence:** loop_.rs:460 `ctx.append(Message::user(user_input));` — the input goes straight to context, never through the sink; TraceEvent (trace.rs:180-242) has no input/system-prompt record type; both frontends pass text directly to run_with_cancel with no trace record. A failed TOP-LEVEL turn (e.g. turn 1 of any session) cannot be replayed or diagnosed from the trace alone: tokens, tool calls and errors are all persisted but what the user asked is not. Child turns don't have this gap (the dispatch prompt/role are in the traced ToolStart args), making the parent the one unreplayable actor. Not listed in any spec accepted-residual or re-stamp follow-up.
- **verifier:** Re-derived from live source: loop_.rs:460 sends user_input only to the context manager, AgentEvent/TraceEvent have no input or system-prompt record type, and both frontends pass text directly to run_with_cancel (server recomposes the system prompt per turn at session.rs:116, making it unrecoverable). No fix, test, or accepted residual exists — the observability re-stamp follow-ups omit it, and the sub-agent trace work closed only the child-replay gap, so a failed top-level turn indeed cannot be replayed or harvested into an eval dataset from the trace alone; that is an eval/diagnosis leverage gap, not a correctness/safety defect, hence med.

**Finding 6.2** — `low`

- **file:line:** `agent/crates/agent-runtime-config/src/trace.rs:106-123`
- **violated principle:** Silent observability degradation is itself a drift risk — a truncated trace should be distinguishable from a crashed session when the file is read post-hoc.
- **source:** first principles + runtime conventions (SKILL.md Spine A component 6: 'without it, no way to tell if the agent is drifting'; the runtime's own SandboxDegraded convention surfaces degradation in-band)
- **concrete proposed fix:** Before setting `inner.w = None` on cap breach or write failure, append one short terminal marker record (e.g. type=trace_disabled with reason cap|io_error), reserving headroom in the cap check so the marker always fits.
- **evidence:** trace.rs:106-114: on cap breach it warns to the tracing log, flushes, and sets `inner.w = None` — the JSONL file simply stops with no in-file marker; same at :120-123 on write failure. A replayer cannot distinguish 'trace capped/disabled mid-run' from 'process died mid-turn', which corrupts exactly the failed-turn forensics the trace exists for.
- **verifier:** Live source matches the citation: cap breach (106-114) and write failure (120-123) both set inner.w = None with only an out-of-band tracing::warn!, leaving the JSONL indistinguishable from a mid-turn crash — and file-size heuristics fail since one oversized line can trip the cap well below max_bytes. The observability spec designed warn-and-disable but never adjudicated an in-file marker, so this is an uncovered gap, not an accepted residual; the fix is one concrete action.

### 7. Context engineering (Spine B) — findings

**Prior state:** All previously-closed fixes touching Spine B were re-verified in live source this session and are intact, with no regression: turn-atomic curation (turn_unit_ranges/evict_start/snap_split_to_unit_boundary/orphaned_tool_positions at context.rs:37-151, debug_assert orphan guards in both build()s, budget-sweep tests) — note eviction has since evolved from the contiguous evict_start window to priority-ladder plan_retention (context.rs:88-115) plus the extractive user-fold ledger (curated.rs:349-450, context-evolve champion v4), still whole-unit and orphan-guarded; the 16 KiB ingestion cap (eager select_oversized step (0) of maintain at curated.rs:236-242, idempotent capped_preview, recall paging bounded to the same budget at context_tools.rs:94-116) with RuntimeConfig.max_tool_result_bytes wired into CLI (main.rs:273-275), server incl. workspace-switch rebuild (session.rs:75-78, 286-289), and dispatch children (dispatch.rs:428-438); calibrated budgeting (calib_ratio_micros EMA α=0.5 clamped [1,4] shrink-only at loop_.rs:156-254, effective_model_limit applied at the turn build :489, overflow MaintCtx+rebuild :527/:533, text-only-exit maintain :714, loop-bottom maintain :992, and the wrap-up build :1014, with the calibration sample folded at :571 against the FINAL request); unconditional set_recall (loop_.rs:453-457); Evicted dedup keyed on (messages, est_tokens) (curated.rs:44, 590); the snapshot memory segment sized from the capped recall block (snapshot.rs:61-73); the compose-time quarter-window warn (assemble.rs:177-187); and the Examples context type (registry.rs examples subset :105-123, L1 [N examples] marker + L2 Examples section with the 50-entry listing cap at tools.rs:62-165). Documented accepted residuals still hold on their original rationale and are not re-reported: the RAM-only, unbounded, conversation-stable offload store (persisted OffloadStore was DECLINED-BY-OWNER 2026-07-02; the eager path adds volume but per-session/bounded-by-transcript, no fresh evidence the acceptance fails), Evicted observable only after tool turns, settings-change cap drift converging on workspace switch, second-overflow-fatal by design, and the eval harness pinning the cap off. The three findings above are new gaps (one from the post-fix ledger evolution, two never-audited seams), not regressions of closed work.


**Finding 7.1** — `med`

- **file:line:** `agent/crates/agent-core/src/curated.rs:206-210 (with agent/crates/agent-core/src/loop_.rs:459)`
- **violated principle:** Every pinned context block must be budgeted — an uncapped always-loaded block violates 'smallest set of high-signal tokens' and can permanently starve the retention window.
- **source:** docs/okf/agent-sdlc/practices/context-engineering.md
- **concrete proposed fix:** Cap the goal block at a token budget (mirror DEFAULT_RECALL_TOKEN_BUDGET: keep the first N estimated tokens of user_input plus an ellipsis marker) in CuratedContext::set_goal; the full input remains in history where fold/eviction/offload can manage it.
- **evidence:** fn set_goal(&mut self, goal: String) { if self.goal.is_none() { self.goal = Some(Message::system(format!("Original goal: {goal}"))); } } — loop_.rs:459 calls ctx.set_goal(user_input.clone()) with the FULL first user input. Every other pinned block is capped (recall 512 tok, folded-facts ledger FOLDED_FACTS_MAX_TOKENS=512, system prompt has the compose-time quarter warn) but the goal pin is unbounded and set-once for the whole session. A large first paste (e.g. a log dump comparable to the window) makes pinned_tokens() exceed effective_model_limit forever: build() budget saturates to 0, compaction/offload/fold only touch history so overflow recovery cannot shrink it, and the second overflow in a turn is fatal by design — every subsequent run on the persistent server/REPL context then fails until a workspace switch resets the context.
- **verifier:** Re-derived from live source: set_goal pins the full first user_input uncapped and set-once while every other pinned block is budgeted (recall budget, FOLDED_FACTS_MAX_TOKENS=512); the ingestion cap only covers Role::Tool messages, all shrink paths (fold/offload/compaction) touch history only, build() saturates the budget to 0 yet plan_retention keeps a newest-unit floor, and second-overflow-in-a-turn is terminal — so an over-window first paste permanently wedges the persistent context with no recovery path, and no fix/test/accepted-residual exists in specs or the audit report.

**Finding 7.2** — `med`

- **file:line:** `agent/crates/agent-runtime-config/src/runtime_config.rs:327-392`
- **violated principle:** Config knobs that can brick the loop need validation guards, matching the crate's own convention of rejecting degenerate values for sibling knobs.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Add a max_tool_result_bytes floor to RuntimeConfig::validate() (reject 0, or clamp to e.g. 1024 — mirroring the max_parallel_tools zero-guard shipped in the runtime-knobs cluster), and optionally warn when the cap's estimated tokens exceed a fraction of context_limit.
- **evidence:** validate() checks max_tokens == 0, max_turns == 0, max_parallel_tools == 0, context_limit < 1024 — but never max_tool_result_bytes. The field is serde-defaulted and user-overridable (test at :692 pins that a partial file {"max_tool_result_bytes": 4096} wins). A user writing 0 to 'disable the cap' (a common 0-means-unlimited convention) gets the opposite: select_oversized selects every non-empty tool result (offload_policy.rs:158 `m.content.len() <= config.max_result_bytes`), capped_preview degrades every result to a marker-only stub (offload_policy.rs:139), and ContextRecallTool with page_bytes=0 pages at ~1 char per call (context_tools.rs:102 `saturating_sub(worst.len()).max(1)`) — the model can never see any tool output. Conversely a cap far above context_limit silently re-opens the single-oversized-result unrecoverable-overflow path the ingestion cap was built to close.
- **verifier:** validate() guards max_tokens/max_turns/max_parallel_tools/context_limit but never max_tool_result_bytes; a user-set 0 (verified live: offload_policy.rs:158 selects every non-empty result, :139 degrades all to marker-only stubs, context_tools.rs:102+107 pages recall at ~1 char/call) silently starves the model of all tool output. No guard, clamp, or accepted-residual note exists in specs/ or the audit; the runtime-knobs spec confirms the sibling zero-guard convention. Med: needs explicit misconfiguration, so missing hardening rather than a live correctness defect; fix is one concrete validate() check.

**Finding 7.3** — `low`

- **file:line:** `agent/crates/agent-core/src/snapshot.rs:32-101 (with agent/crates/agent-core/src/curated.rs:178-189)`
- **violated principle:** The context snapshot must report what the context actually injects (the exact principle behind the S13 memory-segment fix) — a pinned block invisible to the explorer is unfaithful observability.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Pass folded_facts (or the merged goal+ledger block) into build_snapshot and emit a 'ledger' segment (or fold its tokens into the goal segment to match pinned()'s merge), so est_total and the per-category breakdown include the up-to-512-token pinned ledger.
- **evidence:** build_snapshot(turn, model_limit, &self.system, self.goal.as_ref(), &self.recall, self.recall_budget, self.compaction_summary.as_ref(), &self.history) — no folded_facts parameter, and snapshot.rs has no ledger segment. The folded-facts ledger (context-evolve champion v4, merged after the S13 snapshot-fidelity fix) rides inside the goal block in pinned() (curated.rs:98-104) and is charged in pinned_tokens(), but the Context Explorer snapshot's goal segment sizes only message_tokens(g) of the bare goal — up to FOLDED_FACTS_MAX_TOKENS (512) tokens of pinned context are missing from est_total and every segment.
- **verifier:** Re-derived from live source: pinned() injects the folded-facts ledger (curated.rs:98-104) and pinned_tokens() charges it (:158-160), but snapshot() passes only the bare goal into build_snapshot, which has no ledger segment and sizes goal as message_tokens(g) alone — so est_total and every segment omit up to FOLDED_FACTS_MAX_TOKENS=512 (curated.rs:289) tokens of injected pinned context. No fix, test, or accepted-residual note exists in follow-ups.md or the audits; severity stays low since it is explorer-display fidelity only — the actual context build and budget math are correct and the miss is capped.

### 8. Desktop/web design-tab harness — findings

**Prior state:** The two-layer localhost guard from the July work is intact. The Rust coarse layer (agent-tools/src/render.rs validate_local_url) still rejects userinfo/@-authority bypasses — the authority is split on `/?#` and any `@` is refused (render.rs:14), and the colon-in-userinfo cases (`http://localhost:5173@evil.com`) are covered by tests at render.rs:314-315. The JS authoritative layer (web/src/components/inspector/urlGuard.ts) still uses the WHATWG URL parser and an exact hostname allowlist, and UrlArtifact/DesignPane both call it before an iframe is created. The single-process dev-server invariant (start() calls stop() first), both-pipes-drained-to-EOF reader loop, ANSI/CSI stripping, and the server-side pm+script whitelist all remain in live source with their regression tests. The Tauri command surface is still funneled through one all_handlers! macro shared by prod and mock builds. No regression of a previously-closed fix was found; the findings above are gaps not previously reported (the design-tab dimension had no section in the 2026-07-01 deep audit, which predates this feature). Note the one deliberately-declined item nearby — live trace toggle — is unrelated to these findings.


**Finding 8.1** — `med`

- **file:line:** `src-tauri/src/devserver.rs:176-199`
- **violated principle:** A Tauri command must not trust an SPA-provided filesystem path without a server-side containment check; capabilities should be explicitly scoped, not ambient.
- **source:** docs/okf/agent-sdlc/practices/harness-engineering.md
- **concrete proposed fix:** In DevServerManager::start, canonicalize cand.dir and require it to be inside the current workspace root (or verify the exact candidate came from a fresh detect()), before spawning; reject otherwise.
- **evidence:** start() validates only pm+script: `if !ALLOWED_PMS.contains(&cand.package_manager…)` / `if !SERVER_SCRIPTS.contains(&cand.script…)` then `Command::new(&cand.package_manager).arg("run").arg(&cand.script).current_dir(&cand.dir)` — cand.dir arrives verbatim from the IPC command dev_server_start(candidate) (lib.rs:79-85) and is never checked against the workspace. The in-code comment claims the whitelist stops "a future XSS … arbitrary-local-binary execution", but `npm run dev` executes the arbitrary body of the `dev` script in any attacker-chosen dir with a planted package.json, so the stated mitigation is incomplete.
- **verifier:** Verified live: start() whitelists only pm+script and spawns with current_dir(&cand.dir) taken verbatim from the IPC command (lib.rs:79-85) with no canonicalization or workspace-containment check, so a compromised webview can execute an arbitrary `dev` script body from any planted package.json — defeating the in-code XSS-mitigation claim. No accepted residual exists; the spec's Security section (2026-07-06-auto-dev-server-canvas-design.md:175-177) actually promises workspace containment that the code does not enforce. Med fits the rubric: an incomplete defense-in-depth layer gated on a prior webview compromise, not a directly reachable break.

**Finding 8.2** — `low` *(auditor said med; verifier's call stands)*

- **file:line:** `src-tauri/src/devserver.rs:194-196`
- **violated principle:** Teardown must reap the spawned process group on every exit path including unclean ones; a long-running child should die with its parent.
- **source:** docs/okf/agent-sdlc/practices/harness-engineering.md
- **concrete proposed fix:** On Unix, set PR_SET_PDEATHSIG (via a pre_exec or the process_group + pdeathsig combo) so the dev server dies if the app is SIGKILLed/aborts, since kill_on_drop and the Destroyed/Drop handlers cover only graceful exits.
- **evidence:** `cmd.process_group(0); // child becomes its own group leader` detaches the child from the parent's group, so a parent SIGKILL/panic-abort no longer cascades to it; the only reapers are `.kill_on_drop(true)` and the WindowEvent::Destroyed/Drop calls to stop() (lib.rs:247-251, devserver.rs:260-262), none of which run on SIGKILL or a non-unwinding abort — leaving an orphaned server holding its port. Tab-close/restart/graceful-quit are covered; hard-kill is not.
- **verifier:** Gap is real: no PR_SET_PDEATHSIG/pre_exec exists, and every reaper (kill_on_drop, Drop for DevServerManager, WindowEvent::Destroyed -> stop()) is destructor/handler-based, so a SIGKILLed or aborted app orphans the dev-server process group; the spec's "orphan prevention" claim covers only graceful paths and no accepted-residual note exists. Downgraded to low: only unclean-exit paths of a local dev convenience are affected (panic-unwind and all graceful paths are covered), the orphan is user-visible and manually recoverable, and the auditor's causal claim that process_group(0) broke SIGKILL cascade is imprecise (parent SIGKILL never cascades to children regardless of group).

**Finding 8.3** — `med`

- **file:line:** `src-tauri/tauri.conf.json:17`
- **violated principle:** An iframe source must be permitted by CSP frame-src; relying on webview leniency for an unset directive is fragile.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Add an explicit `frame-src 'self' http://localhost:* http://127.0.0.1:*` to the desktop CSP so the live-preview iframe (UrlArtifact src=http://localhost:5173) is allowed deterministically; verify the auto-dev-server canvas renders under the production (non-dev) CSP.
- **evidence:** CSP = `default-src 'self'; connect-src 'self' ws://127.0.0.1:* ws://localhost:* http://localhost:*; img-src 'self' data:; …` — no frame-src/child-src, so frame-src falls back to default-src 'self' (=tauri://localhost). UrlArtifact.tsx:24 renders `<iframe src={url} …>` with a `http://localhost:*` src, which frame-src 'self' would block; the auto-dev-server preview is desktop-only (isTauri-gated in DesignPane.tsx:33), i.e. exactly the surface this CSP governs.
- **verifier:** CSP at the cited line has no frame-src/child-src, so frames fall back to default-src 'self' (the tauri app origin in bundled builds), which blocks the UrlArtifact iframe (UrlArtifact.tsx:24, desktop-gated via DesignPane.tsx:33) at http://localhost:*; connect-src does not cover frames, and no fix or documented accepted-residual exists anywhere in the repo (frame-src appears in zero files). Dev works only because the production CSP isn't applied to the Vite-served dev webview — exactly the leniency the finding flags — so the fix (explicit frame-src + verify production build) is one concrete, needed action.

**Finding 8.4** — `low`

- **file:line:** `src-tauri/src/devserver.rs:125`
- **violated principle:** A local-only URL check must anchor the host, not prefix-match it; sole reliance on a downstream layer removes the intended defense-in-depth.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Match the parsed host up to its port/path boundary against exact `localhost`/`127.0.0.1`, or run the dev-server URL through the same Rust validate_local_url used by the render tool, restoring two-layer parity for this path.
- **evidence:** `if host.starts_with("localhost") || host.starts_with("127.0.0.1")` where `host = &url[scheme.len()..]` includes the rest of the URL, so `http://localhost.evil.com:5173/` is accepted and stored via addUrlVersion (DesignPane.tsx:42). Unlike the render tool (render.rs validate_local_url + JS isLocalUrl = two layers), the dev-server URL path has only the JS isLocalUrl guard at render time; the Rust side here mis-accepts.
- **verifier:** Re-derived: `host.starts_with("localhost")` accepts `http://localhost.evil.com:5173/` (and `localhost@evil.com`), contradicting parse_url's own doc comment and the spec's advertised two-layer guard; the URL is stored unchecked via addUrlVersion (DesignPane.tsx:41). No accepted-residual note found. Severity stays low because the authoritative WHATWG isLocalUrl guard at the sole iframe choke-point (UrlArtifact) still blocks rendering, and the input comes from a user-launched whitelisted workspace script — impact is a bogus persisted URL, not an iframe load.

**Finding 8.5** — `low`

- **file:line:** `web/src/components/inspector/ArtifactRenderer.tsx:57-60`
- **violated principle:** Agent-controlled artifact fields should not become an arbitrary outbound-fetch channel from the canvas.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Restrict Image src to data: URIs (or validate the host), since the agent fully controls `content`; in the browser SPA path there is no CSP to fall back on.
- **evidence:** `const src = data.startsWith("http") || data.startsWith("data:") ? data : …` renders `<img src={src}>` with an agent-supplied http(s) URL. Desktop CSP img-src is `'self' data:` (blocks it), but web/index.html ships no CSP meta, so on the browser-via-Worker path an agent can make the browser fetch an arbitrary external URL (tracking-pixel / exfil beacon).
- **verifier:** Re-derived: lines 57-60 render an agent-supplied http(s) URL straight into <img src>; desktop CSP (tauri.conf.json img-src 'self' data:) blocks it but web/index.html ships no CSP, so the browser-via-Worker path is an ungated outbound-fetch/beacon channel. The 2026-06-23 spec's "trusted local daemon" acceptance covers transport trust, not model-authored URLs — and the newer UrlArtifact localhost guard shows agent URLs here are otherwise validated, so this is a genuine uncovered gap; impact is narrow (URL-sized leak, injected-agent precondition), hence low.

**Finding 8.6** — `low`

- **file:line:** `src-tauri/src/devserver.rs:169-171`
- **violated principle:** A SIGTERM-then-SIGKILL teardown should give the child a grace window, else the graceful signal is meaningless.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Send SIGTERM, wait a short bounded interval (e.g. via try_wait/timeout), then SIGKILL only if still alive, so dev servers can flush and remove pidfiles.
- **evidence:** `libc::kill(-(r.pid as i32), libc::SIGTERM); libc::kill(-(r.pid as i32), libc::SIGKILL);` — the SIGKILL is issued on the very next line with no delay, so the process is force-killed before it can act on SIGTERM.
- **verifier:** Verified live: SIGKILL is issued on the statement immediately after SIGTERM with no wait, so the graceful signal is meaningless and the "SIGKILL as a backstop" doc comment (line 164) misstates the behavior; no spec/audit note adjudicates this as an accepted residual. Low is right: the design spec already accepts force-kill on teardown and dev servers tolerate hard kills, so the residual harm (unflushed pidfiles/state) is polish, not correctness.

### 9. Skills & knowledge layer — findings

**Prior state:** The 2026-07-01 deep-audit fixes touching the skills/knowledge dimension are all still live: (1) the CLAUDE.md "Two skill trees" gotcha distinguishing Claude-facing `.agents/skills/` from the runtime's `<workspace>/.agent/skills` + `~/.agent/skills` roots is present and matches live `agent/crates/agent-skills/src/registry.rs:23-41`; (2) the "Do not use for…" deflection blocks the audit asked for now exist in every skill I read (harness-engineering:27-29, context-management:19-21, context-evolve:23-25, wayland ~line 33, graphify-best-practices:22-25, plus explicit sibling deflections in harness-evolve→context-evolve and auto-drive-tauri→tauri); (3) the Examples-type capability survives in `create_skill`'s description (agent-skills/src/tools.rs:210, examples/ surfaced as worked exemplars). The 2026-07-06 OKF bundle is in place and internally consistent: 36 sources / 23 concepts match the advertised counts, all 36 Source nodes carry `resource:` URLs, every intra-bundle link I swept is bundle-root absolute per the documented convention, and the MEMORY follow-up "graphify ingest" is done (graphify-out/graph.json rebuilt 2026-07-06 22:30 contains the okf/agent-sdlc corpus). No regressions found; the findings below are new staleness/guard-gap items, not reopened prior findings.


**Finding 9.1** — `med`

- **file:line:** `.agents/skills/auto-drive-tauri/SKILL.md:56-58`
- **violated principle:** A loaded skill must not point agents at files that no longer exist — stale rule-file content is wrong-signal context that sends the agent chasing a nonexistent pattern.
- **source:** docs/okf/agent-sdlc/practices/context-engineering.md
- **concrete proposed fix:** Rewrite the L0/L1-hybrid paragraph to reference a live test (src-tauri/tests/smoke_context_explorer.rs or llama_health.rs) or restore an offline bridge-wiring test; the referenced file and test were deleted in commit 474b7af and grep finds no relocated equivalent in src-tauri/ or agent-server/.
- **evidence:** `src-tauri/tests/bridge.rs` (`bridge_serves_local_runtime`) is an L0/L1 hybrid: it exercises the full bridge→serve wiring with a **closed** model port… Copy its pattern for new protocol tests. — but src-tauri/tests/ contains only e2e_harness, gui_smoke.rs, llama_health.rs, smoke_context_explorer.rs; bridge.rs was deleted in 474b7af
- **verifier:** Live SKILL.md lines 56-58 still direct agents to copy the pattern of src-tauri/tests/bridge.rs (bridge_serves_local_runtime), but that file was deleted in commit 474b7af and grep finds no relocated equivalent — src-tauri/tests/ holds only e2e_harness, gui_smoke.rs, llama_health.rs, smoke_context_explorer.rs. Stale skill context misdirects test authoring (leverage/efficiency), so med holds.

**Finding 9.2** — `med`

- **file:line:** `scripts/ci.sh:9-19`
- **violated principle:** When a machine-checkable pass/fail signal exists, it should gate changes automatically rather than rely on the author remembering to run it.
- **source:** docs/okf/agent-sdlc/practices/verification-first-agent-coding.md
- **concrete proposed fix:** Add `python3 scripts/okf_check.py docs/okf/agent-sdlc` (and `python3 scripts/test_okf_check.py`) as a step in scripts/ci.sh so the pre-push hook and CI enforce bundle conformance instead of authoring.md's manual step 4.
- **evidence:** ci.sh runs only cargo fmt/clippy/test + web typecheck/vitest — no okf_check invocation; authoring.md:83-90 says 'Validate: python3 scripts/okf_check.py docs/okf/agent-sdlc … Require OK' as a manual workflow step
- **verifier:** Re-derived: scripts/ci.sh:9-19 runs only cargo fmt/clippy/test and web typecheck/vitest — no okf_check invocation anywhere in ci.sh, .githooks, or .github/workflows; the checker exists, is stdlib-only, currently exits 0 ("OK"), and .agents/skills/agent-sdlc/authoring.md step 4 (~lines 82-90) relies on the author manually running it. No spec, plan, or audit re-stamp documents an accepted residual for keeping it out of CI. Med fits the rubric: it's a leverage/enforcement gap (unenforced doc-bundle conformance), not a correctness/safety defect.

**Finding 9.3** — `low`

- **file:line:** `scripts/okf_check.py:86-121`
- **violated principle:** A conformance checker should cover the conventions the bundle's trust model advertises; unchecked conventions drift silently while the skill claims 'conformance clean'.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Extend okf_check.py to (a) require a `resource:` URL on type: Source nodes (agent-sdlc/SKILL.md:61 calls it 'the ultimate ground truth'), (b) validate `type` against the vocabulary in authoring.md (Source/Practice/Lifecycle Phase/Perspective/Comparison — today any non-empty string passes), (c) verify body `[n]` markers correspond to numbered Citations entries, and (d) check each directory index.md lists every node; document that semantic claim drift remains a human re-verification duty with a dated stamp.
- **evidence:** checker only tests `not str(fm.get("type", "")).strip()` for frontmatter and `[t for t in iter_links(section) if t.startswith("/sources/")]` for citations — no resource: check, no type vocabulary, no [n]-marker resolution, no index-coverage check (all 36 sources currently conformant, so the gap is latent)
- **verifier:** Re-derived and empirically demonstrated: a bundle copy with `type: Sorce` and no `resource:` on a Source node passes okf_check.py with OK, while SKILL.md's trust model and authoring.md advertise resource-as-ground-truth, a fixed type vocabulary, [n] markers, and index listings — none machine-checked. No spec or docstring records these omissions as an accepted residual with rationale; severity low is correct (docs-tooling polish, gap currently latent since all 36 sources conform).

**Finding 9.4** — `low`

- **file:line:** `/home/kalen/.claude/projects/-home-kalen-rust-agent-runtime/memory/MEMORY.md:6,9,22-23` *(auditor cited `/home/kalen/.claude/projects/-home-kalen-rust-agent-runtime/memory/MEMORY.md:16-19`)*
- **violated principle:** Always-loaded static context should carry the smallest set of high-signal tokens; an index entry that embeds the full detail defeats the pointer-plus-detail-file split.
- **source:** docs/okf/agent-sdlc/practices/context-engineering.md
- **concrete proposed fix:** Trim the four longest index lines (936, 909, 827, 601 chars — context-evolve campaign state, Harness deep audit, Harness product decisions, harness-evolve campaign state) to one-sentence pointers, moving the campaign detail (commit lists, per-task scores, gotcha chains) into their existing topic files.
- **evidence:** single index line runs to 936 chars, e.g. '[context-evolve campaign state](context-evolve-campaign-state.md) — RESUME POINT (2026-07-03): **CHAMPION v4 PROMOTED (458c383) — MANIFEST GAP CLOSED 0/5→5/5 at 20/20** via extractive fold…' — the detail file it links to duplicates this content
- **verifier:** Re-derived exactly: the four longest index lines measure 936/909/827/601 chars (at lines 23/9/6/22, not the cited 16-19) and duplicate content already in their linked detail files (e.g. context-evolve-campaign-state.md repeats 458c383/9f5edd5/manifest 0/5→5/5 and sweep scores), violating the pointer-plus-detail split from docs/okf/agent-sdlc/practices/context-engineering.md; no spec or memory note marks this as an accepted residual. Caveat for the fix: keep one-clause guardrails like "6 DECLINED-BY-OWNER — don't re-propose" in the index, as those earn always-loaded placement.

### 10. Eval & quality flywheel — findings

**Prior state:** All July eval-flywheel fixes verified live this session: `.github/workflows/ci.yml` gate job runs `scripts/ci.sh` (fmt+clippy+cargo test agent/ + web typecheck/vitest, matching CLAUDE.md's claim exactly) plus the continue-on-error llvm-cov coverage job, which HAS produced a real run (run 28807481970, 2026-07-06, TOTAL 93.34% region coverage); `.githooks/pre-push` execs the same ci.sh. RunResult.trajectory/denials/gold_matched, TaskSpec.gold_trajectory, and the diagnostic-only posture ("Diagnostic only — the promotion gate does not" at eval/result.rs:34) are live; `eval-denied:` stderr emission live at tests/eval_context.rs:387-390; sealed hidden-test step and AGENT_E2E_MODEL pinning intact. policy_corpus grew 86→98 rows and DOES cover the post-corpus /dev-redirect wave including the //dev and /../dev lexical-normalization bypasses (tsv:37-52); the post-exec validator (a hook, not a policy decision) is covered by its own loop_.rs test module (loop_.rs:5591+), not the corpus — appropriate. The neutralized ingestion cap is documented in eval/config.rs (None = historical cap-off, now opt-in genome axis 8) and the one new dependent (H6b experiment) documents the dependence explicitly in its README caveats; the formerly-unbounded L2 skill listings now cap at 50/section (agent-skills/src/tools.rs:149,178). gold_trajectory has a real consumer (H6b arm tasks, gold [list_skills, use_skill, read_skill_file]) with the smoke result and keep-DEFERRED verdict recorded durably in .superpowers/sdd/progress.md; context-/harness-evolve results land in program.md ledgers, champion configs pinned in-repo (champion_v0/v4, memory-roster champion_k10.json), paired-guard + "(config, rate) pair" rule live in harness-evolve/train.md:14-34. One ledger-accuracy discrepancy (not a code regression, the guard never existed): audit.md's accepted-residual note cites an "in-test multi-space assertion" pin for TSV whitespace-smuggling rows that is absent from live source — filed as a finding.


**Finding 10.1** — `med`

- **file:line:** `scripts/ci.sh:9-19`
- **violated principle:** The evaluation/verification suite must act as a quality gate that runs automatically with every proposed change — a tested component outside every CI leg is not gated.
- **source:** docs/okf/agent-sdlc/practices/eval-driven-development.md
- **concrete proposed fix:** Add a fast leg to scripts/ci.sh (or a small Actions job): `python3 scripts/test_okf_check.py && python3 scripts/okf_check.py docs/okf/agent-sdlc` so both the checker's own unit tests and live bundle validity are gated on push.
- **evidence:** ci.sh steps are only: cargo fmt --check / cargo clippy / cargo test (agent/) / web typecheck+vitest. scripts/okf_check.py and scripts/test_okf_check.py (a real unittest suite, merged 2026-07-06 with the okf bundle) appear in no CI leg, and nothing automated validates docs/okf/agent-sdlc/ — the bundle this audit's own citation discipline depends on. The agent-sdlc authoring playbook only instructs a manual `python3 scripts/okf_check.py docs/okf/agent-sdlc` (authoring.md:86).
- **verifier:** Re-derived: ci.sh's four legs (fmt/clippy/cargo test/web) are the entire gate (ci.yml gate job and pre-push hook both just run ci.sh; coverage job is non-gating), and no automation anywhere runs scripts/okf_check.py or its passing 9-test suite — only a manual instruction at authoring.md:86. No accepted-residual exists (the only documented ci.sh exclusion is src-tauri/GTK). Med is right: a tested component and doc bundle outside every gate is a leverage/regression-detection gap, not a correctness/safety defect in shipped code.

**Finding 10.2** — `low` *(auditor said med; verifier's call stands)*

- **file:line:** `.github/workflows/ci.yml:23-40`
- **violated principle:** Eval/quality baselines should become trackable, versioned artifacts — a 'tracked number' that lives only in expiring CI logs is not tracked and its results don't land anywhere actionable.
- **source:** docs/okf/agent-sdlc/practices/eval-driven-development.md
- **concrete proposed fix:** Have the coverage step write the summary to $GITHUB_STEP_SUMMARY and/or upload the llvm-cov summary as an artifact (or append TOTAL to a dated ledger), keeping continue-on-error so it still never blocks a merge.
- **evidence:** The coverage job comment says 'Tracked number, not a gate (spec 2026-07-02 eval-flywheel §5)', but `cargo llvm-cov --workspace --summary-only` only prints to the job log (verified live: run 28807481970 printed TOTAL 93.34%). continue-on-error:true means even the number silently vanishing is invisible; Actions logs expire (~90 days) and no file, step summary, artifact, or ledger records the value — drift between runs is unobservable without manually diffing old logs.
- **verifier:** Verified live: the coverage job only prints `cargo llvm-cov --workspace --summary-only` to the job log with continue-on-error:true; no step summary, artifact, or ledger persists the number, and the eval-flywheel spec's recorded residuals cover only thresholds/ratchets, not persistence. Downgraded to low: the source audit rated coverage tooling itself low, nothing consumes the number today, and the fix is a trivial one-line summary write — polish, not a flywheel leverage bottleneck.

**Finding 10.3** — `low`

- **file:line:** `agent/crates/agent-runtime-config/src/eval/config.rs:184-186`
- **violated principle:** Documented eval-harness invariants must match the code one screen above them — a stale 'neutralized knob' comment misleads future campaign work about what the genome can vary.
- **source:** first principles + runtime conventions
- **concrete proposed fix:** Reword the favorable_disables_curation comment to: 'Ingestion cap is neutralized by DEFAULT (None = cap off); candidates may opt into a realistic cap via max_result_bytes (harness-evolve axis 8).'
- **evidence:** Test comment reads 'Ingestion cap is neutralized for the whole eval harness (not part of the candidate genome)', but the same file defines it AS a genome axis — config.rs:61-63: 'Ingestion cap (axis 8). None = the eval's historical "cap off" semantics.' with `pub max_result_bytes: Option<usize>`, pinned by max_result_bytes_defaults_to_neutralized_and_overrides (config.rs:327-333).
- **verifier:** The comment at lines 184-185 claims the ingestion cap is "not part of the candidate genome," but the same file defines max_result_bytes as genome axis 8 (lines 61-63, 166-169), pins candidate override behavior in max_result_bytes_defaults_to_neutralized_and_overrides (lines 327-333), and the eval driver applies it via cc.offload_config() (tests/eval_context.rs:307) — the comment is stale from before the 2026-07-03 harness-evolve v2 axes and no accepted residual covers it. Comment-only inconsistency with no runtime impact = low (polish).

### 11. Process — the SDLC as run by agents (meaning 2) — findings

**Prior state:** The 2026-07-01 deep audit remains fully closed and its process-dimension fixes are still live: scripts/ci.sh is the single-source CI gate (fmt + clippy + cargo test + web typecheck/vitest), .githooks/pre-push execs it, core.hooksPath IS configured in this clone (verified via git config), .github/workflows/ci.yml runs the same script plus the spec-sanctioned non-blocking coverage job ("continue-on-error ... per spec 2026-07-02 eval-flywheel §5"), and CLAUDE.md documents the per-clone opt-in honestly. The dated re-stamp trail in .agents/skills/harness-engineering/audit.md runs unbroken from 2026-06-30 through the 2026-07-02 backlog-drain 6/6 close-out ("No inline finding remains open in this file"), and the owner adjudication round (5 shipped / 6 declined) is durably recorded in .superpowers/sdd/progress.md:1-60. Spec-first held for every sampled merged feature since 2026-06-25 (claude-design-tab, architecture-viewer, design-tab-url-canvas, auto-dev-server-canvas, webdriver-gui-driving, sandbox-dev-image, cli-panel): each carries a dated spec committed before the plan, plan before implementation, per-task subagent reviews, a whole-branch review with recorded verdict and fix waves, and a --no-ff merge; conventional commits held (only "ci:"/"Merge"/"merge:" variants, all conforming or merge commits); review-*.diff artifacts (200+) evidence the review gate. No process theater detected (spec commits precede implementation in git ancestry in every sample). No regression in previously-closed fixes; the findings below are new bookkeeping gaps plus one 2026-07-01 follow-up that was never closed.


**Finding 11.1** — `med`

- **file:line:** `scripts/ci.sh:3` *(auditor cited `scripts/ci.sh:2-3`)*
- **violated principle:** Verification must be continuously enforced — CI prevents regressions so new commits cannot break existing code; a gate that excludes a whole workspace relies on per-campaign human discipline instead of a machine check.
- **source:** docs/okf/agent-sdlc/practices/verification-first-agent-coding.md
- **concrete proposed fix:** Add a conditional src-tauri step to scripts/ci.sh (cargo build + clippy -D warnings + cargo test from src-tauri/, skipped when GTK/WebKitGTK dev deps are absent so the GitHub runner rationale still holds), or a companion pre-push desktop gate — closing the follow-up recorded in specs/2026-07-01-harness-observability-ci-design.md:39.
- **evidence:** ci.sh: "# Single source of truth for the CI gate ... src-tauri is intentionally excluded (GTK deps)." The exclusion was logged as a follow-up, not a permanent residual (observability-ci spec line 39: "src-tauri in CI (needs GTK/WebKitGTK system deps; follow-up)"), yet four desktop-heavy features merged since (c54b47b, 41b28f0, dfec8b7, 9389912) with only manual per-campaign checks — progress.md:291: "src-tauri (NOT in ci.sh, GTK-excluded) checked separately ... intentionally NOT fmt-gated". The GTK rationale does not apply to the local pre-push path, where the desktop app builds daily.
- **verifier:** Live ci.sh gates only agent/ and web/ and explicitly excludes src-tauri ("intentionally excluded (GTK deps)"); the spec (2026-07-01-harness-observability-ci-design.md:39) logged the exclusion as a follow-up, not an accepted residual, and no later spec/audit re-stamp accepts it — meanwhile src-tauri-touching merges (41b28f0, dfec8b7, 9389912; note c54b47b actually touched no src-tauri files, so the evidence overcounts by one) were verified only by per-campaign manual build/clippy/test per progress.md:291, and those same manual runs prove the GTK deps exist locally, so a conditional pre-push step is feasible. Only the fmt exclusion has a separately documented rationale (hand-format churn); build/clippy/test gating has none. Severity med per rubric: a leverage/enforcement gap, not a live correctness defect.

**Finding 11.2** — `low`

- **file:line:** `.superpowers/sdd/progress.md:296`
- **violated principle:** Durable progress artifacts must hold accurate final state so a fresh session can pick up the work cleanly; an unclosed ledger entry is an open loop.
- **source:** docs/okf/agent-sdlc/practices/harness-engineering.md
- **concrete proposed fix:** Append the MERGED close-out line to the auto-dev-server-canvas section (merge dfec8b7, --no-ff, branch deleted, feature CLOSED), matching every sibling section.
- **evidence:** Section ends "- BRANCH READY TO MERGE @ 370b64a. All 7 tasks + 3 fixes complete, all gates green." with no MERGED record, while git log shows "dfec8b7 Merge feature/auto-dev-server-canvas" on main. Every other feature section in the ledger records its merge (lines 213, 226, 244, 261, 277, 306); this is the only one left reading as in-flight.
- **verifier:** Line 296 still ends the auto-dev-server-canvas section at "BRANCH READY TO MERGE @ 370b64a" with no MERGED record, yet git shows merge commit dfec8b7 on main (370b64a is an ancestor, branch deleted) and all six sibling sections carry MERGED close-out lines. Pure ledger-hygiene gap with a single concrete fix, so severity stays low.

**Finding 11.3** — `low`

- **file:line:** `.superpowers/sdd/progress-parallel-dispatch.md:33`
- **violated principle:** Stale ledgers are the anti-pattern durable-progress-artifact practice warns about: a completed feature's ledger frozen at an in-flight state misleads any agent that reads it as live context.
- **source:** docs/okf/agent-sdlc/practices/harness-engineering.md
- **concrete proposed fix:** Close and rename the completed non-archived ledgers (progress-parallel-dispatch.md, progress-sandbox-degraded.md, progress-harness-engineering-skill.md, progress-context-explorer-*.md) to the established .archive.md convention with a one-line merged stamp.
- **evidence:** Tail reads "Branch HEAD after doc-align commit. READY for finishing-a-development-branch." though the branch merged to main days ago; progress-sandbox-degraded.md likewise ends "Ready for finishing-a-development-branch" post-merge (be67413). Ten sibling ledgers were properly renamed *.archive.md; these five were not, so the directory presents false in-flight state.
- **verifier:** Re-derived: line 33 (the file's last line) still reads "READY for finishing-a-development-branch" yet the branch's commits (96ec134, 171f573, 7329bd1) are all ancestors of main (merged ~2026-06-30, a week before today); progress-sandbox-degraded.md likewise ends "Ready for finishing-a-development-branch" with its HEAD 9cf68e7 also in main, while ten sibling ledgers were properly renamed *.archive.md and no spec/audit re-stamp documents this as an accepted residual (the 2026-07-06 audit plan itself flags "stale ledgers" as an anti-pattern to scan for). Minor caveat: progress-context-explorer-feature.md's text does say "MERGED TO MAIN", so for that one file the defect is only the missing archive rename, not false in-flight content — the fix (rename + merged stamp) still applies to all five. Severity low: repo hygiene/polish, no correctness or leverage impact.

**Finding 11.4** — `low`

- **file:line:** `.superpowers/sdd/progress.md:309`
- **violated principle:** Human review gates should sit before integration; when a documented gate is waived by an unwritten convention, the process contract and actual practice diverge.
- **source:** docs/okf/agent-sdlc/practices/human-in-the-loop-gates.md
- **concrete proposed fix:** Either run docs-only campaigns on a branch like feature work, or record the docs-on-main exception and its compensating control (whole-campaign review must pass before any push) in CLAUDE.md's "How we work" so the contract matches practice.
- **evidence:** "executing on main (ops plan: absolute paths pinned to live checkout, docs-only commits)" and line 321 "executing on main (docs-only, precedent per prior campaigns)" — the precedent is recorded only in the ledger, not in the CLAUDE.md process contract. The qwen campaign shows the cost: whole-campaign review found 2 Importants AFTER commits landed on main, and one could not be repaired ("Declined: retro-timestamping ledger pass-count lines (cannot retro-fix honestly)", progress.md:317). Mitigation exists (main is never auto-pushed; fix waves landed promptly), hence low.
- **verifier:** Lines 309/321 confirm two campaigns executed directly on main citing only ledger precedent, while CLAUDE.md's contract says branch off main and documents no docs-only exception (verified by repo-wide search of CLAUDE.md, specs, audits). The claimed cost is real: whole-campaign review found 2 Importants after commits landed on main and one was unrepairable (progress.md:317 "cannot retro-fix honestly"), but never-auto-pushed main and prompt fix waves keep this at polish-level severity.

---

## Appendix A — dropped findings (refuted by verification)

- **guardrails** `agent/crates/agent-tools/src/shell.rs:35 + agent/crates/agent-core/src/loop_.rs:825-834` — claimed: Tiered oversight must route actions by actual risk — a flat tier declaration makes downstream guardrails (here the post-exec validator trigger) fire on routine read-only work, adding friction/latency without safety benefit. **Refuted:** The code behavior is real (execute_command declares flat Access::Write, so a git status/grep-only turn sets turn_mutated and runs validators), but it is a documented accepted residual: docs/superpowers/specs/2026-07-02-post-tool-validator-hook-design.md:46-49 explicitly records "Accepted over-trigger: execute_command is always Access::Write even for a read-only command (git status)... Documented, not gated further," with a still-valid rationale (feature is opt-in/default-empty, idempotent, appends only on failure) — so the cluster-D contract is not contradicted, and per the procedure an accepted residual with valid rationale is a refutation. Residual cost is latency/context noise in an opt-in path, i.e. polish, not med.
- **observability** `agent/crates/agent-core/src/loop_.rs:1125-1129 (ApprovalRequest at agent-policy/src/engine.rs:18-21; trace record at agent-runtime-config/src/trace.rs:220-223)` — claimed: Events must be correlatable by id, not by adjacency — the runtime's own convention since the sub-agent observability spec moved the web reducer to id-first correlation. **Refuted:** The claimed failure mode does not exist: approvals are gated sequentially by design ("Sequential by design so approval prompts never overlap", loop_.rs:1054-1056; the Phase-1 for-loop awaits each gate_tool before the next, loop_.rs:765-786), so Approval records can never interleave — max_parallel_tools only bounds Phase-2 execution of already-approved calls. Moreover each Approval trace record deterministically and immediately follows the ToolStart{id} of the exact call it gates (emitted first inside gate_tool, loop_.rs:1057-1062), so attribution is unambiguous even for identical summaries; the residual "store the id/decision in a field instead of relying on guaranteed ordering" is polish, not an observability gap.
- **eval-flywheel** `agent/crates/agent-runtime-config/tests/policy_corpus.rs:100-132` — claimed: An accepted-residual's recorded rationale must describe protections that actually exist — the re-stamp ledger names an in-test pin that is absent from live source. **Refuted:** The residual note at .agents/skills/harness-engineering/audit.md:589-592 does not claim the pin exists: its main clause says rows "could be silently degraded" (only true with no pin), and the parenthetical names the multi-space assertion as the available cheap remedy, matching the conditional-remedy phrasing of sibling accepted residuals. The gap (no assertion, no .gitattributes) is real but is an honestly-recorded accepted residual with a still-valid rationale, so the finding's ledger-misrepresentation premise fails; its "rows 15/24" evidence also overreaches (row 24 is single-spaced and unaffected by multi-space collapse).

## Appendix B — dimension / verification stats

| Dimension | Raised | Confirmed | Refuted | Unverified |
|---|---|---|---|---|
| Instructions & rule files | 1 | 1 | 0 | 0 |
| Tools | 5 | 5 | 0 | 0 |
| Sandboxes & execution | 5 | 5 | 0 | 0 |
| Orchestration & sub-agents | 5 | 5 | 0 | 0 |
| Guardrails & policy | 4 | 3 | 1 | 0 |
| Observability | 3 | 2 | 1 | 0 |
| Context engineering (Spine B) | 3 | 3 | 0 | 0 |
| Desktop/web design-tab harness | 6 | 6 | 0 | 0 |
| Skills & knowledge layer | 4 | 4 | 0 | 0 |
| Eval & quality flywheel | 4 | 3 | 1 | 0 |
| Process — the SDLC as run by agents (meaning 2) | 4 | 4 | 0 | 0 |
| **Total** | **44** | **41** | **3** | **0** |

Confirmed severity mix: 18 med, 23 low, 0 high. Fleet: 55 agents (11 auditors + 44
per-finding verifiers), ~2.57M subagent tokens, 798 tool uses, 23.6 min wall clock.
Ground-truth gates (`okf_check`, `ci.sh`) ran once, read-only, before the fan-out.
