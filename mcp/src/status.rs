//! On-demand status for a running (or most recent) phase.
//!
//! `execute_phase` blocks the architect for the whole run, so live status can
//! only come from outside that call. This reads the session JSONL log the loop
//! flushes per-record under `<repo>/.rexymcp/sessions/` and reports the latest
//! liveness — the human runs `rexymcp status --repo <path>` in another terminal
//! while a phase is in flight.

use std::path::{Path, PathBuf};

use rexymcp_executor::store::sessions::event::{FileNumstat, SessionEvent, SessionRecord};
use rexymcp_executor::store::sessions::jsonl::read_session_log;
use serde::Serialize;

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
    /// `Some(status)` once the loop wrote a `SessionEnd`; `None` while running.
    pub ended: Option<String>,
}

/// Fold a session log's records into the latest-state summary. Pure: the
/// "running vs. ended" distinction is `ended.is_none()`, and freshness is left
/// to the caller (it owns the clock).
pub fn summarize(records: &[SessionRecord]) -> StatusSummary {
    let mut summary = StatusSummary {
        session_id: None,
        phase: None,
        model: None,
        latest_turn: 0,
        latest_stage: None,
        latest_message: None,
        files_changed: Vec::new(),
        last_ts: None,
        ended: None,
    };

    for rec in records {
        summary.last_ts = Some(match summary.last_ts {
            Some(prev) => prev.max(rec.ts),
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
            _ => {}
        }
    }

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

/// Resolve the log to report on, read it, and return `(summary, json_records)`.
/// `session` selects a specific log file whose name contains the substring;
/// `None` picks the most recently modified one.
pub fn load_status(repo: &Path, session: Option<&str>) -> Result<StatusSummary, String> {
    let dir = sessions_dir(repo);

    let log_path = match session {
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
                })?
        }
        None => find_latest_session_log(&dir)
            .ok_or_else(|| format!("no session logs found under {}", dir.display()))?,
    };

    let records = read_session_log(&log_path)
        .map_err(|e| format!("failed to read {}: {}", log_path.display(), e))?;

    Ok(summarize(&records))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn summarize_empty_log_is_all_none() {
        let s = summarize(&[]);
        assert_eq!(s.session_id, None);
        assert_eq!(s.latest_turn, 0);
        assert_eq!(s.ended, None);
        assert_eq!(s.last_ts, None);
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
}
