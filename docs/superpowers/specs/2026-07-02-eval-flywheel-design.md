# Eval flywheel — trajectory capture, gold comparator, adversarial policy corpus, coverage in CI

**Date:** 2026-07-02
**Status:** Approved (autonomous backlog-drain run; deep audit Eval-dimension MEDs/LOW)
**Cluster:** 5 of 6 in the 2026-07 residual-backlog drain

## Problem

1. **No process signal in eval results.** `RunResult` is `{passed, tokens, turns}`
   (eval/result.rs) and the gate ignores `turns`; the harness can't score *how* a run
   solved a task (the campaign's open locked-portmap discriminator likely needs exactly
   this — audit: "trajectory/process evals", eval.md playbook).
2. **No gold-trajectory comparator** to express "a correct run uses these tools in this
   order".
3. **Denylist regressions guard the hottest component with 25 individual `#[test]`s**
   (agent-policy command.rs) — every new wrapping trick costs a hand-written test;
   there's no one-line-per-case corpus covering all historically closed bypasses
   end-to-end through the real default lists.
4. **No coverage number anywhere** — agent-cli/render, approval, and sandbox
   docker-strategy thinness is invisible.
5. **`SafeApproval.denied` is accumulated and never read** (eval_context.rs:58-61) —
   silent policy friction in eval runs is invisible.

## Approaches considered

- Trajectory siting: **(chosen)** additive `#[serde(default)] trajectory` on `RunResult`
  captured by the existing eval sink — old JSON lines stay parseable, the gate is
  untouched (trajectory is diagnostic, not gating; gating on process is a campaign
  decision, not a harness default). Alternative — a separate trajectory file per run —
  splits the run artifact in two for no benefit. Rejected.
- Comparator semantics: gold = ordered **subsequence of tool names** (a run may
  interleave extra calls; the gold names must appear in order). Args-matching and
  LM-judge rubrics are heavier process-eval machinery the campaign can add when a task
  needs it (YAGNI).
- Corpus siting: **(chosen)** `agent-runtime-config/tests/` — the only crate that sees
  both the real `default_allowlist()`/`default_denylist()` and the `agent-policy`
  engine (agent-policy can't depend on runtime-config). A TSV corpus file keeps
  new cases one-line additions.
- Coverage: **(chosen)** a separate non-blocking GitHub Actions job — NOT in
  `scripts/ci.sh`, which doubles as the pre-push hook; instrumented rebuilds would tax
  every push. The job reports `cargo llvm-cov --workspace --summary-only`; no threshold
  gate initially (a tracked number first, a ratchet later if wanted).

## Design

### 1. `TrajectoryStep` + `RunResult.trajectory` (eval/result.rs)

```rust
/// One tool invocation observed during an eval run (ToolStart order).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrajectoryStep {
    pub tool: String,
    pub args: serde_json::Value,
}
```

`RunResult` gains `#[serde(default)] pub trajectory: Vec<TrajectoryStep>` and
`#[serde(default)] pub denials: usize`. Both additive-with-default: existing recorded
JSON lines and the `eval_gate` CLI keep parsing.

### 2. Gold comparator (eval/result.rs, alongside BatchResult)

```rust
/// True iff `gold` (tool names) appears as an ordered subsequence of the
/// trajectory's tool names. Extra calls in between are allowed; order is not.
pub fn trajectory_matches_gold(trajectory: &[TrajectoryStep], gold: &[String]) -> bool
```

Empty gold → `true` (vacuous). `TaskSpec` gains
`#[serde(default)] pub gold_trajectory: Vec<String>` (task.rs; additive — frozen task
JSONs without the field keep parsing). `RunResult` gains
`#[serde(default)] pub gold_matched: Option<bool>` — filled by the harness when the
task defines a non-empty gold, `None` otherwise. The gate (gate.rs) is UNCHANGED.

### 3. Harness wiring (tests/eval_context.rs)

- `TokenMeter` (or a sibling recorder registered on the same sink path — the sink is a
  single `Arc`; extend `TokenMeter` with `trajectory: Mutex<Vec<TrajectoryStep>>`)
  captures every `AgentEvent::ToolStart` as `TrajectoryStep { tool: name, args }`.
  Child (sub-agent) ToolStarts, if any ever appear here, are captured too — they carry
  `sub…:` id prefixes but the same name field; fine for a diagnostic.
- After the sealed grading step: `denials = safe_approval.denied.lock().len()`;
  non-empty denial lists are printed to **stderr** (one line per entry, prefixed
  `eval-denied:`) so they never pollute the stdout JSON contract; `gold_matched`
  computed via the comparator when `!task.gold_trajectory.is_empty()`.
- `soak_live.rs` keeps its own copy of SafeApproval untouched (separate concern; its
  denials are not part of the RunResult contract).

### 4. Adversarial policy corpus (agent-runtime-config)

- New `agent/crates/agent-runtime-config/tests/policy_corpus.rs` + data file
  `tests/policy_corpus.tsv` (`expected<TAB>command`, `#` comments, blank lines
  skipped; expected ∈ `allow|ask|deny`).
- The test reproduces the engine's command-decision ladder exactly as production wires
  it: build a `RulePolicy` (or the engine type `assemble_loop` uses) from
  `default_denylist()` + `default_allowlist()`, feed each command as an
  `execute_command` ToolIntent, and map the `Decision` to allow/ask/deny. If the
  engine's API makes intent construction awkward from this crate, fall back to the
  same two-layer check production uses (`hard-floor deny` → `is_auto_allowed` → ask)
  — but prefer the engine path so the corpus also guards the wiring.
- Seed rows (≥30): every historically closed bypass class — `rm -rf /` spacing/flag
  variants, forkbomb, `mkfs` + `mkfs.ext4`, whitespace-smuggled forms, `sudo`/`mkfs`
  in program position vs benign `man mkfs`/`which sudo` (allow-vs-ask per current
  behavior), env-prefix/`$()`/backtick/subshell forms, `find -exec sudo`,
  `git push --force` → ask, `git reset --hard` → ask, `git status`/`log`/`diff` →
  allow, `git log --output=/tmp/x` → ask (cluster 4), `git diff
  --output-indicator-new=+` → allow, `cargo build` → allow, `bash -c "sudo x"` → ask,
  `dd of=/dev/sda` → deny, `echo x > /dev/sda` → ask (redirection asymmetry,
  documented; behavior changed to deny as of the /dev-redirect-denial spec,
  2026-07-02). Expected values are pinned from CURRENT behavior — the corpus is a
  regression net, not a wishlist; rows that document accepted asymmetries carry a
  `#` comment saying so.

### 5. Coverage job (.github/workflows/ci.yml)

Additive second job:

```yaml
  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: llvm-tools-preview }
      - uses: Swatinem/rust-cache@v2
        with: { workspaces: agent }
      - name: Install cargo-llvm-cov
        uses: taiki-e/install-action@cargo-llvm-cov
      - name: Coverage summary
        working-directory: agent
        run: cargo llvm-cov --workspace --summary-only
```

Non-blocking number (job failure only on build/test failure, which the main job
already gates). `scripts/ci.sh` untouched. Local verification of the YAML is
syntactic (the job runs on GitHub); the cargo command is verified locally only if
`cargo-llvm-cov` is already installed — otherwise noted in the report.

## Error handling

- Corpus parser: malformed lines fail the test loudly with line numbers.
- Trajectory capture is lock-append only; no behavior change to the run.

## Testing

1. Unit: comparator table — exact match, subsequence with extras, order violation,
   missing tool, empty gold, empty trajectory + non-empty gold.
2. Unit: `RunResult`/`TaskSpec` serde round-trip WITHOUT the new fields (old JSON
   parses; defaults hold) and with them.
3. Corpus test: all seed rows pass; a deliberately wrong row was flipped during
   development to prove the harness fails loudly (not committed).
4. eval_context.rs compiles and the harness changes are exercised by the ignored
   live-model test only — the additive capture logic gets a small non-ignored unit
   where feasible (comparator + serde cover the pure parts; the sink capture is
   asserted via a direct `TokenMeter.emit(ToolStart{..})` unit if the type is
   test-visible, else noted).

## Out of scope (recorded residuals)

- Gating on trajectory/gold match (campaign decision; gate.rs untouched).
- Args-level or LM-judge trajectory scoring.
- Widening `CandidateConfig` to prompt/protocol/tool variants (product decision).
- Coverage thresholds/ratchets; coverage for `src-tauri` (GTK deps, excluded from CI).
- proptest on the tokenizer (corpus covers the closed-bypass classes).
- Resuming the context-evolve campaign (separate program).
