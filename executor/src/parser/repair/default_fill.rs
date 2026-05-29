use serde_json::Value;
use std::sync::Arc;

use crate::tools::ToolRegistry;

use super::super::Candidate;

/// Apply up to `budget` default-fill transforms to the candidate: fill missing
/// non-required params that declare a `default`. Returns the number actually
/// applied (`0..=budget`); appends a `RepairOp::DefaultFill`. `budget == 0`
/// returns 0 immediately.
pub fn apply(candidate: &mut Candidate, registry: &ToolRegistry, budget: usize) -> usize {
    if budget == 0 {
        return 0;
    }

    let tool = match resolve_tool(candidate, registry) {
        Some(t) => t,
        None => return 0,
    };

    let args = match candidate.arguments.as_mut() {
        Some(Value::Object(map)) => map,
        _ => return 0,
    };

    let schema = tool.schema();
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    let properties = match schema.get("properties").and_then(|v| v.as_object()) {
        Some(p) => p,
        None => return 0,
    };

    let mut applied = 0;
    for (key, prop_schema) in properties {
        if applied >= budget {
            break;
        }
        if required.contains(&key.as_str()) {
            continue;
        }
        if args.contains_key(key) {
            continue;
        }
        let default_val = match prop_schema.get("default") {
            Some(d) => d,
            None => continue,
        };
        args.insert(key.clone(), default_val.clone());
        candidate
            .repairs_attempted
            .push(super::super::RepairOp::DefaultFill { field: key.clone() });
        applied += 1;
    }

    applied
}

fn resolve_tool(
    candidate: &Candidate,
    registry: &ToolRegistry,
) -> Option<Arc<dyn crate::tools::Tool>> {
    let name = candidate.name.as_deref()?;
    registry.get(name)
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::super::super::{Format, RepairOp};
    use super::*;
    use crate::tools::ToolResult;

    struct MockToolWithDefaults;

    #[async_trait]
    impl crate::tools::Tool for MockToolWithDefaults {
        fn name(&self) -> &str {
            "mock"
        }
        fn description(&self) -> &str {
            "test mock"
        }
        fn schema(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "required_no_default": { "type": "string" },
                    "required_with_default": { "type": "integer", "default": 7 },
                    "optional_with_default_a": { "type": "integer", "default": 42 },
                    "optional_with_default_b": { "type": "boolean", "default": true },
                    "optional_no_default": { "type": "string" }
                },
                "required": ["required_no_default", "required_with_default"]
            })
        }
        async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult {
                output: String::new(),
                error: None,
                metadata: None,
            })
        }
    }

    fn mock_registry() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        r.register(std::sync::Arc::new(MockToolWithDefaults));
        r
    }

    fn make_candidate(name: Option<&str>, args: Option<Value>) -> Candidate {
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
    fn fills_missing_optional_with_default() {
        let registry = mock_registry();
        let mut candidate = make_candidate(
            Some("mock"),
            Some(serde_json::json!({
                "required_no_default": "x",
                "required_with_default": 1
            })),
        );
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 2);

        let args = candidate.arguments.as_ref().unwrap().as_object().unwrap();
        assert_eq!(args["optional_with_default_a"], Value::Number(42.into()));
        assert_eq!(args["optional_with_default_b"], Value::Bool(true));

        assert_eq!(candidate.repairs_attempted.len(), 2);
        assert!(matches!(
            &candidate.repairs_attempted[0],
            RepairOp::DefaultFill { field } if field == "optional_with_default_a"
        ));
        assert!(matches!(
            &candidate.repairs_attempted[1],
            RepairOp::DefaultFill { field } if field == "optional_with_default_b"
        ));
    }

    #[test]
    fn does_not_fill_required_param_even_with_default() {
        let registry = mock_registry();
        let mut candidate = make_candidate(
            Some("mock"),
            Some(serde_json::json!({ "required_no_default": "x" })),
        );
        let applied = apply(&mut candidate, &registry, 4);

        // optional_with_default_a and optional_with_default_b get filled (2);
        // required_with_default has a default but is required → skip.
        assert_eq!(applied, 2);
        let args = candidate.arguments.as_ref().unwrap().as_object().unwrap();
        assert!(!args.contains_key("required_with_default"));
    }

    #[test]
    fn does_not_fill_already_present_param() {
        let registry = mock_registry();
        let mut candidate = make_candidate(
            Some("mock"),
            Some(serde_json::json!({
                "required_no_default": "x",
                "required_with_default": 1,
                "optional_with_default_a": 99
            })),
        );
        let applied = apply(&mut candidate, &registry, 4);

        // optional_with_default_a already present → skip; b still gets filled.
        assert_eq!(applied, 1);
        let args = candidate.arguments.as_ref().unwrap().as_object().unwrap();
        assert_eq!(args["optional_with_default_a"], Value::Number(99.into()));
    }

    #[test]
    fn respects_budget_in_default_fill() {
        let registry = mock_registry();
        let mut candidate = make_candidate(
            Some("mock"),
            Some(serde_json::json!({
                "required_no_default": "x",
                "required_with_default": 1
            })),
        );
        let applied = apply(&mut candidate, &registry, 1);
        assert_eq!(applied, 1);

        let args = candidate.arguments.as_ref().unwrap().as_object().unwrap();
        assert!(
            args.contains_key("optional_with_default_a")
                || args.contains_key("optional_with_default_b")
        );
    }

    #[test]
    fn returns_zero_when_arguments_none() {
        let registry = mock_registry();
        let mut candidate = make_candidate(Some("mock"), None);
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 0);
        assert!(candidate.arguments.is_none());
    }

    #[test]
    fn returns_zero_when_name_unknown() {
        let registry = mock_registry();
        let mut candidate = make_candidate(Some("not_a_tool"), Some(serde_json::json!({})));
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 0);
    }
}
