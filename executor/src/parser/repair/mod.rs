pub mod alias;
pub mod coerce;
pub mod default_fill;
pub mod json;
pub mod name;
pub mod strings;

use crate::tools::ToolRegistry;

use super::Candidate;

/// The total cap on transforms per candidate. Enforced across the whole repair
/// pipeline; once the cap is reached, later transforms run as no-ops.
const CAP: usize = 4;

/// Apply the repair pipeline to a candidate.
///
/// Runs deterministic, ordered transforms (the order is contract):
///   1. name fuzzy-match
///   2. param alias resolution
///   3. type coercion
///   4. default fill
///   5. json_repair
///   6. newline_escape
///
/// Total transformations applied are capped at `CAP`.
pub fn apply(candidate: &mut Candidate, registry: &ToolRegistry) {
    let mut applied: usize = 0;

    applied += name::apply(candidate, registry, CAP.saturating_sub(applied));
    applied += alias::apply(candidate, registry, CAP.saturating_sub(applied));
    applied += coerce::apply(candidate, registry, CAP.saturating_sub(applied));
    applied += default_fill::apply(candidate, registry, CAP.saturating_sub(applied));
    applied += json::apply(candidate, registry, CAP.saturating_sub(applied));
    let _ = strings::apply(candidate, registry, CAP.saturating_sub(applied));
}

#[cfg(test)]
mod tests {
    use super::super::{Format, RepairOp};
    use super::*;
    use crate::security::scope::Scope;
    use crate::tools::{bash, find_files, patch, read_file, search, symbols, write_file};
    use serde_json::Value;

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
    fn composes_name_alias_and_coerce() {
        let registry = test_registry();
        let mut candidate = make_candidate(
            Some("read_fil"),
            Some(serde_json::json!({"file_path": "src/main.rs", "start_line": "10"})),
        );
        apply(&mut candidate, &registry);

        assert_eq!(candidate.name.as_deref(), Some("read_file"));
        let args = candidate.arguments.as_ref().unwrap().as_object().unwrap();
        assert!(args.contains_key("path"));
        assert!(!args.contains_key("file_path"));
        assert_eq!(args["start_line"], Value::Number(10.into()));

        assert_eq!(candidate.repairs_attempted.len(), 3);
        assert!(matches!(
            &candidate.repairs_attempted[0],
            RepairOp::NameFuzzyMatch { .. }
        ));
        assert!(matches!(
            &candidate.repairs_attempted[1],
            RepairOp::ParamAlias { .. }
        ));
        assert!(matches!(
            &candidate.repairs_attempted[2],
            RepairOp::TypeCoerce { .. }
        ));
    }

    #[test]
    fn respects_total_cap_of_four() {
        let registry = test_registry();
        let mut candidate = make_candidate(
            Some("read_fil"),
            Some(serde_json::json!({
                "file_path": "x",
                "start_line": "1",
                "end_line": "5"
            })),
        );
        apply(&mut candidate, &registry);

        // name(1) + alias(1) + coerce(2) = 4
        assert_eq!(candidate.repairs_attempted.len(), 4);
    }

    #[test]
    fn name_repair_unblocks_subsequent_alias_match() {
        let registry = test_registry();
        let mut candidate = make_candidate(
            Some("read_fil"),
            Some(serde_json::json!({"file_path": "x"})),
        );
        apply(&mut candidate, &registry);

        assert_eq!(candidate.name.as_deref(), Some("read_file"));
        let args = candidate.arguments.as_ref().unwrap().as_object().unwrap();
        assert!(args.contains_key("path"));
        assert!(!args.contains_key("file_path"));
    }

    #[test]
    fn default_fill_runs_after_coerce_in_orchestrator() {
        use crate::tools::ToolResult;
        use async_trait::async_trait;

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
                        "optional_int_with_default": { "type": "integer", "default": 42 },
                        "optional_bool_with_default": { "type": "boolean", "default": true }
                    },
                    "required": ["required_no_default"]
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

        let mut registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(MockToolWithDefaults));

        let mut candidate = make_candidate(
            Some("mock"),
            Some(serde_json::json!({
                "required_no_default": "x",
                "optional_int_with_default": "99"
            })),
        );
        apply(&mut candidate, &registry);

        let args = candidate.arguments.as_ref().unwrap().as_object().unwrap();
        assert_eq!(args["optional_int_with_default"], Value::Number(99.into()));
        assert_eq!(args["optional_bool_with_default"], Value::Bool(true));

        assert!(matches!(
            &candidate.repairs_attempted[0],
            RepairOp::TypeCoerce { field, .. } if field == "optional_int_with_default"
        ));
        assert!(matches!(
            &candidate.repairs_attempted[1],
            RepairOp::DefaultFill { field } if field == "optional_bool_with_default"
        ));
    }

    #[test]
    fn json_repair_runs_after_default_fill() {
        let mut candidate = Candidate {
            format: Format::Hermes,
            name: Some("read_file".to_string()),
            arguments: None,
            score: 0,
            repairs_attempted: Vec::new(),
            raw_body: Some(r#"{"name":"read_file","arguments":{"path":"x",}}"#.to_string()),
        };
        apply(&mut candidate, &test_registry());

        assert!(candidate.arguments.is_some());
        assert!(
            candidate
                .repairs_attempted
                .iter()
                .any(|r| matches!(r, RepairOp::JsonRepair))
        );
    }

    #[test]
    fn respects_cap_with_all_six_transforms() {
        let registry = test_registry();
        let mut candidate = Candidate {
            format: Format::Hermes,
            name: Some("read_fil".to_string()),
            arguments: Some(serde_json::json!({
                "file_path": "x",
                "start_line": "1",
                "end_line": "5"
            })),
            score: 0,
            repairs_attempted: Vec::new(),
            raw_body: None,
        };
        apply(&mut candidate, &registry);

        assert_eq!(candidate.repairs_attempted.len(), 4);
    }
}
