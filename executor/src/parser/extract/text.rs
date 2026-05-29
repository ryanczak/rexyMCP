use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value};

use super::super::{Candidate, Format};

fn re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\w+)\s*\(([^)]*)\)").unwrap())
}

/// Plain-text imperative extractor.
///
/// Matches `name(arg=value, ...)` patterns. Values are parsed as JSON when
/// possible, falling back to raw strings.
pub fn extract(response: &str) -> Vec<Candidate> {
    let mut out = Vec::new();

    for cap in re().captures_iter(response) {
        let name = cap.get(1).unwrap().as_str().to_string();
        let raw_args = cap.get(2).unwrap().as_str();
        let arguments = parse_args(raw_args);

        out.push(Candidate {
            format: Format::PlainText,
            name: Some(name),
            arguments: Some(Value::Object(arguments)),
            score: 0,
            repairs_attempted: Vec::new(),
            raw_body: Some(raw_args.to_string()),
        });
    }

    out
}

fn parse_args(raw: &str) -> Map<String, Value> {
    let mut map = Map::new();

    if raw.trim().is_empty() {
        return map;
    }

    for token in raw.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let Some(eq_pos) = token.find('=') else {
            continue;
        };
        let key = token[..eq_pos].trim().to_string();
        let value = token[eq_pos + 1..].trim();

        let parsed =
            serde_json::from_str::<Value>(value).unwrap_or(Value::String(value.to_string()));
        map.insert(key, parsed);
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_plain_text_call_with_quoted_value() {
        let input = "read_file(path=\"src/main.rs\")";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.format, Format::PlainText);
        assert_eq!(c.name.as_deref(), Some("read_file"));
        assert_eq!(
            c.arguments.as_ref().unwrap(),
            &json!({"path": "src/main.rs"})
        );
    }

    #[test]
    fn extracts_plain_text_call_with_unquoted_value() {
        let input = "read_file(path=src/main.rs)";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.name.as_deref(), Some("read_file"));
        assert_eq!(
            c.arguments.as_ref().unwrap(),
            &json!({"path": "src/main.rs"})
        );
    }

    #[test]
    fn extracts_multiple_args() {
        let input = "bash(command=\"ls\", timeout_secs=30)";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let args = out[0].arguments.as_ref().unwrap();
        assert_eq!(args["command"], "ls");
        assert_eq!(args["timeout_secs"], 30);
    }

    #[test]
    fn extracts_multiple_calls() {
        let input = "read_file(path=a) write_file(path=b, content=\"hi\")";
        let out = extract(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name.as_deref(), Some("read_file"));
        assert_eq!(out[1].name.as_deref(), Some("write_file"));
    }

    #[test]
    fn populates_raw_body() {
        let input = "read_file(path=x, n=1)";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].raw_body.as_deref(), Some("path=x, n=1"));
    }
}
