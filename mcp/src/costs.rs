//! Cost-report core — `rexymcp costs` CLI.
//!
//! Computes Saved / Executor / Architect / Net across Session / Milestone /
//! Project scopes. Executor cost is derived from `cfg.model_rates` (phase-03
//! pricing), not hardcoded `$0.00`.

use std::path::Path;

use rexymcp_executor::config::Config;
use rexymcp_executor::store::telemetry::{self, ArchitectTokens, PhaseRun};

use crate::dashboard::{BudgetRates, ScopeCosts};
use crate::status;

/// One scope's four cost lines, in dollars. `saved`/`net` are `None` when no
/// saved rate is configured (rendered `—`); `executor`/`architect` are always
/// present (`0.0` when unpriced).
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct ScopeReport {
    pub saved: Option<f64>,
    pub executor: f64,
    pub architect: Option<f64>,
    pub net: Option<f64>,
    /// Executor tokens for this scope, all four classes summed. Rendered in
    /// tokens mode; `0` when the scope has no runs.
    pub executor_tokens: u64,
    /// Architect tokens for this scope, all four classes summed.
    pub architect_tokens: u64,
}

/// Saved/Executor/Architect/Net across the three scopes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CostReport {
    pub session: ScopeReport,
    /// `None` when no active milestone could be resolved (no project runs).
    pub milestone: Option<ScopeReport>,
    pub project: ScopeReport,
    pub assists: u32,
    pub by_skill: Vec<SkillCost>,
}

/// Compute one scope's dollar lines. `exec_rates` are the executor model's
/// `$/Mtok` (from `cfg.model_rates`); `saved_rates` carries the cloud-baseline +
/// architect rates. u64-safe (does NOT route token totals through the u32
/// `TokenBreakdown`).
pub fn scope_report(
    costs: &ScopeCosts,
    exec_rates: &telemetry::ModelRates,
    saved_rates: &BudgetRates,
) -> ScopeReport {
    let per_m = |t: u64, r: f64| (t as f64 / 1_000_000.0) * r;
    let no_saved_rates = saved_rates.input_per_mtok == 0.0 && saved_rates.output_per_mtok == 0.0;

    let executor = per_m(costs.executor_in, exec_rates.input_per_mtok)
        + per_m(costs.executor_out, exec_rates.output_per_mtok)
        + per_m(costs.executor_cache_read, exec_rates.cache_read_per_mtok)
        + per_m(
            costs.executor_cache_write,
            exec_rates.cache_creation_per_mtok,
        );
    let architect = costs.architect_cost;
    let saved_cost = if no_saved_rates {
        None
    } else {
        Some(
            per_m(costs.executor_in, saved_rates.input_per_mtok)
                + per_m(costs.executor_out, saved_rates.output_per_mtok),
        )
    };
    let net = match (saved_cost, architect) {
        (Some(s), Some(a)) => Some(s - executor - a),
        _ => None,
    };

    let executor_tokens = costs
        .executor_in
        .saturating_add(costs.executor_out)
        .saturating_add(costs.executor_cache_read)
        .saturating_add(costs.executor_cache_write);
    let architect_tokens = costs
        .architect
        .input
        .saturating_add(costs.architect.output)
        .saturating_add(costs.architect.cache_creation)
        .saturating_add(costs.architect.cache_read);

    ScopeReport {
        saved: saved_cost,
        executor,
        architect,
        net,
        executor_tokens,
        architect_tokens,
    }
}

/// One skill's architect spend: total tokens (all four classes) and per-model USD cost.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SkillCost {
    pub skill: String,
    pub tokens: u64,
    pub cost: f64,
}

/// Display name for a stored architect-ledger skill key.
///
/// The harvester buckets messages with no `attributionSkill` under the stable
/// storage key `other`. That is untagged architect work — non-skill sessions and
/// the user↔architect conversation between phase runs — so it renders as
/// `architect chat`. Mapping here rather than at write time keeps already-
/// harvested records valid and cannot split one bucket across two rows.
pub(crate) fn display_skill(skill: &str) -> &str {
    match skill {
        "other" => "architect chat",
        s => s,
    }
}

/// Per-skill architect cost for a project, from the ledger, priced per-model.
/// Sorted by `cost` descending (ties broken by `skill` for determinism).
pub(crate) fn skill_costs(
    ledgers: &[telemetry::ArchitectLedger],
    architect: &rexymcp_executor::config::ArchitectConfig,
    project_id: &str,
) -> Vec<SkillCost> {
    use std::collections::HashMap;
    let mut acc: HashMap<String, (u64, f64)> = HashMap::new();
    for l in ledgers
        .iter()
        .filter(|l| l.project_id.as_deref() == Some(project_id))
    {
        let toks = l
            .tokens
            .input
            .saturating_add(l.tokens.cache_creation)
            .saturating_add(l.tokens.cache_read)
            .saturating_add(l.tokens.output);
        let cost = architect
            .rates_for(&l.model)
            .map_or(0.0, |(i, o)| l.cost(i, o));
        let key = display_skill(&l.skill).to_string();
        let e = acc.entry(key).or_insert((0, 0.0));
        e.0 = e.0.saturating_add(toks);
        e.1 += cost;
    }
    let mut out: Vec<SkillCost> = acc
        .into_iter()
        .map(|(skill, (tokens, cost))| SkillCost {
            skill,
            tokens,
            cost,
        })
        .collect();
    out.sort_by(|a, b| {
        b.cost
            .total_cmp(&a.cost)
            .then_with(|| a.skill.cmp(&b.skill))
    });
    out
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

    let (discount_in, discount_out) = cfg.architect.effective_rates();
    let saved_rates = BudgetRates {
        input_per_mtok: discount_in,
        output_per_mtok: discount_out,
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

    let session_report = scope_report(&session_costs, &exec_rates, &saved_rates);

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
        let project_report = scope_report(&project_costs, &exec_rates, &saved_rates);

        // Find the latest milestone_id from project runs.
        let latest_milestone_id = runs
            .iter()
            .filter(|r| r.project_id.as_deref() == Some(pid))
            .filter(|r| r.milestone_id.is_some())
            .max_by_key(|r| r.ts)
            .and_then(|r| r.milestone_id.as_deref());

        let milestone_report = latest_milestone_id.map(|mid| {
            let costs = scope_costs(&runs, &ledgers, &cfg.architect, pid, Some(mid));
            scope_report(&costs, &exec_rates, &saved_rates)
        });

        // Assists: count folded activities with project_id and activity == "assist".
        let assists = activities
            .iter()
            .filter(|a| a.project_id.as_deref() == Some(pid) && a.activity == "assist")
            .count() as u32;

        let by_skill = skill_costs(&ledgers, &cfg.architect, pid);
        Ok(CostReport {
            session: session_report,
            milestone: milestone_report,
            project: project_report,
            assists,
            by_skill,
        })
    } else {
        // No project_id: session still computes; project/milestone are zero.
        let zero = ScopeCosts::default();
        let zero_report = scope_report(&zero, &exec_rates, &saved_rates);
        Ok(CostReport {
            session: session_report,
            milestone: None,
            project: zero_report,
            assists: 0,
            by_skill: Vec::new(),
        })
    }
}

/// Format the cost report as a human-readable table, optionally in token mode.
pub fn format_costs_with(report: &CostReport, units: LedgerUnits) -> String {
    let mut lines = ledger_lines(
        &report.session,
        report.milestone.as_ref(),
        &report.project,
        units,
    );

    lines.push(format!("Assists: {}", report.assists));

    // Legend — only in dollars mode.
    if units == LedgerUnits::Dollars {
        lines.push(String::new());
        lines.push("Executor = Claude cost avoided at [architect] rates; ( ) = debit.".to_string());
    }

    // Per-skill architect cost table (project-scoped).
    if !report.by_skill.is_empty() {
        let total: f64 = report.by_skill.iter().map(|s| s.cost).sum();
        lines.push(String::new());
        lines.push("By skill (architect)".to_string());
        lines.push(format!(
            "{:<20}{:>10}{:>10}{:>8}",
            "SKILL", "TOKENS", "COST", "%"
        ));
        for s in &report.by_skill {
            let pct = if total > 0.0 {
                s.cost / total * 100.0
            } else {
                0.0
            };
            let tokens_str = format_tokens(s.tokens);
            lines.push(format!(
                "{:<20}{:>10}{:>10}{:>7.1}%",
                s.skill,
                tokens_str,
                format!("${:.2}", s.cost),
                pct,
            ));
        }
    }

    lines.join("\n")
}

/// Format a token count for display: "—", raw, "{:.1}k", or "{:.1}M".
pub(crate) fn format_tokens(count: u64) -> String {
    if count == 0 {
        "—".to_string()
    } else if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

/// Which units the Budget ledger renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LedgerUnits {
    #[default]
    Dollars,
    Tokens,
}

/// Wrap a dollar value in parens for debit rendering. Tight `(—)` when no value.
fn paren(v: String) -> String {
    if v == "—" {
        "(—)  ".to_string()
    } else {
        format!("({v})")
    }
}

/// Build a row string with the right column widths for the given scope count.
fn make_row(label: &str, v1: String, v2: String, v3: String, has_milestone: bool) -> String {
    if has_milestone {
        format!("  {:<10}{:>10}{:>10}{:>10}", label, v1, v2, v3)
    } else {
        format!("  {:<10}{:>9}{:>9}", label, v1, v3)
    }
}

/// The Budget ledger: a header plus Architect / Executor / Net rows across the
/// available scopes. Debits are parenthesised, credits plain — the parens carry
/// the sign, so no separate "saved" row is needed.
///
/// Dollars mode:  Architect = debit (Claude spend); Executor = credit (Claude
/// cost avoided, minus local cost when the executor is priced); Net = the two
/// summed, parenthesised when negative.
/// Tokens mode:   both rows are token counts; Net is `—`.
///
/// Returns an empty Vec when there is nothing to render — never a lone header.
pub fn ledger_lines(
    session: &ScopeReport,
    milestone: Option<&ScopeReport>,
    project: &ScopeReport,
    units: LedgerUnits,
) -> Vec<String> {
    let has_milestone = milestone.is_some();
    let mile_default = ScopeReport::default();
    let mile = milestone.unwrap_or(&mile_default);

    let header = if has_milestone {
        match units {
            LedgerUnits::Tokens => format!(
                "{:<12}{:>10}{:>10}{:>10}",
                "Spend (tok)", "Session", "Milestone", "Project"
            ),
            LedgerUnits::Dollars => format!(
                "{:<12}{:>10}{:>10}{:>10}",
                "Spend", "Session", "Milestone", "Project"
            ),
        }
    } else {
        match units {
            LedgerUnits::Tokens => format!("{:<12}{:>9}{:>9}", "Spend (tok)", "Session", "Project"),
            LedgerUnits::Dollars => {
                format!("{:<12}{:>9}{:>9}", "Spend", "Session", "Project")
            }
        }
    };

    let mut out = Vec::new();

    match units {
        LedgerUnits::Tokens => {
            out.push(header);
            out.push(make_row(
                "Architect:",
                format_tokens(session.architect_tokens),
                format_tokens(mile.architect_tokens),
                format_tokens(project.architect_tokens),
                has_milestone,
            ));
            out.push(make_row(
                "Executor:",
                format_tokens(session.executor_tokens),
                format_tokens(mile.executor_tokens),
                format_tokens(project.executor_tokens),
                has_milestone,
            ));
            out.push(make_row(
                "Net:",
                "—".to_string(),
                "—".to_string(),
                "—".to_string(),
                has_milestone,
            ));
        }
        LedgerUnits::Dollars => {
            let fmt_dollars = |v: f64| format!("${v:.2}");
            let fmt_opt =
                |v: Option<f64>| -> String { v.map_or("—".to_string(), |x| format!("${x:.2}")) };

            out.push(header);

            // Architect: debit → parenthesised
            out.push(make_row(
                "Architect:",
                paren(fmt_opt(session.architect)),
                paren(fmt_opt(mile.architect)),
                paren(fmt_opt(project.architect)),
                has_milestone,
            ));

            // Executor: credit = saved - executor, plain; parenthesised if negative
            let executor_val = |r: &ScopeReport| -> String {
                let saved = r.saved.unwrap_or(0.0);
                let val = saved - r.executor;
                if val < 0.0 {
                    format!("({:.2})", val.abs())
                } else {
                    fmt_dollars(val)
                }
            };
            out.push(make_row(
                "Executor:",
                executor_val(session),
                executor_val(mile),
                executor_val(project),
                has_milestone,
            ));

            // Net: sum of the two rendered rows; parenthesised when negative
            let net_val = |r: &ScopeReport| -> String {
                let net = r.net.unwrap_or(0.0);
                if net < 0.0 {
                    format!("(${:.2})", net.abs())
                } else {
                    fmt_dollars(net)
                }
            };
            out.push(make_row(
                "Net:",
                net_val(session),
                net_val(mile),
                net_val(project),
                has_milestone,
            ));
        }
    }

    out
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

    fn priced_saved_rates() -> BudgetRates {
        BudgetRates {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            executor: telemetry::ModelRates::default(),
        }
    }

    #[test]
    fn scope_report_priced_executor_and_saved() {
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
        let saved_rates = priced_saved_rates();
        let r = scope_report(&costs, &exec, &saved_rates);

        // executor = 1M * 5.0 + 1M * 15.0 = $20.00
        assert_eq!(r.executor, 20.0);
        // architect passes through the pre-computed per-model cost.
        assert_eq!(r.architect, Some(31.2));
        // saved = 1M*15 + 1M*75 = $90.00
        assert_eq!(r.saved, Some(90.0));
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
        let saved_rates = BudgetRates {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            executor: telemetry::ModelRates::default(),
        };
        let r = scope_report(&costs, &zero_exec, &saved_rates);

        // Unpriced executor computes to 0.0 (not a literal "$0.00" stub).
        assert_eq!(r.executor, 0.0);
        // Saved and net still compute normally.
        assert_eq!(r.saved, Some(90.0));
        // architect is None (no ledger) => net is not attributable.
        assert_eq!(r.net, None);
    }

    #[test]
    fn scope_report_no_saved_rate_is_none() {
        let costs = ScopeCosts {
            executor_in: 1_000_000,
            executor_out: 1_000_000,
            ..Default::default()
        };
        let exec = priced_exec_rates();
        let zero = zero_rates();
        let r = scope_report(&costs, &exec, &zero);

        assert_eq!(r.saved, None);
        assert_eq!(r.net, None);
        // Executor and architect still compute.
        assert_eq!(r.executor, 20.0);
        assert_eq!(r.architect, None);
    }

    #[test]
    fn format_costs_omits_milestone_when_none() {
        let report = CostReport {
            session: ScopeReport {
                saved: None,
                executor: 5.0,
                architect: Some(0.0),
                net: None,
                ..Default::default()
            },
            milestone: None,
            project: ScopeReport {
                saved: Some(100.0),
                executor: 50.0,
                architect: Some(20.0),
                net: Some(30.0),
                ..Default::default()
            },
            assists: 3,
            by_skill: Vec::new(),
        };
        let out = format_costs_with(&report, LedgerUnits::Dollars);
        assert!(out.contains("Session"));
        assert!(out.contains("Project"));
        // Session architect is $0.00 (not —) because it's Some(0.0)
        // Session net is None so it renders as —
        assert!(out.contains("Architect:"));
        assert!(out.contains("Executor:"));
        assert!(out.contains("Net:"));
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
                saved: Some(10.0),
                executor: 5.0,
                architect: Some(0.0),
                net: Some(5.0),
                ..Default::default()
            },
            milestone: Some(ScopeReport {
                saved: Some(50.0),
                executor: 25.0,
                architect: Some(10.0),
                net: Some(15.0),
                ..Default::default()
            }),
            project: ScopeReport {
                saved: Some(100.0),
                executor: 50.0,
                architect: Some(20.0),
                net: Some(30.0),
                ..Default::default()
            },
            assists: 3,
            by_skill: Vec::new(),
        };
        let out = format_costs_with(&report, LedgerUnits::Dollars);
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
        let saved_rates = BudgetRates {
            input_per_mtok: 0.0,
            output_per_mtok: 0.0,
            executor: telemetry::ModelRates::default(),
        };
        let r = scope_report(&costs, &exec_rates, &saved_rates);

        // executor = 1M*5 + 0.5M*15 + 0.2M*2 + 0.1M*8 = 5 + 7.5 + 0.4 + 0.8 = $13.70
        assert!((r.executor - 13.7).abs() < 1e-6);
        // Saved is None (no saved rate configured).
        assert_eq!(r.saved, None);
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

    #[test]
    fn skill_costs_groups_and_prices_per_model() {
        // Two dispatch records (opus + sonnet-5) and one review record (opus).
        // Dispatch: opus ($30) + sonnet-5 ($12) = $42; review: opus ($30).
        let mut ledgers = vec![
            ledger("claude-opus-4-8"), // dispatch: $30
            ledger("claude-sonnet-5"), // dispatch: $12
        ];
        let mut review = ledger("claude-opus-4-8");
        review.skill = "review".to_string();
        ledgers.push(review); // review: $30

        let cfg = rexymcp_executor::config::ArchitectConfig::default();
        let costs = skill_costs(&ledgers, &cfg, "P");

        assert_eq!(costs.len(), 2);
        assert_eq!(costs[0].skill, "dispatch");
        assert!(
            (costs[0].cost - 42.0).abs() < 1e-9,
            "dispatch cost: {}",
            costs[0].cost
        );
        assert_eq!(costs[0].tokens, 4_000_000); // 2 records × 2M tokens
        assert_eq!(costs[1].skill, "review");
        assert!(
            (costs[1].cost - 30.0).abs() < 1e-9,
            "review cost: {}",
            costs[1].cost
        );
        assert_eq!(costs[1].tokens, 2_000_000);
    }

    #[test]
    fn skill_costs_sorted_by_cost_desc() {
        // "zeta" has higher cost than "alpha" so it should sort first.
        let mut ledgers = vec![];
        let mut alpha = ledger("claude-sonnet-5");
        alpha.skill = "alpha".to_string();
        ledgers.push(alpha); // $12
        let mut zeta = ledger("claude-opus-4-8");
        zeta.skill = "zeta".to_string();
        ledgers.push(zeta); // $30

        let cfg = rexymcp_executor::config::ArchitectConfig::default();
        let costs = skill_costs(&ledgers, &cfg, "P");

        assert_eq!(costs.len(), 2);
        assert_eq!(costs[0].skill, "zeta"); // higher cost first
        assert_eq!(costs[1].skill, "alpha");
    }

    #[test]
    fn skill_costs_empty_is_empty() {
        let cfg = rexymcp_executor::config::ArchitectConfig::default();
        let costs = skill_costs(&[], &cfg, "P");
        assert!(costs.is_empty());
    }

    #[test]
    fn display_skill_maps_other_to_architect_chat() {
        assert_eq!(display_skill("other"), "architect chat");
    }

    #[test]
    fn display_skill_passes_through_named_skills() {
        assert_eq!(display_skill("rexymcp:dispatch"), "rexymcp:dispatch");
        assert_eq!(display_skill("rexymcp:auto"), "rexymcp:auto");
    }

    #[test]
    fn skill_costs_renders_other_as_architect_chat() {
        let mut ledgers = vec![ledger("claude-opus-4-8")]; // dispatch: $30
        let mut other = ledger("claude-sonnet-5");
        other.skill = "other".to_string();
        ledgers.push(other); // other: $12

        let cfg = rexymcp_executor::config::ArchitectConfig::default();
        let costs = skill_costs(&ledgers, &cfg, "P");

        assert_eq!(costs.len(), 2);
        let skills: Vec<&str> = costs.iter().map(|c| c.skill.as_str()).collect();
        assert!(skills.contains(&"dispatch"));
        assert!(skills.contains(&"architect chat"));
        assert!(!skills.contains(&"other"));
    }

    #[test]
    fn skill_costs_folds_other_and_architect_chat_into_one_row() {
        let mut ledgers = vec![ledger("claude-opus-4-8")]; // dispatch $30, 2M tokens
        let mut other = ledger("claude-sonnet-5");
        other.skill = "other".to_string();
        ledgers.push(other); // other $12, 2M tokens
        let mut already_renamed = ledger("claude-sonnet-5");
        already_renamed.skill = "architect chat".to_string();
        ledgers.push(already_renamed); // architect chat $12, 2M tokens

        let cfg = rexymcp_executor::config::ArchitectConfig::default();
        let costs = skill_costs(&ledgers, &cfg, "P");

        assert_eq!(costs.len(), 2);
        let chat = costs.iter().find(|c| c.skill == "architect chat").unwrap();
        assert_eq!(chat.tokens, 4_000_000); // 2 records × 2M tokens
        assert!(
            (chat.cost - 24.0).abs() < 1e-9,
            "architect chat cost: {}",
            chat.cost
        );
    }

    #[test]
    fn format_costs_appends_by_skill_percent() {
        let report = CostReport {
            session: ScopeReport {
                saved: None,
                executor: 0.0,
                architect: None,
                net: None,
                ..Default::default()
            },
            milestone: None,
            project: ScopeReport {
                saved: None,
                executor: 0.0,
                architect: None,
                net: None,
                ..Default::default()
            },
            assists: 0,
            by_skill: vec![
                SkillCost {
                    skill: "dispatch".to_string(),
                    tokens: 0,
                    cost: 30.0,
                },
                SkillCost {
                    skill: "review".to_string(),
                    tokens: 0,
                    cost: 10.0,
                },
            ],
        };

        let output = format_costs_with(&report, LedgerUnits::Dollars);
        assert!(output.contains("dispatch"));
        assert!(output.contains("review"));
        assert!(output.contains("$30.00"));
        assert!(output.contains("$10.00"));
        assert!(output.contains("75.0%"));
        assert!(output.contains("25.0%"));
    }

    #[test]
    fn format_costs_omits_by_skill_when_empty() {
        let report = CostReport {
            session: ScopeReport {
                saved: None,
                executor: 0.0,
                architect: None,
                net: None,
                ..Default::default()
            },
            milestone: None,
            project: ScopeReport {
                saved: None,
                executor: 0.0,
                architect: None,
                net: None,
                ..Default::default()
            },
            assists: 0,
            by_skill: Vec::new(),
        };

        let output = format_costs_with(&report, LedgerUnits::Dollars);
        assert!(!output.contains("By skill"));
        assert!(!output.contains("SKILL"));
    }

    #[test]
    fn format_costs_by_skill_percent_zero_when_total_zero() {
        let report = CostReport {
            session: ScopeReport {
                saved: None,
                executor: 0.0,
                architect: None,
                net: None,
                ..Default::default()
            },
            milestone: None,
            project: ScopeReport {
                saved: None,
                executor: 0.0,
                architect: None,
                net: None,
                ..Default::default()
            },
            assists: 0,
            by_skill: vec![SkillCost {
                skill: "dispatch".to_string(),
                tokens: 0,
                cost: 0.0,
            }],
        };

        let output = format_costs_with(&report, LedgerUnits::Dollars);
        assert!(
            output.contains("0.0%"),
            "zero total should show 0.0%: {output}"
        );
    }

    #[test]
    fn format_costs_header_has_no_baseline_column() {
        let report = CostReport {
            session: ScopeReport::default(),
            milestone: None,
            project: ScopeReport::default(),
            assists: 0,
            by_skill: Vec::new(),
        };
        let output = format_costs_with(&report, LedgerUnits::Dollars);
        let header = output.lines().next().expect("header line present");
        let expected = format!("{:<12}{:>9}{:>9}", "Spend", "Session", "Project");
        assert_eq!(header, expected, "header mismatch: {header}");
    }

    #[test]
    fn format_costs_legend_present_when_saved_priced() {
        let report = CostReport {
            session: ScopeReport {
                saved: Some(10.0),
                executor: 5.0,
                architect: Some(0.0),
                net: Some(5.0),
                ..Default::default()
            },
            milestone: None,
            project: ScopeReport {
                saved: Some(50.0),
                executor: 25.0,
                architect: Some(10.0),
                net: Some(15.0),
                ..Default::default()
            },
            assists: 0,
            by_skill: Vec::new(),
        };
        let output = format_costs_with(&report, LedgerUnits::Dollars);
        assert!(
            output.contains("Executor = Claude cost avoided at [architect] rates"),
            "new legend line missing: {output}"
        );
    }

    #[test]
    fn format_costs_legend_absent_in_tokens_mode() {
        let report = CostReport {
            session: ScopeReport {
                saved: None,
                executor: 5.0,
                architect: None,
                net: None,
                ..Default::default()
            },
            milestone: None,
            project: ScopeReport {
                saved: None,
                executor: 25.0,
                architect: None,
                net: None,
                ..Default::default()
            },
            assists: 0,
            by_skill: Vec::new(),
        };
        let output = format_costs_with(&report, LedgerUnits::Tokens);
        assert!(
            !output.contains("Executor = Claude cost avoided"),
            "legend should be absent in tokens mode: {output}"
        );
    }

    #[test]
    fn discount_rate_comes_from_architect_config() {
        use rexymcp_executor::ai::types::TokenBreakdown;
        use rexymcp_executor::store::telemetry::{self, Gates, GenerationParams, PhaseRun};

        let tmp = tempfile::tempdir().unwrap();
        let tel_dir = tmp.path().join("telemetry");
        std::fs::create_dir_all(&tel_dir).unwrap();
        let config_path = tmp.path().join("rexymcp.toml");
        std::fs::write(
            &config_path,
            format!(
                r#"
[project]
id = "PID"

[executor]
model = "local-unpriced"
provider = "ollama"
base_url = "http://localhost:1234/v1"

[commands]
format = "cargo fmt --all"
build = "cargo build"
lint = "cargo clippy"
test = "cargo test"

[telemetry]
dir = "{}"

[architect]
model = "claude-fable-5"
"#,
                tel_dir.display()
            ),
        )
        .unwrap();

        // One run: 1M input, 1M output, attributed to project PID.
        let run = PhaseRun {
            ts: 1,
            model: "local-unpriced".into(),
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
                input_tokens: 1_000_000,
                output_tokens: 1_000_000,
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
            project_id: Some("PID".into()),
            milestone_id: None,
            tier_telemetry: Default::default(),
            ..Default::default()
        };
        telemetry::append(&tel_dir, &run).unwrap();

        let report = load_cost_report(&config_path, tmp.path(), None, None).unwrap();

        // claude-fable-5 = $10/Mtok in, $50/Mtok out.
        // 1M * 10 + 1M * 50 = $60.00 — NOT the $30.00 opus-4-8 would give.
        let saved = report.project.saved.expect("project saved must be priced");
        assert!(
            (saved - 60.0).abs() < 1e-9,
            "discount must use [architect] rates (fable-5 => $60.00), got {saved}"
        );
    }

    // --- Ledger tests ---

    #[test]
    fn ledger_row_order_is_architect_executor_net() {
        let sess = ScopeReport {
            saved: Some(10.0),
            executor: 2.0,
            architect: Some(5.0),
            net: Some(3.0),
            ..Default::default()
        };
        let proj = ScopeReport {
            saved: Some(100.0),
            executor: 10.0,
            architect: Some(50.0),
            net: Some(40.0),
            ..Default::default()
        };
        let lines = ledger_lines(&sess, None, &proj, LedgerUnits::Dollars);
        let labels: Vec<&str> = lines[1..]
            .iter()
            .map(|l| {
                let end = l.find(':').unwrap_or(l.len());
                l[..end].trim()
            })
            .collect();
        assert_eq!(labels, vec!["Architect", "Executor", "Net"]);
    }

    #[test]
    fn ledger_executor_row_is_saved_minus_executor_cost() {
        let sess = ScopeReport {
            saved: Some(100.0),
            executor: 25.0,
            architect: Some(50.0),
            net: Some(25.0),
            ..Default::default()
        };
        let proj = ScopeReport {
            saved: Some(200.0),
            executor: 50.0,
            architect: Some(100.0),
            net: Some(50.0),
            ..Default::default()
        };
        let lines = ledger_lines(&sess, None, &proj, LedgerUnits::Dollars);
        let executor_line = lines
            .iter()
            .find(|l| l.contains("Executor:"))
            .expect("Executor row present");
        // Executor = saved - executor = 100 - 25 = 75 for session
        assert!(
            executor_line.contains("$75.00"),
            "Executor row should show saved-executor: {executor_line}"
        );
    }

    #[test]
    fn ledger_net_equals_sum_of_rendered_rows() {
        let sess = ScopeReport {
            saved: Some(100.0),
            executor: 25.0,
            architect: Some(50.0),
            net: Some(25.0),
            ..Default::default()
        };
        let proj = ScopeReport {
            saved: Some(200.0),
            executor: 50.0,
            architect: Some(100.0),
            net: Some(50.0),
            ..Default::default()
        };
        let lines = ledger_lines(&sess, None, &proj, LedgerUnits::Dollars);
        // Net = (saved - executor) + (-architect) = 75 + (-50) = 25
        let net_line = lines
            .iter()
            .find(|l| l.contains("Net:"))
            .expect("Net row present");
        assert!(
            net_line.contains("$25.00"),
            "Net should equal executor_row + architect_row: {net_line}"
        );
        // Also equals ScopeReport.net
        assert_eq!(sess.net, Some(25.0));
    }

    #[test]
    fn ledger_negative_net_is_parenthesised() {
        let sess = ScopeReport {
            saved: Some(10.0),
            executor: 5.0,
            architect: Some(100.0),
            net: Some(-95.0),
            ..Default::default()
        };
        let proj = ScopeReport {
            saved: Some(20.0),
            executor: 10.0,
            architect: Some(200.0),
            net: Some(-190.0),
            ..Default::default()
        };
        let lines = ledger_lines(&sess, None, &proj, LedgerUnits::Dollars);
        let net_line = lines
            .iter()
            .find(|l| l.contains("Net:"))
            .expect("Net row present");
        assert!(
            net_line.contains("($95.00)"),
            "Negative net must be parenthesised: {net_line}"
        );
        assert!(
            !net_line.contains("$-95.00"),
            "Negative net must NOT use minus sign: {net_line}"
        );
    }

    #[test]
    fn ledger_positive_net_is_not_parenthesised() {
        let sess = ScopeReport {
            saved: Some(100.0),
            executor: 10.0,
            architect: Some(20.0),
            net: Some(70.0),
            ..Default::default()
        };
        let proj = ScopeReport {
            saved: Some(200.0),
            executor: 20.0,
            architect: Some(40.0),
            net: Some(140.0),
            ..Default::default()
        };
        let lines = ledger_lines(&sess, None, &proj, LedgerUnits::Dollars);
        let net_line = lines
            .iter()
            .find(|l| l.contains("Net:"))
            .expect("Net row present");
        assert!(
            !net_line.contains("($"),
            "Positive net must NOT be parenthesised: {net_line}"
        );
        assert!(
            net_line.contains("$70.00"),
            "Positive net must show dollar value: {net_line}"
        );
    }

    #[test]
    fn ledger_executor_row_renders_when_cost_is_zero() {
        let sess = ScopeReport {
            saved: None,
            executor: 0.0,
            architect: None,
            net: None,
            ..Default::default()
        };
        let proj = ScopeReport {
            saved: None,
            executor: 0.0,
            architect: None,
            net: None,
            ..Default::default()
        };
        let lines = ledger_lines(&sess, None, &proj, LedgerUnits::Dollars);
        let executor_line = lines
            .iter()
            .find(|l| l.contains("Executor:"))
            .expect("Executor row must always render even with zero cost");
        assert!(
            executor_line.contains("$0.00"),
            "Executor row with zero cost: {executor_line}"
        );
    }

    #[test]
    fn ledger_tokens_mode_shows_counts_and_dash_net() {
        let sess = ScopeReport {
            saved: None,
            executor: 0.0,
            architect: None,
            net: None,
            executor_tokens: 500_000,
            architect_tokens: 1_200_000,
        };
        let proj = ScopeReport {
            saved: None,
            executor: 0.0,
            architect: None,
            net: None,
            executor_tokens: 2_000_000,
            architect_tokens: 5_500_000,
        };
        let lines = ledger_lines(&sess, None, &proj, LedgerUnits::Tokens);
        let architect_line = lines
            .iter()
            .find(|l| l.contains("Architect:"))
            .expect("Architect row present");
        let executor_line = lines
            .iter()
            .find(|l| l.contains("Executor:"))
            .expect("Executor row present");
        let net_line = lines
            .iter()
            .find(|l| l.contains("Net:"))
            .expect("Net row present");
        assert!(
            architect_line.contains("1.2M"),
            "Architect tokens should show compacted: {architect_line}"
        );
        assert!(
            executor_line.contains("500.0k"),
            "Executor tokens should show compacted: {executor_line}"
        );
        assert!(
            net_line.contains('—'),
            "Net in tokens mode must be —: {net_line}"
        );
    }

    #[test]
    fn ledger_tokens_mode_has_no_parens() {
        let sess = ScopeReport {
            saved: Some(10.0),
            executor: 5.0,
            architect: Some(100.0),
            net: Some(-95.0),
            executor_tokens: 500_000,
            architect_tokens: 1_200_000,
        };
        let proj = ScopeReport {
            saved: Some(20.0),
            executor: 10.0,
            architect: Some(200.0),
            net: Some(-190.0),
            executor_tokens: 2_000_000,
            architect_tokens: 5_500_000,
        };
        let lines = ledger_lines(&sess, None, &proj, LedgerUnits::Tokens);
        // Skip the header (which contains "(tok)") — only data rows must have no parens.
        for line in &lines[1..] {
            assert!(
                !line.contains('('),
                "Tokens mode data rows must not contain parens: {line}"
            );
        }
    }

    #[test]
    fn format_costs_tokens_mode_omits_dollar_legend() {
        let report = CostReport {
            session: ScopeReport {
                saved: Some(10.0),
                executor: 5.0,
                architect: Some(0.0),
                net: Some(5.0),
                ..Default::default()
            },
            milestone: None,
            project: ScopeReport {
                saved: Some(50.0),
                executor: 25.0,
                architect: Some(10.0),
                net: Some(15.0),
                ..Default::default()
            },
            assists: 0,
            by_skill: Vec::new(),
        };
        let out = format_costs_with(&report, LedgerUnits::Tokens);
        assert!(
            !out.contains("Executor = Claude cost avoided"),
            "Dollar legend must be omitted in tokens mode: {out}"
        );
        assert!(
            !out.contains("SAVED ="),
            "Old SAVED legend must not appear: {out}"
        );
        // Dollars mode should have the legend
        let out_dollars = format_costs_with(&report, LedgerUnits::Dollars);
        assert!(
            out_dollars.contains("Executor = Claude cost avoided"),
            "Dollar legend must appear in dollars mode"
        );
    }
}
