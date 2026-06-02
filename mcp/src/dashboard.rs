//! Live dashboard — polls session logs and renders a TUI summary pane.
//!
//! Wraps `status::load_status` in a continuously-refreshed `ratatui` terminal.

use std::path::Path;

use crate::status::{self, StatusSummary};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

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

/// Render the dashboard summary into a single bordered pane.
fn render_summary(frame: &mut Frame, area: Rect, data: &DashboardData) {
    let lines = if let Some(ref err) = data.error {
        vec![Line::from(Span::styled(
            format!("Error: {err}"),
            Style::new().fg(Color::Red),
        ))]
    } else {
        format_summary_lines(&data.summary)
    };

    let paragraph =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Dashboard "));
    frame.render_widget(paragraph, area);
}

/// Format the summary into TUI lines. Mirrors `status::format_status`.
fn format_summary_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let phase = summary.phase.as_deref().unwrap_or("<unknown>");
    let session = summary.session_id.as_deref().unwrap_or("<unknown>");
    lines.push(Line::from(format!("phase: {phase}  session: {session}")));

    if let Some(model) = &summary.model {
        lines.push(Line::from(format!("model: {model}")));
    }

    let state = match &summary.ended {
        Some(status) => format!("ended ({status})"),
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

    if let Some(msg) = &summary.latest_message {
        lines.push(Line::from(msg.clone()));
    }

    if !summary.files_changed.is_empty() {
        lines.push(Line::from(""));
        for f in &summary.files_changed {
            lines.push(Line::from(format!(
                "  {} +{} -{}",
                f.path, f.added, f.removed
            )));
        }
    }

    lines
}

/// Run the dashboard event loop. This is the main entry point called by `main.rs`.
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
        let data = load_data(repo, session);
        terminal.draw(|frame| render_summary(frame, frame.area(), &data))?;

        if event::poll(Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                _ => {}
            }
        }

        if data.summary.ended.is_some() {
            terminal.draw(|frame| render_summary(frame, frame.area(), &data))?;
            std::thread::sleep(Duration::from_secs(2));
            break;
        }
    }

    Ok(())
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

    #[test]
    fn format_summary_lines_shows_error_style() {
        let summary = StatusSummary::default();
        let lines = format_summary_lines(&summary);
        assert!(!lines.is_empty());
        let first = &lines[0];
        assert!(format!("{first}").contains("phase: <unknown>"));
    }
}
