use serde_json::Value;

use super::super::{Candidate, Format};

const OPEN_TAG: &str = "<function=";
const CLOSE_TAG: &str = "</function>";

/// XML-variant extractor.
///
/// Finds `<function=NAME>{json}</function>` tags. The name comes from the tag;
/// the JSON object is the arguments.
pub fn extract(response: &str) -> Vec<Candidate> {
    let bytes = response.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0usize;

    while let Some(rel) = response[pos..].find(OPEN_TAG) {
        let tag_start = pos + rel;
        let after_tag = tag_start + OPEN_TAG.len();

        let name_end = match bytes[after_tag..].iter().position(|&b| b == b'>') {
            Some(p) => after_tag + p,
            None => {
                pos = after_tag;
                continue;
            }
        };

        let name_bytes = &bytes[after_tag..name_end];
        let name_str = match std::str::from_utf8(name_bytes) {
            Ok(s) => s.trim(),
            Err(_) => {
                pos = name_end + 1;
                continue;
            }
        };
        if !name_str.chars().all(|c| c.is_alphanumeric() || c == '_') || name_str.is_empty() {
            pos = name_end + 1;
            continue;
        }
        let name = name_str.to_string();

        let mut i = name_end + 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'{' {
            pos = name_end + 1;
            continue;
        }

        match find_balanced_end(bytes, i) {
            Some(end_exclusive) => {
                let body = &response[i..end_exclusive];
                let arguments = parse_body(body);
                out.push(Candidate {
                    format: Format::XmlVariant,
                    name: Some(name),
                    arguments,
                    score: 0,
                    repairs_attempted: Vec::new(),
                    raw_body: Some(body.to_string()),
                });

                let mut next = end_exclusive;
                let rest = &response[next..];
                let leading_ws_len = rest.len() - rest.trim_start().len();
                let after_ws = next + leading_ws_len;
                if response.get(after_ws..after_ws + CLOSE_TAG.len()) == Some(CLOSE_TAG) {
                    next = after_ws + CLOSE_TAG.len();
                }
                pos = next;
            }
            None => {
                pos = name_end + 1;
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

fn parse_body(body: &str) -> Option<Value> {
    serde_json::from_str(body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_xml_variant_call() {
        let input = "<function=read_file>{\"path\":\"x\"}</function>";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name.as_deref(), Some("read_file"));
        assert_eq!(out[0].arguments.as_ref().unwrap(), &json!({"path": "x"}));
    }

    #[test]
    fn name_comes_from_tag_not_body() {
        let input = "<function=read_file>{\"path\":\"x\",\"name\":\"WRONG\"}</function>";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name.as_deref(), Some("read_file"));
    }

    #[test]
    fn handles_body_containing_close_tag_in_string() {
        let input = "<function=patch>{\"new_str\":\"</function>\"}</function>";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let args = out[0].arguments.as_ref().unwrap();
        assert_eq!(args["new_str"], "</function>");
    }

    #[test]
    fn returns_empty_for_no_function_tag() {
        let input = "just some text with no tags";
        let out = extract(input);
        assert!(out.is_empty());
    }

    #[test]
    fn populates_raw_body() {
        let input = "<function=read_file>{\"path\":\"x\"}</function>";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].raw_body.as_deref(), Some("{\"path\":\"x\"}"));
    }
}
