use serde_json::Value;
use std::sync::Arc;

use crate::tools::ToolRegistry;

use super::super::Candidate;

const ALIAS_TABLE: &[(&str, &str)] = &[
    ("cmd", "command"),
    ("file_path", "path"),
    ("filename", "path"),
    ("query", "pattern"),
];

/// Apply up to `budget` param-alias transforms to the candidate. Returns the
/// number actually applied (`0..=budget`); appends a `RepairOp::ParamAlias`.
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
    for &(alias, canonical) in ALIAS_TABLE {
        if applied >= budget {
            break;
        }
        if !properties.contains_key(canonical) {
            continue;
        }
        if !args.contains_key(alias) {
            continue;
        }
        if args.contains_key(canonical) {
            continue;
        }
        if let Some(val) = args.remove(alias) {
            args.insert(canonical.to_string(), val);
            candidate
                .repairs_attempted
                .push(super::super::RepairOp::ParamAlias {
                    from: alias.to_string(),
                    to: canonical.to_string(),
                });
            applied += 1;
        }
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
    fn applies_known_alias_for_read_file() {
        let registry = test_registry();
        let mut candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"file_path": "x"})),
        );
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 1);
        let args = candidate.arguments.as_ref().unwrap().as_object().unwrap();
        assert!(args.contains_key("path"));
        assert!(!args.contains_key("file_path"));
        assert!(matches!(
            &candidate.repairs_attempted[0],
            RepairOp::ParamAlias { from, to }
            if from == "file_path" && to == "path"
        ));
    }

    #[test]
    fn skips_alias_when_target_present() {
        let registry = test_registry();
        let mut candidate = make_candidate(
            Some("read_file"),
            Some(serde_json::json!({"file_path": "x", "path": "y"})),
        );
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 0);
        let args = candidate.arguments.as_ref().unwrap().as_object().unwrap();
        assert_eq!(args["file_path"], "x");
        assert_eq!(args["path"], "y");
        assert!(candidate.repairs_attempted.is_empty());
    }

    #[test]
    fn respects_budget_in_alias_pass() {
        let registry = test_registry();
        let mut candidate = make_candidate(
            Some("search"),
            Some(serde_json::json!({"query": "x", "filename": "y"})),
        );
        let applied1 = apply(&mut candidate, &registry, 1);
        assert_eq!(applied1, 1);

        let mut candidate2 = make_candidate(
            Some("search"),
            Some(serde_json::json!({"query": "x", "filename": "y"})),
        );
        let applied2 = apply(&mut candidate2, &registry, 2);
        assert_eq!(applied2, 2);
    }

    #[test]
    fn returns_zero_when_no_alias_keys_present() {
        let registry = test_registry();
        let mut candidate =
            make_candidate(Some("read_file"), Some(serde_json::json!({"path": "x"})));
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 0);
        assert!(candidate.repairs_attempted.is_empty());
    }
}
