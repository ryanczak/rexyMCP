use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use super::filter::{ActivityFilter, FILTER_ITEM_COUNT, FilterState};
use super::panels::{
    BudgetRates, budget_lines, compactions_lines, dollars_saved_line, files_lines, panel,
    session_lines,
};
use super::transcript::transcript_lines;
use crate::dashboard::DashboardData;

/// View-state for the dashboard activity pane.
pub(crate) struct ViewState {
    pub(crate) offset: u16,
    pub(crate) follow: bool,
    pub(crate) spinner: Option<usize>,
    pub(crate) filter: FilterState,
}

/// Clamp a scroll offset so it can't run past the last line.
pub(crate) fn clamp_scroll(scroll: u16, total_lines: usize) -> u16 {
    let max = total_lines.saturating_sub(1) as u16;
    scroll.min(max)
}

/// Resolve the scroll offset to display. `follow` pins to the bottom (newest):
/// the offset that shows the last `viewport` lines. Otherwise the manual `offset`
/// is clamped so it can't scroll past the bottom.
pub(crate) fn visible_offset(follow: bool, offset: u16, total_lines: usize, viewport: u16) -> u16 {
    let total = total_lines.min(u16::MAX as usize) as u16;
    let max = total.saturating_sub(viewport);
    if follow { max } else { offset.min(max) }
}

/// Render the dashboard into a three-panel header band (Session · Budget ·
/// Compactions) above a body (Activity wide-left · Files right), or a
/// single error pane when `data.error` is set.
/// Transcript is newest-first when `follow` is true (tail-pinned).
pub(crate) fn render_dashboard(
    frame: &mut Frame,
    area: Rect,
    data: &DashboardData,
    now_ms: u64,
    state: &ViewState,
    rates: BudgetRates,
) {
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

    // Header band: Session · Budget · Compactions.
    // Budget uses Min(56) so the combined tok/s line
    // "tok/s: X.X  (avg: X.X, max: X.X, min: X.X)" fits without wrapping.
    // Session uses Fill(1) so it yields width to Budget when the terminal is
    // narrow; Compactions takes whatever remains.
    let [session_area, budget_area, compactions_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Min(56),
        Constraint::Fill(1),
    ])
    .areas::<3>(header);

    frame.render_widget(
        panel(" Session ", session_lines(&data.summary, now_ms)),
        session_area,
    );
    let mut budget = budget_lines(&data.summary);
    if let Some(line) = dollars_saved_line(&data.summary, rates) {
        budget.push(line);
    }
    frame.render_widget(panel(" Budget ", budget), budget_area);
    frame.render_widget(
        panel(" Compactions ", compactions_lines(&data.summary)),
        compactions_area,
    );

    // Body: Activity (wide-left) · Files (right).
    let [activity_area, files_area] =
        Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)])
            .areas::<2>(body);

    let filter_state = &state.filter;
    if filter_state.open {
        // Filter panel replaces the transcript while open.
        let mut filter_lines: Vec<Line<'static>> = (0..FILTER_ITEM_COUNT)
            .map(|i| {
                let check = if filter_state.filter.is_enabled(i) {
                    "✓"
                } else {
                    "✗"
                };
                let label = ActivityFilter::item_label(i);
                let text = format!(" {check}  {label}");
                if i == filter_state.cursor {
                    Line::from(Span::styled(
                        text,
                        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(text)
                }
            })
            .collect();
        filter_lines.push(Line::from(Span::styled(
            " ↑↓/jk move · space toggle · f/Esc close",
            Style::new().fg(Color::DarkGray),
        )));
        frame.render_widget(
            Paragraph::new(filter_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Activity [filter] "),
            ),
            activity_area,
        );
    } else {
        let transcript = transcript_lines(&data.records, &filter_state.filter, state.spinner);
        let viewport = activity_area.height.saturating_sub(2); // minus top+bottom border
        let n = transcript.len();
        let scroll = visible_offset(state.follow, state.offset, n, viewport);
        frame.render_widget(
            Paragraph::new(transcript).scroll((scroll, 0)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Activity [f=filter] "),
            ),
            activity_area,
        );
    }
    frame.render_widget(panel(" Files ", files_lines(&data.summary)), files_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- visible_offset tests ---

    #[test]
    fn visible_offset_follows_tail() {
        assert_eq!(visible_offset(true, 0, 100, 20), 80);
        assert_eq!(visible_offset(true, 0, 10, 20), 0);
    }

    #[test]
    fn visible_offset_manual_clamped() {
        assert_eq!(visible_offset(false, 999, 100, 20), 80);
        assert_eq!(visible_offset(false, 5, 100, 20), 5);
    }

    // --- clamp_scroll tests ---

    #[test]
    fn clamp_scroll_bounds_to_last_line() {
        assert_eq!(clamp_scroll(5, 3), 2);
        assert_eq!(clamp_scroll(0, 0), 0);
        assert_eq!(clamp_scroll(10, 100), 10);
        assert_eq!(clamp_scroll(0, 1), 0);
    }
}
