use serde_json::{Map, Value};
use std::sync::Arc;

use crate::tools::ToolRegistry;

use super::super::Candidate;

/// Apply up to `budget` type-coercion transforms to the candidate. Returns the
/// number actually applied (`0..=budget`); appends a `RepairOp::TypeCoerce`.
/// `budget == 0` returns 0 immediately.
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
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let mut applied = 0;
    for (key, val) in args.clone() {
        if applied >= budget {
            break;
        }
        let prop_schema = match properties.get(&key) {
            Some(p) => p,
            None => continue,
        };
        let declared_type = match prop_schema.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => continue,
        };
        let Value::String(s) = &val else {
            continue;
        };

        if let Some(coerced) = try_coerce(declared_type, s) {
            let field = key.clone();
            args.insert(key, coerced);
            candidate
                .repairs_attempted
                .push(super::super::RepairOp::TypeCoerce {
                    field,
                    from_type: "string".to_string(),
                    to_type: declared_type.to_string(),
                });
            applied += 1;
        }
    }

    applied
}

fn try_coerce(schema_type: &str, s: &str) -> Option<Value> {
    match schema_type {
        "integer" => s.parse::<i64>().ok().map(Value::from),
        "number" => s
            .parse::<f64>()
            .ok()
            .and_then(|n| serde_json::Number::from_f64(n).map(Value::from)),
        "boolean" => match s {
            "true" => Some(Value::Bool(true)),
            "false" => Some(Value::Bool(false)),
            _ => None,
        },
        "object" => serde_json::from_str::<Map<String, Value>>(s)
            .ok()
            .map(Value::Object),
        "array" => serde_json::from_str::<Vec<Value>>(s).ok().map(Value::Array),
        _ => None,
    }
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
    use super::super::super::{Format, RepairOp};
    use super::*;
    use crate::security::scope::Scope;
    use crate::tools::{bash, find_files, patch, read_file, search, symbols, write_file};

    fn make_candidate(name: Option<&str>, args: Option<serde_json::Value>) -> Candidate {
        Candidate {
            format: Format::Hermes,
            name: name.map(String::from),
            arguments: args,
            score: 0,
            repairs_attempted: Vec::new(),
            raw_body: None,
        }
    }

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

    #[test]
    fn coerces_string_to_integer() {
        let registry = test_registry();
        let mut candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"path": "x", "start_line": "42"})),
        );
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 1);
        let args = candidate.arguments.as_ref().unwrap().as_object().unwrap();
        assert_eq!(args["start_line"], Value::Number(42.into()));
        assert!(matches!(
            &candidate.repairs_attempted[0],
            RepairOp::TypeCoerce { field, from_type, to_type }
            if field == "start_line" && from_type == "string" && to_type == "integer"
        ));
    }

    #[test]
    fn coerces_string_to_boolean() {
        let registry = test_registry();
        let mut candidate = make_candidate(
            Some("search"),
            Some(serde_json::json!({"pattern": "x", "case_insensitive": "true"})),
        );
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 1);
        let args = candidate.arguments.as_ref().unwrap().as_object().unwrap();
        assert_eq!(args["case_insensitive"], Value::Bool(true));
    }

    #[test]
    fn skips_coercion_when_already_correct_type() {
        let registry = test_registry();
        let mut candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"path": "x", "start_line": 42})),
        );
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 0);
        assert!(candidate.repairs_attempted.is_empty());
    }

    #[test]
    fn respects_budget_in_coerce_pass() {
        let registry = test_registry();
        let mut candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"path": "x", "start_line": "1", "end_line": "5"})),
        );
        let applied1 = apply(&mut candidate, &registry, 1);
        assert_eq!(applied1, 1);

        let mut candidate2 = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"path": "x", "start_line": "1", "end_line": "5"})),
        );
        let applied2 = apply(&mut candidate2, &registry, 2);
        assert_eq!(applied2, 2);
    }
}
