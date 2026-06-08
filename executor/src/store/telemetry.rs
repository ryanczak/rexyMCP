//! Cross-project `PhaseRun` telemetry — one summary record per `execute_phase`,
//! appended as JSONL to a single global store (not per-repo). The durable
//! substrate for the M7 model scorecard (`model × tag`) and project review
//! (`milestone × phase`). The executor fills the objective fields at phase end;
//! the architect's review fills the supervision fields (`bugs_filed`,
//! `bounces_to_approval`, `architect_verdict`, `warnings`) later.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ai::types::TokenBreakdown;
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
}
