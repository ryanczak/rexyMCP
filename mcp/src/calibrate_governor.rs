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
    /// Author-attributed verifier error count per `Verify` event, in order.
    verifier_error_counts: Vec<usize>,
    /// Per `Completion` event: whether it was blank/think-only. NOTE: this misses
    /// truncation-driven empties (`finish_reason == length` is not logged), so it
    /// is a lower bound on the loop's empty-completion counter.
    completion_empty: Vec<bool>,
}

/// Signal extractor seam — extensible for 06b.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    /// Distinct normalized targets per full novelty window — many samples/run.
    NoveltyDistinct,
    /// Longest consecutive read-only run in the run — one sample/run.
    MaxReadOnlyRun,
    /// Longest run of consecutive identical (tool, arguments).
    IdenticalRun,
    /// Minimum distinct (tool, arguments) over any sliding window.
    OscillationMinDistinct,
    /// Longest streak of consecutive non-decreasing positive author-error counts.
    VerifierPersistenceRun,
    /// Longest run of consecutive empty completions (lower bound — misses truncation).
    EmptyCompletionRun,
}

impl Signal {
    fn label(self) -> &'static str {
        match self {
            Signal::NoveltyDistinct => "novelty_distinct_targets",
            Signal::MaxReadOnlyRun => "max_read_only_run",
            Signal::IdenticalRun => "identical_run",
            Signal::OscillationMinDistinct => "oscillation_min_distinct",
            Signal::VerifierPersistenceRun => "verifier_persistence_run",
            Signal::EmptyCompletionRun => "empty_completion_run",
        }
    }

    /// Extract this signal's raw samples from one replayed run.
    fn samples(self, run_: &RunReplay, novelty_window: usize) -> Vec<usize> {
        match self {
            Signal::NoveltyDistinct => {
                // Replay turn-by-turn: measure_novelty over the growing history,
                // collecting distinct_targets at every full-window measurement.
                let mut deque: VecDeque<ToolCallSnapshot> = VecDeque::new();
                let mut out = Vec::new();
                for c in &run_.tool_calls {
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
                for c in &run_.tool_calls {
                    if tools::mutates_files(&c.tool) {
                        run = 0;
                    } else {
                        run += 1;
                        max = max.max(run);
                    }
                }
                vec![max]
            }
            Signal::IdenticalRun => {
                let mut max = 0usize;
                let mut run = 0usize;
                let mut prev: Option<&ToolCallSnapshot> = None;
                for c in &run_.tool_calls {
                    let same = prev.is_some_and(|p| p.tool == c.tool && p.arguments == c.arguments);
                    run = if same { run + 1 } else { 1 };
                    max = max.max(run);
                    prev = Some(c);
                }
                if run_.tool_calls.is_empty() {
                    vec![]
                } else {
                    vec![max]
                }
            }
            Signal::OscillationMinDistinct => {
                let calls = &run_.tool_calls;
                if calls.len() < OSCILLATION_WINDOW {
                    return vec![];
                }
                let mut min = usize::MAX;
                for start in 0..=calls.len() - OSCILLATION_WINDOW {
                    let window = &calls[start..start + OSCILLATION_WINDOW];
                    let mut distinct: Vec<(&str, &serde_json::Value)> = Vec::new();
                    for c in window {
                        let key = (c.tool.as_str(), &c.arguments);
                        if !distinct.iter().any(|(t, a)| *t == key.0 && *a == key.1) {
                            distinct.push(key);
                        }
                    }
                    min = min.min(distinct.len());
                }
                vec![min]
            }
            Signal::VerifierPersistenceRun => {
                if run_.verifier_error_counts.is_empty() {
                    return vec![];
                }
                let mut max = 0usize;
                let mut run = 0usize;
                let mut prev = 0usize;
                for &c in &run_.verifier_error_counts {
                    run = if c == 0 {
                        0
                    } else if run == 0 || c >= prev {
                        run + 1
                    } else {
                        1 // positive but decreased → a fresh streak of length 1
                    };
                    max = max.max(run);
                    prev = c;
                }
                vec![max]
            }
            Signal::EmptyCompletionRun => {
                let mut max = 0usize;
                let mut run = 0usize;
                for &empty in &run_.completion_empty {
                    run = if empty { run + 1 } else { 0 };
                    max = max.max(run);
                }
                vec![max]
            }
        }
    }
}

const OSCILLATION_WINDOW: usize = 8;

const SIGNALS: &[Signal] = &[
    Signal::NoveltyDistinct,
    Signal::MaxReadOnlyRun,
    Signal::IdenticalRun,
    Signal::OscillationMinDistinct,
    Signal::VerifierPersistenceRun,
    Signal::EmptyCompletionRun,
];

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
    runs: usize,
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
    let signals: &[&str] = &[
        "novelty_distinct_targets",
        "max_read_only_run",
        "identical_run",
        "oscillation_min_distinct",
        "verifier_persistence_run",
        "empty_completion_run",
    ];
    for signal in signals {
        let signal_rows: Vec<_> = rows.iter().filter(|r| r.signal == *signal).collect();
        if signal_rows.is_empty() {
            continue;
        }
        let header = if *signal == "empty_completion_run" {
            "signal: empty_completion_run  (lower bound — excludes length-truncated turns)"
        } else {
            &format!("signal: {}", signal)
        };
        lines.push(header.to_string());
        lines.push("MODEL  OUTCOME  RUNS  N  P50  P90  P99".to_string());
        for row in signal_rows {
            lines.push(format!(
                "{:<8} {:<10} {:>4}  {:>4}  {:>4}  {:>4}  {:>4}",
                row.model, row.outcome, row.runs, row.n, row.p50, row.p90, row.p99
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

/// Accumulator for per-cell aggregation: tracks runs and samples separately.
#[derive(Default)]
struct CellAccum {
    runs: usize,
    samples: Vec<usize>,
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
        // Aggregate: collect per-cell data (runs + samples) per (model, outcome)
        let mut by_model_outcome: HashMap<(String, String), CellAccum> = HashMap::new();

        for replay in &all_replays {
            let samples = signal.samples(replay, novelty_window);
            if samples.is_empty() {
                continue;
            }
            let key = (replay.model.clone(), replay.outcome.clone());
            let cell = by_model_outcome.entry(key).or_default();
            cell.runs += 1;
            cell.samples.extend(samples);
        }

        // Global rows (all models, by outcome) — compute from by_model_outcome
        let mut global_by_outcome: HashMap<String, CellAccum> = HashMap::new();
        for ((_, outcome), cell) in &by_model_outcome {
            let entry = global_by_outcome.entry(outcome.clone()).or_default();
            entry.runs += cell.runs;
            entry.samples.extend(&cell.samples);
        }

        for (outcome, cell) in global_by_outcome {
            let mut sorted = cell.samples;
            sorted.sort();
            let n = sorted.len();
            if n == 0 {
                continue;
            }
            rows.push(ReportRow {
                signal: signal.label().to_string(),
                model: "(all)".to_string(),
                outcome,
                runs: cell.runs,
                n,
                p50: percentile(&sorted, 0.5),
                p90: percentile(&sorted, 0.9),
                p99: percentile(&sorted, 0.99),
            });
        }

        // Per-model rows (filtered by min_runs on RUN count, not sample count)
        for ((model, outcome), cell) in by_model_outcome {
            if cell.runs < min_runs {
                continue;
            }
            let mut sorted = cell.samples;
            sorted.sort();
            let n = sorted.len();
            rows.push(ReportRow {
                signal: signal.label().to_string(),
                model,
                outcome,
                runs: cell.runs,
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
    let mut verifier_error_counts = Vec::new();
    let mut completion_empty = Vec::new();
    for rec in records {
        match &rec.event {
            SessionEvent::SessionStart { model: m, .. } => model = m.clone(),
            SessionEvent::SessionEnd { status, .. } => outcome = status.clone(),
            SessionEvent::Parsed { tool_call } => tool_calls.push(ToolCallSnapshot {
                tool: tool_call.name.clone(),
                arguments: tool_call.arguments.clone(),
                succeeded: true, // the 06a stall signals key on tool+args, not success
            }),
            SessionEvent::Verify { diagnostics } => {
                verifier_error_counts.push(diagnostics.len());
            }
            SessionEvent::Completion { raw } => {
                completion_empty.push(
                    rexymcp_executor::parser::strip_think_blocks(raw)
                        .trim()
                        .is_empty(),
                );
            }
            _ => {}
        }
    }
    RunReplay {
        model,
        outcome,
        tool_calls,
        verifier_error_counts,
        completion_empty,
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
        let run = RunReplay {
            model: "test".into(),
            outcome: "complete".into(),
            tool_calls: calls,
            verifier_error_counts: Vec::new(),
            completion_empty: Vec::new(),
        };
        let samples = Signal::NoveltyDistinct.samples(&run, 24);
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
        let run = RunReplay {
            model: "test".into(),
            outcome: "complete".into(),
            tool_calls: calls,
            verifier_error_counts: Vec::new(),
            completion_empty: Vec::new(),
        };
        let samples = Signal::MaxReadOnlyRun.samples(&run, 24);
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
    fn one_run_many_samples_dropped_by_min_runs() {
        // A single run with enough calls to produce many novelty samples
        // should be dropped by --min-runs 3 because it is only 1 run.
        let tmp = TempDir::new().unwrap();
        let calls: Vec<(String, serde_json::Value)> = (0..30)
            .map(|i| {
                (
                    "read_file".into(),
                    serde_json::json!({"path": format!("file{}.rs", i)}),
                )
            })
            .collect();
        make_session_file(tmp.path(), "big", "model_x", "complete", &calls);

        let args = CalibrateGovernorArgs {
            sessions_dir: tmp.path(),
            model_filter: None,
            novelty_window: 5,
            min_runs: 3,
            json: false,
        };
        let out = run(&args);
        // model_x has 1 run with many novelty samples — should be dropped from per-model
        // but its samples should still feed the (all) global row
        assert!(
            !out.contains("model_x"),
            "model_x should be dropped (1 run < min_runs=3): {out}"
        );
        // The global row should still exist (samples from model_x feed it)
        assert!(
            out.contains("(all)"),
            "global row should include model_x samples: {out}"
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
        let run = RunReplay {
            model: "test".into(),
            outcome: "complete".into(),
            tool_calls: calls,
            verifier_error_counts: Vec::new(),
            completion_empty: Vec::new(),
        };
        let samples = Signal::NoveltyDistinct.samples(&run, 24);
        // With only 2 calls and window=24, measure_novelty returns None until window is reached
        assert!(
            samples.is_empty(),
            "expected no samples for short sequence: {samples:?}"
        );
    }

    // --- Tests for the four new signal extractors (06b) ---

    /// Helper: create a RunReplay with Verify and Completion events.
    fn make_replay_with_verify_and_completion(
        tool_calls: Vec<ToolCallSnapshot>,
        verifier_error_counts: Vec<usize>,
        completion_empty: Vec<bool>,
    ) -> RunReplay {
        RunReplay {
            model: "test".into(),
            outcome: "complete".into(),
            tool_calls,
            verifier_error_counts,
            completion_empty,
        }
    }

    #[test]
    fn identical_run_counts_longest_consecutive_identical() {
        // Sequence: read a, read a, read b, read b, read b -> longest run = 3 (the b's)
        let calls = vec![
            ToolCallSnapshot {
                tool: "read_file".into(),
                arguments: serde_json::json!({"path": "a.rs"}),
                succeeded: true,
            },
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
                tool: "read_file".into(),
                arguments: serde_json::json!({"path": "b.rs"}),
                succeeded: true,
            },
            ToolCallSnapshot {
                tool: "read_file".into(),
                arguments: serde_json::json!({"path": "b.rs"}),
                succeeded: true,
            },
        ];
        let run = make_replay_with_verify_and_completion(calls, Vec::new(), Vec::new());
        let samples = Signal::IdenticalRun.samples(&run, 24);
        assert_eq!(samples, vec![3], "expected longest run of 3: {samples:?}");
    }

    #[test]
    fn identical_run_returns_no_sample_for_empty_calls() {
        let run = make_replay_with_verify_and_completion(Vec::new(), Vec::new(), Vec::new());
        let samples = Signal::IdenticalRun.samples(&run, 24);
        assert!(
            samples.is_empty(),
            "expected no sample for empty calls: {samples:?}"
        );
    }

    #[test]
    fn oscillation_min_distinct_no_sample_below_window() {
        // A run shorter than OSCILLATION_WINDOW (8) -> no sample
        let calls = vec![
            ToolCallSnapshot {
                tool: "read_file".into(),
                arguments: serde_json::json!({"path": "a.rs"}),
                succeeded: true,
            },
            ToolCallSnapshot {
                tool: "patch".into(),
                arguments: serde_json::json!({"path": "a.rs"}),
                succeeded: true,
            },
        ];
        let run = make_replay_with_verify_and_completion(calls, Vec::new(), Vec::new());
        let samples = Signal::OscillationMinDistinct.samples(&run, 24);
        assert!(
            samples.is_empty(),
            "expected no sample for 2 calls < OSCILLATION_WINDOW=8: {samples:?}"
        );
    }

    #[test]
    fn oscillation_min_distinct_finds_tightest_window() {
        // Build a sequence with an A,B,A,B stretch inside a longer varied run.
        // We need at least OSCILLATION_WINDOW calls.
        // Calls: A,B,A,B,A,B,A,B (8 calls, 2 distinct per window)
        let a = serde_json::json!({"path": "a.rs"});
        let b = serde_json::json!({"path": "b.rs"});
        let calls: Vec<ToolCallSnapshot> = (0..8)
            .map(|i| {
                let (tool, args) = if i % 2 == 0 {
                    ("read_file".into(), a.clone())
                } else {
                    ("patch".into(), b.clone())
                };
                ToolCallSnapshot {
                    tool,
                    arguments: args,
                    succeeded: true,
                }
            })
            .collect();
        let run = make_replay_with_verify_and_completion(calls, Vec::new(), Vec::new());
        let samples = Signal::OscillationMinDistinct.samples(&run, 24);
        assert_eq!(
            samples,
            vec![2],
            "expected min distinct of 2 for A,B,A,B,A,B,A,B: {samples:?}"
        );
    }

    #[test]
    fn verifier_persistence_run_matches_detector_semantics() {
        // Counts [1, 2, 2, 0, 3] -> longest non-decreasing positive streak = 3 (the 1,2,2)
        let run =
            make_replay_with_verify_and_completion(Vec::new(), vec![1, 2, 2, 0, 3], Vec::new());
        let samples = Signal::VerifierPersistenceRun.samples(&run, 24);
        assert_eq!(
            samples,
            vec![3],
            "expected longest non-decreasing positive streak of 3: {samples:?}"
        );
    }

    #[test]
    fn verifier_persistence_reset_on_decrease() {
        // Counts [2, 1] -> positive but decreased -> streak resets to 1, not 2
        let run = make_replay_with_verify_and_completion(Vec::new(), vec![2, 1], Vec::new());
        let samples = Signal::VerifierPersistenceRun.samples(&run, 24);
        assert_eq!(
            samples,
            vec![1],
            "expected streak of 1 for [2, 1] (decrease resets): {samples:?}"
        );
    }

    #[test]
    fn verifier_persistence_no_sample_without_verify_events() {
        // A run with no Verify events -> no sample
        let run = make_replay_with_verify_and_completion(
            vec![ToolCallSnapshot {
                tool: "read_file".into(),
                arguments: serde_json::json!({"path": "a.rs"}),
                succeeded: true,
            }],
            Vec::new(),
            Vec::new(),
        );
        let samples = Signal::VerifierPersistenceRun.samples(&run, 24);
        assert!(
            samples.is_empty(),
            "expected no sample without Verify events: {samples:?}"
        );
    }

    #[test]
    fn empty_completion_run_counts_consecutive_blanks() {
        // Completions ["hi", "", "", "x"] -> longest empty run = 2
        let run = make_replay_with_verify_and_completion(
            Vec::new(),
            Vec::new(),
            vec![false, true, true, false],
        );
        let samples = Signal::EmptyCompletionRun.samples(&run, 24);
        assert_eq!(
            samples,
            vec![2],
            "expected longest empty run of 2: {samples:?}"
        );
    }

    #[test]
    fn empty_completion_run_think_only_counts_as_empty() {
        // A think-only completion should count as empty (strip_think_blocks removes it)
        let run = make_replay_with_verify_and_completion(Vec::new(), Vec::new(), vec![true]);
        let samples = Signal::EmptyCompletionRun.samples(&run, 24);
        assert_eq!(
            samples,
            vec![1],
            "expected longest empty run of 1 for single think-only: {samples:?}"
        );
    }

    #[test]
    fn remaining_signals_appear_in_report() {
        // E2E-style: run() over a fixture dir asserts the four new signal labels are present.
        // Use make_session_file for the base events, then append Verify and Completion
        // events to the same JSONL file.
        let tmp = TempDir::new().unwrap();
        // Create a session with 8 identical calls (triggers identical_run + meets OSCILLATION_WINDOW)
        let calls: Vec<(String, serde_json::Value)> = (0..8)
            .map(|_| ("read_file".into(), serde_json::json!({"path": "a.rs"})))
            .collect();
        make_session_file(tmp.path(), "e2e", "llama-3", "complete", &calls);

        // Now append Verify and Completion events to the same JSONL file
        let log_path = tmp.path().join("session-e2e.jsonl");
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap();
        // Verify event with empty diagnostics
        let verify_record = SessionRecord {
            ts: 1000,
            turn: 1,
            event: SessionEvent::Verify {
                diagnostics: vec![],
            },
        };
        writeln!(file, "{}", serde_json::to_string(&verify_record).unwrap()).unwrap();
        // Completion event with non-empty raw
        let completion_record = SessionRecord {
            ts: 2000,
            turn: 1,
            event: SessionEvent::Completion {
                raw: "hello".into(),
            },
        };
        writeln!(
            file,
            "{}",
            serde_json::to_string(&completion_record).unwrap()
        )
        .unwrap();
        drop(file);

        let args = CalibrateGovernorArgs {
            sessions_dir: tmp.path(),
            model_filter: None,
            novelty_window: 24,
            min_runs: 0,
            json: false,
        };
        let out = run(&args);

        // All four new signals should appear in the report
        assert!(
            out.contains("identical_run"),
            "report should contain identical_run: {out}"
        );
        assert!(
            out.contains("oscillation_min_distinct"),
            "report should contain oscillation_min_distinct: {out}"
        );
        assert!(
            out.contains("verifier_persistence_run"),
            "report should contain verifier_persistence_run: {out}"
        );
        assert!(
            out.contains("empty_completion_run"),
            "report should contain empty_completion_run: {out}"
        );
    }
}
