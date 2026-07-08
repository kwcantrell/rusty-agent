# train.md — the per-iteration optimization loop

One iteration = one hypothesis, tested under the gate. Repeat until budget is spent or
K=6 consecutive iterations fail to improve.

## Each iteration

1. **Read `program.md`.** Note the current champion config and every hypothesis already
   tried — **never retry a logged dead end.**
2. **Form ONE mechanism-level hypothesis.** Say *why* tokens are high or the model
   drifts, in terms of the machinery. Examples:
   - "Compaction fires too late (0.85); large resolved sub-tasks sit in-window costing
     prompt tokens every turn." → lower `high_water_pct`.
   - "Offload threshold (1024 B) leaves medium tool outputs in-window." → lower
     `output_min_bytes`.
   - "Auto-recall injects 5 memories at 512 tok every turn but only 1 is used." → lower
     `default_k` / raise `relevance_threshold`.
3. **Make ONE change.**
   - **Tier A:** copy the champion JSON to `cand.json` and edit the one field. No rebuild.
   - **Tier B:** edit the curation code, then
     `cd agent && cargo build -p agent-runtime-config --tests`.
4. **Eval N=5–8, paired.** Run the candidate AND re-run the champion the **same N**
   back-to-back (so shared server noise cancels):

   ```bash
   cd agent
   T=/home/kalen/rust-agent-runtime/.agents/skills/context-evolve/tasks/drift-ledger
   run() { EVAL_REAL_EMBEDDINGS=1 AGENT_E2E_URL=http://localhost:8080 AGENT_E2E_MODEL=qwen3.6-35b-a3b \
     TASK_JSON=$T/task.json CONFIG_JSON="$1" HIDDEN_TESTS_DIR=$T/hidden_tests \
     cargo test -p agent-runtime-config --test eval_context -- --ignored --nocapture 2>&1 \
     | grep -E '^\{"passed"'; }
   # EVAL_REAL_EMBEDDINGS=1 is part of every memory-task ceiling's (config, rate)
   # pair: without it eval_context wires the exact-match StubEmbedder and recall
   # deterministically zero-hits (memory-recall 0/5 vs 5/5, 2026-07-07 sweep
   # incident). Harmless for memory-disabled tasks — keep it on unconditionally.
   : > /tmp/champ.jsonl; : > /tmp/cand.jsonl
   for i in $(seq 1 6); do run /tmp/champion.json >> /tmp/champ.jsonl; run /tmp/cand.json >> /tmp/cand.jsonl; done
   ```

5. **Gate.**
   ```bash
   cargo run -q -p agent-runtime-config --bin eval_gate -- gate /tmp/champ.jsonl /tmp/cand.jsonl
   ```
   - `Promote` → the candidate did not lower the pass count and reduced median tokens.
     For Tier-B or structural changes, ALSO run the held-out tasks and confirm no
     per-task pass-rate regression before promoting.
   - `Reject: ...` → discard the change.
6. **Promote or not, append to `program.md`:** the hypothesis, the change, both JSONL
   batches (or their pass-rate + median), and the verdict. On promote, replace
   `/tmp/champion.json` with `/tmp/cand.json` and update the champion block.

## Tiering

Warm up on Tier-A params until you trust the signal and have a sense of the
sensitivity surface. Then unlock Tier-B code rewrites — that is where the larger,
non-obvious wins live (new offload heuristics, smarter compaction, better memory ranking).

## Stopping

Stop after K=6 consecutive non-improvements or when the token budget is exhausted.
Then run the **locked real-commit set once** (`prepare.md`) and record whether the
champion beats v0 on tasks it was never tuned toward — the honest success metric.
