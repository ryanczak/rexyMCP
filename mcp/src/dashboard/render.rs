use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

use super::filter::{ActivityFilter, FILTER_ITEM_COUNT, FilterState};
use super::panels::{
    BudgetRates, budget_lines, files_lines, milestone_line, panel, reclaim_lines, savings_lines,
    session_lines, spinner_line, tasks_lines,
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

/// Wrap one styled line to `width` columns on **word boundaries**, preserving
/// span styles. A word — a maximal run of non-space chars — is never split
/// across rows when it would fit on a row by itself; it moves whole to the next
/// row instead. A word longer than `width` has no fitting row, so it is
/// hard-split to fill each row (the prior mid-word behavior, now only the
/// fallback). Spaces are placed as encountered, so no characters are dropped and
/// concatenating all rows reproduces the input. `width == 0` or an empty line
/// returns a single row unchanged. Char-count based (not unicode display width).
pub(crate) fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![line.clone()];
    }
    let chars: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
        .collect();
    if chars.is_empty() {
        return vec![line.clone()];
    }

    let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
    let mut cur: Vec<(char, Style)> = Vec::new();
    let mut col = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i].0 == ' ' {
            // Space: place it; break only when the row is already full.
            if col == width {
                rows.push(std::mem::take(&mut cur));
                col = 0;
            }
            cur.push(chars[i]);
            col += 1;
            i += 1;
            continue;
        }
        // Measure the next word (a run of non-space chars).
        let start = i;
        while i < chars.len() && chars[i].0 != ' ' {
            i += 1;
        }
        let word = &chars[start..i];
        if word.len() <= width {
            // Word fits on a row: break before it if it won't fit on this one.
            if col + word.len() > width {
                rows.push(std::mem::take(&mut cur));
                col = 0;
            }
            cur.extend_from_slice(word);
            col += word.len();
        } else {
            // Word longer than any row: hard-split to fill each row.
            for &c in word {
                if col == width {
                    rows.push(std::mem::take(&mut cur));
                    col = 0;
                }
                cur.push(c);
                col += 1;
            }
        }
    }
    rows.push(cur);

    rows.into_iter().map(row_to_line).collect()
}

/// Coalesce a row of styled chars into a `Line`, merging adjacent equal-styled
/// chars into a single span (so the span count matches the prior behavior).
fn row_to_line(row: Vec<(char, Style)>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut cur_style: Option<Style> = None;
    for (ch, style) in row {
        if cur_style != Some(style) {
            if let Some(s) = cur_style {
                spans.push(Span::styled(std::mem::take(&mut buf), s));
            }
            cur_style = Some(style);
        }
        buf.push(ch);
    }
    if let Some(s) = cur_style {
        spans.push(Span::styled(buf, s));
    }
    Line::from(spans)
}

/// Wrap every line in `lines` to `width` characters with a hanging indent: the
/// **first** row of each source line has no prefix (flush left); **continuation**
/// rows get `continuation_indent` spaces prepended. Content width =
/// `width − continuation_indent` for all rows so continuations fit within `width`.
pub(crate) fn wrap_lines_hanging(
    lines: &[Line<'static>],
    width: usize,
    continuation_indent: usize,
) -> Vec<Line<'static>> {
    let content_width = width.saturating_sub(continuation_indent);
    lines
        .iter()
        .flat_map(|l| {
            wrap_line(l, content_width)
                .into_iter()
                .enumerate()
                .map(move |(i, row)| {
                    if i == 0 || continuation_indent == 0 {
                        row
                    } else {
                        let mut spans = Vec::with_capacity(row.spans.len() + 1);
                        spans.push(Span::raw(" ".repeat(continuation_indent)));
                        spans.extend(row.spans);
                        Line::from(spans)
                    }
                })
        })
        .collect()
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
) -> usize {
    if let Some(ref err) = data.error {
        let error_pane = panel(
            " Dashboard ",
            vec![Line::from(Span::styled(
                format!("Error: {err}"),
                Style::new().fg(Color::Red),
            ))],
        );
        frame.render_widget(error_pane, area);
        return 0;
    }

    // Outer split: fixed-height header band + filling body.
    // Height 11 = 2 border rows + 9 content rows. The 9 content rows is the tallest
    // panel's exact need: Session = Milestone/Phase/Session/Model/State/Duration/
    // last-update/Turn-stage/spinner; Budget = Tokens-in/out + Tok/s (3) plus the
    // savings block (header + Baseline/Executor/Architect/Net + Assists = 6). Budget
    // and Context share this height; the body (Activity · Tasks/Files) fills the rest.
    let total_wrapped;
    let [header, body] =
        Layout::vertical([Constraint::Length(11), Constraint::Min(0)]).areas::<2>(area);

    // Header band: Session · Budget · Compactions.
    // Budget uses Min(52) so the combined tok/s line
    // "tok/s: X.X  (avg: X.X, max: X.X, min: X.X)" fits without wrapping.
    // Session uses Fill(1) so it yields width to Budget when the terminal is
    // narrow; Compactions uses Percentage(28) to mirror the Files panel below,
    // aligning the Budget/Compactions border with the Activity/Files border.
    let [session_area, budget_area, compactions_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Min(52),
        Constraint::Percentage(28),
    ])
    .areas::<3>(header);

    let session_inner_width = session_area.width.saturating_sub(2) as usize;
    let mut session = Vec::new();
    if let Some(name) = &data.milestone {
        session.push(milestone_line(name, session_inner_width));
    }
    session.extend(session_lines(&data.summary, now_ms));
    if let Some(line) = spinner_line(state.spinner, session_inner_width) {
        session.push(line);
    }
    frame.render_widget(panel(" Session ", session), session_area);
    let mut budget = Vec::new();
    budget.extend(budget_lines(&data.summary));
    budget.extend(savings_lines(
        &data.summary,
        rates,
        data.milestone_costs,
        data.project_costs,
        data.project_escalation_count,
    ));
    frame.render_widget(panel(" Budget ", budget), budget_area);
    frame.render_widget(
        panel(" Context ", reclaim_lines(&data.summary)),
        compactions_area,
    );

    // Body: Activity (wide-left) · right column (Tasks over Files).
    let [activity_area, right_area] =
        Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)])
            .areas::<2>(body);
    let [tasks_area, files_area] =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas::<2>(right_area);

    let filter_state = &state.filter;
    // Hanging indent: [tNN] header rows flush left, continuation rows indented 4 chars.
    // 4-char right gutter so text doesn't run flush to the panel border.
    const INDENT: usize = 4;
    let inner_width = activity_area.width.saturating_sub(2) as usize;
    let wrap_width = inner_width.saturating_sub(4); // leaves 4-char right gutter

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
            Style::new().fg(Color::Rgb(200, 200, 200)),
        )));
        frame.render_widget(
            Paragraph::new(filter_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Activity [filter] "),
            ),
            activity_area,
        );
        total_wrapped = wrap_lines_hanging(
            &transcript_lines(&data.records, &filter_state.filter),
            wrap_width,
            INDENT,
        )
        .len();
    } else {
        let transcript = transcript_lines(&data.records, &filter_state.filter);
        let wrapped = wrap_lines_hanging(&transcript, wrap_width, INDENT);
        total_wrapped = wrapped.len();
        let viewport = activity_area.height.saturating_sub(2);
        let scroll = visible_offset(state.follow, state.offset, total_wrapped, viewport);
        frame.render_widget(
            Paragraph::new(wrapped).scroll((scroll, 0)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Activity [f=filter] "),
            ),
            activity_area,
        );
        let mut sb_state = ScrollbarState::new(total_wrapped)
            .viewport_content_length(viewport as usize)
            .position(scroll as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            activity_area.inner(Margin::new(0, 1)),
            &mut sb_state,
        );
    }
    let tasks_inner_width = tasks_area.width.saturating_sub(2) as usize;
    frame.render_widget(
        panel(
            " Tasks ",
            tasks_lines(&data.summary, tasks_inner_width, state.spinner),
        ),
        tasks_area,
    );
    let files_inner_width = files_area.width.saturating_sub(2) as usize;
    frame.render_widget(
        panel(" Files ", files_lines(&data.summary, files_inner_width)),
        files_area,
    );
    total_wrapped
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

    // --- wrap_line / wrap_lines tests ---

    #[test]
    fn wrap_line_splits_long_line_into_rows() {
        let line = Line::from(Span::styled("a".repeat(25), Style::new().fg(Color::Red)));
        let rows = wrap_line(&line, 10);
        assert_eq!(rows.len(), 3);
        let joined: String = rows
            .iter()
            .flat_map(|r| r.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert_eq!(joined, "a".repeat(25));
        assert_eq!(rows[0].spans[0].style.fg, Some(Color::Red));
    }

    #[test]
    fn wrap_line_keeps_short_line_intact() {
        let line = Line::from("hello");
        let rows = wrap_line(&line, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].spans[0].content.as_ref(), "hello");
    }

    #[test]
    fn wrap_line_zero_width_is_noop() {
        let line = Line::from("hello");
        let rows = wrap_line(&line, 0);
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn wrap_line_preserves_multispan_styles() {
        let line = Line::from(vec![
            Span::styled("aa", Style::new().fg(Color::Red)),
            Span::styled("bbbb", Style::new().fg(Color::Blue)),
        ]);
        let rows = wrap_line(&line, 3);
        assert_eq!(rows.len(), 2);
        // Row 0: "aa" (red) + "b" (blue, split from "bbbb")
        assert_eq!(rows[0].spans.len(), 2);
        assert_eq!(rows[0].spans[0].style.fg, Some(Color::Red));
        assert_eq!(rows[0].spans[1].style.fg, Some(Color::Blue));
        // Row 1: "bb" (blue, remainder of "bbbb")
        assert_eq!(rows[1].spans.len(), 1);
        assert_eq!(rows[1].spans[0].style.fg, Some(Color::Blue));
    }

    #[test]
    fn wrap_lines_hanging_no_row_exceeds_width() {
        let lines = vec![Line::from("short"), Line::from("x".repeat(23))];
        let width = 10;
        let wrapped = wrap_lines_hanging(&lines, width, 0);
        for row in &wrapped {
            let char_count: usize = row.spans.iter().map(|s| s.content.chars().count()).sum();
            assert!(
                char_count <= width,
                "row exceeds width: {char_count} > {width}"
            );
        }
    }

    #[test]
    fn wrap_lines_hanging_total_drives_follow_offset() {
        let lines = vec![Line::from("hdr"), Line::from("x".repeat(25))];
        let total = wrap_lines_hanging(&lines, 10, 0).len();
        assert_eq!(total, 4); // "hdr" = 1 row, 25 chars at width 10 = 3 rows
        let viewport: u16 = 1;
        assert_eq!(visible_offset(true, 0, total, viewport), 3);
        assert_eq!(visible_offset(true, 0, lines.len(), viewport), 1);
    }

    #[test]
    fn wrap_lines_hanging_first_row_has_no_indent() {
        // 14-wide with 4-char continuation_indent → content_width=10; 25-char line wraps to 3 rows.
        let line = Line::from("x".repeat(25));
        let rows = wrap_lines_hanging(&[line], 14, 4);
        assert_eq!(rows.len(), 3);
        // Row 0: no indent prefix — first span is content, not spaces.
        assert_ne!(
            rows[0].spans[0].content.as_ref(),
            "    ",
            "first row must NOT start with the indent"
        );
        let first_content: usize = rows[0]
            .spans
            .iter()
            .map(|s| s.content.chars().count())
            .sum();
        assert_eq!(first_content, 10); // content_width only, no indent
    }

    #[test]
    fn wrap_lines_hanging_continuations_are_indented() {
        // Same setup: continuations (rows 1+) must start with the 4-space indent.
        let line = Line::from("x".repeat(25));
        let rows = wrap_lines_hanging(&[line], 14, 4);
        assert_eq!(rows.len(), 3);
        for row in &rows[1..] {
            assert_eq!(
                row.spans[0].content.as_ref(),
                "    ",
                "continuation rows must start with the indent"
            );
            let total: usize = row.spans.iter().map(|s| s.content.chars().count()).sum();
            assert!(total <= 14, "continuation row exceeds 14 chars: {total}");
            assert!(total > 4, "continuation row is indent-only: {total}");
        }
    }

    #[test]
    fn wrap_line_breaks_on_word_boundary() {
        let line = Line::from("hello world foo");
        let rows = wrap_line(&line, 8);
        let row_texts: Vec<String> = rows.iter().map(|r| format!("{r}")).collect();
        // "hello" (5) + " " (1) = 6 chars, "world" (5) won't fit (6 + 5 > 8),
        // so "world" moves to its own row.
        assert!(
            row_texts.iter().any(|t| t.contains("hello")),
            "some row should contain 'hello'"
        );
        assert!(
            row_texts.iter().any(|t| t.trim() == "world"),
            "'world' should occupy its own row"
        );
        assert!(
            row_texts.iter().any(|t| t.contains("foo")),
            "some row should contain 'foo'"
        );
    }

    #[test]
    fn wrap_line_hard_splits_word_longer_than_width() {
        let line = Line::from("supercalifragi");
        let rows = wrap_line(&line, 8);
        assert_eq!(rows.len(), 2, "should produce 2 rows");
        let joined: String = rows
            .iter()
            .flat_map(|r| r.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert_eq!(joined, "supercalifragi");
    }

    #[test]
    fn wrap_line_word_boundary_preserves_styles() {
        let line = Line::from(vec![
            Span::styled("hello", Style::new().fg(Color::Red)),
            Span::raw(" "),
            Span::styled("world", Style::new().fg(Color::Blue)),
        ]);
        let rows = wrap_line(&line, 8);
        // Find the row containing "world" and check its style.
        let world_row = rows
            .iter()
            .find(|r| format!("{r}").contains("world"))
            .expect("should have a row with 'world'");
        let world_span = world_row
            .spans
            .iter()
            .find(|s| s.content.as_ref().contains("world"))
            .expect("should have a span with 'world'");
        assert_eq!(
            world_span.style.fg,
            Some(Color::Blue),
            "'world' should carry blue style"
        );
    }
}
