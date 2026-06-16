//! Model × settings scorecard CLI — `rexymcp scorecard` subcommand.

use std::path::Path;

use rexymcp_executor::config::Config;
use rexymcp_executor::store::telemetry::PhaseRun;

use crate::scorecard::{ScorecardFilter, SettingsScorecardRow, aggregate_by_settings};

/// Resolve the telemetry store path from config, read, aggregate by settings,
/// and return `SettingsScorecardRow` records.
pub fn load_settings_scorecard(
    config_path: &Path,
    telemetry_path: Option<&Path>,
    filter: &ScorecardFilter,
) -> Result<Vec<SettingsScorecardRow>, String> {
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
    Ok(aggregate_by_settings(&runs, filter))
}

/// Format a list of settings-scorecard rows as a human-readable table.
pub fn format_settings_scorecard(rows: &[SettingsScorecardRow]) -> String {
    if rows.is_empty() {
        return "(no runs)".to_string();
    }

    let mut lines = Vec::new();
    lines.push(
        "MODEL  SETTINGS          N  GATES  PARSE_FAIL  LENGTH_FIN  AFT_RATE  TURNS_MEAN  PEAK_CXT  RECLAIMED"
            .to_string(),
    );

    for row in rows {
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
            "{:<7} {:<15} {:>2}  {:>5.2}  {:>9.2}  {:>9}  {:>8}  {:>9.2}  {:>8}  {:>9}",
            row.model,
            row.settings,
            row.n_runs,
            row.gates_pass_rate,
            row.parse_failure_rate_mean,
            length_finish,
            aft,
            row.turns_mean,
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
        }
    }

    #[test]
    fn load_settings_scorecard_reads_and_aggregates() {
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
                serde_json::to_string(&run1).unwrap(),
                serde_json::to_string(&run2).unwrap(),
            ),
        )
        .unwrap();

        let filter = ScorecardFilter::default();
        let rows = load_settings_scorecard(Path::new("/dev/null"), Some(&file), &filter).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model, "qwen");
        assert_eq!(rows[0].settings, "temp=0.2,seed=42");
        assert_eq!(rows[0].n_runs, 2);
        assert!(rows[0].length_finish_rate_mean.is_some());
        assert!((rows[0].length_finish_rate_mean.unwrap() - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn load_settings_scorecard_telemetry_disabled_errors() {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("rexymcp.toml");
        std::fs::write(
            &config,
            r#"
[executor]
provider = "openai"
base_url = "http://localhost:8000/v1"
model = "qwen"
"#,
        )
        .unwrap();

        let filter = ScorecardFilter::default();
        let err = load_settings_scorecard(&config, None, &filter).unwrap_err();
        assert!(
            err.contains("telemetry disabled"),
            "expected telemetry disabled error: {err}"
        );
    }

    #[test]
    fn format_settings_scorecard_shows_settings_and_signal() {
        let rows = vec![SettingsScorecardRow {
            model: "qwen".to_string(),
            settings: "temp=0.2,seed=42".to_string(),
            n_runs: 2,
            gates_pass_rate: 1.0,
            parse_failure_rate_mean: 0.1,
            length_finish_rate_mean: Some(0.25),
            repairs_per_call_mean: 0.5,
            tool_success_rate_mean: 0.9,
            verifier_retries_mean: 2.0,
            turns_mean: 7.0,
            wall_clock_s_mean: 12.5,
            escalation_rate: 0.0,
            n_with_verdict: 0,
            approved_first_try_rate: None,
            bounces_to_approval_mean: None,
            peak_context_pct_mean: Some(0.71),
            tokens_reclaimed_mean: Some(9216.0),
        }];

        let out = format_settings_scorecard(&rows);
        assert!(
            out.contains("temp=0.2,seed=42"),
            "expected settings label in output: {out}"
        );
        assert!(
            out.contains("0.25"),
            "expected length_finish_rate_mean in output: {out}"
        );
        assert!(out.contains("PEAK_CXT"), "expected PEAK_CXT header: {out}");
        assert!(
            out.contains("RECLAIMED"),
            "expected RECLAIMED header: {out}"
        );
        assert!(
            out.contains("71%"),
            "expected 71% for peak_context_pct_mean=0.71: {out}"
        );
        assert!(
            out.contains("9k"),
            "expected 9k for tokens_reclaimed_mean=9216: {out}"
        );

        // None means render as "—"
        let rows_none = vec![SettingsScorecardRow {
            model: "gemma".to_string(),
            settings: "default".to_string(),
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
        let out_none = format_settings_scorecard(&rows_none);
        assert!(
            out_none.contains('—'),
            "expected '—' for None length_finish_rate_mean: {out_none}"
        );
        // Sentinel for the new columns: the row must not render "0%" or "0" for the absent means
        assert!(
            !out_none.contains("0%"),
            "expected no '0%' for None peak_context_pct_mean: {out_none}"
        );
    }

    #[test]
    fn format_settings_scorecard_empty_is_no_runs() {
        let out = format_settings_scorecard(&[]);
        assert!(out.contains("(no runs)"));
    }
}
