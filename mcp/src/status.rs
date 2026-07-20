//! On-demand status for a running (or most recent) phase.
//!
//! `execute_phase` blocks the architect for the whole run, so live status can
//! only come from outside that call. This reads the session JSONL log the loop
//! flushes per-record under `<repo>/.rexymcp/sessions/` and reports the latest
//! liveness — the human runs `rexymcp status --repo <path>` in another terminal
//! while a phase is in flight.

use std::path::{Path, PathBuf};

use rexymcp_executor::store::sessions::event::{
    FileNumstat, SessionEvent, SessionRecord, TaskState,
};
use rexymcp_executor::store::sessions::jsonl::read_session_log;
use serde::Serialize;

/// One tracked task as last seen in the log (M12 Arc A), in first-seen order.
/// The dashboard Tasks panel renders these by title + state; `summarize` keeps
/// the vec insertion-ordered with last-write-wins on title and state per id.
#[derive(Debug, Clone, Serialize)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub state: TaskState,
}

/// Distilled view of a session log, derived from its records.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StatusSummary {
    pub session_id: Option<String>,
    pub phase: Option<String>,
    pub model: Option<String>,
    pub latest_turn: usize,
    pub latest_stage: Option<String>,
    pub latest_message: Option<String>,
    pub files_changed: Vec<FileNumstat>,
    /// Timestamp (unix millis) of the most recent record — the last sign of life.
    pub last_ts: Option<u64>,
    /// Timestamp (unix millis) of the *earliest* record — when the session began.
    /// Symmetric with `last_ts`; drives the Session panel's `duration:` line.
    pub started_at: Option<u64>,
    /// `Some(status)` once the loop wrote a `SessionEnd`; `None` while running.
    pub ended: Option<String>,
    /// Count of `ParseFailed` records seen so far.
    pub parse_failures: usize,
    /// Feedback text from the most recent `ParseFailed`, if any.
    pub last_parse_feedback: Option<String>,
    /// Diagnostic count from the most recent `Verify`. `None` = no verify yet;
    /// `Some(0)` = the last verify was clean.
    pub last_verify_diagnostics: Option<usize>,
    /// Name of the most recent `ToolResult`, and whether it succeeded.
    pub last_tool: Option<String>,
    pub last_tool_ok: Option<bool>,
    /// Reason from a `HardFail` record, if one was logged.
    pub hard_fail_reason: Option<String>,
    /// Cumulative input tokens from the most recent `Metrics` record.
    pub last_input_tokens: Option<u32>,
    /// Cumulative output tokens from the most recent `Metrics` record.
    pub last_output_tokens: Option<u32>,
    /// Context-window fraction (0.0..=1.0+) from the most recent `Metrics`;
    /// `None` = no metrics yet. A value of 0.0 means the run had no real
    /// ceiling (unmeasured sentinel).
    pub last_context_pct: Option<f64>,
    /// Unix-millis timestamp of the most recent `Metrics` record.
    pub last_metrics_ts: Option<u64>,
    /// Timestamp of the *second*-most-recent `Metrics` record.
    pub prev_metrics_ts: Option<u64>,
    /// Cumulative output tokens at the second-most-recent `Metrics` record.
    pub prev_output_tokens: Option<u32>,
    /// Tokens currently occupying the context window (0 = unmeasured).
    pub last_context_used: Option<u32>,
    /// Budget ceiling in tokens (0 = no real ceiling configured).
    pub last_context_window: Option<u32>,
    /// Number of `Compaction` records seen so far.
    pub compaction_count: usize,
    /// Sum of `tokens_before` across all `Compaction` records.
    pub compaction_tokens_before: usize,
    /// Sum of `tokens_after` across all `Compaction` records.
    pub compaction_tokens_after: usize,
    /// Number of `OutputFiltered` records (Arc-A boundary filter) seen so far.
    pub output_filtered_count: usize,
    /// Sum of tokens reclaimed by the boundary filter (`tokens_before - tokens_after`).
    pub output_filtered_tokens: usize,
    /// Number of `ReadEvicted` records (Arc-B superseded-read eviction) seen so far.
    pub read_evicted_count: usize,
    /// Sum of `tokens_reclaimed` across all `ReadEvicted` records.
    pub read_evicted_tokens: usize,
    /// Number of `ReadDeduped` records (Arc-B redundant-read dedupe) seen so far.
    pub read_deduped_count: usize,
    /// Sum of `tokens_saved` across all `ReadDeduped` records.
    pub read_deduped_tokens: usize,
    /// Tracked-task counts (M12 Arc A): total distinct task ids seen, and how
    /// many are currently `Done` / `Active` (last-write-wins per id).
    pub tasks_total: usize,
    pub tasks_done: usize,
    pub tasks_active: usize,
    /// Tracked tasks (M12 Arc A) in first-seen order, with titles — drives the
    /// dashboard Tasks panel's named-task list. The `tasks_total/done/active`
    /// counts above are derived from this vec.
    pub tasks: Vec<TaskRow>,
    /// Min/avg/max interval (ms) between consecutive records. Present when ≥2 intervals exist.
    pub update_interval_min_ms: Option<u64>,
    pub update_interval_avg_ms: Option<u64>,
    pub update_interval_max_ms: Option<u64>,
    /// Min/avg/max generation throughput (tok/s) across all Metrics intervals.
    /// Present when ≥2 intervals exist (i.e. ≥3 Metrics records).
    pub tok_per_sec_min: Option<f64>,
    pub tok_per_sec_avg: Option<f64>,
    pub tok_per_sec_max: Option<f64>,
}

/// Fold a session log's records into the latest-state summary. Pure: the
/// "running vs. ended" distinction is `ended.is_none()`, and freshness is left
/// to the caller (it owns the clock).
pub fn summarize(records: &[SessionRecord]) -> StatusSummary {
    let mut summary = StatusSummary::default();
    let mut prev_rec_ts: Option<u64> = None;
    let mut update_intervals: Vec<u64> = Vec::new();
    let mut metrics_snapshots: Vec<(u64, u32)> = Vec::new();

    let mut tasks: Vec<TaskRow> = Vec::new();

    for rec in records {
        if prev_rec_ts.is_some_and(|prev| rec.ts > prev) {
            update_intervals.push(rec.ts - prev_rec_ts.unwrap());
        }
        prev_rec_ts = Some(rec.ts);

        summary.last_ts = Some(match summary.last_ts {
            Some(prev) => prev.max(rec.ts),
            None => rec.ts,
        });
        summary.started_at = Some(match summary.started_at {
            Some(prev) => prev.min(rec.ts),
            None => rec.ts,
        });

        match &rec.event {
            SessionEvent::SessionStart {
                session_id,
                model,
                phase,
            } => {
                summary.session_id = Some(session_id.clone());
                summary.model = Some(model.clone());
                summary.phase = Some(phase.clone());
            }
            SessionEvent::Progress {
                turn,
                stage,
                files_changed,
                message,
            } => {
                summary.latest_turn = *turn;
                summary.latest_stage = Some(stage.clone());
                summary.latest_message = Some(message.clone());
                summary.files_changed = files_changed.clone();
            }
            SessionEvent::SessionEnd { status, turns } => {
                summary.ended = Some(status.clone());
                summary.latest_turn = summary.latest_turn.max(*turns);
            }
            SessionEvent::ParseFailed { failure } => {
                summary.parse_failures += 1;
                summary.last_parse_feedback = Some(failure.feedback.clone());
            }
            SessionEvent::Verify { diagnostics } => {
                summary.last_verify_diagnostics = Some(diagnostics.len());
            }
            SessionEvent::ToolResult {
                name, succeeded, ..
            } => {
                summary.last_tool = Some(name.clone());
                summary.last_tool_ok = Some(*succeeded);
            }
            SessionEvent::HardFail { reason } => {
                summary.hard_fail_reason = Some(reason.clone());
            }
            SessionEvent::Metrics {
                input_tokens,
                output_tokens,
                context_pct,
                context_used,
                context_window,
            } => {
                metrics_snapshots.push((rec.ts, *output_tokens));
                // Shift the prior latest snapshot into "prev" for throughput.
                summary.prev_metrics_ts = summary.last_metrics_ts;
                summary.prev_output_tokens = summary.last_output_tokens;
                summary.last_metrics_ts = Some(rec.ts);
                summary.last_input_tokens = Some(*input_tokens);
                summary.last_output_tokens = Some(*output_tokens);
                summary.last_context_pct = Some(*context_pct);
                summary.last_context_used = Some(*context_used);
                summary.last_context_window = Some(*context_window);
            }
            SessionEvent::Compaction {
                tokens_before,
                tokens_after,
                ..
            } => {
                summary.compaction_count += 1;
                summary.compaction_tokens_before += *tokens_before;
                summary.compaction_tokens_after += *tokens_after;
            }
            SessionEvent::OutputFiltered {
                tokens_before,
                tokens_after,
                ..
            } => {
                summary.output_filtered_count += 1;
                summary.output_filtered_tokens += tokens_before.saturating_sub(*tokens_after);
            }
            SessionEvent::ReadEvicted {
                tokens_reclaimed, ..
            } => {
                summary.read_evicted_count += 1;
                summary.read_evicted_tokens += *tokens_reclaimed;
            }
            SessionEvent::ReadDeduped { tokens_saved, .. } => {
                summary.read_deduped_count += 1;
                summary.read_deduped_tokens += *tokens_saved;
            }
            SessionEvent::TaskUpdate { id, title, state } => {
                if let Some(row) = tasks.iter_mut().find(|r| &r.id == id) {
                    row.title = title.clone();
                    row.state = *state;
                } else {
                    tasks.push(TaskRow {
                        id: id.clone(),
                        title: title.clone(),
                        state: *state,
                    });
                }
            }
            _ => {} // Prompt, Completion, Parsed remain intentionally unread
        }
    }

    // Update-interval stats: show when ≥2 intervals give a meaningful spread.
    if update_intervals.len() >= 2 {
        summary.update_interval_min_ms = update_intervals.iter().copied().min();
        summary.update_interval_max_ms = update_intervals.iter().copied().max();
        summary.update_interval_avg_ms =
            Some(update_intervals.iter().sum::<u64>() / update_intervals.len() as u64);
    }

    // Tok/s stats across all Metrics intervals: show when ≥2 rates give a spread.
    let tok_rates: Vec<f64> = metrics_snapshots
        .windows(2)
        .filter_map(|w| {
            let dt_ms = w[1].0.checked_sub(w[0].0)?;
            if dt_ms == 0 {
                return None;
            }
            let d_out = w[1].1.saturating_sub(w[0].1);
            Some(d_out as f64 / (dt_ms as f64 / 1000.0))
        })
        .collect();
    if tok_rates.len() >= 2 {
        summary.tok_per_sec_min = tok_rates.iter().cloned().reduce(f64::min);
        summary.tok_per_sec_max = tok_rates.iter().cloned().reduce(f64::max);
        summary.tok_per_sec_avg = Some(tok_rates.iter().sum::<f64>() / tok_rates.len() as f64);
    }

    summary.tasks_total = tasks.len();
    summary.tasks_done = tasks.iter().filter(|r| r.state == TaskState::Done).count();
    summary.tasks_active = tasks
        .iter()
        .filter(|r| r.state == TaskState::Active)
        .count();
    summary.tasks = tasks;

    summary
}

/// Pick the most recently modified `*.jsonl` log under `sessions_dir`, or
/// `None` if the directory is absent or holds no logs.
pub fn find_latest_session_log(sessions_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(sessions_dir).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, path));
        }
    }

    best.map(|(_, p)| p)
}

/// The standard session-log location under a target repo.
pub fn sessions_dir(repo: &Path) -> PathBuf {
    repo.join(".rexymcp").join("sessions")
}

/// Render the summary for a human. `now_ms` is the current unix-millis clock,
/// injected so the "age" line is testable.
pub fn format_status(summary: &StatusSummary, now_ms: u64) -> String {
    let mut lines = Vec::new();

    let phase = summary.phase.as_deref().unwrap_or("<unknown>");
    let session = summary.session_id.as_deref().unwrap_or("<unknown>");
    lines.push(format!("phase: {phase}  session: {session}"));

    if let Some(model) = &summary.model {
        lines.push(format!("model: {model}"));
    }

    let state = match &summary.ended {
        Some(status) => format!("ended ({status})"),
        None => "running".to_string(),
    };
    lines.push(format!("state: {state}"));

    let stage = summary.latest_stage.as_deref().unwrap_or("<none>");
    lines.push(format!("turn {}, stage {stage}", summary.latest_turn));

    if let Some(msg) = &summary.latest_message {
        lines.push(msg.clone());
    }

    if let Some(ts) = summary.last_ts {
        let age_ms = now_ms.saturating_sub(ts);
        lines.push(format!("last update: {} ago", humanize_age(age_ms)));
    }

    let reclaimed = summary.output_filtered_tokens
        + summary.read_evicted_tokens
        + summary.read_deduped_tokens
        + summary
            .compaction_tokens_before
            .saturating_sub(summary.compaction_tokens_after);
    if reclaimed > 0 {
        lines.push(format!(
            "reclaimed: {reclaimed} tokens (filter {}, evict {}, dedupe {}, compaction {})",
            summary.output_filtered_tokens,
            summary.read_evicted_tokens,
            summary.read_deduped_tokens,
            summary
                .compaction_tokens_before
                .saturating_sub(summary.compaction_tokens_after),
        ));
    }

    if summary.tasks_total > 0 {
        lines.push(format!(
            "tasks: {}/{} done ({} active)",
            summary.tasks_done, summary.tasks_total, summary.tasks_active
        ));
    }

    lines.join("\n")
}

/// Compact "5s" / "3m12s" / "1h04m" age string from a millisecond span.
pub(crate) fn humanize_age(age_ms: u64) -> String {
    let secs = age_ms / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Resolve which session log to read this poll. `session = None` follows the
/// most-recently-modified log (so a newly-started session is picked up on the
/// next poll); `session = Some(needle)` pins to the log whose file name contains
/// `needle` and never moves off it, regardless of which log is newest.
pub fn resolve_session_log(repo: &Path, session: Option<&str>) -> Result<PathBuf, String> {
    let dir = sessions_dir(repo);
    match session {
        Some(needle) => {
            let entries = std::fs::read_dir(&dir)
                .map_err(|e| format!("no session logs under {}: {}", dir.display(), e))?;
            entries
                .flatten()
                .map(|e| e.path())
                .find(|p| {
                    p.extension().and_then(|e| e.to_str()) == Some("jsonl")
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.contains(needle))
                })
                .ok_or_else(|| {
                    format!("no session log matching '{needle}' under {}", dir.display())
                })
        }
        None => find_latest_session_log(&dir)
            .ok_or_else(|| format!("no session logs found under {}", dir.display())),
    }
}

/// Resolve the log to report on and return its raw records (chronological).
/// Shares resolution + read with `load_status`; the dashboard transcript needs
/// the raw records, not the distilled summary.
pub fn load_records(repo: &Path, session: Option<&str>) -> Result<Vec<SessionRecord>, String> {
    let log_path = resolve_session_log(repo, session)?;
    read_session_log(&log_path).map_err(|e| format!("failed to read {}: {}", log_path.display(), e))
}

/// Resolve the log to report on, read it, and return `(summary, json_records)`.
/// `session` selects a specific log file whose name contains the substring;
/// `None` picks the most recently modified one.
pub fn load_status(repo: &Path, session: Option<&str>) -> Result<StatusSummary, String> {
    Ok(summarize(&load_records(repo, session)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn rec(ts: u64, turn: usize, event: SessionEvent) -> SessionRecord {
        SessionRecord { ts, turn, event }
    }

    fn start() -> SessionEvent {
        SessionEvent::SessionStart {
            session_id: "abc123".into(),
            model: "qwen".into(),
            phase: "phase-01".into(),
        }
    }

    fn progress(turn: usize, stage: &str) -> SessionEvent {
        SessionEvent::Progress {
            turn,
            stage: stage.into(),
            files_changed: vec![],
            message: format!("turn={turn} stage={stage} +0/-0 files=0"),
        }
    }

    fn parse_failed(feedback: &str) -> SessionEvent {
        SessionEvent::ParseFailed {
            failure: rexymcp_executor::parser::ParseFailure {
                raw: String::new(),
                detected_format: None,
                candidates: vec![],
                feedback: feedback.into(),
            },
        }
    }

    fn verify(diagnostics: usize) -> SessionEvent {
        SessionEvent::Verify {
            diagnostics: (0..diagnostics)
                .map(|i| rexymcp_executor::governor::verifier::Diagnostic {
                    path: std::path::PathBuf::from("src/test.rs"),
                    message: format!("diag {i}"),
                    severity: rexymcp_executor::governor::verifier::Severity::Warning,
                    line: 1,
                    column: Some(1),
                    code: None,
                })
                .collect(),
        }
    }

    fn tool_result(name: &str, succeeded: bool) -> SessionEvent {
        SessionEvent::ToolResult {
            name: name.into(),
            succeeded,
            output_preview: String::new(),
            output_bytes: 0,
        }
    }

    fn hard_fail(reason: &str) -> SessionEvent {
        SessionEvent::HardFail {
            reason: reason.into(),
        }
    }

    fn metrics(input_tokens: u32, output_tokens: u32, context_pct: f64) -> SessionEvent {
        SessionEvent::Metrics {
            input_tokens,
            output_tokens,
            context_pct,
            context_used: 0,
            context_window: 0,
        }
    }

    fn compaction(tokens_before: usize, tokens_after: usize) -> SessionEvent {
        SessionEvent::Compaction {
            tokens_before,
            tokens_after,
            messages_signaturized: 0,
            messages_evicted: 0,
        }
    }

    fn output_filtered(tokens_before: usize, tokens_after: usize, filter: &str) -> SessionEvent {
        SessionEvent::OutputFiltered {
            tokens_before,
            tokens_after,
            filter: filter.into(),
        }
    }

    fn read_evicted(path: &str, reads_evicted: usize, tokens_reclaimed: usize) -> SessionEvent {
        SessionEvent::ReadEvicted {
            path: path.into(),
            reads_evicted,
            tokens_reclaimed,
        }
    }

    fn read_deduped(path: &str, tokens_saved: usize, prior_turn: usize) -> SessionEvent {
        SessionEvent::ReadDeduped {
            path: path.into(),
            tokens_saved,
            prior_turn,
        }
    }

    fn task_update(id: &str, title: &str, state: TaskState) -> SessionEvent {
        SessionEvent::TaskUpdate {
            id: id.into(),
            title: title.into(),
            state,
        }
    }

    #[test]
    fn summarize_empty_log_is_all_none() {
        let s = summarize(&[]);
        assert_eq!(s.session_id, None);
        assert_eq!(s.latest_turn, 0);
        assert_eq!(s.ended, None);
        assert_eq!(s.last_ts, None);
        assert_eq!(s.started_at, None);
    }

    #[test]
    fn summarize_captures_started_at() {
        let recs = vec![rec(300, 1, progress(1, "tool_start")), rec(100, 0, start())];
        let s = summarize(&recs);
        assert_eq!(s.started_at, Some(100));
        assert_eq!(s.last_ts, Some(300));
    }

    #[test]
    fn summarize_picks_latest_progress_and_metadata() {
        let recs = vec![
            rec(100, 0, start()),
            rec(200, 1, progress(1, "turn_start")),
            rec(300, 2, progress(2, "tool:read_file")),
        ];
        let s = summarize(&recs);
        assert_eq!(s.session_id.as_deref(), Some("abc123"));
        assert_eq!(s.phase.as_deref(), Some("phase-01"));
        assert_eq!(s.model.as_deref(), Some("qwen"));
        assert_eq!(s.latest_turn, 2);
        assert_eq!(s.latest_stage.as_deref(), Some("tool:read_file"));
        assert_eq!(s.last_ts, Some(300));
        assert_eq!(s.ended, None);
    }

    #[test]
    fn summarize_marks_ended_on_session_end() {
        let recs = vec![
            rec(100, 0, start()),
            rec(200, 1, progress(1, "turn_start")),
            rec(
                300,
                3,
                SessionEvent::SessionEnd {
                    status: "complete".into(),
                    turns: 3,
                },
            ),
        ];
        let s = summarize(&recs);
        assert_eq!(s.ended.as_deref(), Some("complete"));
        assert_eq!(s.latest_turn, 3);
    }

    #[test]
    fn format_status_running_shows_state_and_age() {
        let s = summarize(&[
            rec(1_000, 0, start()),
            rec(2_000, 1, progress(1, "tool:patch")),
        ]);
        let out = format_status(&s, 5_000);
        assert!(out.contains("phase: phase-01"));
        assert!(out.contains("session: abc123"));
        assert!(out.contains("state: running"));
        assert!(out.contains("turn 1, stage tool:patch"));
        assert!(out.contains("last update: 3s ago"));
    }

    #[test]
    fn format_status_ended_shows_status() {
        let s = summarize(&[
            rec(1_000, 0, start()),
            rec(
                2_000,
                2,
                SessionEvent::SessionEnd {
                    status: "hard_fail".into(),
                    turns: 2,
                },
            ),
        ]);
        let out = format_status(&s, 2_000);
        assert!(out.contains("state: ended (hard_fail)"));
    }

    #[test]
    fn summarize_folds_output_filtered_count_and_tokens() {
        let recs = vec![
            rec(100, 0, start()),
            rec(200, 1, output_filtered(1000, 200, "cargo")),
        ];
        let s = summarize(&recs);
        assert_eq!(s.output_filtered_count, 1);
        assert_eq!(s.output_filtered_tokens, 800);
    }

    #[test]
    fn summarize_folds_read_evicted() {
        let recs = vec![
            rec(100, 0, start()),
            rec(200, 1, read_evicted("src/lib.rs", 1, 500)),
        ];
        let s = summarize(&recs);
        assert_eq!(s.read_evicted_count, 1);
        assert_eq!(s.read_evicted_tokens, 500);
    }

    #[test]
    fn summarize_folds_read_deduped() {
        let recs = vec![
            rec(100, 0, start()),
            rec(200, 1, read_deduped("src/lib.rs", 300, 3)),
        ];
        let s = summarize(&recs);
        assert_eq!(s.read_deduped_count, 1);
        assert_eq!(s.read_deduped_tokens, 300);
    }

    #[test]
    fn summarize_folds_task_states_last_write_wins() {
        let recs = vec![
            rec(100, 0, task_update("1", "Read config", TaskState::Pending)),
            rec(200, 1, task_update("2", "Write tests", TaskState::Pending)),
            rec(300, 2, task_update("1", "Read config", TaskState::Done)),
        ];
        let s = summarize(&recs);
        assert_eq!(s.tasks_total, 2);
        assert_eq!(s.tasks_done, 1);
        assert_eq!(s.tasks_active, 0);
    }

    #[test]
    fn summarize_captures_task_titles_in_order() {
        let recs = vec![
            rec(100, 0, task_update("1", "Read config", TaskState::Pending)),
            rec(200, 1, task_update("2", "Write tests", TaskState::Pending)),
            rec(300, 2, task_update("1", "Read config", TaskState::Done)),
        ];
        let s = summarize(&recs);
        assert_eq!(s.tasks.len(), 2);
        assert_eq!(s.tasks[0].id, "1");
        assert_eq!(s.tasks[0].title, "Read config");
        assert_eq!(s.tasks[0].state, TaskState::Done);
        assert_eq!(s.tasks[1].id, "2");
        assert_eq!(s.tasks[1].title, "Write tests");
        assert_eq!(s.tasks[1].state, TaskState::Pending);
    }

    #[test]
    fn summarize_no_tasks_when_absent() {
        let recs = vec![rec(100, 0, start()), rec(200, 1, progress(1, "turn_start"))];
        let s = summarize(&recs);
        assert_eq!(s.tasks_total, 0);
        assert_eq!(s.tasks_done, 0);
        assert_eq!(s.tasks_active, 0);
    }

    #[test]
    fn format_status_shows_tasks_line_when_present() {
        let recs = vec![
            rec(100, 0, task_update("1", "T1", TaskState::Pending)),
            rec(200, 1, task_update("2", "T2", TaskState::Done)),
        ];
        let s = summarize(&recs);
        let output = format_status(&s, 500);
        assert!(
            output.contains("tasks: 1/2 done"),
            "expected tasks line in output, got: {output}"
        );
    }

    #[test]
    fn format_status_omits_tasks_line_when_absent() {
        let recs = vec![rec(100, 0, start()), rec(200, 1, progress(1, "turn_start"))];
        let s = summarize(&recs);
        let output = format_status(&s, 500);
        assert!(
            !output.contains("tasks:"),
            "expected no tasks line in output, got: {output}"
        );
    }

    #[test]
    fn summarize_reclaim_levers_default_zero_when_absent() {
        let recs = vec![rec(100, 0, start()), rec(200, 1, progress(1, "turn_start"))];
        let s = summarize(&recs);
        assert_eq!(s.output_filtered_count, 0);
        assert_eq!(s.output_filtered_tokens, 0);
        assert_eq!(s.read_evicted_count, 0);
        assert_eq!(s.read_evicted_tokens, 0);
        assert_eq!(s.read_deduped_count, 0);
        assert_eq!(s.read_deduped_tokens, 0);
    }

    #[test]
    fn format_status_shows_reclaimed_line_when_reclaim_occurred() {
        let s = summarize(&[
            rec(1_000, 0, start()),
            rec(2_000, 1, output_filtered(1000, 200, "cargo")),
        ]);
        let out = format_status(&s, 5_000);
        assert!(out.contains("reclaimed:"));
        assert!(out.contains("800"));
    }

    #[test]
    fn format_status_omits_reclaimed_line_when_no_reclaim() {
        let s = summarize(&[
            rec(1_000, 0, start()),
            rec(2_000, 1, progress(1, "tool:patch")),
        ]);
        let out = format_status(&s, 5_000);
        assert!(!out.contains("reclaimed:"));
    }

    #[test]
    fn humanize_age_buckets() {
        assert_eq!(humanize_age(5_000), "5s");
        assert_eq!(humanize_age(192_000), "3m12s");
        assert_eq!(humanize_age(3_840_000), "1h04m");
    }

    #[test]
    fn find_latest_returns_the_only_jsonl() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("session-phase-01-aaa.jsonl");
        std::fs::write(&log, "").unwrap();
        assert_eq!(find_latest_session_log(dir.path()), Some(log));
    }

    #[test]
    fn find_latest_ignores_non_jsonl_and_missing_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(find_latest_session_log(&dir.path().join("absent")), None);

        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        assert_eq!(find_latest_session_log(dir.path()), None);
    }

    #[test]
    fn load_status_reads_latest_log() {
        let dir = tempfile::TempDir::new().unwrap();
        let sessions = sessions_dir(dir.path());
        std::fs::create_dir_all(&sessions).unwrap();
        let log = sessions.join("session-phase-01-abc.jsonl");
        let body = format!(
            "{}\n{}\n",
            serde_json::to_string(&rec(100, 0, start())).unwrap(),
            serde_json::to_string(&rec(200, 1, progress(1, "verify"))).unwrap(),
        );
        std::fs::write(&log, body).unwrap();

        let s = load_status(dir.path(), None).unwrap();
        assert_eq!(s.latest_stage.as_deref(), Some("verify"));
        assert_eq!(s.phase.as_deref(), Some("phase-01"));
    }

    #[test]
    fn load_records_returns_raw_records_in_order() {
        let dir = tempfile::TempDir::new().unwrap();
        let sessions = sessions_dir(dir.path());
        std::fs::create_dir_all(&sessions).unwrap();
        let log = sessions.join("session-phase-01-abc.jsonl");
        let body = format!(
            "{}\n{}\n",
            serde_json::to_string(&rec(100, 0, start())).unwrap(),
            serde_json::to_string(&rec(200, 1, progress(1, "verify"))).unwrap(),
        );
        std::fs::write(&log, body).unwrap();

        let records = load_records(dir.path(), None).unwrap();
        assert_eq!(records.len(), 2);
        assert!(matches!(
            &records[0].event,
            SessionEvent::SessionStart { .. }
        ));
        assert!(matches!(&records[1].event, SessionEvent::Progress { .. }));
    }

    #[test]
    fn load_records_errs_when_no_logs() {
        let dir = tempfile::TempDir::new().unwrap();
        let err = load_records(dir.path(), None).unwrap_err();
        assert!(err.contains("no session logs found"));
    }

    #[test]
    fn load_status_errors_when_no_logs() {
        let dir = tempfile::TempDir::new().unwrap();
        let err = load_status(dir.path(), None).unwrap_err();
        assert!(err.contains("no session logs found"));
    }

    #[test]
    fn status_renders_awaiting_model_stage() {
        let recs = vec![
            rec(100, 0, start()),
            rec(200, 1, progress(1, "verify")),
            rec(300, 2, progress(2, "awaiting_model")),
        ];
        let s = summarize(&recs);
        assert_eq!(s.latest_stage.as_deref(), Some("awaiting_model"));
        let out = format_status(&s, 500);
        assert!(
            out.contains("stage awaiting_model"),
            "status output should show awaiting_model stage: {out}"
        );
    }

    #[test]
    fn summarize_counts_parse_failures() {
        let recs = vec![
            rec(100, 0, start()),
            rec(200, 1, parse_failed("first error")),
            rec(300, 2, parse_failed("second error")),
        ];
        let s = summarize(&recs);
        assert_eq!(s.parse_failures, 2);
        assert_eq!(s.last_parse_feedback.as_deref(), Some("second error"));
    }

    #[test]
    fn summarize_records_last_verify() {
        let recs = vec![
            rec(100, 0, start()),
            rec(200, 1, verify(3)),
            rec(300, 2, verify(0)),
        ];
        let s = summarize(&recs);
        assert_eq!(s.last_verify_diagnostics, Some(0));
    }

    #[test]
    fn summarize_records_last_tool() {
        let recs = vec![
            rec(100, 0, start()),
            rec(200, 1, tool_result("bash", false)),
        ];
        let s = summarize(&recs);
        assert_eq!(s.last_tool.as_deref(), Some("bash"));
        assert_eq!(s.last_tool_ok, Some(false));
    }

    #[test]
    fn summarize_records_hard_fail() {
        let recs = vec![rec(100, 0, start()), rec(200, 1, hard_fail("boom"))];
        let s = summarize(&recs);
        assert_eq!(s.hard_fail_reason.as_deref(), Some("boom"));
    }

    #[test]
    fn summarize_clean_run_has_no_activity() {
        let recs = vec![rec(100, 0, start()), rec(200, 1, progress(1, "turn_start"))];
        let s = summarize(&recs);
        assert_eq!(s.parse_failures, 0);
        assert_eq!(s.last_parse_feedback, None);
        assert_eq!(s.last_verify_diagnostics, None);
        assert_eq!(s.last_tool, None);
        assert_eq!(s.last_tool_ok, None);
        assert_eq!(s.hard_fail_reason, None);
    }

    #[test]
    fn summarize_records_latest_metrics() {
        let recs = vec![
            rec(100, 0, start()),
            rec(200, 1, metrics(500, 100, 0.30)),
            rec(300, 2, metrics(1200, 340, 0.62)),
        ];
        let s = summarize(&recs);
        assert_eq!(s.last_input_tokens, Some(1200));
        assert_eq!(s.last_output_tokens, Some(340));
        assert_eq!(s.last_context_pct, Some(0.62));
    }

    #[test]
    fn summarize_tracks_prev_and_last_metrics() {
        let recs = vec![
            rec(100, 0, start()),
            rec(1000, 1, metrics(500, 100, 0.30)),
            rec(3000, 2, metrics(1200, 300, 0.62)),
        ];
        let s = summarize(&recs);
        assert_eq!(s.prev_metrics_ts, Some(1000));
        assert_eq!(s.prev_output_tokens, Some(100));
        assert_eq!(s.last_metrics_ts, Some(3000));
        assert_eq!(s.last_output_tokens, Some(300));
    }

    #[test]
    fn summarize_one_metric_has_no_prev() {
        let recs = vec![rec(100, 0, start()), rec(200, 1, metrics(500, 100, 0.30))];
        let s = summarize(&recs);
        assert_eq!(s.prev_metrics_ts, None);
        assert_eq!(s.prev_output_tokens, None);
        assert_eq!(s.last_metrics_ts, Some(200));
        assert_eq!(s.last_output_tokens, Some(100));
    }

    #[test]
    fn summarize_clean_run_has_no_metrics() {
        let recs = vec![rec(100, 0, start()), rec(200, 1, progress(1, "turn_start"))];
        let s = summarize(&recs);
        assert_eq!(s.last_input_tokens, None);
        assert_eq!(s.last_output_tokens, None);
        assert_eq!(s.last_context_pct, None);
    }

    #[test]
    fn summarize_folds_compaction_counts_and_tokens() {
        let recs = vec![
            rec(100, 0, start()),
            rec(200, 1, compaction(1000, 600)),
            rec(300, 2, compaction(800, 500)),
        ];
        let s = summarize(&recs);
        assert_eq!(s.compaction_count, 2);
        assert_eq!(s.compaction_tokens_before, 1800);
        assert_eq!(s.compaction_tokens_after, 1100);
    }

    #[test]
    fn summarize_computes_update_interval_stats() {
        // Intervals: 100ms, 200ms, 300ms — avg=200, min=100, max=300.
        let recs = vec![
            rec(0, 0, start()),
            rec(100, 1, progress(1, "a")),
            rec(300, 2, progress(2, "b")),
            rec(600, 3, progress(3, "c")),
        ];
        let s = summarize(&recs);
        assert_eq!(s.update_interval_min_ms, Some(100));
        assert_eq!(s.update_interval_max_ms, Some(300));
        assert_eq!(s.update_interval_avg_ms, Some(200));
    }

    #[test]
    fn summarize_no_interval_stats_with_fewer_than_two_intervals() {
        // One interval — not enough to show a meaningful spread.
        let recs = vec![rec(0, 0, start()), rec(500, 1, progress(1, "a"))];
        let s = summarize(&recs);
        assert_eq!(s.update_interval_min_ms, None);
        assert_eq!(s.update_interval_avg_ms, None);
        assert_eq!(s.update_interval_max_ms, None);
    }

    #[test]
    fn summarize_computes_tok_per_sec_stats() {
        // Three Metrics snapshots → two intervals:
        //   ts 0→1000ms, d_out=100 → 100.0 tok/s
        //   ts 1000→3000ms, d_out=200 → 100.0 tok/s  (same rate, so avg=min=max=100.0)
        let recs = vec![
            rec(0, 0, start()),
            rec(0, 0, metrics(500, 0, 0.10)),
            rec(1000, 1, metrics(600, 100, 0.20)),
            rec(3000, 2, metrics(800, 300, 0.40)),
        ];
        let s = summarize(&recs);
        assert_eq!(s.tok_per_sec_avg, Some(100.0));
        assert_eq!(s.tok_per_sec_min, Some(100.0));
        assert_eq!(s.tok_per_sec_max, Some(100.0));
    }

    #[test]
    fn summarize_no_tok_per_sec_stats_with_fewer_than_two_intervals() {
        // Two Metrics records → one interval, not enough for a spread.
        let recs = vec![
            rec(0, 0, start()),
            rec(0, 0, metrics(500, 0, 0.10)),
            rec(1000, 1, metrics(600, 100, 0.20)),
        ];
        let s = summarize(&recs);
        assert_eq!(s.tok_per_sec_min, None);
        assert_eq!(s.tok_per_sec_avg, None);
        assert_eq!(s.tok_per_sec_max, None);
    }

    fn write_log_with_mtime(
        dir: &std::path::Path,
        name: &str,
        mtime: SystemTime,
    ) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "").unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_modified(mtime).unwrap();
        path
    }

    #[test]
    fn resolve_unpinned_picks_newest_log() {
        let dir = tempfile::TempDir::new().unwrap();
        let sessions = sessions_dir(dir.path());
        std::fs::create_dir_all(&sessions).unwrap();

        let t0 = SystemTime::UNIX_EPOCH;
        let t1 = t0 + Duration::from_secs(10);

        let _aaa = write_log_with_mtime(&sessions, "session-phase-01-aaa.jsonl", t0);
        let bbb = write_log_with_mtime(&sessions, "session-phase-02-bbb.jsonl", t1);

        let resolved = resolve_session_log(dir.path(), None).unwrap();
        assert_eq!(resolved, bbb);
    }

    #[test]
    fn resolve_unpinned_follows_when_newer_log_appears() {
        let dir = tempfile::TempDir::new().unwrap();
        let sessions = sessions_dir(dir.path());
        std::fs::create_dir_all(&sessions).unwrap();

        let t0 = SystemTime::UNIX_EPOCH;
        let aaa = write_log_with_mtime(&sessions, "session-phase-01-aaa.jsonl", t0);

        let resolved = resolve_session_log(dir.path(), None).unwrap();
        assert_eq!(resolved, aaa);

        let t1 = t0 + Duration::from_secs(10);
        let bbb = write_log_with_mtime(&sessions, "session-phase-02-bbb.jsonl", t1);

        let resolved = resolve_session_log(dir.path(), None).unwrap();
        assert_eq!(resolved, bbb);
    }

    #[test]
    fn resolve_pinned_ignores_newer_nonmatching_log() {
        let dir = tempfile::TempDir::new().unwrap();
        let sessions = sessions_dir(dir.path());
        std::fs::create_dir_all(&sessions).unwrap();

        let t0 = SystemTime::UNIX_EPOCH;
        let t1 = t0 + Duration::from_secs(10);

        let aaa = write_log_with_mtime(&sessions, "session-phase-01-aaa.jsonl", t0);
        let _bbb = write_log_with_mtime(&sessions, "session-phase-02-bbb.jsonl", t1);

        let resolved = resolve_session_log(dir.path(), Some("aaa")).unwrap();
        assert_eq!(resolved, aaa);
    }

    #[test]
    fn resolve_unpinned_errs_when_no_logs() {
        let dir = tempfile::TempDir::new().unwrap();
        let err = resolve_session_log(dir.path(), None).unwrap_err();
        assert!(err.contains("no session logs found"));
    }

    #[test]
    fn resolve_pinned_errs_when_no_match() {
        let dir = tempfile::TempDir::new().unwrap();
        let sessions = sessions_dir(dir.path());
        std::fs::create_dir_all(&sessions).unwrap();

        let t0 = SystemTime::UNIX_EPOCH;
        let _aaa = write_log_with_mtime(&sessions, "session-phase-01-aaa.jsonl", t0);

        let err = resolve_session_log(dir.path(), Some("zzz")).unwrap_err();
        assert!(err.contains("no session log matching 'zzz'"));
    }
}
