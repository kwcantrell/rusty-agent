use crate::eval::result::BatchResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admissibility {
    /// Red under realistic config AND green under favorable: a capturable weakness.
    Admitted,
    /// Favorable run overflowed the window — the transcript doesn't fit; re-size.
    IllSized,
    /// Even favorable fails: the model can't do it regardless of context. Discard.
    CapabilityBound,
    /// Realistic already passes: there's nothing for the loop to capture. Discard.
    NoWeakness,
}

/// Favorable must reliably pass; realistic must reliably fail. Thresholds: favorable
/// pass-rate >= 0.8 ("the model can do it given ideal context"), realistic pass-rate
/// < 0.5 ("the weakness bites a majority of the time").
const FAVORABLE_MIN: f64 = 0.8;
const REALISTIC_MAX: f64 = 0.5;

pub fn admit(
    favorable: &BatchResult,
    realistic: &BatchResult,
    favorable_overflowed: bool,
) -> Admissibility {
    if favorable_overflowed {
        return Admissibility::IllSized;
    }
    if favorable.pass_rate() < FAVORABLE_MIN {
        return Admissibility::CapabilityBound;
    }
    if realistic.pass_rate() >= REALISTIC_MAX {
        return Admissibility::NoWeakness;
    }
    Admissibility::Admitted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::result::{BatchResult, RunResult};
    fn batch(passes: usize, n: usize) -> BatchResult {
        BatchResult {
            runs: (0..n)
                .map(|i| RunResult {
                    passed: i < passes,
                    tokens: 1,
                    turns: 1,
                })
                .collect(),
        }
    }
    #[test]
    fn admits_when_favorable_passes_and_realistic_fails() {
        // favorable 5/5 green, realistic 1/5 red -> a real, capturable weakness
        assert_eq!(
            admit(&batch(5, 5), &batch(1, 5), false),
            Admissibility::Admitted
        );
    }
    #[test]
    fn ill_sized_when_favorable_overflowed() {
        assert_eq!(
            admit(&batch(5, 5), &batch(0, 5), true),
            Admissibility::IllSized
        );
    }
    #[test]
    fn capability_bound_when_favorable_also_fails() {
        assert_eq!(
            admit(&batch(1, 5), &batch(0, 5), false),
            Admissibility::CapabilityBound
        );
    }
    #[test]
    fn no_weakness_when_realistic_already_passes() {
        assert_eq!(
            admit(&batch(5, 5), &batch(4, 5), false),
            Admissibility::NoWeakness
        );
    }
}
