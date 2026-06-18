use serde_json::Value;

use super::Candidate;
use super::validate::{TypeMismatch, ValidationError};
use crate::tools::ToolRegistry;

/// Format a model-readable feedback message from a validation failure on the
/// best-scoring candidate.
///
/// "Name the wrong value, suggest the fix": every message names the offending
/// value (what the model emitted) and proposes a concrete fix (what would work).
pub fn format_failure(
    _best: &Candidate,
    error: &ValidationError,
    registry: &ToolRegistry,
) -> String {
    match error {
        ValidationError::UnknownTool {
            attempted_name,
            available_tools,
        } => format_unknown_tool(attempted_name, available_tools),
        ValidationError::SchemaFailures {
            tool,
            missing_required,
            unknown_params,
            type_mismatches,
        } => {
            if !missing_required.is_empty() {
                format_missing_required(tool, missing_required, registry)
            } else if !type_mismatches.is_empty() {
                format_type_mismatch(tool, type_mismatches)
            } else if !unknown_params.is_empty() {
                format_unknown_params(tool, unknown_params, registry)
            } else {
                format!("Tool `{tool}` failed validation.")
            }
        }
    }
}

/// Used when no candidates were extracted at all. Short, generic guidance —
/// there's no specific value to point at.
pub fn format_no_match(response_excerpt: &str) -> String {
    let excerpt = if response_excerpt.chars().count() > 200 {
        format!(
            "{}...",
            response_excerpt.chars().take(200).collect::<String>()
        )
    } else {
        response_excerpt.to_string()
    };
    format!(
        "No tool call was found in your response. \
         Emit a single tool call in the expected format, \
         or respond without a tool call if you are done.\n\
         Excerpt: {excerpt}"
    )
}

/// Feedback for a turn the backend cut off at the output-token ceiling
/// (`finish_reason == "length"`) before a tool call appeared — the model ran out
/// of output budget mid-stream, so its stub is not a deliberate completion.
pub fn format_truncated(response_excerpt: &str) -> String {
    // char-safe truncation (do not byte-slice — multibyte boundaries panic).
    let excerpt: String = response_excerpt.chars().take(200).collect();
    format!(
        "Your previous response was cut off at the output-token limit before you \
         emitted a tool call. Do not keep reasoning — emit a single tool call in \
         the expected format now, and keep any reasoning brief.\n\
         Excerpt: {excerpt}"
    )
}

/// Escalating feedback for consecutive empty completions: the first empty gets the
/// standard "emit a tool call" nudge; a second or later empty escalates to a
/// no-reasoning directive, since the model is spending the turn inside `<think>`
/// and emitting nothing.
pub fn empty_recovery_feedback(consecutive_empty: usize, response_excerpt: &str) -> String {
    if consecutive_empty >= 2 {
        "You have returned multiple empty responses in a row. Do NOT write any \
          <think> reasoning this turn. Respond with exactly one tool call in the \
         expected format and nothing else."
            .to_string()
    } else {
        format_no_match(response_excerpt)
    }
}

fn format_unknown_tool(attempted_name: &Option<String>, available_tools: &[String]) -> String {
    let tools_list = available_tools.join(", ");

    match attempted_name {
        Some(name) => {
            let suggestion = closest_tool(name, available_tools);
            match suggestion {
                Some(s) => format!(
                    "Tool name `{name}` is unknown. Did you mean `{s}`? Available tools: {tools_list}."
                ),
                None => format!("Tool name `{name}` is unknown. Available tools: {tools_list}."),
            }
        }
        None => format!("Tool call lacked a name field. Available tools: {tools_list}."),
    }
}

fn closest_tool(name: &str, available: &[String]) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for tool in available {
        let d = levenshtein(name, tool);
        if d <= 2 {
            match best {
                Some((best_d, _)) if d < best_d => {
                    best = Some((d, tool.as_str()));
                }
                Some((best_d, best_name)) if d == best_d && tool.as_str() < best_name => {
                    best = Some((d, tool.as_str()));
                }
                None => {
                    best = Some((d, tool.as_str()));
                }
                _ => {}
            }
        }
    }
    best.map(|(_, n)| n.to_string())
}

fn format_missing_required(tool: &str, missing: &[String], registry: &ToolRegistry) -> String {
    let to_show = missing.iter().take(3).collect::<Vec<_>>();

    if let Some(tool_obj) = registry.get(tool) {
        let schema = tool_obj.schema();
        let properties = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        to_show
            .iter()
            .map(|param| {
                let param_type = properties
                    .get(param.as_str())
                    .and_then(|p| p.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                format!("Tool `{tool}` requires `{param}` ({param_type}) but it was not provided.")
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        to_show
            .iter()
            .map(|param| format!("Tool `{tool}` requires `{param}` but it was not provided."))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn format_type_mismatch(tool: &str, mismatches: &[TypeMismatch]) -> String {
    let to_show = mismatches.iter().take(3).collect::<Vec<_>>();

    to_show
        .iter()
        .map(|tm| {
            let quoted = quote_value(&tm.actual_value);
            let fix = suggest_fix(&tm.expected_type, &tm.actual_value);
            match fix {
                Some(f) => format!(
                    "Tool `{tool}` received `{field}: {quoted}` but expects {expected}. Use `{field}: {f}`.",
                    field = tm.field,
                    expected = tm.expected_type,
                ),
                None => format!(
                    "Tool `{tool}` received `{field}: {quoted}` but expects {expected}.",
                    field = tm.field,
                    expected = tm.expected_type,
                ),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_unknown_params(tool: &str, params: &[String], registry: &ToolRegistry) -> String {
    let to_show = params.iter().take(3).collect::<Vec<_>>();

    let valid_list = if let Some(tool_obj) = registry.get(tool) {
        let schema = tool_obj.schema();
        schema
            .get("properties")
            .and_then(|v| v.as_object())
            .map(|m| m.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    } else {
        vec![]
    };

    let valid = if valid_list.is_empty() {
        "none defined".to_string()
    } else {
        valid_list.join(", ")
    };

    to_show
        .iter()
        .map(|param| {
            format!(
                "Tool `{tool}` received unknown parameter `{param}`. Valid parameters: {valid}."
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_value(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{s}\""),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        Value::Array(_) => value.to_string(),
        Value::Object(_) => value.to_string(),
    }
}

fn suggest_fix(expected_type: &str, actual: &Value) -> Option<String> {
    match expected_type {
        "integer" => {
            if let Value::String(s) = actual {
                if let Ok(n) = s.parse::<i64>() {
                    return Some(n.to_string());
                }
                if let Ok(n) = s.parse::<u64>() {
                    return Some(n.to_string());
                }
            }
            None
        }
        "boolean" => {
            if let Value::String(s) = actual {
                if s == "true" {
                    return Some("true".to_string());
                }
                if s == "false" {
                    return Some("false".to_string());
                }
            }
            None
        }
        "string" => match actual {
            Value::Number(n) => Some(format!("\"{n}\"")),
            Value::Bool(b) => Some(format!("\"{b}\"")),
            Value::Null => Some("\"\"".to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let n = a_chars.len();
    let m = b_chars.len();

    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }

    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];

    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[m]
}

#[cfg(test)]
mod tests {
    use super::super::Format;
    use super::super::validate::ValidationError;
    use super::*;
    use crate::security::scope::Scope;
    use crate::tools::{bash, find_files, patch, read_file, search, symbols, write_file};

    fn test_registry() -> ToolRegistry {
        let dir = tempfile::TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let mut r = ToolRegistry::new();
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

    fn sorted_names(registry: &ToolRegistry) -> Vec<String> {
        let mut v: Vec<String> = registry.all().map(|t| t.name().to_string()).collect();
        v.sort();
        v
    }

    fn make_candidate(name: Option<&str>) -> Candidate {
        Candidate {
            format: Format::Hermes,
            name: name.map(String::from),
            arguments: None,
            score: 0,
            repairs_attempted: vec![],
            raw_body: None,
        }
    }

    #[test]
    fn unknown_tool_with_close_suggestion() {
        let registry = test_registry();
        let err = ValidationError::UnknownTool {
            attempted_name: Some("read_fil".to_string()),
            available_tools: sorted_names(&registry),
        };
        let msg = format_failure(&make_candidate(Some("read_fil")), &err, &registry);
        assert!(msg.contains("Tool name `read_fil` is unknown"), "{msg}");
        assert!(msg.contains("Did you mean `read_file`?"), "{msg}");
    }

    #[test]
    fn unknown_tool_without_close_match_lists_tools() {
        let registry = test_registry();
        let err = ValidationError::UnknownTool {
            attempted_name: Some("xyzpdq".to_string()),
            available_tools: sorted_names(&registry),
        };
        let msg = format_failure(&make_candidate(Some("xyzpdq")), &err, &registry);
        assert!(msg.contains("Tool name `xyzpdq` is unknown"), "{msg}");
        assert!(!msg.contains("Did you mean"), "{msg}");
        assert!(msg.contains("Available tools:"), "{msg}");
    }

    #[test]
    fn unknown_tool_with_none_name() {
        let registry = test_registry();
        let err = ValidationError::UnknownTool {
            attempted_name: None,
            available_tools: sorted_names(&registry),
        };
        let msg = format_failure(&make_candidate(None), &err, &registry);
        assert!(msg.contains("Tool call lacked a name field"), "{msg}");
        assert!(msg.contains("Available tools:"), "{msg}");
    }

    #[test]
    fn missing_required_message() {
        let registry = test_registry();
        let err = ValidationError::SchemaFailures {
            tool: "patch".to_string(),
            missing_required: vec!["old_str".to_string()],
            unknown_params: vec![],
            type_mismatches: vec![],
        };
        let msg = format_failure(&make_candidate(Some("patch")), &err, &registry);
        assert!(msg.contains("Tool `patch` requires `old_str`"), "{msg}");
    }

    #[test]
    fn type_mismatch_with_integer_suggestion() {
        let err = ValidationError::SchemaFailures {
            tool: "bash".to_string(),
            missing_required: vec![],
            unknown_params: vec![],
            type_mismatches: vec![TypeMismatch {
                field: "timeout_secs".to_string(),
                expected_type: "integer".to_string(),
                actual_value: Value::String("30".to_string()),
            }],
        };
        let msg = format_failure(&make_candidate(Some("bash")), &err, &test_registry());
        assert!(msg.contains("Use `timeout_secs: 30`"), "{msg}");
    }

    #[test]
    fn type_mismatch_with_boolean_suggestion() {
        let err = ValidationError::SchemaFailures {
            tool: "search".to_string(),
            missing_required: vec![],
            unknown_params: vec![],
            type_mismatches: vec![TypeMismatch {
                field: "case_insensitive".to_string(),
                expected_type: "boolean".to_string(),
                actual_value: Value::String("true".to_string()),
            }],
        };
        let msg = format_failure(&make_candidate(Some("search")), &err, &test_registry());
        assert!(msg.contains("Use `case_insensitive: true`"), "{msg}");
    }

    #[test]
    fn prioritizes_unknown_tool_over_schema_failures() {
        let registry = test_registry();
        let err = ValidationError::UnknownTool {
            attempted_name: Some("nonexistent".to_string()),
            available_tools: sorted_names(&registry),
        };
        let msg = format_failure(&make_candidate(Some("nonexistent")), &err, &registry);
        assert!(msg.contains("unknown"), "{msg}");
    }

    #[test]
    fn prioritizes_missing_required_over_type_mismatch() {
        let registry = test_registry();
        let err = ValidationError::SchemaFailures {
            tool: "read_file".to_string(),
            missing_required: vec!["path".to_string()],
            unknown_params: vec![],
            type_mismatches: vec![TypeMismatch {
                field: "bogus".to_string(),
                expected_type: "string".to_string(),
                actual_value: Value::Number(42.into()),
            }],
        };
        let msg = format_failure(&make_candidate(Some("read_file")), &err, &registry);
        assert!(msg.contains("requires"), "{msg}");
        assert!(!msg.contains("received"), "{msg}");
    }

    #[test]
    fn format_truncated_tells_model_it_was_cut_off() {
        let msg = format_truncated("some long response text");
        assert!(msg.contains("cut off"), "{msg}");
        assert!(msg.contains("tool call"), "{msg}");
    }

    #[test]
    fn empty_recovery_feedback_first_empty_is_standard_nudge() {
        let msg = empty_recovery_feedback(1, "x");
        assert!(msg.contains("No tool call was found"), "{msg}");
    }

    #[test]
    fn empty_recovery_feedback_escalates_after_two() {
        let msg1 = empty_recovery_feedback(1, "x");
        let msg2 = empty_recovery_feedback(2, "x");
        assert!(msg2.contains("Do NOT write"), "{msg2}");
        assert!(msg2.contains("nothing else"), "{msg2}");
        assert_ne!(
            msg2, msg1,
            "escalated message must differ from first-empty message"
        );
    }

    #[test]
    fn format_no_match_handles_multibyte_boundary() {
        let input = "a".repeat(199) + "é" + "bbb";
        let result = format_no_match(&input);
        assert!(result.contains("No tool call"), "{result}");
    }
}
