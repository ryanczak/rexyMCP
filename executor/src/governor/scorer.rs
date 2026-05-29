use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCounts {
    pub successes: u32,
    pub failures: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scorer {
    pub counts: HashMap<String, ToolCounts>,
}

impl Scorer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, tool: &str, succeeded: bool) {
        let entry = self.counts.entry(tool.to_string()).or_default();
        if succeeded {
            entry.successes = entry.successes.saturating_add(1);
        } else {
            entry.failures = entry.failures.saturating_add(1);
        }
    }

    pub fn score(&self, tool: &str) -> f64 {
        let counts = self.counts.get(tool).cloned().unwrap_or_default();
        let successes = counts.successes as f64;
        let failures = counts.failures as f64;
        (successes + 1.0) / (successes + failures + 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_scorer_is_empty() {
        assert!(Scorer::new().counts.is_empty());
    }

    #[test]
    fn record_increments_successes() {
        let mut scorer = Scorer::new();
        scorer.record("foo", true);
        scorer.record("foo", true);
        scorer.record("foo", true);
        let counts = scorer.counts.get("foo").unwrap();
        assert_eq!(counts.successes, 3);
        assert_eq!(counts.failures, 0);
    }

    #[test]
    fn record_increments_failures() {
        let mut scorer = Scorer::new();
        scorer.record("foo", false);
        scorer.record("foo", false);
        let counts = scorer.counts.get("foo").unwrap();
        assert_eq!(counts.successes, 0);
        assert_eq!(counts.failures, 2);
    }

    #[test]
    fn score_unobserved_returns_half() {
        let scorer = Scorer::new();
        assert!((scorer.score("never_used") - 0.5).abs() < 1e-9);
    }

    #[test]
    fn score_matches_laplace_formula() {
        let mut scorer = Scorer::new();
        scorer.record("foo", true);
        scorer.record("foo", true);
        scorer.record("foo", true);
        scorer.record("foo", false);
        let s = scorer.score("foo");
        let expected = (3.0 + 1.0) / (3.0 + 1.0 + 2.0);
        assert!((s - expected).abs() < 1e-9, "expected {expected}, got {s}");
    }

    #[test]
    fn score_pure_successes_approaches_one_but_doesnt_reach() {
        let mut scorer = Scorer::new();
        for _ in 0..100 {
            scorer.record("foo", true);
        }
        let s = scorer.score("foo");
        assert!(s > 0.95, "score should be > 0.95, got {s}");
        assert!(s < 1.0, "score should be < 1.0, got {s}");
    }
}
