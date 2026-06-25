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

## prepare.md — freezing a trustworthy task

Given a commit (argument; if none supplied, the agent selects a challenging one):

1. Check out the commit's **parent** into an isolated eval workspace.
2. **Move the test files out** to a sealed location the agent's workspace cannot see (hidden oracle —
   prevents a degenerate run from hardcoding to the assertions and banking a fake "pass").
3. **Scrub git** — drop `.git`/history so the model cannot read the solution diff or `git log`.
4. Write the **task prompt in natural language**, describing the goal in its own words. Never
   "implement commit `abc123`".
5. Record **champion v0**: run the eval N× with the *unmodified* context-management config and store
   the baseline pass-rate + median tokens into `program.md`.

The first task is chosen to stress **drift / re-grounding**: long-horizon, the window fills, and the
re-grounding block is what keeps the agent anchored to the original goal.

Each task is encoded as **one entry in a task list** from day one, so growing to a full
train/held-out/locked suite is a config change, not a harness rewrite.

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

- **Training set** — agent sees these scores every iteration and optimizes against them.
- **Held-out set** — different task *instances* and at least one *mode the agent never tunes against*;
  used only as a promotion gate.
- **Locked set** — run **once** at campaign end vs the v0 baseline for the honest generalization
  report. Never fed back into the loop; the moment you tune to pass it, it stops being held out.

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

1. **v0:** skill + `eval_context` harness + 1 `drift` training task. Get the loop working end-to-end,
   prove the signal on Tier-A params, paired eval in place.
2. **Before trusting any accepted change:** add 1–2 training tasks of other modes, 1–2 held-out tasks
   (at least one a held-out mode), and lock 1–2 tasks.
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
- Optimizer is **agent-driven** (autoresearch-style reasoning loop), not automated search.
- Token metric uses **server-reported usage**, not the internal estimate.

## Risks and open items

- **Cost.** Each iteration is N minutes-long live runs; full suite multiplies this. Mitigated by staged
  gating (held-out only for training-winners) and the server's parallel slots.
- **Coarse pass-rate.** N=5–8 binary outcomes make pass-rate granular (e.g. 5/6); fine for gating, but a
  single-run pass-rate swing should not alone promote.
- **Tier-B build cost.** Code rewrites recompile; iterations slow. Acceptable given Tier-B is unlocked
  only after Tier-A proves the signal.
- **Task authorship.** Hand-picking commits whose difficulty is genuinely *context-management-bound*
  (not knowledge-bound) is itself work; the first `drift` task must be validated to actually fail under
  a deliberately bad config and pass under a good one (otherwise it has no discriminating power).

## File-layout decisions

- Skill lives at `.agents/skills/context-evolve/` (alongside the existing `context-management` skill).
- Harness is a **test-binary** (`agent-runtime-config/tests/eval_context.rs`), reusing the established
  `soak_live` live-server + `SafeApproval` + temp-workspace pattern rather than a standalone CLI.
