use serde_json::Value;

use super::{Candidate, Origin, ToolCall};
use crate::tools::ToolRegistry;

/// Validate a (possibly-repaired) candidate against the registry's tool schemas.
///
/// On success: a `ToolCall` ready for dispatch. On failure: a structured
/// `ValidationError` the feedback formatter consumes.
pub fn validate(
    candidate: &Candidate,
    registry: &ToolRegistry,
) -> Result<ToolCall, ValidationError> {
    match &candidate.name {
        None => {
            return Err(ValidationError::UnknownTool {
                attempted_name: None,
                available_tools: sorted_tool_names(registry),
            });
        }
        Some(name) => {
            if registry.get(name).is_none() {
                return Err(ValidationError::UnknownTool {
                    attempted_name: Some(name.clone()),
                    available_tools: sorted_tool_names(registry),
                });
            }
        }
    }

    let name = candidate.name.clone().expect("name checked Some above");
    let tool = registry.get(&name).expect("tool existence checked above");
    let schema = tool.schema();

    let required: Vec<String> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let args = match &candidate.arguments {
        Some(Value::Object(map)) => map.clone(),
        _ => serde_json::Map::new(),
    };

    let missing_required: Vec<String> = required
        .iter()
        .filter(|r| !args.contains_key(r.as_str()))
        .cloned()
        .collect();

    let unknown_params: Vec<String> = args
        .keys()
        .filter(|k| !properties.contains_key(k.as_str()))
        .cloned()
        .collect();

    let type_mismatches: Vec<TypeMismatch> = args
        .iter()
        .filter_map(|(key, val)| {
            let prop_schema = properties.get(key)?;
            let expected_type = prop_schema.get("type")?.as_str()?;
            if !type_matches(expected_type, val) {
                Some(TypeMismatch {
                    field: key.clone(),
                    expected_type: expected_type.to_string(),
                    actual_value: val.clone(),
                })
            } else {
                None
            }
        })
        .collect();

    if !missing_required.is_empty() || !unknown_params.is_empty() || !type_mismatches.is_empty() {
        return Err(ValidationError::SchemaFailures {
            tool: name,
            missing_required,
            unknown_params,
            type_mismatches,
        });
    }

    let origin = if candidate.repairs_attempted.is_empty() {
        Origin::Extracted {
            format: candidate.format,
        }
    } else {
        Origin::Repaired {
            format: candidate.format,
            repairs: candidate.repairs_attempted.clone(),
        }
    };

    Ok(ToolCall {
        name,
        arguments: Value::Object(args),
        origin,
    })
}

#[derive(Debug, Clone)]
pub enum ValidationError {
    UnknownTool {
        attempted_name: Option<String>,
        available_tools: Vec<String>,
    },
    SchemaFailures {
        tool: String,
        missing_required: Vec<String>,
        unknown_params: Vec<String>,
        type_mismatches: Vec<TypeMismatch>,
    },
}

#[derive(Debug, Clone)]
pub struct TypeMismatch {
    pub field: String,
    pub expected_type: String,
    pub actual_value: serde_json::Value,
}

fn sorted_tool_names(registry: &ToolRegistry) -> Vec<String> {
    let mut names: Vec<String> = registry.all().map(|t| t.name().to_string()).collect();
    names.sort();
    names
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

#[cfg(test)]
mod tests {
    use super::super::{Format, RepairOp};
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

    fn make_candidate(
        name: Option<&str>,
        args: Option<Value>,
        repairs: Vec<RepairOp>,
    ) -> Candidate {
        Candidate {
            format: Format::Hermes,
            name: name.map(String::from),
            arguments: args,
            score: 0,
            repairs_attempted: repairs,
            raw_body: None,
        }
    }

    #[test]
    fn validates_ok_when_all_required_present_and_typed() {
        let registry = test_registry();
        let candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"path": "x"})),
            vec![],
        );
        let result = validate(&candidate, &registry).unwrap();
        assert_eq!(result.name, "read_file");
        assert!(matches!(
            result.origin,
            Origin::Extracted {
                format: Format::Hermes
            }
        ));
    }

    #[test]
    fn unknown_tool_name_errors() {
        let registry = test_registry();
        let candidate = make_candidate(Some("nonexistent"), Some(serde_json::json!({})), vec![]);
        let err = validate(&candidate, &registry).unwrap_err();
        match err {
            ValidationError::UnknownTool { attempted_name, .. } => {
                assert_eq!(attempted_name, Some("nonexistent".to_string()));
            }
            _ => panic!("expected UnknownTool"),
        }
    }

    #[test]
    fn unknown_tool_when_name_none() {
        let registry = test_registry();
        let candidate = make_candidate(None, Some(serde_json::json!({})), vec![]);
        let err = validate(&candidate, &registry).unwrap_err();
        match err {
            ValidationError::UnknownTool { attempted_name, .. } => {
                assert!(attempted_name.is_none());
            }
            _ => panic!("expected UnknownTool"),
        }
    }

    #[test]
    fn available_tools_listed_in_unknown_tool_error() {
        let registry = test_registry();
        let candidate = make_candidate(Some("nonexistent"), Some(serde_json::json!({})), vec![]);
        let err = validate(&candidate, &registry).unwrap_err();
        match err {
            ValidationError::UnknownTool {
                available_tools, ..
            } => {
                assert!(available_tools.contains(&"read_file".to_string()));
                assert!(available_tools.contains(&"write_file".to_string()));
                assert!(available_tools.contains(&"patch".to_string()));
                assert!(available_tools.contains(&"search".to_string()));
                assert!(available_tools.contains(&"find_files".to_string()));
                assert!(available_tools.contains(&"bash".to_string()));
                let mut sorted = available_tools.clone();
                sorted.sort();
                assert_eq!(available_tools, sorted);
            }
            _ => panic!("expected UnknownTool"),
        }
    }

    #[test]
    fn missing_required_param_errors() {
        let registry = test_registry();
        let candidate = make_candidate(Some("read_file"), Some(serde_json::json!({})), vec![]);
        let err = validate(&candidate, &registry).unwrap_err();
        match err {
            ValidationError::SchemaFailures {
                missing_required, ..
            } => {
                assert!(missing_required.contains(&"path".to_string()));
            }
            _ => panic!("expected SchemaFailures"),
        }
    }

    #[test]
    fn unknown_param_errors() {
        let registry = test_registry();
        let candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"path": "x", "bogus": 1})),
            vec![],
        );
        let err = validate(&candidate, &registry).unwrap_err();
        match err {
            ValidationError::SchemaFailures { unknown_params, .. } => {
                assert!(unknown_params.contains(&"bogus".to_string()));
            }
            _ => panic!("expected SchemaFailures"),
        }
    }

    #[test]
    fn type_mismatch_errors() {
        let registry = test_registry();
        let candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"path": 42})),
            vec![],
        );
        let err = validate(&candidate, &registry).unwrap_err();
        match err {
            ValidationError::SchemaFailures {
                type_mismatches, ..
            } => {
                assert_eq!(type_mismatches.len(), 1);
                assert_eq!(type_mismatches[0].field, "path");
                assert_eq!(type_mismatches[0].expected_type, "string");
            }
            _ => panic!("expected SchemaFailures"),
        }
    }

    #[test]
    fn multiple_issues_all_reported() {
        let registry = test_registry();
        let candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"bogus": 1})),
            vec![],
        );
        let err = validate(&candidate, &registry).unwrap_err();
        match err {
            ValidationError::SchemaFailures {
                missing_required,
                unknown_params,
                ..
            } => {
                assert!(missing_required.contains(&"path".to_string()));
                assert!(unknown_params.contains(&"bogus".to_string()));
            }
            _ => panic!("expected SchemaFailures"),
        }
    }

    #[test]
    fn repaired_candidate_has_repaired_origin() {
        let registry = test_registry();
        let candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"path": "x"})),
            vec![RepairOp::NameFuzzyMatch {
                from: "read_fil".to_string(),
                to: "read_file".to_string(),
            }],
        );
        let result = validate(&candidate, &registry).unwrap();
        assert!(matches!(
            result.origin,
            Origin::Repaired { format: Format::Hermes, repairs } if repairs.len() == 1
        ));
    }

    #[test]
    fn unrepaired_candidate_has_extracted_origin() {
        let registry = test_registry();
        let candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"path": "x"})),
            vec![],
        );
        let result = validate(&candidate, &registry).unwrap();
        assert!(matches!(
            result.origin,
            Origin::Extracted {
                format: Format::Hermes
            }
        ));
    }
}
