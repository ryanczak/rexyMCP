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
            if let Some(tcs) = &msg.tool_calls {
                for tc in tcs {
                    total = total.saturating_add(tokens::count(&tc.arguments));
                }
            }
            if let Some(trs) = &msg.tool_results {
                for tr in trs {
                    total = total.saturating_add(tokens::count(&tr.content));
                }
            }
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
    use crate::ai::types::{ToolCall, ToolResult};

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
    fn estimate_includes_tool_result_content() {
        let budget = Budget::new(10_000);
        let messages = vec![Message {
            role: "tool".to_string(),
            content: String::new(),
            tool_calls: None,
            tool_results: Some(vec![ToolResult {
                tool_call_id: "id1".to_string(),
                tool_name: "read_file".to_string(),
                content: "file content goes here".to_string(),
            }]),
            turn: Some(1),
        }];
        let estimated = budget.estimate("", &messages);
        assert!(
            estimated > 0,
            "estimate must count tool_result content, not just msg.content"
        );
        assert_eq!(estimated, tokens::count("file content goes here"));
    }

    #[test]
    fn estimate_includes_tool_call_arguments() {
        let budget = Budget::new(10_000);
        let messages = vec![Message {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: "tc1".to_string(),
                name: "patch".to_string(),
                arguments: r#"{"path":"foo.rs","old_str":"x","new_str":"y"}"#.to_string(),
                thought_signature: None,
            }]),
            tool_results: None,
            turn: Some(2),
        }];
        let estimated = budget.estimate("", &messages);
        assert!(
            estimated > 0,
            "estimate must count tool_call arguments, not just msg.content"
        );
        assert_eq!(
            estimated,
            tokens::count(r#"{"path":"foo.rs","old_str":"x","new_str":"y"}"#)
        );
    }

    #[test]
    fn estimate_counts_all_payloads_in_a_tool_exchange() {
        let budget = Budget::new(10_000);
        let args = r#"{"path":"src/lib.rs"}"#;
        let result_body = "pub fn hello() {}";
        let messages = vec![
            Message {
                role: "assistant".to_string(),
                content: String::new(),
                tool_calls: Some(vec![ToolCall {
                    id: "tc2".to_string(),
                    name: "read_file".to_string(),
                    arguments: args.to_string(),
                    thought_signature: None,
                }]),
                tool_results: None,
                turn: Some(1),
            },
            Message {
                role: "tool".to_string(),
                content: String::new(),
                tool_calls: None,
                tool_results: Some(vec![ToolResult {
                    tool_call_id: "tc2".to_string(),
                    tool_name: "read_file".to_string(),
                    content: result_body.to_string(),
                }]),
                turn: Some(1),
            },
        ];
        let estimated = budget.estimate("", &messages);
        let expected = tokens::count(args) + tokens::count(result_body);
        assert_eq!(estimated, expected);
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
