//! Governor calibration framework — `rexymcp calibrate-governor` subcommand.
//!
//! This module replays session logs, extracts stall-signal samples, aggregates
//! percentiles, and renders a report for human threshold calibration.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use rexymcp_executor::governor::hard_fail::{ToolCallSnapshot, measure_novelty};
use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};
use rexymcp_executor::store::sessions::jsonl::read_session_log;
use rexymcp_executor::tools;

/// One replayed run: model, terminal outcome, and the reconstructed tool-call
/// sequence the governor saw. `outcome` is the `SessionEnd.status` string
/// (`complete` / `hard_fail` / `budget_exceeded` / `cancelled`); a log with no
/// `SessionEnd` (crashed / in-flight) is labeled `"unknown"`.
struct RunReplay {
    model: String,
    outcome: String,
    tool_calls: Vec<ToolCallSnapshot>,
}

/// Signal extractor seam — extensible for 06b.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    /// Distinct normalized targets per full novelty window — many samples/run.
    NoveltyDistinct,
    /// Longest consecutive read-only run in the run — one sample/run.
    MaxReadOnlyRun,
}

impl Signal {
    fn label(self) -> &'static str {
        match self {
            Signal::NoveltyDistinct => "novelty_distinct_targets",
            Signal::MaxReadOnlyRun => "max_read_only_run",
        }
    }

    /// Extract this signal's raw samples from one run's tool-call sequence.
    fn samples(self, calls: &[ToolCallSnapshot], novelty_window: usize) -> Vec<usize> {
        match self {
            Signal::NoveltyDistinct => {
                // Replay turn-by-turn: measure_novelty over the growing history,
                // collecting distinct_targets at every full-window measurement.
                let mut deque: VecDeque<ToolCallSnapshot> = VecDeque::new();
                let mut out = Vec::new();
                for c in calls {
                    deque.push_back(c.clone());
                    if let Some(m) = measure_novelty(&deque, novelty_window) {
                        out.push(m.distinct_targets);
                    }
                }
                out
            }
            Signal::MaxReadOnlyRun => {
                let mut max = 0usize;
                let mut run = 0usize;
                for c in calls {
                    if tools::mutates_files(&c.tool) {
                        run = 0;
                    } else {
                        run += 1;
                        max = max.max(run);
                    }
                }
                vec![max]
            }
        }
    }
}

const SIGNALS: &[Signal] = &[Signal::NoveltyDistinct, Signal::MaxReadOnlyRun];

/// Nearest-rank percentile of a sorted slice. `p` in 0.0..=1.0. Empty → 0.
fn percentile(sorted: &[usize], p: f64) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

/// Deliverable report row for JSON output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReportRow {
    signal: String,
    model: String,
    outcome: String,
    n: usize,
    p50: usize,
    p90: usize,
    p99: usize,
}

/// Format the calibration report as a text table.
fn format_report(rows: &[ReportRow]) -> String {
    if rows.is_empty() {
        return "(no data)".to_string();
    }

    let mut lines = Vec::new();
    for signal in &["novelty_distinct_targets", "max_read_only_run"] {
        let signal_rows: Vec<_> = rows.iter().filter(|r| r.signal == *signal).collect();
        if signal_rows.is_empty() {
            continue;
        }
        lines.push(format!("signal: {}", signal));
        lines.push("MODEL  OUTCOME  N  P50  P90  P99".to_string());
        for row in signal_rows {
            lines.push(format!(
                "{:<8} {:<10} {:>2}  {:>4}  {:>4}  {:>4}",
                row.model, row.outcome, row.n, row.p50, row.p90, row.p99
            ));
        }
        lines.push("".to_string());
    }
    lines.join("\n")
}

/// Calibration arguments.
pub struct CalibrateGovernorArgs<'a> {
    pub sessions_dir: &'a Path,
    pub model_filter: Option<&'a str>,
    pub novelty_window: usize,
    pub min_runs: usize,
    pub json: bool,
}

/// Enumerate session logs, replay, aggregate, and return the rendered report.
pub fn run(args: &CalibrateGovernorArgs<'_>) -> String {
    let sessions_dir = args.sessions_dir;
    let model_filter = args.model_filter;
    let novelty_window = args.novelty_window;
    let min_runs = args.min_runs;
    let json = args.json;

    // Enumerate session log files
    let mut files: Vec<PathBuf> = match std::fs::read_dir(sessions_dir) {
        Ok(e) => e
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();

    // Replay each session log
    let mut all_replays: Vec<RunReplay> = Vec::new();
    for file in &files {
        let records = match read_session_log(file) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let replay = replay(&records);
        if let Some(mf) = model_filter
            && replay.model != mf
        {
            continue;
        }
        all_replays.push(replay);
    }

    // Extract signal samples and aggregate by (model, outcome)
    let mut rows: Vec<ReportRow> = Vec::new();
    for signal in SIGNALS {
        // Aggregate: collect all samples per (model, outcome)
        let mut by_model_outcome: HashMap<(String, String), Vec<usize>> = HashMap::new();

        for replay in &all_replays {
            let samples = signal.samples(&replay.tool_calls, novelty_window);
            let key = (replay.model.clone(), replay.outcome.clone());
            by_model_outcome.entry(key).or_default().extend(samples);
        }

        // Global rows (all models, by outcome) — compute from by_model_outcome
        let mut global_by_outcome: HashMap<String, Vec<usize>> = HashMap::new();
        for ((_, outcome), samples) in &by_model_outcome {
            global_by_outcome
                .entry(outcome.clone())
                .or_default()
                .extend(samples);
        }

        for (outcome, samples) in global_by_outcome {
            let mut sorted = samples;
            sorted.sort();
            let n = sorted.len();
            if n == 0 {
                continue;
            }
            rows.push(ReportRow {
                signal: signal.label().to_string(),
                model: "(all)".to_string(),
                outcome,
                n,
                p50: percentile(&sorted, 0.5),
                p90: percentile(&sorted, 0.9),
                p99: percentile(&sorted, 0.99),
            });
        }

        // Per-model rows (filtered by min_runs)
        for ((model, outcome), samples) in by_model_outcome {
            let mut sorted = samples;
            sorted.sort();
            let n = sorted.len();
            if n < min_runs {
                continue;
            }
            rows.push(ReportRow {
                signal: signal.label().to_string(),
                model,
                outcome,
                n,
                p50: percentile(&sorted, 0.5),
                p90: percentile(&sorted, 0.9),
                p99: percentile(&sorted, 0.99),
            });
        }
    }

    if json {
        serde_json::to_string_pretty(&rows).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
    } else {
        format_report(&rows)
    }
}

/// Replay a single session log into a `RunReplay`.
fn replay(records: &[SessionRecord]) -> RunReplay {
    let mut model = String::from("(unknown)");
    let mut outcome = String::from("unknown");
    let mut tool_calls = Vec::new();
    for rec in records {
        match &rec.event {
            SessionEvent::SessionStart { model: m, .. } => model = m.clone(),
            SessionEvent::SessionEnd { status, .. } => outcome = status.clone(),
            SessionEvent::Parsed { tool_call } => tool_calls.push(ToolCallSnapshot {
                tool: tool_call.name.clone(),
                arguments: tool_call.arguments.clone(),
                succeeded: true, // the 06a stall signals key on tool+args, not success
            }),
            _ => {}
        }
    }
    RunReplay {
        model,
        outcome,
        tool_calls,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_session_file(
        dir: &Path,
        name: &str,
        model: &str,
        status: &str,
        tool_calls: &[(String, serde_json::Value)],
    ) {
        let records: Vec<SessionRecord> = vec![SessionRecord {
            ts: 0,
            turn: 0,
            event: SessionEvent::SessionStart {
                session_id: "s1".into(),
                model: model.into(),
                phase: "phase-01".into(),
            },
        }];
        let mut records = records;
        for (i, (tool, args)) in tool_calls.iter().enumerate() {
            records.push(SessionRecord {
                ts: i as u64,
                turn: i + 1,
                event: SessionEvent::Parsed {
                    tool_call: rexymcp_executor::parser::ToolCall {
                        name: tool.clone(),
                        arguments: args.clone(),
                        origin: rexymcp_executor::parser::Origin::Extracted {
                            format: rexymcp_executor::parser::Format::Hermes,
                        },
                    },
                },
            });
        }
        records.push(SessionRecord {
            ts: (tool_calls.len() + 1) as u64,
            turn: tool_calls.len() + 1,
            event: SessionEvent::SessionEnd {
                status: status.into(),
                turns: tool_calls.len() + 1,
            },
        });

        let mut file = std::fs::File::create(dir.join(format!("session-{}.jsonl", name))).unwrap();
        for rec in records {
            writeln!(file, "{}", serde_json::to_string(&rec).unwrap()).unwrap();
        }
    }

    fn make_session_file_no_end(dir: &Path, name: &str, model: &str) {
        let records = vec![SessionRecord {
            ts: 0,
            turn: 0,
            event: SessionEvent::SessionStart {
                session_id: "s1".into(),
                model: model.into(),
                phase: "phase-01".into(),
            },
        }];
        let mut file = std::fs::File::create(dir.join(format!("session-{}.jsonl", name))).unwrap();
        for rec in records {
            writeln!(file, "{}", serde_json::to_string(&rec).unwrap()).unwrap();
        }
    }

    #[test]
    fn replay_extracts_model_outcome_and_tool_calls() {
        let tmp = TempDir::new().unwrap();
        make_session_file(
            tmp.path(),
            "test",
            "llama-3",
            "complete",
            &[
                ("read_file".into(), serde_json::json!({"path": "a.rs"})),
                ("patch".into(), serde_json::json!({"path": "a.rs"})),
            ],
        );
        let file = tmp.path().join("session-test.jsonl");
        let records = read_session_log(&file).unwrap();
        let replay = replay(&records);
        assert_eq!(replay.model, "llama-3");
        assert_eq!(replay.outcome, "complete");
        assert_eq!(replay.tool_calls.len(), 2);
        assert_eq!(replay.tool_calls[0].tool, "read_file");
        assert_eq!(replay.tool_calls[1].tool, "patch");
    }

    #[test]
    fn run_with_no_sessionend_is_labeled_unknown() {
        let tmp = TempDir::new().unwrap();
        make_session_file_no_end(tmp.path(), "test", "llama-3");
        let file = tmp.path().join("session-test.jsonl");
        let records = read_session_log(&file).unwrap();
        let replay = replay(&records);
        assert_eq!(replay.model, "llama-3");
        assert_eq!(replay.outcome, "unknown");
    }

    #[test]
    fn novelty_distinct_samples_match_measure_novelty() {
        // Build a sequence of 30 read_file calls with distinct paths
        let mut calls = Vec::new();
        for i in 0..30 {
            calls.push(ToolCallSnapshot {
                tool: "read_file".into(),
                arguments: serde_json::json!({"path": format!("file{}.rs", i)}),
                succeeded: true,
            });
        }
        let samples = Signal::NoveltyDistinct.samples(&calls, 24);
        // After 24 calls, measure_novelty returns Some, and we get samples for calls 24..30
        assert!(!samples.is_empty(), "expected samples: {samples:?}");
        // All samples should have 24 distinct targets (each file is unique)
        assert!(
            samples.iter().all(|s| *s == 24),
            "expected all samples to be 24: {samples:?}"
        );
    }

    #[test]
    fn max_read_only_run_resets_on_mutating_call() {
        let calls = vec![
            ToolCallSnapshot {
                tool: "read_file".into(),
                arguments: serde_json::json!({"path": "a.rs"}),
                succeeded: true,
            },
            ToolCallSnapshot {
                tool: "read_file".into(),
                arguments: serde_json::json!({"path": "b.rs"}),
                succeeded: true,
            },
            ToolCallSnapshot {
                tool: "patch".into(),
                arguments: serde_json::json!({"path": "a.rs"}),
                succeeded: true,
            },
            ToolCallSnapshot {
                tool: "read_file".into(),
                arguments: serde_json::json!({"path": "c.rs"}),
                succeeded: true,
            },
        ];
        let samples = Signal::MaxReadOnlyRun.samples(&calls, 24);
        // Max read-only run is 2 (the first two read_file calls before patch)
        assert_eq!(
            samples,
            vec![2],
            "expected max read-only run of 2: {samples:?}"
        );
    }

    #[test]
    fn min_runs_drops_thin_per_model_cells_into_global() {
        let tmp = TempDir::new().unwrap();
        make_session_file(
            tmp.path(),
            "a",
            "model_a",
            "complete",
            &[("read_file".into(), serde_json::json!({"path": "a.rs"}))],
        );
        make_session_file(
            tmp.path(),
            "b",
            "model_b",
            "complete",
            &[("read_file".into(), serde_json::json!({"path": "b.rs"}))],
        );

        let args = CalibrateGovernorArgs {
            sessions_dir: tmp.path(),
            model_filter: None,
            novelty_window: 24,
            min_runs: 2,
            json: false,
        };
        let out = run(&args);
        // With min_runs=2 and only 1 run per model, per-model rows should be dropped
        // but global "(all)" rows should still appear
        assert!(out.contains("(all)"), "expected global row: {out}");
        assert!(
            !out.contains("model_a") && !out.contains("model_b"),
            "per-model rows should be dropped: {out}"
        );
    }

    #[test]
    fn empty_sessions_dir_yields_empty_report() {
        let tmp = TempDir::new().unwrap();
        let args = CalibrateGovernorArgs {
            sessions_dir: &tmp.path().join("nonexistent"),
            model_filter: None,
            novelty_window: 24,
            min_runs: 0,
            json: false,
        };
        let out = run(&args);
        assert!(out.contains("(no data)"), "{out}");
    }

    #[test]
    fn percentile_boundaries() {
        assert_eq!(percentile(&[], 0.5), 0);
        assert_eq!(percentile(&[], 0.9), 0);
        let single = vec![42];
        assert_eq!(percentile(&single, 0.5), 42);
        assert_eq!(percentile(&single, 0.9), 42);
        assert_eq!(percentile(&single, 0.99), 42);
    }

    #[test]
    fn signal_samples_novelty_distinct_uses_measure_novelty() {
        // The NoveltyDistinct signal must delegate to measure_novelty
        let calls = vec![
            ToolCallSnapshot {
                tool: "read_file".into(),
                arguments: serde_json::json!({"path": "a.rs"}),
                succeeded: true,
            },
            ToolCallSnapshot {
                tool: "read_file".into(),
                arguments: serde_json::json!({"path": "b.rs"}),
                succeeded: true,
            },
        ];
        let samples = Signal::NoveltyDistinct.samples(&calls, 24);
        // With only 2 calls and window=24, measure_novelty returns None until window is reached
        assert!(
            samples.is_empty(),
            "expected no samples for short sequence: {samples:?}"
        );
    }
}
