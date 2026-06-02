use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Serialize;

use rexymcp_executor::store::telemetry::{Gates, PhaseRun};

/// One row of the model × tag matrix.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ScorecardRow {
    pub model: String,
    pub tag: String,
    pub n_runs: usize,
    /// Fraction of runs where every configured gate (fmt/build/lint/test)
    /// reported `Some(true)`. A `None` gate counts as a non-pass.
    pub gates_pass_rate: f64,
    pub parse_failure_rate_mean: f64,
    pub repairs_per_call_mean: f64,
    pub tool_success_rate_mean: f64,
    pub verifier_retries_mean: f64,
    pub turns_mean: f64,
    pub wall_clock_s_mean: f64,
    /// Fraction of runs with `escalated == true`.
    pub escalation_rate: f64,
    /// Number of runs in this bucket that have `architect_verdict` set.
    pub n_with_verdict: usize,
    /// Fraction of verdict-present runs that were `approved_first_try`.
    /// `None` when `n_with_verdict == 0`.
    pub approved_first_try_rate: Option<f64>,
    /// Mean of `bounces_to_approval` over runs where it is `Some`.
    /// `None` when no such runs.
    pub bounces_to_approval_mean: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct ScorecardFilter<'a> {
    /// Restrict runs to those whose `tags` contains **all** of these tags.
    pub tags: &'a [String],
    /// Restrict to one model. `None` = all models.
    pub model: Option<&'a str>,
    /// Drop output rows with `n_runs < min_runs`. `0` = no minimum.
    pub min_runs: usize,
}

/// Maximum number of rows returned by the MCP tool.
pub const MAX_ROWS: usize = 500;

/// Internal accumulator for a single (model, tag) bucket.
#[derive(Debug, Default)]
struct Accumulator {
    n: usize,
    gates_all_pass: usize,
    parse_failure_rate_sum: f64,
    repairs_per_call_sum: f64,
    tool_success_rate_sum: f64,
    verifier_retries_sum: f64,
    turns_sum: f64,
    wall_clock_s_sum: f64,
    escalated_count: usize,
    n_with_verdict: usize,
    approved_first_try_count: usize,
    bounces_sum: f64,
    bounces_n: usize,
}

fn gates_all_pass(gates: &Gates) -> bool {
    gates.fmt == Some(true)
        && gates.build == Some(true)
        && gates.lint == Some(true)
        && gates.test == Some(true)
}

pub fn aggregate(runs: &[PhaseRun], filter: &ScorecardFilter) -> Vec<ScorecardRow> {
    let mut buckets: BTreeMap<(String, String), Accumulator> = BTreeMap::new();

    for run in runs {
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

            if gates_all_pass(&run.gates) {
                acc.gates_all_pass += 1;
            }
            acc.parse_failure_rate_sum += run.parse_failure_rate;
            acc.repairs_per_call_sum += run.repairs_per_call;
            acc.tool_success_rate_sum += run.tool_success_rate;
            acc.verifier_retries_sum += run.verifier_retries as f64;
            acc.turns_sum += run.turns as f64;
            acc.wall_clock_s_sum += run.wall_clock_s;

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
        }
    }

    let mut rows: Vec<ScorecardRow> = buckets
        .into_iter()
        .filter_map(|((model, tag), acc)| {
            if acc.n < filter.min_runs {
                return None;
            }

            let n = acc.n as f64;

            Some(ScorecardRow {
                model,
                tag,
                n_runs: acc.n,
                gates_pass_rate: acc.gates_all_pass as f64 / n,
                parse_failure_rate_mean: acc.parse_failure_rate_sum / n,
                repairs_per_call_mean: acc.repairs_per_call_sum / n,
                tool_success_rate_mean: acc.tool_success_rate_sum / n,
                verifier_retries_mean: acc.verifier_retries_sum / n,
                turns_mean: acc.turns_sum / n,
                wall_clock_s_mean: acc.wall_clock_s_sum / n,
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
    use rexymcp_executor::ai::types::TokenBreakdown;
    use rexymcp_executor::store::telemetry::GenerationParams;

    fn make_run(
        model: &str,
        tags: &[&str],
        gates: Gates,
        escalated: bool,
        verdict: Option<&str>,
        bounces: Option<u32>,
    ) -> PhaseRun {
        PhaseRun {
            ts: 1_717_000_000_000,
            model: model.to_string(),
            generation_params: GenerationParams::default(),
            phase_id: "test".to_string(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            status: "complete".to_string(),
            escalated,
            gates,
            parse_failure_rate: 0.1,
            repairs_per_call: 0.5,
            verifier_retries: 2,
            tool_success_rate: 0.9,
            turns: 7,
            wall_clock_s: 12.5,
            tokens: TokenBreakdown::default(),
            warnings: None,
            bugs_filed: None,
            bounces_to_approval: bounces,
            architect_verdict: verdict.map(|s| s.to_string()),
        }
    }

    fn all_pass_gates() -> Gates {
        Gates {
            fmt: Some(true),
            build: Some(true),
            lint: Some(true),
            test: Some(true),
        }
    }

    fn all_fail_gates() -> Gates {
        Gates {
            fmt: Some(false),
            build: Some(false),
            lint: Some(false),
            test: Some(false),
        }
    }

    fn one_none_gate() -> Gates {
        Gates {
            fmt: Some(true),
            build: Some(true),
            lint: None,
            test: Some(true),
        }
    }

    #[test]
    fn empty_runs_returns_empty() {
        let rows = aggregate(&[], &ScorecardFilter::default());
        assert!(rows.is_empty());
    }

    #[test]
    fn model_filter_only_matching_contribute() {
        let runs = vec![
            make_run("m1", &["rust"], all_pass_gates(), false, None, None),
            make_run("m2", &["rust"], all_pass_gates(), false, None, None),
        ];
        let filter = ScorecardFilter {
            model: Some("m1"),
            ..Default::default()
        };
        let rows = aggregate(&runs, &filter);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model, "m1");
    }

    #[test]
    fn tags_and_filter_only_runs_containing_all_tags() {
        let runs = vec![
            make_run(
                "m1",
                &["rust", "feature"],
                all_pass_gates(),
                false,
                None,
                None,
            ),
            make_run(
                "m1",
                &["rust", "feature"],
                all_pass_gates(),
                false,
                None,
                None,
            ),
            make_run(
                "m1",
                &["rust", "bugfix"],
                all_pass_gates(),
                false,
                None,
                None,
            ),
            make_run("m1", &["go"], all_pass_gates(), false, None, None),
        ];
        let filter = ScorecardFilter {
            tags: &["rust".to_string(), "feature".to_string()],
            ..Default::default()
        };
        let rows = aggregate(&runs, &filter);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tag, "feature");
        assert_eq!(rows[1].tag, "rust");
        assert_eq!(rows[0].n_runs, 2);
        assert_eq!(rows[1].n_runs, 2);
    }

    #[test]
    fn combined_model_and_tags_filter() {
        let runs = vec![
            make_run(
                "m1",
                &["rust", "feature"],
                all_pass_gates(),
                false,
                None,
                None,
            ),
            make_run(
                "m2",
                &["rust", "feature"],
                all_pass_gates(),
                false,
                None,
                None,
            ),
        ];
        let filter = ScorecardFilter {
            model: Some("m1"),
            tags: &["feature".to_string()],
            ..Default::default()
        };
        let rows = aggregate(&runs, &filter);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].model, "m1");
        assert_eq!(rows[1].model, "m1");
    }

    #[test]
    fn empty_filter_every_run_tag_contributes() {
        let runs = vec![make_run(
            "m1",
            &["a", "b"],
            all_pass_gates(),
            false,
            None,
            None,
        )];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].tag, "a");
        assert_eq!(rows[1].tag, "b");
    }

    #[test]
    fn explode_by_tag_single_run_multiple_tags() {
        let runs = vec![make_run(
            "m1",
            &["a", "b", "c"],
            all_pass_gates(),
            false,
            None,
            None,
        )];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert_eq!(rows.len(), 3);
        for row in &rows {
            assert_eq!(row.n_runs, 1);
        }
    }

    #[test]
    fn gates_pass_rate_all_pass_is_one() {
        let runs = vec![
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
        ];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert_eq!(rows[0].gates_pass_rate, 1.0);
    }

    #[test]
    fn gates_pass_rate_mixed() {
        let runs = vec![
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
            make_run("m1", &["t"], all_fail_gates(), false, None, None),
        ];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert!((rows[0].gates_pass_rate - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn gates_pass_rate_none_gate_counts_as_fail() {
        let runs = vec![make_run("m1", &["t"], one_none_gate(), false, None, None)];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert_eq!(rows[0].gates_pass_rate, 0.0);
    }

    #[test]
    fn mean_fields_are_arithmetic_means() {
        let runs = vec![
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
        ];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        let r = &rows[0];
        assert!((r.parse_failure_rate_mean - 0.1).abs() < f64::EPSILON);
        assert!((r.repairs_per_call_mean - 0.5).abs() < f64::EPSILON);
        assert!((r.tool_success_rate_mean - 0.9).abs() < f64::EPSILON);
        assert!((r.verifier_retries_mean - 2.0).abs() < f64::EPSILON);
        assert!((r.turns_mean - 7.0).abs() < f64::EPSILON);
        assert!((r.wall_clock_s_mean - 12.5).abs() < f64::EPSILON);
    }

    #[test]
    fn escalation_rate_mixed() {
        let runs = vec![
            make_run("m1", &["t"], all_pass_gates(), true, None, None),
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
        ];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert!((rows[0].escalation_rate - 1.0 / 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn n_with_verdict_zero_gives_none_supervision() {
        let runs = vec![make_run("m1", &["t"], all_pass_gates(), false, None, None)];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert_eq!(rows[0].n_with_verdict, 0);
        assert!(rows[0].approved_first_try_rate.is_none());
        assert!(rows[0].bounces_to_approval_mean.is_none());
    }

    #[test]
    fn approved_first_try_rate_partial_verdicts() {
        let runs = vec![
            make_run(
                "m1",
                &["t"],
                all_pass_gates(),
                false,
                Some("approved_first_try"),
                None,
            ),
            make_run(
                "m1",
                &["t"],
                all_pass_gates(),
                false,
                Some("rejected"),
                None,
            ),
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
        ];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert_eq!(rows[0].n_with_verdict, 2);
        assert!(rows[0].approved_first_try_rate.is_some());
        assert!((rows[0].approved_first_try_rate.unwrap() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn approved_first_try_rate_all_verdicts() {
        let runs = vec![
            make_run(
                "m1",
                &["t"],
                all_pass_gates(),
                false,
                Some("approved_first_try"),
                None,
            ),
            make_run(
                "m1",
                &["t"],
                all_pass_gates(),
                false,
                Some("approved_first_try"),
                None,
            ),
        ];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert_eq!(rows[0].n_with_verdict, 2);
        assert!((rows[0].approved_first_try_rate.unwrap() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn bounces_mean_partial() {
        let runs = vec![
            make_run(
                "m1",
                &["t"],
                all_pass_gates(),
                false,
                Some("approved_first_try"),
                Some(0),
            ),
            make_run(
                "m1",
                &["t"],
                all_pass_gates(),
                false,
                Some("rejected"),
                Some(2),
            ),
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
        ];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert!((rows[0].bounces_to_approval_mean.unwrap() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn bounces_mean_none_when_no_bounces_data() {
        let runs = vec![
            make_run(
                "m1",
                &["t"],
                all_pass_gates(),
                false,
                Some("approved_first_try"),
                None,
            ),
            make_run("m1", &["t"], all_pass_gates(), false, None, None),
        ];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert!(rows[0].bounces_to_approval_mean.is_none());
    }

    #[test]
    fn min_runs_drops_low_sample_buckets() {
        let runs = vec![
            make_run("m1", &["rare"], all_pass_gates(), false, None, None),
            make_run("m1", &["common"], all_pass_gates(), false, None, None),
            make_run("m1", &["common"], all_pass_gates(), false, None, None),
        ];
        let filter = ScorecardFilter {
            min_runs: 2,
            ..Default::default()
        };
        let rows = aggregate(&runs, &filter);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tag, "common");
        assert_eq!(rows[0].n_runs, 2);
    }

    #[test]
    fn sort_order_tag_asc_n_runs_desc_model_asc() {
        let runs = vec![
            make_run("beta", &["z", "a"], all_pass_gates(), false, None, None),
            make_run("alpha", &["z", "a"], all_pass_gates(), false, None, None),
            make_run("alpha", &["z"], all_pass_gates(), false, None, None),
        ];
        let rows = aggregate(&runs, &ScorecardFilter::default());
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].tag, "a");
        assert_eq!(rows[0].model, "alpha");
        assert_eq!(rows[0].n_runs, 1);
        assert_eq!(rows[1].tag, "a");
        assert_eq!(rows[1].model, "beta");
        assert_eq!(rows[1].n_runs, 1);
        assert_eq!(rows[2].tag, "z");
        assert_eq!(rows[2].model, "alpha");
        assert_eq!(rows[2].n_runs, 2);
        assert_eq!(rows[3].tag, "z");
        assert_eq!(rows[3].model, "beta");
        assert_eq!(rows[3].n_runs, 1);
    }
}
