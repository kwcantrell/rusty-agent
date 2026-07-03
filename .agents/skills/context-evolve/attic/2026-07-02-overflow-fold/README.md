# 2026-07-02 overflow-fold round — REVERTED, kept for the record

Seven variants of "fold overflow user turns instead of silently evicting them",
all 0/5 on longhaul-manifest; the final tree regressed locked-portmap 10/10→1/6
and drift-ledger 6/6→1/6 on the guard sweep and was fully reverted. Full story:
program.md, "Iteration log (Tier-B — overflow-user folding, post-v3)".

- `patch_maintain_at_start.diff` — loop_.rs: move maintain() to start-of-turn
  (fixes the maintain-skipped-on-text-only-turns gap; re-baselines everything).
- `patch_fold_guard_curated.diff` — curated.rs/context.rs: fold_overflow_users,
  consolidated in-history marker, MIN_RECOMPACTION_SPAN_TOKENS guard, tests.

Do not re-apply blind: each piece individually changed eval behavior. Any future
attempt needs its own spec + full re-baseline of every admitted task.
