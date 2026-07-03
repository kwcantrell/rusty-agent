# harness-evolve — a whole-harness optimization campaign

**Date:** 2026-07-03
**Status:** Approved (brainstorm round 2026-07-03; all four design decisions user-approved)
**Branch:** `evolve/harness-evolve-campaign`

## Problem

context-evolve optimizes one harness component — in-window curation + long-term
memory — and its admitted tasks are synthetic fact-retention probes plus one
trivial-transcription cargo task. Nothing measures, or improves, the harness's
ability to carry a **long-running, complex programming task end to end**: the
canonical target is the agent building a working TypeScript website (Vite,
typecheck, tests) at a realistic window, graded by real commands.

The PDF this campaign draws on (docs/superpowers/context/
the-new-sdlc-with-vibe-coding-google-2026.pdf) frames it as *Agent = Model +
Harness*: prompts, tools, context policies, sub-agents, sandboxes, skills, and
observability dominate agent behavior — on Terminal Bench 2.0 a team moved a
coding agent from outside the Top 30 to Top 5 by changing only the harness.
This runtime now has all of those surfaces built (sub-agent dispatch complete,
skills with disclosure levels, ModelRef routing, sampler knobs, docker
sandbox), and none of them has ever been under a measured optimization loop.

**harness-evolve** is that loop: a self-improving campaign, sibling to
context-evolve, that iterates on the WHOLE harness against real web-coding
tasks. Where `harness-engineering` (the audit/build skill) designs by
judgment, harness-evolve promotes by measurement.

## Identity and relationships

- **Skill:** `.agents/skills/harness-evolve/` (Claude-facing campaign skill;
  SKILL.md / prepare.md / train.md / program.md + `tasks/`).
- **Sibling to context-evolve**, never a replacement: different objective
  surface (whole-harness task success vs curation fidelity), different task
  class (real web builds vs fact-retention probes), separate champion lineage
  and program.md. The two campaigns share ONE runtime, so **each campaign's
  guard sweep includes the other's admitted set** (see Guard sweep).
- **Runtime skills it authors** (the agent's own SDLC, axis 5) are runtime
  artifacts served via `skills_dirs`, NOT `.agents/skills/` content — the
  two-tree gotcha in CLAUDE.md. Candidate skill variants live under the
  campaign dir (`.agents/skills/harness-evolve/artifacts/agent-skills/<variant>/`)
  and are pointed at per-candidate.

## Objective (never violate)

Lexicographic, inherited from context-evolve:

1. A change that lowers the pass count on the training set is **rejected**.
2. Among correctness-preserving changes, prefer **lower median tokens**
   (passing runs only).
3. A promotion must not regress **any** held-out task's pass rate, **nor any
   task in context-evolve's admitted set** (hard gate — shared runtime).
4. The honest success metric is the **locked task set** (the canonical
   end-to-end website build), run **once** at campaign end.

Wall-clock is recorded (new additive `wall_ms` on `RunResult`) as a
**diagnostic only — it never gates**: the shared llama server (`-np 4`,
unified KV) makes wall-clock too noisy to compare candidates on; tokens
remain the cost currency.

"Passed" for a web task = `test_cmd` exit 0, running **offline**:
`tsc --noEmit && vitest run && vite build` plus content greps over `dist/`.
**No Playwright in v1** — a browser adds minutes and flake per run with
little discrimination beyond build+tests+greps; a dev-server probe tool is a
candidate *tool axis* for a later iteration instead.

## Phase-0: the `node-offline` exec profile (prerequisite, not an axis)

The eval harness has **never used the docker sandbox**: `tests/eval_context.rs`
forces `sandbox_mode: "off"` (line ~152) and runs both agent commands and
`test_cmd` as host bash. "cargo is allowlisted" means the eval's `SafeApproval`
list (eval_context.rs:53) + the policy prefix allowlist — a policy boundary,
not an execution boundary. That was acceptable for std-only cargo crates with
`--offline`; it is not acceptable for node: `npm run <script>` executes
agent-writable `package.json` content, and vite/vitest binaries execute
agent-written code. Web tasks get a real boundary.

### Design: offline-first, sandboxed

- **Offline by construction.** `npm install` runs ONCE, by the task author, at
  task-freeze time; the frozen task ships a pinned lockfile + pre-seeded
  `node_modules` (kept out of git via the task's own storage discipline —
  see prepare.md; a `seed.sh` regenerates it deterministically from the
  lockfile). Agent runs and grading never touch the network. No install
  command is allowlisted for the agent at all. This simultaneously:
  - answers the egress question — there is no egress; `--network none` stays;
  - makes every run warm and deterministic (no install in the loop — the
    run-cost answer);
  - keeps grading reproducible across the campaign.
- **Execution inside the docker sandbox.** Additive
  `TaskSpec.exec_profile: Option<String>`:
  - `None` → today's behavior byte-identical (sandbox off, host exec). All
    existing context-evolve tasks and champion numbers stand; no re-baseline.
  - `Some("node-offline")` → the eval builds the loop with
    `sandbox_mode: "enforce"`, `sandbox_image: "node:22-bookworm-slim"`
    (pinned; matches the host's node 22), network **none** (the existing
    default). `sandbox_image` is already a `RuntimeConfig` field; the eval
    stops forcing sandbox off for these tasks.
- **Grading is sandboxed too.** For `node-offline` tasks, `test_cmd` runs in
  the same container profile (one-shot `docker run` with identical hardening
  flags), because vitest/vite execute agent-written code — untrusted output
  gets the same boundary as the agent. Host-bash grading remains for
  `exec_profile: None` tasks.
- **Allowlist:** scoped prefixes only, mirroring the cargo-subcommand
  precedent: `npx tsc`, `npx vitest run`, `npx vite build`, `node`. NOT
  allowlisted: `npm install`, `npm ci`, bare `npm`, `npm run` (the
  package.json-scripts hole). If iteration shows the model insists on
  `npm run build`-style invocations, the allowlist may add specific
  `npm run <script>` forms — inside the sandbox that is contained; record the
  decision in program.md when it happens.

### Security review (phase-0's own)

| risk | disposition |
|---|---|
| npm registry egress / supply chain at run time | eliminated: `--network none`, no install commands allowlisted, deps frozen at authoring time by a human-audited lockfile |
| agent-written code executing during grading | contained: grading runs in the same sandbox profile |
| `npm run` = arbitrary exec via agent-writable package.json | not allowlisted in v1; if ever added, sandbox contains it |
| lifecycle scripts during authoring-time `npm install` | trusted operation, run by the author (same trust level as `web/`'s own npm usage); `--ignore-scripts` preferred where the dep tree allows |
| container escape | out of scope — docker is the accepted boundary (consistent with the sandbox-fail-closed spec) |
| resource exhaustion (vite/tsc are hungry) | container limits need headroom: tmpfs size, `--pids-limit`, memory — sized during implementation, recorded in program.md |
| node_modules uid/gid on the mounted workspace | the sandbox runs `--user uid:gid` (nobody fallback); the seed step must chown/verify readability — implementation checklist item |

Fallback (recorded as debt if used): if docker-in-eval hits blocking friction,
the cargo-precedent host route (eval-only allowlist + offline seeds) is the
temporary bridge — weaker boundary, explicitly marked in program.md, replaced
before any Tier-B promotion that touches exec paths.

## Genome widening (CandidateConfig v2)

All additive `#[serde(default)]` `Option` fields, inherit-on-`None`, following
the shipped `system_prompt`/`protocol` pattern (2026-07-02 widening spec).
Existing frozen configs parse unchanged; `favorable(window)` leaves every new
field `None`.

| new field | type | axis |
|---|---|---|
| `active_skills` | `Option<Vec<String>>` | 5 — the agent's own SDLC as runtime skills |
| `skills_dirs` | `Option<Vec<String>>` | 5 — per-candidate skill tree |
| `temperature`, `top_p`, `top_k`, `min_p` | `Option<f32/u32>` | 7 — sampling (per-request; no server restart) |
| `subagents` | `Option<bool>` | 4 — sub-agent policy |
| `subagent_max_turns`, `subagent_max_depth` | `Option<usize>` | 4 |
| `subagent_model` | `Option<ModelRef>` | 4/7 — role/protocol/base_url routing (built) |
| `tool_descriptions` | `Option<HashMap<String,String>>` | 3 — tool vocabulary |
| `max_result_bytes` | `Option<usize>` | 8 — ingestion cap (None = today's pinned-off eval semantics; web tasks can opt into the realistic 16 KiB cap without re-baselining context-evolve) |
| `max_turns` | `Option<usize>` | driver seam — replaces the hardcoded 12 in eval_context.rs; a website run needs more |

Notes:

- **`tool_descriptions` builds the declined seam** from
  `2026-07-02-tool-description-override-seam-design.md` — its recorded revisit
  trigger ("a campaign that wants tool descriptions as a candidate axis") has
  fired. Build per that sketch: `ToolRegistry.description_overrides` +
  `RuntimeConfig.tool_description_overrides` (serde-default) + the
  CandidateConfig field; re-verify the three anchor points against live source
  first, as the sketch instructs.
- **`wall_ms`** is an additive `#[serde(default)]` field on `RunResult` (old
  JSONL lines parse). `gate`/`admit` are untouched — they compare
  `passes()`/`median_tokens_passing()` only.
- The eval driver applies each field only when `Some` — champion configs from
  context-evolve remain valid candidate inputs forever.

## Tiering

- **Tier A — genome (no rebuild):** every field above, plus editing the
  *content* of candidate runtime-skill files under the campaign's artifacts
  dir (file edits, no rebuild — "Tier A′"). Prove signal here first.
- **Tier B — runtime code (rebuild):** `agent-core` (loop, dispatch,
  curation), `agent-tools` (new tools — e.g. a dev-server probe),
  `agent-skills`, `agent-memory` internals, prompts.rs. Snapshot-binary
  pairing discipline (git stash / built-binary snapshots per code state);
  guard sweep mandatory (both campaigns' sets).
- **Tier C — server topology (NEW):** llama-server startup flags, model
  swaps, speculative decoding, multi-model serving. Every Tier-C change is a
  **re-baselining event** (the calibrated-budgeting lesson: it rewrites the
  eval landscape), never a per-iteration variable. Tier-C work happens as
  dedicated spikes with before/after baselines of BOTH campaigns' suites.

### The model-topology question (axis 7) — staged spike, not a commitment

Hardware arithmetic (verified 2026-07-03): RTX 3090 24 GB; 35B-A3B IQ4_XS =
17.7 GB resident (server settles ~21.6/24.6 GB with 192K q8_0 KV);
27B Q5_K_XL = 20 GB; 27B Q4_K_XL = 17.6 GB. **No 27B variant co-resides with
the 35B.** Staging:

1. **Orchestrator-as-role on the SAME 35B** (feasible today, zero VRAM cost):
   `subagent_model` pointing at the same server with a distinct `role` block,
   protocol, and sampler settings. This is the first sub-agent/topology
   hypothesis family.
2. **Serial model swap per phase** — only if role separation shows real
   signal: measure swap cost honestly (~30 s container restart + model load +
   total KV/prompt-cache loss per swap). Expected to fail the run-cost test;
   measure once, record, move on.
3. **Partial CPU offload for co-residency** — expected dead end (dense 27B
   layers on CPU collapse throughput); record the arithmetic in program.md so
   it is never re-tried casually. `gpt-oss-120b` (MoE, ~5B active, on disk)
   is a wildcard for CPU-heavy serving; spike-tier only.

Startup-only vs per-request flags are catalogued in the `local-llama-server`
memory; only per-request knobs (samplers) are Tier-A.

## First discriminator: `web-multipage`

Weakness-first and favorable-passable. The locked-hostpolicy lesson binds:
favorable must be ≈5/5 or the signal is mud — so **vanilla Vite + TS, no
framework** (React raises the capability bar for a ~3B-active model with no
added discrimination; it becomes a later, harder task once a champion exists).

- **Seeded workspace:** complete Vite+TS scaffold — pinned lockfile,
  pre-seeded `node_modules`, vitest configured, `tsconfig` strict; a tiny
  hash-router with `routeFor()` stubbed; a `fetchStats()` data hook stubbed
  against a local JSON fixture; one intentionally failing vitest spec driving
  a small feature.
- **Pressure:** 6–8 requirements (route → title/content pairs; the hook's
  field mapping) delivered across turns, each behind a noise read — the
  portmap delivery pattern, already proven favorable-passable in shape — then
  a final "implement everything" turn.
- **Grading:** hidden vitest suite + `tsc --noEmit` + `vite build` + dist
  greps; offline; in-sandbox.
- **Admission:** `eval_gate admit`, N=5 per side, thresholds as shipped
  (favorable ≥0.8, realistic <0.5). The realistic window is found
  empirically per prepare.md — start ~8000 (web tool outputs are fatter than
  the Rust tasks; 4000 may not even build a window).
- **Ladder if the verdict is wrong:**
  - `CapabilityBound` → strip the data hook; then the failing-test feature;
    the floor is a routes-transcription core (portmap-with-npm).
  - `NoWeakness` → shrink `context_limit` and/or add requirement turns.
  - `IllSized` → shorten noise reads / requirement count.
- **Task-authoring hazards (inherited):** `set_goal` pins the FIRST user
  prompt verbatim — no load-bearing facts in prompt #1; noise reads create
  the pressure, not workspace file size (large outputs offload).

The **canonical end-to-end website build** (empty dir → working multi-page
app with router, data fetch, tests, all green) is NOT the first
discriminator: it is the campaign's **locked task**, authored later per
prepare.md's locked-task rules and run **once** at campaign end — the honest
generalization metric, exactly like locked-portmap's role in context-evolve.

## Campaign roadmap (axes, in unlock order)

0. **Phase-0** (above) + admit `web-multipage` + champion v0 baseline.
1. **System prompt (axis 2)** — Tier-A `system_prompt` variants; the base
   prompt is one hardcoded const (prompts.rs BASE_SYSTEM_PROMPT) that has
   never been evaluated.
2. **Agent SDLC skills (axis 5)** — Tier-A′ `skills_dirs`/`active_skills`:
   a spec→plan→implement→verify skill the agent follows inside the runtime,
   inspired by the PDF's factory-model/phases and the superpowers structure.
   Start with ONE skill (e.g. "verify before done: run tsc/vitest and read
   failures before replying") — one hypothesis per iteration still applies.
3. **Sampling (axis 7a)** — temperature/top_p per the web task; later per
   sub-agent role.
4. **Tools (axis 3)** — description overrides first (Tier-A once the seam
   lands); missing tools (dev-server probe, typed test-runner tool) are
   Tier-B iterations.
5. **Sub-agents (axis 4)** — dispatch policy: when to delegate, role prompts,
   depth/turn budgets; orchestrator-as-role topology (spike 1).
6. **Memory (axis 6)** — real-embeddings only (stub is non-semantic — the
   memory-recall correction stands); cross-session web tasks come later.
7. **Tier-C topology spikes (axis 7b)** — staged as above.
8. **Axis 8 backlog** (program.md hypothesis queue from day one): summary
   poisoning by transient tool errors (context-evolve open issue #2);
   ingestion-cap realism (`max_result_bytes` genome field); anything the
   first CE_DEBUG dumps surface.

## Method discipline (inherited wholesale from context-evolve)

- **Diagnose before designing** — CE_DEBUG-style window/trace dumps before
  any hypothesis; both 2026-07-03 context-evolve wins came from window dumps.
- **One mechanism-level hypothesis per iteration; one change.**
- **Paired champion-vs-candidate at equal N** (N=5 for web tasks — see run
  cost), same-night batches; snapshot binaries for Tier-B.
- **Attribute single misses by prefix identity, not batch counts** (llama.cpp
  is not bit-deterministic at temp 0).
- **Promote only on strictly-more-passes** (or equal passes + fewer tokens);
  the `gate` 0-passes token artifact is documented — read `passes()` directly.
- **THE GUARD SWEEP IS NOT OPTIONAL** and spans both campaigns:
  - harness-evolve's own held-outs (as they accrue), and
  - context-evolve's admitted set with its v4 ceilings: longhaul-manifest
    5/5 (20/20 entries), locked-portmap 10/10, drift-ledger ~11/12,
    longhaul-codename 5/5, offload-recall 5/5, memory-recall 5/5 (real
    embeddings), memory-roster ~9/10 (known ~5–10% per-batch storage-slip
    noise).
  Tier-A changes that are provably inert to context curation (e.g. a sampler
  change with context knobs untouched) may run a reduced sweep — but any
  Tier-B change, any prompt/skill/tool change, and anything touching the loop
  runs the full sweep. When in doubt, full sweep.
- **K=6 consecutive non-improvements → stop; then run the locked set once.**
- **Append-only program.md; never retry a logged dead end.**

## Run-cost discipline

A web run ≈ 2–4 min (more turns than drift-ledger's ~45 s, bigger tool
outputs, container exec) ⇒ a paired N=5 iteration ≈ 30–45 min + guard sweep.
Rules:

- **Warm by construction:** node_modules pre-seeded; no network; container
  image pulled once. The eval must not add per-run installs, ever.
- **Honest N:** N=5 paired for iteration; N=5/5 for admission; no mid-batch
  config or code edits; a batch interrupted by a server restart is discarded
  whole.
- **`{"passed":false,"tokens":0,"turns":0}` on every run = server down or
  `AGENT_E2E_URL` wrong** (http://localhost:8080, no /v1) — check `docker ps`
  before debugging anything else.
- Batch runs use absolute paths for `TASK_JSON`/`CONFIG_JSON`/
  `HIDDEN_TESTS_DIR` (integration-test cwd gotcha).

## Scaffolding layout (the session deliverable)

```
.agents/skills/harness-evolve/
  SKILL.md      — objective, tiers A/A′/B/C, prerequisites, do-not list
  prepare.md    — web-task authoring: offline-seed recipe (lockfile freeze,
                  seed.sh, node_modules discipline), exec_profile, admission
                  ladder, locked-task rules
  train.md      — iteration loop; cross-campaign guard list (explicit task
                  ids + ceilings); Tier-C re-baseline rule
  program.md    — day-one seed: hardware constraint table, phase-0 security
                  decisions, topology arithmetic/dead-ends, hypothesis
                  backlog (roadmap above), champion v0 block once admitted
  tasks/web-multipage/
    task.json, hidden_tests/, favorable.json, champion_v0.json, seed.sh
  artifacts/agent-skills/   — candidate runtime-skill variants (axis 5)
```

## Testing

- Genome widening: unit tests per field following the existing
  `widening_tests` pattern (serde-default back-compat pin, resolver
  inherit/override, favorable-leaves-None).
- `wall_ms`: old-JSONL parse pin; gate/admit unaffected (existing tests
  already pin outcome-only comparison).
- Tool-description seam: registry unit tests per the recorded sketch
  (override replaces base description; `when_not_to_call` fold still
  appends; unknown names warn-and-ignore).
- `exec_profile`: TaskSpec serde default (None) pin; a unit test that
  `None` produces today's LoopConfig (sandbox off) and `"node-offline"`
  produces enforce+image+network-none — the live container path is exercised
  only under the `#[ignore]` eval like everything else.
- The first admitted task IS the integration test of phase-0 (favorable 5/5
  requires the whole sandboxed-node path to work).
- `bash scripts/ci.sh` green before merge (fmt + clippy + cargo test +
  web typecheck/vitest).

## Out of scope (recorded)

- Implementing any optimization axis (the campaign iterates on them later;
  this spec delivers the loop, not the wins).
- Playwright/browser-driven grading (revisit as a tool axis with evidence).
- Python-in-sandbox (the brief's "possibly python" — add a profile when a
  task needs it; the exec_profile seam makes it additive).
- Multi-model serving / speculative decoding builds (Tier-C spike outcomes
  decide if they ever get specs).
- Production (non-eval) exposure of the new genome knobs beyond what already
  exists on RuntimeConfig.
- Session rehydration, live trace toggle, and other declined-by-owner items
  (do not re-propose).

## Open questions (tracked in program.md, not blockers)

- Does the model *use* `npx`-style invocations naturally, or does it insist
  on `npm run`? (Decides whether scoped `npm run <script>` allowlist entries
  are needed — contained by the sandbox either way.)
- Realistic window for web tasks (~8000 starting guess; found empirically).
- Whether `wall_ms` noise under `-np 4` is small enough to ever justify a
  wall-clock tiebreak (diagnostic data will answer this).
