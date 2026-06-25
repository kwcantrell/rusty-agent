use serde::{Deserialize, Serialize};

/// The outcome of one eval run: did the hidden tests pass, and how many total
/// tokens did the model process (sum of server prompt+completion over all turns).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub passed: bool,
    pub tokens: u64,
    pub turns: usize,
}

/// N runs of one config on one task.
#[derive(Debug, Clone, Default)]
pub struct BatchResult {
    pub runs: Vec<RunResult>,
}

impl BatchResult {
    pub fn passes(&self) -> usize {
        self.runs.iter().filter(|r| r.passed).count()
    }

    pub fn pass_rate(&self) -> f64 {
        if self.runs.is_empty() {
            return 0.0;
        }
        self.passes() as f64 / self.runs.len() as f64
    }

    /// Median token count over passing runs only — failed runs are not comparable
    /// (a run that gave up early is "cheap" but worthless).
    pub fn median_tokens_passing(&self) -> Option<u64> {
        let mut v: Vec<u64> = self.runs.iter().filter(|r| r.passed).map(|r| r.tokens).collect();
        if v.is_empty() {
            return None;
        }
        v.sort_unstable();
        Some(v[v.len() / 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rr(passed: bool, tokens: u64) -> RunResult {
        RunResult { passed, tokens, turns: 1 }
    }
    #[test]
    fn median_uses_only_passing_runs() {
        let b = BatchResult { runs: vec![rr(true, 100), rr(false, 1), rr(true, 300), rr(true, 200)] };
        assert_eq!(b.passes(), 3);
        assert!((b.pass_rate() - 0.75).abs() < 1e-9);
        assert_eq!(b.median_tokens_passing(), Some(200)); // median of {100,200,300}
    }
    #[test]
    fn no_passing_runs_has_no_median() {
        let b = BatchResult { runs: vec![rr(false, 5), rr(false, 9)] };
        assert_eq!(b.median_tokens_passing(), None);
        assert_eq!(b.pass_rate(), 0.0);
    }
}
