use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::config::GovernorConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallSnapshot {
    pub tool: String,
    pub arguments: serde_json::Value,
    pub succeeded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    BackendError {
        message: String,
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
            Self::BackendError { message } => {
                format!("backend error: {message}")
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
}
