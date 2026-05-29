//! Context compactor. When the budget engine reports overflow,
//! the compactor mutates the message list to free tokens:
//!   1. Replace oldest tool-result message bodies with
//!      compact signatures (size + token count, drops the
//!      actual content).
//!   2. If signatures don't free enough, evict oldest
//!      non-system messages, oldest-first.
//!
//! Never evicts role=`system` — that's contract.

use crate::ai::types::Message;
use crate::context::budget::Budget;
use crate::context::tokens;

/// Aim to leave at least 25% of the budget headroom after
/// compaction. Without headroom, compaction would fire on
/// every turn after the first overflow — noisy UX, repeated
/// work.
const TARGET_FRACTION: f64 = 0.75;

/// What the compactor did this call. Returned to the caller so
/// the loop can surface it to the user or log it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionReport {
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub messages_signaturized: usize,
    pub messages_evicted: usize,
}

impl CompactionReport {
    /// Total tokens freed by this compaction call.
    pub fn tokens_freed(&self) -> usize {
        self.tokens_before.saturating_sub(self.tokens_after)
    }
}

/// Compact messages in place. Mutates `messages` to fit under
/// `target = ceiling × 0.75`. Returns a report of what was done.
/// Caller is responsible for re-checking overflow afterward
/// (the compactor doesn't guarantee `!budget.would_overflow()` —
/// e.g., if the system prompt alone exceeds budget, no amount
/// of message compaction helps).
pub fn compact(
    messages: &mut Vec<Message>,
    budget: &Budget,
    system_prompt: &str,
) -> CompactionReport {
    let tokens_before = budget.estimate(system_prompt, messages);
    let target = (budget.ceiling as f64 * TARGET_FRACTION) as usize;

    let mut messages_signaturized = 0usize;
    let mut messages_evicted = 0usize;

    // Maintain a running token estimate for efficiency.
    let mut running_total = tokens_before;

    // ── Pass 1: signaturize tool-result messages oldest-first ──
    for msg in messages.iter_mut() {
        if running_total <= target {
            break;
        }
        if !is_tool_result(msg) || is_already_signaturized(msg) {
            continue;
        }
        let old_tokens = tokens::count(&msg.content);
        let signature = format_signature(msg);
        let new_tokens = tokens::count(&signature);
        msg.content = signature;
        running_total = running_total
            .saturating_sub(old_tokens)
            .saturating_add(new_tokens);
        messages_signaturized += 1;
    }

    // ── Pass 2: evict oldest non-system messages until under target ──
    while running_total > target {
        let evict_idx = messages.iter().position(|m| m.role != "system");
        let Some(idx) = evict_idx else {
            break;
        };
        let removed = messages.remove(idx);
        running_total = running_total.saturating_sub(tokens::count(&removed.content));
        messages_evicted += 1;
    }

    let tokens_after = running_total;

    CompactionReport {
        tokens_before,
        tokens_after,
        messages_signaturized,
        messages_evicted,
    }
}

/// True iff this message is a `<tool_result>`-wrapped user message.
fn is_tool_result(msg: &Message) -> bool {
    msg.role == "user"
        && msg.content.starts_with("<tool_result>")
        && msg.content.ends_with("</tool_result>")
}

/// True iff this message's content is already in signature form
/// (so re-compaction is a no-op).
fn is_already_signaturized(msg: &Message) -> bool {
    msg.content.contains("[compacted:")
}

/// Replace a tool-result message's body with a signature.
/// Format is spec-pinned: tests grep for the `[compacted: `
/// substring.
fn format_signature(msg: &Message) -> String {
    let body_start = "<tool_result>".len();
    let body_end = msg.content.len() - "</tool_result>".len();
    let body = &msg.content[body_start..body_end];
    let byte_size = body.len();
    let token_count = tokens::count(body);
    format!(
        "<tool_result>[compacted: {} bytes / {} tokens — original content compacted for budget]</tool_result>",
        byte_size, token_count,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool_result(content: &str) -> Message {
        Message {
            role: "user".to_string(),
            content: format!("<tool_result>{content}</tool_result>"),
            tool_calls: None,
            tool_results: None,
            turn: None,
        }
    }

    fn make_system(content: &str) -> Message {
        Message {
            role: "system".to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_results: None,
            turn: None,
        }
    }

    fn make_user(content: &str) -> Message {
        Message {
            role: "user".to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_results: None,
            turn: None,
        }
    }

    fn make_assistant(content: &str) -> Message {
        Message {
            role: "assistant".to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_results: None,
            turn: None,
        }
    }

    #[test]
    fn compact_replaces_tool_result_with_signature() {
        let large_content = "x".repeat(500);
        let mut messages = vec![make_system("sys"), make_tool_result(&large_content)];

        let budget = Budget::new(150);
        let report = compact(&mut messages, &budget, "sys");

        assert_eq!(report.messages_signaturized, 1);
        assert_eq!(report.messages_evicted, 0);
        assert!(messages[1].content.contains("[compacted:"));
        assert!(messages[1].content.contains("bytes /"));
        assert!(messages[1].content.contains("tokens"));
        assert!(messages[1].content.starts_with("<tool_result>"));
        assert!(messages[1].content.ends_with("</tool_result>"));
    }

    #[test]
    fn compact_preserves_system_messages() {
        let large_content = "x".repeat(500);
        let mut messages = vec![
            make_system("system prompt 1"),
            make_system("system prompt 2"),
            make_user(&large_content),
            make_assistant(&large_content),
            make_user(&large_content),
        ];

        let budget = Budget::new(100);
        let report = compact(&mut messages, &budget, "system prompt 1system prompt 2");

        let system_count = messages.iter().filter(|m| m.role == "system").count();
        assert_eq!(system_count, 2, "both system messages must survive");
        assert!(report.messages_evicted > 0);
    }

    #[test]
    fn compact_evicts_oldest_first() {
        let large = "x".repeat(200);
        let mut messages = vec![
            make_user(&large),
            make_assistant(&large),
            make_user(&large),
            make_assistant(&large),
            make_user(&large),
            make_assistant(&large),
            make_user(&large),
            make_assistant(&large),
        ];

        let budget = Budget::new(300);
        compact(&mut messages, &budget, "");

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].content, large);
        assert_eq!(messages[1].content, large);
        assert_eq!(messages[2].content, large);
        assert_eq!(messages[3].content, large);
    }

    #[test]
    fn compact_idempotent_on_already_signaturized() {
        let large_content = "x".repeat(500);
        let mut messages = vec![make_system("sys"), make_tool_result(&large_content)];

        let budget = Budget::new(150);
        let report1 = compact(&mut messages, &budget, "sys");
        let signaturized_content = messages[1].content.clone();

        let report2 = compact(&mut messages, &budget, "sys");

        assert_eq!(messages[1].content, signaturized_content);
        assert_eq!(report2.messages_signaturized, 0);
        assert_eq!(report2.messages_evicted, 0);
        assert_eq!(report2.tokens_before, report1.tokens_after);
        assert_eq!(report2.tokens_after, report1.tokens_after);
    }

    #[test]
    fn compact_does_nothing_when_under_target() {
        let mut messages = vec![make_user("hello"), make_assistant("hi there")];

        let budget = Budget::new(100_000);
        let report = compact(&mut messages, &budget, "");

        assert_eq!(report.messages_signaturized, 0);
        assert_eq!(report.messages_evicted, 0);
        assert_eq!(report.tokens_before, report.tokens_after);
    }

    #[test]
    fn compact_stops_at_target_fraction() {
        let large_content = "x".repeat(500);
        let mut messages = vec![make_system("sys")];
        for _ in 0..10 {
            messages.push(make_tool_result(&large_content));
        }

        let budget = Budget::new(200);
        let report = compact(&mut messages, &budget, "sys");

        let target = (budget.ceiling as f64 * TARGET_FRACTION) as usize;
        assert!(
            report.tokens_after <= target,
            "tokens_after ({}) should be <= target ({})",
            report.tokens_after,
            target
        );
    }

    #[test]
    fn compact_returns_report_with_correct_counts() {
        let large_content = "x".repeat(500);
        let mut messages = vec![
            make_system("sys"),
            make_tool_result(&large_content),
            make_tool_result(&large_content),
            make_user("user msg"),
        ];

        let budget = Budget::new(150);
        let report = compact(&mut messages, &budget, "sys");

        assert!(report.tokens_before > report.tokens_after);
        assert_eq!(report.messages_signaturized, 2);
    }

    #[test]
    fn compact_handles_only_system_messages_left() {
        let mut messages = vec![
            make_system("very long system prompt that alone exceeds budget"),
            make_user("user msg"),
        ];

        let budget = Budget::new(10);
        let report = compact(
            &mut messages,
            &budget,
            "very long system prompt that alone exceeds budget",
        );

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "system");
        assert_eq!(report.messages_evicted, 1);
        assert!(report.tokens_after > 0);
    }
}
