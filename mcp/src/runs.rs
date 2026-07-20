//! Per-run statistics view — `rexymcp runs` CLI subcommand.

use std::path::Path;

use rexymcp_executor::config::Config;
use rexymcp_executor::store::metrics;
use rexymcp_executor::store::telemetry::PhaseRun;

/// Filter applied to the raw list of `PhaseRun` records before display.
pub struct RunsFilter<'a> {
    /// Exact model match. `None` = all models.
    pub model: Option<&'a str>,
    /// Run's `tags` must contain **all** of these (AND). Empty = no tag filter.
    pub tags: &'a [String],
    /// Cap on rows after sorting (most recent first). `0` = no cap.
    pub limit: usize,
}

/// Filter, sort newest-first, and cap. Pure.
pub fn select(mut runs: Vec<PhaseRun>, filter: &RunsFilter) -> Vec<PhaseRun> {
    runs.retain(|r| {
        if let Some(m) = filter.model
            && r.model != m
        {
            return false;
        }
        if !filter.tags.is_empty() && !filter.tags.iter().all(|t| r.tags.contains(t)) {
            return false;
        }
        true
    });
    runs.sort_by_key(|r| std::cmp::Reverse(r.ts));
    if filter.limit != 0 && runs.len() > filter.limit {
        runs.truncate(filter.limit);
    }
    runs
}

/// Compact "5s" / "3m12s" / "1h04m" / "2d" age string from a millisecond span.
fn humanize_age(age_ms: u64) -> String {
    let secs = age_ms / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Render one gate field: `Some(true)` → `✓`, everything else → `✗`.
fn gate_char(v: Option<bool>) -> char {
    if v == Some(true) { '✓' } else { '✗' }
}

fn fmt_tokens(total: u32) -> String {
    if total == 0 {
        "—".to_string()
    } else if total >= 1024 {
        format!("{}k", total / 1024)
    } else {
        format!("{total}")
    }
}

/// Cost cell: `—` when unpriced/zero, else `$` with 4 decimals.
fn fmt_cost(cost: f64) -> String {
    if cost == 0.0 {
        "—".to_string()
    } else {
        format!("${cost:.4}")
    }
}

/// Throughput cell: `—` when unmeasured, else whole tok/s.
fn fmt_tok_per_sec(tps: Option<f64>) -> String {
    match tps {
        Some(v) => format!("{v:.0}"),
        None => "—".to_string(),
    }
}

/// Format a list of runs as a human-readable table. `now_ms` is the current
/// unix-millis clock, injected so the age column is testable.
pub fn format_runs(runs: &[PhaseRun], now_ms: u64, config: &Config) -> String {
    if runs.is_empty() {
        return "(no runs)".to_string();
    }

    let mut lines = Vec::new();
    lines.push(
        "ID        AGE     MODEL  TAGS           SETTINGS     GATES  TURNS  STATUS    VERDICT  SERVED_MODEL  TRUNC  CXT_WIN  PEAK_CXT  RECLAIMED  TOKENS  COST      TOK/S".to_string(),
    );

    for run in runs {
        let id = metrics::run_id(run);
        let age = humanize_age(now_ms.saturating_sub(run.ts));

        let tags = run.tags.join(",");

        let settings = metrics::settings_label(&run.generation_params);

        let gates = format!(
            "{}{}{}{}",
            gate_char(run.gates.fmt),
            gate_char(run.gates.build),
            gate_char(run.gates.lint),
            gate_char(run.gates.test),
        );

        let verdict = run.architect_verdict.as_deref().unwrap_or("—");

        let served_model = run.served_model.as_deref().unwrap_or("—");
        let trunc = run
            .length_finish_rate
            .map(|r| format!("{:.0}%", r * 100.0))
            .unwrap_or_else(|| "—".to_string());

        let cxt_win = run
            .context_window
            .map(|n| {
                if n >= 1024 {
                    format!("{}k", n / 1024)
                } else {
                    format!("{}", n)
                }
            })
            .unwrap_or_else(|| "—".to_string());

        let eff = &run.context_efficiency;

        let peak_cxt = if eff.peak_context_pct == 0.0 {
            "—".to_string()
        } else {
            format!("{:.0}%", eff.peak_context_pct * 100.0)
        };

        let reclaimed_total = metrics::reclaimed_total(eff);
        let reclaimed = if reclaimed_total == 0 {
            "—".to_string()
        } else if reclaimed_total >= 1024 {
            format!("{}k", reclaimed_total / 1024)
        } else {
            format!("{}", reclaimed_total)
        };

        let tokens_cell = fmt_tokens(run.tokens.total());
        let rates = config.model_rates(&run.model);
        let cost_cell = fmt_cost(metrics::token_cost(&run.tokens, &rates));
        let tps_cell = fmt_tok_per_sec(metrics::tokens_per_sec(
            run.tokens.output_tokens,
            run.gen_time_s,
        ));

        lines.push(format!(
            "{:<9} {:<7} {:<6} {:<14} {:<12} {}  {:<6} {:<9} {:<11} {:<13} {:<7} {:<7} {:<10} {:<7} {:<9} {:<6} {:<6}",
            id,
            age,
            run.model,
            tags,
            settings,
            gates,
            run.turns,
            run.status,
            verdict,
            served_model,
            trunc,
            cxt_win,
            peak_cxt,
            reclaimed,
            tokens_cell,
            cost_cell,
            tps_cell,
        ));
    }

    lines.join("\n")
}

/// Resolve the telemetry store path from config, read, filter, and return
/// matching `PhaseRun` records.
pub fn load_runs(
    config_path: &Path,
    telemetry_path: Option<&Path>,
    filter: &RunsFilter,
) -> Result<Vec<PhaseRun>, String> {
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

    let runs =
        rexymcp_executor::store::telemetry::read(&telemetry_file).map_err(|e| e.to_string())?;
    let reviews = rexymcp_executor::store::telemetry::read_reviews(&telemetry_file)
        .map_err(|e| e.to_string())?;
    let runs = rexymcp_executor::store::telemetry::fold_reviews(runs, &reviews);
    Ok(select(runs, filter))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::store::telemetry::{Gates, GenerationParams};
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

    fn make_run_with_params(
        ts: u64,
        model: &str,
        tags: &[&str],
        temperature: Option<f64>,
        seed: Option<u64>,
    ) -> PhaseRun {
        PhaseRun {
            ts,
            model: model.to_string(),
            generation_params: GenerationParams { temperature, seed },
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
            architect_verdict: None,
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

    #[test]
    fn select_filters_by_model_exact() {
        let runs = vec![
            make_run(1000, "qwen", &["rust"], None),
            make_run(2000, "gemma", &["rust"], None),
            make_run(3000, "qwen", &["feature"], None),
        ];
        let filter = RunsFilter {
            model: Some("qwen"),
            tags: &[],
            limit: 0,
        };
        let result = select(runs, &filter);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|r| r.model == "qwen"));
    }

    #[test]
    fn select_requires_all_tags() {
        let runs = vec![
            make_run(1000, "qwen", &["rust", "feature"], None),
            make_run(2000, "qwen", &["rust"], None),
        ];
        let filter = RunsFilter {
            model: None,
            tags: &["rust".to_string(), "feature".to_string()],
            limit: 0,
        };
        let result = select(runs, &filter);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].tags, ["rust", "feature"]);
    }

    #[test]
    fn select_sorts_newest_first() {
        let runs = vec![
            make_run(100, "qwen", &[], None),
            make_run(300, "qwen", &[], None),
            make_run(200, "qwen", &[], None),
        ];
        let filter = RunsFilter {
            model: None,
            tags: &[],
            limit: 0,
        };
        let result = select(runs, &filter);
        assert_eq!(result[0].ts, 300);
        assert_eq!(result[1].ts, 200);
        assert_eq!(result[2].ts, 100);
    }

    #[test]
    fn select_limit_caps_after_sort() {
        let runs: Vec<PhaseRun> = (0..5)
            .map(|i| make_run((i + 1) * 100, "qwen", &[], None))
            .collect();
        let filter = RunsFilter {
            model: None,
            tags: &[],
            limit: 2,
        };
        let result = select(runs.clone(), &filter);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].ts, 500);
        assert_eq!(result[1].ts, 400);

        let filter_all = RunsFilter {
            model: None,
            tags: &[],
            limit: 0,
        };
        let result_all = select(runs, &filter_all);
        assert_eq!(result_all.len(), 5);
    }

    #[test]
    fn format_runs_includes_model_and_verdict() {
        let runs = vec![
            make_run(1000, "qwen", &["rust"], Some("approved_first_try")),
            make_run(2000, "gemma", &["feature"], None),
        ];
        let out = format_runs(&runs, 5000, &Config::default());
        assert!(out.contains("qwen"));
        assert!(out.contains("approved_first_try"));
        assert!(out.contains("gemma"));
        assert!(out.contains("—"));
    }

    #[test]
    fn format_runs_renders_default_settings() {
        let runs = vec![
            make_run_with_params(1000, "qwen", &[], None, None),
            make_run_with_params(2000, "gemma", &[], Some(0.2), None),
        ];
        let out = format_runs(&runs, 5000, &Config::default());
        assert!(
            out.contains("default"),
            "expected 'default' in output: {out}"
        );
        assert!(out.contains("0.2"), "expected '0.2' in output: {out}");
    }

    #[test]
    fn format_runs_empty_is_no_runs_line() {
        let out = format_runs(&[], 5000, &Config::default());
        assert!(out.contains("(no runs)"));
    }

    #[test]
    fn load_runs_reads_and_selects() {
        let dir = TempDir::new().unwrap();
        let telemetry_dir = dir.path().join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();

        let run1 = make_run(1000, "qwen", &["rust"], None);
        let run2 = make_run(2000, "gemma", &["feature"], Some("good"));

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

        let filter = RunsFilter {
            model: None,
            tags: &[],
            limit: 0,
        };
        let result = load_runs(Path::new("/dev/null"), Some(&file), &filter).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].ts, 2000); // newest first
        assert_eq!(result[1].ts, 1000);
    }

    #[test]
    fn load_runs_telemetry_disabled_errors() {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("rexymcp.toml");
        // Config with no [telemetry] section
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

        let filter = RunsFilter {
            model: None,
            tags: &[],
            limit: 0,
        };
        let err = load_runs(&config, None, &filter).unwrap_err();
        assert!(
            err.contains("telemetry disabled"),
            "expected telemetry disabled error: {err}"
        );
    }

    #[test]
    fn humanize_age_buckets() {
        assert_eq!(humanize_age(5_000), "5s");
        assert_eq!(humanize_age(192_000), "3m12s");
        assert_eq!(humanize_age(3_840_000), "1h");
        assert_eq!(humanize_age(172_800_000), "2d");
    }

    #[test]
    fn format_runs_shows_served_model_and_truncation() {
        let mut run_with_provenance = make_run(1000, "qwen", &["rust"], None);
        run_with_provenance.served_model = Some("qwen-served".into());
        run_with_provenance.length_finish_rate = Some(0.25);

        let run_without_provenance = make_run(2000, "gemma", &["feature"], None);

        let runs = vec![run_with_provenance, run_without_provenance];
        let out = format_runs(&runs, 5000, &Config::default());

        // Run with provenance: served model and truncation rate appear
        assert!(
            out.contains("qwen-served"),
            "expected served model in output: {out}"
        );
        assert!(
            out.contains("25%"),
            "expected 25%% truncation rate in output: {out}"
        );

        // Run without provenance: both render as "—"
        // The gemma line should contain two "—" sentinels for served_model and trunc
        let gemma_line = out
            .lines()
            .find(|l| l.contains("gemma"))
            .expect("expected a gemma line in output: {out}");
        // Count "—" occurrences on the gemma line — verdict is also "—", so we need at least 3
        let dash_count = gemma_line.matches('—').count();
        assert!(
            dash_count >= 3,
            "expected at least 3 '—' sentinels on gemma line (verdict + served_model + trunc): {gemma_line}"
        );
    }

    #[test]
    fn format_runs_shows_context_window() {
        let mut run_with_cxt = make_run(1000, "qwen", &["rust"], None);
        run_with_cxt.context_window = Some(262_144);

        let run_without_cxt = make_run(2000, "gemma", &["feature"], None);

        let runs = vec![run_with_cxt, run_without_cxt];
        let out = format_runs(&runs, 5000, &Config::default());

        // Run with context window: compact form appears (262144 / 1024 = 256k)
        assert!(
            out.contains("256k"),
            "expected 256k context window in output: {out}"
        );

        // Run without context window: renders as "—"
        let gemma_line = out
            .lines()
            .find(|l| l.contains("gemma"))
            .expect("expected a gemma line in output: {out}");
        assert!(
            gemma_line.contains('—'),
            "expected '—' sentinel for missing context window on gemma line: {gemma_line}"
        );
    }

    #[test]
    fn format_runs_shows_context_efficiency_columns() {
        use rexymcp_executor::store::telemetry::ContextEfficiency;

        let mut run = make_run(1000, "qwen", &["rust"], None);
        run.context_efficiency = ContextEfficiency {
            peak_context_pct: 0.68,
            compaction_count: 2,
            compaction_tokens_reclaimed: 8000,
            output_filtered_tokens: 3000,
            read_evicted_tokens: 1000,
            read_deduped_tokens: 288,
        };

        let out = format_runs(&[run], 5000, &Config::default());

        // Header contains both new column names
        assert!(
            out.lines().next().unwrap().contains("PEAK_CXT"),
            "expected PEAK_CXT in header: {out}"
        );
        assert!(
            out.lines().next().unwrap().contains("RECLAIMED"),
            "expected RECLAIMED in header: {out}"
        );

        // Run line shows 68% and 12k (8000+3000+1000+288 = 12288 → 12k)
        let qwen_line = out
            .lines()
            .find(|l| l.contains("qwen"))
            .expect("expected a qwen line in output: {out}");
        assert!(
            qwen_line.contains("68%"),
            "expected 68%% peak context in qwen line: {qwen_line}"
        );
        assert!(
            qwen_line.contains("12k"),
            "expected 12k reclaimed in qwen line: {qwen_line}"
        );
    }

    #[test]
    fn format_runs_reclaimed_sums_all_four_sources() {
        use rexymcp_executor::store::telemetry::ContextEfficiency;

        let mut run = make_run(1000, "qwen", &["rust"], None);
        run.context_efficiency = ContextEfficiency {
            peak_context_pct: 0.0,
            compaction_count: 0,
            compaction_tokens_reclaimed: 20,
            output_filtered_tokens: 100,
            read_evicted_tokens: 50,
            read_deduped_tokens: 30,
        };

        let out = format_runs(&[run], 5000, &Config::default());

        // Sum = 100 + 50 + 30 + 20 = 200 (sub-1024, renders as "200")
        let qwen_line = out
            .lines()
            .find(|l| l.contains("qwen"))
            .expect("expected a qwen line in output: {out}");
        assert!(
            qwen_line.contains("200"),
            "expected 200 reclaimed (sum of all four sources) in qwen line: {qwen_line}"
        );
    }

    #[test]
    fn format_runs_context_efficiency_dashes_when_zero() {
        // make_run already defaults context_efficiency to all-zeros
        let run = make_run(1000, "qwen", &["rust"], None);

        let out = format_runs(&[run], 5000, &Config::default());

        let qwen_line = out
            .lines()
            .find(|l| l.contains("qwen"))
            .expect("expected a qwen line in output: {out}");

        // Both new columns render as "—" sentinel, not "0" or "0%"
        assert!(
            !qwen_line.contains("0%"),
            "expected no '0%%' on qwen line when peak_context_pct is 0: {qwen_line}"
        );
        // The line should have em-dashes for the new columns
        // Count dashes — we need at least the verdict dash plus the two new ones
        let dash_count = qwen_line.matches('—').count();
        assert!(
            dash_count >= 3,
            "expected at least 3 '—' sentinels on qwen line (verdict + peak_cxt + reclaimed): {qwen_line}"
        );
    }

    #[test]
    fn load_runs_folds_review_verdict() {
        use rexymcp_executor::store::telemetry::{
            PhaseReview, REVIEW_RECORD_TAG, append, append_review,
        };
        use std::fs;

        let dir = TempDir::new().unwrap();
        let telemetry_dir = dir.path().join("telemetry");
        fs::create_dir_all(&telemetry_dir).unwrap();

        // Write a config pointing at the telemetry dir
        let config_path = dir.path().join("rexymcp.toml");
        fs::write(
            &config_path,
            format!(
                r#"[project]
id = "test-proj"

[executor]
provider = "openai"
base_url = "http://localhost:8000/v1"
model = "qwen"

[telemetry]
dir = "{}"
"#,
                telemetry_dir.display()
            ),
        )
        .unwrap();

        let phase_doc = "/abs/path/to/phase-05.md";

        // Append a PhaseRun with verdict None
        let run = PhaseRun {
            ts: 1_717_000_000_000,
            model: "qwen".to_string(),
            generation_params: GenerationParams::default(),
            phase_id: "phase-05".to_string(),
            phase_doc_path: Some(phase_doc.to_string()),
            tags: vec!["rust".to_string()],
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
            architect_verdict: None,
            served_model: None,
            length_finish_rate: None,
            context_window: None,
            context_efficiency: Default::default(),
            project_id: Some("test-proj".to_string()),
            milestone_id: None,
            tier_telemetry: Default::default(),
            ..Default::default()
        };
        append(&telemetry_dir, &run).unwrap();

        // Append a matching PhaseReview
        let review = PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 1_717_000_001_000,
            phase_doc_path: Some(phase_doc.to_string()),
            phase_id: "phase-05".to_string(),
            project_id: Some("test-proj".to_string()),
            architect_verdict: "approved_first_try".to_string(),
            bounces_to_approval: Some(0),
            bugs_filed: Some(0),
            warnings: Some(0),
            failure_class: vec!["none".to_string()],
        };
        append_review(&telemetry_dir, &review).unwrap();

        // load_runs should fold the review verdict onto the run
        let filter = RunsFilter {
            model: None,
            tags: &[],
            limit: 0,
        };
        let runs = load_runs(&config_path, None, &filter).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].architect_verdict,
            Some("approved_first_try".to_string())
        );
    }

    #[test]
    fn format_runs_shows_id_tokens_cost_speed_columns() {
        let mut run = make_run(1_717_000_000_000, "qwen", &["rust"], None);
        run.tokens = rexymcp_executor::ai::types::TokenBreakdown {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        run.gen_time_s = 5.0;

        let mut cfg = Config::default();
        cfg.models.insert(
            "qwen".to_string(),
            rexymcp_executor::config::ModelOverride {
                input_per_mtok: Some(2.0),
                output_per_mtok: Some(9.0),
                ..Default::default()
            },
        );

        let out = format_runs(&[run], 5_000, &cfg);
        assert!(out.contains("TOKENS"), "expected TOKENS header: {out}");
        assert!(out.contains("COST"), "expected COST header: {out}");
        assert!(out.contains("TOK/S"), "expected TOK/S header: {out}");
        assert!(out.contains('$'), "expected a $ cost cell: {out}");
        assert!(out.contains("100000"), "expected 100000 tok/s: {out}");
    }

    #[test]
    fn format_runs_unpriced_cost_is_dash() {
        let mut run = make_run(1_717_000_000_000, "qwen", &["rust"], None);
        run.tokens = rexymcp_executor::ai::types::TokenBreakdown {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        let out = format_runs(&[run], 5_000, &Config::default());
        let line = out.lines().find(|l| l.contains("qwen")).expect("qwen line");
        assert!(
            !line.contains('$'),
            "unpriced run must not show a $ cost: {line}"
        );
        assert!(
            line.contains('—'),
            "unpriced cost should render em dash: {line}"
        );
    }

    #[test]
    fn format_runs_zero_gen_time_speed_is_dash() {
        let mut run = make_run(1_717_000_000_000, "qwen", &["rust"], None);
        run.gen_time_s = 0.0;
        let out = format_runs(&[run], 5_000, &Config::default());
        let line = out.lines().find(|l| l.contains("qwen")).expect("qwen line");
        assert!(
            line.trim_end().ends_with('—'),
            "zero gen_time should render TOK/S as em dash: {line}"
        );
    }
}
