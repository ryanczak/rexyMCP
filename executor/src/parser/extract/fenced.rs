use serde_json::Value;

use super::super::{Candidate, Format};

const FENCE: &str = "```json";

/// Fenced-JSON extractor.
///
/// Pulls all ` ```json … ``` ` blocks. Uses balanced-brace counting on the JSON
/// body — the fence boundary is a hint, not a delimiter.
pub fn extract(response: &str) -> Vec<Candidate> {
    let bytes = response.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0usize;

    while let Some(rel) = response[pos..].find(FENCE) {
        let after_fence = pos + rel + FENCE.len();

        let mut i = after_fence;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'{' {
            pos = after_fence;
            continue;
        }

        match find_balanced_end(bytes, i) {
            Some(end_exclusive) => {
                let body = &response[i..end_exclusive];
                let candidate = parse_body(body);
                out.push(candidate);

                let mut next = end_exclusive;
                let rest = &response[next..];
                let leading_ws_len = rest.len() - rest.trim_start().len();
                let after_ws = next + leading_ws_len;
                if response.get(after_ws..after_ws + 3) == Some("```") {
                    next = after_ws + 3;
                }
                pos = next;
            }
            None => {
                pos = after_fence;
            }
        }
    }

    out
}

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
        format: Format::FencedJson,
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
    fn extracts_fenced_json_with_name_and_arguments() {
        let input = "```json\n{\"name\":\"x\",\"arguments\":{\"a\":1}}\n```";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.format, Format::FencedJson);
        assert_eq!(c.name.as_deref(), Some("x"));
        assert_eq!(c.arguments.as_ref().unwrap(), &json!({"a": 1}));
        assert_eq!(c.score, 0);
        assert!(c.repairs_attempted.is_empty());
    }

    #[test]
    fn handles_body_containing_backtick_in_string() {
        let input =
            "```json\n{\"name\":\"x\",\"arguments\":{\"code\":\"fn x() { `backtick` }\"}}\n```";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let args = out[0].arguments.as_ref().unwrap();
        assert_eq!(args["code"], "fn x() { `backtick` }");
    }

    #[test]
    fn extracts_multiple_fenced_blocks() {
        let input = "```json\n{\"name\":\"a\",\"arguments\":{}}\n```\nsome text\n```json\n{\"name\":\"b\",\"arguments\":{}}\n```";
        let out = extract(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name.as_deref(), Some("a"));
        assert_eq!(out[1].name.as_deref(), Some("b"));
    }

    #[test]
    fn emits_malformed_candidate_for_bad_json_body() {
        let input = "```json\n{not json}\n```";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].format, Format::FencedJson);
        assert_eq!(out[0].name, None);
        assert_eq!(out[0].arguments, None);
    }

    #[test]
    fn populates_raw_body() {
        let input = "```json\n{\"name\":\"x\",\"arguments\":{\"a\":1}}\n```";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].raw_body.as_deref(),
            Some("{\"name\":\"x\",\"arguments\":{\"a\":1}}")
        );
    }
}
