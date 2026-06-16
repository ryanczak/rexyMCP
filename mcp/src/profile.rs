use std::collections::{BTreeMap, HashMap};

use schemars::JsonSchema;
use serde::Serialize;

use rexymcp_executor::store::telemetry::{Gates, PhaseReview, PhaseRun, fold_reviews};

use crate::scorecard::ScorecardFilter;

/// One failure class and how many reviews in the bucket carried it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct FailureClassCount {
    pub class: String,
    pub count: usize,
}

/// Per-(model, tag) capability profile: strengths from folded runs, weaknesses
/// from the matched reviews' failure classes.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ModelProfile {
    pub model: String,
    pub tag: String,
    pub n_runs: usize,
    // --- strengths (folded runs) ---
    pub gates_pass_rate: f64,
    pub tool_success_rate_mean: f64,
    pub parse_failure_rate_mean: f64,
    pub escalation_rate: f64,
    /// Number of runs in the bucket carrying an `architect_verdict`.
    pub n_with_verdict: usize,
    /// Fraction of verdict-present runs that were `approved_first_try`.
    /// `None` when `n_with_verdict == 0`.
    pub approved_first_try_rate: Option<f64>,
    /// Mean `bounces_to_approval` over runs where it is `Some`. `None` if none.
    pub bounces_to_approval_mean: Option<f64>,
    // --- weaknesses (matched reviews) ---
    /// Number of reviews attributed to this bucket (its matched run's tags).
    pub n_reviews: usize,
    /// Failure classes seen in this bucket, **excluding `none`**, ranked by
    /// `count` descending then `class` ascending. `spec_bug`/`infra_blip` ARE
    /// included here (they are real observations); use [`is_model_attributable`]
    /// to separate honest model weaknesses from spec/infra noise — the surfacing
    /// layer (phase-04) does that, this layer stays neutral.
    pub ranked_failure_classes: Vec<FailureClassCount>,
}

/// False for failure classes that must NOT be charged against a model's
/// competency: `none` (no failure), `spec_bug` (architect's fault),
/// `infra_blip` (transient backend). True for everything else. Single-sources
/// the README taxonomy's "judged on what *it* got wrong" rule.
pub fn is_model_attributable(class: &str) -> bool {
    !matches!(class, "none" | "spec_bug" | "infra_blip")
}

/// Identity key for matching reviews to runs — mirrors `fold_reviews`.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum Key {
    Path(String),
    IdProject(String, String),
}

fn key_for_review(r: &PhaseReview) -> Key {
    if let Some(ref p) = r.phase_doc_path {
        Key::Path(p.clone())
    } else {
        Key::IdProject(r.phase_id.clone(), r.project_id.clone().unwrap_or_default())
    }
}

fn key_for_run(r: &PhaseRun) -> Key {
    if let Some(ref p) = r.phase_doc_path {
        Key::Path(p.clone())
    } else {
        Key::IdProject(r.phase_id.clone(), r.project_id.clone().unwrap_or_default())
    }
}

fn gates_all_pass(gates: &Gates) -> bool {
    gates.fmt == Some(true)
        && gates.build == Some(true)
        && gates.lint == Some(true)
        && gates.test == Some(true)
}

/// Internal accumulator for a single (model, tag) profile bucket.
#[derive(Debug, Default)]
struct ProfileAccumulator {
    n: usize,
    gates_all_pass: usize,
    tool_success_rate_sum: f64,
    parse_failure_rate_sum: f64,
    escalated_count: usize,
    n_with_verdict: usize,
    approved_first_try_count: usize,
    bounces_sum: f64,
    bounces_n: usize,
    // weakness pass
    n_reviews: usize,
    failure_class_counts: BTreeMap<String, usize>,
}

/// Aggregate runs + reviews into per-(model, tag) profiles. `runs` and
/// `reviews` are the raw store reads (`telemetry::read` / `read_reviews`); this
/// function folds internally so callers pass both unmodified. Strengths come
/// from the folded runs; failure-class counts come from each review joined to
/// its matching **latest** run (the same run `fold_reviews` annotates), bucketed
/// under that run's `(model, tag)` pairs.
pub fn aggregate_profiles(
    runs: &[PhaseRun],
    reviews: &[PhaseReview],
    filter: &ScorecardFilter,
) -> Vec<ModelProfile> {
    let folded = fold_reviews(runs.to_vec(), reviews);

    // Build latest_review map: key -> review with max ts
    let mut latest_review: HashMap<Key, &PhaseReview> = HashMap::new();
    for rev in reviews {
        let k = key_for_review(rev);
        latest_review
            .entry(k)
            .and_modify(|existing| {
                if rev.ts > existing.ts {
                    *existing = rev;
                }
            })
            .or_insert(rev);
    }

    // Build latest_run_ts map: key -> max ts among folded runs
    let mut latest_run_ts: HashMap<Key, u64> = HashMap::new();
    for run in &folded {
        let k = key_for_run(run);
        latest_run_ts
            .entry(k)
            .and_modify(|existing| {
                if run.ts > *existing {
                    *existing = run.ts;
                }
            })
            .or_insert(run.ts);
    }

    let mut buckets: BTreeMap<(String, String), ProfileAccumulator> = BTreeMap::new();

    for run in &folded {
        if let Some(model) = filter.model
            && run.model != model
        {
            continue;
        }

        if !filter.tags.is_empty() && !filter.tags.iter().all(|t| run.tags.contains(t)) {
            continue;
        }

        for tag in &run.tags {
            let key = (run.model.clone(), tag.clone());
            let acc = buckets.entry(key).or_default();
            acc.n += 1;

            // Strengths
            if gates_all_pass(&run.gates) {
                acc.gates_all_pass += 1;
            }
            acc.tool_success_rate_sum += run.tool_success_rate;
            acc.parse_failure_rate_sum += run.parse_failure_rate;
            if run.escalated {
                acc.escalated_count += 1;
            }
            if run.architect_verdict.is_some() {
                acc.n_with_verdict += 1;
                if run.architect_verdict.as_deref() == Some("approved_first_try") {
                    acc.approved_first_try_count += 1;
                }
            }
            if let Some(b) = run.bounces_to_approval {
                acc.bounces_sum += b as f64;
                acc.bounces_n += 1;
            }

            // Weakness pass: attribute failure classes only for the latest run
            let run_key = key_for_run(run);
            if run.ts == *latest_run_ts.get(&run_key).unwrap_or(&0)
                && let Some(rev) = latest_review.get(&run_key)
            {
                acc.n_reviews += 1;
                for class in &rev.failure_class {
                    if class != "none" {
                        *acc.failure_class_counts.entry(class.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let mut rows: Vec<ModelProfile> = buckets
        .into_iter()
        .filter_map(|((model, tag), acc)| {
            if acc.n < filter.min_runs {
                return None;
            }

            let n = acc.n as f64;

            let ranked_failure_classes: Vec<FailureClassCount> = acc
                .failure_class_counts
                .into_iter()
                .map(|(class, count)| FailureClassCount { class, count })
                .collect();
            // Sort: count desc, then class asc
            let mut ranked = ranked_failure_classes;
            ranked.sort_by(|a, b| b.count.cmp(&a.count).then(a.class.cmp(&b.class)));

            Some(ModelProfile {
                model,
                tag,
                n_runs: acc.n,
                gates_pass_rate: acc.gates_all_pass as f64 / n,
                tool_success_rate_mean: acc.tool_success_rate_sum / n,
                parse_failure_rate_mean: acc.parse_failure_rate_sum / n,
                escalation_rate: acc.escalated_count as f64 / n,
                n_with_verdict: acc.n_with_verdict,
                approved_first_try_rate: if acc.n_with_verdict > 0 {
                    Some(acc.approved_first_try_count as f64 / acc.n_with_verdict as f64)
                } else {
                    None
                },
                bounces_to_approval_mean: if acc.bounces_n > 0 {
                    Some(acc.bounces_sum / acc.bounces_n as f64)
                } else {
                    None
                },
                n_reviews: acc.n_reviews,
                ranked_failure_classes: ranked,
            })
        })
        .collect();

    rows.sort_by(|a, b| {
        a.tag
            .cmp(&b.tag)
            .then(b.n_runs.cmp(&a.n_runs))
            .then(a.model.cmp(&b.model))
    });

    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::store::telemetry::{Gates, GenerationParams};

    fn make_run(ts: u64, model: &str, tags: &[&str], verdict: Option<&str>) -> PhaseRun {
        PhaseRun {
            ts,
            model: model.to_string(),
            generation_params: GenerationParams::default(),
            phase_id: "phase-01".to_string(),
            phase_doc_path: None,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            status: "complete".to_string(),
            escalated: false,
            gates: Gates {
                fmt: Some(true),
                build: Some(true),
                lint: Some(true),
                test: Some(true),
            },
            parse_failure_rate: 0.0,
            repairs_per_call: 0.0,
            verifier_retries: 0,
            tool_success_rate: 1.0,
            turns: 5,
            wall_clock_s: 10.0,
            tokens: Default::default(),
            warnings: None,
            bugs_filed: None,
            bounces_to_approval: None,
            architect_verdict: verdict.map(|s| s.to_string()),
            served_model: None,
            length_finish_rate: None,
            context_window: None,
            context_efficiency: Default::default(),
            project_id: None,
            milestone_id: None,
            tier_telemetry: Default::default(),
        }
    }

    fn make_run_with_path(
        ts: u64,
        model: &str,
        tags: &[&str],
        phase_doc_path: &str,
        verdict: Option<&str>,
    ) -> PhaseRun {
        PhaseRun {
            ts,
            model: model.to_string(),
            generation_params: GenerationParams::default(),
            phase_id: "phase-01".to_string(),
            phase_doc_path: Some(phase_doc_path.to_string()),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            status: "complete".to_string(),
            escalated: false,
            gates: Gates {
                fmt: Some(true),
                build: Some(true),
                lint: Some(true),
                test: Some(true),
            },
            parse_failure_rate: 0.0,
            repairs_per_call: 0.0,
            verifier_retries: 0,
            tool_success_rate: 1.0,
            turns: 5,
            wall_clock_s: 10.0,
            tokens: Default::default(),
            warnings: None,
            bugs_filed: None,
            bounces_to_approval: None,
            architect_verdict: verdict.map(|s| s.to_string()),
            served_model: None,
            length_finish_rate: None,
            context_window: None,
            context_efficiency: Default::default(),
            project_id: None,
            milestone_id: None,
            tier_telemetry: Default::default(),
        }
    }

    fn make_review(
        ts: u64,
        phase_doc_path: Option<&str>,
        verdict: &str,
        failure_class: &[&str],
    ) -> PhaseReview {
        PhaseReview {
            record: "review".to_string(),
            ts,
            phase_doc_path: phase_doc_path.map(|s| s.to_string()),
            phase_id: "phase-01".to_string(),
            project_id: None,
            architect_verdict: verdict.to_string(),
            bounces_to_approval: None,
            bugs_filed: None,
            warnings: None,
            failure_class: failure_class.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn strengths_come_from_folded_runs() {
        // One run with verdict None in the store + a matching approved_first_try review
        let runs = vec![make_run(100, "claude", &["kind=feature"], None)];
        let reviews = vec![make_review(200, None, "approved_first_try", &["none"])];
        let filter = ScorecardFilter {
            tags: &[],
            model: None,
            min_runs: 0,
        };

        let profiles = aggregate_profiles(&runs, &reviews, &filter);

        // fold_reviews should have populated architect_verdict on the run
        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        assert_eq!(p.model, "claude");
        assert_eq!(p.tag, "kind=feature");
        assert_eq!(p.n_with_verdict, 1);
        assert_eq!(p.approved_first_try_rate, Some(1.0));
    }

    #[test]
    fn ranks_failure_classes_by_count_then_name() {
        // Reviews yielding parse_format×2, prod_unwrap×2, scope_deviation×1
        let runs = vec![
            make_run_with_path(100, "claude", &["kind=feature"], "p1", None),
            make_run_with_path(110, "claude", &["kind=feature"], "p2", None),
            make_run_with_path(120, "claude", &["kind=feature"], "p3", None),
            make_run_with_path(130, "claude", &["kind=feature"], "p4", None),
            make_run_with_path(140, "claude", &["kind=feature"], "p5", None),
        ];
        let reviews = vec![
            make_review(200, Some("p1"), "bounce", &["parse_format"]),
            make_review(210, Some("p2"), "bounce", &["parse_format"]),
            make_review(220, Some("p3"), "bounce", &["prod_unwrap"]),
            make_review(230, Some("p4"), "bounce", &["prod_unwrap"]),
            make_review(240, Some("p5"), "bounce", &["scope_deviation"]),
        ];
        let filter = ScorecardFilter {
            tags: &[],
            model: None,
            min_runs: 0,
        };

        let profiles = aggregate_profiles(&runs, &reviews, &filter);

        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        let ranked = &p.ranked_failure_classes;
        assert_eq!(ranked.len(), 3);
        // parse_format(2) and prod_unwrap(2) tied on count, sorted by name asc
        assert_eq!(ranked[0].class, "parse_format");
        assert_eq!(ranked[0].count, 2);
        assert_eq!(ranked[1].class, "prod_unwrap");
        assert_eq!(ranked[1].count, 2);
        assert_eq!(ranked[2].class, "scope_deviation");
        assert_eq!(ranked[2].count, 1);
    }

    #[test]
    fn excludes_none_from_failure_ranking() {
        // A bucket whose only review is ["none"]
        let runs = vec![make_run(100, "claude", &["kind=feature"], None)];
        let reviews = vec![make_review(200, None, "approved_first_try", &["none"])];
        let filter = ScorecardFilter {
            tags: &[],
            model: None,
            min_runs: 0,
        };

        let profiles = aggregate_profiles(&runs, &reviews, &filter);

        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        assert!(p.n_reviews >= 1);
        assert!(p.ranked_failure_classes.is_empty());
    }

    #[test]
    fn attributes_failure_to_matched_run_tags() {
        // A prod_unwrap review matched to a run tagged ["language=rust","kind=feature"]
        // lands in both tag buckets. A second review whose phase_doc_path matches NO run
        // adds nothing.
        let runs = vec![make_run(
            100,
            "claude",
            &["language=rust", "kind=feature"],
            None,
        )];
        let reviews = vec![
            make_review(200, None, "bounce", &["prod_unwrap"]),
            make_review(300, Some("nonexistent"), "bounce", &["parse_format"]),
        ];
        let filter = ScorecardFilter {
            tags: &[],
            model: None,
            min_runs: 0,
        };

        let profiles = aggregate_profiles(&runs, &reviews, &filter);

        // Two buckets: one per tag
        assert_eq!(profiles.len(), 2);
        for p in &profiles {
            assert_eq!(p.model, "claude");
            assert_eq!(p.ranked_failure_classes.len(), 1);
            assert_eq!(p.ranked_failure_classes[0].class, "prod_unwrap");
            assert_eq!(p.ranked_failure_classes[0].count, 1);
        }
    }

    #[test]
    fn attributes_to_latest_run_only() {
        // Two runs share a key (phase_doc_path), different ts, different tag sets
        let runs = vec![
            make_run_with_path(100, "claude", &["old-tag"], "shared-path", None),
            make_run_with_path(200, "claude", &["new-tag"], "shared-path", None),
        ];
        let reviews = vec![make_review(
            300,
            Some("shared-path"),
            "bounce",
            &["prod_unwrap"],
        )];
        let filter = ScorecardFilter {
            tags: &[],
            model: None,
            min_runs: 0,
        };

        let profiles = aggregate_profiles(&runs, &reviews, &filter);

        // Two buckets: old-tag and new-tag
        assert_eq!(profiles.len(), 2);

        // The review's class is counted under the latest run's tag (new-tag)
        let new_bucket = profiles.iter().find(|p| p.tag == "new-tag").unwrap();
        assert_eq!(new_bucket.ranked_failure_classes.len(), 1);
        assert_eq!(new_bucket.ranked_failure_classes[0].class, "prod_unwrap");

        // The earlier run's tag bucket has NO failure classes
        let old_bucket = profiles.iter().find(|p| p.tag == "old-tag").unwrap();
        assert!(old_bucket.ranked_failure_classes.is_empty());
    }

    #[test]
    fn multi_class_review_counts_each_class() {
        // A review with ["parse_format","prod_unwrap"] increments both; n_reviews == 1
        let runs = vec![make_run(100, "claude", &["kind=feature"], None)];
        let reviews = vec![make_review(
            200,
            None,
            "bounce",
            &["parse_format", "prod_unwrap"],
        )];
        let filter = ScorecardFilter {
            tags: &[],
            model: None,
            min_runs: 0,
        };

        let profiles = aggregate_profiles(&runs, &reviews, &filter);

        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        assert_eq!(p.n_reviews, 1);
        assert_eq!(p.ranked_failure_classes.len(), 2);
        // Both count 1, sorted by class asc: parse_format < prod_unwrap
        assert_eq!(p.ranked_failure_classes[0].class, "parse_format");
        assert_eq!(p.ranked_failure_classes[0].count, 1);
        assert_eq!(p.ranked_failure_classes[1].class, "prod_unwrap");
        assert_eq!(p.ranked_failure_classes[1].count, 1);
    }

    #[test]
    fn is_model_attributable_separates_spec_and_infra() {
        assert!(!is_model_attributable("none"));
        assert!(!is_model_attributable("spec_bug"));
        assert!(!is_model_attributable("infra_blip"));
        assert!(is_model_attributable("prod_unwrap"));
        assert!(is_model_attributable("false_completion"));
        assert!(is_model_attributable("parse_format"));
        assert!(is_model_attributable("scope_deviation"));
    }

    #[test]
    fn min_runs_filters_small_buckets() {
        let runs = vec![make_run(100, "claude", &["kind=feature"], None)];
        let reviews = vec![];
        let filter = ScorecardFilter {
            tags: &[],
            model: None,
            min_runs: 2,
        };

        let profiles = aggregate_profiles(&runs, &reviews, &filter);

        assert!(profiles.is_empty());
    }
}
