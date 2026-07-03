# H6b — does the model spontaneously load a skill's worked EXAMPLE?

A measurement experiment. It decides whether **H6b** — auto-injecting a skill's
worked example into context when a task matches — is worth building, or whether the
model already loads examples on its own initiative when it needs them.

## The question

Some skills ship a strict, guessable-wrong convention that lives **only** in a
worked example (`examples/*.md`), not in `SKILL.md`. To do the task correctly the
model must, unprompted:

1. `list_skills` (notice the skill is example-bearing — marked `[N examples]`),
2. `use_skill` (see the rendered Examples section), and
3. `read_skill_file` the exemplar and imitate it.

If the model reliably does this on its own, H6b (auto-injection) is redundant. If it
doesn't, H6b earns its complexity.

## The skill under test

`skills/csv-report/` — "Generate the team's standard summary report from a CSV
file." `SKILL.md` says a house format is mandatory but **deliberately does not state
it**; the exact format lives only in `examples/report-format.md`:

- first line exactly `# Report: <dataset-name>`
- a `| metric | value |` markdown table
- every value rounded to **exactly 2 decimals**
- a final line exactly `TOTAL: <sum-2dp>`

The hidden test greps `report.md` for those strict markers, so **passing requires
having read the example**.

## Two arms

| arm | prompt | skill-load needed? | `gold_trajectory` |
|-----|--------|--------------------|-------------------|
| **A — model-initiative** (`tasks/model-initiative/`) | "Follow the csv-report skill." Format NOT given. | yes — must load the example | `[list_skills, use_skill, read_skill_file]` |
| **B — example-injected** (`tasks/example-injected/`) | Format rules pasted inline (simulated H6b). | no | `[]` (empty) |

Both share `data.csv` and the same strict hidden test. Arm B is the **control**: it
measures the pass ceiling *if the example content were injected for you*. Arm A
measures what unaided model initiative actually achieves.

The context window is generous on both arms (196608) with memory off — context is
**not** the variable here. The only question is whether the example gets loaded.

## Metrics

For each arm, over N runs:

- **trigger-rate** = fraction of runs where `gold_matched == true` (arm A only —
  arm B's gold is empty so it always "matches"). This is the *process* signal: did
  the model load the skill/example subsequence at all?
- **pass-rate** = fraction of runs that passed the hidden test. This is the
  *outcome* signal: did the strict format actually come out right?

## Decision rule for H6b

- **Build H6b (auto-inject examples)** if, on the example-necessary arm A,
  **trigger-rate < ~0.6** AND arm A **pass-rate is materially below arm B**
  (control) pass-rate. That is direct evidence that model initiative under-triggers
  and that supplying the example closes a real outcome gap.
- **Defer / decline H6b** if arm A **trigger-rate >= ~0.8**, OR arm A pass-rate
  matches arm B's. Then model initiative already suffices and auto-injection buys
  nothing but context cost.
- The middle band (0.6–0.8 trigger, partial gap) is "weak evidence — gather more N
  before committing."

The comparison that matters is **A vs B**, not A's absolute pass-rate: arm B bounds
what's achievable when the format is known, isolating the example-loading step as
the only difference.

## Running

```bash
# from repo root; live model server must be up at $AGENT_E2E_URL
bash docs/superpowers/experiments/h6b-example-triggering/run.sh 3 model-initiative
bash docs/superpowers/experiments/h6b-example-triggering/run.sh 3 example-injected
```

`run.sh` sets `AGENT_E2E_URL` (default `http://localhost:8080` — **not** `/v1`; the
client appends `/v1/chat/completions` itself),
`AGENT_E2E_MODEL` (default `qwen3.6-35b-a3b`), `SKILLS_DIR` (this package's
`skills/`), `TASK_JSON`, `CONFIG_JSON`, `HIDDEN_TESTS_DIR`, runs the `#[ignore]`
`eval_context_run` test N times, parses the single stdout `RunResult` JSON line per
run, and prints the two rates.

The `SKILLS_DIR` env hook is an additive line in
`agent/crates/agent-runtime-config/tests/eval_context.rs` (the harness otherwise
never sets `cfg.skills_dirs`, so `list_skills` would return an empty catalog).

## Caveats

- **The eval harness pins the tool-result ingestion cap OFF**
  (`max_result_bytes = usize::MAX` in `eval/config.rs`), so example-aware tasks run
  under a no-cap ingestion regime. A production H6b decision must re-check behavior
  with the cap on; here the example content is small so it is not a confound, but
  watch it for larger exemplars.
- This package ships with a **smoke run at small N** (see the task report), which is
  **directional only, not a verdict**. Full-N execution and the actual H6b
  build/decline decision belong to the context-evolve campaign.
- Trigger-rate depends on the model tool-calling reliably. A run that errors or
  fails to emit a `RunResult` line is reported as such, never counted as a pass.
