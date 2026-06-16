//! Cross-project `PhaseRun` telemetry — one summary record per `execute_phase`,
//! appended as JSONL to a single global store (not per-repo). The durable
//! substrate for the M7 model scorecard (`model × tag`) and project review
//! (`milestone × phase`). The executor fills the objective fields at phase end;
//! the architect's review fills the supervision fields (`bugs_filed`,
//! `bounces_to_approval`, `architect_verdict`, `warnings`) later.

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
/// only `Default` (the `ContextEfficiency` precedent). Only `tier` is populated
/// in M20 phase-02 — the configured executor tier, available from `[executor]
/// tier`. `doc_level` is wired in M22 (phase-doc detail level L1/L2/L3 → 1/2/3);
/// `escalation_count` and the two `architect_*_tokens` are wired in M21 when the
/// mid-phase Architect-assist loop fires. Default (tier `None`, levels `None`,
/// counts `0`) for legacy records and every run that did not escalate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TierTelemetry {
    /// Configured executor capability tier (`[executor] tier`); `None` when the
    /// project has not run `rexymcp calibrate`.
    pub tier: Option<Tier>,
    /// Phase-doc detail level (1/2/3). `None` until M22 wires doc levels.
    pub doc_level: Option<u8>,
    /// Number of mid-phase Architect assists that fired this run. `0` until M21.
    pub escalation_count: u32,
    /// Architect input tokens spent on assists this run. `0` until M21.
    pub architect_input_tokens: u64,
    /// Architect output tokens spent on assists this run. `0` until M21.
    pub architect_output_tokens: u64,
}

/// One per-phase metrics row. Objective fields are filled by the executor; the
/// supervision fields are filled by the architect at review (M7).
/// (No `PartialEq` — `TokenBreakdown` doesn't implement it; compare via JSON.)
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// M20 tier/cost instrumentation (tier, doc_level, escalation cost). Default
    /// for legacy records and non-escalating runs.
    #[serde(default)]
    pub tier_telemetry: TierTelemetry,
}

/// Append one `PhaseRun` as a JSON line to `<telemetry_dir>/phase_runs.jsonl`,
/// creating the directory if needed. Returns the file path.
pub fn append(telemetry_dir: &Path, run: &PhaseRun) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let line = serde_json::to_string(run).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// Read all `PhaseRun` records from a store file (skips blank/corrupt lines).
pub fn read(path: &Path) -> std::io::Result<Vec<PhaseRun>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<PhaseRun>(l).ok())
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
/// (the same store as `PhaseRun`). Returns the file path.
pub fn append_review(telemetry_dir: &Path, review: &PhaseReview) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let line = serde_json::to_string(review).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// Read all `PhaseReview` records from a store file. Lines that are `PhaseRun`
/// records (or anything without `record == "review"`) are skipped.
pub fn read_reviews(path: &Path) -> std::io::Result<Vec<PhaseReview>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<PhaseReview>(l).ok())
        .filter(|r| r.record == REVIEW_RECORD_TAG)
        .collect())
}

/// An append-only record of a single mid-phase Architect assist, appended to
/// `phase_runs.jsonl` alongside `PhaseRun` and `PhaseReview`. The `record`
/// discriminator (`"escalation"`) keeps the three readers from confusing the
/// line types. **No code writes one in M20** — the producer is wired in M21 when
/// the SMALL-tier escalation loop fires; this phase defines the schema and the
/// store API only (the `PhaseReview` substrate precedent from M18 phase-01).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EscalationEvent {
    /// Literal discriminator. Always `"escalation"`. `#[serde(default)]` so a
    /// `PhaseRun` line (no `record` field) deserializes to `""` here and is
    /// filtered out by `read_escalations`.
    #[serde(default)]
    pub record: String,
    pub ts: u64,
    /// Identity of the phase that escalated. Prefer `phase_doc_path`; `phase_id`
    /// + `project_id` are the fallback key (mirrors `PhaseReview`).
    #[serde(default)]
    pub phase_doc_path: Option<String>,
    pub phase_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    /// 1-based index of this assist within the phase (1st assist = 1).
    pub assist_index: u32,
    /// Architect model that produced the assist (e.g. `"claude-opus-4-8"`).
    #[serde(default)]
    pub model: Option<String>,
    /// Architect input tokens spent on this single assist.
    pub architect_input_tokens: u64,
    /// Architect output tokens spent on this single assist.
    pub architect_output_tokens: u64,
}

/// The literal value of `EscalationEvent.record`. Use everywhere instead of a
/// bare string so the discriminator is single-sourced.
pub const ESCALATION_RECORD_TAG: &str = "escalation";

/// Append one `EscalationEvent` as a JSON line to
/// `<telemetry_dir>/phase_runs.jsonl` (the same store as `PhaseRun`). Returns
/// the file path.
pub fn append_escalation(
    telemetry_dir: &Path,
    event: &EscalationEvent,
) -> std::io::Result<PathBuf> {
    use std::io::Write;

    std::fs::create_dir_all(telemetry_dir)?;
    let path = telemetry_dir.join("phase_runs.jsonl");
    let line = serde_json::to_string(event).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// Read all `EscalationEvent` records from a store file. Lines that are
/// `PhaseRun` or `PhaseReview` records (or anything without
/// `record == "escalation"`) are skipped.
pub fn read_escalations(path: &Path) -> std::io::Result<Vec<EscalationEvent>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<EscalationEvent>(l).ok())
        .filter(|e| e.record == ESCALATION_RECORD_TAG)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::sessions::event::{SessionEvent, SessionRecord};
    use tempfile::TempDir;

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
    fn phase_run_tier_telemetry_round_trips() {
        let mut run = sample();
        run.tier_telemetry = TierTelemetry {
            tier: Some(Tier::Medium),
            doc_level: Some(2),
            escalation_count: 1,
            architect_input_tokens: 1000,
            architect_output_tokens: 200,
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
    fn escalation_event_round_trips() {
        let dir = TempDir::new().unwrap();
        let event = EscalationEvent {
            record: ESCALATION_RECORD_TAG.to_string(),
            ts: 1_000,
            phase_doc_path: Some("/docs/phase-02.md".to_string()),
            phase_id: "phase-02".to_string(),
            project_id: Some("proj-a".to_string()),
            assist_index: 1,
            model: Some("claude-opus-4-8".to_string()),
            architect_input_tokens: 1500,
            architect_output_tokens: 300,
        };
        append_escalation(dir.path(), &event).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let events = read_escalations(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], event);
    }

    #[test]
    fn read_escalations_excludes_run_lines() {
        // A PhaseRun line must not be read as an EscalationEvent.
        let dir = TempDir::new().unwrap();
        append(dir.path(), &sample()).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let events = read_escalations(&path).unwrap();
        assert!(
            events.is_empty(),
            "PhaseRun lines must not be read as escalations"
        );
    }

    #[test]
    fn read_escalations_excludes_review_by_discriminator() {
        // A PhaseReview line parses far enough to reach the discriminator filter
        // (it has ts/phase_id and no required escalation-only field would block it
        // only if absent) — assert the `record != "escalation"` filter is what
        // excludes it. This pins the discriminator as load-bearing (M18 bug-01-1).
        let dir = TempDir::new().unwrap();
        let review = PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 2_000,
            phase_doc_path: Some("/docs/phase-02.md".to_string()),
            phase_id: "phase-02".to_string(),
            project_id: None,
            architect_verdict: "approved".to_string(),
            bounces_to_approval: None,
            bugs_filed: None,
            warnings: None,
            failure_class: vec!["none".to_string()],
        };
        append_review(dir.path(), &review).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let events = read_escalations(&path).unwrap();
        assert!(
            events.is_empty(),
            "review lines must be excluded from escalations"
        );
    }

    #[test]
    fn read_skips_escalation_lines() {
        // The existing PhaseRun reader must not pick up escalation lines.
        let dir = TempDir::new().unwrap();
        append(dir.path(), &sample()).unwrap();
        let event = EscalationEvent {
            record: ESCALATION_RECORD_TAG.to_string(),
            ts: 1,
            phase_doc_path: None,
            phase_id: "p".to_string(),
            project_id: None,
            assist_index: 1,
            model: None,
            architect_input_tokens: 1,
            architect_output_tokens: 1,
        };
        append_escalation(dir.path(), &event).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let runs = read(&path).unwrap();
        assert_eq!(runs.len(), 1, "read() must skip the escalation line");
    }

    #[test]
    fn read_reviews_skips_escalation_lines() {
        let dir = TempDir::new().unwrap();
        let event = EscalationEvent {
            record: ESCALATION_RECORD_TAG.to_string(),
            ts: 1,
            phase_doc_path: None,
            phase_id: "p".to_string(),
            project_id: None,
            assist_index: 1,
            model: None,
            architect_input_tokens: 1,
            architect_output_tokens: 1,
        };
        append_escalation(dir.path(), &event).unwrap();
        let path = dir.path().join("phase_runs.jsonl");
        let reviews = read_reviews(&path).unwrap();
        assert!(
            reviews.is_empty(),
            "read_reviews() must skip the escalation line"
        );
    }
}
