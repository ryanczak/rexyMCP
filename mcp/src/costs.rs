//! Cost-report core — `rexymcp costs` CLI.
//!
//! Computes Baseline / Executor / Architect / Net across Session / Milestone /
//! Project scopes. Executor cost is derived from `cfg.model_rates` (phase-03
//! pricing), not hardcoded `$0.00`.

use std::path::Path;

use rexymcp_executor::config::Config;
use rexymcp_executor::store::telemetry::{self, ArchitectTokens, PhaseRun};

use crate::dashboard::{BudgetRates, ScopeCosts};
use crate::status;

/// One scope's four cost lines, in dollars. `baseline`/`net` are `None` when no
/// baseline rate is configured (rendered `—`); `executor`/`architect` are always
/// present (`0.0` when unpriced).
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct ScopeReport {
    pub baseline: Option<f64>,
    pub executor: f64,
    pub architect: Option<f64>,
    pub net: Option<f64>,
}

/// Baseline/Executor/Architect/Net across the three scopes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CostReport {
    pub session: ScopeReport,
    /// `None` when no active milestone could be resolved (no project runs).
    pub milestone: Option<ScopeReport>,
    pub project: ScopeReport,
    pub assists: u32,
}

/// Compute one scope's dollar lines. `exec_rates` are the executor model's
/// `$/Mtok` (from `cfg.model_rates`); `baseline` carries the cloud-baseline +
/// architect rates. u64-safe (does NOT route token totals through the u32
/// `TokenBreakdown`).
pub fn scope_report(
    costs: &ScopeCosts,
    exec_rates: &telemetry::ModelRates,
    baseline: &BudgetRates,
) -> ScopeReport {
    let per_m = |t: u64, r: f64| (t as f64 / 1_000_000.0) * r;
    let no_baseline = baseline.input_per_mtok == 0.0 && baseline.output_per_mtok == 0.0;

    let executor = per_m(costs.executor_in, exec_rates.input_per_mtok)
        + per_m(costs.executor_out, exec_rates.output_per_mtok)
        + per_m(costs.executor_cache_read, exec_rates.cache_read_per_mtok)
        + per_m(
            costs.executor_cache_write,
            exec_rates.cache_creation_per_mtok,
        );
    let architect = costs.architect_cost;
    let baseline_cost = if no_baseline {
        None
    } else {
        Some(
            per_m(costs.executor_in, baseline.input_per_mtok)
                + per_m(costs.executor_out, baseline.output_per_mtok),
        )
    };
    let net = match (baseline_cost, architect) {
        (Some(b), Some(a)) => Some(b - executor - a),
        _ => None,
    };

    ScopeReport {
        baseline: baseline_cost,
        executor,
        architect,
        net,
    }
}

/// Sum executor tokens over project runs, optionally scoped to one milestone_id.
/// Architect cost is priced per-model from the ledger (no milestone scope).
pub(crate) fn scope_costs(
    runs: &[PhaseRun],
    ledgers: &[telemetry::ArchitectLedger],
    architect: &rexymcp_executor::config::ArchitectConfig,
    project_id: &str,
    milestone_id: Option<&str>,
) -> ScopeCosts {
    let exec: ScopeCosts = runs
        .iter()
        .filter(|r| {
            r.project_id.as_deref() == Some(project_id)
                && (milestone_id.is_none() || r.milestone_id.as_deref() == milestone_id)
        })
        .fold(ScopeCosts::default(), |mut c, r| {
            c.executor_in = c.executor_in.saturating_add(r.tokens.input_tokens as u64);
            c.executor_out = c.executor_out.saturating_add(r.tokens.output_tokens as u64);
            c.executor_cache_read = c
                .executor_cache_read
                .saturating_add(r.tokens.cache_read_tokens as u64);
            c.executor_cache_write = c
                .executor_cache_write
                .saturating_add(r.tokens.cache_write_tokens as u64);
            c
        });

    // Architect: attributable at PROJECT scope only (the ledger has no milestone).
    let (architect_tokens, architect_cost) = if milestone_id.is_some() {
        (ArchitectTokens::default(), None)
    } else {
        let mut toks = ArchitectTokens::default();
        let mut cost = 0.0_f64;
        for l in ledgers
            .iter()
            .filter(|l| l.project_id.as_deref() == Some(project_id))
        {
            toks.input = toks.input.saturating_add(l.tokens.input);
            toks.cache_creation = toks.cache_creation.saturating_add(l.tokens.cache_creation);
            toks.cache_read = toks.cache_read.saturating_add(l.tokens.cache_read);
            toks.output = toks.output.saturating_add(l.tokens.output);
            if let Some((inp, out)) = architect.rates_for(&l.model) {
                cost += l.cost(inp, out);
            }
        }
        (toks, Some(cost))
    };

    ScopeCosts {
        executor_in: exec.executor_in,
        executor_out: exec.executor_out,
        executor_cache_read: exec.executor_cache_read,
        executor_cache_write: exec.executor_cache_write,
        architect: architect_tokens,
        architect_cost,
    }
}

/// Load a full cost report from config + repo + telemetry.
pub fn load_cost_report(
    config_path: &Path,
    repo: &Path,
    session: Option<&str>,
    telemetry_path: Option<&Path>,
) -> Result<CostReport, String> {
    let cfg =
        Config::load_with_env(config_path).map_err(|e| format!("failed to load config: {e}"))?;

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

    let baseline = BudgetRates {
        input_per_mtok: cfg.dashboard.effective_rates().0,
        output_per_mtok: cfg.dashboard.effective_rates().1,
        executor: telemetry::ModelRates::default(),
    };
    let exec_rates = cfg.model_rates(&cfg.executor.model);

    // Session scope: from the live session log. No architect cost.
    let session_costs = match status::load_records(repo, session) {
        Ok(records) => {
            let summary = status::summarize(&records);
            ScopeCosts {
                executor_in: summary.last_input_tokens.unwrap_or(0) as u64,
                executor_out: summary.last_output_tokens.unwrap_or(0) as u64,
                ..Default::default()
            }
        }
        Err(_) => ScopeCosts::default(),
    };

    let session_report = scope_report(&session_costs, &exec_rates, &baseline);

    // Project and milestone scopes require project_id.
    let project_id = cfg.project.id.as_deref();

    // Read telemetry.
    let runs: Vec<PhaseRun> =
        telemetry::read(&telemetry_file).map_err(|e| format!("failed to read telemetry: {e}"))?;
    let activities = telemetry::fold_activities(
        telemetry::read_architect_activities(&telemetry_file).unwrap_or_default(),
    );
    let ledgers = telemetry::fold_ledger(
        telemetry::read_architect_ledger(&telemetry_file).unwrap_or_default(),
    );

    if let Some(pid) = project_id {
        let project_costs = scope_costs(&runs, &ledgers, &cfg.architect, pid, None);
        let project_report = scope_report(&project_costs, &exec_rates, &baseline);

        // Find the latest milestone_id from project runs.
        let latest_milestone_id = runs
            .iter()
            .filter(|r| r.project_id.as_deref() == Some(pid))
            .filter(|r| r.milestone_id.is_some())
            .max_by_key(|r| r.ts)
            .and_then(|r| r.milestone_id.as_deref());

        let milestone_report = latest_milestone_id.map(|mid| {
            let costs = scope_costs(&runs, &ledgers, &cfg.architect, pid, Some(mid));
            scope_report(&costs, &exec_rates, &baseline)
        });

        // Assists: count folded activities with project_id and activity == "assist".
        let assists = activities
            .iter()
            .filter(|a| a.project_id.as_deref() == Some(pid) && a.activity == "assist")
            .count() as u32;

        Ok(CostReport {
            session: session_report,
            milestone: milestone_report,
            project: project_report,
            assists,
        })
    } else {
        // No project_id: session still computes; project/milestone are zero.
        let zero = ScopeCosts::default();
        let zero_report = scope_report(&zero, &exec_rates, &baseline);
        Ok(CostReport {
            session: session_report,
            milestone: None,
            project: zero_report,
            assists: 0,
        })
    }
}

/// Format the cost report as a human-readable table.
pub fn format_costs(report: &CostReport) -> String {
    let fmt_dollars = |v: f64| format!("${v:.2}");
    let fmt_opt = |v: Option<f64>| match v {
        Some(d) => fmt_dollars(d),
        None => "—".to_string(),
    };

    let header = format!(
        "{:<12}{:>10}{:>10}{:>10}{:>10}",
        "SCOPE", "BASELINE", "EXECUTOR", "ARCHITECT", "NET"
    );

    let row = |label: &str, r: &ScopeReport| {
        format!(
            "{:<12}{:>10}{:>10}{:>10}{:>10}",
            label,
            fmt_opt(r.baseline),
            fmt_dollars(r.executor),
            fmt_opt(r.architect),
            fmt_opt(r.net),
        )
    };

    let mut lines = vec![header, row("Session", &report.session)];

    if let Some(ref milestone) = report.milestone {
        lines.push(row("Milestone", milestone));
    }
    lines.push(row("Project", &report.project));
    lines.push(format!("Assists: {}", report.assists));

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_rates() -> BudgetRates {
        BudgetRates {
            input_per_mtok: 0.0,
            output_per_mtok: 0.0,
            executor: telemetry::ModelRates::default(),
        }
    }

    fn priced_exec_rates() -> telemetry::ModelRates {
        telemetry::ModelRates {
            input_per_mtok: 5.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 2.0,
            cache_creation_per_mtok: 8.0,
        }
    }

    fn priced_baseline() -> BudgetRates {
        BudgetRates {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            executor: telemetry::ModelRates::default(),
        }
    }

    #[test]
    fn scope_report_priced_executor_and_baseline() {
        let costs = ScopeCosts {
            executor_in: 1_000_000,
            executor_out: 1_000_000,
            executor_cache_read: 0,
            executor_cache_write: 0,
            architect: ArchitectTokens {
                input: 500_000,
                cache_creation: 100_000,
                cache_read: 200_000,
                output: 300_000,
            },
            architect_cost: Some(31.2),
        };
        let exec = priced_exec_rates();
        let baseline = priced_baseline();
        let r = scope_report(&costs, &exec, &baseline);

        // executor = 1M * 5.0 + 1M * 15.0 = $20.00
        assert_eq!(r.executor, 20.0);
        // architect passes through the pre-computed per-model cost.
        assert_eq!(r.architect, Some(31.2));
        // baseline = 1M*15 + 1M*75 = $90.00
        assert_eq!(r.baseline, Some(90.0));
        // net = 90 - 20 - 31.2
        assert_eq!(r.net, Some(90.0 - 20.0 - 31.2));
    }

    #[test]
    fn scope_report_unpriced_executor_is_zero_not_stub() {
        let costs = ScopeCosts {
            executor_in: 1_000_000,
            executor_out: 1_000_000,
            ..Default::default()
        };
        let zero_exec = telemetry::ModelRates::default();
        let baseline = BudgetRates {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            executor: telemetry::ModelRates::default(),
        };
        let r = scope_report(&costs, &zero_exec, &baseline);

        // Unpriced executor computes to 0.0 (not a literal "$0.00" stub).
        assert_eq!(r.executor, 0.0);
        // Baseline and net still compute normally.
        assert_eq!(r.baseline, Some(90.0));
        // architect is None (no ledger) => net is not attributable.
        assert_eq!(r.net, None);
    }

    #[test]
    fn scope_report_no_baseline_is_none() {
        let costs = ScopeCosts {
            executor_in: 1_000_000,
            executor_out: 1_000_000,
            ..Default::default()
        };
        let exec = priced_exec_rates();
        let zero = zero_rates();
        let r = scope_report(&costs, &exec, &zero);

        assert_eq!(r.baseline, None);
        assert_eq!(r.net, None);
        // Executor and architect still compute.
        assert_eq!(r.executor, 20.0);
        assert_eq!(r.architect, None);
    }

    #[test]
    fn format_costs_omits_milestone_when_none() {
        let report = CostReport {
            session: ScopeReport {
                baseline: None,
                executor: 5.0,
                architect: Some(0.0),
                net: None,
            },
            milestone: None,
            project: ScopeReport {
                baseline: Some(100.0),
                executor: 50.0,
                architect: Some(20.0),
                net: Some(30.0),
            },
            assists: 3,
        };
        let out = format_costs(&report);
        assert!(out.contains("Session"));
        assert!(out.contains("Project"));
        assert!(out.contains("—"));
        // Milestone data row should NOT appear.
        let lines: Vec<&str> = out.lines().collect();
        let data_lines: Vec<&str> = lines.iter().skip(1).copied().collect();
        for line in &data_lines {
            assert!(
                !line.starts_with("Milestone"),
                "Milestone data row should be omitted: {line}"
            );
        }
    }

    #[test]
    fn format_costs_shows_milestone_when_some() {
        let report = CostReport {
            session: ScopeReport {
                baseline: Some(10.0),
                executor: 5.0,
                architect: Some(0.0),
                net: Some(5.0),
            },
            milestone: Some(ScopeReport {
                baseline: Some(50.0),
                executor: 25.0,
                architect: Some(10.0),
                net: Some(15.0),
            }),
            project: ScopeReport {
                baseline: Some(100.0),
                executor: 50.0,
                architect: Some(20.0),
                net: Some(30.0),
            },
            assists: 3,
        };
        let out = format_costs(&report);
        assert!(out.contains("Session"));
        assert!(out.contains("Milestone"));
        assert!(out.contains("Project"));
        assert!(out.contains("Assists: 3"));
    }

    #[test]
    fn load_cost_report_telemetry_disabled_errors() {
        // Use a temp config file with telemetry.enabled = false.
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("rexymcp.toml");
        let _ = std::fs::write(
            &config_path,
            r#"
[executor]
model = "AEON-7"
provider = "ollama"
base_url = "http://localhost:1234/v1"

[commands]
format = "cargo fmt --all"
build = "cargo build"
lint = "cargo clippy"
test = "cargo test"

[telemetry]
enabled = false

[dashboard]
saved_input_per_mtok = 0.0
saved_output_per_mtok = 0.0
"#,
        );
        let err = load_cost_report(&config_path, tmp.path(), None, None).unwrap_err();
        assert!(
            err.contains("telemetry disabled"),
            "expected telemetry disabled error: {err}"
        );
    }

    #[test]
    fn scope_costs_none_sums_all_milestones() {
        use rexymcp_executor::ai::types::TokenBreakdown;
        use rexymcp_executor::store::telemetry::{Gates, GenerationParams, PhaseRun};
        let run = |proj: &str, mile: &str, inp: u32, outp: u32| PhaseRun {
            ts: 1,
            model: "m".into(),
            generation_params: GenerationParams::default(),
            phase_id: "p".into(),
            phase_doc_path: None,
            tags: vec![],
            status: "complete".into(),
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
            turns: 1,
            wall_clock_s: 1.0,
            tokens: TokenBreakdown {
                input_tokens: inp,
                output_tokens: outp,
                ..Default::default()
            },
            warnings: None,
            bugs_filed: None,
            bounces_to_approval: None,
            architect_verdict: None,
            served_model: None,
            length_finish_rate: None,
            context_window: None,
            context_efficiency: Default::default(),
            project_id: Some(proj.into()),
            milestone_id: Some(mile.into()),
            tier_telemetry: Default::default(),
            ..Default::default()
        };
        let runs = vec![
            run("P", "mA", 100, 10),
            run("P", "mB", 200, 20),
            run("OTHER", "mA", 999, 999), // different project — must be excluded
        ];
        // None = all milestones of project P: 100+200 input, 10+20 output.
        let all = scope_costs(
            &runs,
            &[],
            &rexymcp_executor::config::ArchitectConfig::default(),
            "P",
            None,
        );
        assert_eq!(all.executor_in, 300);
        assert_eq!(all.executor_out, 30);
        // Some("mA") = only that milestone.
        let just_a = scope_costs(
            &runs,
            &[],
            &rexymcp_executor::config::ArchitectConfig::default(),
            "P",
            Some("mA"),
        );
        assert_eq!(just_a.executor_in, 100);
        // Superset: project (None) >= milestone (Some).
        assert!(all.executor_in >= just_a.executor_in);
    }

    #[test]
    fn scope_report_includes_executor_cache() {
        let costs = ScopeCosts {
            executor_in: 1_000_000,
            executor_out: 500_000,
            executor_cache_read: 200_000,
            executor_cache_write: 100_000,
            architect: Default::default(),
            architect_cost: None,
        };
        let exec_rates = priced_exec_rates();
        let baseline = BudgetRates {
            input_per_mtok: 0.0,
            output_per_mtok: 0.0,
            executor: telemetry::ModelRates::default(),
        };
        let r = scope_report(&costs, &exec_rates, &baseline);

        // executor = 1M*5 + 0.5M*15 + 0.2M*2 + 0.1M*8 = 5 + 7.5 + 0.4 + 0.8 = $13.70
        assert!((r.executor - 13.7).abs() < 1e-6);
        // Baseline is None (no baseline rate configured).
        assert_eq!(r.baseline, None);
        assert_eq!(r.net, None);
    }

    #[test]
    fn scope_costs_sums_cache_buckets() {
        use rexymcp_executor::ai::types::TokenBreakdown;
        use rexymcp_executor::store::telemetry::{Gates, GenerationParams, PhaseRun};
        let run = |proj: &str, inp: u32, outp: u32, cache_read: u32, cache_write: u32| PhaseRun {
            ts: 1,
            model: "m".into(),
            generation_params: GenerationParams::default(),
            phase_id: "p".into(),
            phase_doc_path: None,
            tags: vec![],
            status: "complete".into(),
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
            turns: 1,
            wall_clock_s: 1.0,
            tokens: TokenBreakdown {
                input_tokens: inp,
                output_tokens: outp,
                cache_read_tokens: cache_read,
                cache_write_tokens: cache_write,
            },
            warnings: None,
            bugs_filed: None,
            bounces_to_approval: None,
            architect_verdict: None,
            served_model: None,
            length_finish_rate: None,
            context_window: None,
            context_efficiency: Default::default(),
            project_id: Some(proj.into()),
            milestone_id: None,
            tier_telemetry: Default::default(),
            ..Default::default()
        };
        let runs = vec![run("P", 100, 10, 50, 30), run("P", 200, 20, 100, 70)];
        let all = scope_costs(
            &runs,
            &[],
            &rexymcp_executor::config::ArchitectConfig::default(),
            "P",
            None,
        );
        assert_eq!(all.executor_in, 300);
        assert_eq!(all.executor_out, 30);
        assert_eq!(all.executor_cache_read, 150);
        assert_eq!(all.executor_cache_write, 100);
    }

    fn ledger(model: &str) -> telemetry::ArchitectLedger {
        telemetry::ArchitectLedger {
            record: telemetry::ARCHITECT_LEDGER_RECORD_TAG.to_string(),
            project_id: Some("P".to_string()),
            session_id: "s".to_string(),
            model: model.to_string(),
            skill: "dispatch".to_string(),
            tokens: ArchitectTokens {
                input: 1_000_000,
                cache_creation: 0,
                cache_read: 0,
                output: 1_000_000,
            },
            cache_creation_5m: 0,
            cache_creation_1h: 0,
            messages: 1,
            last_ts: 1,
        }
    }

    #[test]
    fn scope_costs_prices_architect_per_model_from_ledger() {
        // Two ledger records with DIFFERENT models must each be priced at their
        // own rate: opus 1M in + 1M out = $5 + $25 = $30; sonnet-5 = $2 + $10 = $12.
        let ledgers = vec![ledger("claude-opus-4-8"), ledger("claude-sonnet-5")];
        let cfg = rexymcp_executor::config::ArchitectConfig::default();
        let c = scope_costs(&[], &ledgers, &cfg, "P", None);
        let expected = 30.0 + 12.0;
        assert!(
            (c.architect_cost.unwrap() - expected).abs() < 1e-9,
            "per-model architect cost should be $42.00, got {:?}",
            c.architect_cost
        );
    }

    #[test]
    fn scope_costs_milestone_architect_is_none() {
        // Architect cost is not attributable at milestone scope (ledger has no milestone).
        let cfg = rexymcp_executor::config::ArchitectConfig::default();
        let c = scope_costs(&[], &[ledger("claude-opus-4-8")], &cfg, "P", Some("M35"));
        assert_eq!(c.architect_cost, None);
    }
}
