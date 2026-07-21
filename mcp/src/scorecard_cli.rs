//! Model × settings scorecard CLI — `rexymcp scorecard` subcommand.

use std::path::Path;

use rexymcp_executor::config::Config;
use rexymcp_executor::store::telemetry::PhaseRun;

use crate::scorecard::{ScorecardBucket, ScorecardDimension, ScorecardFilter, aggregate_scorecard};

/// Resolve the telemetry store path from config, read, aggregate,
/// and return `ScorecardBucket` records.
pub fn load_scorecard(
    config_path: &Path,
    telemetry_path: Option<&Path>,
    dimension: ScorecardDimension,
    filter: &ScorecardFilter,
) -> Result<Vec<ScorecardBucket>, String> {
    let cfg =
        Config::load_with_env(config_path).map_err(|e| format!("failed to load config: {}", e))?;

    let telemetry_file = if let Some(p) = telemetry_path {
        p.to_path_buf()
    } else if let Some(ref dir) = cfg.telemetry.dir {
        dir.join("phase_runs.jsonl")
    } else {
        return Err(
            "telemetry disabled: cfg.telemetry.dir not set and no --telemetry-path provided"
                .to_string(),
        );
    };

    let runs: Vec<PhaseRun> =
        rexymcp_executor::store::telemetry::read(&telemetry_file).map_err(|e| e.to_string())?;
    let reviews = rexymcp_executor::store::telemetry::read_reviews(&telemetry_file)
        .map_err(|e| e.to_string())?;
    let runs = rexymcp_executor::store::telemetry::fold_reviews(runs, &reviews);
    Ok(aggregate_scorecard(&runs, dimension, filter))
}

fn key_header(dim: ScorecardDimension) -> &'static str {
    match dim {
        ScorecardDimension::Settings => "SETTINGS",
        ScorecardDimension::Tag => "TAG",
        ScorecardDimension::Model => "KEY",
    }
}

/// Format a list of scorecard rows as a human-readable table.
pub fn format_scorecard(rows: &[ScorecardBucket], dimension: ScorecardDimension) -> String {
    if rows.is_empty() {
        return "(no runs)".to_string();
    }

    let key_col = key_header(dimension);

    let mut lines = Vec::new();
    lines.push(
        format!(
            "MODEL  {}          N  GATES  PARSE_FAIL  LENGTH_FIN  AFT_RATE  TURNS_MEAN  REPAIRS  VERIF_RET  WALL_S  PEAK_CXT  RECLAIMED",
            key_col
        ),
    );

    for row in rows {
        let key_display = if row.key.is_empty() {
            "—".to_string()
        } else {
            row.key.clone()
        };

        let length_finish = row
            .length_finish_rate_mean
            .map(|v| format!("{:.2}", v))
            .unwrap_or_else(|| "—".to_string());

        let aft = row
            .approved_first_try_rate
            .map(|v| format!("{:.2}", v))
            .unwrap_or_else(|| "—".to_string());

        let peak_cxt = row
            .peak_context_pct_mean
            .map(|v| format!("{:.0}%", v * 100.0))
            .unwrap_or_else(|| "—".to_string());

        let reclaimed = match row.tokens_reclaimed_mean {
            None => "—".to_string(),
            Some(v) if v >= 1024.0 => format!("{:.0}k", v / 1024.0),
            Some(v) => format!("{:.0}", v),
        };

        lines.push(format!(
            "{:<7} {:<15} {:>2}  {:>5.2}  {:>9.2}  {:>9}  {:>8}  {:>9.2}  {:>7.2}  {:>9.2}  {:>6.1}  {:>8}  {:>9}",
            row.model,
            key_display,
            row.n_runs,
            row.gates_pass_rate,
            row.parse_failure_rate_mean,
            length_finish,
            aft,
            row.turns_mean,
            row.repairs_per_call_mean,
            row.verifier_retries_mean,
            row.wall_clock_s_mean,
            peak_cxt,
            reclaimed,
        ));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::ai::types::TokenBreakdown;
    use rexymcp_executor::store::telemetry::{Gates, GenerationParams, PhaseRun};
    use tempfile::TempDir;

    fn make_run(
        model: &str,
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
            tags: vec!["rust".to_string()],
            status: "complete".to_string(),
            escalated: false,
            gates: Gates {
                fmt: Some(true),
                build: Some(true),
                lint: Some(true),
                test: Some(true),
            },
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
            project_id: None,
            milestone_id: None,
            tier_telemetry: Default::default(),
            ..Default::default()
        }
    }

    #[test]
    fn load_scorecard_reads_and_aggregates() {
        let dir = TempDir::new().unwrap();
        let telemetry_dir = dir.path().join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();

        let run1 = make_run("qwen", Some(0.2), Some(42), Some(0.25));
        let run2 = make_run("qwen", Some(0.2), Some(42), None);

        let file = telemetry_dir.join("phase_runs.jsonl");
        std::fs::write(
            &file,
            format!(
                "{}\n{}\n",
                serde_json::to_string(&run1)
                    .unwrap()
                    .replacen('{', "{\"schema_version\":1,", 1),
                serde_json::to_string(&run2)
                    .unwrap()
                    .replacen('{', "{\"schema_version\":1,", 1),
            ),
        )
        .unwrap();

        let filter = ScorecardFilter::default();
        let rows = load_scorecard(
            Path::new("/dev/null"),
            Some(&file),
            ScorecardDimension::Settings,
            &filter,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model, "qwen");
        assert_eq!(rows[0].key, "temp=0.2,seed=42");
        assert_eq!(rows[0].n_runs, 2);
        assert!(rows[0].length_finish_rate_mean.is_some());
        assert!((rows[0].length_finish_rate_mean.unwrap() - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn load_scorecard_telemetry_disabled_errors() {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("rexymcp.toml");
        std::fs::write(
            &config,
            r#"
[executor]
provider = "openai"
base_url = "http://localhost:8000/v1"
model = "qwen"

[telemetry]
enabled = false
"#,
        )
        .unwrap();

        let filter = ScorecardFilter::default();
        let err = load_scorecard(&config, None, ScorecardDimension::Settings, &filter).unwrap_err();
        assert!(
            err.contains("telemetry disabled"),
            "expected telemetry disabled error: {err}"
        );
    }

    #[test]
    fn format_scorecard_shows_dropped_columns() {
        let rows = vec![ScorecardBucket {
            model: "qwen".to_string(),
            key: "temp=0.2,seed=42".to_string(),
            n_runs: 2,
            gates_pass_rate: 1.0,
            parse_failure_rate_mean: 0.1,
            length_finish_rate_mean: Some(0.25),
            repairs_per_call_mean: 1.5,
            tool_success_rate_mean: 0.9,
            verifier_retries_mean: 3.0,
            turns_mean: 7.0,
            wall_clock_s_mean: 12.5,
            escalation_rate: 0.0,
            n_with_verdict: 0,
            approved_first_try_rate: None,
            bounces_to_approval_mean: None,
            peak_context_pct_mean: None,
            tokens_reclaimed_mean: None,
        }];

        let out = format_scorecard(&rows, ScorecardDimension::Settings);
        // Verify the new columns are present
        assert!(out.contains("REPAIRS"), "expected REPAIRS header: {out}");
        assert!(
            out.contains("VERIF_RET"),
            "expected VERIF_RET header: {out}"
        );
        assert!(out.contains("WALL_S"), "expected WALL_S header: {out}");
        // Verify the values are rendered
        assert!(
            out.contains("1.50"),
            "expected repairs_per_call_mean=1.50 in output: {out}"
        );
        assert!(
            out.contains("3.00"),
            "expected verifier_retries_mean=3.00 in output: {out}"
        );
        assert!(
            out.contains("12.5"),
            "expected wall_clock_s_mean=12.5 in output: {out}"
        );
    }

    #[test]
    fn format_scorecard_key_header_follows_dimension() {
        let rows = vec![ScorecardBucket {
            model: "qwen".to_string(),
            key: "temp=0.2".to_string(),
            n_runs: 1,
            gates_pass_rate: 1.0,
            parse_failure_rate_mean: 0.1,
            length_finish_rate_mean: None,
            repairs_per_call_mean: 0.5,
            tool_success_rate_mean: 0.9,
            verifier_retries_mean: 1.0,
            turns_mean: 5.0,
            wall_clock_s_mean: 8.0,
            escalation_rate: 0.0,
            n_with_verdict: 0,
            approved_first_try_rate: None,
            bounces_to_approval_mean: None,
            peak_context_pct_mean: None,
            tokens_reclaimed_mean: None,
        }];

        // Settings dimension → "SETTINGS" header
        let out_settings = format_scorecard(&rows, ScorecardDimension::Settings);
        assert!(
            out_settings.contains("SETTINGS"),
            "expected SETTINGS header for Settings dimension: {out_settings}"
        );

        // Tag dimension → "TAG" header
        let out_tag = format_scorecard(&rows, ScorecardDimension::Tag);
        assert!(
            out_tag.contains("TAG"),
            "expected TAG header for Tag dimension: {out_tag}"
        );
        assert!(
            !out_tag.contains("SETTINGS"),
            "expected no SETTINGS header for Tag dimension: {out_tag}"
        );

        // Model dimension → "KEY" header
        let out_model = format_scorecard(&rows, ScorecardDimension::Model);
        assert!(
            out_model.contains("KEY"),
            "expected KEY header for Model dimension: {out_model}"
        );
        assert!(
            !out_model.contains("SETTINGS"),
            "expected no SETTINGS header for Model dimension: {out_model}"
        );
        assert!(
            !out_model.contains("TAG"),
            "expected no TAG header for Model dimension: {out_model}"
        );
    }

    #[test]
    fn format_scorecard_empty_is_no_runs() {
        let out = format_scorecard(&[], ScorecardDimension::Settings);
        assert!(out.contains("(no runs)"));
    }

    #[test]
    fn format_scorecard_none_columns_render_dash() {
        let rows = vec![ScorecardBucket {
            model: "gemma".to_string(),
            key: "default".to_string(),
            n_runs: 1,
            gates_pass_rate: 0.5,
            parse_failure_rate_mean: 0.2,
            length_finish_rate_mean: None,
            repairs_per_call_mean: 0.3,
            tool_success_rate_mean: 0.8,
            verifier_retries_mean: 1.0,
            turns_mean: 5.0,
            wall_clock_s_mean: 8.0,
            escalation_rate: 0.0,
            n_with_verdict: 0,
            approved_first_try_rate: None,
            bounces_to_approval_mean: None,
            peak_context_pct_mean: None,
            tokens_reclaimed_mean: None,
        }];
        let out = format_scorecard(&rows, ScorecardDimension::Settings);
        assert!(
            out.contains('—'),
            "expected '—' for None length_finish_rate_mean: {out}"
        );
    }

    #[test]
    fn load_scorecard_by_tag_and_by_model() {
        let dir = TempDir::new().unwrap();
        let telemetry_dir = dir.path().join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();

        let run1 = make_run("qwen", Some(0.2), Some(42), None);
        let run2 = make_run("gemma", Some(0.7), Some(1), None);

        let file = telemetry_dir.join("phase_runs.jsonl");
        std::fs::write(
            &file,
            format!(
                "{}\n{}\n",
                serde_json::to_string(&run1)
                    .unwrap()
                    .replacen('{', "{\"schema_version\":1,", 1),
                serde_json::to_string(&run2)
                    .unwrap()
                    .replacen('{', "{\"schema_version\":1,", 1),
            ),
        )
        .unwrap();

        let filter = ScorecardFilter::default();

        // By tag: both runs have tag "rust" but different models, so two buckets
        let rows = load_scorecard(
            Path::new("/dev/null"),
            Some(&file),
            ScorecardDimension::Tag,
            &filter,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert!(
                !row.key.is_empty(),
                "Tag dimension should have non-empty key, got: {}",
                row.key
            );
        }

        // By model: two models, each with empty key
        let rows = load_scorecard(
            Path::new("/dev/null"),
            Some(&file),
            ScorecardDimension::Model,
            &filter,
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert!(
                row.key.is_empty(),
                "Model dimension should have empty key, got: {}",
                row.key
            );
        }
    }
}
