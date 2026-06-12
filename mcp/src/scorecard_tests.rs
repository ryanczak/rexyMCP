use super::*;
use rexymcp_executor::ai::types::TokenBreakdown;
use rexymcp_executor::store::telemetry::{ContextEfficiency, GenerationParams};

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
        phase_doc_path: None,
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
        served_model: None,
        length_finish_rate: None,
        context_window: None,
        context_efficiency: Default::default(),
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

fn make_run_with_settings(
    model: &str,
    tags: &[&str],
    temperature: Option<f64>,
    seed: Option<u64>,
    length_finish_rate: Option<f64>,
) -> PhaseRun {
    PhaseRun {
        ts: 1_717_000_000_000,
        model: model.to_string(),
        generation_params: GenerationParams { temperature, seed },
        phase_id: "test".to_string(),
        phase_doc_path: None,
        tags: tags.iter().map(|s| s.to_string()).collect(),
        status: "complete".to_string(),
        escalated: false,
        gates: all_pass_gates(),
        parse_failure_rate: 0.1,
        repairs_per_call: 0.5,
        verifier_retries: 2,
        tool_success_rate: 0.9,
        turns: 7,
        wall_clock_s: 12.5,
        tokens: TokenBreakdown::default(),
        warnings: None,
        bugs_filed: None,
        bounces_to_approval: None,
        architect_verdict: None,
        served_model: None,
        length_finish_rate,
        context_window: None,
        context_efficiency: Default::default(),
    }
}

#[test]
fn by_settings_buckets_distinct_settings() {
    let runs = vec![
        make_run_with_settings("m1", &["t"], Some(0.2), None, None),
        make_run_with_settings("m1", &["t"], Some(0.7), None, None),
        make_run_with_settings("m1", &["t"], Some(0.2), None, None),
    ];
    let rows = aggregate_by_settings(&runs, &ScorecardFilter::default());
    assert_eq!(rows.len(), 2);
    let row_02 = rows.iter().find(|r| r.settings == "temp=0.2").unwrap();
    let row_07 = rows.iter().find(|r| r.settings == "temp=0.7").unwrap();
    assert_eq!(row_02.n_runs, 2);
    assert_eq!(row_07.n_runs, 1);
}

#[test]
fn by_settings_default_label_for_none() {
    let runs = vec![make_run_with_settings("m1", &["t"], None, None, None)];
    let rows = aggregate_by_settings(&runs, &ScorecardFilter::default());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].settings, "default");
}

#[test]
fn by_settings_does_not_explode_per_tag() {
    let runs = vec![make_run_with_settings(
        "m1",
        &["a", "b", "c"],
        None,
        None,
        None,
    )];
    let rows = aggregate_by_settings(&runs, &ScorecardFilter::default());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].n_runs, 1);
}

#[test]
fn by_settings_length_finish_rate_mean() {
    let runs = vec![
        make_run_with_settings("m1", &["t"], Some(0.2), None, Some(0.2)),
        make_run_with_settings("m1", &["t"], Some(0.2), None, Some(0.4)),
    ];
    let rows = aggregate_by_settings(&runs, &ScorecardFilter::default());
    assert_eq!(rows.len(), 1);
    assert!(rows[0].length_finish_rate_mean.is_some());
    assert!((rows[0].length_finish_rate_mean.unwrap() - 0.3).abs() < f64::EPSILON);

    let runs_none = vec![
        make_run_with_settings("m1", &["t"], None, None, None),
        make_run_with_settings("m1", &["t"], None, None, None),
    ];
    let rows_none = aggregate_by_settings(&runs_none, &ScorecardFilter::default());
    assert!(rows_none[0].length_finish_rate_mean.is_none());
}

#[test]
fn by_settings_min_runs_drops_low_sample() {
    let runs = vec![
        make_run_with_settings("m1", &["t"], Some(0.2), None, None),
        make_run_with_settings("m1", &["t"], Some(0.7), None, None),
        make_run_with_settings("m1", &["t"], Some(0.7), None, None),
    ];
    let filter = ScorecardFilter {
        min_runs: 2,
        ..Default::default()
    };
    let rows = aggregate_by_settings(&runs, &filter);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].settings, "temp=0.7");
    assert_eq!(rows[0].n_runs, 2);
}

// --- Context efficiency aggregation tests (phase 08c) ---

#[test]
fn scorecard_peak_context_pct_mean_averages_measured_runs() {
    let eff = |pct| ContextEfficiency {
        peak_context_pct: pct,
        ..Default::default()
    };
    let runs = vec![
        {
            let mut r = make_run("m1", &["rust"], all_pass_gates(), false, None, None);
            r.context_efficiency = eff(0.6);
            r
        },
        {
            let mut r = make_run("m1", &["rust"], all_pass_gates(), false, None, None);
            r.context_efficiency = eff(0.8);
            r
        },
    ];
    let rows = aggregate(&runs, &ScorecardFilter::default());
    let row = &rows.iter().find(|r| r.tag == "rust").unwrap();
    assert!(row.peak_context_pct_mean.is_some());
    assert!((row.peak_context_pct_mean.unwrap() - 0.7).abs() < f64::EPSILON);
}

#[test]
fn scorecard_tokens_reclaimed_mean_sums_all_four_sources() {
    let eff = ContextEfficiency {
        peak_context_pct: 0.5,
        compaction_count: 0,
        output_filtered_tokens: 100,
        read_evicted_tokens: 50,
        read_deduped_tokens: 30,
        compaction_tokens_reclaimed: 20,
    };
    let runs = vec![{
        let mut r = make_run("m1", &["rust"], all_pass_gates(), false, None, None);
        r.context_efficiency = eff;
        r
    }];
    let rows = aggregate(&runs, &ScorecardFilter::default());
    let row = &rows.iter().find(|r| r.tag == "rust").unwrap();
    assert!(row.tokens_reclaimed_mean.is_some());
    assert!((row.tokens_reclaimed_mean.unwrap() - 200.0).abs() < f64::EPSILON);
}

#[test]
fn scorecard_context_efficiency_none_when_all_legacy() {
    let runs = vec![make_run(
        "m1",
        &["rust"],
        all_pass_gates(),
        false,
        None,
        None,
    )];
    let rows = aggregate(&runs, &ScorecardFilter::default());
    let row = &rows.iter().find(|r| r.tag == "rust").unwrap();
    assert!(row.peak_context_pct_mean.is_none());
    assert!(row.tokens_reclaimed_mean.is_none());
}

#[test]
fn scorecard_context_measured_excludes_legacy_runs() {
    let eff = |pct, reclaim| ContextEfficiency {
        peak_context_pct: pct,
        compaction_count: 0,
        output_filtered_tokens: reclaim,
        read_evicted_tokens: 0,
        read_deduped_tokens: 0,
        compaction_tokens_reclaimed: 0,
    };
    let runs = vec![
        {
            let mut r = make_run("m1", &["rust"], all_pass_gates(), false, None, None);
            r.context_efficiency = eff(0.5, 400);
            r
        },
        make_run("m1", &["rust"], all_pass_gates(), false, None, None), // legacy: all zeros
    ];
    let rows = aggregate(&runs, &ScorecardFilter::default());
    let row = &rows.iter().find(|r| r.tag == "rust").unwrap();
    assert!((row.peak_context_pct_mean.unwrap() - 0.5).abs() < f64::EPSILON);
    assert!((row.tokens_reclaimed_mean.unwrap() - 400.0).abs() < f64::EPSILON);
}

#[test]
fn scorecard_measured_run_with_zero_reclaim_contributes() {
    let eff = ContextEfficiency {
        peak_context_pct: 0.5,
        compaction_count: 0,
        output_filtered_tokens: 0,
        read_evicted_tokens: 0,
        read_deduped_tokens: 0,
        compaction_tokens_reclaimed: 0,
    };
    let runs = vec![{
        let mut r = make_run("m1", &["rust"], all_pass_gates(), false, None, None);
        r.context_efficiency = eff;
        r
    }];
    let rows = aggregate(&runs, &ScorecardFilter::default());
    let row = &rows.iter().find(|r| r.tag == "rust").unwrap();
    assert!(row.tokens_reclaimed_mean.is_some());
    assert!((row.tokens_reclaimed_mean.unwrap() - 0.0).abs() < f64::EPSILON);
}

// --- Context efficiency aggregation tests (phase 08d — model × settings) ---

fn make_run_with_eff(
    model: &str,
    temperature: Option<f64>,
    eff: rexymcp_executor::store::telemetry::ContextEfficiency,
) -> PhaseRun {
    let mut r = make_run_with_settings(model, &["rust"], temperature, None, None);
    r.context_efficiency = eff;
    r
}

#[test]
fn by_settings_peak_context_pct_mean_averages_measured_runs() {
    let runs = vec![
        make_run_with_eff(
            "m1",
            Some(0.2),
            ContextEfficiency {
                peak_context_pct: 0.6,
                ..Default::default()
            },
        ),
        make_run_with_eff(
            "m1",
            Some(0.2),
            ContextEfficiency {
                peak_context_pct: 0.8,
                ..Default::default()
            },
        ),
    ];
    let rows = aggregate_by_settings(&runs, &ScorecardFilter::default());
    assert_eq!(rows.len(), 1);
    assert!(rows[0].peak_context_pct_mean.is_some());
    assert!(
        (rows[0].peak_context_pct_mean.unwrap() - 0.7).abs() < f64::EPSILON,
        "expected 0.7, got {:?}",
        rows[0].peak_context_pct_mean
    );
}

#[test]
fn by_settings_tokens_reclaimed_mean_sums_all_four_sources() {
    let eff = ContextEfficiency {
        peak_context_pct: 0.5,
        compaction_count: 0,
        output_filtered_tokens: 100,
        read_evicted_tokens: 50,
        read_deduped_tokens: 30,
        compaction_tokens_reclaimed: 20,
    };
    let runs = vec![make_run_with_eff("m1", Some(0.2), eff)];
    let rows = aggregate_by_settings(&runs, &ScorecardFilter::default());
    assert_eq!(rows.len(), 1);
    assert!(rows[0].tokens_reclaimed_mean.is_some());
    assert!(
        (rows[0].tokens_reclaimed_mean.unwrap() - 200.0).abs() < f64::EPSILON,
        "expected 200.0, got {:?}",
        rows[0].tokens_reclaimed_mean
    );
}

#[test]
fn by_settings_context_efficiency_none_when_all_legacy() {
    let runs = vec![make_run_with_eff("m1", None, ContextEfficiency::default())];
    let rows = aggregate_by_settings(&runs, &ScorecardFilter::default());
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].peak_context_pct_mean.is_none(),
        "expected None for legacy run"
    );
    assert!(
        rows[0].tokens_reclaimed_mean.is_none(),
        "expected None for legacy run"
    );
}

#[test]
fn by_settings_context_measured_excludes_legacy_runs() {
    let measured = ContextEfficiency {
        peak_context_pct: 0.5,
        compaction_count: 0,
        output_filtered_tokens: 400,
        read_evicted_tokens: 0,
        read_deduped_tokens: 0,
        compaction_tokens_reclaimed: 0,
    };
    let runs = vec![
        make_run_with_eff("m1", Some(0.2), measured),
        make_run_with_eff("m1", Some(0.2), ContextEfficiency::default()), // legacy
    ];
    let rows = aggregate_by_settings(&runs, &ScorecardFilter::default());
    assert_eq!(rows.len(), 1);
    assert!(
        (rows[0].peak_context_pct_mean.unwrap() - 0.5).abs() < f64::EPSILON,
        "expected measured-only mean 0.5, got {:?}",
        rows[0].peak_context_pct_mean
    );
    assert!(
        (rows[0].tokens_reclaimed_mean.unwrap() - 400.0).abs() < f64::EPSILON,
        "expected measured-only mean 400.0, got {:?}",
        rows[0].tokens_reclaimed_mean
    );
}

#[test]
fn by_settings_measured_run_with_zero_reclaim_contributes() {
    let eff = ContextEfficiency {
        peak_context_pct: 0.5,
        compaction_count: 0,
        output_filtered_tokens: 0,
        read_evicted_tokens: 0,
        read_deduped_tokens: 0,
        compaction_tokens_reclaimed: 0,
    };
    let runs = vec![make_run_with_eff("m1", Some(0.2), eff)];
    let rows = aggregate_by_settings(&runs, &ScorecardFilter::default());
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].tokens_reclaimed_mean.is_some(),
        "expected Some(0.0) for measured run with zero reclaim, got None"
    );
    assert!(
        (rows[0].tokens_reclaimed_mean.unwrap() - 0.0).abs() < f64::EPSILON,
        "expected 0.0, got {:?}",
        rows[0].tokens_reclaimed_mean
    );
}

#[test]
fn scorecard_row_serializes_context_efficiency_means() {
    let eff = ContextEfficiency {
        peak_context_pct: 0.7,
        compaction_count: 0,
        output_filtered_tokens: 4096,
        read_evicted_tokens: 4096,
        read_deduped_tokens: 2048,
        compaction_tokens_reclaimed: 2048,
    };
    let runs = vec![{
        let mut r = make_run("m1", &["rust"], all_pass_gates(), false, None, None);
        r.context_efficiency = eff;
        r
    }];
    let rows = aggregate(&runs, &ScorecardFilter::default());
    let row = &rows.iter().find(|r| r.tag == "rust").unwrap();
    let json = serde_json::to_string(row).unwrap();
    assert!(json.contains(r#""peak_context_pct_mean""#));
    assert!(json.contains(r#""tokens_reclaimed_mean""#));
    assert!(json.contains(r#""peak_context_pct_mean":0.7"#));
    assert!(json.contains(r#""tokens_reclaimed_mean":12288.0"#));
}
