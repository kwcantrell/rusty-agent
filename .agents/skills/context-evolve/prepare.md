# prepare.md — author and admit a trustworthy task

A task is only trusted once it is shown, red-first, to be **context-management-bound**
(not capability-bound). Training/held-out tasks are **weakness-first synthetic**;
locked tasks are **real commits**.

## Weakness-first synthesis (training / held-out)

1. **Analyze** the context manager and **hypothesize one weakness** — e.g. "compaction
   drops early instruction turns once the window fills, so a long ledger of small
   updates loses its first entries."
2. **Author a task + hidden tests** that target that weakness, as a `TaskSpec` JSON in
   `tasks/<id>/task.json` plus `tasks/<id>/hidden_tests/` (see `tasks/drift-ledger/`).
   The hidden tests are **never** in the seeded workspace — the harness copies them in
   only for the sealed post-run grading step. The prompts describe the goal in natural
   language; never "implement commit X".
3. **Two-sided admissibility (correctness only)** — build two config JSONs and run N
   each. Admit **only if both hold** across the batch:
   - **Red under realistic config** — the weakness bites; the run fails the hidden tests.
   - **Green under favorable config** — full window, no offload/compaction, generous
     retrieval; the model passes. This proves the failure is context-bound.

## Building the two configs

Favorable = "context manager neutralized" (do not edit between tasks):

```json
{ "context_limit": 196608, "high_water_pct": 1.0, "keep_recent": 4294967295,
  "error_min_bytes": 18446744073709551615, "output_min_bytes": 18446744073709551615,
  "recall_budget": 4096, "memory_enabled": false, "default_k": 20,
  "relevance_threshold": 0.0, "dedup_threshold": 0.95, "forget_threshold": 0.85,
  "max_recall_chars": 65536, "recall_token_budget": 8192, "auto_recall": true }
```

Realistic = the shipping defaults, but **the window (`context_limit`) is what creates
the pressure**. NOTE: large tool outputs are offloaded, so a big workspace file alone
does NOT fill the window — you must shrink `context_limit` until the *history itself*
(instruction turns) forces compaction. For `drift-ledger`, 16000 does NOT discriminate
(model keeps all increments → passes); **4000 does** (early increments get compacted
away → wrong total). Champion v0:

```json
{ "context_limit": 4000, "high_water_pct": 0.85, "keep_recent": 2,
  "error_min_bytes": 200, "output_min_bytes": 1024, "recall_budget": 512,
  "memory_enabled": false, "default_k": 5, "relevance_threshold": 0.3,
  "dedup_threshold": 0.95, "forget_threshold": 0.85, "max_recall_chars": 4096,
  "recall_token_budget": 512, "auto_recall": true }
```

## Running the admissibility check

Always use ABSOLUTE paths (integration tests run with cwd = the crate dir):

```bash
source ~/.cargo/env && cd agent
T=/home/kalen/rust-agent-runtime/.agents/skills/context-evolve/tasks/drift-ledger
run() { AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
  TASK_JSON=$T/task.json CONFIG_JSON="$1" HIDDEN_TESTS_DIR=$T/hidden_tests \
  cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture 2>&1 \
  | grep -E '^\{"passed"'; }
: > /tmp/fav.jsonl;  for i in 1 2 3 4 5; do run /tmp/favorable.json >> /tmp/fav.jsonl;  done
: > /tmp/real.jsonl; for i in 1 2 3 4 5; do run /tmp/champion.json  >> /tmp/real.jsonl; done
cargo run -q -p agent-runtime-config --bin eval_gate -- admit /tmp/fav.jsonl /tmp/real.jsonl
```

Interpreting the verdict:

- `Admitted` — keep the task; record it + both configs in `program.md`.
- `CapabilityBound` — even favorable fails; the task is too hard regardless of context.
  Simplify it.
- `NoWeakness` — realistic already passes; shrink `context_limit` (or lengthen the task)
  until the weakness bites, then re-run.
- `IllSized` — the favorable transcript overflowed the window; shrink the task so its
  full uncurated transcript fits with headroom (target ≤ ~75% of the favorable window).

## Baseline

Once admitted, the realistic config is **champion v0**. Record its pass-rate and median
tokens (passing runs) in `program.md`.

## Locked real-commit tasks

For the end-of-campaign generalization report: check out a challenging commit's
**parent** into the task workspace, move its tests into `hidden_tests/`, scrub `.git`
so the model can't read the solution, and write a natural-language prompt. Run these
**once** at the end, never inside the loop.
