use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::status::{self, StatusSummary};
use rexymcp_executor::store::sessions::event::TaskState;

/// Total usable content width for a Files panel line (indent + path + space +
/// numstat). Conservative for the 28%-wide panel at typical terminal widths.
/// The path budget is computed per-entry as `FILE_LINE_MAX - 2 - 1 - numstat_width`,
/// so the total rendered line is always ≤ `FILE_LINE_MAX + 2` chars regardless of
/// how large the added/removed counts are.
const FILE_LINE_MAX: usize = 28;

/// Cells in the Tasks progress bar.
const GAUGE_CELLS: usize = 10;

/// Cloud-baseline $/Mtok rates for the Budget panel's "$ saved" line.
#[derive(Debug, Clone, Copy, Default)]
pub struct BudgetRates {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

/// Return hardcoded cloud baseline rates for a known Claude model name.
/// Returns `None` for unrecognised names (caller falls back to configured rates).
///
/// Pricing as of 2026-06-04 ($/MTok input / $/MTok output):
/// - Fable 5 / Mythos 5: $10.00 / $50.00
/// - Opus 4.8 / 4.7 / 4.6: $5.00 / $25.00
/// - Sonnet 4.6: $3.00 / $15.00
/// - Haiku 4.5: $1.00 / $5.00
pub fn model_rates(model: &str) -> Option<BudgetRates> {
    match model {
        "claude-fable-5" | "claude-mythos-5" => Some(BudgetRates {
            input_per_mtok: 10.0,
            output_per_mtok: 50.0,
        }),
        "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => Some(BudgetRates {
            input_per_mtok: 5.0,
            output_per_mtok: 25.0,
        }),
        "claude-sonnet-4-6" => Some(BudgetRates {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
        }),
        "claude-haiku-4-5" => Some(BudgetRates {
            input_per_mtok: 1.0,
            output_per_mtok: 5.0,
        }),
        _ => None,
    }
}

/// Wall-clock session duration in ms: **live** (`now_ms − started_at`) while the
/// session is running, **frozen** (`last_ts − started_at`) once it has ended.
/// `None` for an empty log (no `started_at`). `saturating_sub` guards a clock that
/// reads behind the first record.
pub(crate) fn session_duration_ms(summary: &StatusSummary, now_ms: u64) -> Option<u64> {
    let start = summary.started_at?;
    let end = if summary.ended.is_some() {
        summary.last_ts.unwrap_or(start)
    } else {
        now_ms
    };
    Some(end.saturating_sub(start))
}

/// Session panel: phase / session / model / state / turn / stage / freshness.
/// `now_ms` is injected (unix millis) so the age line is testable.
/// The spinner is composed externally in `render.rs` via `spinner_line`.
pub(crate) fn session_lines(summary: &StatusSummary, now_ms: u64) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let phase = summary.phase.as_deref().unwrap_or("<unknown>");
    let session = summary.session_id.as_deref().unwrap_or("<unknown>");
    lines.push(Line::from(format!("Phase: {phase}")));
    lines.push(Line::from(format!("Session: {session}")));

    if let Some(model) = &summary.model {
        lines.push(Line::from(format!("Model: {model}")));
    }

    let state = match &summary.ended {
        Some(s) => format!("ended ({s})"),
        None => "running".to_string(),
    };
    lines.push(Line::from(Span::styled(
        format!("State: {state}"),
        Style::new()
            .add_modifier(ratatui::style::Modifier::BOLD)
            .fg(if summary.ended.is_some() {
                Color::Yellow
            } else {
                Color::Green
            }),
    )));

    if let Some(dur) = session_duration_ms(summary, now_ms) {
        lines.push(Line::from(format!(
            "Duration: {}",
            status::humanize_age(dur)
        )));
    }

    if let Some(line) = last_update_line(summary, now_ms) {
        lines.push(line);
    }

    let stage = summary.latest_stage.as_deref().unwrap_or("<none>");
    lines.push(Line::from(format!(
        "Turn {}, stage {stage}",
        summary.latest_turn
    )));

    lines
}

/// Display cells the spinner sprite occupies (one wide dog glyph). The dog's
/// horizontal offset is bounded so `offset + SPRITE_CELLS <= width`, keeping the
/// rendered line inside the panel.
const SPRITE_CELLS: usize = 2;

/// Full-width liveness spinner: a dog that trots back and forth across the Session
/// panel. `spinner` is `Some(tick)` — a monotonic counter from the event loop —
/// while the session runs, and `None` once it ends (→ `None`, no spinner line, same
/// as today's ended behavior). `width` is the Session panel's inner width. The dog's
/// offset is a triangle wave over `[0, width − SPRITE_CELLS]`, so the dog walks the
/// full width and the line never exceeds it.
///
/// Char-count vs display-width caveat: the dog glyph is one code point but two
/// display cells; `SPRITE_CELLS` budgets for that. Wide-glyph rounding may leave the
/// line a cell short of the border — acceptable, matching `wrap_line`'s existing
/// char-count approach.
pub(crate) fn spinner_line(spinner: Option<usize>, width: usize) -> Option<Line<'static>> {
    let tick = spinner?;
    let span = width.saturating_sub(SPRITE_CELLS);
    if span == 0 {
        return Some(Line::from("🐕"));
    }
    let period = span * 2;
    let phase = tick % period;
    let offset = if phase <= span { phase } else { period - phase };
    Some(Line::from(format!("{}🐕", " ".repeat(offset))))
}

/// Reclaim panel: compaction plus the three M10 per-lever reclaim sources.
pub(crate) fn reclaim_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if summary.compaction_count > 0 {
        let before = summary.compaction_tokens_before;
        let after = summary.compaction_tokens_after;
        lines.push(Line::from(format!("Events: {}", summary.compaction_count)));
        lines.push(Line::from(format!(
            "Freed: {} tokens",
            before.saturating_sub(after)
        )));
        if after != 0 {
            let ratio = before as f64 / after as f64;
            lines.push(Line::from(format!("Ratio: {ratio:.1}x")));
        }
    }
    if summary.output_filtered_count > 0 {
        lines.push(Line::from(format!(
            "Filter: {} calls, {} freed",
            summary.output_filtered_count, summary.output_filtered_tokens
        )));
    }
    if summary.read_evicted_count > 0 {
        lines.push(Line::from(format!(
            "Evict: {} reads, {} freed",
            summary.read_evicted_count, summary.read_evicted_tokens
        )));
    }
    if summary.read_deduped_count > 0 {
        lines.push(Line::from(format!(
            "Dedupe: {} reads, {} saved",
            summary.read_deduped_count, summary.read_deduped_tokens
        )));
    }

    if lines.is_empty() {
        return vec![Line::from("(No reclaim yet)")];
    }
    lines
}

/// Truncate a task title to at most `max` chars, appending `…` when shortened.
fn truncate_title(title: &str, max: usize) -> String {
    if title.chars().count() <= max {
        return title.to_string();
    }
    let keep = max.saturating_sub(1);
    let head: String = title.chars().take(keep).collect();
    format!("{head}…")
}

/// Done/total progress gauge for the Tasks panel — a filled bar plus
/// `done/total (pct%)`, colored by completion (progress-oriented: green = near/at
/// done, neutral grey = no progress). Matches the Budget context-gauge *style*
/// (a single colored text `Line`), not a ratatui `Gauge` widget.
pub(crate) fn tasks_gauge_line(done: usize, total: usize) -> Line<'static> {
    let pct = if total == 0 {
        0
    } else {
        ((done as f64 / total as f64) * 100.0).round() as u32
    };
    let filled = if total == 0 {
        0
    } else {
        (((done as f64 / total as f64) * GAUGE_CELLS as f64).round() as usize).min(GAUGE_CELLS)
    };
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(GAUGE_CELLS - filled));
    let color = if pct >= 80 {
        Color::Green
    } else if pct >= 40 {
        Color::Yellow
    } else {
        Color::Rgb(200, 200, 200)
    };
    Line::from(Span::styled(
        format!("{bar} {done}/{total} ({pct}%)"),
        Style::new().fg(color),
    ))
}

/// Tasks panel: a done/total progress gauge over a list of named tasks, or a
/// placeholder when none are tracked.
pub(crate) fn tasks_lines(summary: &StatusSummary, width: usize) -> Vec<Line<'static>> {
    if summary.tasks_total == 0 {
        return vec![Line::from("(no tasks tracked yet)")];
    }
    let title_max = width.saturating_sub(2); // 1 glyph cell + 1 space
    let mut lines = vec![tasks_gauge_line(summary.tasks_done, summary.tasks_total)];
    for task in &summary.tasks {
        let (glyph, color) = match task.state {
            TaskState::Done => ("☑", Color::Green),
            TaskState::Active => ("▶", Color::Yellow),
            TaskState::Pending => ("☐", Color::Rgb(200, 200, 200)),
        };
        lines.push(Line::from(vec![
            Span::styled(glyph, Style::new().fg(color)),
            Span::raw(format!(" {}", truncate_title(&task.title, title_max))),
        ]));
    }
    lines
}

/// Files panel: one line per changed file, or a placeholder when none.
pub(crate) fn files_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
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
pub(crate) fn tokens_per_sec(
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
pub(crate) fn budget_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    if summary.last_input_tokens.is_none() {
        return vec![Line::from("(No metrics yet)")];
    }

    let in_toks = summary.last_input_tokens.unwrap_or(0);
    let out_toks = summary.last_output_tokens.unwrap_or(0);
    let mut lines = vec![
        Line::from(format!("Tokens in:  {in_toks}")),
        Line::from(format!("Tokens out: {out_toks}")),
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
            lines.push(Line::from(format!("Tok/s: {rate:.1}{stats}")));
        }
        None => lines.push(Line::from("Tok/s: —")),
    }

    if let Some(pct) = summary.last_context_pct {
        if pct == 0.0 {
            lines.push(Line::from("Context: — (unmeasured)"));
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
                    format!("Context: {pct_int}% ({used}/{window})")
                }
                _ => format!("Context: {pct_int}%"),
            };
            lines.push(Line::from(Span::styled(label, Style::new().fg(color))));
        }
    }

    lines
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
pub(crate) fn dollars_saved_line(
    summary: &StatusSummary,
    rates: BudgetRates,
) -> Option<Line<'static>> {
    let in_tok = summary.last_input_tokens?;
    let out_tok = summary.last_output_tokens.unwrap_or(0);
    if rates.input_per_mtok == 0.0 && rates.output_per_mtok == 0.0 {
        return Some(Line::from("$ saved: —"));
    }
    let saved = dollars_saved(in_tok, out_tok, rates.input_per_mtok, rates.output_per_mtok);
    Some(Line::from(format!("$ saved: ${saved:.2}")))
}

/// "last update: …" freshness line for the Budget panel — the age of the most
/// recent record, with the average update interval when enough records exist.
/// `Some` whenever the session has at least one record (`last_ts`); `None` for an
/// empty log. Mirrors the optional-line shape of `dollars_saved_line`.
pub(crate) fn last_update_line(summary: &StatusSummary, now_ms: u64) -> Option<Line<'static>> {
    let ts = summary.last_ts?;
    let age_str = status::humanize_age(now_ms.saturating_sub(ts));
    let line = match summary.update_interval_avg_ms {
        Some(avg) => format!(
            "Last update: {age_str} ago (avg: {})",
            status::humanize_age(avg),
        ),
        None => format!("Last update: {age_str} ago"),
    };
    Some(Line::from(line))
}

/// Wrap lines in a bordered `Block` with the given title.
pub(crate) fn panel(title: &'static str, lines: Vec<Line<'static>>) -> Paragraph<'static> {
    Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::store::sessions::event::FileNumstat;

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
        assert!(text.iter().any(|s| s == "Phase: phase-02"));
        assert!(text.iter().any(|s| s == "Session: abc"));
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
    fn session_lines_shows_turn_stage() {
        let summary = StatusSummary {
            latest_turn: 5,
            latest_stage: Some("verify".into()),
            ..StatusSummary::default()
        };
        let lines = session_lines(&summary, 4000);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("Turn 5")));
        assert!(text.iter().any(|s| s.contains("verify")));
    }

    #[test]
    fn session_lines_shows_duration_while_running() {
        let summary = StatusSummary {
            started_at: Some(1000),
            ended: None,
            ..StatusSummary::default()
        };
        let lines = session_lines(&summary, 4000);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s == "Duration: 3s"));
    }

    #[test]
    fn session_lines_omits_duration_when_no_started_at() {
        let summary = StatusSummary::default();
        let lines = session_lines(&summary, 9999);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(!text.iter().any(|s| s.contains("duration:")));
    }

    #[test]
    fn session_lines_includes_last_update_when_ts_present() {
        let summary = StatusSummary {
            last_ts: Some(1000),
            update_interval_avg_ms: Some(500),
            ..StatusSummary::default()
        };
        let lines = session_lines(&summary, 4000);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(
            text.iter().any(|s| s.contains("Last update:")),
            "session_lines must contain 'Last update:' when last_ts is set, got: {text:?}",
        );
    }

    #[test]
    fn session_lines_places_last_update_under_duration() {
        let summary = StatusSummary {
            session_id: Some("abc".into()),
            started_at: Some(0),
            last_ts: Some(1000),
            latest_turn: 2,
            latest_stage: Some("plan".into()),
            ..StatusSummary::default()
        };
        let lines = session_lines(&summary, 4000);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        let dur_idx = text
            .iter()
            .position(|s| s.contains("Duration:"))
            .expect("Duration: line missing");
        let lu_idx = text
            .iter()
            .position(|s| s.contains("Last update:"))
            .expect("Last update: line missing");
        let turn_idx = text
            .iter()
            .position(|s| s.contains("Turn"))
            .expect("Turn line missing");
        assert!(
            dur_idx < lu_idx && lu_idx < turn_idx,
            "Last update: (idx {lu_idx}) must be after Duration: (idx {dur_idx}) and before Turn (idx {turn_idx}), got: {text:?}",
        );
    }

    // --- spinner_line tests ---

    #[test]
    fn spinner_line_none_when_ended() {
        assert_eq!(spinner_line(None, 40), None);
    }

    #[test]
    fn spinner_line_starts_flush_left() {
        let line = spinner_line(Some(0), 40).unwrap();
        let text = format!("{line}");
        assert!(text.contains('🐕'), "must contain dog: {text}");
        assert_eq!(
            text.chars().take_while(|c| *c == ' ').count(),
            0,
            "no leading spaces at tick 0: {text}"
        );
    }

    #[test]
    fn spinner_line_never_exceeds_width() {
        let width = 20;
        for tick in 0..200 {
            let line = spinner_line(Some(tick), width).unwrap();
            let text = format!("{line}");
            let leading = text.chars().take_while(|c| *c == ' ').count();
            assert!(
                leading <= width.saturating_sub(SPRITE_CELLS),
                "tick {tick}: leading spaces {leading} exceeds bound {} (width {width})",
                width.saturating_sub(SPRITE_CELLS)
            );
        }
        // Huge tick must also stay bounded.
        let line = spinner_line(Some(999_999), width).unwrap();
        let text = format!("{line}");
        let leading = text.chars().take_while(|c| *c == ' ').count();
        assert!(
            leading <= width.saturating_sub(SPRITE_CELLS),
            "huge tick: leading spaces {leading} exceeds bound"
        );
    }

    #[test]
    fn spinner_line_bounces_at_right_edge() {
        let width = 5; // span = 3, period = 6
        let expected_offsets = [0, 1, 2, 3, 2, 1, 0, 1];
        for (i, &expected) in expected_offsets.iter().enumerate() {
            let line = spinner_line(Some(i), width).unwrap();
            let text = format!("{line}");
            let leading = text.chars().take_while(|c| *c == ' ').count();
            assert_eq!(
                leading, expected,
                "tick {i}: expected offset {expected}, got {leading}: {text}"
            );
        }
    }

    #[test]
    fn spinner_line_tiny_width_does_not_panic() {
        let line = spinner_line(Some(7), 1);
        assert!(line.is_some(), "must return Some even for tiny width");
        let text = format!("{}", line.unwrap());
        assert!(text.contains('🐕'), "must contain dog: {text}");
    }

    // --- session_duration_ms tests ---

    #[test]
    fn session_duration_ms_running_uses_now() {
        let summary = StatusSummary {
            started_at: Some(1000),
            ended: None,
            ..StatusSummary::default()
        };
        assert_eq!(session_duration_ms(&summary, 4000), Some(3000));
    }

    #[test]
    fn session_duration_ms_ended_uses_last_ts() {
        let summary = StatusSummary {
            started_at: Some(1000),
            last_ts: Some(5000),
            ended: Some("complete".into()),
            ..StatusSummary::default()
        };
        // ended: uses last_ts - started_at, NOT now_ms - started_at
        assert_eq!(session_duration_ms(&summary, 9000), Some(4000));
    }

    #[test]
    fn session_duration_ms_none_for_empty_log() {
        assert_eq!(session_duration_ms(&StatusSummary::default(), 5000), None);
    }

    // --- last_update_line tests ---

    #[test]
    fn last_update_line_shows_age() {
        let summary = StatusSummary {
            last_ts: Some(1000),
            ..StatusSummary::default()
        };
        let line = last_update_line(&summary, 4000);
        assert!(line.is_some());
        let text = format!("{}", line.unwrap());
        assert!(text.contains("Last update: 3s ago"));
    }

    #[test]
    fn last_update_line_none_for_empty_log() {
        assert_eq!(last_update_line(&StatusSummary::default(), 4000), None);
    }

    #[test]
    fn last_update_line_shows_interval_stats() {
        let summary = StatusSummary {
            last_ts: Some(5000),
            update_interval_avg_ms: Some(2000),
            update_interval_max_ms: Some(3000),
            update_interval_min_ms: Some(1000),
            ..StatusSummary::default()
        };
        let line = last_update_line(&summary, 5000).unwrap();
        let text = format!("{line}");
        assert!(text.contains("avg:"), "expected avg in: {text}");
    }

    #[test]
    fn last_update_line_omits_interval_stats_without_enough_data() {
        let summary = StatusSummary {
            last_ts: Some(5000),
            update_interval_avg_ms: None,
            ..StatusSummary::default()
        };
        let line = last_update_line(&summary, 5000).unwrap();
        let text = format!("{line}");
        assert!(!text.contains("avg:"), "unexpected avg in: {text}");
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

    // --- tasks_lines tests ---

    #[test]
    fn tasks_lines_empty_placeholder() {
        let summary = StatusSummary::default();
        let lines = tasks_lines(&summary, 40);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("no tasks tracked")));
        // No gauge line when empty.
        assert!(
            text.iter().all(|s| !s.contains('/') && !s.contains('█')),
            "empty placeholder must not contain gauge artifacts"
        );
    }

    #[test]
    fn tasks_lines_lists_named_tasks_with_glyphs() {
        use crate::status::TaskRow;
        let summary = StatusSummary {
            tasks_total: 3,
            tasks_done: 1,
            tasks_active: 1,
            tasks: vec![
                TaskRow {
                    id: "1".into(),
                    title: "Read config".into(),
                    state: TaskState::Done,
                },
                TaskRow {
                    id: "2".into(),
                    title: "Write tests".into(),
                    state: TaskState::Active,
                },
                TaskRow {
                    id: "3".into(),
                    title: "Refactor".into(),
                    state: TaskState::Pending,
                },
            ],
            ..StatusSummary::default()
        };
        let lines = tasks_lines(&summary, 40);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        // First line is the gauge.
        assert!(
            text[0].contains('/'),
            "first line should be gauge with fraction: {}",
            text[0]
        );
        // Task lines follow with glyphs.
        assert!(
            text.iter()
                .any(|s| s.contains('☑') && s.contains("Read config")),
            "done task should have check glyph: {text:?}"
        );
        assert!(
            text.iter()
                .any(|s| s.contains('▶') && s.contains("Write tests")),
            "active task should have play glyph: {text:?}"
        );
        assert!(
            text.iter()
                .any(|s| s.contains('☐') && s.contains("Refactor")),
            "pending task should have empty box glyph: {text:?}"
        );
    }

    #[test]
    fn tasks_lines_truncates_long_title() {
        use crate::status::TaskRow;
        let long_title = "This is a very long task title that should be truncated";
        let short_title = "Short";
        let summary = StatusSummary {
            tasks_total: 2,
            tasks_done: 0,
            tasks_active: 0,
            tasks: vec![
                TaskRow {
                    id: "1".into(),
                    title: long_title.into(),
                    state: TaskState::Pending,
                },
                TaskRow {
                    id: "2".into(),
                    title: short_title.into(),
                    state: TaskState::Pending,
                },
            ],
            ..StatusSummary::default()
        };
        let lines = tasks_lines(&summary, 26);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        // Long title is truncated with ellipsis.
        assert!(
            text.iter().any(|s| s.contains('…')),
            "long title line should contain ellipsis: {text:?}"
        );
        // Short title is not truncated (no ellipsis on that line).
        assert!(
            text.iter()
                .any(|s| s.contains(short_title) && !s.contains('…')),
            "short title line should not contain ellipsis: {text:?}"
        );
    }

    #[test]
    fn tasks_lines_uses_full_panel_width() {
        use crate::status::TaskRow;
        let title_50 = "A".repeat(50);
        let summary = StatusSummary {
            tasks_total: 1,
            tasks: vec![TaskRow {
                id: "1".into(),
                title: title_50.clone(),
                state: TaskState::Pending,
            }],
            ..StatusSummary::default()
        };
        // width=60: title_max=58, 50-char title fits without truncation.
        let lines = tasks_lines(&summary, 60);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(
            text.iter()
                .any(|s| s.contains(&title_50) && !s.contains('…')),
            "50-char title should not be truncated at width=60: {text:?}"
        );
        // width=28: title_max=26, 50-char title is truncated.
        let lines_narrow = tasks_lines(&summary, 28);
        let text_narrow: Vec<String> = lines_narrow.iter().map(|l| format!("{l}")).collect();
        assert!(
            text_narrow.iter().any(|s| s.contains('…')),
            "50-char title should be truncated at width=28: {text_narrow:?}"
        );
    }

    #[test]
    fn tasks_gauge_line_full_is_green_and_complete() {
        let line = tasks_gauge_line(4, 4);
        let text = format!("{line}");
        assert!(text.contains("4/4"), "should contain fraction: {text}");
        assert!(text.contains("100%"), "should contain 100%%: {text}");
        assert_eq!(
            text.matches('█').count(),
            10,
            "should have 10 filled cells: {text}"
        );
        assert_eq!(
            text.matches('░').count(),
            0,
            "should have 0 empty cells: {text}"
        );
        assert_eq!(
            line.spans[0].style.fg,
            Some(Color::Green),
            "should be green"
        );
    }

    #[test]
    fn tasks_gauge_line_half() {
        let line = tasks_gauge_line(1, 2);
        let text = format!("{line}");
        assert!(text.contains("1/2"), "should contain fraction: {text}");
        assert!(text.contains("50%"), "should contain 50%%: {text}");
        assert_eq!(
            text.matches('█').count(),
            5,
            "should have 5 filled cells: {text}"
        );
        assert_eq!(
            text.matches('░').count(),
            5,
            "should have 5 empty cells: {text}"
        );
        assert_eq!(
            line.spans[0].style.fg,
            Some(Color::Yellow),
            "should be yellow"
        );
    }

    #[test]
    fn tasks_gauge_line_zero_progress() {
        let line = tasks_gauge_line(0, 5);
        let text = format!("{line}");
        assert!(text.contains("0/5"), "should contain fraction: {text}");
        assert!(text.contains("0%"), "should contain 0%%: {text}");
        assert_eq!(
            text.matches('█').count(),
            0,
            "should have 0 filled cells: {text}"
        );
        assert_eq!(
            text.matches('░').count(),
            10,
            "should have 10 empty cells: {text}"
        );
        assert_eq!(
            line.spans[0].style.fg,
            Some(Color::Rgb(200, 200, 200)),
            "should be grey"
        );
    }

    #[test]
    fn tasks_gauge_line_fraction_and_fill() {
        let line = tasks_gauge_line(3, 8);
        let text = format!("{line}");
        assert!(text.contains("3/8"), "should contain fraction: {text}");
        assert!(
            text.contains("38%"),
            "should contain 38%% (round(37.5)): {text}"
        );
        assert_eq!(
            text.matches('█').count(),
            4,
            "should have 4 filled cells (round(3.75)): {text}"
        );
    }

    #[test]
    fn tasks_gauge_line_zero_total_does_not_panic() {
        let line = tasks_gauge_line(0, 0);
        let text = format!("{line}");
        assert!(text.contains("0/0"), "should contain 0/0: {text}");
        assert!(text.contains("0%"), "should contain 0%%: {text}");
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
        let ctx_line = text.iter().find(|s| s.contains("Context:")).unwrap();
        assert!(ctx_line.contains("68%"), "pct in: {ctx_line}");
        assert!(ctx_line.contains("31195"), "used in: {ctx_line}");
        assert!(ctx_line.contains("45875"), "window in: {ctx_line}");
    }

    #[test]
    fn budget_lines_context_omits_fraction_when_window_zero() {
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
        let ctx_line = text.iter().find(|s| s.contains("Context:")).unwrap();
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
        assert!(text.iter().any(|s| s.contains("No metrics")));
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
        assert!(text.iter().any(|s| s.contains("Tok/s: 100.0")));
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
        assert!(text.iter().any(|s| s == "Tok/s: —"));
        assert!(
            !text
                .iter()
                .any(|s| s.starts_with("Tok/s:") && s.contains('.'))
        );
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
        let stats_line = text.iter().find(|s| s.contains("Tok/s:")).unwrap();
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

    // --- tokens_per_sec tests ---

    #[test]
    fn tokens_per_sec_computes_recent_rate() {
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

    // --- reclaim_lines tests ---

    #[test]
    fn reclaim_lines_empty_placeholder() {
        let summary = StatusSummary::default();
        let lines = reclaim_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("No reclaim yet")));
    }

    #[test]
    fn reclaim_lines_shows_compaction_events_and_ratio() {
        let summary = StatusSummary {
            compaction_count: 2,
            compaction_tokens_before: 1000,
            compaction_tokens_after: 600,
            ..StatusSummary::default()
        };
        let lines = reclaim_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("Events: 2")));
        assert!(text.iter().any(|s| s.contains("Freed: 400")));
        assert!(text.iter().any(|s| s.contains("1.7x")));
    }

    #[test]
    fn reclaim_lines_omits_ratio_when_after_zero() {
        let summary = StatusSummary {
            compaction_count: 1,
            compaction_tokens_before: 500,
            compaction_tokens_after: 0,
            ..StatusSummary::default()
        };
        let lines = reclaim_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("Events: 1")));
        assert!(text.iter().any(|s| s.contains("Freed: 500")));
        assert!(!text.iter().any(|s| s.contains("x")));
    }

    #[test]
    fn reclaim_lines_shows_filter_lever() {
        let summary = StatusSummary {
            output_filtered_count: 3,
            output_filtered_tokens: 2048,
            ..StatusSummary::default()
        };
        let lines = reclaim_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("Filter: 3 calls")));
        assert!(text.iter().any(|s| s.contains("2048")));
    }

    #[test]
    fn reclaim_lines_shows_evict_and_dedupe_levers() {
        let summary = StatusSummary {
            read_evicted_count: 2,
            read_evicted_tokens: 900,
            read_deduped_count: 1,
            read_deduped_tokens: 120,
            ..StatusSummary::default()
        };
        let lines = reclaim_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("Evict: 2 reads")));
        assert!(text.iter().any(|s| s.contains("Dedupe: 1 reads")));
    }

    #[test]
    fn reclaim_lines_lever_absent_renders_no_lever_line() {
        let summary = StatusSummary {
            compaction_count: 1,
            compaction_tokens_before: 500,
            compaction_tokens_after: 200,
            ..StatusSummary::default()
        };
        let lines = reclaim_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(!text.iter().any(|s| s.contains("Filter:")));
        assert!(!text.iter().any(|s| s.contains("Evict:")));
        assert!(!text.iter().any(|s| s.contains("Dedupe:")));
    }

    // --- dollars_saved tests ---

    #[test]
    fn dollars_saved_computes_cost() {
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

    // --- model_rates tests ---

    #[test]
    fn model_rates_opus_48_returns_correct_pricing() {
        let rates = model_rates("claude-opus-4-8").expect("opus-4-8 should have rates");
        assert_eq!(rates.input_per_mtok, 5.0);
        assert_eq!(rates.output_per_mtok, 25.0);
    }

    #[test]
    fn model_rates_fable_5_returns_correct_pricing() {
        let rates = model_rates("claude-fable-5").expect("fable-5 should have rates");
        assert_eq!(rates.input_per_mtok, 10.0);
        assert_eq!(rates.output_per_mtok, 50.0);
    }

    #[test]
    fn model_rates_unknown_model_is_none() {
        assert!(
            model_rates("gpt-4").is_none(),
            "unknown model should return None"
        );
        assert!(model_rates("").is_none(), "empty string should return None");
    }
}
