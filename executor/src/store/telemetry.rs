//! Cross-project `PhaseRun` telemetry — one summary record per `execute_phase`,
//! appended as JSONL to a single global store (not per-repo). The durable
//! substrate for the M7 model scorecard (`model × tag`) and project review
//! (`milestone × phase`). The executor fills the objective fields at phase end;
//! the architect's review fills the supervision fields (`bugs_filed`,
//! `bounces_to_approval`, `architect_verdict`, `warnings`) later.
//!
//! **Versioning.** Every record is stamped with `schema_version` at the write
//! boundary. Readers skip records whose `schema_version` is missing or not
//! equal to the current version (`TELEMETRY_SCHEMA_VERSION`). This is how
//! pre-M35 records are retired — they simply have no version field.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ai::types::TokenBreakdown;
use crate::config::Tier;
use crate::store::sessions::event::{SessionEvent, SessionRecord};

/// Generation knobs for the run — "how" the model was asked. The executor layer
/// often does not know these (M5 populates from the request); `None` until then.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerationParams {
    pub temperature: Option<f64>,
    pub seed: Option<u64>,
}

/// Pass/fail of the final command set, captured on clean completion. `None` for a
/// command that was not configured, or any field when the phase did not complete
/// (the command set runs only on a clean finish).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Gates {
    pub fmt: Option<bool>,
    pub build: Option<bool>,
    pub lint: Option<bool>,
    pub test: Option<bool>,
}

/// Context-efficiency signal for one run, aggregated from the session JSONL at
/// phase end (M10). All token figures are chars/4 estimates, consistent with the
/// per-lever events that produce them. Nested in `PhaseRun` as a single
/// `#[serde(default)]` field so legacy records (and every struct literal) need
/// only `Default`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextEfficiency {
    /// Highest `context_pct` observed across the run's per-turn `Metrics`
    /// events; `0.0` if none were emitted.
    pub peak_context_pct: f64,
    /// Number of `Compaction` events the loop emitted.
    pub compaction_count: usize,
    /// Tokens freed by compaction: Σ(tokens_before − tokens_after) over
    /// `Compaction` events.
    pub compaction_tokens_reclaimed: usize,
    /// Tokens reclaimed by the Arc-A boundary output filter: Σ(tokens_before −
    /// tokens_after) over `OutputFiltered` events.
    pub output_filtered_tokens: usize,
    /// Tokens reclaimed by superseded-read eviction: Σ tokens_reclaimed over
    /// `ReadEvicted` events.
    pub read_evicted_tokens: usize,
    /// Tokens saved by redundant-read dedupe: Σ tokens_saved over
    /// `ReadDeduped` events.
    pub read_deduped_tokens: usize,
}

/// Aggregate the context-efficiency signal from a run's session-log records.
/// Pure over the slice; an empty slice yields `ContextEfficiency::default()`.
pub fn aggregate_context_efficiency(records: &[SessionRecord]) -> ContextEfficiency {
    let mut eff = ContextEfficiency::default();
    for rec in records {
        match &rec.event {
            SessionEvent::Metrics { context_pct, .. } => {
                eff.peak_context_pct = eff.peak_context_pct.max(*context_pct);
            }
            SessionEvent::Compaction {
                tokens_before,
                tokens_after,
                ..
            } => {
                eff.compaction_count += 1;
                eff.compaction_tokens_reclaimed += tokens_before.saturating_sub(*tokens_after);
            }
            SessionEvent::OutputFiltered {
                tokens_before,
                tokens_after,
                ..
            } => {
                eff.output_filtered_tokens += tokens_before.saturating_sub(*tokens_after);
            }
            SessionEvent::ReadEvicted {
                tokens_reclaimed, ..
            } => {
                eff.read_evicted_tokens += *tokens_reclaimed;
            }
            SessionEvent::ReadDeduped { tokens_saved, .. } => {
                eff.read_deduped_tokens += *tokens_saved;
            }
            _ => {}
        }
    }
    eff
}

/// Per-run M20 tier/cost instrumentation. Nested in `PhaseRun` as a single
/// `#[serde(default)]` field so legacy records and every struct literal need
/// only `Default` (the `ContextEfficiency` precedent). Only `tier` is
/// populated by the executor — the configured executor tier from
/// `[executor] tier`. Assist *counts* are derived from `assist`
/// `ArchitectActivity` journal records, not stored here.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TierTelemetry {
    /// Configured executor capability tier (`[executor] tier`); `None` when
    /// the project has not run `rexymcp calibrate`.
    pub tier: Option<Tier>,
}

/// One per-phase metrics row. Objective fields are filled by the executor; the
/// supervision fields are filled by the architect at review (M7).
/// (No `PartialEq` — `TokenBreakdown` doesn't implement it; compare via JSON.)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PhaseRun {
    pub ts: u64,
    // identity
    pub model: String,
    pub generation_params: GenerationParams,
    pub phase_id: String,
    /// Full path to the phase doc, for milestone-aware savings queries.
    /// `None` for legacy records that predate this field (M7 phase-08b and earlier).
    #[serde(default)]
    pub phase_doc_path: Option<String>,
    pub tags: Vec<String>,
    // outcome
    pub status: String,
    pub escalated: bool,
    // quality (objective)
    pub gates: Gates,
    // reliability (objective)
    pub parse_failure_rate: f64,
    pub repairs_per_call: f64,
    pub verifier_retries: usize,
    pub tool_success_rate: f64,
    // efficiency (objective)
    pub turns: usize,
    pub wall_clock_s: f64,
    /// Total wall time spent awaiting model generation across all calls,
    /// in seconds. tok/s derives as `tokens.output_tokens / gen_time_s`
    /// (guard zero). `0.0` for v1 records written before this field existed.
    #[serde(default)]
    pub gen_time_s: f64,
    pub tokens: TokenBreakdown,
    // supervision (architect-filled at review — M7)
    pub warnings: Option<u32>,
    pub bugs_filed: Option<u32>,
    pub bounces_to_approval: Option<u32>,
    pub architect_verdict: Option<String>,
    // provenance (endpoint-reported, captured from the chat stream)
    #[serde(default)]
    pub served_model: Option<String>,
    #[serde(default)]
    pub length_finish_rate: Option<f64>,
    /// Endpoint-reported context window (`max_model_len` from `/v1/models`);
    /// `None` if unknown or the endpoint does not report it.
    #[serde(default)]
    pub context_window: Option<usize>,
    /// Context-efficiency signal aggregated from the session JSONL at phase end
    /// (M10/phase-08a). Default (all zeros) for legacy records and for runs that
    /// produced no reclaim/metrics events.
    #[serde(default)]
    pub context_efficiency: ContextEfficiency,
    /// UUID from the target project's `[project] id` in `rexymcp.toml`. Used to
    /// scope telemetry to a specific project regardless of filesystem path.
    /// `None` for legacy records and projects that haven't run `rexymcp init`.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Milestone directory slug (e.g. `"M17-dashboard-polish-3"`) derived from
    /// the phase doc path. Used for milestone-scoped savings queries without
    /// relying on path substring matching.
    /// `None` when the phase doc is not inside a milestone directory.
    #[serde(default)]
    pub milestone_id: Option<String>,

    /// M20 tier/cost instrumentation. Default when the project has not run
    /// `rexymcp calibrate`.
    #[serde(default)]
    pub tier_telemetry: TierTelemetry,
}

/// Version stamped on every record this build writes; readers ignore records
/// at any other version (including pre-M35 records, which have none).
pub const TELEMETRY_SCHEMA_VERSION: u32 = 1;

/// Append one `PhaseRun` as a JSON line to `<telemetry_dir>/phase_runs.jsonl`,
/// creating the directory if needed. Stamps `schema_version` at the write
/// boundary so readers can version-gate. Returns the file path.
pub fn append(telemetry_dir: &Path, run: &PhaseRun) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let mut value = serde_json::to_value(run).map_err(std::io::Error::other)?;
    value["schema_version"] = TELEMETRY_SCHEMA_VERSION.into();
    let line = serde_json::to_string(&value).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// Read all `PhaseRun` records from a store file. Records with a missing or
/// non-current `schema_version` are skipped (pre-M35 retirement).
pub fn read(path: &Path) -> std::io::Result<Vec<PhaseRun>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| {
            v.get("schema_version").and_then(serde_json::Value::as_u64)
                == Some(TELEMETRY_SCHEMA_VERSION as u64)
        })
        .filter_map(|v| serde_json::from_value::<PhaseRun>(v).ok())
        .collect())
}

/// Overlay each `PhaseReview` onto its matching `PhaseRun`, returning runs with
/// the supervision fields populated. For each run, the matching review is the
/// **latest** (max `ts`) review whose phase identity equals the run's:
/// `phase_doc_path` when both have it, else (`project_id`, `phase_id`). A review
/// applies only to the **latest** run sharing that identity (the approved run);
/// earlier bounce runs are left unannotated. Runs with no matching review are
/// returned unchanged.
///
/// Note: `failure_class` is **not** stored on `PhaseRun` in this phase (no
/// `PhaseRun` field is added); the failure-class data reaches consumers through
/// `read_reviews` directly in phase-03.
pub fn fold_reviews(runs: Vec<PhaseRun>, reviews: &[PhaseReview]) -> Vec<PhaseRun> {
    // Build a map from identity key -> latest review (max ts)
    use std::collections::HashMap;

    #[derive(Debug, Clone, Hash, PartialEq, Eq)]
    enum Key {
        Path(String),
        IdProject(String, String),
    }

    fn key_for(r: &PhaseReview) -> Key {
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

    let mut latest_review: HashMap<Key, &PhaseReview> = HashMap::new();
    for rev in reviews {
        let k = key_for(rev);
        latest_review
            .entry(k)
            .and_modify(|existing| {
                if rev.ts > existing.ts {
                    *existing = rev;
                }
            })
            .or_insert(rev);
    }

    // Find the latest run per key
    let mut latest_run_ts: HashMap<Key, u64> = HashMap::new();
    for run in &runs {
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

    // Apply reviews to runs
    runs.into_iter()
        .map(|mut run| {
            let k = key_for_run(&run);
            if let Some(rev) = latest_review.get(&k) {
                // Only apply to the latest run for this key
                if run.ts == *latest_run_ts.get(&k).unwrap_or(&0) {
                    run.architect_verdict = Some(rev.architect_verdict.clone());
                    run.bounces_to_approval = rev.bounces_to_approval;
                    run.bugs_filed = rev.bugs_filed;
                    run.warnings = rev.warnings;
                }
            }
            run
        })
        .collect()
}

/// Canonical failure-class vocabulary for `PhaseReview.failure_class`. The list
/// is intentionally open — new classes fold in as they recur (WORKFLOW
/// § Calibration) — so this is a *documented* vocabulary, not a closed enum.
/// `spec_bug` and `infra_blip` exist so a bounce caused by the architect's spec
/// or by transient infrastructure is NOT charged against the model's competency.
pub const FAILURE_CLASSES: &[&str] = &[
    "none",              // clean approval
    "false_completion",  // self-reported complete on a red gate
    "prod_unwrap",       // unwrap/expect in a production path (STANDARDS §2.1)
    "multi_site_break",  // breaking multi-site type change ran out of verifier runway
    "parse_format",      // tool-call format / forgiving-parser repair churn
    "masked_diagnostic", // #[allow]/#[ignore] used to hide a warning/error
    "scope_deviation",   // touched out-of-scope files or widened scope
    "spec_bug",          // the bounce was the architect's spec fault, not the model's
    "infra_blip",        // transient backend/decode error, not a work defect
];

/// True if `class` is in the canonical `FAILURE_CLASSES` vocabulary.
pub fn is_known_failure_class(class: &str) -> bool {
    FAILURE_CLASSES.contains(&class)
}

/// An append-only architect-review annotation, folded onto its matching
/// `PhaseRun` at read time. Written by the `rexymcp review` CLI (phase-02);
/// the executor never writes one. Coexists with `PhaseRun` in
/// `phase_runs.jsonl`; the `record` discriminator keeps the two readers from
/// confusing the line types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseReview {
    /// Literal discriminator. Always `"review"`. `#[serde(default)]` so a
    /// `PhaseRun` line (which has no `record` field) deserializes to `""` here
    /// and is filtered out by `read_reviews`.
    #[serde(default)]
    pub record: String,
    pub ts: u64,
    /// Identity of the phase being reviewed. Prefer `phase_doc_path` (unique per
    /// phase doc); `phase_id` + `project_id` are the fallback key for runs that
    /// predate `phase_doc_path`.
    #[serde(default)]
    pub phase_doc_path: Option<String>,
    pub phase_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    // the supervision label
    pub architect_verdict: String,
    #[serde(default)]
    pub bounces_to_approval: Option<u32>,
    #[serde(default)]
    pub bugs_filed: Option<u32>,
    #[serde(default)]
    pub warnings: Option<u32>,
    /// Structured failure classes from `FAILURE_CLASSES`. Empty or `["none"]`
    /// for a clean approval. May carry several (a phase can fail two ways).
    #[serde(default)]
    pub failure_class: Vec<String>,
}

/// The literal value of `PhaseReview.record`. Use everywhere instead of a bare
/// string so the discriminator is single-sourced.
pub const REVIEW_RECORD_TAG: &str = "review";

/// Append one `PhaseReview` as a JSON line to `<telemetry_dir>/phase_runs.jsonl`
/// (the same store as `PhaseRun`). Stamps `schema_version` at the write
/// boundary. Returns the file path.
pub fn append_review(telemetry_dir: &Path, review: &PhaseReview) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let mut value = serde_json::to_value(review).map_err(std::io::Error::other)?;
    value["schema_version"] = TELEMETRY_SCHEMA_VERSION.into();
    let line = serde_json::to_string(&value).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// Read all `PhaseReview` records from a store file. Lines with a missing or
/// non-current `schema_version` are skipped, as are lines without
/// `record == "review"`.
pub fn read_reviews(path: &Path) -> std::io::Result<Vec<PhaseReview>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| {
            v.get("schema_version").and_then(serde_json::Value::as_u64)
                == Some(TELEMETRY_SCHEMA_VERSION as u64)
        })
        .filter_map(|v| serde_json::from_value::<PhaseReview>(v).ok())
        .filter(|r| r.record == REVIEW_RECORD_TAG)
        .collect())
}

/// Anthropic prompt-cache rate multipliers relative to the base input rate:
/// a **5-minute** cache write costs 1.25× input, a **1-hour** cache write costs 2×
/// input, and a cache **read** (hit) costs 0.1× input.
pub const CACHE_CREATION_RATE_MULTIPLIER: f64 = 1.25;
pub const CACHE_CREATION_1H_RATE_MULTIPLIER: f64 = 2.0;
pub const CACHE_READ_RATE_MULTIPLIER: f64 = 0.1;

/// The four token classes an architect (Claude Code) request bills separately.
/// One coherent type threaded everywhere the architect touches tokens, replacing
/// the flat `architect_*_tokens` pairs. `#[serde(default)]` so a legacy
/// `ArchitectActivity` line (flat fields, or none) deserializes to all-zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArchitectTokens {
    /// Uncached input tokens (`usage.input_tokens`).
    pub input: u64,
    /// Cache-creation input tokens (`usage.cache_creation_input_tokens`).
    pub cache_creation: u64,
    /// Cache-read input tokens (`usage.cache_read_input_tokens`).
    pub cache_read: u64,
    /// Output tokens (`usage.output_tokens`).
    pub output: u64,
}

/// Per-Mtok USD rates for each `ArchitectTokens` class.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ArchitectRates {
    pub input_per_mtok: f64,
    pub cache_creation_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub output_per_mtok: f64,
}

impl ArchitectTokens {
    /// Total USD cost of these tokens at the given per-class rates.
    pub fn cost(&self, rates: &ArchitectRates) -> f64 {
        let per_m = |toks: u64, rate: f64| (toks as f64 / 1_000_000.0) * rate;
        per_m(self.input, rates.input_per_mtok)
            + per_m(self.cache_creation, rates.cache_creation_per_mtok)
            + per_m(self.cache_read, rates.cache_read_per_mtok)
            + per_m(self.output, rates.output_per_mtok)
    }
}

/// Per-class USD-per-Mtok rates for **any** model's token cost (executor or
/// architect). Structurally identical to the architect rate type; aliased so
/// call sites read as model-neutral.
pub type ModelRates = ArchitectRates;

/// An append-only record of one architect activity in a `/rexymcp:auto` loop run — the portable loop journal. Appended to `phase_runs.jsonl` alongside `PhaseRun` and `PhaseReview`; the `record` discriminator (`"architect_activity"`) keeps the readers from confusing the line types. Written by the `rexymcp journal` CLI (the loop skill invokes it); the executor never writes one. The `tokens` field defaults to all-zero and is filled by the phase-05b usage harvester on Claude Code; on other clients they stay zero (counts-and-durations, never fabricated).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectActivity {
    /// Literal discriminator. Always `"architect_activity"`. `#[serde(default)]` so a `PhaseRun` line (no `record` field) deserializes to `""` here and is filtered out by `read_architect_activities`.
    #[serde(default)]
    pub record: String,
    pub ts: u64,
    /// Identity of the phase this activity concerns. Prefer `phase_doc_path`; `phase_id` + `project_id` are the fallback key (mirrors `PhaseReview`).
    #[serde(default)]
    pub phase_doc_path: Option<String>,
    pub phase_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    /// Milestone directory slug (e.g. `"M27-autonomous-escalation-loop"`) for milestone-scoped queries. `None` when the loop did not supply one.
    #[serde(default)]
    pub milestone_id: Option<String>,
    /// The activity kind — one of `ARCHITECT_ACTIVITIES`.
    pub activity: String,
    /// Free-text outcome of the activity (e.g. `"complete"`, `"hard_fail"`, `"approved_first_try"`, `"bounced"`). `None` when not applicable.
    #[serde(default)]
    pub outcome: Option<String>,
    /// Architect model that performed the activity (e.g. `"claude-opus-4-8"`).
    #[serde(default)]
    pub model: Option<String>,
    /// Token usage for this activity, by class. All zero until the phase-05b
    /// harvester fills it; on non-Claude-Code clients they stay zero
    /// (counts-and-durations, never fabricated).
    #[serde(default)]
    pub tokens: ArchitectTokens,
}

/// The literal value of `ArchitectActivity.record`. Use everywhere instead of a bare string so the discriminator is single-sourced.
pub const ARCHITECT_ACTIVITY_RECORD_TAG: &str = "architect_activity";

/// Canonical architect-activity vocabulary for `ArchitectActivity.activity`. Intentionally open (new kinds fold in as the loop grows) — a *documented* vocabulary, not a closed enum, matching `FAILURE_CLASSES`.
pub const ARCHITECT_ACTIVITIES: &[&str] = &[
    "draft",    // authored or refined a phase doc
    "dispatch", // dispatched a phase to the executor
    "review",   // reviewed a completed phase against the DoD
    "assist",   // refined + re-dispatched after hard_fail/budget_exceeded
    "takeover", // took the phase over directly (session takeover)
    "boundary", // reached a milestone boundary or a loop stop condition
];

/// True if `activity` is in the canonical `ARCHITECT_ACTIVITIES` vocabulary.
pub fn is_known_activity(activity: &str) -> bool {
    ARCHITECT_ACTIVITIES.contains(&activity)
}

/// Collapse `ArchitectActivity` records to one per activity identity, keeping the
/// **last** occurrence in input order. The phase-05b harvester appends an enriched
/// copy (same `phase_id`/`activity`/`ts`, tokens filled) after the original
/// zero-token record; since `read_architect_activities` preserves file (append)
/// order, the later enriched copy wins. Identity key: `(phase_id, activity, ts)`.
pub fn fold_activities(activities: Vec<ArchitectActivity>) -> Vec<ArchitectActivity> {
    use std::collections::HashMap;
    // Index of the winning (latest) record per key, into a preserved-order Vec.
    let mut latest: HashMap<(String, String, u64), usize> = HashMap::new();
    let mut out: Vec<ArchitectActivity> = Vec::new();
    for act in activities {
        let key = (act.phase_id.clone(), act.activity.clone(), act.ts);
        if let Some(&idx) = latest.get(&key) {
            out[idx] = act;
        } else {
            latest.insert(key, out.len());
            out.push(act);
        }
    }
    out
}

/// Append one `ArchitectActivity` as a JSON line to `<telemetry_dir>/phase_runs.jsonl`.
/// Stamps `schema_version` at the write boundary. Returns the file path.
pub fn append_architect_activity(
    telemetry_dir: &Path,
    activity: &ArchitectActivity,
) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let mut value = serde_json::to_value(activity).map_err(std::io::Error::other)?;
    value["schema_version"] = TELEMETRY_SCHEMA_VERSION.into();
    let line = serde_json::to_string(&value).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// Read all `ArchitectActivity` records from a store file. Lines with a missing
/// or non-current `schema_version` are skipped, as are lines without
/// `record == "architect_activity"`.
pub fn read_architect_activities(path: &Path) -> std::io::Result<Vec<ArchitectActivity>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| {
            v.get("schema_version").and_then(serde_json::Value::as_u64)
                == Some(TELEMETRY_SCHEMA_VERSION as u64)
        })
        .filter_map(|v| serde_json::from_value::<ArchitectActivity>(v).ok())
        .filter(|a| a.record == ARCHITECT_ACTIVITY_RECORD_TAG)
        .collect())
}

/// The literal value of `ArchitectLedger.record`. Single-sources the discriminator.
pub const ARCHITECT_LEDGER_RECORD_TAG: &str = "architect_ledger";

/// One harvested architect-usage bucket: the token totals for a single
/// `(project_id, session_id, model, skill)` slice of a project's Claude Code
/// transcripts. Written by `rexymcp harvest`; the executor never writes one.
/// Coexists with `PhaseRun` / `PhaseReview` / `ArchitectActivity` in
/// `phase_runs.jsonl`, discriminated by `record`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectLedger {
    /// Literal discriminator. Always `"architect_ledger"`. `#[serde(default)]`
    /// so a line of another record type deserializes with `record == ""` here
    /// and is filtered out by `read_architect_ledger`.
    #[serde(default)]
    pub record: String,
    /// Project identity (from `[project].id` or `--project-id`). `None` if unset.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Claude Code session identity — the transcript file stem (== the file's
    /// `sessionId`). Since all `message.id` duplicates are within one file,
    /// dedup makes this unambiguous.
    pub session_id: String,
    /// The architect model that produced these tokens (`message.model`).
    pub model: String,
    /// The skill/slash-command the tokens were attributed to
    /// (`attributionSkill`), or `"other"` when the message carried none.
    pub skill: String,
    /// Summed four-class token usage over the deduped messages in this slice.
    pub tokens: ArchitectTokens,
    /// 5-minute-TTL share of `tokens.cache_creation`
    /// (`usage.cache_creation.ephemeral_5m_input_tokens`, summed).
    #[serde(default)]
    pub cache_creation_5m: u64,
    /// 1-hour-TTL share of `tokens.cache_creation`
    /// (`usage.cache_creation.ephemeral_1h_input_tokens`, summed).
    #[serde(default)]
    pub cache_creation_1h: u64,
    /// Count of deduped messages folded into this slice.
    pub messages: u64,
    /// Epoch-ms of the latest message in this slice (harvest freshness signal).
    pub last_ts: u64,
}

impl ArchitectLedger {
    /// USD cost of this ledger slice at the given base `(input, output)` $/Mtok
    /// rates. Cache rates derive from the input rate via the standard Anthropic
    /// multipliers (read 0.1×, 5m-write 1.25×, 1h-write 2×), pricing the 5m and 1h
    /// cache-write buckets separately.
    pub fn cost(&self, input_per_mtok: f64, output_per_mtok: f64) -> f64 {
        let per_m = |toks: u64, rate: f64| (toks as f64 / 1_000_000.0) * rate;
        per_m(self.tokens.input, input_per_mtok)
            + per_m(self.tokens.output, output_per_mtok)
            + per_m(
                self.tokens.cache_read,
                input_per_mtok * CACHE_READ_RATE_MULTIPLIER,
            )
            + per_m(
                self.cache_creation_5m,
                input_per_mtok * CACHE_CREATION_RATE_MULTIPLIER,
            )
            + per_m(
                self.cache_creation_1h,
                input_per_mtok * CACHE_CREATION_1H_RATE_MULTIPLIER,
            )
    }
}

/// Fold `ArchitectLedger` records: keep the **last** occurrence per
/// `(project_id, session_id, model, skill)` key, preserving input order.
/// This is what makes re-harvest idempotent: a second harvest appends fresh
/// full-sum records that replace the prior ones per key.
pub fn fold_ledger(ledgers: Vec<ArchitectLedger>) -> Vec<ArchitectLedger> {
    use std::collections::HashMap;
    let mut latest: HashMap<(Option<String>, String, String, String), usize> = HashMap::new();
    let mut out: Vec<ArchitectLedger> = Vec::new();
    for l in ledgers {
        let key = (
            l.project_id.clone(),
            l.session_id.clone(),
            l.model.clone(),
            l.skill.clone(),
        );
        if let Some(&idx) = latest.get(&key) {
            out[idx] = l;
        } else {
            latest.insert(key, out.len());
            out.push(l);
        }
    }
    out
}

/// Append one `ArchitectLedger` as a JSON line to `<telemetry_dir>/phase_runs.jsonl`.
/// Stamps `schema_version` at the write boundary. Returns the file path.
pub fn append_architect_ledger(
    telemetry_dir: &Path,
    ledger: &ArchitectLedger,
) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let mut value = serde_json::to_value(ledger).map_err(std::io::Error::other)?;
    value["schema_version"] = TELEMETRY_SCHEMA_VERSION.into();
    let line = serde_json::to_string(&value).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// Read all `ArchitectLedger` records from a store file. Lines with a missing
/// or non-current `schema_version` are skipped, as are lines without
/// `record == "architect_ledger"`.
pub fn read_architect_ledger(path: &Path) -> std::io::Result<Vec<ArchitectLedger>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| {
            v.get("schema_version").and_then(serde_json::Value::as_u64)
                == Some(TELEMETRY_SCHEMA_VERSION as u64)
        })
        .filter_map(|v| serde_json::from_value::<ArchitectLedger>(v).ok())
        .filter(|l| l.record == ARCHITECT_LEDGER_RECORD_TAG)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::sessions::event::{SessionEvent, SessionRecord};
    use tempfile::TempDir;

    #[test]
    fn append_stamps_schema_version() {
        let dir = TempDir::new().unwrap();
        let run = sample();
        append(dir.path(), &run).unwrap();
        let content = std::fs::read_to_string(dir.path().join("phase_runs.jsonl")).unwrap();
        assert!(
            content.contains("\"schema_version\":1"),
            "appended line should contain schema_version:1"
        );
    }

    #[test]
    fn read_skips_records_without_current_schema_version() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let pre_m35 = r#"{"ts":1,"model":"t","generation_params":{},"phase_id":"p","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{}}"#;
        let old_version = r#"{"schema_version":999,"ts":1,"model":"t","generation_params":{},"phase_id":"p","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{}}"#;
        let mut file = std::fs::File::create(&path).unwrap();
        use std::io::Write;
        file.write_all(pre_m35.as_bytes()).unwrap();
        file.write_all(b"\n").unwrap();
        file.write_all(old_version.as_bytes()).unwrap();
        file.write_all(b"\n").unwrap();
        let run = sample();
        append(dir.path(), &run).unwrap();
        let results = read(&path).unwrap();
        assert_eq!(
            results.len(),
            1,
            "only the current-version record should be returned"
        );
    }

    #[test]
    fn read_reviews_version_gates_and_keeps_record_tag_filter() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let legacy_review = r#"{"record":"review","ts":1,"model":"t","generation_params":{},"phase_id":"p","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{}}"#;
        let mut file = std::fs::File::create(&path).unwrap();
        use std::io::Write;
        file.write_all(legacy_review.as_bytes()).unwrap();
        file.write_all(b"\n").unwrap();
        let run = sample();
        append(dir.path(), &run).unwrap();
        let results = read_reviews(&path).unwrap();
        assert_eq!(
            results.len(),
            0,
            "no reviews should survive version gate + record tag filter"
        );
    }

    #[test]
    fn read_architect_activities_version_gates() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let legacy_activity = r#"{"record":"architect_activity","ts":1,"model":"t","generation_params":{},"phase_id":"p","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{}}"#;
        let mut file = std::fs::File::create(&path).unwrap();
        use std::io::Write;
        file.write_all(legacy_activity.as_bytes()).unwrap();
        file.write_all(b"\n").unwrap();
        let results = read_architect_activities(&path).unwrap();
        assert_eq!(
            results.len(),
            0,
            "no activities should survive version gate"
        );
    }

    fn make_metrics(context_pct: f64) -> SessionRecord {
        SessionRecord {
            ts: 0,
            turn: 0,
            event: SessionEvent::Metrics {
                input_tokens: 0,
                output_tokens: 0,
                context_pct,
                context_used: 0,
                context_window: 0,
            },
        }
    }

    fn make_compaction(tokens_before: usize, tokens_after: usize) -> SessionRecord {
        SessionRecord {
            ts: 0,
            turn: 0,
            event: SessionEvent::Compaction {
                tokens_before,
                tokens_after,
                messages_signaturized: 0,
                messages_evicted: 0,
            },
        }
    }

    fn make_output_filtered(tokens_before: usize, tokens_after: usize) -> SessionRecord {
        SessionRecord {
            ts: 0,
            turn: 0,
            event: SessionEvent::OutputFiltered {
                tokens_before,
                tokens_after,
                filter: "test".into(),
            },
        }
    }

    fn make_read_evicted(tokens_reclaimed: usize) -> SessionRecord {
        SessionRecord {
            ts: 0,
            turn: 0,
            event: SessionEvent::ReadEvicted {
                path: "file.rs".into(),
                reads_evicted: 1,
                tokens_reclaimed,
            },
        }
    }

    fn make_read_deduped(tokens_saved: usize) -> SessionRecord {
        SessionRecord {
            ts: 0,
            turn: 0,
            event: SessionEvent::ReadDeduped {
                path: "file.rs".into(),
                tokens_saved,
                prior_turn: 0,
            },
        }
    }

    #[test]
    fn aggregate_context_efficiency_empty_is_default() {
        assert_eq!(
            aggregate_context_efficiency(&[]),
            ContextEfficiency::default()
        );
    }

    #[test]
    fn aggregate_context_efficiency_peak_is_max_not_last() {
        let records = vec![make_metrics(0.4), make_metrics(0.9), make_metrics(0.2)];
        let eff = aggregate_context_efficiency(&records);
        assert_eq!(eff.peak_context_pct, 0.9);
    }

    #[test]
    fn aggregate_context_efficiency_sums_compaction() {
        let records = vec![make_compaction(1000, 600), make_compaction(500, 500)];
        let eff = aggregate_context_efficiency(&records);
        assert_eq!(eff.compaction_count, 2);
        assert_eq!(eff.compaction_tokens_reclaimed, 400);
    }

    #[test]
    fn aggregate_context_efficiency_sums_each_reclaim_source_independently() {
        let records = vec![
            make_output_filtered(200, 100),
            make_read_evicted(50),
            make_read_deduped(30),
        ];
        let eff = aggregate_context_efficiency(&records);
        assert_eq!(eff.output_filtered_tokens, 100);
        assert_eq!(eff.read_evicted_tokens, 50);
        assert_eq!(eff.read_deduped_tokens, 30);
        assert_eq!(eff.compaction_count, 0);
        assert_eq!(eff.peak_context_pct, 0.0);
    }

    #[test]
    fn aggregate_context_efficiency_ignores_unrelated_events() {
        let records = vec![
            SessionRecord {
                ts: 0,
                turn: 0,
                event: SessionEvent::Prompt {
                    rendered: "hi".into(),
                },
            },
            SessionRecord {
                ts: 0,
                turn: 0,
                event: SessionEvent::Completion { raw: "done".into() },
            },
            SessionRecord {
                ts: 0,
                turn: 0,
                event: SessionEvent::SessionStart {
                    session_id: "s".into(),
                    model: "m".into(),
                    phase: "p".into(),
                },
            },
            SessionRecord {
                ts: 0,
                turn: 0,
                event: SessionEvent::SessionEnd {
                    status: "complete".into(),
                    turns: 1,
                },
            },
        ];
        assert_eq!(
            aggregate_context_efficiency(&records),
            ContextEfficiency::default()
        );
    }

    #[test]
    fn phase_run_without_context_efficiency_deserializes() {
        // Legacy JSONL line lacking context_efficiency (and context_window)
        let legacy_json = r#"{"ts":1717000000000,"model":"qwen2.5-coder","generation_params":{"temperature":null,"seed":null},"phase_id":"phase-08","tags":["rust"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.1,"repairs_per_call":0.5,"verifier_retries":2,"tool_success_rate":0.9,"turns":7,"wall_clock_s":12.5,"tokens":{"input_tokens":0,"output_tokens":0,"cache_read_tokens":0,"cache_write_tokens":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null,"served_model":null,"length_finish_rate":null}"#;
        let run: PhaseRun = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(run.context_efficiency, ContextEfficiency::default());
    }

    fn sample() -> PhaseRun {
        PhaseRun {
            ts: 1_717_000_000_000,
            model: "qwen2.5-coder".to_string(),
            generation_params: GenerationParams::default(),
            phase_id: "phase-08".to_string(),
            phase_doc_path: Some("/docs/dev/milestones/M17/phase-08.md".to_string()),
            tags: vec!["rust".to_string(), "feature".to_string()],
            status: "complete".to_string(),
            escalated: false,
            gates: Gates {
                fmt: Some(true),
                build: Some(true),
                lint: Some(true),
                test: Some(false),
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
    fn phase_run_round_trips_through_json() {
        let run = sample();
        let json = serde_json::to_string(&run).unwrap();
        let back: PhaseRun = serde_json::from_str(&json).unwrap();
        // TokenBreakdown isn't PartialEq; compare via re-serialization.
        assert_eq!(json, serde_json::to_string(&back).unwrap());
    }

    #[test]
    fn phase_run_phase_doc_path_round_trips() {
        let json = r#"{"ts":1,"model":"t","generation_params":{},"phase_id":"p","phase_doc_path":"/a/b.md","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{}}"#;
        let run: PhaseRun = serde_json::from_str(json).unwrap();
        assert_eq!(run.phase_doc_path.as_deref(), Some("/a/b.md"));
    }

    #[test]
    fn phase_run_phase_doc_path_defaults_none_on_legacy_record() {
        // A JSON record without phase_doc_path — as emitted before this phase.
        let json = r#"{"ts":1,"model":"t","generation_params":{},"phase_id":"p","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{}}"#;
        let run: PhaseRun = serde_json::from_str(json).unwrap();
        assert!(run.phase_doc_path.is_none(), "legacy record must not error");
    }

    #[test]
    fn append_writes_one_line_per_run() {
        let dir = TempDir::new().unwrap();
        let path = append(dir.path(), &sample()).unwrap();
        append(dir.path(), &sample()).unwrap();
        let records = read(&path).unwrap();
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn read_missing_file_is_empty() {
        let records = read(Path::new("/nonexistent/phase_runs.jsonl")).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn phase_run_without_provenance_fields_deserializes() {
        // Legacy JSONL line lacking served_model and length_finish_rate
        let legacy_json = r#"{"ts":1717000000000,"model":"qwen2.5-coder","generation_params":{"temperature":null,"seed":null},"phase_id":"phase-08","tags":["rust"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.1,"repairs_per_call":0.5,"verifier_retries":2,"tool_success_rate":0.9,"turns":7,"wall_clock_s":12.5,"tokens":{"input_tokens":0,"output_tokens":0,"cache_read_tokens":0,"cache_write_tokens":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}"#;
        let run: PhaseRun = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(run.served_model, None);
        assert_eq!(run.length_finish_rate, None);
        assert_eq!(run.model, "qwen2.5-coder");
    }

    #[test]
    fn phase_run_without_context_window_deserializes() {
        // Legacy JSONL line lacking context_window
        let legacy_json = r#"{"ts":1717000000000,"model":"qwen2.5-coder","generation_params":{"temperature":null,"seed":null},"phase_id":"phase-08","tags":["rust"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.1,"repairs_per_call":0.5,"verifier_retries":2,"tool_success_rate":0.9,"turns":7,"wall_clock_s":12.5,"tokens":{"input_tokens":0,"output_tokens":0,"cache_read_tokens":0,"cache_write_tokens":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null,"served_model":null,"length_finish_rate":null}"#;
        let run: PhaseRun = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(run.context_window, None);
        assert_eq!(run.model, "qwen2.5-coder");
    }

    #[test]
    fn phase_review_round_trips() {
        let dir = TempDir::new().unwrap();
        let review = PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 1_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            phase_id: "phase-01".to_string(),
            project_id: Some("proj-a".to_string()),
            architect_verdict: "approved".to_string(),
            bounces_to_approval: Some(1),
            bugs_filed: Some(0),
            warnings: Some(2),
            failure_class: vec!["none".to_string()],
        };
        append_review(dir.path(), &review).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let reviews = read_reviews(&path).unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0], review);
    }

    #[test]
    fn read_skips_review_lines() {
        let dir = TempDir::new().unwrap();
        let run = sample();
        append(dir.path(), &run).unwrap();
        let review = PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 2_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            phase_id: "phase-01".to_string(),
            project_id: Some("proj-a".to_string()),
            architect_verdict: "approved".to_string(),
            bounces_to_approval: Some(1),
            bugs_filed: Some(0),
            warnings: Some(2),
            failure_class: vec!["none".to_string()],
        };
        append_review(dir.path(), &review).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let records = read(&path).unwrap();
        assert_eq!(records.len(), 1, "read should return only PhaseRun records");
        assert_eq!(records[0].phase_id, run.phase_id);
        assert_eq!(records[0].model, run.model);
    }

    #[test]
    fn read_reviews_skips_run_lines() {
        let dir = TempDir::new().unwrap();
        // Must carry a non-null architect_verdict so the serialized line
        // deserializes into a phantom PhaseReview (record defaults to "",
        // architect_verdict is a required String). The .record filter is what
        // excludes it — without the filter this test would see 2 reviews.
        let run = PhaseRun {
            architect_verdict: Some("approved".to_string()),
            ..sample()
        };
        append(dir.path(), &run).unwrap();
        let review = PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 2_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            phase_id: "phase-01".to_string(),
            project_id: Some("proj-a".to_string()),
            architect_verdict: "approved".to_string(),
            bounces_to_approval: Some(1),
            bugs_filed: Some(0),
            warnings: Some(2),
            failure_class: vec!["none".to_string()],
        };
        append_review(dir.path(), &review).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let reviews = read_reviews(&path).unwrap();
        assert_eq!(
            reviews.len(),
            1,
            "read_reviews should return only review records"
        );
        assert_eq!(reviews[0].record, "review");
    }

    #[test]
    fn fold_reviews_overlays_by_doc_path() {
        let run = PhaseRun {
            ts: 1_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            ..sample()
        };
        let review = PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 2_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            phase_id: "phase-01".to_string(),
            project_id: Some("proj-a".to_string()),
            architect_verdict: "approved".to_string(),
            bounces_to_approval: Some(1),
            bugs_filed: Some(0),
            warnings: Some(2),
            failure_class: vec!["none".to_string()],
        };
        let folded = fold_reviews(vec![run], &[review]);
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].architect_verdict, Some("approved".to_string()));
        assert_eq!(folded[0].bounces_to_approval, Some(1));
        assert_eq!(folded[0].bugs_filed, Some(0));
        assert_eq!(folded[0].warnings, Some(2));
    }

    #[test]
    fn fold_reviews_falls_back_to_id_project() {
        let run = PhaseRun {
            ts: 1_000,
            phase_doc_path: None,
            phase_id: "phase-01".to_string(),
            project_id: Some("proj-a".to_string()),
            ..sample()
        };
        let review = PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 2_000,
            phase_doc_path: None,
            phase_id: "phase-01".to_string(),
            project_id: Some("proj-a".to_string()),
            architect_verdict: "approved".to_string(),
            bounces_to_approval: Some(1),
            bugs_filed: Some(0),
            warnings: Some(2),
            failure_class: vec!["none".to_string()],
        };
        let folded = fold_reviews(vec![run], std::slice::from_ref(&review));
        assert_eq!(folded[0].architect_verdict, Some("approved".to_string()));

        // Differing project_id must NOT match (pinned negative)
        let run_b = PhaseRun {
            ts: 1_000,
            phase_doc_path: None,
            phase_id: "phase-01".to_string(),
            project_id: Some("proj-b".to_string()),
            ..sample()
        };
        let folded_b = fold_reviews(vec![run_b], &[review]);
        assert_eq!(
            folded_b[0].architect_verdict, None,
            "different project_id must not cross-fold"
        );
    }

    #[test]
    fn fold_reviews_latest_review_wins() {
        let run = PhaseRun {
            ts: 1_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            ..sample()
        };
        let review_old = PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 2_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            phase_id: "phase-01".to_string(),
            project_id: None,
            architect_verdict: "bounced".to_string(),
            bounces_to_approval: None,
            bugs_filed: None,
            warnings: None,
            failure_class: vec!["false_completion".to_string()],
        };
        let review_new = PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 3_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            phase_id: "phase-01".to_string(),
            project_id: None,
            architect_verdict: "approved".to_string(),
            bounces_to_approval: Some(2),
            bugs_filed: Some(1),
            warnings: Some(0),
            failure_class: vec!["none".to_string()],
        };
        let folded = fold_reviews(vec![run], &[review_old, review_new]);
        assert_eq!(
            folded[0].architect_verdict,
            Some("approved".to_string()),
            "latest review (ts=3000) should win"
        );
        assert_eq!(folded[0].bounces_to_approval, Some(2));
    }

    #[test]
    fn fold_reviews_applies_to_latest_run() {
        let run_old = PhaseRun {
            ts: 1_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            ..sample()
        };
        let run_new = PhaseRun {
            ts: 2_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            ..sample()
        };
        let review = PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 3_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            phase_id: "phase-01".to_string(),
            project_id: None,
            architect_verdict: "approved".to_string(),
            bounces_to_approval: Some(1),
            bugs_filed: Some(0),
            warnings: Some(0),
            failure_class: vec!["none".to_string()],
        };
        let folded = fold_reviews(vec![run_old, run_new], &[review]);
        assert_eq!(
            folded[0].architect_verdict, None,
            "earlier run should stay unannotated"
        );
        assert_eq!(
            folded[1].architect_verdict,
            Some("approved".to_string()),
            "latest run should be annotated"
        );
    }

    #[test]
    fn fold_reviews_leaves_unmatched_none() {
        let run = PhaseRun {
            ts: 1_000,
            phase_doc_path: Some("/docs/phase-01.md".to_string()),
            ..sample()
        };
        let review = PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 2_000,
            phase_doc_path: Some("/docs/other-phase.md".to_string()),
            phase_id: "phase-02".to_string(),
            project_id: None,
            architect_verdict: "approved".to_string(),
            bounces_to_approval: None,
            bugs_filed: None,
            warnings: None,
            failure_class: vec!["none".to_string()],
        };
        let folded = fold_reviews(vec![run], &[review]);
        assert_eq!(folded[0].architect_verdict, None);
        assert_eq!(folded[0].bounces_to_approval, None);
        assert_eq!(folded[0].bugs_filed, None);
        assert_eq!(folded[0].warnings, None);
    }

    #[test]
    fn is_known_failure_class_validates_vocabulary() {
        assert!(is_known_failure_class("false_completion"));
        assert!(is_known_failure_class("none"));
        assert!(is_known_failure_class("spec_bug"));
        assert!(is_known_failure_class("infra_blip"));
        assert!(!is_known_failure_class("made_up"));
    }

    #[test]
    fn tier_telemetry_current_shape_roundtrips() {
        let mut run = sample();
        run.tier_telemetry = TierTelemetry {
            tier: Some(Tier::Medium),
        };
        let json = serde_json::to_string(&run).unwrap();
        let back: PhaseRun = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tier_telemetry, run.tier_telemetry);
    }

    #[test]
    fn phase_run_without_tier_telemetry_deserializes() {
        // A legacy JSONL line lacking tier_telemetry — as emitted before this phase.
        let legacy_json = r#"{"ts":1,"model":"t","generation_params":{},"phase_id":"p","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{}}"#;
        let run: PhaseRun = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(run.tier_telemetry, TierTelemetry::default());
        assert_eq!(run.tier_telemetry.tier, None);
    }

    #[test]
    fn phase_run_ignores_retired_escalation_count_key() {
        // Legacy tier_telemetry keys (escalation_count, architect_*_tokens) are
        // silently ignored — serde drops unknown fields.
        let json = r#"{"ts":1,"model":"t","generation_params":{},"phase_id":"p","tags":[],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{},"tier_telemetry":{"tier":"MEDIUM","escalation_count":3,"architect_input_tokens":1000,"architect_output_tokens":200}}"#;
        let run: PhaseRun = serde_json::from_str(json).unwrap();
        assert_eq!(run.tier_telemetry.tier, Some(Tier::Medium));
    }

    #[test]
    fn tier_serializes_uppercase_in_telemetry() {
        let mut run = sample();
        run.tier_telemetry.tier = Some(Tier::Small);
        let json = serde_json::to_string(&run).unwrap();
        assert!(
            json.contains("\"tier\":\"SMALL\""),
            "tier must serialize UPPERCASE: {json}"
        );
    }

    #[test]
    fn architect_activity_round_trips() {
        let dir = TempDir::new().unwrap();
        let activity = ArchitectActivity {
            record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
            ts: 1_000,
            phase_doc_path: Some("/docs/phase-02.md".to_string()),
            phase_id: "phase-02".to_string(),
            project_id: Some("proj-a".to_string()),
            milestone_id: Some("M27-autonomous-escalation-loop".to_string()),
            activity: "assist".to_string(),
            outcome: Some("complete".to_string()),
            model: Some("claude-opus-4-8".to_string()),
            tokens: ArchitectTokens {
                input: 1500,
                cache_creation: 0,
                cache_read: 0,
                output: 300,
            },
        };
        append_architect_activity(dir.path(), &activity).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let activities = read_architect_activities(&path).unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0], activity);
    }

    #[test]
    fn read_architect_activities_excludes_run_lines() {
        // A PhaseRun line must not be read as an ArchitectActivity.
        let dir = TempDir::new().unwrap();
        append(dir.path(), &sample()).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let activities = read_architect_activities(&path).unwrap();
        assert!(
            activities.is_empty(),
            "PhaseRun lines must not be read as architect activities"
        );
    }

    #[test]
    fn read_architect_activities_excludes_review_by_discriminator() {
        // An activity-SHAPED line (all required ArchitectActivity fields present)
        // with a non-"architect_activity" record tag must be excluded by the `record`
        // filter — not by structural mismatch. Deleting the
        // `.filter(|a| a.record == ARCHITECT_ACTIVITY_RECORD_TAG)` line in
        // read_architect_activities MUST fail this test (M18 bug-01-1 lesson: pin the
        // discriminator as load-bearing).
        let dir = TempDir::new().unwrap();
        let mistagged = ArchitectActivity {
            record: REVIEW_RECORD_TAG.to_string(), // wrong tag, activity shape
            ts: 2_000,
            phase_doc_path: Some("/docs/phase-02.md".to_string()),
            phase_id: "phase-02".to_string(),
            project_id: None,
            milestone_id: None,
            activity: "assist".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        append_architect_activity(dir.path(), &mistagged).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let activities = read_architect_activities(&path).unwrap();
        assert!(
            activities.is_empty(),
            "a non-\"architect_activity\" record tag must be excluded by the discriminator"
        );
    }

    #[test]
    fn read_skips_architect_activity_lines() {
        // The existing PhaseRun reader must not pick up architect activity lines.
        let dir = TempDir::new().unwrap();
        append(dir.path(), &sample()).unwrap();
        let activity = ArchitectActivity {
            record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
            ts: 1,
            phase_doc_path: None,
            phase_id: "p".to_string(),
            project_id: None,
            milestone_id: None,
            activity: "assist".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        append_architect_activity(dir.path(), &activity).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let runs = read(&path).unwrap();
        assert_eq!(
            runs.len(),
            1,
            "read() must skip the architect activity line"
        );
    }

    #[test]
    fn read_reviews_skips_architect_activity_lines() {
        let dir = TempDir::new().unwrap();
        let activity = ArchitectActivity {
            record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
            ts: 1,
            phase_doc_path: None,
            phase_id: "p".to_string(),
            project_id: None,
            milestone_id: None,
            activity: "assist".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        append_architect_activity(dir.path(), &activity).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let reviews = read_reviews(&path).unwrap();
        assert!(
            reviews.is_empty(),
            "read_reviews() must skip the architect activity line"
        );
    }

    #[test]
    fn is_known_activity_validates_vocabulary() {
        assert!(is_known_activity("draft"));
        assert!(is_known_activity("assist"));
        assert!(is_known_activity("boundary"));
        assert!(!is_known_activity("made_up"));
    }

    #[test]
    fn architect_tokens_cost_bills_each_class_at_its_own_rate() {
        let tokens = ArchitectTokens {
            input: 1_000_000,
            cache_creation: 1_000_000,
            cache_read: 1_000_000,
            output: 1_000_000,
        };
        let rates = ArchitectRates {
            input_per_mtok: 5.0,
            cache_creation_per_mtok: 6.25,
            cache_read_per_mtok: 0.5,
            output_per_mtok: 25.0,
        };
        let cost = tokens.cost(&rates);
        assert!((cost - 36.75).abs() < f64::EPSILON);
    }

    #[test]
    fn architect_tokens_default_is_zero_cost() {
        let tokens = ArchitectTokens::default();
        let rates = ArchitectRates {
            input_per_mtok: 5.0,
            cache_creation_per_mtok: 6.25,
            cache_read_per_mtok: 0.5,
            output_per_mtok: 25.0,
        };
        assert_eq!(tokens.cost(&rates), 0.0);
    }

    #[test]
    fn fold_activities_enriched_copy_wins() {
        let zero = ArchitectActivity {
            record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
            ts: 1_000,
            phase_doc_path: None,
            phase_id: "p1".to_string(),
            project_id: None,
            milestone_id: None,
            activity: "assist".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        let enriched = ArchitectActivity {
            record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
            ts: 1_000,
            phase_doc_path: None,
            phase_id: "p1".to_string(),
            project_id: None,
            milestone_id: None,
            activity: "assist".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens {
                input: 500,
                cache_creation: 0,
                cache_read: 0,
                output: 0,
            },
        };
        // Enriched second → wins
        let out = fold_activities(vec![zero.clone(), enriched.clone()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tokens.input, 500);
        // Reversed: zero second → wins (proves order-based, not max)
        let out2 = fold_activities(vec![enriched, zero]);
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].tokens.input, 0);
    }

    #[test]
    fn fold_activities_distinct_ts_keeps_both() {
        let a = ArchitectActivity {
            record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
            ts: 1_000,
            phase_doc_path: None,
            phase_id: "p1".to_string(),
            project_id: None,
            milestone_id: None,
            activity: "assist".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        let b = ArchitectActivity {
            record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
            ts: 2_000,
            phase_doc_path: None,
            phase_id: "p1".to_string(),
            project_id: None,
            milestone_id: None,
            activity: "assist".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        let out = fold_activities(vec![a, b]);
        assert_eq!(out.len(), 2, "different ts → distinct identity");
    }

    #[test]
    fn fold_activities_distinct_activity_keeps_both() {
        let a = ArchitectActivity {
            record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
            ts: 1_000,
            phase_doc_path: None,
            phase_id: "p1".to_string(),
            project_id: None,
            milestone_id: None,
            activity: "draft".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        let b = ArchitectActivity {
            record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
            ts: 1_000,
            phase_doc_path: None,
            phase_id: "p1".to_string(),
            project_id: None,
            milestone_id: None,
            activity: "review".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        let out = fold_activities(vec![a, b]);
        assert_eq!(out.len(), 2, "different activity → distinct identity");
    }

    #[test]
    fn architect_activity_roundtrips_nested_tokens() {
        let dir = TempDir::new().unwrap();
        let activity = ArchitectActivity {
            record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
            ts: 1_000,
            phase_doc_path: Some("/docs/phase-02.md".to_string()),
            phase_id: "phase-02".to_string(),
            project_id: Some("proj-a".to_string()),
            milestone_id: Some("M27-autonomous-escalation-loop".to_string()),
            activity: "assist".to_string(),
            outcome: Some("complete".to_string()),
            model: Some("claude-opus-4-8".to_string()),
            tokens: ArchitectTokens {
                input: 10_000,
                cache_creation: 5_000,
                cache_read: 3_000,
                output: 2_000,
            },
        };
        append_architect_activity(dir.path(), &activity).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let activities = read_architect_activities(&path).unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].tokens, activity.tokens);
    }

    #[test]
    fn current_activity_line_without_tokens_defaults_zero() {
        // M35: a legacy (unversioned) line is now excluded entirely by the
        // schema_version gate — read_architect_activities_version_gates pins
        // that. This test instead covers the surviving behavior: a *current*
        // (schema_version-stamped) record that omits the optional `tokens`
        // object still deserializes, defaulting tokens to zero.
        let dir = TempDir::new().unwrap();
        let line = r#"{"schema_version":1,"record":"architect_activity","ts":1,"phase_doc_path":null,"phase_id":"p1","project_id":null,"milestone_id":null,"activity":"assist","outcome":null,"model":null}"#;
        std::fs::write(dir.path().join("phase_runs.jsonl"), format!("{line}\n")).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let activities = read_architect_activities(&path).unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].tokens, ArchitectTokens::default());
    }

    #[test]
    fn phase_run_line_without_gen_time_s_parses_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        // Pre-phase-02 line: has schema_version:1 but no gen_time_s
        let line = r#"{"schema_version":1,"ts":1,"model":"t","generation_params":{},"phase_id":"p","tags":["rust"],"status":"c","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{},"warnings":null,"bugs_filed":null}"#;
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let results = read(&path).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].gen_time_s, 0.0);
    }

    // ---- ArchitectLedger roundtrip ----

    #[test]
    fn architect_ledger_roundtrips_through_store() {
        let dir = TempDir::new().unwrap();
        let telemetry_dir = dir.path().join("telemetry");
        let ledger = ArchitectLedger {
            record: ARCHITECT_LEDGER_RECORD_TAG.to_string(),
            project_id: Some("proj".to_string()),
            session_id: "session-1".to_string(),
            model: "claude-opus-4-8".to_string(),
            skill: "rexymcp:dispatch".to_string(),
            tokens: ArchitectTokens {
                input: 1000,
                cache_creation: 2000,
                cache_read: 3000,
                output: 400,
            },
            cache_creation_5m: 500,
            cache_creation_1h: 1500,
            messages: 3,
            last_ts: 1_717_000_000_000,
        };
        append_architect_ledger(&telemetry_dir, &ledger).unwrap();
        let path = telemetry_dir.join("phase_runs.jsonl");
        let results = read_architect_ledger(&path).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], ledger);
    }

    #[test]
    fn read_ledger_ignores_other_record_types() {
        let dir = TempDir::new().unwrap();
        let telemetry_dir = dir.path().join("telemetry");

        // Write a PhaseRun (via sample)
        let sample = sample();
        append(&telemetry_dir, &sample).unwrap();

        // Write an ArchitectActivity
        let activity = ArchitectActivity {
            record: ARCHITECT_ACTIVITY_RECORD_TAG.to_string(),
            ts: 1_717_000_000_000,
            phase_doc_path: None,
            phase_id: "p1".to_string(),
            project_id: Some("proj".to_string()),
            milestone_id: None,
            activity: "review".to_string(),
            outcome: None,
            model: None,
            tokens: ArchitectTokens::default(),
        };
        append_architect_activity(&telemetry_dir, &activity).unwrap();

        // Write an ArchitectLedger
        let ledger = ArchitectLedger {
            record: ARCHITECT_LEDGER_RECORD_TAG.to_string(),
            project_id: Some("proj".to_string()),
            session_id: "s1".to_string(),
            model: "claude-opus-4-8".to_string(),
            skill: "dispatch".to_string(),
            tokens: ArchitectTokens {
                input: 100,
                cache_creation: 200,
                cache_read: 300,
                output: 400,
            },
            cache_creation_5m: 100,
            cache_creation_1h: 100,
            messages: 1,
            last_ts: 1_717_000_000_000,
        };
        append_architect_ledger(&telemetry_dir, &ledger).unwrap();

        let path = telemetry_dir.join("phase_runs.jsonl");

        // read_architect_ledger returns exactly the one ledger
        let ledgers = read_architect_ledger(&path).unwrap();
        assert_eq!(ledgers.len(), 1);
        assert_eq!(ledgers[0], ledger);

        // read_architect_activities still returns exactly the one activity
        let activities = read_architect_activities(&path).unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0], activity);
    }

    #[test]
    fn fold_ledger_keeps_last_per_key() {
        let l1 = ArchitectLedger {
            record: ARCHITECT_LEDGER_RECORD_TAG.to_string(),
            project_id: Some("proj".to_string()),
            session_id: "s1".to_string(),
            model: "m1".to_string(),
            skill: "skill_a".to_string(),
            tokens: ArchitectTokens {
                input: 100,
                cache_creation: 0,
                cache_read: 0,
                output: 0,
            },
            cache_creation_5m: 0,
            cache_creation_1h: 0,
            messages: 1,
            last_ts: 100,
        };
        let l2 = ArchitectLedger {
            record: ARCHITECT_LEDGER_RECORD_TAG.to_string(),
            project_id: Some("proj".to_string()),
            session_id: "s1".to_string(),
            model: "m1".to_string(),
            skill: "skill_a".to_string(),
            tokens: ArchitectTokens {
                input: 200,
                cache_creation: 0,
                cache_read: 0,
                output: 0,
            },
            cache_creation_5m: 0,
            cache_creation_1h: 0,
            messages: 2,
            last_ts: 200,
        };
        let l3 = ArchitectLedger {
            record: ARCHITECT_LEDGER_RECORD_TAG.to_string(),
            project_id: Some("proj".to_string()),
            session_id: "s1".to_string(),
            model: "m1".to_string(),
            skill: "skill_b".to_string(),
            tokens: ArchitectTokens {
                input: 300,
                cache_creation: 0,
                cache_read: 0,
                output: 0,
            },
            cache_creation_5m: 0,
            cache_creation_1h: 0,
            messages: 3,
            last_ts: 300,
        };

        let folded = fold_ledger(vec![l1, l2, l3]);
        assert_eq!(folded.len(), 2);
        // l2 (last for skill_a) replaces l1
        assert_eq!(folded[0].tokens.input, 200);
        assert_eq!(folded[0].skill, "skill_a");
        // l3 stays separate
        assert_eq!(folded[1].tokens.input, 300);
        assert_eq!(folded[1].skill, "skill_b");
    }

    #[test]
    fn architect_ledger_cost_prices_5m_and_1h_split() {
        let l = ArchitectLedger {
            record: ARCHITECT_LEDGER_RECORD_TAG.to_string(),
            project_id: Some("proj".to_string()),
            session_id: "s1".to_string(),
            model: "claude-opus-4-8".to_string(),
            skill: "dispatch".to_string(),
            tokens: ArchitectTokens {
                input: 1_000_000,
                cache_creation: 0,
                cache_read: 1_000_000,
                output: 1_000_000,
            },
            cache_creation_5m: 1_000_000,
            cache_creation_1h: 1_000_000,
            messages: 5,
            last_ts: 1_717_000_000_000,
        };
        // input: 1M * $5.00 = $5.00
        // output: 1M * $25.00 = $25.00
        // cache_read: 1M * $5.00 * 0.1 = $0.50
        // cache_creation_5m: 1M * $5.00 * 1.25 = $6.25
        // cache_creation_1h: 1M * $5.00 * 2.0 = $10.00
        // total = $46.75
        let cost = l.cost(5.0, 25.0);
        let expected = 46.75;
        assert!(
            (cost - expected).abs() < 1e-9,
            "got {cost}, expected {expected}"
        );
    }

    #[test]
    fn architect_ledger_cost_ignores_total_cache_creation() {
        let l = ArchitectLedger {
            record: ARCHITECT_LEDGER_RECORD_TAG.to_string(),
            project_id: Some("proj".to_string()),
            session_id: "s1".to_string(),
            model: "claude-opus-4-8".to_string(),
            skill: "dispatch".to_string(),
            tokens: ArchitectTokens {
                input: 1_000_000,
                cache_creation: 500_000, // inconsistent — should be ignored by cost()
                cache_read: 0,
                output: 0,
            },
            cache_creation_5m: 1_000_000,
            cache_creation_1h: 1_000_000,
            messages: 1,
            last_ts: 1_717_000_000_000,
        };
        // Only the split fields are priced:
        // input: 1M * $5.00 = $5.00
        // cache_creation_5m: 1M * $5.00 * 1.25 = $6.25
        // cache_creation_1h: 1M * $5.00 * 2.0 = $10.00
        // total = $21.25
        let cost = l.cost(5.0, 25.0);
        let expected = 21.25;
        assert!(
            (cost - expected).abs() < 1e-9,
            "got {cost}, expected {expected}"
        );
    }
}
