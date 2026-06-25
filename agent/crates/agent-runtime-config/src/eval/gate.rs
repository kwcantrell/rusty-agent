use crate::eval::result::BatchResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Promote,
    Reject { reason: String },
}

/// Lexicographic: correctness is a hard gate, tokens are the tiebreaker. A
/// candidate is promoted only if it does not lower the pass count AND strictly
/// reduces the median tokens among passing runs.
pub fn gate(champion: &BatchResult, candidate: &BatchResult) -> Verdict {
    if candidate.passes() < champion.passes() {
        return Verdict::Reject {
            reason: format!(
                "correctness regressed: {} < {} passes",
                candidate.passes(),
                champion.passes()
            ),
        };
    }
    match (candidate.median_tokens_passing(), champion.median_tokens_passing()) {
        (Some(cand), Some(champ)) if cand < champ => Verdict::Promote,
        (Some(cand), Some(champ)) => Verdict::Reject {
            reason: format!("tokens not improved: {cand} >= {champ}"),
        },
        _ => Verdict::Reject { reason: "no passing runs to compare tokens".into() },
    }
}

/// Held-out is a hard pass-rate gate: a promotion is rejected if it regresses ANY
/// individual held-out task's pass rate. Tokens on held-out are advisory and not
/// checked here. `champion`/`candidate` are aligned per-task (same order).
pub fn heldout_ok(champion: &[BatchResult], candidate: &[BatchResult]) -> bool {
    champion.len() == candidate.len()
        && champion.iter().zip(candidate).all(|(c, n)| n.pass_rate() >= c.pass_rate())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::result::{BatchResult, RunResult};
    fn batch(spec: &[(bool, u64)]) -> BatchResult {
        BatchResult {
            runs: spec.iter().map(|&(passed, tokens)| RunResult { passed, tokens, turns: 1 }).collect(),
        }
    }
    #[test]
    fn rejects_when_correctness_regresses() {
        let champ = batch(&[(true, 500), (true, 500), (true, 500)]);
        let cand = batch(&[(true, 100), (false, 1), (true, 100)]); // fewer passes
        assert!(matches!(gate(&champ, &cand), Verdict::Reject { .. }));
    }
    #[test]
    fn promotes_when_correctness_holds_and_tokens_drop() {
        let champ = batch(&[(true, 500), (true, 500), (true, 500)]);
        let cand = batch(&[(true, 300), (true, 300), (true, 300)]);
        assert!(matches!(gate(&champ, &cand), Verdict::Promote));
    }
    #[test]
    fn rejects_when_tokens_not_better() {
        let champ = batch(&[(true, 300), (true, 300), (true, 300)]);
        let cand = batch(&[(true, 300), (true, 300), (true, 300)]);
        assert!(matches!(gate(&champ, &cand), Verdict::Reject { .. }));
    }
    #[test]
    fn heldout_blocks_any_pass_rate_regression() {
        let champ = vec![batch(&[(true, 9), (true, 9)]), batch(&[(true, 9), (true, 9)])];
        let cand_ok = vec![batch(&[(true, 9), (true, 9)]), batch(&[(true, 9), (true, 9)])];
        let cand_bad = vec![batch(&[(true, 9), (true, 9)]), batch(&[(true, 9), (false, 9)])];
        assert!(heldout_ok(&champ, &cand_ok));
        assert!(!heldout_ok(&champ, &cand_bad));
    }
}
