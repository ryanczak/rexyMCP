use crate::tools::ToolRegistry;
use serde_json::Value;

use super::super::{Candidate, Format, RepairOp};

/// JSON syntax repair.
///
/// Attempts to repair common JSON syntax errors in `candidate.raw_body` and
/// re-parse. On a clean re-parse, repopulates the candidate's `name` and
/// `arguments` (per the candidate's format) and appends `RepairOp::JsonRepair`.
/// Returns 1 on a successful repair; 0 otherwise.
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

    let repaired = repair_all(&raw_body);
    let parsed: Value = match serde_json::from_str(&repaired) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    populate_from_parsed(candidate, &parsed);
    candidate.repairs_attempted.push(RepairOp::JsonRepair);
    1
}

fn repair_all(input: &str) -> String {
    let s = strip_trailing_commas(input);
    let s = quote_unquoted_keys(&s);
    let s = convert_single_quotes(&s);
    close_unclosed_braces(&s)
}

fn strip_trailing_commas(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
                out.push(b as char);
            } else if b == b'\\' {
                escaped = true;
                out.push(b as char);
            } else if b == b'"' {
                in_string = false;
                out.push(b as char);
            } else {
                out.push(b as char);
            }
        } else {
            match b {
                b'"' => {
                    in_string = true;
                    out.push(b as char);
                }
                b',' => {
                    let mut j = i + 1;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < bytes.len() && (bytes[j] == b'}' || bytes[j] == b']') {
                        i = j;
                        out.push(bytes[j] as char);
                    } else {
                        out.push(b as char);
                    }
                }
                _ => {
                    out.push(b as char);
                }
            }
        }
        i += 1;
    }

    out
}

fn quote_unquoted_keys(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
                out.push(b as char);
            } else if b == b'\\' {
                escaped = true;
                out.push(b as char);
            } else if b == b'"' {
                in_string = false;
                out.push(b as char);
            } else {
                out.push(b as char);
            }
        } else {
            match b {
                b'"' => {
                    in_string = true;
                    out.push(b as char);
                }
                b'{' | b',' | b'[' => {
                    out.push(b as char);
                    if let Some((key_end, rest)) = scan_unquoted_key(bytes, i + 1) {
                        let key = &input[i + 1..key_end];
                        if !key.is_empty()
                            && key
                                .chars()
                                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                        {
                            out.push('"');
                            out.push_str(key);
                            out.push('"');
                            i = rest;
                            continue;
                        }
                    }
                }
                _ => {
                    out.push(b as char);
                }
            }
        }
        i += 1;
    }

    out
}

fn scan_unquoted_key(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let key_start = i;
    while i < bytes.len()
        && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
    {
        i += 1;
    }
    if i == key_start {
        return None;
    }
    let key_end = i;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b':' {
        Some((key_end, i))
    } else {
        None
    }
}

fn convert_single_quotes(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut in_double_string = false;
    let mut escaped = false;

    for &b in bytes {
        if in_double_string {
            if escaped {
                escaped = false;
                out.push(b as char);
            } else if b == b'\\' {
                escaped = true;
                out.push(b as char);
            } else if b == b'"' {
                in_double_string = false;
                out.push(b as char);
            } else {
                out.push(b as char);
            }
        } else {
            match b {
                b'"' => {
                    in_double_string = true;
                    out.push(b as char);
                }
                b'\'' => {
                    out.push('"');
                }
                _ => {
                    out.push(b as char);
                }
            }
        }
    }

    out
}

fn close_unclosed_braces(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut depth_brace: i32 = 0;
    let mut depth_bracket: i32 = 0;
    let mut in_string = false;
    let mut escaped = false;

    for &b in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'{' => depth_brace += 1,
                b'}' => depth_brace -= 1,
                b'[' => depth_bracket += 1,
                b']' => depth_bracket -= 1,
                _ => {}
            }
        }
    }

    if depth_brace < 0 || depth_bracket < 0 {
        return input.to_string();
    }

    let mut out = input.to_string();
    for _ in 0..depth_bracket {
        out.push(']');
    }
    for _ in 0..depth_brace {
        out.push('}');
    }
    out
}

pub(crate) fn populate_from_parsed(candidate: &mut Candidate, parsed: &Value) {
    if let Value::Object(map) = parsed {
        match candidate.format {
            Format::Hermes | Format::FencedJson | Format::LooseJson | Format::Yaml => {
                if let Some(name_val) = map.get("name")
                    && let Some(name) = name_val.as_str()
                {
                    candidate.name = Some(name.to_string());
                }
                if let Some(args) = map.get("arguments") {
                    candidate.arguments = Some(args.clone());
                }
            }
            Format::XmlVariant => {
                candidate.arguments = Some(parsed.clone());
            }
            Format::PlainText => {}
        }
    }
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
    fn repairs_trailing_comma_in_object() {
        let mut c = make_candidate(r#"{"name":"x","arguments":{"a":1,}}"#);
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 1);
        assert_eq!(c.arguments.as_ref().unwrap(), &json!({"a": 1}));
        assert_eq!(c.repairs_attempted.len(), 1);
        assert!(matches!(&c.repairs_attempted[0], RepairOp::JsonRepair));
    }

    #[test]
    fn repairs_trailing_comma_in_array() {
        let mut c = make_candidate(r#"{"name":"x","arguments":{"a":[1,2,]}}"#);
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 1);
        assert_eq!(c.arguments.as_ref().unwrap(), &json!({"a": [1, 2]}));
    }

    #[test]
    fn repairs_unquoted_keys() {
        let mut c = make_candidate(r#"{name: "x"}"#);
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 1);
        assert_eq!(c.name.as_deref(), Some("x"));
    }

    #[test]
    fn repairs_single_quoted_strings() {
        let mut c = make_candidate(r#"{'name': 'x'}"#);
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 1);
        assert_eq!(c.name.as_deref(), Some("x"));
    }

    #[test]
    fn repairs_unclosed_brace() {
        let mut c = make_candidate(r#"{"name":"x","arguments":{"#);
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 1);
        assert_eq!(c.name.as_deref(), Some("x"));
        assert!(c.arguments.is_some());
    }

    #[test]
    fn combines_multiple_repairs() {
        let mut c = make_candidate(r#"{'name':'x','arguments':{'a':1,}"#);
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 1);
        assert_eq!(c.name.as_deref(), Some("x"));
        assert_eq!(c.arguments.as_ref().unwrap(), &json!({"a": 1}));
        assert_eq!(c.repairs_attempted.len(), 1);
    }

    #[test]
    fn no_op_when_arguments_already_some() {
        let mut c = Candidate {
            format: Format::Hermes,
            name: None,
            arguments: Some(json!({"a": 1})),
            score: 0,
            repairs_attempted: Vec::new(),
            raw_body: Some(r#"{"a":1,}"#.to_string()),
        };
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 0);
        assert_eq!(c.repairs_attempted.len(), 0);
    }

    #[test]
    fn no_op_when_format_is_plain_text() {
        let mut c = Candidate {
            format: Format::PlainText,
            name: None,
            arguments: None,
            score: 0,
            repairs_attempted: Vec::new(),
            raw_body: Some(r#"{"a":1,}"#.to_string()),
        };
        let result = apply(&mut c, &ToolRegistry::new(), 4);
        assert_eq!(result, 0);
    }
}
