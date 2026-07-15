use std::collections::VecDeque;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::GovernorConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallSnapshot {
    pub tool: String,
    pub arguments: serde_json::Value,
    pub succeeded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum HardFailSignal {
    IdenticalToolCallRepetition {
        tool: String,
        consecutive_count: u32,
    },
    VerifierFailurePersistent {
        consecutive_failures: u32,
    },
    RunawayOutput {
        tool: String,
        bytes: usize,
    },
    EmptyCompletionStall {
        consecutive_count: u32,
    },
    StuckGateFeedback {
        consecutive_count: u32,
    },
    BackendError {
        message: String,
    },
    Oscillation {
        distinct_calls: usize,
        window: usize,
    },
    CumulativeOutputFlood {
        window: usize,
        bytes: usize,
    },
    NoProgressStall {
        consecutive_read_only: u32,
    },
}

impl HardFailSignal {
    pub fn describe(&self) -> String {
        match self {
            Self::IdenticalToolCallRepetition {
                tool,
                consecutive_count,
            } => {
                format!("identical {tool} call repeated {consecutive_count} times")
            }
            Self::VerifierFailurePersistent {
                consecutive_failures,
            } => {
                format!("verifier flagged errors on {consecutive_failures} consecutive turns")
            }
            Self::RunawayOutput { tool, bytes } => {
                format!("tool {tool} produced {bytes} bytes (over threshold)")
            }
            Self::EmptyCompletionStall { consecutive_count } => {
                format!("model emitted {consecutive_count} consecutive empty completions")
            }
            Self::StuckGateFeedback { consecutive_count } => {
                format!(
                    "the same gate feedback was re-injected {consecutive_count} times with no progress"
                )
            }
            Self::BackendError { message } => {
                format!("backend error: {message}")
            }
            Self::Oscillation {
                distinct_calls,
                window,
            } => {
                format!(
                    "only {distinct_calls} distinct tool calls across the last {window} turns (oscillation)"
                )
            }
            Self::CumulativeOutputFlood { window, bytes } => {
                format!(
                    "tool output flooded {bytes} bytes across the last {window} calls (over threshold)"
                )
            }
            Self::NoProgressStall {
                consecutive_read_only,
            } => {
                format!(
                    "{consecutive_read_only} consecutive read-only tool calls with no file edit (no-progress stall)"
                )
            }
        }
    }
}

pub fn evaluate(
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    recent_verifier_error_counts: &[usize],
    last_tool_output: Option<(&str, usize)>,
    config: &GovernorConfig,
) -> Option<HardFailSignal> {
    if let Some(signal) =
        check_identical_repetition(recent_tool_calls, config.identical_call_threshold)
    {
        return Some(signal);
    }
    if let Some(signal) = check_verifier_persistence(
        recent_verifier_error_counts,
        config.verifier_persistence_threshold,
    ) {
        return Some(signal);
    }
    if let Some(signal) = check_runaway_output(last_tool_output, config.runaway_output_bytes) {
        return Some(signal);
    }
    None
}

fn check_identical_repetition(
    recent: &VecDeque<ToolCallSnapshot>,
    threshold: usize,
) -> Option<HardFailSignal> {
    if recent.len() < threshold {
        return None;
    }
    let last_n: Vec<_> = recent.iter().rev().take(threshold).collect();
    let first = &last_n[0];
    let all_identical = last_n
        .iter()
        .all(|c| c.tool == first.tool && c.arguments == first.arguments);
    if !all_identical {
        return None;
    }
    Some(HardFailSignal::IdenticalToolCallRepetition {
        tool: first.tool.clone(),
        consecutive_count: threshold as u32,
    })
}

fn check_verifier_persistence(counts: &[usize], threshold: usize) -> Option<HardFailSignal> {
    if counts.len() < threshold {
        return None;
    }
    let last_n = &counts[counts.len() - threshold..];

    // Must all be > 0
    if last_n.contains(&0) {
        return None;
    }

    // Must be non-decreasing oldest -> newest
    for w in last_n.windows(2) {
        if w[0] > w[1] {
            return None;
        }
    }

    Some(HardFailSignal::VerifierFailurePersistent {
        consecutive_failures: threshold as u32,
    })
}

fn check_runaway_output(output: Option<(&str, usize)>, limit: usize) -> Option<HardFailSignal> {
    let (tool, bytes) = output?;
    if bytes <= limit {
        return None;
    }
    Some(HardFailSignal::RunawayOutput {
        tool: tool.to_string(),
        bytes,
    })
}

/// Empty-completion stall: the model emitted `consecutive_empty` blank/think-only
/// completions in a row (no tool call, no answer text). Distinct from
/// `IdenticalToolCallRepetition`, which only sees turns that produced a tool call.
pub fn check_empty_completion_stall(
    consecutive_empty: usize,
    threshold: usize,
) -> Option<HardFailSignal> {
    if consecutive_empty >= threshold {
        Some(HardFailSignal::EmptyCompletionStall {
            consecutive_count: threshold as u32,
        })
    } else {
        None
    }
}

/// Stuck gate-feedback stall: the loop re-injected byte-identical gate feedback
/// (gate-retry / task-coverage) `consecutive_repeats` times in a row
/// with no intervening state change.
pub fn check_repeated_gate_feedback(
    consecutive_repeats: usize,
    threshold: usize,
) -> Option<HardFailSignal> {
    if consecutive_repeats >= threshold {
        Some(HardFailSignal::StuckGateFeedback {
            consecutive_count: threshold as u32,
        })
    } else {
        None
    }
}

/// Oscillation stall: the last `window` tool calls collapse to only a small set
/// of distinct `(tool, arguments)` pairs (e.g. an A,B,A,B read↔patch cycle) that
/// `IdenticalToolCallRepetition` misses because the calls are not *consecutively*
/// identical. Fires when the distinct count is in `2..=distinct_max`. A distinct
/// count of 1 is left to `check_identical_repetition`; `window == 0` disables.
pub fn check_oscillation(
    recent: &VecDeque<ToolCallSnapshot>,
    window: usize,
    distinct_max: usize,
) -> Option<HardFailSignal> {
    if window == 0 || recent.len() < window {
        return None;
    }
    let mut distinct: Vec<(&str, &serde_json::Value)> = Vec::new();
    for call in recent.iter().rev().take(window) {
        let key = (call.tool.as_str(), &call.arguments);
        if !distinct.iter().any(|(t, a)| *t == key.0 && *a == key.1) {
            distinct.push(key);
        }
    }
    let n = distinct.len();
    if n >= 2 && n <= distinct_max {
        Some(HardFailSignal::Oscillation {
            distinct_calls: n,
            window,
        })
    } else {
        None
    }
}

/// File-mutating tools whose presence resets the no-progress counter. Kept in
/// sync with `agent::tools` (the only tools that change files on disk).
const MUTATING_TOOLS: [&str; 2] = ["patch", "write_file"];

/// No-progress read-only stall: the executor has made `threshold` consecutive
/// tool calls without a single file-mutating call (`patch`/`write_file`) among
/// them — the signature of a verify-loop (repeated `grep`/test/`git status`
/// calls that make no code progress) that `check_identical_repetition` (needs
/// *exactly* repeated calls) and `check_oscillation` (needs a *small distinct*
/// set) both miss when the calls vary. The trailing run resets on any mutating
/// call, so ordinary read-heavy exploration *between* edits never trips it.
/// `threshold == 0` disables.
pub fn check_read_only_stall(
    recent: &VecDeque<ToolCallSnapshot>,
    threshold: usize,
) -> Option<HardFailSignal> {
    if threshold == 0 {
        return None;
    }
    let mut run = 0usize;
    for call in recent.iter().rev() {
        if MUTATING_TOOLS.contains(&call.tool.as_str()) {
            break;
        }
        run += 1;
    }
    if run >= threshold {
        Some(HardFailSignal::NoProgressStall {
            consecutive_read_only: run as u32,
        })
    } else {
        None
    }
}

/// Cumulative-output flood: the sum of the last `window` tool outputs exceeds
/// `limit` bytes, catching a multi-call flood of sub-`runaway_output_bytes`
/// outputs that `check_runaway_output` (single-call only) misses. Requires a full
/// window; `window == 0` disables.
pub fn check_windowed_output(
    recent_output_bytes: &VecDeque<usize>,
    window: usize,
    limit: usize,
) -> Option<HardFailSignal> {
    if window == 0 || recent_output_bytes.len() < window {
        return None;
    }
    let bytes: usize = recent_output_bytes.iter().rev().take(window).sum();
    if bytes > limit {
        Some(HardFailSignal::CumulativeOutputFlood { window, bytes })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- describe tests ---

    #[test]
    fn describe_identical_repetition() {
        let signal = HardFailSignal::IdenticalToolCallRepetition {
            tool: "patch".to_string(),
            consecutive_count: 3,
        };
        let desc = signal.describe();
        assert!(desc.contains("identical "));
        assert!(desc.contains("patch"));
        assert!(desc.contains("repeated "));
    }

    #[test]
    fn describe_verifier_persistence() {
        let signal = HardFailSignal::VerifierFailurePersistent {
            consecutive_failures: 3,
        };
        let desc = signal.describe();
        assert!(desc.contains("verifier flagged errors on "));
        assert!(desc.contains("3"));
    }

    #[test]
    fn describe_runaway_output() {
        let signal = HardFailSignal::RunawayOutput {
            tool: "read_file".to_string(),
            bytes: 200_000,
        };
        let desc = signal.describe();
        assert!(desc.contains("produced "));
        assert!(desc.contains("read_file"));
        assert!(desc.contains(" bytes"));
    }

    // --- positive detection tests ---

    #[test]
    fn detects_identical_repetition() {
        let mut recent = VecDeque::new();
        let snap = ToolCallSnapshot {
            tool: "patch".to_string(),
            arguments: serde_json::json!({"path": "x.rs", "old": "a", "new": "b"}),
            succeeded: true,
        };
        for _ in 0..6 {
            recent.push_back(snap.clone());
        }
        let signal = evaluate(&recent, &[], None, &GovernorConfig::default()).unwrap();
        assert!(matches!(
            signal,
            HardFailSignal::IdenticalToolCallRepetition { .. }
        ));
    }

    #[test]
    fn detects_verifier_persistence() {
        let counts = [2usize, 2, 2, 2, 2, 2];
        let recent = VecDeque::new();
        let signal = evaluate(&recent, &counts, None, &GovernorConfig::default()).unwrap();
        assert!(matches!(
            signal,
            HardFailSignal::VerifierFailurePersistent { .. }
        ));
    }

    #[test]
    fn detects_runaway_output() {
        let recent = VecDeque::new();
        let cfg = GovernorConfig::default();
        let signal = evaluate(
            &recent,
            &[],
            Some(("read_file", cfg.runaway_output_bytes + 1)),
            &cfg,
        )
        .unwrap();
        assert!(matches!(signal, HardFailSignal::RunawayOutput { .. }));
    }

    // --- negative boundary tests ---

    #[test]
    fn healthy_session_returns_none() {
        let mut recent = VecDeque::new();
        recent.push_back(ToolCallSnapshot {
            tool: "read_file".to_string(),
            arguments: serde_json::json!({"path": "a"}),
            succeeded: true,
        });
        recent.push_back(ToolCallSnapshot {
            tool: "patch".to_string(),
            arguments: serde_json::json!({"path": "b"}),
            succeeded: true,
        });
        assert!(
            evaluate(
                &recent,
                &[1],
                Some(("read_file", 100)),
                &GovernorConfig::default()
            )
            .is_none()
        );
    }

    #[test]
    fn no_repetition_when_arguments_differ() {
        let mut recent = VecDeque::new();
        for i in 0..3 {
            recent.push_back(ToolCallSnapshot {
                tool: "patch".to_string(),
                arguments: serde_json::json!({"path": format!("file_{i}.rs")}),
                succeeded: true,
            });
        }
        assert!(evaluate(&recent, &[], None, &GovernorConfig::default()).is_none());
    }

    #[test]
    fn no_repetition_below_threshold() {
        let mut recent = VecDeque::new();
        let snap = ToolCallSnapshot {
            tool: "patch".to_string(),
            arguments: serde_json::json!({"path": "x.rs"}),
            succeeded: true,
        };
        for _ in 0..2 {
            recent.push_back(snap.clone());
        }
        assert!(evaluate(&recent, &[], None, &GovernorConfig::default()).is_none());
    }

    #[test]
    fn no_verifier_persistence_when_errors_decrease() {
        let counts = [5usize, 3, 1];
        let recent = VecDeque::new();
        assert!(evaluate(&recent, &counts, None, &GovernorConfig::default()).is_none());
    }

    #[test]
    fn no_verifier_persistence_when_a_count_is_zero() {
        let counts = [2usize, 0, 2];
        let recent = VecDeque::new();
        assert!(evaluate(&recent, &counts, None, &GovernorConfig::default()).is_none());
    }

    #[test]
    fn no_runaway_at_exact_threshold() {
        let recent = VecDeque::new();
        let cfg = GovernorConfig::default();
        assert!(
            evaluate(
                &recent,
                &[],
                Some(("read_file", cfg.runaway_output_bytes)),
                &cfg
            )
            .is_none()
        );
    }

    #[test]
    fn check_order_repetition_precedes_verifier() {
        let mut recent = VecDeque::new();
        let snap = ToolCallSnapshot {
            tool: "patch".to_string(),
            arguments: serde_json::json!({"path": "x.rs"}),
            succeeded: false,
        };
        for _ in 0..6 {
            recent.push_back(snap.clone());
        }
        let counts = [2usize, 2, 2, 2, 2, 2];
        let signal = evaluate(&recent, &counts, None, &GovernorConfig::default()).unwrap();
        assert!(matches!(
            signal,
            HardFailSignal::IdenticalToolCallRepetition { .. }
        ));
    }

    // --- empty-completion stall tests ---

    #[test]
    fn empty_completion_stall_fires_at_threshold() {
        let signal = check_empty_completion_stall(3, 3);
        assert!(matches!(
            signal,
            Some(HardFailSignal::EmptyCompletionStall {
                consecutive_count: 3
            })
        ));
    }

    #[test]
    fn empty_completion_stall_silent_below_threshold() {
        let signal = check_empty_completion_stall(2, 3);
        assert!(signal.is_none());
    }

    #[test]
    fn describe_empty_completion_stall() {
        let signal = HardFailSignal::EmptyCompletionStall {
            consecutive_count: 5,
        };
        let desc = signal.describe();
        assert!(desc.contains("empty completions"));
        assert!(desc.contains("5"));
    }

    // --- M22 phase-02: stuck gate-feedback stall tests ---

    #[test]
    fn repeated_gate_feedback_fires_at_threshold() {
        let signal = check_repeated_gate_feedback(5, 5);
        assert_eq!(
            signal,
            Some(HardFailSignal::StuckGateFeedback {
                consecutive_count: 5
            })
        );
    }

    #[test]
    fn repeated_gate_feedback_silent_below_threshold() {
        let signal = check_repeated_gate_feedback(4, 5);
        assert!(signal.is_none());
    }

    #[test]
    fn describe_stuck_gate_feedback() {
        let signal = HardFailSignal::StuckGateFeedback {
            consecutive_count: 7,
        };
        let desc = signal.describe();
        assert!(desc.contains("re-injected"));
        assert!(desc.contains("7"));
    }

    // --- M26 phase-07a: oscillation stall tests ---

    #[test]
    fn oscillation_fires_on_two_call_cycle() {
        let mut recent = VecDeque::new();
        let a = ToolCallSnapshot {
            tool: "read_file".to_string(),
            arguments: serde_json::json!({"path": "a.txt"}),
            succeeded: true,
        };
        let b = ToolCallSnapshot {
            tool: "read_file".to_string(),
            arguments: serde_json::json!({"path": "b.txt"}),
            succeeded: true,
        };
        // A, B, A, B — 4 calls, 2 distinct
        for _ in 0..2 {
            recent.push_back(a.clone());
            recent.push_back(b.clone());
        }
        let signal = check_oscillation(&recent, 4, 2).unwrap();
        assert!(matches!(
            signal,
            HardFailSignal::Oscillation {
                distinct_calls: 2,
                window: 4
            }
        ));
    }

    #[test]
    fn oscillation_silent_when_window_not_full() {
        let mut recent = VecDeque::new();
        let a = ToolCallSnapshot {
            tool: "read_file".to_string(),
            arguments: serde_json::json!({"path": "a.txt"}),
            succeeded: true,
        };
        let b = ToolCallSnapshot {
            tool: "read_file".to_string(),
            arguments: serde_json::json!({"path": "b.txt"}),
            succeeded: true,
        };
        recent.push_back(a.clone());
        recent.push_back(b.clone());
        recent.push_back(a);
        // Only 3 calls, window=4
        assert!(check_oscillation(&recent, 4, 2).is_none());
    }

    #[test]
    fn oscillation_silent_when_all_identical() {
        let mut recent = VecDeque::new();
        let snap = ToolCallSnapshot {
            tool: "patch".to_string(),
            arguments: serde_json::json!({"path": "x.rs"}),
            succeeded: true,
        };
        for _ in 0..4 {
            recent.push_back(snap.clone());
        }
        // distinct=1, should be left to check_identical_repetition
        assert!(check_oscillation(&recent, 4, 2).is_none());
    }

    #[test]
    fn oscillation_silent_when_too_many_distinct() {
        let mut recent = VecDeque::new();
        for i in 0..4 {
            recent.push_back(ToolCallSnapshot {
                tool: "read_file".to_string(),
                arguments: serde_json::json!({"path": format!("file_{i}.txt")}),
                succeeded: true,
            });
        }
        assert!(check_oscillation(&recent, 4, 2).is_none());
    }

    #[test]
    fn oscillation_disabled_when_window_zero() {
        let mut recent = VecDeque::new();
        let a = ToolCallSnapshot {
            tool: "read_file".to_string(),
            arguments: serde_json::json!({"path": "a.txt"}),
            succeeded: true,
        };
        let b = ToolCallSnapshot {
            tool: "read_file".to_string(),
            arguments: serde_json::json!({"path": "b.txt"}),
            succeeded: true,
        };
        for _ in 0..4 {
            recent.push_back(a.clone());
            recent.push_back(b.clone());
        }
        assert!(check_oscillation(&recent, 0, 2).is_none());
    }

    // --- M26 phase-07a: windowed output tests ---

    #[test]
    fn windowed_output_fires_when_sum_exceeds_limit() {
        let mut recent_output_bytes: VecDeque<usize> = VecDeque::new();
        recent_output_bytes.push_back(400);
        recent_output_bytes.push_back(400);
        recent_output_bytes.push_back(400);
        let signal = check_windowed_output(&recent_output_bytes, 3, 1000).unwrap();
        assert!(matches!(
            signal,
            HardFailSignal::CumulativeOutputFlood {
                window: 3,
                bytes: 1200
            }
        ));
    }

    #[test]
    fn windowed_output_silent_at_or_below_limit() {
        let mut recent_output_bytes: VecDeque<usize> = VecDeque::new();
        recent_output_bytes.push_back(333);
        recent_output_bytes.push_back(333);
        recent_output_bytes.push_back(334);
        // Sum = 1000, exactly at limit — strict > means no fire
        assert!(check_windowed_output(&recent_output_bytes, 3, 1000).is_none());
    }

    #[test]
    fn windowed_output_silent_when_window_not_full() {
        let mut recent_output_bytes: VecDeque<usize> = VecDeque::new();
        recent_output_bytes.push_back(400);
        recent_output_bytes.push_back(400);
        // Only 2 outputs, window=3
        assert!(check_windowed_output(&recent_output_bytes, 3, 1000).is_none());
    }

    #[test]
    fn windowed_output_disabled_when_window_zero() {
        let mut recent_output_bytes: VecDeque<usize> = VecDeque::new();
        recent_output_bytes.push_back(400);
        recent_output_bytes.push_back(400);
        recent_output_bytes.push_back(400);
        assert!(check_windowed_output(&recent_output_bytes, 0, 1000).is_none());
    }

    #[test]
    fn describe_oscillation() {
        let signal = HardFailSignal::Oscillation {
            distinct_calls: 2,
            window: 4,
        };
        let desc = signal.describe();
        assert!(desc.contains("oscillation"));
        assert!(desc.contains("2"));
        assert!(desc.contains("4"));
    }

    #[test]
    fn describe_cumulative_output_flood() {
        let signal = HardFailSignal::CumulativeOutputFlood {
            window: 3,
            bytes: 1200,
        };
        let desc = signal.describe();
        assert!(desc.contains("flooded"));
        assert!(desc.contains("1200"));
        assert!(desc.contains("3"));
    }

    // --- no-progress read-only stall tests ---

    fn read_only_call(i: usize) -> ToolCallSnapshot {
        // Distinct read-only calls (varying args), the exact shape that evades
        // identical-repetition and oscillation detectors.
        let tool = ["bash", "read_file", "grep"][i % 3];
        ToolCallSnapshot {
            tool: tool.to_string(),
            arguments: serde_json::json!({"q": format!("query_{i}")}),
            succeeded: true,
        }
    }

    #[test]
    fn read_only_stall_fires_on_long_varied_run() {
        let mut recent = VecDeque::new();
        for i in 0..20 {
            recent.push_back(read_only_call(i));
        }
        let signal = check_read_only_stall(&recent, 20).unwrap();
        assert!(matches!(
            signal,
            HardFailSignal::NoProgressStall {
                consecutive_read_only: 20
            }
        ));
    }

    #[test]
    fn read_only_stall_resets_on_mutating_call() {
        let mut recent = VecDeque::new();
        // 30 read-only calls, but a patch 5 calls from the end resets the run.
        for i in 0..30 {
            recent.push_back(read_only_call(i));
        }
        // Insert a mutating call, then 5 more read-only calls.
        recent.push_back(ToolCallSnapshot {
            tool: "patch".to_string(),
            arguments: serde_json::json!({"path": "x.rs"}),
            succeeded: true,
        });
        for i in 30..35 {
            recent.push_back(read_only_call(i));
        }
        // Trailing run is only 5 — below the threshold.
        assert!(check_read_only_stall(&recent, 20).is_none());
    }

    #[test]
    fn read_only_stall_silent_below_threshold() {
        let mut recent = VecDeque::new();
        for i in 0..19 {
            recent.push_back(read_only_call(i));
        }
        assert!(check_read_only_stall(&recent, 20).is_none());
    }

    #[test]
    fn read_only_stall_disabled_when_threshold_zero() {
        let mut recent = VecDeque::new();
        for i in 0..50 {
            recent.push_back(read_only_call(i));
        }
        assert!(check_read_only_stall(&recent, 0).is_none());
    }

    #[test]
    fn read_only_stall_counts_write_file_as_progress() {
        let mut recent = VecDeque::new();
        recent.push_back(ToolCallSnapshot {
            tool: "write_file".to_string(),
            arguments: serde_json::json!({"path": "x.rs"}),
            succeeded: true,
        });
        for i in 0..10 {
            recent.push_back(read_only_call(i));
        }
        // Only 10 read-only calls since the last write — below threshold.
        assert!(check_read_only_stall(&recent, 20).is_none());
    }

    #[test]
    fn describe_no_progress_stall() {
        let signal = HardFailSignal::NoProgressStall {
            consecutive_read_only: 20,
        };
        let desc = signal.describe();
        assert!(desc.contains("read-only"));
        assert!(desc.contains("20"));
        assert!(desc.contains("no-progress"));
    }
}
