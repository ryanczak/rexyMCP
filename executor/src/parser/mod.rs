// The forgiving tool-call parser.
//
// Turns a weak local model's response into a validated `ToolCall`, or — when it
// can't — into a `ParseFailure` carrying feedback the model can recover from.
// This module defines the pipeline's shared types plus two self-contained stages:
// `strip_think_blocks` and `detect`. Extraction, scoring, repair, validation,
// feedback, and the `parse` orchestration that composes them land in later phases.

use serde::Serialize;
use serde_json::Value;

/// Strip `<think>…</think>` blocks from text. Backends wrap reasoning deltas in
/// these tags so the user sees the model's chain-of-thought live; strip them
/// before persisting the assistant message so the reasoning is not replayed back
/// to the model every turn (real tokens, no behavioral benefit — the model
/// already reasoned that round).
///
/// An unterminated `<think>` block discards everything after the opening tag — a
/// reasoning-only response that got truncated has nothing worth keeping. A
/// trailing newline after `</think>` is also consumed since backends append one.
pub fn strip_think_blocks(s: &str) -> String {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + OPEN.len()..];
        let Some(end) = after_open.find(CLOSE) else {
            return out;
        };
        let mut next = &after_open[end + CLOSE.len()..];
        if let Some(stripped) = next.strip_prefix('\n') {
            next = stripped;
        }
        rest = next;
    }
    out.push_str(rest);
    out
}

pub mod detect;
pub mod extract;

/// A tool call extracted (and possibly repaired) from a model response. Produced
/// by every successful pipeline run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
    pub origin: Origin,
}

/// Where a `ToolCall` came from — the audit trail the telemetry layer uses to
/// tune the parser.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Origin {
    /// Backend emitted a native tool_calls / tool_use block.
    Native,
    /// Text format extracted cleanly, no repair needed.
    Extracted { format: Format },
    /// Text format extracted after one or more repair transforms.
    Repaired {
        format: Format,
        repairs: Vec<RepairOp>,
    },
}

/// The six text formats the parser recognizes. Order here is **not** priority
/// order — priority lives in `detect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Hermes,
    FencedJson,
    LooseJson,
    Yaml,
    XmlVariant,
    PlainText,
}

/// One repair transform applied to a candidate during the repair pass.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RepairOp {
    /// Fuzzy match of the tool name (Levenshtein ≤ 2).
    NameFuzzyMatch { from: String, to: String },
    /// Param renamed via the per-tool alias table.
    ParamAlias { from: String, to: String },
    /// Argument coerced to the schema's expected type.
    TypeCoerce {
        field: String,
        from_type: String,
        to_type: String,
    },
    /// Missing non-required param filled with the schema default.
    DefaultFill { field: String },
    /// One-pass JSON syntax repair (trailing commas, unquoted keys, single
    /// quotes, missing closing brace).
    JsonRepair,
    /// Newline-in-string-literal escape.
    NewlineEscape,
}

/// What the parser returns when every stage fails. `feedback` is the advisory
/// message sent back to the model when no candidate could be validated.
#[derive(Debug, Clone, Serialize)]
pub struct ParseFailure {
    pub raw: String,
    pub detected_format: Option<Format>,
    pub candidates: Vec<Candidate>,
    pub feedback: String,
}

/// A scored, possibly-repaired extraction candidate. The highest-scoring
/// candidate that validates wins; the rest accumulate in
/// `ParseFailure.candidates` for telemetry.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub format: Format,
    pub name: Option<String>,
    pub arguments: Option<Value>,
    pub score: i32,
    pub repairs_attempted: Vec<RepairOp>,
    /// The raw text the extractor attempted to parse, when the extractor format
    /// has a parseable body. `None` for `Origin::Native` candidates. Used by the
    /// json_repair and newline_escape repair transforms to retry parsing after
    /// fixing syntax errors.
    pub raw_body: Option<String>,
}

/// The three possible outcomes of running the parser pipeline on a response.
#[derive(Debug)]
pub enum ParseResult {
    /// No tool-call attempt detected — the model is producing a final answer.
    NoToolCall,
    /// A tool call was extracted, repaired if necessary, and validated.
    Found(ToolCall),
    /// A tool call was attempted but couldn't be validated; `feedback` says why.
    Failed(ParseFailure),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_think_blocks_removes_single_block_and_trailing_newline() {
        let input = "<think>reasoning here</think>\nanswer";
        assert_eq!(strip_think_blocks(input), "answer");
    }

    #[test]
    fn strip_think_blocks_handles_multiple_blocks() {
        let input = "<think>a</think>\nmid <think>b</think>\nend";
        assert_eq!(strip_think_blocks(input), "mid end");
    }

    #[test]
    fn strip_think_blocks_passes_through_when_no_block() {
        assert_eq!(strip_think_blocks("just answer"), "just answer");
    }

    #[test]
    fn strip_think_blocks_drops_unterminated_tail() {
        let input = "answer <think>truncated reasoning";
        assert_eq!(strip_think_blocks(input), "answer ");
    }

    #[test]
    fn strip_think_blocks_preserves_tool_call_after_closed_block() {
        let input = "<think>plan</think>\n<tool_call>{\"name\":\"x\"}</tool_call>";
        assert_eq!(
            strip_think_blocks(input),
            "<tool_call>{\"name\":\"x\"}</tool_call>"
        );
    }
}
