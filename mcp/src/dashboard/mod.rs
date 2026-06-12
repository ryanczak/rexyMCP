//! Live dashboard — polls session logs and renders a paned TUI summary.
//!
//! Continuously refreshes a `ratatui` terminal with a header band (Session ·
//! Budget · Compactions) above a body (Activity · Files).

use std::path::Path;

use rexymcp_executor::store::sessions::event::SessionRecord;
use rexymcp_executor::store::telemetry::PhaseRun;

use crate::status::{self, StatusSummary};

mod event_loop;
mod filter;
mod highlight;
mod panels;
mod render;
mod transcript;

pub use panels::{BudgetRates, model_rates};

/// Snapshot of the latest session data or an error loading it.
pub struct DashboardData {
    pub summary: StatusSummary,
    pub records: Vec<SessionRecord>,
    pub error: Option<String>,
    pub milestone: Option<String>,
    /// Cumulative (input_tokens, output_tokens) from `PhaseRun` records whose
    /// `phase_doc_path` belongs to the active milestone. `None` when telemetry
    /// is absent, no phase is active, or no matching records exist.
    pub milestone_savings: Option<(u32, u32)>,
    /// Cumulative (input_tokens, output_tokens) from ALL `PhaseRun` records in
    /// the telemetry file. `(0, 0)` when telemetry dir is not configured.
    pub project_savings: (u32, u32),
}

/// Load the latest session data. Pure, testable.
/// `project_id` is the UUID from the watched repo's `[project] id`; filters
/// telemetry to runs belonging to this project. `None` → project savings is `(0,0)`.
pub fn load_data(
    repo: &Path,
    session: Option<&str>,
    telemetry_dir: Option<&Path>,
    project_id: Option<&str>,
) -> DashboardData {
    // Phase runs are independent of session records — read them before the match
    // so project_savings is always computed even when session loading fails.
    let phase_runs: Vec<PhaseRun> = telemetry_dir.map(read_phase_runs).unwrap_or_default();
    let project_savings = match project_id {
        Some(pid) => phase_runs
            .iter()
            .filter(|r| r.project_id.as_deref() == Some(pid))
            .fold((0u32, 0u32), |(i, o), r| {
                (
                    i.saturating_add(r.tokens.input_tokens),
                    o.saturating_add(r.tokens.output_tokens),
                )
            }),
        None => (0u32, 0u32),
    };

    match status::load_records(repo, session) {
        Ok(records) => {
            let summary = status::summarize(&records);
            let milestone = resolve_milestone(repo, summary.phase.as_deref());
            // Milestone savings requires the active milestone dir from session data.
            // Filter by both project_id and milestone_id for accurate scoping.
            let milestone_savings = resolve_milestone_dir(repo, summary.phase.as_deref())
                .zip(project_id)
                .map(|(milestone_dir, pid)| {
                    phase_runs
                        .iter()
                        .filter(|r| {
                            r.project_id.as_deref() == Some(pid)
                                && r.milestone_id.as_deref() == Some(milestone_dir.as_str())
                        })
                        .fold((0u32, 0u32), |(i, o), r| {
                            (
                                i.saturating_add(r.tokens.input_tokens),
                                o.saturating_add(r.tokens.output_tokens),
                            )
                        })
                })
                .filter(|&(i, o)| i > 0 || o > 0);
            DashboardData {
                summary,
                records,
                error: None,
                milestone,
                milestone_savings,
                project_savings,
            }
        }
        Err(e) => DashboardData {
            summary: StatusSummary::default(),
            records: Vec::new(),
            error: Some(e),
            milestone: None,
            milestone_savings: None,
            project_savings,
        },
    }
}

/// Run the dashboard event loop. Called by `main.rs`.
pub fn run_dashboard(
    repo: &Path,
    session: Option<&str>,
    rates: BudgetRates,
    telemetry_dir: Option<&Path>,
    project_id: Option<String>,
) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result =
        event_loop::run_loop(&mut terminal, repo, session, rates, telemetry_dir, project_id);
    ratatui::restore();
    result
}

/// Parse `<telemetry_dir>/phase_runs.jsonl`, returning one `PhaseRun` per
/// valid line; silently skips empty lines and malformed JSON.
fn read_phase_runs(telemetry_dir: &Path) -> Vec<PhaseRun> {
    let path = telemetry_dir.join("phase_runs.jsonl");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Returns the milestone **directory name** (e.g. `"M17-dashboard-polish-3"`)
/// for the running phase, using the same candidate-selection rules as
/// `resolve_milestone`. `None` when no matching milestone directory is found.
fn resolve_milestone_dir(repo: &Path, phase: Option<&str>) -> Option<String> {
    let phase = phase?;
    let milestones = repo.join("docs/dev/milestones");
    let prefix = format!("{phase}-"); // e.g. "phase-03-"

    // (milestone_number, dir_name, is_active)
    let mut candidates: Vec<(u32, String, bool)> = Vec::new();
    let entries = std::fs::read_dir(&milestones).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(num) = milestone_number(dir_name) else {
            continue;
        };
        // Find a phase doc in this milestone matching the running phase id.
        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        for f in files.flatten() {
            let fname = f.file_name();
            let Some(fname) = fname.to_str() else {
                continue;
            };
            if fname.starts_with(&prefix) && fname.ends_with(".md") {
                let active = match std::fs::read_to_string(f.path()) {
                    Ok(body) => !phase_doc_is_done(&body),
                    Err(_) => false,
                };
                candidates.push((num, dir_name.to_string(), active));
                break;
            }
        }
    }

    // Prefer an active milestone; else the highest-numbered match.
    candidates
        .iter()
        .filter(|(_, _, active)| *active)
        .max_by_key(|(num, _, _)| *num)
        .or_else(|| candidates.iter().max_by_key(|(num, _, _)| *num))
        .map(|(_, dir, _)| dir.clone())
}

/// Thin wrapper — same contract as before, unchanged external behaviour.
fn resolve_milestone(repo: &Path, phase: Option<&str>) -> Option<String> {
    resolve_milestone_dir(repo, phase).map(|dir| format_milestone_name(&dir))
}

/// Parse the leading `M<n>` milestone number from a directory name like
/// `M15-dashboard-polish-2`. `None` if the name doesn't start with `M` followed
/// by digits and a `-`.
fn milestone_number(dir: &str) -> Option<u32> {
    let rest = dir.strip_prefix('M')?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// True when a phase doc's `**Status:**` line reads `done`.
fn phase_doc_is_done(body: &str) -> bool {
    body.lines()
        .find(|l| l.contains("Status:"))
        .map(|l| l.contains("done"))
        .unwrap_or(false)
}

/// Format a milestone directory name into a display label:
/// `M15-dashboard-polish-2` → `M15 — Dashboard Polish 2`. Splits off the `M<n>`
/// prefix, then capitalizes each remaining hyphen-separated word. A directory not
/// matching the `M<n>-<rest>` shape is returned unchanged.
fn format_milestone_name(dir: &str) -> String {
    match dir.split_once('-') {
        Some((prefix, rest)) if milestone_number(prefix).is_some() => {
            let words: Vec<String> = rest.split('-').map(capitalize_word).collect();
            format!("{prefix} — {}", words.join(" "))
        }
        _ => dir.to_string(),
    }
}

/// Uppercase the first character of `w`, leaving the rest unchanged. `"polish"`
/// → `"Polish"`, `"2"` → `"2"`, `""` → `""`.
fn capitalize_word(w: &str) -> String {
    let mut chars = w.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::sessions_dir;
    use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};
    use tempfile::TempDir;

    fn rec(ts: u64, turn: usize, event: SessionEvent) -> SessionRecord {
        SessionRecord { ts, turn, event }
    }

    fn start_event() -> SessionEvent {
        SessionEvent::SessionStart {
            session_id: "test-session".into(),
            model: "test-model".into(),
            phase: "phase-01".into(),
        }
    }

    fn progress_event(turn: usize, stage: &str) -> SessionEvent {
        SessionEvent::Progress {
            turn,
            stage: stage.into(),
            files_changed: vec![],
            message: format!("turn={turn} stage={stage} +0/-0 files=0"),
        }
    }

    // --- load_data tests (carried over from phase-01) ---

    #[test]
    fn load_data_returns_error_when_no_sessions_dir() {
        let dir = TempDir::new().unwrap();
        let data = load_data(dir.path(), None, None, None);
        assert!(data.error.is_some());
        assert!(data.records.is_empty());
        assert!(data.summary.ended.is_none());
    }

    #[test]
    fn load_data_carries_raw_records() {
        let dir = TempDir::new().unwrap();
        let sessions = sessions_dir(dir.path());
        std::fs::create_dir_all(&sessions).unwrap();
        let log = sessions.join("session-phase-01-test.jsonl");
        let body = format!(
            "{}\n{}\n",
            serde_json::to_string(&rec(100, 0, start_event())).unwrap(),
            serde_json::to_string(&rec(200, 1, progress_event(1, "verify"))).unwrap(),
        );
        std::fs::write(&log, body).unwrap();

        let data = load_data(dir.path(), None, None, None);
        assert!(data.error.is_none());
        assert!(!data.records.is_empty());
        assert_eq!(data.records.len(), 2);
        assert_eq!(data.summary.phase.as_deref(), Some("phase-01"));
    }

    #[test]
    fn load_data_empty_records_on_error() {
        let dir = TempDir::new().unwrap();
        let data = load_data(dir.path(), None, None, None);
        assert!(data.error.is_some());
        assert!(data.records.is_empty());
    }

    #[test]
    fn load_data_returns_summary_when_log_exists() {
        let dir = TempDir::new().unwrap();
        let sessions = sessions_dir(dir.path());
        std::fs::create_dir_all(&sessions).unwrap();
        let log = sessions.join("session-phase-01-test.jsonl");
        let body = format!(
            "{}\n{}\n",
            serde_json::to_string(&rec(100, 0, start_event())).unwrap(),
            serde_json::to_string(&rec(200, 1, progress_event(1, "verify"))).unwrap(),
        );
        std::fs::write(&log, body).unwrap();

        let data = load_data(dir.path(), None, None, None);
        assert!(data.error.is_none());
        assert!(data.summary.phase.is_some());
        assert_eq!(data.summary.phase.as_deref(), Some("phase-01"));
    }

    #[test]
    fn load_data_reads_project_savings_from_phase_runs() {
        let dir = TempDir::new().unwrap();
        let sessions = sessions_dir(dir.path());
        std::fs::create_dir_all(&sessions).unwrap();
        let pid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let run1 = format!(
            r#"{{"ts":1,"model":"t","generation_params":{{}},"phase_id":"p1","project_id":"{pid}","tags":[],"status":"complete","escalated":false,"gates":{{}},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{{"input_tokens":1000,"output_tokens":500}}}}"#
        );
        let run2 = format!(
            r#"{{"ts":2,"model":"t","generation_params":{{}},"phase_id":"p2","project_id":"{pid}","tags":[],"status":"complete","escalated":false,"gates":{{}},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{{"input_tokens":2000,"output_tokens":800}}}}"#
        );
        let telemetry_dir = dir.path().join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();
        std::fs::write(
            telemetry_dir.join("phase_runs.jsonl"),
            format!("{run1}\n{run2}\n"),
        )
        .unwrap();

        let data = load_data(dir.path(), None, Some(&telemetry_dir), Some(pid));
        assert_eq!(
            data.project_savings,
            (3000, 1300),
            "project savings must sum phase runs with matching project_id"
        );
        // No session phase id → no milestone match.
        assert!(data.milestone_savings.is_none());
    }

    #[test]
    fn load_data_project_savings_excludes_other_projects() {
        let dir = TempDir::new().unwrap();
        let sessions = sessions_dir(dir.path());
        std::fs::create_dir_all(&sessions).unwrap();
        let this_pid = "11111111-1111-1111-1111-111111111111";
        let other_pid = "22222222-2222-2222-2222-222222222222";
        let this_run = format!(
            r#"{{"ts":1,"model":"t","generation_params":{{}},"phase_id":"p1","project_id":"{this_pid}","tags":[],"status":"complete","escalated":false,"gates":{{}},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{{"input_tokens":1000,"output_tokens":500}}}}"#
        );
        let other_run = format!(
            r#"{{"ts":2,"model":"t","generation_params":{{}},"phase_id":"p2","project_id":"{other_pid}","tags":[],"status":"complete","escalated":false,"gates":{{}},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{{"input_tokens":9000,"output_tokens":4000}}}}"#
        );
        // Legacy record without project_id must be excluded.
        let legacy_run = r#"{"ts":3,"model":"t","generation_params":{},"phase_id":"p3","tags":[],"status":"complete","escalated":false,"gates":{},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{"input_tokens":500,"output_tokens":200}}"#;
        let telemetry_dir = dir.path().join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();
        std::fs::write(
            telemetry_dir.join("phase_runs.jsonl"),
            format!("{this_run}\n{other_run}\n{legacy_run}\n"),
        )
        .unwrap();

        let data = load_data(dir.path(), None, Some(&telemetry_dir), Some(this_pid));
        assert_eq!(
            data.project_savings,
            (1000, 500),
            "project savings must exclude runs from other project UUIDs and legacy records"
        );
    }

    #[test]
    fn load_data_project_savings_zero_when_no_project_id() {
        let dir = TempDir::new().unwrap();
        let sessions = sessions_dir(dir.path());
        std::fs::create_dir_all(&sessions).unwrap();
        let pid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let run = format!(
            r#"{{"ts":1,"model":"t","generation_params":{{}},"phase_id":"p1","project_id":"{pid}","tags":[],"status":"complete","escalated":false,"gates":{{}},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{{"input_tokens":1000,"output_tokens":500}}}}"#
        );
        let telemetry_dir = dir.path().join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();
        std::fs::write(telemetry_dir.join("phase_runs.jsonl"), format!("{run}\n")).unwrap();

        // project_id=None → savings are (0,0) regardless of what's in the store.
        let data = load_data(dir.path(), None, Some(&telemetry_dir), None);
        assert_eq!(
            data.project_savings,
            (0, 0),
            "project savings must be (0,0) when no project_id is configured"
        );
    }

    // --- milestone resolver + formatter tests ---

    #[test]
    fn format_milestone_name_capitalizes_words() {
        assert_eq!(
            format_milestone_name("M15-dashboard-polish-2"),
            "M15 — Dashboard Polish 2"
        );
        assert_eq!(format_milestone_name("M7-scorecard"), "M7 — Scorecard");
    }

    #[test]
    fn format_milestone_name_passthrough_for_nonstandard() {
        assert_eq!(format_milestone_name("scratch"), "scratch");
        assert_eq!(format_milestone_name("no-match"), "no-match");
    }

    #[test]
    fn milestone_number_parses_and_rejects() {
        assert_eq!(milestone_number("M15-dashboard-polish-2"), Some(15));
        assert_eq!(milestone_number("M7-scorecard"), Some(7));
        assert_eq!(milestone_number("scratch"), None);
        assert_eq!(milestone_number("MX-foo"), None);
    }

    #[test]
    fn resolve_milestone_prefers_active_milestone() {
        let dir = TempDir::new().unwrap();
        let milestones = dir.path().join("docs/dev/milestones");
        // M15 with done phase
        let m15 = milestones.join("M15-foo-bar");
        std::fs::create_dir_all(&m15).unwrap();
        std::fs::write(m15.join("phase-03-x.md"), "# Phase\n\n**Status:** done\n").unwrap();
        // M16 with in-progress phase
        let m16 = milestones.join("M16-baz");
        std::fs::create_dir_all(&m16).unwrap();
        std::fs::write(
            m16.join("phase-03-y.md"),
            "# Phase\n\n**Status:** in-progress\n",
        )
        .unwrap();

        let result = resolve_milestone(dir.path(), Some("phase-03"));
        assert_eq!(result, Some("M16 — Baz".to_string()));
    }

    #[test]
    fn resolve_milestone_falls_back_to_highest_when_none_active() {
        let dir = TempDir::new().unwrap();
        let milestones = dir.path().join("docs/dev/milestones");
        // Both done
        let m15 = milestones.join("M15-foo-bar");
        std::fs::create_dir_all(&m15).unwrap();
        std::fs::write(m15.join("phase-03-x.md"), "# Phase\n\n**Status:** done\n").unwrap();
        let m16 = milestones.join("M16-baz");
        std::fs::create_dir_all(&m16).unwrap();
        std::fs::write(m16.join("phase-03-y.md"), "# Phase\n\n**Status:** done\n").unwrap();

        let result = resolve_milestone(dir.path(), Some("phase-03"));
        assert_eq!(result, Some("M16 — Baz".to_string()));
    }

    #[test]
    fn resolve_milestone_active_lower_number_wins() {
        let dir = TempDir::new().unwrap();
        let milestones = dir.path().join("docs/dev/milestones");
        // M20 done
        let m20 = milestones.join("M20-old");
        std::fs::create_dir_all(&m20).unwrap();
        std::fs::write(m20.join("phase-03-x.md"), "# Phase\n\n**Status:** done\n").unwrap();
        // M16 active
        let m16 = milestones.join("M16-cur");
        std::fs::create_dir_all(&m16).unwrap();
        std::fs::write(
            m16.join("phase-03-y.md"),
            "# Phase\n\n**Status:** in-progress\n",
        )
        .unwrap();

        let result = resolve_milestone(dir.path(), Some("phase-03"));
        assert_eq!(result, Some("M16 — Cur".to_string()));
    }

    #[test]
    fn resolve_milestone_none_when_no_match() {
        let dir = TempDir::new().unwrap();
        // Empty milestones dir
        let milestones = dir.path().join("docs/dev/milestones");
        std::fs::create_dir_all(&milestones).unwrap();
        assert_eq!(resolve_milestone(dir.path(), Some("phase-03")), None);
        // phase = None
        assert_eq!(resolve_milestone(dir.path(), None), None);
    }
}
