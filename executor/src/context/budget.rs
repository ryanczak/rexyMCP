//! Context-budget engine. A `Budget` is a thin wrapper over a token
//! ceiling — typically `model.context_length × max_context_pct / 100`.
//! Stateless: every query recomputes from the supplied messages, so callers
//! don't have to keep the engine in sync with session state.

use crate::ai::types::Message;
use crate::context::tokens;

/// A token budget ceiling. Stateless — the ceiling is fixed per session;
/// queries against this budget recompute on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// Maximum total tokens the in-context messages + system prompt +
    /// in-flight user input may consume.
    pub ceiling: usize,
}

impl Default for Budget {
    /// A "never overflows" budget. Sensible for tests that don't care
    /// about budget behavior; production overrides via `from_context`.
    fn default() -> Self {
        Self {
            ceiling: usize::MAX,
        }
    }
}

impl Budget {
    /// Construct with an explicit ceiling.
    pub fn new(ceiling: usize) -> Self {
        Self { ceiling }
    }

    /// Construct from the model's context length and the configured
    /// percentage. Formula: `context_length × max_context_pct / 100`.
    pub fn from_context(context_length: usize, max_context_pct: u8) -> Self {
        let ceiling = context_length.saturating_mul(max_context_pct as usize) / 100;
        Self { ceiling }
    }

    /// Estimate the total tokens for the given system prompt plus the
    /// message history. Recomputes from scratch each call — cheap enough
    /// at typical session sizes.
    pub fn estimate(&self, system_prompt: &str, messages: &[Message]) -> usize {
        let mut total = tokens::count(system_prompt);
        for msg in messages {
            total = total.saturating_add(tokens::count(&msg.content));
        }
        total
    }

    /// True iff the estimate would meet or exceed the ceiling.
    /// Conservative: equality counts as overflow.
    pub fn would_overflow(&self, system_prompt: &str, messages: &[Message]) -> bool {
        self.estimate(system_prompt, messages) >= self.ceiling
    }

    /// Fraction of the ceiling consumed by the current state.
    /// Returns 0.0..=1.0+ (can exceed 1.0 when over budget).
    pub fn fraction_used(&self, system_prompt: &str, messages: &[Message]) -> f64 {
        if self.ceiling == 0 || self.ceiling == usize::MAX {
            return 0.0;
        }
        self.estimate(system_prompt, messages) as f64 / self.ceiling as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_never_overflow() {
        assert_eq!(Budget::default().ceiling, usize::MAX);
    }

    #[test]
    fn new_sets_ceiling_directly() {
        let budget = Budget::new(5_000);
        assert_eq!(budget.ceiling, 5_000);
    }

    #[test]
    fn from_context_computes_ceiling_via_formula() {
        let budget = Budget::from_context(32_768, 70);
        assert_eq!(budget.ceiling, 22_937);
    }

    #[test]
    fn from_context_saturates_on_overflow() {
        let budget = Budget::from_context(usize::MAX, 100);
        assert!(budget.ceiling > 0);
    }

    #[test]
    fn estimate_sums_prompt_and_messages() {
        let budget = Budget::new(10_000);
        let messages = vec![Message {
            role: "user".to_string(),
            content: "world".to_string(),
            tool_calls: None,
            tool_results: None,
            turn: None,
        }];
        let result = budget.estimate("hello", &messages);
        let expected = tokens::count("hello") + tokens::count("world");
        assert_eq!(result, expected);
    }

    #[test]
    fn would_overflow_true_at_ceiling() {
        let budget = Budget::new(5);
        let messages = vec![Message {
            role: "user".to_string(),
            content: "a b c d e f g".to_string(),
            tool_calls: None,
            tool_results: None,
            turn: None,
        }];
        assert!(budget.would_overflow("", &messages));
    }

    #[test]
    fn would_overflow_false_below_ceiling() {
        let budget = Budget::new(1_000);
        let messages = vec![Message {
            role: "user".to_string(),
            content: "short".to_string(),
            tool_calls: None,
            tool_results: None,
            turn: None,
        }];
        assert!(!budget.would_overflow("", &messages));
    }

    #[test]
    fn fraction_used_returns_ratio() {
        let budget = Budget::new(100);
        let prompt = "a".repeat(100);
        let messages = vec![Message {
            role: "user".to_string(),
            content: "b".repeat(100),
            tool_calls: None,
            tool_results: None,
            turn: None,
        }];
        let total = tokens::count(&prompt) + tokens::count(&messages[0].content);
        let fraction = budget.fraction_used(&prompt, &messages);
        let expected = total as f64 / 100.0;
        assert!((fraction - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn fraction_used_zero_for_sentinel_ceiling() {
        let budget_max = Budget::new(usize::MAX);
        let budget_zero = Budget::new(0);
        assert_eq!(budget_max.fraction_used("hello", &[]), 0.0);
        assert_eq!(budget_zero.fraction_used("hello", &[]), 0.0);
    }
}
