use serde::{Deserialize, Serialize};

/// One tool invocation observed during an eval run (ToolStart order).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrajectoryStep {
    pub tool: String,
    pub args: serde_json::Value,
}

/// The outcome of one eval run: did the hidden tests pass, and how many total
/// tokens did the model process (sum of server prompt+completion over all turns).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub passed: bool,
    pub tokens: u64,
    pub turns: usize,
    /// Ordered ToolStart capture (diagnostic; additive — old lines parse).
    #[serde(default)]
    pub trajectory: Vec<TrajectoryStep>,
    /// SafeApproval denials during the run (silent friction made visible).
    #[serde(default)]
    pub denials: usize,
    /// Some(matched) when the task defines a non-empty gold_trajectory.
    #[serde(default)]
    pub gold_matched: Option<bool>,
}

/// True iff `gold` (tool names) appears as an ordered subsequence of the
/// trajectory's tool names. Extra calls in between are allowed; order is not.
/// Empty gold is vacuously true. Diagnostic only — the promotion gate does not
/// consume it (spec 2026-07-02 eval-flywheel).
pub fn trajectory_matches_gold(trajectory: &[TrajectoryStep], gold: &[String]) -> bool {
    let mut want = gold.iter();
    let mut next = want.next();
    for s in trajectory {
        if let Some(g) = next {
            if &s.tool == g {
                next = want.next();
            }
        }
    }
    next.is_none()
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
        let mut v: Vec<u64> = self
            .runs
            .iter()
            .filter(|r| r.passed)
            .map(|r| r.tokens)
            .collect();
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
        RunResult {
            passed,
            tokens,
            turns: 1,
            trajectory: Vec::new(),
            denials: 0,
            gold_matched: None,
        }
    }
    fn step(tool: &str) -> TrajectoryStep {
        TrajectoryStep {
            tool: tool.into(),
            args: serde_json::json!({}),
        }
    }
    fn gold(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn gold_subsequence_semantics() {
        let traj: Vec<_> = ["read_file", "grep_x", "edit_file", "cargo_test"]
            .iter()
            .map(|t| step(t))
            .collect();
        assert!(trajectory_matches_gold(
            &traj,
            &gold(&["read_file", "edit_file", "cargo_test"])
        )); // extras ok
        assert!(trajectory_matches_gold(&traj, &gold(&[]))); // empty gold vacuous
        assert!(!trajectory_matches_gold(
            &traj,
            &gold(&["edit_file", "read_file"])
        )); // order violated
        assert!(!trajectory_matches_gold(&traj, &gold(&["write_file"]))); // missing tool
        assert!(!trajectory_matches_gold(&[], &gold(&["read_file"]))); // empty traj, non-empty gold

        // Duplicate gold names each need a distinct trajectory step (subsequence, not set).
        assert!(!trajectory_matches_gold(&[step("a")], &gold(&["a", "a"]))); // one "a" can't cover two
        assert!(trajectory_matches_gold(
            &[step("a"), step("b"), step("a")],
            &gold(&["a", "a"])
        )); // two "a"s, order preserved
    }

    #[test]
    fn run_result_old_json_still_parses() {
        let old = r#"{"passed":true,"tokens":123,"turns":4}"#;
        let r: RunResult = serde_json::from_str(old).unwrap();
        assert!(r.trajectory.is_empty());
        assert_eq!(r.denials, 0);
        assert_eq!(r.gold_matched, None);
    }
    #[test]
    fn median_uses_only_passing_runs() {
        let b = BatchResult {
            runs: vec![rr(true, 100), rr(false, 1), rr(true, 300), rr(true, 200)],
        };
        assert_eq!(b.passes(), 3);
        assert!((b.pass_rate() - 0.75).abs() < 1e-9);
        assert_eq!(b.median_tokens_passing(), Some(200)); // median of {100,200,300}
    }
    #[test]
    fn no_passing_runs_has_no_median() {
        let b = BatchResult {
            runs: vec![rr(false, 5), rr(false, 9)],
        };
        assert_eq!(b.median_tokens_passing(), None);
        assert_eq!(b.pass_rate(), 0.0);
    }
}
