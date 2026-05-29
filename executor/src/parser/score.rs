use serde_json::Value;

use super::Candidate;
use crate::tools::ToolRegistry;

/// Score a single candidate against the registry. Higher is better; negative
/// scores are possible (more wrong params than right). The caller assigns the
/// result to `Candidate.score` and picks the maximum-scoring candidate.
///
/// Weights: exact name +5, fuzzy name (Levenshtein ≤ 2) +3; required param
/// present +1 / missing -2; unknown param -1; argument type match +2.
pub fn score(candidate: &Candidate, registry: &ToolRegistry) -> i32 {
    let mut total = 0i32;

    // Name signal: find the matched tool (exact or fuzzy).
    let matched_tool = match &candidate.name {
        Some(name) => {
            if let Some(tool) = registry.get(name) {
                total += 5;
                Some(tool)
            } else {
                // Fuzzy match.
                let mut best_dist = usize::MAX;
                let mut best_name: Option<String> = None;
                for tool in registry.all() {
                    let d = levenshtein(name, tool.name());
                    if d <= 2 && d < best_dist {
                        best_dist = d;
                        best_name = Some(tool.name().to_string());
                    } else if d <= 2 && d == best_dist {
                        // Tiebreak: lexicographically smallest.
                        if let Some(ref bn) = best_name
                            && tool.name() < bn.as_str()
                        {
                            best_name = Some(tool.name().to_string());
                        }
                    }
                }
                if let Some(ref matched) = best_name {
                    total += 3;
                    registry.get(matched)
                } else {
                    None
                }
            }
        }
        None => None,
    };

    // Param signals require a matched tool and arguments.
    let (tool, args) = match (&matched_tool, &candidate.arguments) {
        (Some(tool), Some(Value::Object(map))) => (tool, map),
        _ => return total,
    };

    let schema = tool.schema();
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    // Required param signals.
    for req in required {
        if args.contains_key(req) {
            total += 1;
        } else {
            total -= 2;
        }
    }

    // Unknown param signals + type match signals.
    for (key, val) in args {
        if let Some(prop_schema) = properties.get(key) {
            if let Some(expected_type) = prop_schema.get("type").and_then(|v| v.as_str())
                && type_matches(expected_type, val)
            {
                total += 2;
            }
        } else {
            total -= 1;
        }
    }

    total
}

fn type_matches(schema_type: &str, value: &Value) -> bool {
    match schema_type {
        "string" => matches!(value, Value::String(_)),
        "integer" => {
            if let Value::Number(n) = value {
                n.is_i64() || n.is_u64()
            } else {
                false
            }
        }
        "number" => matches!(value, Value::Number(_)),
        "boolean" => matches!(value, Value::Bool(_)),
        "object" => matches!(value, Value::Object(_)),
        "array" => matches!(value, Value::Array(_)),
        _ => false,
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

    fn make_candidate(name: Option<&str>, args: Option<Value>) -> Candidate {
        use super::super::Format;
        Candidate {
            format: Format::Hermes,
            name: name.map(String::from),
            arguments: args,
            score: 0,
            repairs_attempted: Vec::new(),
            raw_body: None,
        }
    }

    #[test]
    fn exact_name_match_scores_plus_five() {
        let registry = test_registry();
        let candidate = make_candidate(Some("read_file"), Some(Value::Object(Default::default())));
        // +5 name, -2 missing required "path" = +3
        assert_eq!(score(&candidate, &registry), 3);
    }

    #[test]
    fn fuzzy_name_match_scores_plus_three() {
        let registry = test_registry();
        let candidate = make_candidate(Some("read_fil"), Some(Value::Object(Default::default())));
        // +3 fuzzy name, -2 missing required "path" = +1
        assert_eq!(score(&candidate, &registry), 1);
    }

    #[test]
    fn fuzzy_match_picks_smallest_distance() {
        let registry = test_registry();
        let candidate = make_candidate(Some("redfile"), Some(serde_json::json!({"path": "x"})));
        // "redfile" is distance 2 from "read_file": +3 fuzzy, +1 path, +2 type = +6
        assert_eq!(score(&candidate, &registry), 6);
    }

    #[test]
    fn name_too_far_scores_zero() {
        let registry = test_registry();
        let candidate = make_candidate(Some("xyzpdq"), Some(serde_json::json!({"path": "x"})));
        // No name match, no matched tool, so no param signals.
        assert_eq!(score(&candidate, &registry), 0);
    }

    #[test]
    fn none_name_scores_zero_on_name_signal() {
        let registry = test_registry();
        let candidate = make_candidate(None, None);
        assert_eq!(score(&candidate, &registry), 0);
    }

    #[test]
    fn required_param_present_scores_plus_one() {
        let registry = test_registry();
        let candidate = make_candidate(Some("read_file"), Some(serde_json::json!({"path": "x"})));
        // +5 exact name, +1 required present, +2 type match = +8
        assert_eq!(score(&candidate, &registry), 8);
    }

    #[test]
    fn required_param_missing_scores_minus_two() {
        let registry = test_registry();
        let candidate = make_candidate(Some("read_file"), Some(Value::Object(Default::default())));
        // +5 exact name, -2 required missing = +3
        assert_eq!(score(&candidate, &registry), 3);
    }

    #[test]
    fn unknown_param_scores_minus_one_per_key() {
        let registry = test_registry();
        let candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"path": "x", "bogus": 1, "extra": 2})),
        );
        // +5 name, +1 path present, +2 path type, -1 bogus, -1 extra = +6
        assert_eq!(score(&candidate, &registry), 6);
    }

    #[test]
    fn type_match_string_scores_plus_two() {
        let registry = test_registry();
        let candidate = make_candidate(Some("read_file"), Some(serde_json::json!({"path": "x"})));
        // +5 name, +1 present, +2 type match = +8
        assert_eq!(score(&candidate, &registry), 8);
    }

    #[test]
    fn type_mismatch_does_not_add() {
        let registry = test_registry();
        let candidate = make_candidate(Some("read_file"), Some(serde_json::json!({"path": 42})));
        // +5 name, +1 present, 0 type mismatch = +6
        assert_eq!(score(&candidate, &registry), 6);
    }
}
