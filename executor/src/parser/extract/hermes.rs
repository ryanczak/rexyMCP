use serde_json::Value;

use super::super::{Candidate, Format};

const OPEN_TAG: &str = "<tool_call>";
const CLOSE_TAG: &str = "</tool_call>";

/// Hermes-style extractor.
///
/// Walks the response and extracts every `<tool_call>...</tool_call>` substring
/// as a `Candidate`. Uses balanced-brace counting on the JSON body — the literal
/// `</tool_call>` closing tag is **a hint, not a delimiter**. This is correct when
/// the body contains the literal text `</tool_call>` inside a JSON string value.
///
/// Returns one `Candidate` per detected `<tool_call>` opening, even if the body
/// fails to parse as JSON. Malformed candidates have `name: None` and
/// `arguments: None`; the repair pass may fix them later. Empty `Vec` means no
/// `<tool_call>` tags were found.
pub fn extract(response: &str) -> Vec<Candidate> {
    let bytes = response.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0usize;

    while let Some(rel) = response[pos..].find(OPEN_TAG) {
        let open_start = pos + rel;
        let after_open = open_start + OPEN_TAG.len();

        // Skip whitespace, find the first '{'.
        let mut i = after_open;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'{' {
            // No JSON body after this opening tag; skip past it.
            pos = after_open;
            continue;
        }

        match find_balanced_end(bytes, i) {
            Some(end_exclusive) => {
                let body = &response[i..end_exclusive];
                let candidate = parse_body(body);
                out.push(candidate);

                // Advance past the body. If a literal close tag follows, step over
                // it; otherwise leave the cursor at the body end.
                let mut next = end_exclusive;
                let rest = &response[next..];
                let leading_ws_len = rest.len() - rest.trim_start().len();
                if response[next + leading_ws_len..].starts_with(CLOSE_TAG) {
                    next += leading_ws_len + CLOSE_TAG.len();
                }
                pos = next;
            }
            None => {
                // Unmatched braces / unterminated string. Skip past the opening
                // tag and keep looking; do not emit a candidate for a truncated
                // body.
                pos = after_open;
            }
        }
    }

    out
}

/// Scan from `start` (which must point at `{`) and return the exclusive end index
/// of the matching `}`, respecting JSON string literals (braces inside `"..."`
/// don't count; `\"` is escaped). Returns `None` if the braces never balance.
fn find_balanced_end(bytes: &[u8], start: usize) -> Option<usize> {
    debug_assert_eq!(bytes[start], b'{');

    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut i = start;

    while i < bytes.len() {
        let b = bytes[i];
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
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }

    None
}

fn parse_body(body: &str) -> Candidate {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let (name, arguments) = match parsed {
        Some(Value::Object(map)) => {
            let name = map
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let arguments = map.get("arguments").cloned();
            (name, arguments)
        }
        _ => (None, None),
    };

    Candidate {
        format: Format::Hermes,
        name,
        arguments,
        score: 0,
        repairs_attempted: Vec::new(),
        raw_body: Some(body.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_single_hermes_tool_call() {
        let input =
            "<tool_call>{\"name\":\"read_file\",\"arguments\":{\"path\":\"x\"}}</tool_call>";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.format, Format::Hermes);
        assert_eq!(c.name.as_deref(), Some("read_file"));
        assert_eq!(c.arguments.as_ref().unwrap(), &json!({"path": "x"}));
        assert_eq!(c.score, 0);
        assert!(c.repairs_attempted.is_empty());
    }

    #[test]
    fn extracts_multiple_hermes_tool_calls() {
        let input = "<tool_call>{\"name\":\"a\",\"arguments\":{}}</tool_call> middle <tool_call>{\"name\":\"b\",\"arguments\":{}}</tool_call>";
        let out = extract(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name.as_deref(), Some("a"));
        assert_eq!(out[1].name.as_deref(), Some("b"));
    }

    #[test]
    fn handles_body_containing_close_tag_in_string_literal() {
        let input = "<tool_call>{\"name\":\"patch\",\"arguments\":{\"new_str\":\"</tool_call>\"}}</tool_call>";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let args = out[0].arguments.as_ref().unwrap();
        assert_eq!(args["new_str"], "</tool_call>");
    }

    #[test]
    fn handles_body_containing_brace_in_string_literal() {
        let input = "<tool_call>{\"name\":\"write_file\",\"arguments\":{\"content\":\"fn x() { }\"}}</tool_call>";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let args = out[0].arguments.as_ref().unwrap();
        assert_eq!(args["content"], "fn x() { }");
    }

    #[test]
    fn handles_missing_close_tag() {
        let input = "<tool_call>{\"name\":\"x\",\"arguments\":{}}";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name.as_deref(), Some("x"));
    }

    #[test]
    fn emits_malformed_candidate_for_unparseable_body() {
        let input = "<tool_call>{not valid json}</tool_call>";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].format, Format::Hermes);
        assert_eq!(out[0].name, None);
        assert_eq!(out[0].arguments, None);
    }

    #[test]
    fn returns_empty_for_no_hermes_tags() {
        let out = extract("plain text with no tool calls");
        assert!(out.is_empty());
    }

    #[test]
    fn populates_raw_body() {
        let input =
            "<tool_call>{\"name\":\"read_file\",\"arguments\":{\"path\":\"x\"}}</tool_call>";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].raw_body.as_deref(),
            Some("{\"name\":\"read_file\",\"arguments\":{\"path\":\"x\"}}")
        );
    }
}
