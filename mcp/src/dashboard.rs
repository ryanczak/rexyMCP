//! Live dashboard — polls session logs and renders a paned TUI summary.
//!
//! Wraps `status::load_status` in a continuously-refreshed `ratatui` terminal
//! split into three panels: Session (identity/state), Heartbeat (liveness),
//! and Files (per-file diff numstat).

use std::path::Path;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::status::{self, StatusSummary};

/// Snapshot of the latest session data or an error loading it.
pub struct DashboardData {
    pub summary: StatusSummary,
    pub error: Option<String>,
}

/// Load the latest session data. Pure, testable.
pub fn load_data(repo: &Path, session: Option<&str>) -> DashboardData {
    match status::load_status(repo, session) {
        Ok(summary) => DashboardData {
            summary,
            error: None,
        },
        Err(e) => DashboardData {
            summary: StatusSummary::default(),
            error: Some(e),
        },
    }
}

// --- Per-panel content formatters (pure, testable) ---

/// Session panel: phase / session / model / state / turn / stage / freshness.
/// `now_ms` is injected (unix millis) so the age line is testable.
fn session_lines(summary: &StatusSummary, now_ms: u64) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let phase = summary.phase.as_deref().unwrap_or("<unknown>");
    let session = summary.session_id.as_deref().unwrap_or("<unknown>");
    lines.push(Line::from(format!("phase: {phase}")));
    lines.push(Line::from(format!("session: {session}")));

    if let Some(model) = &summary.model {
        lines.push(Line::from(format!("model: {model}")));
    }

    let state = match &summary.ended {
        Some(s) => format!("ended ({s})"),
        None => "running".to_string(),
    };
    lines.push(Line::from(Span::styled(
        format!("state: {state}"),
        Style::new()
            .add_modifier(Modifier::BOLD)
            .fg(if summary.ended.is_some() {
                Color::Yellow
            } else {
                Color::Green
            }),
    )));

    let stage = summary.latest_stage.as_deref().unwrap_or("<none>");
    lines.push(Line::from(format!(
        "turn {}, stage {stage}",
        summary.latest_turn
    )));

    if let Some(ts) = summary.last_ts {
        let age_ms = now_ms.saturating_sub(ts);
        lines.push(Line::from(format!(
            "last update: {} ago",
            status::humanize_age(age_ms)
        )));
    }

    lines
}

/// Compactions panel: count, freed tokens, compression ratio.
fn compactions_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    if summary.compaction_count == 0 {
        return vec![Line::from("(no compactions)")];
    }
    let before = summary.compaction_tokens_before;
    let after = summary.compaction_tokens_after;
    let mut lines = vec![
        Line::from(format!("events: {}", summary.compaction_count)),
        Line::from(format!("freed: {} tokens", before.saturating_sub(after))),
    ];
    if after != 0 {
        let ratio = before as f64 / after as f64;
        lines.push(Line::from(format!("ratio: {ratio:.1}x")));
    }
    lines
}

/// Total usable content width for a Files panel line (indent + path + space +
/// numstat). Conservative for the 28%-wide panel at typical terminal widths.
/// The path budget is computed per-entry as `FILE_LINE_MAX - 2 - 1 - numstat_width`,
/// so the total rendered line is always ≤ `FILE_LINE_MAX + 2` chars regardless of
/// how large the added/removed counts are.
const FILE_LINE_MAX: usize = 28;

/// Files panel: one line per changed file, or a placeholder when none.
fn files_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    if summary.files_changed.is_empty() {
        return vec![Line::from("(no files changed yet)")];
    }
    summary
        .files_changed
        .iter()
        .map(|f| {
            let numstat = format!("+{} -{}", f.added, f.removed);
            // Path budget = total line max − indent(2) − separator(1) − numstat.
            // This guarantees the numstat is always visible regardless of its width.
            let path_max = FILE_LINE_MAX.saturating_sub(1 + numstat.chars().count());
            Line::from(format!(
                "  {} {}",
                trim_path_left(&f.path, path_max),
                numstat
            ))
        })
        .collect()
}

/// Trim a file path to fit within `max` chars, always keeping the filename visible.
///
/// Three tiers:
/// 1. Full path fits → return unchanged.
/// 2. `…/{filename}` fits → return with directory trimmed.
/// 3. Filename itself is too long → left-trim the filename, returning `…{tail}`.
fn trim_path_left(path: &str, max: usize) -> String {
    if path.chars().count() <= max {
        return path.to_string();
    }
    let filename = path.rsplit('/').next().unwrap_or(path);
    let with_prefix = format!("…/{filename}");
    if with_prefix.chars().count() <= max {
        return with_prefix;
    }
    // Filename itself exceeds the budget — keep the rightmost tail.
    let available = max.saturating_sub(1); // one char for '…'
    let tail: String = filename
        .chars()
        .rev()
        .take(available)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
}

/// Activity panel: tool / verify / parse / hard-fail signals.
fn activity_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let has_any = summary.last_tool.is_some()
        || summary.last_verify_diagnostics.is_some()
        || summary.parse_failures > 0
        || summary.hard_fail_reason.is_some();

    if !has_any {
        return vec![Line::from("(no activity yet)")];
    }

    if let Some(name) = &summary.last_tool {
        let status = if summary.last_tool_ok.unwrap_or(true) {
            Span::styled("ok", Style::new().fg(Color::Green))
        } else {
            Span::styled("FAIL", Style::new().fg(Color::Red))
        };
        lines.push(Line::from(vec![format!("tool: {name} ").into(), status]));
    }

    if let Some(n) = summary.last_verify_diagnostics {
        if n == 0 {
            lines.push(Line::from(Span::styled(
                "verify: clean".to_string(),
                Style::new().fg(Color::Green),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                format!("verify: {n} diagnostic(s)"),
                Style::new().fg(Color::Red),
            )));
        }
    }

    if summary.parse_failures > 0 {
        lines.push(Line::from(Span::styled(
            format!("parse failures: {}", summary.parse_failures),
            Style::new().fg(Color::Yellow),
        )));
        if let Some(ref feedback) = summary.last_parse_feedback {
            let truncated: String = feedback.chars().take(80).collect();
            lines.push(Line::from(truncated));
        }
    }

    if let Some(reason) = &summary.hard_fail_reason {
        lines.push(Line::from(Span::styled(
            format!("HARD FAIL: {reason}"),
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        )));
    }

    lines
}

/// Budget panel: token counts and context-window gauge.
fn budget_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    if summary.last_input_tokens.is_none() {
        return vec![Line::from("(no metrics yet)")];
    }

    let in_toks = summary.last_input_tokens.unwrap_or(0);
    let out_toks = summary.last_output_tokens.unwrap_or(0);
    let mut lines = vec![
        Line::from(format!("tokens in:  {in_toks}")),
        Line::from(format!("tokens out: {out_toks}")),
    ];

    if let Some(pct) = summary.last_context_pct {
        if pct == 0.0 {
            lines.push(Line::from("context: — (unmeasured)"));
        } else {
            let pct_int = (pct * 100.0).round() as u32;
            let color = if pct_int < 50 {
                Color::Green
            } else if pct_int < 80 {
                Color::Yellow
            } else {
                Color::Red
            };
            lines.push(Line::from(Span::styled(
                format!("context: {pct_int}%"),
                Style::new().fg(color),
            )));
        }
    }

    lines
}

// --- Panel helpers ---

/// Wrap lines in a bordered `Block` with the given title.
fn panel(title: &'static str, lines: Vec<Line<'static>>) -> Paragraph<'static> {
    Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title))
}

// --- Renderer ---

/// Render the dashboard into a three-panel header band (Session · Budget ·
/// Compactions) above a body (Activity wide-left · Files right), or a
/// single error pane when `data.error` is set.
fn render_dashboard(frame: &mut Frame, area: Rect, data: &DashboardData, now_ms: u64) {
    if let Some(ref err) = data.error {
        let error_pane = panel(
            " Dashboard ",
            vec![Line::from(Span::styled(
                format!("Error: {err}"),
                Style::new().fg(Color::Red),
            ))],
        );
        frame.render_widget(error_pane, area);
        return;
    }

    // Outer split: fixed-height header band + filling body.
    let [header, body] =
        Layout::vertical([Constraint::Length(9), Constraint::Min(0)]).areas::<2>(area);

    // Header band: Session (phase/session/model/state/turn/stage/age) · Budget · Compactions.
    let [session_area, budget_area, compactions_area] = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
    ])
    .areas::<3>(header);

    frame.render_widget(
        panel(" Session ", session_lines(&data.summary, now_ms)),
        session_area,
    );
    frame.render_widget(panel(" Budget ", budget_lines(&data.summary)), budget_area);
    frame.render_widget(
        panel(" Compactions ", compactions_lines(&data.summary)),
        compactions_area,
    );

    // Body: Activity (wide-left) · Files (right).
    let [activity_area, files_area] =
        Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)])
            .areas::<2>(body);

    frame.render_widget(
        panel(" Activity ", activity_lines(&data.summary)),
        activity_area,
    );
    frame.render_widget(panel(" Files ", files_lines(&data.summary)), files_area);
}

// --- Entry points ---

/// Run the dashboard event loop. Called by `main.rs`.
pub fn run_dashboard(repo: &Path, session: Option<&str>) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, repo, session);
    ratatui::restore();
    result
}

fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    repo: &Path,
    session: Option<&str>,
) -> std::io::Result<()> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind};
    use std::time::Duration;

    loop {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let data = load_data(repo, session);
        terminal.draw(|frame| render_dashboard(frame, frame.area(), &data, now_ms))?;

        if event::poll(Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                _ => {}
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::sessions_dir;
    use rexymcp_executor::store::sessions::event::{FileNumstat, SessionEvent, SessionRecord};
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
        let data = load_data(dir.path(), None);
        assert!(data.error.is_some());
        assert!(data.summary.ended.is_none());
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

        let data = load_data(dir.path(), None);
        assert!(data.error.is_none());
        assert!(data.summary.phase.is_some());
        assert_eq!(data.summary.phase.as_deref(), Some("phase-01"));
    }

    // --- session_lines tests ---

    #[test]
    fn session_lines_shows_phase_session_separate_lines() {
        let summary = StatusSummary {
            phase: Some("phase-02".into()),
            session_id: Some("abc".into()),
            ended: None,
            ..StatusSummary::default()
        };
        let lines = session_lines(&summary, 0);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s == "phase: phase-02"));
        assert!(text.iter().any(|s| s == "session: abc"));
        assert!(text.iter().any(|s| s.contains("running")));
    }

    #[test]
    fn session_lines_shows_ended_state() {
        let summary = StatusSummary {
            phase: Some("phase-02".into()),
            ended: Some("complete".into()),
            ..StatusSummary::default()
        };
        let lines = session_lines(&summary, 0);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("ended (complete)")));
    }

    #[test]
    fn session_lines_shows_turn_stage_and_age() {
        let summary = StatusSummary {
            latest_turn: 5,
            latest_stage: Some("verify".into()),
            last_ts: Some(1000),
            ..StatusSummary::default()
        };
        let lines = session_lines(&summary, 4000);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("turn 5")));
        assert!(text.iter().any(|s| s.contains("verify")));
        assert!(text.iter().any(|s| s.contains("3s ago")));
    }

    #[test]
    fn session_lines_omits_age_when_no_ts() {
        let summary = StatusSummary {
            last_ts: None,
            ..StatusSummary::default()
        };
        let lines = session_lines(&summary, 9999);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(!text.iter().any(|s| s.contains("last update")));
    }

    // --- files_lines tests ---

    #[test]
    fn files_lines_lists_each_numstat() {
        let summary = StatusSummary {
            files_changed: vec![
                FileNumstat {
                    path: "src/a.rs".into(),
                    added: 10,
                    removed: 2,
                },
                FileNumstat {
                    path: "src/b.rs".into(),
                    added: 0,
                    removed: 3,
                },
            ],
            ..StatusSummary::default()
        };
        let lines = files_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(
            text.iter()
                .any(|s| s.contains("src/a.rs") && s.contains("+10") && s.contains("-2"))
        );
        assert!(
            text.iter()
                .any(|s| s.contains("src/b.rs") && s.contains("+0") && s.contains("-3"))
        );
    }

    #[test]
    fn files_lines_empty_placeholder() {
        let summary = StatusSummary::default();
        let lines = files_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("no files changed")));
    }

    // --- activity_lines tests ---

    #[test]
    fn activity_lines_shows_tool_and_verify() {
        let summary = StatusSummary {
            last_tool: Some("bash".into()),
            last_tool_ok: Some(true),
            last_verify_diagnostics: Some(2),
            ..StatusSummary::default()
        };
        let lines = activity_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("bash")));
        assert!(text.iter().any(|s| s.contains("2 diagnostic")));
    }

    #[test]
    fn activity_lines_empty_placeholder() {
        let summary = StatusSummary::default();
        let lines = activity_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("no activity")));
    }

    // --- budget_lines tests ---

    #[test]
    fn budget_lines_shows_tokens_and_context() {
        let summary = StatusSummary {
            last_input_tokens: Some(1200),
            last_output_tokens: Some(340),
            last_context_pct: Some(0.62),
            ..StatusSummary::default()
        };
        let lines = budget_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("1200")));
        assert!(text.iter().any(|s| s.contains("340")));
        assert!(text.iter().any(|s| s.contains("62%")));
    }

    #[test]
    fn budget_lines_unmeasured_when_zero_pct() {
        let summary = StatusSummary {
            last_input_tokens: Some(10),
            last_output_tokens: Some(5),
            last_context_pct: Some(0.0),
            ..StatusSummary::default()
        };
        let lines = budget_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("unmeasured")));
        assert!(!text.iter().any(|s| s.contains("0%")));
    }

    #[test]
    fn budget_lines_empty_placeholder() {
        let summary = StatusSummary::default();
        let lines = budget_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("no metrics")));
    }

    // --- compactions_lines tests ---

    #[test]
    fn compactions_lines_empty_placeholder() {
        let summary = StatusSummary::default();
        let lines = compactions_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("no compactions")));
    }

    #[test]
    fn compactions_lines_shows_events_and_ratio() {
        let summary = StatusSummary {
            compaction_count: 2,
            compaction_tokens_before: 1000,
            compaction_tokens_after: 600,
            ..StatusSummary::default()
        };
        let lines = compactions_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("events: 2")));
        assert!(text.iter().any(|s| s.contains("freed: 400")));
        assert!(text.iter().any(|s| s.contains("1.7x")));
    }

    #[test]
    fn compactions_lines_omits_ratio_when_after_zero() {
        let summary = StatusSummary {
            compaction_count: 1,
            compaction_tokens_before: 500,
            compaction_tokens_after: 0,
            ..StatusSummary::default()
        };
        let lines = compactions_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("events: 1")));
        assert!(text.iter().any(|s| s.contains("freed: 500")));
        assert!(!text.iter().any(|s| s.contains("x")));
    }

    // --- files_lines trim tests ---

    #[test]
    fn files_lines_trims_long_path_left() {
        // Path longer than FILE_LINE_MAX; filename "chars.rs" is short enough to fit.
        let long_path = "a/very/deeply/nested/path/that/is/definitely/longer/forty/chars.rs";
        let summary = StatusSummary {
            files_changed: vec![FileNumstat {
                path: long_path.into(),
                added: 5,
                removed: 1,
            }],
            ..StatusSummary::default()
        };
        let lines = files_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert_eq!(text.len(), 1);
        let line = &text[0];
        // Numstat always visible.
        assert!(
            line.ends_with(" +5 -1"),
            "numstat must always be visible: {line}"
        );
        // Filename preserved via "…/{filename}" trimming.
        assert!(
            line.contains("chars.rs"),
            "filename must be visible: {line}"
        );
        assert!(
            !line.contains("nested"),
            "intermediate dirs must be trimmed: {line}"
        );
        // Path portion fits within FILE_LINE_MAX.
        let path_part = line
            .strip_prefix("  ")
            .unwrap()
            .strip_suffix(" +5 -1")
            .unwrap();
        assert!(
            path_part.chars().count() <= FILE_LINE_MAX,
            "path portion must fit within FILE_LINE_MAX ({FILE_LINE_MAX}): '{path_part}' ({} chars)",
            path_part.chars().count()
        );
    }

    #[test]
    fn files_lines_trims_long_filename_from_left() {
        // Filename alone exceeds FILE_LINE_MAX — must be left-trimmed too.
        let long_filename = "a_very_long_filename_that_definitely_exceeds_the_budget_limit.rs";
        let path = format!("src/{long_filename}");
        let summary = StatusSummary {
            files_changed: vec![FileNumstat {
                path: path.clone(),
                added: 2,
                removed: 0,
            }],
            ..StatusSummary::default()
        };
        let lines = files_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        let line = &text[0];
        // Numstat always visible.
        assert!(
            line.ends_with(" +2 -0"),
            "numstat must always be visible: {line}"
        );
        // Path starts with '…' (was trimmed).
        assert!(line.starts_with("  …"), "must start with ellipsis: {line}");
        // Path portion fits within FILE_LINE_MAX.
        let path_part = line
            .strip_prefix("  ")
            .unwrap()
            .strip_suffix(" +2 -0")
            .unwrap();
        assert!(
            path_part.chars().count() <= FILE_LINE_MAX,
            "path portion must fit within FILE_LINE_MAX ({FILE_LINE_MAX}): '{path_part}'"
        );
    }

    #[test]
    fn files_lines_keeps_short_path_untrimmed() {
        let summary = StatusSummary {
            files_changed: vec![FileNumstat {
                path: "src/a.rs".into(),
                added: 3,
                removed: 0,
            }],
            ..StatusSummary::default()
        };
        let lines = files_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert_eq!(text.len(), 1);
        assert!(
            !text[0].contains('…'),
            "short path should not contain ellipsis: {}",
            text[0]
        );
        assert!(text[0].contains("src/a.rs"));
    }
}
