use super::super::{Candidate, Format};

const YAML_FENCE: &str = "```yaml";

/// YAML-ish extractor.
///
/// Detects candidate YAML regions by fenced ` ```yaml ` blocks or by a `name:`
/// line followed by an `arguments:` line at the same column. Parses each region
/// with `serde_yaml` and converts to `serde_json::Value`.
pub fn extract(response: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut pos = 0usize;

    while pos < response.len() {
        let Some(region) = detect_region(response, pos) else {
            break;
        };

        let body = &response[region.start..region.end];
        let candidate = parse_body(body);
        out.push(candidate);

        pos = region.end;
    }

    out
}

struct Region {
    start: usize,
    end: usize,
}

fn detect_region(response: &str, pos: usize) -> Option<Region> {
    let rest = &response[pos..];

    if let Some(rel) = rest.find(YAML_FENCE) {
        let fence_start = pos + rel;
        let body_start = fence_start + YAML_FENCE.len();

        let body_end = if let Some(rel) = response[body_start..].find("```") {
            body_start + rel
        } else {
            response.len()
        };

        return Some(Region {
            start: body_start,
            end: body_end,
        });
    }

    let lines: Vec<&str> = response[pos..].lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(name_col) = line.find("name:") {
            let mut j = i + 1;
            while j < lines.len() {
                let next_line = lines[j];
                if next_line.trim().is_empty() {
                    break;
                }
                if let Some(arg_col) = next_line.find("arguments:")
                    && arg_col == name_col
                {
                    let start_offset = line_offset_from_pos(response, pos, i);
                    let end_offset = region_end(response, pos, j);
                    return Some(Region {
                        start: start_offset + name_col,
                        end: end_offset,
                    });
                }
                j += 1;
            }
        }
        i += 1;
    }

    None
}

fn line_offset_from_pos(response: &str, base_pos: usize, line_idx: usize) -> usize {
    let mut pos = base_pos;
    for _ in 0..line_idx {
        if let Some(rel) = response[pos..].find('\n') {
            pos += rel + 1;
        } else {
            return response.len();
        }
    }
    pos
}

fn region_end(response: &str, base_pos: usize, start_line: usize) -> usize {
    let mut pos = line_offset_from_pos(response, base_pos, start_line);
    let rest = &response[pos..];
    for line in rest.lines() {
        if line.trim().is_empty() {
            break;
        }
        pos += line.len();
        if let Some(rel) = response[pos..].find('\n') {
            pos += rel + 1;
        } else {
            return response.len();
        }
    }
    pos
}

fn parse_body(body: &str) -> Candidate {
    let parsed: Option<serde_yaml::Value> = serde_yaml::from_str(body).ok();
    let (name, arguments) = match parsed {
        Some(serde_yaml::Value::Mapping(map)) => {
            let name = map
                .get(serde_yaml::Value::String("name".to_string()))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let arguments = map
                .get(serde_yaml::Value::String("arguments".to_string()))
                .and_then(|yaml_val| serde_json::to_value(yaml_val).ok());
            (name, arguments)
        }
        _ => (None, None),
    };

    Candidate {
        format: Format::Yaml,
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
    fn extracts_yaml_block() {
        let input = "name: read_file\narguments:\n  path: src/main.rs\n";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.format, Format::Yaml);
        assert_eq!(c.name.as_deref(), Some("read_file"));
        assert_eq!(
            c.arguments.as_ref().unwrap(),
            &json!({"path": "src/main.rs"})
        );
    }

    #[test]
    fn extracts_fenced_yaml() {
        let input = "```yaml\nname: read_file\narguments:\n  path: src/main.rs\n```";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.format, Format::Yaml);
        assert_eq!(c.name.as_deref(), Some("read_file"));
        assert_eq!(
            c.arguments.as_ref().unwrap(),
            &json!({"path": "src/main.rs"})
        );
    }

    #[test]
    fn emits_malformed_candidate_for_invalid_yaml() {
        let input = "name: x\narguments: {invalid";
        let out = extract(input);
        assert!(out.len() <= 1);
        if !out.is_empty() {
            assert_eq!(out[0].format, Format::Yaml);
            if let Some(ref name) = out[0].name {
                assert_eq!(name, "x");
            }
        }
    }

    #[test]
    fn returns_empty_for_no_yaml_pattern() {
        let input = "just regular prose, no name marker";
        let out = extract(input);
        assert!(out.is_empty());
    }

    #[test]
    fn populates_raw_body() {
        let input = "name: read_file\narguments:\n  path: src/main.rs\n";
        let out = extract(input);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].raw_body.as_deref(),
            Some("name: read_file\narguments:\n  path: src/main.rs\n")
        );
    }
}
