use serde_json::Value;

use super::super::{Candidate, Format};

/// Loose-JSON extractor.
///
/// Scans for balanced-brace JSON objects embedded in prose. Only emits candidates
/// whose parsed JSON has a `"name"` string field, to avoid noisy false positives
/// from incidental `{}` in text.
pub fn extract(response: &str) -> Vec<Candidate> {
    let bytes = response.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0usize;

    while pos < bytes.len() {
        let Some(rel) = response[pos..].find('{') else {
            break;
        };
        let start = pos + rel;

        match find_balanced_end(bytes, start) {
            Some(end_exclusive) => {
                let body = &response[start..end_exclusive];
                if let Some(candidate) = parse_body(body) {
                    out.push(candidate);
                }
                pos = end_exclusive;
            }
            None => {
                pos = start + 1;
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

fn parse_body(body: &str) -> Option<Candidate> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let Value::Object(map) = parsed else {
        return None;
    };

    let name = map.get("name").and_then(|v| v.as_str())?;
    let name = Some(name.to_string());
    let arguments = map.get("arguments").cloned();

    Some(Candidate {
        format: Format::LooseJson,
        name,
        arguments,
        score: 0,
        repairs_attempted: Vec::new(),
        raw_body: Some(body.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_loose_json_with_name_field() {
        let input = "here is a call: {\"name\":\"read\",\"arguments\":{\"x\":1}} thanks";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.format, Format::LooseJson);
        assert_eq!(c.name.as_deref(), Some("read"));
        assert_eq!(c.arguments.as_ref().unwrap(), &json!({"x": 1}));
    }

    #[test]
    fn filters_out_objects_without_name_field() {
        let input = "the config is {\"a\": 1, \"b\": 2}";
        let out = extract(input);
        assert!(out.is_empty());
    }

    #[test]
    fn extracts_multiple_loose_json_objects() {
        let input =
            "first {\"name\":\"a\",\"arguments\":{}} and second {\"name\":\"b\",\"arguments\":{}}";
        let out = extract(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name.as_deref(), Some("a"));
        assert_eq!(out[1].name.as_deref(), Some("b"));
    }

    #[test]
    fn does_not_scan_nested_arguments() {
        let input = "{\"name\":\"x\",\"arguments\":{\"name\":\"y\"}}";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name.as_deref(), Some("x"));
    }

    #[test]
    fn populates_raw_body() {
        let input = "prose {\"name\":\"x\",\"arguments\":{}} more prose";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].raw_body.as_deref(),
            Some("{\"name\":\"x\",\"arguments\":{}}")
        );
    }
}
