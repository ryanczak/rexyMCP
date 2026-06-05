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
    widgets::{Block, Borders, Paragraph},
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

const SPINNER_FRAMES: &[&str] = &["🐾", "🐾🐾", "🐾🐾🐾", "🐾🐾🐾🐾", "🐾🐾🐾", "🐾🐾", "🐾"];

const FILTER_ITEM_COUNT: usize = 11;

/// Per-event-type visibility toggles for the Activity pane.
/// All enabled by default except `progress` (too noisy).
#[derive(Clone, Debug, PartialEq)]
struct ActivityFilter {
    session: bool,
    prompt: bool,
    completion: bool,
    tool_call: bool,
    parse_failed: bool,
    tool_result: bool,
    verify: bool,
    hard_fail: bool,
    progress: bool,
    metrics: bool,
    compaction: bool,
}

impl Default for ActivityFilter {
    fn default() -> Self {
        Self {
            session: true,
            prompt: true,
            completion: true,
            tool_call: true,
            parse_failed: true,
            tool_result: true,
            verify: true,
            hard_fail: true,
            progress: false,
            metrics: true,
            compaction: true,
        }
    }
}

impl ActivityFilter {
    fn allows(&self, event: &SessionEvent) -> bool {
        match event {
            SessionEvent::SessionStart { .. } | SessionEvent::SessionEnd { .. } => self.session,
            SessionEvent::Prompt { .. } => self.prompt,
            SessionEvent::Completion { .. } => self.completion,
            SessionEvent::Parsed { .. } => self.tool_call,
            SessionEvent::ParseFailed { .. } => self.parse_failed,
            SessionEvent::ToolResult { .. } => self.tool_result,
            SessionEvent::Verify { .. } => self.verify,
            SessionEvent::HardFail { .. } => self.hard_fail,
            SessionEvent::Progress { .. } => self.progress,
            SessionEvent::Metrics { .. } => self.metrics,
            SessionEvent::Compaction { .. } => self.compaction,
        }
    }

    fn toggle(&mut self, index: usize) {
        match index {
            0 => self.session = !self.session,
            1 => self.prompt = !self.prompt,
            2 => self.completion = !self.completion,
            3 => self.tool_call = !self.tool_call,
            4 => self.parse_failed = !self.parse_failed,
            5 => self.tool_result = !self.tool_result,
            6 => self.verify = !self.verify,
            7 => self.hard_fail = !self.hard_fail,
            8 => self.progress = !self.progress,
            9 => self.metrics = !self.metrics,
            10 => self.compaction = !self.compaction,
            _ => {}
        }
    }

    fn is_enabled(&self, index: usize) -> bool {
        match index {
            0 => self.session,
            1 => self.prompt,
            2 => self.completion,
            3 => self.tool_call,
            4 => self.parse_failed,
            5 => self.tool_result,
            6 => self.verify,
            7 => self.hard_fail,
            8 => self.progress,
            9 => self.metrics,
            10 => self.compaction,
            _ => false,
        }
    }

    fn item_label(index: usize) -> &'static str {
        match index {
            0 => "session start/end",
            1 => "prompt",
            2 => "completion",
            3 => "tool call",
            4 => "parse fail",
            5 => "tool result",
            6 => "verify",
            7 => "hard fail",
            8 => "progress",
            9 => "metrics",
            10 => "compaction",
            _ => "?",
        }
    }
}

/// Filter panel UI state — open/closed, cursor position, current settings.
#[derive(Clone, Debug, Default)]
struct FilterState {
    open: bool,
    cursor: usize,
    filter: ActivityFilter,
}

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

/// True when `content` looks like unified diff output (git, classic, or patch tool).
fn is_diff_content(content: &str) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    // Unified diff hunk marker is the most unambiguous signal.
    if lines.iter().any(|l| l.starts_with("@@")) {
        return true;
    }
    // Git diff header.
    if content.trim().starts_with("diff --git") {
        return true;
    }
    // Classic unified diff: --- header AND +++ header present.
    lines.iter().any(|l| l.starts_with("--- ")) && lines.iter().any(|l| l.starts_with("+++ "))
}

/// Render unified diff content with line-level background colors:
/// `+` lines → dark green bg, `-` lines → dark red bg, `@@` → cyan,
/// headers / context → dim gray. Matches the Claude Code diff aesthetic.
fn diff_body_lines(content: &str) -> Vec<Line<'static>> {
    let all: Vec<&str> = content.lines().collect();
    let capped = all.len().min(TRANSCRIPT_CONTENT_MAX_LINES);
    let overflow = all.len().saturating_sub(TRANSCRIPT_CONTENT_MAX_LINES);

    let mut result: Vec<Line<'static>> = Vec::new();
    for &line in &all[..capped] {
        let rendered = if line.starts_with('+') && !line.starts_with("+++") {
            Line::from(Span::styled(
                format!("    {line}"),
                Style::new()
                    .fg(Color::Rgb(180, 242, 180))
                    .bg(Color::Rgb(0, 48, 0)),
            ))
        } else if line.starts_with('-') && !line.starts_with("---") {
            Line::from(Span::styled(
                format!("    {line}"),
                Style::new()
                    .fg(Color::Rgb(242, 180, 180))
                    .bg(Color::Rgb(64, 0, 0)),
            ))
        } else if line.starts_with("@@") {
            Line::from(Span::styled(
                format!("    {line}"),
                Style::new().fg(Color::Cyan),
            ))
        } else {
            Line::from(Span::styled(
                format!("    {line}"),
                Style::new().fg(Color::DarkGray),
            ))
        };
        result.push(rendered);
    }
    if overflow > 0 {
        result.push(Line::from(Span::styled(
            format!("    … ({overflow} more lines)"),
            Style::new().fg(Color::DarkGray),
        )));
    }
    result
}

/// Render `content` as indented, syntax-highlighted lines.
/// Falls back to DarkGray when no language is detected.
fn highlighted_body_lines(content: &str) -> Vec<Line<'static>> {
    // Diff output is handled specially with background-color line highlighting.
    if is_diff_content(content) {
        return diff_body_lines(content);
    }

    let ss = syntax_set();

    let Some(syntax) = detect_syntax(content, ss) else {
        return body_lines(content)
            .into_iter()
            .map(|l| Line::from(Span::styled(l, Style::new().fg(Color::Rgb(200, 200, 200)))))
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
/// Filters records through `filter`, appends a spinner frame when `spinner` is
/// `Some`. Returns a placeholder when all records are filtered out.
fn transcript_lines(
    records: &[SessionRecord],
    filter: &ActivityFilter,
    spinner: Option<usize>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = {
        let visible: Vec<_> = records.iter().filter(|r| filter.allows(&r.event)).collect();
        if visible.is_empty() {
            vec![Line::from("(no activity yet)")]
        } else {
            visible.iter().flat_map(|r| record_lines(r)).collect()
        }
    };
    if let Some(frame) = spinner {
        let glyph = SPINNER_FRAMES[frame % SPINNER_FRAMES.len()];
        lines.push(Line::from(glyph.to_string()));
    }
    lines
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
                Some(plain_body_lines(raw, Color::Rgb(200, 200, 200))),
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
        let line = match summary.update_interval_avg_ms {
            Some(avg) => format!(
                "last update: {age_str} ago (avg: {})",
                status::humanize_age(avg),
            ),
            None => format!("last update: {age_str} ago"),
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
            let added_str = format!("+{}", f.added);
            let removed_str = format!("-{}", f.removed);
            let numstat_width = added_str.len() + 1 + removed_str.len();
            // Path budget = total line max − indent(2) − separator(1) − numstat.
            // This guarantees the numstat is always visible regardless of its width.
            let path_max = FILE_LINE_MAX.saturating_sub(1 + numstat_width);
            Line::from(vec![
                Span::raw(format!("  {} ", trim_path_left(&f.path, path_max))),
                Span::styled(added_str, Style::new().fg(Color::Green)),
                Span::raw(" "),
                Span::styled(removed_str, Style::new().fg(Color::Red)),
            ])
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
            let stats = match (
                summary.tok_per_sec_avg,
                summary.tok_per_sec_max,
                summary.tok_per_sec_min,
            ) {
                (Some(avg), Some(max), Some(min)) => {
                    format!("  (avg: {avg:.1}, max: {max:.1}, min: {min:.1})")
                }
                _ => String::new(),
            };
            lines.push(Line::from(format!("tok/s: {rate:.1}{stats}")));
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
            let label = match (summary.last_context_used, summary.last_context_window) {
                (Some(used), Some(window)) if window > 0 => {
                    format!("context: {pct_int}% ({used}/{window})")
                }
                _ => format!("context: {pct_int}%"),
            };
            lines.push(Line::from(Span::styled(label, Style::new().fg(color))));
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

/// View-state for the dashboard activity pane.
struct ViewState {
    offset: u16,
    follow: bool,
    spinner: Option<usize>,
    filter: FilterState,
}

/// Render the dashboard into a three-panel header band (Session · Budget ·
/// Compactions) above a body (Activity wide-left · Files right), or a
/// single error pane when `data.error` is set.
/// Transcript is newest-first when `follow` is true (tail-pinned).
fn render_dashboard(
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
            Paragraph::new(transcript)
                .scroll((scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Activity [f=filter] "),
                ),
            activity_area,
        );
    }
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
    let mut spinner_tick: usize = 0;
    let mut filter_state = FilterState::default();

    loop {
        spinner_tick = spinner_tick.wrapping_add(1);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let data = load_data(repo, session);
        let spinner_active = data.summary.ended.is_none() && data.error.is_none();
        let spinner = if spinner_active {
            Some(spinner_tick % SPINNER_FRAMES.len())
        } else {
            None
        };
        let state = ViewState {
            offset,
            follow,
            spinner,
            filter: filter_state.clone(),
        };
        terminal
            .draw(|frame| render_dashboard(frame, frame.area(), &data, now_ms, &state, rates))?;
        offset = clamp_scroll(
            offset,
            transcript_lines(&data.records, &filter_state.filter, spinner).len(),
        );

        if event::poll(Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            if filter_state.open {
                match key.code {
                    KeyCode::Char('f') | KeyCode::Esc => filter_state.open = false,
                    KeyCode::Char('j') | KeyCode::Down => {
                        filter_state.cursor = (filter_state.cursor + 1) % FILTER_ITEM_COUNT;
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        filter_state.cursor =
                            (filter_state.cursor + FILTER_ITEM_COUNT - 1) % FILTER_ITEM_COUNT;
                    }
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        filter_state.filter.toggle(filter_state.cursor);
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('f') => {
                        filter_state.open = true;
                        filter_state.cursor = 0;
                    }
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
    fn budget_lines_shows_context_used_and_window() {
        let summary = StatusSummary {
            last_input_tokens: Some(1200),
            last_output_tokens: Some(340),
            last_context_pct: Some(0.68),
            last_context_used: Some(31195),
            last_context_window: Some(45875),
            ..StatusSummary::default()
        };
        let lines = budget_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        let ctx_line = text.iter().find(|s| s.contains("context:")).unwrap();
        assert!(ctx_line.contains("68%"), "pct in: {ctx_line}");
        assert!(ctx_line.contains("31195"), "used in: {ctx_line}");
        assert!(ctx_line.contains("45875"), "window in: {ctx_line}");
    }

    #[test]
    fn budget_lines_context_omits_fraction_when_window_zero() {
        // window == 0 means unmeasured/sentinel — no (N/N) suffix.
        let summary = StatusSummary {
            last_input_tokens: Some(500),
            last_output_tokens: Some(100),
            last_context_pct: Some(0.50),
            last_context_used: Some(0),
            last_context_window: Some(0),
            ..StatusSummary::default()
        };
        let lines = budget_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        let ctx_line = text.iter().find(|s| s.contains("context:")).unwrap();
        assert!(ctx_line.contains("50%"), "pct in: {ctx_line}");
        assert!(
            !ctx_line.contains('/'),
            "no fraction when window=0: {ctx_line}"
        );
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
        assert!(age_line.contains("avg:"), "expected avg in: {age_line}");
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
        assert!(!age_line.contains("avg:"), "unexpected avg in: {age_line}");
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
        let stats_line = text.iter().find(|s| s.contains("tok/s:")).unwrap();
        assert!(stats_line.contains("avg: 80.0"), "got: {stats_line}");
        assert!(stats_line.contains("max: 120.0"), "got: {stats_line}");
        assert!(stats_line.contains("min: 60.0"), "got: {stats_line}");
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
        assert!(!text.iter().any(|s| s.contains("avg:")));
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
        let lines = transcript_lines(&[], &ActivityFilter::default(), None);
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
            context_used: 0,
            context_window: 0,
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
        let lines = transcript_lines(&records, &ActivityFilter::default(), None);
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn spinner_appended_when_active() {
        let records = vec![rec(100, 0, start_event())];
        let lines = transcript_lines(&records, &ActivityFilter::default(), Some(0));
        let last = format!("{}", lines.last().unwrap());
        assert_eq!(last, "🐾");
    }

    #[test]
    fn spinner_frame_cycles_through_all_frames() {
        let records = vec![rec(100, 0, start_event())];
        for (i, expected) in SPINNER_FRAMES.iter().enumerate() {
            let lines = transcript_lines(&records, &ActivityFilter::default(), Some(i));
            let last = format!("{}", lines.last().unwrap());
            assert_eq!(last, *expected, "frame {i} mismatch");
        }
        // Index 7 wraps to frame 0
        let lines = transcript_lines(&records, &ActivityFilter::default(), Some(7));
        let last = format!("{}", lines.last().unwrap());
        assert_eq!(last, SPINNER_FRAMES[0], "frame 7 should wrap to 0");
    }

    #[test]
    fn spinner_absent_when_none() {
        let records = vec![rec(100, 0, start_event())];
        let lines = transcript_lines(&records, &ActivityFilter::default(), None);
        let last = format!("{}", lines.last().unwrap());
        assert!(!last.contains("🐾"), "spinner should not appear: {last}");
    }

    #[test]
    fn spinner_appended_to_empty_records() {
        let lines = transcript_lines(&[], &ActivityFilter::default(), Some(3));
        assert_eq!(lines.len(), 2);
        assert_eq!(format!("{}", lines[0]), "(no activity yet)");
        assert_eq!(format!("{}", lines[1]), "🐾🐾🐾🐾");
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

    // --- diff highlighting tests ---

    #[test]
    fn is_diff_content_detects_hunk_marker() {
        assert!(is_diff_content("@@ -1,3 +1,4 @@\n fn foo() {}"));
    }

    #[test]
    fn is_diff_content_detects_classic_unified() {
        let diff = "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new";
        assert!(is_diff_content(diff));
    }

    #[test]
    fn is_diff_content_detects_git_diff_header() {
        assert!(is_diff_content(
            "diff --git a/foo.rs b/foo.rs\n--- a/foo.rs\n+++ b/foo.rs"
        ));
    }

    #[test]
    fn is_diff_content_rejects_plain_text() {
        assert!(!is_diff_content("just some output\nno diff markers here"));
    }

    #[test]
    fn diff_body_lines_renders_patch_tool_output() {
        // Matches the format produced by the patch tool:
        // "✓ patched file\n\n--- file\n+++ file\n@@ ... @@\n context\n+added\n-removed"
        let output = "✓ patched src/main.rs (1 hunk)\n\n--- src/main.rs\n+++ src/main.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    old();\n+    new();\n }";
        let lines = diff_body_lines(output);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();

        // Added line is present.
        assert!(
            text.iter().any(|s| s.contains("+    new()")),
            "missing added line"
        );
        // Removed line is present.
        assert!(
            text.iter().any(|s| s.contains("-    old()")),
            "missing removed line"
        );
        // Hunk header is present.
        assert!(
            text.iter().any(|s| s.contains("@@ -1,3 +1,3 @@")),
            "missing hunk header"
        );
    }

    #[test]
    fn diff_body_lines_does_not_highlight_triple_plus_minus_as_change() {
        // --- / +++ file headers must NOT get add/remove background.
        let diff = "--- a/foo.rs\n+++ b/foo.rs\n@@ -1 +1 @@\n-old\n+new";
        let lines = diff_body_lines(diff);
        // First line starts with "---" → header, must contain "---" text.
        assert!(
            format!("{}", lines[0]).contains("---"),
            "header line must be rendered"
        );
        // Second line "+++ b/foo.rs" must also be present as header, not green-bg.
        assert!(
            format!("{}", lines[1]).contains("+++"),
            "header line must be rendered"
        );
    }

    #[test]
    fn highlighted_body_lines_routes_diff_to_diff_renderer() {
        let patch_output =
            "✓ patched foo.rs (1 hunk)\n\n--- foo.rs\n+++ foo.rs\n@@ -1 +1 @@\n-old\n+new";
        let lines = highlighted_body_lines(patch_output);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("+new")));
        assert!(text.iter().any(|s| s.contains("-old")));
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

    // --- Activity filter tests ---

    #[test]
    fn filter_default_disables_progress() {
        let f = ActivityFilter::default();
        assert!(!f.progress, "progress should be disabled by default");
        assert!(f.session);
        assert!(f.prompt);
        assert!(f.completion);
        assert!(f.tool_call);
        assert!(f.parse_failed);
        assert!(f.tool_result);
        assert!(f.verify);
        assert!(f.hard_fail);
        assert!(f.metrics);
        assert!(f.compaction);
    }

    #[test]
    fn filter_allows_progress_when_enabled() {
        let f = ActivityFilter {
            progress: true,
            ..Default::default()
        };
        let progress_rec = rec(100, 4, progress_event(4, "verify"));
        assert!(f.allows(&progress_rec.event));
    }

    #[test]
    fn filter_blocks_progress_by_default() {
        let f = ActivityFilter::default();
        let progress_rec = rec(100, 4, progress_event(4, "verify"));
        assert!(!f.allows(&progress_rec.event));
    }

    #[test]
    fn filter_toggle_flips_field() {
        let mut f = ActivityFilter::default();
        assert!(!f.progress);
        f.toggle(8);
        assert!(f.progress);
        f.toggle(8);
        assert!(!f.progress);
    }

    #[test]
    fn transcript_lines_excludes_filtered_events() {
        let records = vec![
            rec(100, 0, start_event()),
            rec(200, 1, progress_event(1, "thinking")),
        ];
        let lines = transcript_lines(&records, &ActivityFilter::default(), None);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        let joined = text.join("\n");
        assert!(
            joined.contains("session start"),
            "should contain session start"
        );
        assert!(
            !joined.contains("progress:"),
            "should not contain progress event"
        );
    }

    #[test]
    fn transcript_lines_all_filtered_shows_placeholder() {
        let records = vec![rec(100, 1, progress_event(1, "thinking"))];
        let lines = transcript_lines(&records, &ActivityFilter::default(), None);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(
            text.iter().any(|s| s.contains("no activity yet")),
            "should show placeholder when all events filtered"
        );
    }

    #[test]
    fn filter_cursor_wraps_forward() {
        let mut fs = FilterState::default();
        fs.cursor = FILTER_ITEM_COUNT - 1;
        fs.cursor = (fs.cursor + 1) % FILTER_ITEM_COUNT;
        assert_eq!(fs.cursor, 0);
    }

    #[test]
    fn filter_cursor_wraps_backward() {
        let mut fs = FilterState::default();
        fs.cursor = 0;
        fs.cursor = (fs.cursor + FILTER_ITEM_COUNT - 1) % FILTER_ITEM_COUNT;
        assert_eq!(fs.cursor, FILTER_ITEM_COUNT - 1);
    }
}
