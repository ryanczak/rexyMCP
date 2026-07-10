use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::briefing::Briefing;

/// Terminal status of an `execute_phase` run. Serializes to the contract strings
/// `"complete"` / `"hard_fail"` / `"budget_exceeded"` / `"cancelled"` (M5 returns this as JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    Complete,
    HardFail,
    BudgetExceeded,
    Cancelled,
}

/// Who cancelled the phase. Set by the MCP/CLI layer (phase-03+); the executor
/// loop leaves this `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    UserStop,
    ClaudeStop,
}

/// Cancellation details recorded when a phase is aborted mid-run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cancellation {
    /// Who cancelled. The executor loop leaves this `None`; the MCP/CLI layer
    /// (phase-03+) sets it from the entrypoint that fired the signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CancelReason>,
    /// Where in the turn cycle cancellation was observed.
    pub stage: String,
    /// Turns fully completed before cancellation.
    pub turns_done: usize,
}

/// One file the phase changed, with a short human summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: PathBuf,
    pub change_summary: String,
}

/// Tails of the final command set's stdout/stderr. `None` when a command wasn't
/// run (e.g. the phase failed before the final gate).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandOutputs {
    pub format: Option<String>,
    pub build: Option<String>,
    pub lint: Option<String>,
    pub test: Option<String>,
}

/// The result artifacts common to every status — grouped to keep the
/// constructors from repeating five arguments.
pub struct Artifacts {
    pub files_changed: Vec<FileChange>,
    pub diff: String,
    pub command_outputs: CommandOutputs,
    pub update_log: String,
    /// Path to the on-disk JSONL session log; `None` when the log failed to open.
    pub log_path: Option<PathBuf>,
    /// The executor's final message text (post-think), captured on the
    /// `complete` path. Empty on failure paths and until phase-03b makes the
    /// executor put its Summary/Notes here. Spliced into the server-authored
    /// completion entry.
    pub completion_summary: String,
}

/// The single structured value `execute_phase` returns across the MCP boundary —
/// the entire interface Claude reasons over. `briefing` is present iff the phase
/// did not complete.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseResult {
    pub status: PhaseStatus,
    pub files_changed: Vec<FileChange>,
    pub diff: String,
    pub command_outputs: CommandOutputs,
    pub update_log: String,
    pub briefing: Option<Briefing>,
    /// Path to the on-disk JSONL session log Claude can query; `None` when the log
    /// failed to open.
    pub log_path: Option<PathBuf>,
    /// Non-fatal warnings about the run's *inputs* — e.g. an empty/missing
    /// STANDARDS.md, or a phase doc whose Goal / Acceptance-criteria sections
    /// did not parse. Empty in the common case; surfaced to the architect so a
    /// silently-degraded run is visible in the structured result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The executor's qualitative completion summary (post-think), spliced into
    /// the server-authored completion entry. Empty when absent.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub completion_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<Cancellation>,
}

impl PhaseResult {
    /// A clean completion — no briefing.
    pub fn complete(artifacts: Artifacts) -> Self {
        Self::assemble(PhaseStatus::Complete, None, artifacts)
    }

    /// A hard-fail escalation — carries the briefing.
    pub fn hard_fail(briefing: Briefing, artifacts: Artifacts) -> Self {
        Self::assemble(PhaseStatus::HardFail, Some(briefing), artifacts)
    }

    /// A budget-exhaustion escalation — carries the briefing.
    pub fn budget_exceeded(briefing: Briefing, artifacts: Artifacts) -> Self {
        Self::assemble(PhaseStatus::BudgetExceeded, Some(briefing), artifacts)
    }
    pub fn cancelled(cancellation: Cancellation, artifacts: Artifacts) -> Self {
        let mut result = Self::assemble(PhaseStatus::Cancelled, None, artifacts);
        result.cancellation = Some(cancellation);
        result
    }

    fn assemble(status: PhaseStatus, briefing: Option<Briefing>, artifacts: Artifacts) -> Self {
        Self {
            status,
            files_changed: artifacts.files_changed,
            diff: artifacts.diff,
            command_outputs: artifacts.command_outputs,
            update_log: artifacts.update_log,
            briefing,
            log_path: artifacts.log_path,
            warnings: Vec::new(),
            completion_summary: artifacts.completion_summary,
            cancellation: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::briefing::{Blocker, Briefing};
    use super::*;

    fn artifacts() -> Artifacts {
        Artifacts {
            files_changed: vec![],
            diff: String::new(),
            command_outputs: CommandOutputs::default(),
            update_log: String::new(),
            log_path: None,
            completion_summary: String::new(),
        }
    }

    fn briefing() -> Briefing {
        Briefing {
            goal: "g".to_string(),
            acceptance_criteria: "ac".to_string(),
            diagnostics: vec![],
            working_files: vec![],
            what_was_tried: vec![],
            current_blocker: Blocker::BudgetExceeded,
            budget_remaining: "0 turns".to_string(),
        }
    }

    #[test]
    fn status_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_value(PhaseStatus::Complete).unwrap(),
            serde_json::json!("complete")
        );
        assert_eq!(
            serde_json::to_value(PhaseStatus::HardFail).unwrap(),
            serde_json::json!("hard_fail")
        );
        assert_eq!(
            serde_json::to_value(PhaseStatus::BudgetExceeded).unwrap(),
            serde_json::json!("budget_exceeded")
        );
    }

    #[test]
    fn complete_has_no_briefing() {
        let result = PhaseResult::complete(artifacts());
        assert_eq!(result.status, PhaseStatus::Complete);
        assert!(result.briefing.is_none());
    }

    #[test]
    fn hard_fail_has_briefing() {
        let result = PhaseResult::hard_fail(briefing(), artifacts());
        assert_eq!(result.status, PhaseStatus::HardFail);
        assert!(result.briefing.is_some());
    }

    #[test]
    fn budget_exceeded_has_briefing() {
        let result = PhaseResult::budget_exceeded(briefing(), artifacts());
        assert_eq!(result.status, PhaseStatus::BudgetExceeded);
        assert!(result.briefing.is_some());
    }

    #[test]
    fn command_outputs_serialize_with_pinned_keys() {
        let outputs = CommandOutputs {
            build: Some("ok".to_string()),
            ..Default::default()
        };
        let value = serde_json::to_value(outputs).unwrap();
        let obj = value.as_object().unwrap();
        assert!(obj.contains_key("format"));
        assert!(obj.contains_key("build"));
        assert!(obj.contains_key("lint"));
        assert!(obj.contains_key("test"));
    }

    #[test]
    fn phase_result_with_briefing_round_trips_through_json() {
        let result = PhaseResult::hard_fail(briefing(), artifacts());
        let json = serde_json::to_string(&result).unwrap();
        let back: PhaseResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, back);
    }

    #[test]
    fn phase_result_warnings_round_trip() {
        let mut result = PhaseResult::complete(artifacts());
        result.warnings = vec!["STANDARDS.md is empty".to_string()];
        let json = serde_json::to_string(&result).unwrap();
        let back: PhaseResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result.warnings, back.warnings);
    }

    #[test]
    fn phase_result_empty_warnings_omitted_from_json() {
        let result = PhaseResult::complete(artifacts());
        let json = serde_json::to_string(&result).unwrap();
        assert!(
            !json.contains("\"warnings\""),
            "empty warnings must be omitted from JSON, got: {json}"
        );
    }

    #[test]
    fn phase_result_missing_warnings_key_defaults_empty() {
        let json = r#"{"status":"complete","files_changed":[],"diff":"","command_outputs":{"format":null,"build":null,"lint":null,"test":null},"update_log":"","briefing":null,"log_path":null}"#;
        let result: PhaseResult = serde_json::from_str(json).unwrap();
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn cancelled_status_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_value(PhaseStatus::Cancelled).unwrap(),
            serde_json::json!("cancelled")
        );
    }

    #[test]
    fn cancel_reason_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_value(CancelReason::UserStop).unwrap(),
            serde_json::json!("user_stop")
        );
        assert_eq!(
            serde_json::to_value(CancelReason::ClaudeStop).unwrap(),
            serde_json::json!("claude_stop")
        );
    }

    #[test]
    fn cancelled_result_has_no_briefing_and_carries_cancellation() {
        let cancellation = Cancellation {
            reason: None,
            stage: "between_turns".to_string(),
            turns_done: 3,
        };
        let result = PhaseResult::cancelled(cancellation, artifacts());
        assert_eq!(result.status, PhaseStatus::Cancelled);
        assert!(result.briefing.is_none());
        assert!(result.cancellation.is_some());
        let c = result.cancellation.as_ref().unwrap();
        assert_eq!(c.stage, "between_turns");
        assert_eq!(c.turns_done, 3);
        assert!(c.reason.is_none());
    }

    #[test]
    fn phase_result_cancellation_round_trips_through_json() {
        let cancellation = Cancellation {
            reason: Some(CancelReason::UserStop),
            stage: "awaiting_model".to_string(),
            turns_done: 5,
        };
        let result = PhaseResult::cancelled(cancellation, artifacts());
        let json = serde_json::to_string(&result).unwrap();
        let back: PhaseResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, back);
    }

    #[test]
    fn phase_result_absent_cancellation_omitted_from_json() {
        let result = PhaseResult::complete(artifacts());
        let json = serde_json::to_string(&result).unwrap();
        assert!(
            !json.contains("\"cancellation\""),
            "absent cancellation must be omitted from JSON, got: {json}"
        );
    }

    #[test]
    fn phase_result_completion_summary_round_trips() {
        let mut result = PhaseResult::complete(artifacts());
        result.completion_summary = "I implemented the feature.".to_string();
        let json = serde_json::to_string(&result).unwrap();
        let back: PhaseResult = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.completion_summary, "I implemented the feature.",
            "non-empty completion_summary survives JSON round-trip"
        );
    }

    #[test]
    fn phase_result_empty_completion_summary_omitted_from_json() {
        let result = PhaseResult::complete(artifacts());
        let json = serde_json::to_string(&result).unwrap();
        assert!(
            !json.contains("\"completion_summary\""),
            "empty completion_summary must be omitted from JSON, got: {json}"
        );
    }
}
