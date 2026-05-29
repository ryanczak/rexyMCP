use crate::tools::ToolRegistry;
use serde_json::Value;

use super::super::{Candidate, Format, RepairOp};
use super::json::populate_from_parsed;

/// Newline-in-string-literal escape.
///
/// Walks `candidate.raw_body` and replaces literal `\n`, `\r`, and `\t`
/// characters inside JSON string literals with their escaped equivalents, then
/// attempts to re-parse. On a clean re-parse, repopulates the candidate's `name`
/// and `arguments` and appends `RepairOp::NewlineEscape`.
pub fn apply(candidate: &mut Candidate, _registry: &ToolRegistry, budget: usize) -> usize {
    if budget == 0 {
        return 0;
    }
    if candidate.arguments.is_some() {
        return 0;
    }
    let raw_body = match &candidate.raw_body {
        Some(b) => b.clone(),
        None => return 0,
    };
    if candidate.format == Format::PlainText {
        return 0;
    }

    let repaired = escape_control_chars(&raw_body);
    let parsed: Value = match serde_json::from_str(&repaired) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    populate_from_parsed(candidate, &parsed);
    candidate.repairs_attempted.push(RepairOp::NewlineEscape);
    1
}

fn escape_control_chars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;

    for ch in input.chars() {
        if in_string {
            if escaped {
                escaped = false;
                out.push(ch);
            } else if ch == '\\' {
                escaped = true;
                out.push(ch);
            } else if ch == '"' {
                in_string = false;
                out.push(ch);
            } else {
                match ch {
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push(ch),
                }
            }
        } else {
            if ch == '"' {
                in_string = true;
            }
            out.push(ch);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_candidate(raw_body: &str) -> Candidate {
        Candidate {
            format: Format::Hermes,
            name: None,
            arguments: None,
            score: 0,
            repairs_attempted: Vec::new(),
            raw_body: Some(raw_body.to_string()),
        }
    }

    #[test]
    fn escapes_literal_newline_in_string() {
        let body = "{\"name\":\"x\",\"arguments\":{\"a\":\"line one\nline two\"}}";
        let mut c = make_candidate(body);
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 1);
        assert_eq!(c.arguments.as_ref().unwrap()["a"], "line one\nline two");
        assert!(matches!(&c.repairs_attempted[0], RepairOp::NewlineEscape));
    }

    #[test]
    fn escapes_carriage_return_and_tab() {
        let body = "{\"name\":\"x\",\"arguments\":{\"a\":\"col1\tcol2\rval\"}}";
        let mut c = make_candidate(body);
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 1);
        assert_eq!(c.arguments.as_ref().unwrap()["a"], "col1\tcol2\rval");
    }

    #[test]
    fn does_not_touch_newlines_outside_strings() {
        let body = "{\n  \"name\": \"x\",\n  \"arguments\": {\"a\": 1}\n}\n";
        let mut c = make_candidate(body);
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 1);
        assert_eq!(c.arguments.as_ref().unwrap()["a"], 1);
    }

    #[test]
    fn no_op_when_arguments_already_some() {
        let mut c = Candidate {
            format: Format::Hermes,
            name: None,
            arguments: Some(json!({"a": 1})),
            score: 0,
            repairs_attempted: Vec::new(),
            raw_body: Some("garbage".to_string()),
        };
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 0);
        assert_eq!(c.repairs_attempted.len(), 0);
    }

    #[test]
    fn populates_name_and_arguments_per_format_after_repair() {
        let body = "{\"name\":\"x\",\"arguments\":{\"a\":\"b\nc\"}}";
        let mut c = make_candidate(body);
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 1);
        assert_eq!(c.name.as_deref(), Some("x"));
        assert_eq!(c.arguments.as_ref().unwrap(), &json!({"a": "b\nc"}));
    }
}
