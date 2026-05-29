// The forgiving tool-call parser.
//
// Turns a weak local model's response into a validated `ToolCall`, or — when it
// can't — into a `ParseFailure` carrying feedback the model can recover from.
// This module defines the pipeline's shared types plus two self-contained stages:
// `strip_think_blocks` and `detect`. Extraction, scoring, repair, validation,
// feedback, and the `parse` orchestration that composes them land in later phases.

use serde::{Deserialize, Serialize};
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
pub mod feedback;
pub mod repair;
pub mod score;
pub mod validate;

/// A tool call extracted (and possibly repaired) from a model response. Produced
/// by every successful pipeline run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
    pub origin: Origin,
}

/// Where a `ToolCall` came from — the audit trail the telemetry layer uses to
/// tune the parser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseFailure {
    pub raw: String,
    pub detected_format: Option<Format>,
    pub candidates: Vec<Candidate>,
    pub feedback: String,
}

/// A scored, possibly-repaired extraction candidate. The highest-scoring
/// candidate that validates wins; the rest accumulate in
/// `ParseFailure.candidates` for telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Run the full parser pipeline on a model response.
///
/// Composes detect → extract (all detected formats) → score → repair → validate,
/// with a feedback message on failure. Returns `NoToolCall` when nothing looked
/// like a tool call, `Found` for a validated (possibly repaired) call, or
/// `Failed` with model-readable feedback when a call was attempted but couldn't
/// be validated.
pub fn parse(response: &str, registry: &crate::tools::ToolRegistry) -> ParseResult {
    use detect::detect;
    use extract::{fenced, hermes, loose_json, text, xml, yaml};
    use feedback::format_failure;
    use repair::apply;
    use score::score;
    use validate::validate;

    let formats = detect(response);

    let mut candidates: Vec<Candidate> = Vec::new();
    for fmt in &formats {
        let mut extracted = match fmt {
            Format::Hermes => hermes::extract(response),
            Format::FencedJson => fenced::extract(response),
            Format::LooseJson => loose_json::extract(response),
            Format::Yaml => yaml::extract(response),
            Format::XmlVariant => xml::extract(response),
            Format::PlainText => text::extract(response),
        };
        candidates.append(&mut extracted);
    }

    if candidates.is_empty() {
        return ParseResult::NoToolCall;
    }

    for candidate in &mut candidates {
        candidate.score = score(candidate, registry);
    }

    candidates.sort_by_key(|c| -c.score);

    let mut last_error = None;
    for candidate in &candidates {
        let mut repaired = candidate.clone();
        apply(&mut repaired, registry);
        match validate(&repaired, registry) {
            Ok(tool_call) => return ParseResult::Found(tool_call),
            Err(err) => {
                last_error = Some((repaired, err));
            }
        }
    }

    // Safe: `candidates` is non-empty (checked above) and the loop above ran
    // `validate` on every candidate without returning, so at least one error was
    // recorded in `last_error`.
    let (best_repaired, best_err) =
        last_error.expect("candidates non-empty, so an error was recorded");
    let detected_format = formats.first().copied();

    let mut failed_candidates: Vec<Candidate> = candidates
        .iter()
        .map(|c| {
            let mut repaired = c.clone();
            apply(&mut repaired, registry);
            repaired
        })
        .collect();
    failed_candidates.sort_by_key(|c| -c.score);

    let feedback = format_failure(&best_repaired, &best_err, registry);

    ParseResult::Failed(ParseFailure {
        raw: response.to_string(),
        detected_format,
        candidates: failed_candidates,
        feedback,
    })
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

    use crate::security::scope::Scope;
    use crate::tools::{bash, find_files, patch, read_file, search, symbols, write_file};

    fn test_registry() -> crate::tools::ToolRegistry {
        let dir = tempfile::TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let mut r = crate::tools::ToolRegistry::new();
        for t in [
            read_file(scope.clone()),
            write_file(scope.clone()),
            patch(scope.clone()),
            search(scope.clone()),
            find_files(scope.clone()),
            symbols(scope.clone()),
            bash(scope, 30),
        ] {
            r.register(t);
        }
        r
    }

    #[test]
    fn vllm_qwen3_end_to_end_round_trip_dispatches_bash() {
        // Mirrors the bytes the OpenAI backend accumulates for Qwen3 served by
        // vLLM with auto-tool-choice: reasoning wrapped in <think>, then a
        // synthetic <tool_call>.
        let registry = test_registry();
        let full_response = "<think>The user wants me to run id and share the output. I will use the bash tool to execute `id`.\n</think>\n\n\n<tool_call>{\"name\":\"bash\",\"arguments\":{\"command\":\"id\"}}</tool_call>";
        let stripped = strip_think_blocks(full_response);
        match parse(&stripped, &registry) {
            ParseResult::Found(tc) => {
                assert_eq!(tc.name, "bash");
                assert_eq!(tc.arguments["command"], "id");
            }
            other => panic!("expected Found(bash), got {other:?}"),
        }
    }

    #[test]
    fn parse_returns_no_tool_call_for_plain_prose() {
        let registry = test_registry();
        let result = parse("Just a chat response, no tool call.", &registry);
        assert!(matches!(result, ParseResult::NoToolCall));
    }

    #[test]
    fn parse_returns_found_for_valid_hermes() {
        let registry = test_registry();
        let result = parse(
            "{\"name\":\"read_file\",\"arguments\":{\"path\":\"x\"}}",
            &registry,
        );
        match result {
            ParseResult::Found(tc) => assert_eq!(tc.name, "read_file"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn parse_returns_found_for_valid_fenced_json() {
        let registry = test_registry();
        let result = parse(
            "```json\n{\"name\":\"read_file\",\"arguments\":{\"path\":\"x\"}}\n```",
            &registry,
        );
        match result {
            ParseResult::Found(tc) => assert_eq!(tc.name, "read_file"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn parse_returns_failed_for_unknown_tool() {
        let registry = test_registry();
        let result = parse("{\"name\":\"nonexistent\",\"arguments\":{}}", &registry);
        match result {
            ParseResult::Failed(f) => assert!(f.feedback.contains("unknown")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn parse_returns_failed_for_missing_required() {
        let registry = test_registry();
        let result = parse("{\"name\":\"read_file\",\"arguments\":{}}", &registry);
        match result {
            ParseResult::Failed(f) => assert!(f.feedback.contains("requires")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn parse_picks_highest_scoring_candidate_when_multiple() {
        let registry = test_registry();
        let result = parse(
            "some text {\"name\":\"bad\"} more {\"name\":\"read_file\",\"arguments\":{\"path\":\"x\"}}",
            &registry,
        );
        match result {
            ParseResult::Found(tc) => assert_eq!(tc.name, "read_file"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn parse_repairs_close_typo() {
        let registry = test_registry();
        let result = parse(
            "{\"name\":\"read_fil\",\"arguments\":{\"path\":\"x\"}}",
            &registry,
        );
        match result {
            ParseResult::Found(tc) => assert_eq!(tc.name, "read_file"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn parse_includes_format_in_parse_failure() {
        let registry = test_registry();
        let result = parse("{\"name\":\"nonexistent\",\"arguments\":{}}", &registry);
        match result {
            ParseResult::Failed(f) => assert!(f.detected_format.is_some()),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn parse_returns_failed_for_empty_object() {
        let registry = test_registry();
        let result = parse("<tool_call>{}</tool_call>", &registry);
        match result {
            ParseResult::Failed(f) => assert!(f.feedback.contains("lacked a name field")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
