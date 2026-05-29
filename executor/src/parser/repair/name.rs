use crate::tools::ToolRegistry;

use super::super::Candidate;

/// Apply up to `budget` name fuzzy-match transforms to the candidate. Returns the
/// number actually applied (`0..=budget`); appends a `RepairOp::NameFuzzyMatch`.
/// `budget == 0` returns 0 immediately.
pub fn apply(candidate: &mut Candidate, registry: &ToolRegistry, budget: usize) -> usize {
    if budget == 0 {
        return 0;
    }

    let name = match &candidate.name {
        Some(n) if !n.is_empty() => n.clone(),
        _ => return 0,
    };

    if registry.get(&name).is_some() {
        return 0;
    }

    let mut best_dist = usize::MAX;
    let mut best_tool: Option<String> = None;
    for tool in registry.all() {
        let d = levenshtein(&name, tool.name());
        if d <= 2 && d < best_dist {
            best_dist = d;
            best_tool = Some(tool.name().to_string());
        } else if d <= 2
            && d == best_dist
            && let Some(ref bt) = best_tool
            && tool.name() < bt.as_str()
        {
            best_tool = Some(tool.name().to_string());
        }
    }

    let matched = match best_tool {
        Some(t) => t,
        None => return 0,
    };

    let from = name;
    candidate.name = Some(matched.clone());
    candidate
        .repairs_attempted
        .push(super::super::RepairOp::NameFuzzyMatch { from, to: matched });
    1
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
    fn exact_name_skips_repair() {
        let registry = test_registry();
        let mut candidate = make_candidate(Some("read_file"), None);
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 0);
        assert_eq!(candidate.name.as_deref(), Some("read_file"));
        assert!(candidate.repairs_attempted.is_empty());
    }

    #[test]
    fn fuzzy_name_within_two_gets_repaired() {
        let registry = test_registry();
        let mut candidate = make_candidate(Some("read_fil"), None);
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 1);
        assert_eq!(candidate.name.as_deref(), Some("read_file"));
        assert_eq!(candidate.repairs_attempted.len(), 1);
        assert!(matches!(
            &candidate.repairs_attempted[0],
            RepairOp::NameFuzzyMatch { from, to }
            if from == "read_fil" && to == "read_file"
        ));
    }

    #[test]
    fn unknown_name_too_far_skips() {
        let registry = test_registry();
        let mut candidate = make_candidate(Some("xyzpdq_qux"), None);
        let applied = apply(&mut candidate, &registry, 4);
        assert_eq!(applied, 0);
        assert_eq!(candidate.name.as_deref(), Some("xyzpdq_qux"));
        assert!(candidate.repairs_attempted.is_empty());
    }

    #[test]
    fn budget_zero_returns_zero() {
        let registry = test_registry();
        let mut candidate = make_candidate(Some("read_fil"), None);
        let applied = apply(&mut candidate, &registry, 0);
        assert_eq!(applied, 0);
        assert_eq!(candidate.name.as_deref(), Some("read_fil"));
        assert!(candidate.repairs_attempted.is_empty());
    }
}
