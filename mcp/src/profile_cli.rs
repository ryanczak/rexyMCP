//! Model × tag capability profile CLI — `rexymcp profile` subcommand.

use std::path::Path;

use rexymcp_executor::config::Config;
use rexymcp_executor::store::telemetry::PhaseRun;

use crate::profile::{self, FailureClassCount, ModelProfile, PhaseCost};
use crate::runs::{fmt_cost, fmt_tokens};
use crate::scorecard::ScorecardFilter;

/// Resolve the telemetry store path from config, read, aggregate into profiles,
/// and return `ModelProfile` records.
pub fn load_profiles(
    config_path: &Path,
    telemetry_path: Option<&Path>,
    filter: &ScorecardFilter,
) -> Result<Vec<ModelProfile>, String> {
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
    // aggregate_profiles folds internally — pass raw runs + reviews (no fold_reviews).
    Ok(profile::aggregate_profiles(&runs, &reviews, filter))
}

/// Resolve the telemetry store path from config, read, aggregate into phase costs,
/// and return them.
pub fn load_phase_costs(
    config_path: &Path,
    telemetry_path: Option<&Path>,
    filter: &ScorecardFilter,
) -> Result<Vec<PhaseCost>, String> {
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
    Ok(profile::aggregate_phase_costs(&runs, &reviews, filter))
}

/// Format a list of model profiles as a human-readable table.
pub fn format_profiles(rows: &[ModelProfile]) -> String {
    if rows.is_empty() {
        return "(no profiles)".to_string();
    }

    let mut lines = Vec::new();
    lines.push("MODEL  TAG  N  GATES  AFT  BOUNCES  TOOL  PARSE  ESC  WEAKNESSES".to_string());

    for row in rows {
        let aft = row
            .approved_first_try_rate
            .map(|v| format!("{:.2}", v))
            .unwrap_or_else(|| "—".to_string());

        let bounces = row
            .bounces_to_approval_mean
            .map(|v| format!("{:.2}", v))
            .unwrap_or_else(|| "—".to_string());

        let tool = format!("{:.2}", row.tool_success_rate_mean);
        let parse = format!("{:.2}", row.parse_failure_rate_mean);
        let esc = format!("{:.2}", row.escalation_rate);

        let weaknesses = format_weaknesses(&row.ranked_failure_classes);

        lines.push(format!(
            "{:<7} {:<6} {:>2}  {:>5.2}  {:>4}  {:>7}  {:>4}  {:>5}  {:>3}  {}",
            row.model,
            row.tag,
            row.n_runs,
            row.gates_pass_rate,
            aft,
            bounces,
            tool,
            parse,
            esc,
            weaknesses,
        ));
    }

    lines.join("\n")
}

/// Render ranked failure classes as `class×count` entries joined by spaces.
/// Non-attributable classes (spec_bug, infra_blip) are rendered parenthesized.
fn format_weaknesses(classes: &[FailureClassCount]) -> String {
    if classes.is_empty() {
        return "—".to_string();
    }

    classes
        .iter()
        .map(|c| {
            let entry = format!("{}×{}", c.class, c.count);
            if profile::is_model_attributable(&c.class) {
                entry
            } else {
                format!("({})", entry)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Format phase cost-to-ship rows as a human-readable table.
pub fn format_phase_costs(rows: &[PhaseCost], config: &Config) -> String {
    if rows.is_empty() {
        return "(no shipped phases)".to_string();
    }

    let mut lines = Vec::new();
    lines.push("PHASE  MILESTONE  ATTEMPTS  VERDICT  TOKENS  COST".to_string());

    for row in rows {
        let cost = rexymcp_executor::store::metrics::token_cost(
            &row.tokens,
            &config.model_rates(&row.model),
        );
        let tokens = fmt_tokens(row.tokens.total());
        let cost_str = fmt_cost(cost);
        let milestone = row.milestone_id.as_deref().unwrap_or("—");
        let phase_label = phase_label_str(row);
        lines.push(format!(
            "{:<40} {:<12} {:>8}  {:<20}  {:>10}  {:>8}",
            phase_label, milestone, row.attempts, row.verdict, tokens, cost_str
        ));
    }

    lines.join("\n")
}

/// Derive the human phase label: file stem from `phase_doc_path` when present,
/// falling back to `phase_id`.
fn phase_label_str(row: &PhaseCost) -> String {
    row.phase_doc_path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).file_stem().and_then(|s| s.to_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| row.phase_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::store::telemetry::{
        Gates, GenerationParams, PhaseReview, PhaseRun, REVIEW_RECORD_TAG,
    };
    use tempfile::TempDir;

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
            ..Default::default()
        }
    }

    fn make_review(ts: u64, verdict: &str, failure_class: &[&str]) -> PhaseReview {
        PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts,
            phase_doc_path: None,
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
    fn load_profiles_reads_and_aggregates() {
        let dir = TempDir::new().unwrap();
        let telemetry_dir = dir.path().join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();

        let run = make_run(100, "claude", &["kind=feature"], None);
        // Review with approved_first_try verdict — must fold in via aggregate_profiles
        let review = make_review(200, "approved_first_try", &["none"]);

        let file = telemetry_dir.join("phase_runs.jsonl");
        std::fs::write(
            &file,
            format!(
                "{}\n{}\n",
                serde_json::to_string(&run)
                    .unwrap()
                    .replacen('{', "{\"schema_version\":1,", 1),
                serde_json::to_string(&review)
                    .unwrap()
                    .replacen('{', "{\"schema_version\":1,", 1),
            ),
        )
        .unwrap();

        let filter = ScorecardFilter {
            tags: &[],
            model: None,
            min_runs: 0,
        };
        let profiles = load_profiles(Path::new("/dev/null"), Some(&file), &filter).unwrap();

        assert_eq!(profiles.len(), 1);
        let p = &profiles[0];
        assert_eq!(p.model, "claude");
        assert_eq!(p.tag, "kind=feature");
        // approved_first_try_rate must be Some(1.0) — the review folded in
        assert_eq!(p.approved_first_try_rate, Some(1.0));
    }

    #[test]
    fn load_profiles_telemetry_disabled_errors() {
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

        let filter = ScorecardFilter {
            tags: &[],
            model: None,
            min_runs: 0,
        };
        let err = load_profiles(&config, None, &filter).unwrap_err();
        assert!(
            err.contains("telemetry disabled"),
            "expected telemetry disabled error: {err}"
        );
    }

    #[test]
    fn format_profiles_shows_strengths_and_weaknesses() {
        let rows = vec![ModelProfile {
            model: "claude".to_string(),
            tag: "kind=feature".to_string(),
            n_runs: 4,
            gates_pass_rate: 0.75,
            tool_success_rate_mean: 0.9,
            parse_failure_rate_mean: 0.05,
            escalation_rate: 0.25,
            n_with_verdict: 2,
            approved_first_try_rate: Some(0.5),
            bounces_to_approval_mean: Some(1.0),
            n_reviews: 1,
            ranked_failure_classes: vec![FailureClassCount {
                class: "prod_unwrap".to_string(),
                count: 2,
            }],
        }];

        let out = format_profiles(&rows);
        assert!(out.contains("MODEL"), "expected MODEL header: {out}");
        assert!(out.contains("claude"), "expected model name: {out}");
        assert!(out.contains("prod_unwrap"), "expected prod_unwrap: {out}");
        assert!(out.contains("0.50"), "expected AFT 0.50: {out}");
    }

    #[test]
    fn format_profiles_marks_non_attributable() {
        let rows = vec![ModelProfile {
            model: "claude".to_string(),
            tag: "kind=feature".to_string(),
            n_runs: 3,
            gates_pass_rate: 0.67,
            tool_success_rate_mean: 0.8,
            parse_failure_rate_mean: 0.1,
            escalation_rate: 0.33,
            n_with_verdict: 1,
            approved_first_try_rate: Some(1.0),
            bounces_to_approval_mean: None,
            n_reviews: 2,
            ranked_failure_classes: vec![
                FailureClassCount {
                    class: "prod_unwrap".to_string(),
                    count: 2,
                },
                FailureClassCount {
                    class: "spec_bug".to_string(),
                    count: 1,
                },
            ],
        }];

        let out = format_profiles(&rows);
        // spec_bug must be parenthesized
        assert!(
            out.contains("(spec_bug"),
            "expected spec_bug parenthesized: {out}"
        );
        // prod_unwrap must NOT be parenthesized — find it and check no '(' immediately before
        let prod_idx = out
            .find("prod_unwrap")
            .expect("expected prod_unwrap in output: {out}");
        if prod_idx > 0 {
            assert_ne!(
                out.chars().nth(prod_idx - 1),
                Some('('),
                "prod_unwrap should not be parenthesized: {out}"
            );
        }
    }

    #[test]
    fn format_profiles_empty_is_no_profiles() {
        let out = format_profiles(&[]);
        assert!(out.contains("(no profiles)"));
    }

    #[test]
    fn format_phase_costs_empty_is_no_shipped_phases() {
        let cfg = Config::default();
        let out = format_phase_costs(&[], &cfg);
        assert!(out.contains("(no shipped phases)"));
    }

    #[test]
    fn format_phase_costs_renders_columns() {
        let cfg = Config::default();
        let rows = vec![PhaseCost {
            phase_id: "phase-05a-iii".to_string(),
            phase_doc_path: None,
            milestone_id: Some("M35".to_string()),
            model: "AEON-7".to_string(),
            attempts: 2,
            verdict: "approved_after_1".to_string(),
            tokens: rexymcp_executor::ai::types::TokenBreakdown {
                input_tokens: 1000,
                output_tokens: 500,
                cache_read_tokens: 200,
                cache_write_tokens: 100,
            },
        }];
        let out = format_phase_costs(&rows, &cfg);
        assert!(out.contains("PHASE"));
        assert!(out.contains("ATTEMPTS"));
        assert!(out.contains("VERDICT"));
        assert!(out.contains("TOKENS"));
        assert!(out.contains("COST"));
        assert!(out.contains("—"));
    }

    #[test]
    fn phase_label_uses_doc_path_stem_when_present() {
        let row = PhaseCost {
            phase_id: "phase-05".to_string(),
            phase_doc_path: Some(
                "docs/dev/milestones/M35/phase-05a-iii-scorecard-by-cli.md".to_string(),
            ),
            milestone_id: Some("M35".to_string()),
            model: "AEON-7".to_string(),
            attempts: 1,
            verdict: "approved_first_try".to_string(),
            tokens: Default::default(),
        };
        let label = phase_label_str(&row);
        assert_eq!(label, "phase-05a-iii-scorecard-by-cli");
    }

    #[test]
    fn phase_label_falls_back_to_phase_id_when_no_doc_path() {
        let row = PhaseCost {
            phase_id: "phase-05".to_string(),
            phase_doc_path: None,
            milestone_id: Some("M35".to_string()),
            model: "AEON-7".to_string(),
            attempts: 1,
            verdict: "approved_first_try".to_string(),
            tokens: Default::default(),
        };
        let label = phase_label_str(&row);
        assert_eq!(label, "phase-05");
    }
}
