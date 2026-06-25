# context-evolve — a self-improving harness for the context manager

**Date:** 2026-06-25
**Status:** Design (approved for spec review)

## Goal

An [autoresearch](https://github.com/karpathy/autoresearch)-style optimization loop, packaged as a
skill, where a Claude agent iteratively improves this runtime's context-management subsystem
(`CuratedContext` and friends) so that the *running* model (Qwen3.6 on the local llama.cpp server)
can solve a hard, long-horizon task **without drifting from the original goal** and in **as few total
tokens as possible**.

The optimization objective is lexicographic, never blended:

1. **Correctness is a hard gate.** A change that lowers the task pass-rate is rejected outright.
2. **Tokens are the tiebreaker.** Among changes that hold correctness, prefer the one that processes
   fewer total tokens.

A blended score is explicitly rejected: it would let the optimizer trade away correctness for
cheapness, which is the exact failure mode this skill exists to prevent.

## Non-goals

- Not training/fine-tuning the model. The *only* thing that changes across iterations is the
  context-management code and its parameters. The model is fixed.
- Not an interactive tool. This is an overnight research harness; a single iteration is several
  minutes-long live runs.
- Not a general agent-eval framework. It optimizes context management specifically.

## Architecture overview

A skill at `.agents/skills/context-evolve/` with three markdown playbooks plus a self-introduction:

- `SKILL.md` — entry point. Explains the loop, prerequisites (live server, built harness), and how to
  run a campaign.
- `prepare.md` — playbook: freeze the task, hide the test oracle, scrub git, record the v0 baseline.
- `train.md` — the per-iteration reasoning loop (hypothesize → edit → eval N× → gate → record).
- `program.md` — **written-to**, append-only research memory: every hypothesis tried with its verdict,
  and the current champion config. Survives across sessions so the loop compounds.

Plus one load-bearing code artifact: the **eval harness** (below).

## The eval harness (the piece everything rests on)

A new Rust **test-binary** `eval_context` in `agent/crates/agent-runtime-config/tests/`, modeled
directly on the existing `soak_live.rs` (same live-server client, `SafeApproval` blast-radius gate,
throwaway temp-workspace pattern). One invocation = **one run**:

1. Read a **candidate config** from env/file: the Tier-A parameters
   (`high_water_pct`, `keep_recent`, `error_min_bytes`, `output_min_bytes`, `recall_budget`,
   window size).
2. Drive the real `assemble_loop` against the live server on the frozen task prompt.
3. After the agent stops, **restore the hidden tests** and run them in a sealed step the agent never
   sees. Pass = the commit's frozen test subset goes from red to green.
4. Emit one JSON line: `{ "passed": bool, "tokens": int, "turns": int }`.

**Runtime-injected params (no rebuild for Tier A).** The Tier-A parameters are injected at runtime via
the existing builder methods (`with_high_water_pct`, `with_offload_config`, `with_recall_budget`), so a
candidate config costs **only N live runs, no `cargo build`**. Only Tier-B code rewrites trigger a
rebuild. The server's parallel slots (`-np 4`) let the N runs partly overlap, cutting wall-clock.

**Token metric.** Total tokens = sum over all turns of **server-reported** `usage.prompt_tokens +
usage.completion_tokens`. The internal `message_tokens` estimate is *not* used: it is known to
undercount (it ignores reasoning content and `tool_calls`). Optimizing against a metric that undercounts
the very bloat (reasoning + tool churn) we are trying to reduce would be self-defeating.

## prepare.md — authoring trustworthy tasks

Tasks come from two sources, by tier (see "Three tiers" below):

- **Training + held-out: weakness-first synthetic tasks** — TDD applied to task authoring. The task is
  built to provably expose a context-management weakness *before* it is admitted.
- **Locked: real past commits** — external validity the synthetic loop structurally cannot provide.

### Weakness-first synthesis (training / held-out tasks)

A task is authored red-first, exactly like a test you must see fail before trusting:

1. **Analyze** the context manager and **hypothesize one weakness** (e.g. "compaction drops a path the
   agent needs 20 turns later," "re-grounding is too weak to survive a full window of sub-task churn").
2. **Author a task + hidden tests** that target that weakness. Tests are **moved to a sealed location**
   the agent's workspace cannot see (prevents hardcoding to the assertions and banking a fake "pass").
3. **Two-sided admissibility test** — run the model N× under two context configs. The task is admitted
   **only if both hold**:
   - **Red under a realistic/constrained config** — the weakness bites and the run **fails the
     correctness gate** (hidden tests red). *There is something to capture.*
   - **Green under a deliberately-favorable config** — huge window, compaction off, generous recall.
     *The failure is attributable to context management, not raw model capability.*

   Both-fail ⇒ capability-bound, discard. Both-pass ⇒ no weakness, discard. The gap between the two
   configs **is** the headroom the optimization loop is allowed to close.
4. **Correctness-only discrimination.** The red that admits a task is always a *correctness* failure
   (tests red), never token bloat. Tokens remain the optimization tiebreaker among passing runs, but a
   task's validity as a probe is defined purely by correctness.
5. **Record** the weakness, both configs, and the admissibility results in `program.md`. The task prompt
   is **natural language** describing the goal in its own words.

The first task targets a **drift / re-grounding** weakness: long-horizon, the window fills, and the
re-grounding block is what should keep the agent anchored to the original goal.

### The favorable and realistic configs (concrete)

The two-sided test contrasts two fixed reference configs. Both reuse the existing runtime knobs —
`context_limit` (→ `model_limit`), `high_water_pct`, `OffloadConfig`, `recall_budget` — so no new code
is needed to express them.

**Favorable config — "the context manager neutralized."** Nothing is ever dropped, summarized, or
offloaded; the model sees the entire transcript verbatim with the goal pinned. The only way it can still
fail is genuine incapability.

- **Window:** `context_limit` = the server's full physical context. For the local Qwen3.6 / llama.cpp
  setup that is **196608** (`-c 196608`). This is the *largest* window the server actually serves.
- **Compaction: off.** `high_water_pct = 1.0` (per `with_high_water_pct`, `>= 1.0` effectively disables
  automatic compaction — it can only fire once `used` exceeds the full window, which the task-size
  invariant forbids).
- **Offload: off.** `OffloadConfig { error_min_bytes: usize::MAX, output_min_bytes: usize::MAX, .. }` so
  no tool result ever qualifies; every result stays verbatim in-window.
- **Re-grounding: on.** The goal block stays pinned. Favorable means *best-case good context*, and a
  pinned goal is pure upside; it also limits lost-in-the-middle drift in a long verbatim window.
- **Recall budget:** generous/irrelevant (nothing is offloaded, so nothing is recalled).
- Favorable runs are allowed to be maximally expensive in tokens. Favorable is a **correctness** probe
  only ("can the model do this at all?"); it is never used for the token objective.

**Realistic config — what creates the curation pressure.** This is the **champion** context-management
config (the thing the loop mutates). The weakness must bite here: under realistic curation the run fails
the correctness gate (red).

**Task-size invariant (what makes a favorable run meaningful).** A synthetic task is only admissible if
its *entire uncurated transcript fits within the favorable `context_limit` with headroom* (target
≤ ~75% of the window, both to avoid `build()` silently windowing and to limit lost-in-the-middle
degradation that could make a context-bound task *look* capability-bound). If the favorable run itself
overflows the window, the task is rejected as **ill-sized** — not labeled capability-bound. Scope note:
this means the skill optimizes *curation quality within a window*, not infinite-context behavior.

**Cheap pressure for synthetic tasks (decision, override if undesired).** To keep synthetic tasks small
and fast, their *realistic* config uses a **scaled-down** `context_limit` (e.g. 16–32K) so the weakness
bites with a modest transcript, while their favorable config uses a window just large enough to hold
that transcript verbatim. The **locked real-commit tasks** run the realistic config at the **true
deployment window (196608)** — they are the check that improvements found at the scaled window actually
transfer to the regime the runtime ships in. (Concurrency caveat: the server runs `-np 4` with unified
KV; running several admissibility/eval runs at once shares that 196608-token pool, so either run
favorable/locked-window checks with lower concurrency or size tasks to a fraction of the window.)

### Locked real-commit tasks

For the locked set: check out a challenging commit's **parent** into an isolated eval workspace, move
its test files to the sealed location, **scrub git** (drop `.git`/history so the model cannot read the
solution diff or `git log`), and write a natural-language prompt — never "implement commit `abc123`".
The hidden tests come free from the commit.

### Baseline and task-list shape

Record **champion v0**: run the eval N× with the *unmodified* context-management config and store the
baseline pass-rate + median tokens into `program.md`. Each task is **one entry in a task list** from day
one, so growing the suite is a config change, not a harness rewrite.

## train.md — the per-iteration loop

Each iteration the agent:

1. Reads `program.md` (champion config + everything already tried — never retry a logged dead end).
2. Forms **one** mechanism-level hypothesis, e.g. "tokens are high because compaction fires too late
   at 0.85 and large resolved sub-tasks sit in-window."
3. Makes **one** targeted change:
   - **Tier A (params):** edit the candidate config. No rebuild.
   - **Tier B (code):** rewrite `curated.rs` / `offload_policy.rs` / `compactor.rs` logic, then
     `cargo build`. Unlocked only after the signal is proven on Tier A.
4. Runs the eval **N = 5–8×** on the **training** set.
5. Applies the gate (next section).
6. Promotes to champion iff gated-better; **appends the hypothesis + N raw results + verdict to
   `program.md` either way** (failures included, so they are not retried).
7. Stops on budget exhaustion or K consecutive non-improvements.

## Reward, noise, and overfitting

### Noise handling

- **N = 5–8 runs per candidate**, gate on a statistic, never a single run.
- **Paired evaluation.** When comparing candidate vs champion, run them **back-to-back in the same
  batch** under correlated server conditions and compare the **delta**, not absolute numbers. Shared
  noise (server warmth, slot contention) cancels — directly attacking overfit-to-noise (promoting a
  candidate that won the median by luck).

### Overfitting defense — tasks organized by failure *mode*

Each task is tagged with the context-mgmt mechanism it stresses:

- `drift` — long horizon; re-grounding keeps it on goal.
- `offload` — huge tool outputs create offload pressure.
- `compaction` — many resolved sub-tasks; compaction must fire well.
- `recall` — an exact offloaded value is needed verbatim later.

Held-out tasks that are merely *more drift tasks* prove nothing. Generalization is only real if a change
tuned on some modes is shown not to wreck another mode.

### Three tiers

- **Training set** — weakness-first synthetic tasks; agent sees these scores every iteration and
  optimizes against them.
- **Held-out set** — weakness-first synthetic tasks targeting *different weaknesses/modes the agent
  never tunes against*, with different instances; used only as a promotion gate.
- **Locked set** — **real past commits**, run **once** at campaign end vs the v0 baseline for the honest
  generalization report. Authored by a different process than training (real commits, not synthesized
  around a known weakness) precisely so they are independent evidence. Never fed back into the loop; the
  moment you tune to pass it, it stops being held out.

The circularity this guards against: a loop that synthesizes a task around a weakness and then fixes
that weakness has only proven it can fix weaknesses it designed for. Held-out targets *different*
weaknesses; locked uses an *independent source* (real commits). "Champion beats v0 on real commits it
was never tuned toward" is an honest generalization claim; "champion beats tasks built to be fixable" is
not.

### The generalization principle: *train for cheapness, hold out for correctness*

This extends "correctness gates, tokens tiebreak" to generalization:

- **Token improvement** need only appear on the **training** set.
- **Held-out is a hard gate on pass-rate**, advisory on tokens. A promotion is **rejected if it
  regresses any individual held-out task's pass-rate**. This blocks the "rob correctness on mode B to
  make mode A cheaper" trade, and pass-rate is binary/robust so held-out never promotes on token noise.

### Staged gating keeps held-out cheap

Held-out (more tasks × N runs) runs **rarely**: a candidate must first beat the champion on the
**training** set — most candidates die there on the correctness gate, cheaply — and **only
training-winners pay for held-out runs**.

### No-regression-on-any-task rule

A promotion must not regress *any individual* training or held-out task's pass-rate, not merely the
aggregate. This prevents trading one mode's correctness for another's cheapness within the suite.

## program.md — accumulated learnings + champion

Append-only research log plus a champion block. Holds:

- Champion config and its baseline stats (pass-rate + median tokens per task).
- A dated entry per hypothesis: the change, the N raw results, the gate verdict, kept/rejected.

It is the persistent memory that lets the loop compound across sessions instead of re-deriving from
scratch.

## Staged rollout

1. **v0:** skill + `eval_context` harness + 1 weakness-first `drift` training task, admitted via the
   two-sided test. Get the loop working end-to-end, prove the signal on Tier-A params, paired eval in
   place.
2. **Before trusting any accepted change:** synthesize 1–2 training tasks of other weaknesses/modes, 1–2
   held-out tasks (different weaknesses), and lock 1–2 **real-commit** tasks.
3. **Unlock Tier B** (code rewrites) once Tier A has demonstrably moved the metric under the full gate.

The honest success criterion of the whole skill: **the champion beats v0 on the locked set it never
saw.** That line in `program.md` is what "we actually improved the context manager" means.

## Decisions locked during design

- Oracle: test oracle (red→green on the commit's frozen tests), context-management-bound task.
- Tests are **hidden** from the agent; git is **scrubbed**; the task prompt is natural language.
- Mutation is **tiered**: params first, code second.
- Noise: **N=5–8**, correctness gates, tokens tiebreak, **paired** candidate-vs-champion eval.
- Overfitting: **three tiers** (training/held-out/locked), held-out is a **hard pass-rate gate**, locked
  is reported once at the end.
- Tasks: training/held-out are **weakness-first synthetic** tasks (TDD-style, red before admitted);
  locked are **real commits** (independent source). Admissibility is **two-sided** (red under realistic
  config AND green under favorable config) and **correctness-only**.
- **Favorable config** = curation neutralized: `context_limit` at the server's full window (196608),
  `high_water_pct = 1.0`, offload thresholds at `usize::MAX`, re-grounding on. Subject to the
  **task-size invariant** (uncurated transcript fits the window with headroom). Synthetic tasks use a
  **scaled-down realistic window**; locked real-commit tasks run at the **true deployment window** to
  confirm transfer.
- Optimizer is **agent-driven** (autoresearch-style reasoning loop), not automated search.
- Token metric uses **server-reported usage**, not the internal estimate.

## Risks and open items

- **Cost.** Each iteration is N minutes-long live runs; full suite multiplies this. Mitigated by staged
  gating (held-out only for training-winners) and the server's parallel slots.
- **Coarse pass-rate.** N=5–8 binary outcomes make pass-rate granular (e.g. 5/6); fine for gating, but a
  single-run pass-rate swing should not alone promote.
- **Tier-B build cost.** Code rewrites recompile; iterations slow. Acceptable given Tier-B is unlocked
  only after Tier-A proves the signal.
- **Task authorship cost.** The two-sided admissibility test resolves the *discriminating-power* risk
  (a task with no demonstrated red, or one that is capability-bound, is never admitted). The residual
  cost is that each admissibility check is itself 2 configs × N live runs, and synthetic tasks need
  hand-authored hidden tests. Accepted: this is paid once per task at authoring time, not per iteration.
- **Weakness-hypothesis quality.** The loop can only synthesize tasks for weaknesses the authoring agent
  can articulate; blind spots in the analysis become blind spots in the suite. The locked real-commit
  set is the backstop — it can surface regressions on weaknesses no one thought to synthesize.

## File-layout decisions

- Skill lives at `.agents/skills/context-evolve/` (alongside the existing `context-management` skill).
- Harness is a **test-binary** (`agent-runtime-config/tests/eval_context.rs`), reusing the established
  `soak_live` live-server + `SafeApproval` + temp-workspace pattern rather than a standalone CLI.
