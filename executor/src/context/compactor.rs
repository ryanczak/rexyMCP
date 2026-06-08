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

/// Protect the most recent turns from signaturization — the live diagnostics,
/// the current edit's reads, and the working context the model is actively
/// using all live here. A tool result whose turn is within this many turns of
/// the newest turn in the conversation is never signaturized by the
/// value-ranked pass.
const RECENT_TURNS_PROTECTED: usize = 3;

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

    // ── Pass 1.5: value-ranked, in-place signaturization of structured tool
    // results. Preferred over eviction: shrinks content while preserving every
    // message and every tool-call/tool-result pair. ──
    let newest_turn = messages.iter().filter_map(|m| m.turn).max();
    let mut candidates: Vec<(u8, usize, usize)> = messages
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            reclaim_rank(m, newest_turn).map(|rank| (rank, m.turn.unwrap_or(0), i))
        })
        .collect();
    candidates.sort();
    for (_, _, idx) in candidates {
        if running_total <= target {
            break;
        }
        let before = message_tokens(&messages[idx]);
        signaturize_tool_result(&mut messages[idx]);
        let after = message_tokens(&messages[idx]);
        running_total = running_total.saturating_sub(before.saturating_sub(after));
        messages_signaturized += 1;
    }

    // ── Pass 2: evict oldest non-system messages until under target ──
    while running_total > target {
        let evict_idx = messages.iter().position(|m| m.role != "system");
        let Some(idx) = evict_idx else {
            break;
        };
        let removed = messages.remove(idx);
        running_total = running_total.saturating_sub(message_tokens(&removed));
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

/// Tokens a single message contributes, counting the same parts as
/// `Budget::estimate`: `content` + every `tool_calls[].arguments` +
/// every `tool_results[].content`. The compactor's running total must use
/// this — a structured tool/assistant message carries its payload in
/// `tool_calls`/`tool_results`, not `content`.
fn message_tokens(msg: &Message) -> usize {
    let mut total = tokens::count(&msg.content);
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
    total
}

/// Value rank for in-place signaturization. `Some(rank)` marks a structured
/// tool-result message whose content can be shrunk to a `[compacted: …]`
/// signature; lower rank is reclaimed first. `None` means "leave it alone":
/// not a structured tool result, already reclaimed, or recency-protected.
///
/// Rank order (reclaim cheapest-to-lose first): non-`read_file` tool output
/// (bash/search/etc. — noisy, regenerable) ranks 0, before `read_file` results
/// (the model's working file context) at rank 1.
fn reclaim_rank(msg: &Message, newest_turn: Option<usize>) -> Option<u8> {
    if msg.role != "tool" {
        return None;
    }
    let r = msg.tool_results.as_ref()?.first()?;
    // Already-reclaimed husks / signatures — nothing to gain, don't re-wrap.
    if r.content.contains("[compacted:")
        || r.content.starts_with("[superseded:")
        || r.content.starts_with("[already-read:")
    {
        return None;
    }
    // Protect the last RECENT_TURNS_PROTECTED turns (diagnostics live here).
    if let (Some(t), Some(n)) = (msg.turn, newest_turn)
        && n.saturating_sub(t) < RECENT_TURNS_PROTECTED
    {
        return None;
    }
    if r.tool_name == "read_file" {
        Some(1)
    } else {
        Some(0)
    }
}

/// Replace a structured tool-result's content with a compact signature, in
/// place. The `[compacted:` marker matches `is_already_signaturized` and the
/// pass-1 text-shape format, so re-compaction skips it.
fn signaturize_tool_result(msg: &mut Message) {
    if let Some(results) = msg.tool_results.as_mut()
        && let Some(r) = results.first_mut()
    {
        let byte_size = r.content.len();
        let token_count = tokens::count(&r.content);
        r.content = format!(
            "[compacted: {byte_size} bytes / {token_count} tokens — tool result compacted for budget]"
        );
    }
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

    fn make_user_with_turn(content: &str, turn: usize) -> Message {
        Message {
            role: "user".to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_results: None,
            turn: Some(turn),
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

    fn make_assistant_with_turn(content: &str, turn: usize) -> Message {
        Message {
            role: "assistant".to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_results: None,
            turn: Some(turn),
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

    // ── Helpers for structured tool-exchange tests ──

    fn make_tool_msg(tool_name: &str, content: &str, turn: usize) -> Message {
        Message {
            role: "tool".to_string(),
            content: String::new(),
            tool_calls: None,
            tool_results: Some(vec![crate::ai::types::ToolResult {
                tool_call_id: "c1".to_string(),
                tool_name: tool_name.to_string(),
                content: content.to_string(),
            }]),
            turn: Some(turn),
        }
    }

    // ── message_tokens tests ──

    #[test]
    fn message_tokens_counts_tool_results_content() {
        let big = "x".repeat(400);
        let msg = make_tool_msg("read_file", &big, 1);
        let t = message_tokens(&msg);
        assert!(
            t > 0,
            "message_tokens must count tool_results content (got {})",
            t
        );
        assert_eq!(t, tokens::count(&big));
    }

    #[test]
    fn message_tokens_counts_tool_call_arguments() {
        let msg = Message {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(vec![crate::ai::types::ToolCall {
                id: "tc1".to_string(),
                name: "bash".to_string(),
                arguments: r#"{"command":"echo hello"}"#.to_string(),
                thought_signature: None,
            }]),
            tool_results: None,
            turn: Some(1),
        };
        let t = message_tokens(&msg);
        assert!(
            t > 0,
            "message_tokens must count tool_calls arguments (got {})",
            t
        );
    }

    #[test]
    fn message_tokens_plain_equals_content() {
        let msg = make_user("hello world");
        let t = message_tokens(&msg);
        assert_eq!(t, tokens::count("hello world"));
    }

    // ── reclaim_rank tests ──

    #[test]
    fn reclaim_rank_command_output_before_read() {
        let big = "x".repeat(2000);
        let bash_msg = make_tool_msg("bash", &big, 1);
        let read_msg = make_tool_msg("read_file", &big, 1);

        assert_eq!(reclaim_rank(&bash_msg, Some(10)), Some(0));
        assert_eq!(reclaim_rank(&read_msg, Some(10)), Some(1));
    }

    #[test]
    fn reclaim_rank_protects_recent_turns() {
        let msg = make_tool_msg("bash", &"x".repeat(400), 9);
        assert_eq!(
            reclaim_rank(&msg, Some(10)),
            None,
            "turn 9 is within RECENT_TURNS_PROTECTED of turn 10"
        );
    }

    #[test]
    fn reclaim_rank_skips_already_compacted() {
        let msg = make_tool_msg(
            "bash",
            "[compacted: 100 bytes / 25 tokens — tool result compacted for budget]",
            1,
        );
        assert_eq!(
            reclaim_rank(&msg, Some(10)),
            None,
            "already compacted content must be skipped"
        );
    }

    #[test]
    fn reclaim_rank_skips_superseded_and_already_read_husks() {
        let superseded = make_tool_msg("read_file", "[superseded: file edited at turn 5]", 1);
        let already_read = make_tool_msg("read_file", "[already-read: unchanged since turn 3]", 1);

        assert_eq!(reclaim_rank(&superseded, Some(10)), None);
        assert_eq!(reclaim_rank(&already_read, Some(10)), None);
    }

    #[test]
    fn reclaim_rank_skips_non_tool_messages() {
        assert_eq!(reclaim_rank(&make_user("hi"), Some(10)), None);
        assert_eq!(reclaim_rank(&make_assistant("hi"), Some(10)), None);
        assert_eq!(reclaim_rank(&make_system("hi"), Some(10)), None);
    }

    // ── compact integration over structured shape ──

    #[test]
    fn compact_signaturizes_structured_tool_result_in_place() {
        let big = "x".repeat(2000); // ~500 tokens
        let mut messages = vec![
            make_system("sys"),
            make_tool_msg("read_file", &big, 1),
            make_user_with_turn("do something", 4),
            make_assistant_with_turn("ok", 5),
            make_user_with_turn("more recent", 6),
            make_assistant_with_turn("even more", 7),
            make_user_with_turn("latest", 8),
            make_assistant_with_turn("done", 9),
        ];

        // ceiling=500 → target=375. Before: ~515 tokens (500 from big + ~15 overhead).
        // After signaturizing the tool result (~500→10): ~25 tokens < 375.
        let budget = Budget::new(500);
        let report = compact(&mut messages, &budget, "sys");

        // The turn-1 tool result should be signaturized in place
        let tool_msg = &messages[1];
        assert!(
            tool_msg
                .tool_results
                .as_ref()
                .unwrap()
                .first()
                .unwrap()
                .content
                .contains("[compacted:"),
            "structured tool result content must contain [compacted:"
        );
        // Message is still present (not removed)
        assert_eq!(messages.len(), 8);
        assert!(report.messages_signaturized >= 1);
        assert!(
            report.tokens_after < report.tokens_before,
            "tokens must decrease"
        );
    }

    #[test]
    fn compact_reclaims_command_output_before_file_read() {
        let big = "x".repeat(2000); // ~500 tokens each
        let mut messages = vec![
            make_system("sys"),
            make_tool_msg("bash", &big, 1),
            make_tool_msg("read_file", &big, 2),
            make_user_with_turn("recent", 5),
            make_assistant_with_turn("recent", 6),
            make_user_with_turn("recent", 7),
            make_assistant_with_turn("recent", 8),
            make_user_with_turn("recent", 9),
            make_assistant_with_turn("recent", 10),
        ];

        // ceiling=1200 → target=900. Before: ~1015 tokens (1000 from 2×big + ~15 overhead).
        // After signaturizing bash (~500→10): ~525 tokens < 900.
        // So signaturizing just the bash result reaches target.
        let budget = Budget::new(1200);
        let report = compact(&mut messages, &budget, "sys");

        let bash_content = &messages[1]
            .tool_results
            .as_ref()
            .unwrap()
            .first()
            .unwrap()
            .content;
        let read_content = &messages[2]
            .tool_results
            .as_ref()
            .unwrap()
            .first()
            .unwrap()
            .content;

        assert!(
            bash_content.contains("[compacted:"),
            "bash result must be signaturized (rank 0 < rank 1)"
        );
        assert!(
            !read_content.contains("[compacted:"),
            "read_file result must be left intact (rank 1)"
        );
        assert_eq!(read_content, &big);
        assert_eq!(report.messages_evicted, 0);
    }

    #[test]
    fn compact_protects_recent_tool_result() {
        let big = "x".repeat(2000); // ~500 tokens each
        let newest = 10;
        let mut messages = vec![
            make_system("sys"),
            make_tool_msg("bash", &big, 1),
            make_tool_msg("bash", &big, 2),
            make_tool_msg("bash", &big, newest),
        ];

        // ceiling=1500 → target=1125. Before: ~1503 tokens (1500 from 3×big + ~3 overhead).
        // After signaturizing bash@1 and bash@2 (~500+500→10+10): ~513 tokens < 1125.
        // The newest-turn result must stay protected.
        let budget = Budget::new(1500);
        let report = compact(&mut messages, &budget, "sys");

        // The newest-turn result must be unchanged
        let newest_content = &messages[3]
            .tool_results
            .as_ref()
            .unwrap()
            .first()
            .unwrap()
            .content;
        assert!(
            !newest_content.contains("[compacted:"),
            "newest-turn tool result must be protected"
        );
        assert_eq!(newest_content, &big);
        assert_eq!(report.messages_evicted, 0);
    }

    #[test]
    fn compact_signaturization_preserves_pairing_and_count() {
        let big = "x".repeat(2000); // ~500 tokens each
        let mut messages = vec![
            make_system("sys"),
            Message {
                role: "assistant".to_string(),
                content: String::new(),
                tool_calls: Some(vec![crate::ai::types::ToolCall {
                    id: "tc1".to_string(),
                    name: "bash".to_string(),
                    arguments: r#"{"command":"ls"}"#.to_string(),
                    thought_signature: None,
                }]),
                tool_results: None,
                turn: Some(1),
            },
            make_tool_msg("bash", &big, 1),
            Message {
                role: "assistant".to_string(),
                content: String::new(),
                tool_calls: Some(vec![crate::ai::types::ToolCall {
                    id: "tc2".to_string(),
                    name: "bash".to_string(),
                    arguments: r#"{"command":"pwd"}"#.to_string(),
                    thought_signature: None,
                }]),
                tool_results: None,
                turn: Some(2),
            },
            make_tool_msg("bash", &big, 2),
            make_user_with_turn("recent", 7),
            make_assistant_with_turn("recent", 8),
            make_user_with_turn("recent", 9),
            make_assistant_with_turn("recent", 10),
        ];

        let original_len = messages.len();
        let budget = Budget::new(300);
        let report = compact(&mut messages, &budget, "sys");

        assert_eq!(
            messages.len(),
            original_len,
            "signaturization must not remove messages"
        );
        assert_eq!(
            report.messages_evicted, 0,
            "signaturization alone sufficed — no evictions"
        );
        // At least one result now contains [compacted:
        let has_compacted = messages.iter().any(|m| {
            m.tool_results
                .as_ref()
                .map(|trs| trs.iter().any(|tr| tr.content.contains("[compacted:")))
                .unwrap_or(false)
        });
        assert!(has_compacted, "at least one result must be compacted");
    }

    #[test]
    fn compact_idempotent_on_structured_signature() {
        let big = "x".repeat(2000); // ~500 tokens each
        let mut messages = vec![
            make_system("sys"),
            make_tool_msg("bash", &big, 1),
            make_tool_msg("bash", &big, 2),
            make_user_with_turn("recent", 7),
            make_assistant_with_turn("recent", 8),
            make_user_with_turn("recent", 9),
            make_assistant_with_turn("recent", 10),
        ];

        let budget = Budget::new(300);
        let report1 = compact(&mut messages, &budget, "sys");

        // Capture the already-compacted content
        let compacted_contents: Vec<String> = messages
            .iter()
            .filter_map(|m| m.tool_results.as_ref())
            .flatten()
            .map(|tr| tr.content.clone())
            .collect();

        let report2 = compact(&mut messages, &budget, "sys");

        // Already-compacted content must be byte-identical
        let compacted_contents_after: Vec<String> = messages
            .iter()
            .filter_map(|m| m.tool_results.as_ref())
            .flatten()
            .map(|tr| tr.content.clone())
            .collect();
        assert_eq!(compacted_contents, compacted_contents_after);

        // Second run must not re-signaturize already-compacted results
        assert!(
            report2.messages_signaturized <= report1.messages_signaturized,
            "second run should not re-signaturize already-compacted results"
        );
    }
}
