//! Live dashboard — polls session logs and renders a paned TUI summary.
//!
//! Continuously refreshes a `ratatui` terminal with a header band (Session ·
//! Budget · Compactions) above a body (Activity · Files).

use std::path::Path;
use std::sync::OnceLock;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::{SyntaxReference, SyntaxSet};

use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};

use crate::status::{self, StatusSummary};

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

/// Snapshot of the latest session data or an error loading it.
pub struct DashboardData {
    pub summary: StatusSummary,
    pub records: Vec<SessionRecord>,
    pub error: Option<String>,
}

/// Load the latest session data. Pure, testable.
pub fn load_data(repo: &Path, session: Option<&str>) -> DashboardData {
    match status::load_records(repo, session) {
        Ok(records) => DashboardData {
            summary: status::summarize(&records),
            records,
            error: None,
        },
        Err(e) => DashboardData {
            summary: StatusSummary::default(),
            records: Vec::new(),
            error: Some(e),
        },
    }
}

/// Max chars of free-text content shown per transcript line in 10a (10b expands
/// to full multi-line). Keeps one record = one line.
const TRANSCRIPT_PREVIEW_MAX: usize = 100;

/// Max content lines shown per record before collapsing the rest into a
/// "… (N more lines)" marker. Keeps one large tool output from flooding the panel.
const TRANSCRIPT_CONTENT_MAX_LINES: usize = 20;

// TODO: activity filter — a key binding opens a config dialog to show/hide event
// types (Prompt, Completion, ToolCall, Verify, …). Deferred: requires a modal
// overlay / input handling that doesn't exist yet.

/// Detect a syntax definition from content alone (no filename available).
/// Returns `None` when no language can be confidently identified, which
/// causes the caller to fall back to unstyled DarkGray text.
fn detect_syntax<'a>(content: &str, ss: &'a SyntaxSet) -> Option<&'a SyntaxReference> {
    let trimmed = content.trim();

    // Shebangs and other first-line markers (e.g. `#!/usr/bin/env python`).
    if let Some(s) = ss.find_syntax_by_first_line(content) {
        return Some(s);
    }

    // Unified diff: git diff header or classic --- / +++ opener.
    if (trimmed.starts_with("diff --git") || trimmed.starts_with("---"))
        && let Some(s) = ss.find_syntax_by_extension("diff")
    {
        return Some(s);
    }

    // JSON: curly-brace or array open.
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && let Some(s) = ss.find_syntax_by_extension("json")
    {
        return Some(s);
    }

    // TOML: at least one `[section]` line (check before Rust to avoid false positives).
    let has_toml_section = content.lines().any(|l| {
        let l = l.trim();
        l.starts_with('[') && l.ends_with(']') && l.len() > 2
    });
    if has_toml_section && let Some(s) = ss.find_syntax_by_extension("toml") {
        return Some(s);
    }

    // Rust: 2+ keyword markers present.
    let rust_score = [
        "fn ", "pub ", "use ", "impl ", "struct ", "enum ", "let mut ", "match ",
    ]
    .iter()
    .filter(|&&m| content.contains(m))
    .count();
    if rust_score >= 2
        && let Some(s) = ss.find_syntax_by_extension("rs")
    {
        return Some(s);
    }

    None
}

/// Render `content` as indented, syntax-highlighted lines.
/// Falls back to DarkGray when no language is detected.
fn highlighted_body_lines(content: &str) -> Vec<Line<'static>> {
    let ss = syntax_set();

    let Some(syntax) = detect_syntax(content, ss) else {
        return body_lines(content)
            .into_iter()
            .map(|l| Line::from(Span::styled(l, Style::new().fg(Color::DarkGray))))
            .collect();
    };

    let theme = &theme_set().themes["base16-ocean.dark"];
    let mut h = HighlightLines::new(syntax, theme);

    let all: Vec<&str> = content.lines().collect();
    let capped = all.len().min(TRANSCRIPT_CONTENT_MAX_LINES);
    let overflow = all.len().saturating_sub(TRANSCRIPT_CONTENT_MAX_LINES);

    let mut result: Vec<Line<'static>> = Vec::new();
    for &line in &all[..capped] {
        let line_nl = format!("{line}\n");
        let ranges = h.highlight_line(&line_nl, ss).unwrap_or_default();
        let mut spans = vec![Span::raw("    ")];
        for (style, text) in ranges {
            let text = text.trim_end_matches('\n').to_string();
            if text.is_empty() {
                continue;
            }
            spans.push(Span::styled(
                text,
                Style::new().fg(Color::Rgb(
                    style.foreground.r,
                    style.foreground.g,
                    style.foreground.b,
                )),
            ));
        }
        result.push(Line::from(spans));
    }
    if overflow > 0 {
        result.push(Line::from(Span::styled(
            format!("    … ({overflow} more lines)"),
            Style::new().fg(Color::DarkGray),
        )));
    }

    result
}

/// Render `content` as indented lines, all in the same `color`.
fn plain_body_lines(content: &str, color: Color) -> Vec<Line<'static>> {
    body_lines(content)
        .into_iter()
        .map(|l| Line::from(Span::styled(l, Style::new().fg(color))))
        .collect()
}

/// Build all transcript lines for the given records, in chronological order.
/// Returns a placeholder when there are no records.
fn transcript_lines(records: &[SessionRecord]) -> Vec<Line<'static>> {
    if records.is_empty() {
        return vec![Line::from("(no activity yet)")];
    }
    records.iter().flat_map(record_lines).collect()
}

/// Split `body` on newlines into indented display lines, capped at
/// `TRANSCRIPT_CONTENT_MAX_LINES` with a trailing overflow marker when longer.
fn body_lines(body: &str) -> Vec<String> {
    let all: Vec<&str> = body.split('\n').collect();
    if all.len() <= TRANSCRIPT_CONTENT_MAX_LINES {
        all.iter().map(|l| format!("    {l}")).collect()
    } else {
        let mut out: Vec<String> = all
            .iter()
            .take(TRANSCRIPT_CONTENT_MAX_LINES)
            .map(|l| format!("    {l}"))
            .collect();
        out.push(format!(
            "    … ({} more lines)",
            all.len() - TRANSCRIPT_CONTENT_MAX_LINES
        ));
        out
    }
}

/// Render one record as one or more transcript lines (header + optional body),
/// styled by event type. Completion and ToolResult expand their content across
/// multiple lines; all other events are a single styled header line.
fn record_lines(rec: &SessionRecord) -> Vec<Line<'static>> {
    // (header_summary, header_color, bold, body_lines)
    let (summary, color, bold, body): (String, Color, bool, Option<Vec<Line<'static>>>) =
        match &rec.event {
            SessionEvent::SessionStart { model, phase, .. } => (
                format!("session start — phase {phase}, model {model}"),
                Color::Cyan,
                false,
                None,
            ),
            SessionEvent::Prompt { rendered } => (
                format!("prompt ({} chars)", rendered.chars().count()),
                Color::DarkGray,
                false,
                None,
            ),
            // LLM completions: soft white so the model's words read easily.
            SessionEvent::Completion { raw } => (
                "completion:".to_string(),
                Color::Reset,
                false,
                Some(plain_body_lines(raw, Color::Rgb(180, 180, 180))),
            ),
            SessionEvent::Parsed { tool_call } => (
                format!("→ call {}", tool_call.name),
                Color::Blue,
                false,
                None,
            ),
            SessionEvent::ParseFailed { failure } => (
                format!("parse failed: {}", preview(&failure.feedback)),
                Color::Red,
                false,
                None,
            ),
            SessionEvent::ToolResult {
                name,
                succeeded,
                output_preview,
            } => {
                let status = if *succeeded { "ok" } else { "FAIL" };
                let color = if *succeeded { Color::Green } else { Color::Red };
                (
                    format!("tool {name} [{status}]"),
                    color,
                    false,
                    Some(highlighted_body_lines(output_preview)),
                )
            }
            SessionEvent::Verify { diagnostics } => {
                let color = if diagnostics.is_empty() {
                    Color::Green
                } else {
                    Color::Red
                };
                (
                    format!("verify: {} diagnostic(s)", diagnostics.len()),
                    color,
                    false,
                    None,
                )
            }
            SessionEvent::HardFail { reason } => {
                (format!("HARD FAIL: {reason}"), Color::Red, true, None)
            }
            SessionEvent::Progress { stage, .. } => {
                (format!("progress: {stage}"), Color::DarkGray, false, None)
            }
            SessionEvent::SessionEnd { status, turns } => (
                format!("session end — {status} ({turns} turns)"),
                Color::Cyan,
                false,
                None,
            ),
            SessionEvent::Metrics {
                input_tokens,
                output_tokens,
                ..
            } => (
                format!("metrics: {input_tokens} in / {output_tokens} out"),
                Color::DarkGray,
                false,
                None,
            ),
            SessionEvent::Compaction {
                tokens_before,
                tokens_after,
                ..
            } => (
                format!("compaction: {tokens_before} → {tokens_after} tokens"),
                Color::Magenta,
                false,
                None,
            ),
        };

    let header_text = format!("[t{}] {}", rec.turn, summary);
    let mut style = Style::new().fg(color);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    let mut lines = vec![Line::from(Span::styled(header_text, style))];
    if let Some(body) = body {
        lines.extend(body);
    }

    lines
}

/// Replace newlines/tabs with spaces and truncate to `TRANSCRIPT_PREVIEW_MAX`
/// chars with a trailing `…` when longer. Char-based, not byte-based.
fn preview(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '\n' | '\t' => ' ',
            other => other,
        })
        .collect();
    let chars: Vec<char> = cleaned.chars().collect();
    if chars.len() <= TRANSCRIPT_PREVIEW_MAX {
        chars.into_iter().collect()
    } else {
        let mut result: String = chars.into_iter().take(TRANSCRIPT_PREVIEW_MAX).collect();
        result.push('…');
        result
    }
}

/// Clamp a scroll offset so it can't run past the last line.
fn clamp_scroll(scroll: u16, total_lines: usize) -> u16 {
    let max = total_lines.saturating_sub(1) as u16;
    scroll.min(max)
}

/// Resolve the scroll offset to display. `follow` pins to the bottom (newest):
/// the offset that shows the last `viewport` lines. Otherwise the manual `offset`
/// is clamped so it can't scroll past the bottom.
fn visible_offset(follow: bool, offset: u16, total_lines: usize, viewport: u16) -> u16 {
    let total = total_lines.min(u16::MAX as usize) as u16;
    let max = total.saturating_sub(viewport);
    if follow { max } else { offset.min(max) }
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
        let age_str = status::humanize_age(age_ms);
        let line = match (
            summary.update_interval_avg_ms,
            summary.update_interval_max_ms,
            summary.update_interval_min_ms,
        ) {
            (Some(avg), Some(max), Some(min)) => format!(
                "last update: {age_str} ago (AVG: {}, MAX: {}, MIN: {})",
                status::humanize_age(avg),
                status::humanize_age(max),
                status::humanize_age(min),
            ),
            _ => format!("last update: {age_str} ago"),
        };
        lines.push(Line::from(line));
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

/// Recent generation throughput: cumulative output tokens gained over the most
/// recent `Metrics` interval, divided by that interval's wall-clock seconds.
/// `None` until two `Metrics` records exist, or if the interval is zero-length.
fn tokens_per_sec(
    prev_ts: Option<u64>,
    prev_out: Option<u32>,
    last_ts: Option<u64>,
    last_out: Option<u32>,
) -> Option<f64> {
    let dt_ms = last_ts?.checked_sub(prev_ts?)?;
    if dt_ms == 0 {
        return None;
    }
    let d_out = last_out?.saturating_sub(prev_out?);
    Some(d_out as f64 / (dt_ms as f64 / 1000.0))
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

    match tokens_per_sec(
        summary.prev_metrics_ts,
        summary.prev_output_tokens,
        summary.last_metrics_ts,
        summary.last_output_tokens,
    ) {
        Some(rate) => {
            lines.push(Line::from(format!("tok/s: {rate:.1}")));
            if let (Some(avg), Some(max), Some(min)) = (
                summary.tok_per_sec_avg,
                summary.tok_per_sec_max,
                summary.tok_per_sec_min,
            ) {
                lines.push(Line::from(format!(
                    "  (AVG: {avg:.1}, MAX: {max:.1}, MIN: {min:.1})"
                )));
            }
        }
        None => lines.push(Line::from("tok/s: —")),
    }

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

/// Cloud-baseline $/Mtok rates for the Budget panel's "$ saved" line.
#[derive(Debug, Clone, Copy, Default)]
pub struct BudgetRates {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

/// Dollar cost the given cumulative token usage would incur at the cloud baseline.
fn dollars_saved(
    input_tokens: u32,
    output_tokens: u32,
    in_per_mtok: f64,
    out_per_mtok: f64,
) -> f64 {
    (input_tokens as f64 / 1_000_000.0) * in_per_mtok
        + (output_tokens as f64 / 1_000_000.0) * out_per_mtok
}

/// "$ saved" line for the Budget panel.
/// Returns `None` when there are no metrics yet, `"$ saved: —"` when rates are
/// unconfigured (both 0.0), and `"$ saved: $X.XX"` otherwise.
fn dollars_saved_line(summary: &StatusSummary, rates: BudgetRates) -> Option<Line<'static>> {
    let in_tok = summary.last_input_tokens?;
    let out_tok = summary.last_output_tokens.unwrap_or(0);
    if rates.input_per_mtok == 0.0 && rates.output_per_mtok == 0.0 {
        return Some(Line::from("$ saved: —"));
    }
    let saved = dollars_saved(in_tok, out_tok, rates.input_per_mtok, rates.output_per_mtok);
    Some(Line::from(format!("$ saved: ${saved:.2}")))
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
/// Transcript is newest-first when `follow` is true (tail-pinned).
fn render_dashboard(
    frame: &mut Frame,
    area: Rect,
    data: &DashboardData,
    now_ms: u64,
    offset: u16,
    follow: bool,
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

    let transcript = transcript_lines(&data.records);
    let viewport = activity_area.height.saturating_sub(2); // minus top+bottom border
    // Word-wrap is enabled for the Activity panel. Paragraph::scroll counts
    // *visual* rows (post-wrap), not logical lines, so scroll-to-bottom is
    // unsolvable without measuring actual rendered heights. When tail-following,
    // we instead truncate to the last (viewport * 2) logical lines and skip
    // scrolling — the newest content naturally lands at the bottom. When the user
    // scrolls manually we keep all lines and apply the offset (which is a logical-
    // line approximation, fine for navigation but not pixel-perfect with wrapping).
    let n = transcript.len();
    let (display_lines, scroll_rows) = if follow {
        let keep = (viewport as usize * 2).min(n);
        (transcript[n.saturating_sub(keep)..].to_vec(), 0u16)
    } else {
        let display = visible_offset(false, offset, n, viewport);
        (transcript, display)
    };
    let activity = Paragraph::new(display_lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll_rows, 0))
        .block(Block::default().borders(Borders::ALL).title(" Activity "));
    frame.render_widget(activity, activity_area);
    frame.render_widget(panel(" Files ", files_lines(&data.summary)), files_area);
}

// --- Entry points ---

/// Run the dashboard event loop. Called by `main.rs`.
pub fn run_dashboard(
    repo: &Path,
    session: Option<&str>,
    rates: BudgetRates,
) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, repo, session, rates);
    ratatui::restore();
    result
}

fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    repo: &Path,
    session: Option<&str>,
    rates: BudgetRates,
) -> std::io::Result<()> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind};
    use std::time::Duration;

    let mut offset: u16 = 0;
    let mut follow = true;

    loop {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let data = load_data(repo, session);
        offset = clamp_scroll(offset, transcript_lines(&data.records).len());
        terminal.draw(|frame| {
            render_dashboard(frame, frame.area(), &data, now_ms, offset, follow, rates)
        })?;

        if event::poll(Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Up => {
                    follow = false;
                    offset = offset.saturating_sub(1);
                }
                KeyCode::Down => {
                    follow = false;
                    offset = offset.saturating_add(1);
                }
                KeyCode::PageUp => {
                    follow = false;
                    offset = offset.saturating_sub(10);
                }
                KeyCode::PageDown => {
                    follow = false;
                    offset = offset.saturating_add(10);
                }
                KeyCode::Home => {
                    follow = false;
                    offset = 0;
                }
                KeyCode::End => {
                    follow = true;
                }
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

        let data = load_data(dir.path(), None);
        assert!(data.error.is_none());
        assert!(!data.records.is_empty());
        assert_eq!(data.records.len(), 2);
        assert_eq!(data.summary.phase.as_deref(), Some("phase-01"));
    }

    #[test]
    fn load_data_empty_records_on_error() {
        let dir = TempDir::new().unwrap();
        let data = load_data(dir.path(), None);
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

    #[test]
    fn tokens_per_sec_computes_recent_rate() {
        // 200 tokens over 2.0s = 100.0 tok/s
        assert_eq!(
            tokens_per_sec(Some(1000), Some(100), Some(3000), Some(300)),
            Some(100.0)
        );
    }

    #[test]
    fn tokens_per_sec_none_without_two_samples() {
        assert_eq!(tokens_per_sec(None, None, Some(3000), Some(300)), None);
    }

    #[test]
    fn tokens_per_sec_none_on_zero_interval() {
        // Zero-length interval → None, not NaN or panic
        assert_eq!(
            tokens_per_sec(Some(1000), Some(100), Some(1000), Some(300)),
            None
        );
    }

    #[test]
    fn tokens_per_sec_zero_when_no_new_output() {
        assert_eq!(
            tokens_per_sec(Some(1000), Some(300), Some(3000), Some(300)),
            Some(0.0)
        );
    }

    #[test]
    fn budget_lines_shows_tokens_per_sec() {
        let summary = StatusSummary {
            last_input_tokens: Some(1200),
            last_output_tokens: Some(300),
            last_metrics_ts: Some(3000),
            prev_metrics_ts: Some(1000),
            prev_output_tokens: Some(100),
            ..StatusSummary::default()
        };
        let lines = budget_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("tok/s: 100.0")));
    }

    #[test]
    fn budget_lines_tokens_per_sec_dash_with_one_sample() {
        let summary = StatusSummary {
            last_input_tokens: Some(500),
            last_output_tokens: Some(100),
            last_metrics_ts: Some(2000),
            prev_metrics_ts: None,
            prev_output_tokens: None,
            ..StatusSummary::default()
        };
        let lines = budget_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s == "tok/s: —"));
        // Must not show a numeric rate
        assert!(
            !text
                .iter()
                .any(|s| s.starts_with("tok/s:") && s.contains('.'))
        );
    }

    #[test]
    fn session_lines_shows_update_interval_stats() {
        let summary = StatusSummary {
            last_ts: Some(5000),
            update_interval_avg_ms: Some(2000),
            update_interval_max_ms: Some(3000),
            update_interval_min_ms: Some(1000),
            ..StatusSummary::default()
        };
        let lines = session_lines(&summary, 5000);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        let age_line = text.iter().find(|s| s.contains("last update")).unwrap();
        assert!(age_line.contains("AVG:"), "expected AVG in: {age_line}");
        assert!(age_line.contains("MAX:"), "expected MAX in: {age_line}");
        assert!(age_line.contains("MIN:"), "expected MIN in: {age_line}");
    }

    #[test]
    fn session_lines_omits_interval_stats_without_enough_data() {
        let summary = StatusSummary {
            last_ts: Some(5000),
            update_interval_avg_ms: None,
            ..StatusSummary::default()
        };
        let lines = session_lines(&summary, 5000);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        let age_line = text.iter().find(|s| s.contains("last update")).unwrap();
        assert!(!age_line.contains("AVG:"), "unexpected AVG in: {age_line}");
    }

    #[test]
    fn budget_lines_shows_tok_per_sec_stats() {
        let summary = StatusSummary {
            last_input_tokens: Some(1000),
            last_output_tokens: Some(300),
            last_metrics_ts: Some(3000),
            prev_metrics_ts: Some(1000),
            prev_output_tokens: Some(100),
            tok_per_sec_avg: Some(80.0),
            tok_per_sec_max: Some(120.0),
            tok_per_sec_min: Some(60.0),
            ..StatusSummary::default()
        };
        let lines = budget_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        let stats_line = text.iter().find(|s| s.contains("AVG:")).unwrap();
        assert!(stats_line.contains("AVG: 80.0"), "got: {stats_line}");
        assert!(stats_line.contains("MAX: 120.0"), "got: {stats_line}");
        assert!(stats_line.contains("MIN: 60.0"), "got: {stats_line}");
    }

    #[test]
    fn budget_lines_omits_tok_per_sec_stats_without_enough_data() {
        let summary = StatusSummary {
            last_input_tokens: Some(1000),
            last_output_tokens: Some(300),
            last_metrics_ts: Some(3000),
            prev_metrics_ts: Some(1000),
            prev_output_tokens: Some(100),
            tok_per_sec_avg: None,
            ..StatusSummary::default()
        };
        let lines = budget_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(!text.iter().any(|s| s.contains("AVG:")));
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

    // --- transcript_lines tests ---

    #[test]
    fn transcript_lines_empty_placeholder() {
        let lines = transcript_lines(&[]);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("no activity")));
    }

    /// Join a record's rendered lines into one string for content assertions.
    fn record_text(rec: &SessionRecord) -> String {
        record_lines(rec)
            .iter()
            .map(|l| format!("{l}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn record_lines_single_line_for_short_events() {
        // A Progress record renders as exactly one header line.
        let lines = record_lines(&rec(100, 4, progress_event(4, "verify")));
        assert_eq!(lines.len(), 1);
        assert!(format!("{}", lines[0]).contains("[t4] progress: verify"));
    }

    #[test]
    fn record_lines_renders_each_variant() {
        // SessionStart
        let t = record_text(&rec(100, 0, start_event()));
        assert!(t.contains("[t0]") && t.contains("session start") && t.contains("phase-01"));

        // Completion — header line + body line (multi-line)
        let comp = SessionEvent::Completion {
            raw: "hello world".into(),
        };
        let t = record_text(&rec(200, 1, comp));
        assert!(t.contains("[t1] completion:"));
        assert!(t.contains("hello world"));

        // ToolResult ok — header carries [ok]; body carries the output
        let tool_ok = SessionEvent::ToolResult {
            name: "read_file".into(),
            succeeded: true,
            output_preview: "file contents".into(),
        };
        let t = record_text(&rec(300, 2, tool_ok));
        assert!(t.contains("[t2] tool read_file [ok]"));
        assert!(t.contains("file contents"));

        // ToolResult FAIL
        let tool_fail = SessionEvent::ToolResult {
            name: "bash".into(),
            succeeded: false,
            output_preview: "error output".into(),
        };
        let t = record_text(&rec(400, 3, tool_fail));
        assert!(t.contains("[t3] tool bash [FAIL]"));

        // SessionEnd
        let end = SessionEvent::SessionEnd {
            status: "complete".into(),
            turns: 5,
        };
        let t = record_text(&rec(500, 5, end));
        assert!(t.contains("[t5]") && t.contains("session end — complete (5 turns)"));

        // Compaction
        let compact = SessionEvent::Compaction {
            tokens_before: 1000,
            tokens_after: 600,
            messages_signaturized: 3,
            messages_evicted: 1,
        };
        let t = record_text(&rec(600, 4, compact));
        assert!(t.contains("[t4]") && t.contains("compaction: 1000 → 600 tokens"));

        // HardFail
        let hf = SessionEvent::HardFail {
            reason: "out of memory".into(),
        };
        let t = record_text(&rec(700, 3, hf));
        assert!(t.contains("[t3]") && t.contains("HARD FAIL: out of memory"));

        // Verify
        let verify = SessionEvent::Verify {
            diagnostics: vec![],
        };
        let t = record_text(&rec(800, 2, verify));
        assert!(t.contains("[t2]") && t.contains("verify: 0 diagnostic(s)"));

        // Metrics
        let metrics = SessionEvent::Metrics {
            input_tokens: 500,
            output_tokens: 100,
            context_pct: 0.3,
        };
        let t = record_text(&rec(900, 1, metrics));
        assert!(t.contains("[t1]") && t.contains("metrics: 500 in / 100 out"));

        // Prompt
        let prompt = SessionEvent::Prompt {
            rendered: "short prompt".into(),
        };
        let t = record_text(&rec(1000, 0, prompt));
        assert!(t.contains("[t0]") && t.contains("prompt (12 chars)"));

        // Progress
        let prog = SessionEvent::Progress {
            turn: 1,
            stage: "verify".into(),
            files_changed: vec![],
            message: "done".into(),
        };
        let t = record_text(&rec(1100, 1, prog));
        assert!(t.contains("[t1]") && t.contains("progress: verify"));

        // Parsed
        let parsed = SessionEvent::Parsed {
            tool_call: rexymcp_executor::parser::ToolCall {
                name: "write_file".into(),
                arguments: serde_json::json!({}),
                origin: rexymcp_executor::parser::Origin::Native,
            },
        };
        let t = record_text(&rec(1200, 2, parsed));
        assert!(t.contains("[t2]") && t.contains("→ call write_file"));

        // ParseFailed
        let pf = SessionEvent::ParseFailed {
            failure: rexymcp_executor::parser::ParseFailure {
                raw: String::new(),
                detected_format: None,
                candidates: vec![],
                feedback: "expected a tool call".into(),
            },
        };
        let t = record_text(&rec(1300, 3, pf));
        assert!(t.contains("[t3]") && t.contains("parse failed: expected a tool call"));
    }

    #[test]
    fn record_lines_expands_completion_multiline() {
        let comp = SessionEvent::Completion {
            raw: "a\nb\nc".into(),
        };
        let lines = record_lines(&rec(100, 1, comp));
        // 1 header + 3 body lines.
        assert_eq!(lines.len(), 4);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text[0].contains("[t1] completion:"));
        assert!(text.iter().any(|s| s.contains('a')));
        assert!(text.iter().any(|s| s.contains('c')));
    }

    #[test]
    fn record_lines_expands_tool_output_multiline() {
        let tr = SessionEvent::ToolResult {
            name: "bash".into(),
            succeeded: false,
            output_preview: "line one\nline two".into(),
        };
        let lines = record_lines(&rec(100, 2, tr));
        // 1 header + 2 body lines.
        assert_eq!(lines.len(), 3);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text[0].contains("tool bash [FAIL]"));
        assert!(text[1].contains("line one"));
        assert!(text[2].contains("line two"));
    }

    #[test]
    fn record_lines_caps_long_content() {
        // More than TRANSCRIPT_CONTENT_MAX_LINES content lines → capped + marker.
        let body: String = (0..TRANSCRIPT_CONTENT_MAX_LINES + 5)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let comp = SessionEvent::Completion { raw: body };
        let lines = record_lines(&rec(100, 0, comp));
        // 1 header + TRANSCRIPT_CONTENT_MAX_LINES body + 1 overflow marker.
        assert_eq!(lines.len(), 1 + TRANSCRIPT_CONTENT_MAX_LINES + 1);
        let last = format!("{}", lines[lines.len() - 1]);
        assert!(
            last.contains("more lines"),
            "last line should be the overflow marker: {last}"
        );
    }

    #[test]
    fn transcript_lines_flatmaps_records() {
        // A single-line event + a 3-line completion → 1 + (1 header + 3 body) = 5.
        let records = vec![
            rec(100, 0, start_event()),
            rec(
                200,
                1,
                SessionEvent::Completion {
                    raw: "x\ny\nz".into(),
                },
            ),
        ];
        let lines = transcript_lines(&records);
        assert_eq!(lines.len(), 5);
    }

    // --- detect_syntax / highlighted_body_lines tests ---

    #[test]
    fn detect_syntax_identifies_json() {
        let ss = syntax_set();
        let json = r#"{"key": "value", "n": 42}"#;
        let syntax = detect_syntax(json, ss);
        assert!(syntax.is_some(), "should detect JSON");
        assert!(
            syntax.unwrap().name.to_lowercase().contains("json"),
            "detected: {}",
            syntax.unwrap().name
        );
    }

    #[test]
    fn detect_syntax_identifies_rust() {
        let ss = syntax_set();
        let rust = "pub fn main() {\n    let x = 1;\n    match x {\n        _ => {}\n    }\n}";
        let syntax = detect_syntax(rust, ss);
        assert!(syntax.is_some(), "should detect Rust");
        assert!(
            syntax.unwrap().name.to_lowercase().contains("rust"),
            "detected: {}",
            syntax.unwrap().name
        );
    }

    #[test]
    fn detect_syntax_returns_none_for_plain_text() {
        let ss = syntax_set();
        assert!(detect_syntax("just some plain text output", ss).is_none());
    }

    #[test]
    fn highlighted_body_lines_preserves_content() {
        // Content is preserved regardless of whether highlighting is applied.
        let json = "{\n  \"status\": \"ok\"\n}";
        let lines = highlighted_body_lines(json);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(
            text.iter().any(|s| s.contains("status")),
            "json key must appear in output"
        );
    }

    #[test]
    fn highlighted_body_lines_falls_back_for_plain_text() {
        let lines = highlighted_body_lines("boring plain output");
        assert_eq!(lines.len(), 1, "one line for plain text");
        assert!(format!("{}", lines[0]).contains("boring plain output"));
    }

    // --- visible_offset tests ---

    #[test]
    fn visible_offset_follows_tail() {
        // follow → bottom screenful: total - viewport.
        assert_eq!(visible_offset(true, 0, 100, 20), 80);
        // total < viewport → 0 (nothing to scroll).
        assert_eq!(visible_offset(true, 0, 10, 20), 0);
    }

    #[test]
    fn visible_offset_manual_clamped() {
        // Manual offset past the bottom is clamped to the last screenful.
        assert_eq!(visible_offset(false, 999, 100, 20), 80);
        // Manual offset within range is preserved.
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

    // --- dollars_saved tests ---

    #[test]
    fn dollars_saved_computes_cost() {
        // 1M input @ $3/Mtok + 500k output @ $15/Mtok = $3.00 + $7.50 = $10.50
        assert_eq!(dollars_saved(1_000_000, 500_000, 3.0, 15.0), 10.5);
    }

    #[test]
    fn dollars_saved_line_none_without_metrics() {
        let summary = StatusSummary::default();
        let rates = BudgetRates {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
        };
        assert!(dollars_saved_line(&summary, rates).is_none());
    }

    #[test]
    fn dollars_saved_line_dash_when_rates_unset() {
        let summary = StatusSummary {
            last_input_tokens: Some(500),
            last_output_tokens: Some(100),
            ..StatusSummary::default()
        };
        let rates = BudgetRates::default();
        let line = dollars_saved_line(&summary, rates);
        assert!(line.is_some());
        assert_eq!(format!("{}", line.unwrap()), "$ saved: —");
    }

    #[test]
    fn dollars_saved_line_shows_dollars() {
        let summary = StatusSummary {
            last_input_tokens: Some(1_000_000),
            last_output_tokens: Some(500_000),
            ..StatusSummary::default()
        };
        let rates = BudgetRates {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
        };
        let line = dollars_saved_line(&summary, rates);
        assert!(line.is_some());
        assert_eq!(format!("{}", line.unwrap()), "$ saved: $10.50");
    }
}
