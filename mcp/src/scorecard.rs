use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Serialize;

use rexymcp_executor::store::metrics;
use rexymcp_executor::store::telemetry::{Gates, PhaseRun};

/// One row of the model × settings matrix.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SettingsScorecardRow {
    pub model: String,
    /// Sampling-settings label, e.g. "temp=0.2,seed=42" or "default".
    pub settings: String,
    pub n_runs: usize,
    pub gates_pass_rate: f64,
    pub parse_failure_rate_mean: f64,
    /// Mean of `length_finish_rate` over runs where it is `Some`. `None` when none.
    pub length_finish_rate_mean: Option<f64>,
    pub repairs_per_call_mean: f64,
    pub tool_success_rate_mean: f64,
    pub verifier_retries_mean: f64,
    pub turns_mean: f64,
    pub wall_clock_s_mean: f64,
    pub escalation_rate: f64,
    pub n_with_verdict: usize,
    pub approved_first_try_rate: Option<f64>,
    pub bounces_to_approval_mean: Option<f64>,
    /// Mean peak context-window utilization (a FRACTION in [0.0, 1.0]) over the
    /// runs in this bucket that carry context telemetry (`peak_context_pct >
    /// 0.0`). `None` when no run in the bucket is context-measured.
    pub peak_context_pct_mean: Option<f64>,
    /// Mean total tokens reclaimed (sum of all four M10 sources) over the same
    /// context-measured runs. `None` when none are context-measured. A measured
    /// run that reclaimed nothing contributes `0.0`, not exclusion.
    pub tokens_reclaimed_mean: Option<f64>,
}

/// Which secondary dimension the scorecard buckets by (model is always the
/// primary key). `Model` = no secondary (one row per model); `Tag` = one row per
/// (model, tag) with tag-explosion; `Settings` = one row per (model, settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ScorecardDimension {
    Model,
    Tag,
    Settings,
}

/// One aggregated bucket — the dimension-neutral superset of `ScorecardRow` and
/// `SettingsScorecardRow`. `key` is the secondary-dimension label (`""` for
/// `Model`, the tag for `Tag`, the settings label for `Settings`).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ScorecardBucket {
    pub model: String,
    pub key: String,
    pub n_runs: usize,
    pub gates_pass_rate: f64,
    pub parse_failure_rate_mean: f64,
    pub length_finish_rate_mean: Option<f64>,
    pub repairs_per_call_mean: f64,
    pub tool_success_rate_mean: f64,
    pub verifier_retries_mean: f64,
    pub turns_mean: f64,
    pub wall_clock_s_mean: f64,
    pub escalation_rate: f64,
    pub n_with_verdict: usize,
    pub approved_first_try_rate: Option<f64>,
    pub bounces_to_approval_mean: Option<f64>,
    pub peak_context_pct_mean: Option<f64>,
    pub tokens_reclaimed_mean: Option<f64>,
}

/// Internal accumulator for a single (model, key) bucket.
#[derive(Debug, Default)]
struct SettingsAccumulator {
    n: usize,
    gates_all_pass: usize,
    parse_failure_rate_sum: f64,
    repairs_per_call_sum: f64,
    tool_success_rate_sum: f64,
    verifier_retries_sum: f64,
    turns_sum: f64,
    wall_clock_s_sum: f64,
    escalated_count: usize,
    length_finish_rate_sum: f64,
    length_finish_n: usize,
    n_with_verdict: usize,
    approved_first_try_count: usize,
    bounces_sum: f64,
    bounces_n: usize,
    peak_context_pct_sum: f64,
    tokens_reclaimed_sum: f64,
    context_measured_n: usize,
}

/// Unified scorecard aggregation across any secondary dimension.
///
/// `dimension` controls how runs are bucketed:
/// - `Model` — one bucket per model (no secondary key).
/// - `Tag` — one bucket per (model, tag), with tag-explosion.
/// - `Settings` — one bucket per (model, settings label).
///
/// The `filter` is applied per-run before bucketing.
fn aggregate_scorecard(
    runs: &[PhaseRun],
    dimension: ScorecardDimension,
    filter: &ScorecardFilter,
) -> Vec<ScorecardBucket> {
    let mut buckets: BTreeMap<(String, String), SettingsAccumulator> = BTreeMap::new();

    for run in runs {
        if let Some(model) = filter.model
            && run.model != model
        {
            continue;
        }

        if !filter.tags.is_empty() && !filter.tags.iter().all(|t| run.tags.contains(t)) {
            continue;
        }

        let keys: Vec<String> = match dimension {
            ScorecardDimension::Model => vec![String::new()],
            ScorecardDimension::Settings => {
                vec![metrics::settings_label(&run.generation_params)]
            }
            ScorecardDimension::Tag => run.tags.clone(),
        };

        for key in keys {
            let acc = buckets.entry((run.model.clone(), key)).or_default();
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

            if let Some(lr) = run.length_finish_rate {
                acc.length_finish_rate_sum += lr;
                acc.length_finish_n += 1;
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

            let eff = &run.context_efficiency;
            if eff.peak_context_pct > 0.0 {
                acc.peak_context_pct_sum += eff.peak_context_pct;
                acc.tokens_reclaimed_sum += metrics::reclaimed_total(eff) as f64;
                acc.context_measured_n += 1;
            }
        }
    }

    let mut rows: Vec<ScorecardBucket> = buckets
        .into_iter()
        .filter_map(|((model, key), acc)| {
            if acc.n < filter.min_runs {
                return None;
            }

            let n = acc.n as f64;

            Some(ScorecardBucket {
                model,
                key,
                n_runs: acc.n,
                gates_pass_rate: acc.gates_all_pass as f64 / n,
                parse_failure_rate_mean: acc.parse_failure_rate_sum / n,
                length_finish_rate_mean: if acc.length_finish_n > 0 {
                    Some(acc.length_finish_rate_sum / acc.length_finish_n as f64)
                } else {
                    None
                },
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
                peak_context_pct_mean: if acc.context_measured_n > 0 {
                    Some(acc.peak_context_pct_sum / acc.context_measured_n as f64)
                } else {
                    None
                },
                tokens_reclaimed_mean: if acc.context_measured_n > 0 {
                    Some(acc.tokens_reclaimed_sum / acc.context_measured_n as f64)
                } else {
                    None
                },
            })
        })
        .collect();

    rows.sort_by(|a, b| {
        a.key
            .cmp(&b.key)
            .then(b.n_runs.cmp(&a.n_runs))
            .then(a.model.cmp(&b.model))
    });

    rows
}

/// Aggregate runs into a **model × settings** competency matrix.
///
/// Unlike [`aggregate`] (model × tag, which explodes per tag), each run
/// contributes to exactly one (model, settings) bucket.
pub fn aggregate_by_settings(
    runs: &[PhaseRun],
    filter: &ScorecardFilter,
) -> Vec<SettingsScorecardRow> {
    aggregate_scorecard(runs, ScorecardDimension::Settings, filter)
        .into_iter()
        .map(|b| SettingsScorecardRow {
            model: b.model,
            settings: b.key,
            n_runs: b.n_runs,
            gates_pass_rate: b.gates_pass_rate,
            parse_failure_rate_mean: b.parse_failure_rate_mean,
            length_finish_rate_mean: b.length_finish_rate_mean,
            repairs_per_call_mean: b.repairs_per_call_mean,
            tool_success_rate_mean: b.tool_success_rate_mean,
            verifier_retries_mean: b.verifier_retries_mean,
            turns_mean: b.turns_mean,
            wall_clock_s_mean: b.wall_clock_s_mean,
            escalation_rate: b.escalation_rate,
            n_with_verdict: b.n_with_verdict,
            approved_first_try_rate: b.approved_first_try_rate,
            bounces_to_approval_mean: b.bounces_to_approval_mean,
            peak_context_pct_mean: b.peak_context_pct_mean,
            tokens_reclaimed_mean: b.tokens_reclaimed_mean,
        })
        .collect()
}

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
    /// Mean peak context-window utilization (a FRACTION in [0.0, 1.0]) over the
    /// runs in this bucket that carry context telemetry (`peak_context_pct >
    /// 0.0`). `None` when no run in the bucket is context-measured.
    pub peak_context_pct_mean: Option<f64>,
    /// Mean total tokens reclaimed (sum of all four M10 sources) over the same
    /// context-measured runs. `None` when none are context-measured. A measured
    /// run that reclaimed nothing contributes `0.0`, not exclusion.
    pub tokens_reclaimed_mean: Option<f64>,
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

fn gates_all_pass(gates: &Gates) -> bool {
    gates.fmt == Some(true)
        && gates.build == Some(true)
        && gates.lint == Some(true)
        && gates.test == Some(true)
}
pub fn aggregate(runs: &[PhaseRun], filter: &ScorecardFilter) -> Vec<ScorecardRow> {
    aggregate_scorecard(runs, ScorecardDimension::Tag, filter)
        .into_iter()
        .map(|b| ScorecardRow {
            model: b.model,
            tag: b.key,
            n_runs: b.n_runs,
            gates_pass_rate: b.gates_pass_rate,
            parse_failure_rate_mean: b.parse_failure_rate_mean,
            repairs_per_call_mean: b.repairs_per_call_mean,
            tool_success_rate_mean: b.tool_success_rate_mean,
            verifier_retries_mean: b.verifier_retries_mean,
            turns_mean: b.turns_mean,
            wall_clock_s_mean: b.wall_clock_s_mean,
            escalation_rate: b.escalation_rate,
            n_with_verdict: b.n_with_verdict,
            approved_first_try_rate: b.approved_first_try_rate,
            bounces_to_approval_mean: b.bounces_to_approval_mean,
            peak_context_pct_mean: b.peak_context_pct_mean,
            tokens_reclaimed_mean: b.tokens_reclaimed_mean,
        })
        .collect()
}

#[cfg(test)]
#[path = "scorecard_tests.rs"]
mod tests;
