use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use similar::{ChangeTag, TextDiff};

use crate::governor::hard_fail::{HardFailSignal, ToolCallSnapshot};
use crate::governor::verifier::Diagnostic;
use crate::phase::{
    Artifacts, Blocker, Briefing, Cancellation, CommandOutputs, FileChange, PhaseResult,
    collect_working_files, summarize_attempts,
};

use super::PhaseInput;

/// Cap on the combined unified diff returned in `PhaseResult.diff`.
const MAX_DIFF_CHARS: usize = 50_000;

pub(super) fn hard_fail_result(
    input: &PhaseInput,
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    project_root: &Path,
    diagnostics: Vec<Diagnostic>,
    signal: HardFailSignal,
    artifacts: Artifacts,
) -> PhaseResult {
    let briefing = Briefing {
        goal: input.goal.clone(),
        acceptance_criteria: input.acceptance_criteria.clone(),
        diagnostics,
        working_files: collect_working_files(recent_tool_calls, project_root),
        what_was_tried: summarize_attempts(recent_tool_calls),
        current_blocker: Blocker::HardFail(signal),
        budget_remaining: "halted on hard-fail".to_string(),
    };
    PhaseResult::hard_fail(briefing, artifacts)
}

pub(super) fn turns_line(max_turns: usize) -> String {
    format!("0 of {max_turns} turns remaining")
}

pub(super) fn cancelled_result(stage: &str, turns: usize, artifacts: Artifacts) -> PhaseResult {
    PhaseResult::cancelled(
        Cancellation {
            reason: None,
            stage: stage.to_string(),
            turns_done: turns,
        },
        artifacts,
    )
}

pub(super) fn budget_exceeded_result(
    input: &PhaseInput,
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    project_root: &Path,
    budget_remaining: String,
    artifacts: Artifacts,
) -> PhaseResult {
    let briefing = Briefing {
        goal: input.goal.clone(),
        acceptance_criteria: input.acceptance_criteria.clone(),
        diagnostics: Vec::new(),
        working_files: collect_working_files(recent_tool_calls, project_root),
        what_was_tried: summarize_attempts(recent_tool_calls),
        current_blocker: Blocker::BudgetExceeded,
        budget_remaining,
    };
    PhaseResult::budget_exceeded(briefing, artifacts)
}

/// Build the artifacts common to every terminal return: the unified diff +
/// `files_changed` of what the model edited, the update-log line, the log path,
/// and the (status-specific) command outputs.
pub(super) fn build_artifacts(
    pre_edit_content: &HashMap<PathBuf, Option<String>>,
    project_root: &Path,
    log_path: Option<PathBuf>,
    status: &str,
    turns: usize,
    command_outputs: CommandOutputs,
) -> Artifacts {
    let (diff, files_changed) = build_diff(pre_edit_content, project_root);
    Artifacts {
        files_changed,
        diff,
        command_outputs,
        update_log: format!("Executor run: {status} after {turns} turn(s)."),
        log_path,
        completion_summary: String::new(),
    }
}

/// Render the combined unified diff (capped) and the `files_changed` summary from
/// the pre-edit snapshots. Files whose content is unchanged (e.g. an edit later
/// reverted) are omitted. Deterministic order (sorted by path).
fn build_diff(
    pre_edit_content: &HashMap<PathBuf, Option<String>>,
    project_root: &Path,
) -> (String, Vec<FileChange>) {
    let mut paths: Vec<&PathBuf> = pre_edit_content.keys().collect();
    paths.sort();

    let mut diff = String::new();
    let mut files_changed = Vec::new();
    for path in paths {
        let before = pre_edit_content
            .get(path)
            .and_then(|b| b.clone())
            .unwrap_or_default();
        let after = std::fs::read_to_string(path).unwrap_or_default();
        if before == after {
            continue;
        }
        let rel = path.strip_prefix(project_root).unwrap_or(path);
        let rel_str = rel.display().to_string();
        let text_diff = TextDiff::from_lines(&before, &after);

        let mut added = 0usize;
        let mut removed = 0usize;
        for change in text_diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Insert => added += 1,
                ChangeTag::Delete => removed += 1,
                ChangeTag::Equal => {}
            }
        }

        if diff.chars().count() < MAX_DIFF_CHARS {
            diff.push_str(
                &text_diff
                    .unified_diff()
                    .header(&rel_str, &rel_str)
                    .to_string(),
            );
            if diff.chars().count() > MAX_DIFF_CHARS {
                diff = diff.chars().take(MAX_DIFF_CHARS).collect();
                diff.push_str("\n… (diff truncated)\n");
            }
        }
        files_changed.push(FileChange {
            path: rel.to_path_buf(),
            change_summary: format!("+{added} -{removed}"),
        });
    }
    (diff, files_changed)
}
