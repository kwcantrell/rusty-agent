# eval.md — harness eval co-design

**Goal: isolate the harness's contribution from the model's (the "conflation
problem," arXiv 2503.16416).** A score gain means nothing if you can't say whether
the harness or the model earned it. Hold the model fixed; vary only the harness.

- **Outcome vs process scoring.** Outcome = did the final state pass? Process =
  did the trajectory take the right steps? Harness quality shows up in *process*.
- **Reference-based trajectory comparison** against a gold trajectory: exact /
  partial / unordered / subset match, chosen to fit how strict the step order is.
- **The agentic eval loop:** a while-loop wrapping alternating LLM + tool calls,
  driving a frozen task to termination under one harness config, emitting a
  machine-checkable result line.
- **Correctness-gated / token-tiebreak promotion:** correctness is a hard gate;
  among correctness-preserving configs, prefer fewer total tokens. Never trade a
  pass for tokens.

**Living instance:** `.agents/skills/context-evolve/` already runs exactly this
loop (`eval_context` → RunResult lines → `eval_gate`, correctness-gated /
token-tiebreak) on the context subsystem. Read it as a concrete example — but this
playbook stands alone and applies to any harness component.

---

## 1. The conflation problem

A benchmark score conflates two separate variables: the model's inherent capability
and the harness's scaffolding. If both change between runs, you learn nothing about
either. The discipline is:

> **Hold the model fixed. Vary only the harness.**

Concretely: pin the model name and sampling params (temperature, top_p, seed where
supported). Pin the frozen task set. Change exactly one harness variable at a time —
a prompt edit, a config param, a routing rule, a tool description. This lets you
attribute a result delta to the harness change alone.

Source: arXiv 2503.16416 (Survey on Evaluation of LLM-based Agents). The shared
principles for *which harness variables to vary* live in `SKILL.md` — do not restate
them here.

---

## 2. Outcome vs process scoring

| Scoring mode | What it measures | What shows up |
|---|---|---|
| **Outcome** | Did the final state satisfy the task spec? | Model capability — can it finish? |
| **Process** | Did the trajectory take the intended steps? | Harness quality — did scaffolding steer well? |

Use **both**:

- **Outcome** is the hard gate (pass/fail). It tells you whether the task was
  completed. It is necessary but not sufficient — a model can stumble to the right
  answer via an inefficient, unreliable trajectory.
- **Process** is where harness improvements show up. A tighter prompt, better tool
  descriptions, or a smarter context window will reduce wasted steps and loops even
  when the outcome was already passing.

Practical implication: always record the full trajectory (tool calls, retries,
context reloads) alongside the pass/fail bit. A config that passes in two turns beats
one that passes in nine — and you need the trajectory data to see that.

---

## 3. Reference-based trajectory comparison

Compare the agent's actual trajectory against a **gold trajectory** (the minimal
correct step sequence, authored once when the task is added). Choose the match
strictness to fit the task:

| Match type | When to use | Example |
|---|---|---|
| **Exact** | Step order and content both matter | Security-critical sequences: check → confirm → execute |
| **Partial** | Some key steps required; others optional | Must read config before writing; other reads are fine |
| **Unordered** | Steps all required but order irrelevant | Parallel tool fetches where any order is correct |
| **Subset** | A required subset of gold steps is a pass | Task allows multiple valid paths; gold is one |

Tighter match types (exact) reduce false positives but may over-penalise valid
alternative orderings. Start with subset/partial match and tighten only when you need
to enforce ordering constraints that the task genuinely requires.

Record the gold trajectory in a machine-readable format (e.g. a JSON list of
`{tool, args_schema}` tuples) so the comparator can run without human judgment on
each eval.

---

## 4. The agentic eval loop

The eval harness drives a **frozen task** to termination under a single harness
config, then emits a machine-checkable result line. Pseudo-structure:

```
fn eval_one(task, config) -> RunResult:
    state = task.initial_state()
    turns = 0
    while not state.is_terminal() and turns < MAX_TURNS:
        response = model.complete(context(state, config))   # LLM call (config-fixed)
        tool_calls = parse_tool_calls(response)
        for call in tool_calls:
            result = execute_tool(call, config)             # tool call (config-fixed)
            state = state.apply(call, result)
        turns += 1
    return RunResult {
        passed: task.check(state),
        tokens: total_tokens_from_server_usage(),
        turns: turns,
    }
```

Key discipline:
- **Freeze the task.** Never change the task spec mid-campaign; add new tasks for
  new scenarios.
- **Use server-reported token counts.** Client-side estimates undercount reasoning
  tokens and tool-call overhead. Use the `usage.prompt_tokens +
  usage.completion_tokens` fields the server returns.
- **Emit one result line per run** in a machine-readable format (JSON is
  conventional). This lets `eval_gate` batch across runs without human aggregation.

---

## 5. Correctness-gated / token-tiebreak promotion

The objective is **lexicographic**:

1. **Correctness is a hard gate.** A change that drops the pass count on the
   training set is rejected immediately, regardless of token savings.
2. **Tokens are a tiebreaker.** Among changes that preserve correctness, prefer
   the one with lower median tokens (passing runs only). Never trade a pass for
   tokens.
3. **Held-out gate.** A promotion must not regress any held-out task's pass rate.
   Run the held-out set once at campaign end, not on every iteration.

When to promote:
- `new_pass_count >= champion_pass_count` — correctness preserved
- `median_tokens(new, passing) < median_tokens(champion, passing)` — tokens lower

When to reject:
- `new_pass_count < champion_pass_count` — reject unconditionally
- `new_pass_count == champion_pass_count` and tokens are equal or higher — hold
  the champion; the change has no measurable benefit

Record every run and every verdict in an append-only log (e.g. `program.md` in the
campaign skill). This prevents re-testing a rejected hypothesis and builds a change
history that explains the current champion.

---

## Relationship to SKILL.md

The *why* (the conflation principle, the 90/10 harness/model split, primary sources)
lives in `SKILL.md`. This playbook is the *how* — the concrete eval procedure. Read
both; do not restate `SKILL.md` content here.
