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

/// Cloud-baseline $/Mtok rates for the Budget panel's "Savings:" line.
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

const DOG: char = '🐕';
const BRAIN: char = '🧠';
const DASH: char = '💨';

/// Display cells each emoji sprite occupies (one code point, two terminal cells).
const SPRITE_CELLS: usize = 2;

/// Liveness spinner: a dog chasing its own brain across the Session panel. While
/// the session runs, `spinner` is `Some(tick)` (a monotonic per-loop counter);
/// once it ends, `None` (→ no spinner line). `width` is the panel inner width.
///
/// One cycle has two phases:
/// 1. **Chase** — the brain starts at the panel middle and runs right; the dog
///    starts at the far left and closes at double the brain's pace, catching it
///    just as the brain's right edge meets the border.
/// 2. **Return** — the dog grabs the brain (`💨`) and the `🧠🐕💨` cluster (brain
///    now directly left of the dog) retreats right→left in double-time. When the
///    brain reaches the left edge the cycle resets.
///
/// The chase/return distances scale with `width`.
///
/// Char-count vs display-width caveat (unchanged from the prior impl): each emoji
/// is one `char` but two display cells; positions are computed in display cells so
/// the rendered line is bounded by `width` cells, while its `chars().count()` is
/// smaller. A wide-glyph terminal rounding may leave the line a cell short of the
/// border — acceptable.
pub(crate) fn spinner_line(spinner: Option<usize>, width: usize) -> Option<Line<'static>> {
    let tick = spinner?;
    // The return cluster (brain + dog + dash) is three sprites wide; below a
    // panel that can also fit the chase gap there is no room to animate, so show
    // a static pair.
    if width < SPRITE_CELLS * 4 {
        return Some(Line::from(format!("{DOG}{BRAIN}")));
    }
    // `span`: rightmost left-edge a single sprite can take while touching the
    // right border. `mid`: the brain's starting left-edge (panel middle).
    let span = width - SPRITE_CELLS;
    let mid = width / 2;
    let steps_a = span - mid; // chase: brain advances one cell per frame
    let p0 = width - SPRITE_CELLS * 3; // return cluster's left-edge at the border
    let steps_b = p0 / 2; // return: cluster retreats two cells per frame
    let period = steps_a + steps_b + 2;
    let phase = tick % period;

    if phase <= steps_a {
        // Chase: brain mid→right (1 cell/frame), dog left→behind-brain (2/frame),
        // clamped so the dog never overruns the brain on odd widths.
        let i = phase;
        let brain = mid + i;
        let dog = (2 * i).min(brain - SPRITE_CELLS);
        let gap = brain - dog - SPRITE_CELLS;
        Some(Line::from(format!(
            "{}{DOG}{}{BRAIN}",
            " ".repeat(dog),
            " ".repeat(gap),
        )))
    } else {
        // Return: the brain+dog+dash cluster retreats right→left at double-time.
        let j = phase - (steps_a + 1);
        let p = p0.saturating_sub(2 * j);
        Some(Line::from(format!("{}{BRAIN}{DOG}{DASH}", " ".repeat(p))))
    }
}

/// Reclaim panel: compaction plus the three M10 per-lever reclaim sources.
pub(crate) fn reclaim_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if let Some(pct) = summary.last_context_pct {
        if pct == 0.0 {
            lines.push(Line::from("Usage: — (unmeasured)"));
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
                    format!("Usage: {pct_int}% ({used}/{window})")
                }
                _ => format!("Usage: {pct_int}%"),
            };
            lines.push(Line::from(Span::styled(label, Style::new().fg(color))));
        }
    }

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

/// "Milestone: <name>" line for the top of the Session panel. The name is
/// `…`-truncated so the whole line (label + name) fits within `width` cells.
pub(crate) fn milestone_line(name: &str, width: usize) -> Line<'static> {
    const LABEL: &str = "Milestone: ";
    let budget = width.saturating_sub(LABEL.chars().count());
    Line::from(format!("{LABEL}{}", truncate_title(name, budget)))
}

/// Done/total progress gauge for the Tasks panel — a filled bar plus
/// `done/total (pct%)`, colored by completion (progress-oriented: green = near/at
/// done, neutral grey = no progress). Matches the Budget context-gauge *style*
/// (a single colored text `Line`), not a ratatui `Gauge` widget.
pub(crate) fn tasks_gauge_line(done: usize, total: usize, width: usize) -> Line<'static> {
    let pct = if total == 0 {
        0
    } else {
        ((done as f64 / total as f64) * 100.0).round() as u32
    };
    let suffix = format!(" {done}/{total} ({pct}%)");
    let gauge_cells = width.saturating_sub(suffix.len()).max(1);
    let filled = if total == 0 {
        0
    } else {
        (((done as f64 / total as f64) * gauge_cells as f64).round() as usize).min(gauge_cells)
    };
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(gauge_cells - filled));
    let color = if pct >= 80 {
        Color::Green
    } else if pct >= 40 {
        Color::Yellow
    } else {
        Color::Rgb(200, 200, 200)
    };
    Line::from(Span::styled(
        format!("{bar}{suffix}"),
        Style::new().fg(color),
    ))
}

/// Tasks panel: a done/total progress gauge over a list of named tasks, or a
/// placeholder when none are tracked.
pub(crate) fn tasks_lines(
    summary: &StatusSummary,
    width: usize,
    tick: Option<usize>,
) -> Vec<Line<'static>> {
    if summary.tasks_total == 0 {
        return vec![Line::from("(no tasks tracked yet)")];
    }
    let title_max = width.saturating_sub(2); // 1 glyph cell + 1 space
    let mut lines = vec![tasks_gauge_line(
        summary.tasks_done,
        summary.tasks_total,
        width,
    )];
    for task in &summary.tasks {
        let (glyph, color) = match task.state {
            TaskState::Done => ("☑", Color::Green),
            TaskState::Active => ("▶", Color::Yellow),
            TaskState::Pending => ("☐", Color::Rgb(200, 200, 200)),
        };
        let task_tick = if task.state == TaskState::Active {
            tick
        } else {
            None
        };
        lines.push(Line::from(vec![
            Span::styled(glyph, Style::new().fg(color)),
            Span::raw(format!(
                " {}",
                scrolled_title(&task.title, title_max, task_tick)
            )),
        ]));
    }
    lines
}

/// Window of a task title to show within `max` chars. Titles that fit are
/// returned whole. Overflowing titles pan **back and forth** (ping-pong) driven
/// by `tick`: the visible window slides 0→overflow then overflow→0, repeating.
/// `tick == None` (session ended) or a fitting title → the static head window.
fn scrolled_title(title: &str, max: usize, tick: Option<usize>) -> String {
    let chars: Vec<char> = title.chars().collect();
    if chars.len() <= max || max == 0 {
        return truncate_title(title, max);
    }
    let overflow = chars.len() - max;
    let start = match tick {
        Some(t) => {
            // Triangle wave over [0, overflow]: pan right, then back left.
            // 0.75 chars/tick (3 chars per 4 ticks).
            let step = t * 3 / 4;
            let period = overflow * 2;
            let phase = step % period;
            if phase <= overflow {
                phase
            } else {
                period - phase
            }
        }
        None => return truncate_title(title, max),
    };
    chars[start..start + max].iter().collect()
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

/// Budget-panel savings block. A `Savings` header followed by indented,
/// value-aligned rows: `Session` (always, when session metrics exist), then
/// `Milestone` and `Project` (each only when its token data is available).
/// Dollar values are right-aligned so their decimals line up in a column.
/// Returns empty when there are no session metrics yet — never a lone header.
pub(crate) fn savings_lines(
    summary: &StatusSummary,
    rates: BudgetRates,
    milestone_tok: Option<(u32, u32)>,
    project_tok: (u32, u32),
) -> Vec<Line<'static>> {
    let in_tok = match summary.last_input_tokens {
        Some(v) => v,
        None => return Vec::new(),
    };
    let out_tok = summary.last_output_tokens.unwrap_or(0);
    let no_rates = rates.input_per_mtok == 0.0 && rates.output_per_mtok == 0.0;

    // Dollar value for a scope, or an em-dash when no rates are configured.
    let value = |i: u32, o: u32| -> String {
        if no_rates {
            "—".to_string()
        } else {
            let saved = dollars_saved(i, o, rates.input_per_mtok, rates.output_per_mtok);
            format!("${saved:.2}")
        }
    };
    // Indented row: label padded left, value padded right (decimals align).
    // `lw` covers the longest label ("Milestone:"); `vw` holds "$XXXX.XX".
    let row = |label: &str, v: String| -> Line<'static> {
        Line::from(format!("  {:<lw$}{:>vw$}", label, v, lw = 11, vw = 9))
    };

    let mut lines = vec![Line::from("Savings")];
    lines.push(row("Session:", value(in_tok, out_tok)));
    if let Some((m_in, m_out)) = milestone_tok {
        lines.push(row("Milestone:", value(m_in, m_out)));
    }
    let (p_in, p_out) = project_tok;
    if p_in > 0 || p_out > 0 {
        lines.push(row("Project:", value(p_in, p_out)));
    }
    lines
}

/// "last update: …" freshness line for the Budget panel — the age of the most
/// recent record, with the average update interval when enough records exist.
/// `Some` whenever the session has at least one record (`last_ts`); `None` for an
/// empty log. Returns an optional single line, unlike the multi-line
/// `savings_lines` block.
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
        assert!(spinner_line(None, 40).is_none());
    }

    #[test]
    fn spinner_line_contains_dog_and_brain_during_chase() {
        // tick 0 is in the chase phase (phase=0 <= track for width=40)
        let line = spinner_line(Some(0), 40).unwrap();
        let s = format!("{}", line);
        assert!(s.contains('🐕'), "missing dog: {s}");
        assert!(s.contains('🧠'), "missing brain: {s}");
    }

    #[test]
    fn spinner_line_brain_starts_middle_dog_starts_left() {
        // Tick 0 is the first chase frame: the dog sits flush at the far left and
        // the brain begins around the panel middle, to the dog's right.
        let width: usize = 40;
        let line = spinner_line(Some(0), width).unwrap();
        let s = format!("{}", line);
        assert!(
            s.starts_with('🐕'),
            "dog should start at the far left: {s:?}"
        );
        // Display-cell offset of a sprite's left edge: spaces are 1 cell, the
        // dog sprite ahead of it is 2.
        let cell_of = |target: char| -> usize {
            s.chars()
                .take_while(|&c| c != target)
                .map(|c| if c == ' ' { 1 } else { SPRITE_CELLS })
                .sum()
        };
        let brain_cell = cell_of('🧠');
        // Brain's left edge should begin near the middle (within a sprite of width/2).
        assert!(
            brain_cell >= width / 2 - SPRITE_CELLS && brain_cell <= width / 2 + SPRITE_CELLS,
            "brain should begin near the panel middle (cell {brain_cell}, width {width})"
        );
        assert!(cell_of('🐕') < brain_cell, "dog must be left of the brain");
    }

    #[test]
    fn spinner_line_return_cluster_brain_left_of_dog_with_dash() {
        // Every return frame shows the brain+dog+dash cluster contiguous, with the
        // brain directly left of the dog, traveling back toward the left edge.
        let width: usize = 40;
        let span = width - SPRITE_CELLS;
        let mid = width / 2;
        let steps_a = span - mid;
        let p0 = width - SPRITE_CELLS * 3;
        let steps_b = p0 / 2;
        let period = steps_a + steps_b + 2;

        let mut return_frames = 0;
        let mut prev_lead: Option<usize> = None;
        for phase in (steps_a + 1)..period {
            let line = spinner_line(Some(phase), width).unwrap();
            let s = format!("{}", line);
            assert!(
                s.contains("🧠🐕💨"),
                "return frame must show brain+dog+dash adjacent: {s:?}"
            );
            // The cluster slides leftward (leading spaces strictly decrease).
            let lead = s.chars().take_while(|&c| c == ' ').count();
            if let Some(p) = prev_lead {
                assert!(lead < p, "return cluster must move left: {lead} !< {p}");
            }
            prev_lead = Some(lead);
            return_frames += 1;
        }
        assert_eq!(return_frames, steps_b + 1, "return phase length");
    }

    #[test]
    fn spinner_line_scales_with_width() {
        // Count distinct dog offsets (leading space count) over a full cycle.
        fn distinct_dog_offsets(width: usize) -> usize {
            let track = width.saturating_sub(SPRITE_CELLS * 2);
            let period = track + 2;
            let mut offsets = std::collections::HashSet::new();
            for tick in 0..period {
                let line = spinner_line(Some(tick), width).unwrap();
                let s = format!("{}", line);
                let leading = s.len() - s.trim_start().len();
                offsets.insert(leading);
            }
            offsets.len()
        }
        let offsets_w20 = distinct_dog_offsets(20);
        let offsets_w60 = distinct_dog_offsets(60);
        assert!(
            offsets_w60 > offsets_w20,
            "wider panel should have more distinct dog offsets (w20={offsets_w20}, w60={offsets_w60})"
        );
    }

    #[test]
    fn spinner_line_never_exceeds_width() {
        for &width in &[10_usize, 20, 40, 80] {
            let track = width.saturating_sub(SPRITE_CELLS * 2);
            let period = track + 2;
            for tick in 0..period {
                let line = spinner_line(Some(tick), width).unwrap();
                let s = format!("{}", line);
                let char_count = s.chars().count();
                assert!(
                    char_count <= width,
                    "width={width} tick={tick}: char_count={char_count} exceeds width ({s:?})"
                );
            }
        }
    }

    #[test]
    fn spinner_line_degenerate_narrow_width() {
        // At width <= SPRITE_CELLS * 2, track == 0, so the line is "🐕🧠".
        for &width in &[0_usize, 1, 2, 3, 4] {
            let line = spinner_line(Some(0), width);
            assert!(line.is_some(), "width={width}: expected Some");
            let s = format!("{}", line.unwrap());
            assert_eq!(s, "🐕🧠", "width={width}: expected dog+brain: {s:?}");
        }
    }

    // --- milestone_line tests ---

    #[test]
    fn milestone_line_prefixes_and_truncates() {
        // Full name fits within width
        let line = milestone_line("M15 — Dashboard Polish 2", 80);
        let text = format!("{line}");
        assert!(text.contains("Milestone: M15 — Dashboard Polish 2"));

        // Narrow width truncates with ellipsis
        let line = milestone_line("M15 — Dashboard Polish 2", 20);
        let text = format!("{line}");
        assert!(
            text.contains('…'),
            "narrow width should truncate with ellipsis: {text}"
        );
        assert!(
            text.chars().count() <= 20,
            "truncated line must fit within width: {} chars",
            text.chars().count()
        );
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
        let lines = tasks_lines(&summary, 40, None);
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
        let lines = tasks_lines(&summary, 40, None);
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
        let lines = tasks_lines(&summary, 26, None);
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
        let lines = tasks_lines(&summary, 60, None);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(
            text.iter()
                .any(|s| s.contains(&title_50) && !s.contains('…')),
            "50-char title should not be truncated at width=60: {text:?}"
        );
        // width=28: title_max=26, 50-char title is truncated.
        let lines_narrow = tasks_lines(&summary, 28, None);
        let text_narrow: Vec<String> = lines_narrow.iter().map(|l| format!("{l}")).collect();
        assert!(
            text_narrow.iter().any(|s| s.contains('…')),
            "50-char title should be truncated at width=28: {text_narrow:?}"
        );
    }

    #[test]
    fn tasks_lines_non_active_tasks_do_not_pan() {
        // 30-char title; title_max = 20, overflow = 10.
        // At tick=4: step = 4*3/4 = 3, so Active window shifts to chars[3..23].
        // Done/Pending receive tick=None → truncate_title → frozen at "abcdefghijklmnopqrst…".
        let long = "abcdefghijklmnopqrstuvwxyzABCD".to_string(); // 30 distinct chars
        let summary = StatusSummary {
            tasks_total: 3,
            tasks_done: 1,
            tasks: vec![
                crate::status::TaskRow {
                    id: "a".into(),
                    title: long.clone(),
                    state: TaskState::Done,
                },
                crate::status::TaskRow {
                    id: "b".into(),
                    title: long.clone(),
                    state: TaskState::Active,
                },
                crate::status::TaskRow {
                    id: "c".into(),
                    title: long.clone(),
                    state: TaskState::Pending,
                },
            ],
            ..StatusSummary::default()
        };
        let width = 22; // title_max = 20
        let lines_0 = tasks_lines(&summary, width, Some(0));
        let lines_4 = tasks_lines(&summary, width, Some(4));
        let text_0: Vec<String> = lines_0.iter().map(|l| format!("{l}")).collect();
        let text_4: Vec<String> = lines_4.iter().map(|l| format!("{l}")).collect();
        // Index 0 = gauge, 1 = done task, 2 = active task, 3 = pending task.
        assert_eq!(text_0[1], text_4[1], "done task must not pan");
        assert_eq!(text_0[3], text_4[3], "pending task must not pan");
        assert_ne!(text_0[2], text_4[2], "active task must pan at tick=4");
    }

    #[test]
    fn tasks_gauge_line_full_is_green_and_complete() {
        let line = tasks_gauge_line(4, 4, 40);
        let text = format!("{line}");
        assert!(text.contains("4/4"), "should contain fraction: {text}");
        assert!(text.contains("100%"), "should contain 100%%: {text}");
        assert_eq!(
            text.matches('█').count(),
            29,
            "should have 29 filled cells: {text}"
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
        let line = tasks_gauge_line(1, 2, 40);
        let text = format!("{line}");
        assert!(text.contains("1/2"), "should contain fraction: {text}");
        assert!(text.contains("50%"), "should contain 50%%: {text}");
        assert_eq!(
            text.matches('█').count(),
            15,
            "should have 15 filled cells: {text}"
        );
        assert_eq!(
            text.matches('░').count(),
            15,
            "should have 15 empty cells: {text}"
        );
        assert_eq!(
            line.spans[0].style.fg,
            Some(Color::Yellow),
            "should be yellow"
        );
    }

    #[test]
    fn tasks_gauge_line_zero_progress() {
        let line = tasks_gauge_line(0, 5, 40);
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
            31,
            "should have 31 empty cells: {text}"
        );
        assert_eq!(
            line.spans[0].style.fg,
            Some(Color::Rgb(200, 200, 200)),
            "should be grey"
        );
    }

    #[test]
    fn tasks_gauge_line_fraction_and_fill() {
        let line = tasks_gauge_line(3, 8, 40);
        let text = format!("{line}");
        assert!(text.contains("3/8"), "should contain fraction: {text}");
        assert!(
            text.contains("38%"),
            "should contain 38%% (round(37.5)): {text}"
        );
        assert_eq!(
            text.matches('█').count(),
            11,
            "should have 11 filled cells (round(11.25) at gauge_cells=30): {text}"
        );
    }

    #[test]
    fn tasks_gauge_line_zero_total_does_not_panic() {
        let line = tasks_gauge_line(0, 0, 40);
        let text = format!("{line}");
        assert!(text.contains("0/0"), "should contain 0/0: {text}");
        assert!(text.contains("0%"), "should contain 0%%: {text}");
    }

    #[test]
    fn tasks_gauge_line_fills_panel_width() {
        // pct = round(3/7*100) = 43; suffix = " 3/7 (43%)" = 10 chars;
        // gauge_cells = 40-10 = 30; text.chars().count() = 30 + 10 = 40.
        let width = 40;
        let line = tasks_gauge_line(3, 7, width);
        let text = format!("{line}");
        assert_eq!(
            text.chars().count(),
            width,
            "gauge line must fill panel width {width}: got {} chars in {text:?}",
            text.chars().count()
        );
    }

    // --- scrolled_title tests ---

    const FIXTURE: &str = "abcdefghijklmnopqrstuvwxyzABCD"; // 30 distinct chars

    #[test]
    fn scrolled_title_returns_whole_when_fits() {
        assert_eq!(scrolled_title("short", 20, Some(5)), "short");
    }

    #[test]
    fn scrolled_title_pans_overflowing_title() {
        let max = 10;
        // tick = 0 → start 0 → "abcdefghij"
        assert_eq!(scrolled_title(FIXTURE, max, Some(0)), "abcdefghij");
        // tick = 4 → step = 4*3/4 = 3 → start 3 → "defghijklm"
        assert_eq!(scrolled_title(FIXTURE, max, Some(4)), "defghijklm");
    }

    #[test]
    fn scrolled_title_ping_pongs() {
        let max = 10;
        let overflow = FIXTURE.len() - max; // 20
        let mut starts = Vec::new();
        for t in 0..=200usize {
            let window = scrolled_title(FIXTURE, max, Some(t));
            let start = FIXTURE.find(&window).unwrap_or(0);
            starts.push(start);
        }
        let max_start = *starts.iter().max().unwrap();
        assert_eq!(
            max_start, overflow,
            "max start ({max_start}) should equal overflow ({overflow})"
        );
        let descends = starts.windows(2).any(|w| w[1] < w[0]);
        assert!(descends, "ping-pong sequence must descend at some point");
    }

    #[test]
    fn scrolled_title_frozen_when_tick_none() {
        let max = 10;
        let frozen = scrolled_title(FIXTURE, max, None);
        // Frozen uses truncate_title: max-1 chars + "…"
        assert_eq!(frozen, truncate_title(FIXTURE, max));
        assert_eq!(frozen, "abcdefghi…");
        // Scrolling head (Some(0)) is the raw first `max` chars — no ellipsis.
        assert_eq!(scrolled_title(FIXTURE, max, Some(0)), "abcdefghij");
    }

    #[test]
    fn scrolled_title_char_indexed_multibyte() {
        let title = "日本語テスト日本語テスト日本語テスト";
        let max = 5;
        let result = scrolled_title(title, max, Some(4));
        let chars: Vec<char> = result.chars().collect();
        assert_eq!(
            chars.len(),
            max,
            "should return exactly max chars for multibyte title"
        );
    }

    // --- budget_lines tests ---

    #[test]
    fn budget_lines_shows_tokens() {
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
        assert!(
            !text
                .iter()
                .any(|s| s.contains("Context:") || s.contains("Usage:")),
            "context line must not appear in budget_lines"
        );
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

    #[test]
    fn reclaim_lines_shows_usage_when_context_pct_set() {
        let summary = StatusSummary {
            last_context_pct: Some(0.62),
            ..StatusSummary::default()
        };
        let lines = reclaim_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(
            text.iter().any(|s| s.contains("Usage:")),
            "Usage line must be present: {text:?}"
        );
        assert!(
            text.iter().any(|s| s.contains("62%")),
            "percentage must appear: {text:?}"
        );
    }

    #[test]
    fn reclaim_lines_usage_is_first_line() {
        let summary = StatusSummary {
            last_context_pct: Some(0.55),
            compaction_count: 1,
            compaction_tokens_before: 1000,
            compaction_tokens_after: 600,
            ..StatusSummary::default()
        };
        let lines = reclaim_lines(&summary);
        let first = format!("{}", lines[0]);
        assert!(
            first.contains("Usage:"),
            "Usage must be the first line; got: {first}"
        );
    }

    #[test]
    fn reclaim_lines_usage_color_red_when_high() {
        let summary = StatusSummary {
            last_context_pct: Some(0.85),
            ..StatusSummary::default()
        };
        let lines = reclaim_lines(&summary);
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(Color::Red),
            "pct >= 80 must render red"
        );
    }

    #[test]
    fn reclaim_lines_usage_shows_fraction_with_used_and_window() {
        let summary = StatusSummary {
            last_context_pct: Some(0.68),
            last_context_used: Some(31195),
            last_context_window: Some(45875),
            ..StatusSummary::default()
        };
        let lines = reclaim_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        let usage_line = text.iter().find(|s| s.contains("Usage:")).unwrap();
        assert!(usage_line.contains("68%"), "pct in: {usage_line}");
        assert!(usage_line.contains("31195"), "used in: {usage_line}");
        assert!(usage_line.contains("45875"), "window in: {usage_line}");
    }

    #[test]
    fn reclaim_lines_usage_unmeasured_when_zero_pct() {
        let summary = StatusSummary {
            last_context_pct: Some(0.0),
            ..StatusSummary::default()
        };
        let lines = reclaim_lines(&summary);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("unmeasured")));
        assert!(!text.iter().any(|s| s.contains("0%")));
    }

    // --- savings_lines tests ---

    #[test]
    fn savings_lines_empty_without_session_metrics() {
        let rates = BudgetRates {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
        };
        let result = savings_lines(&StatusSummary::default(), rates, None, (0, 0));
        assert!(result.is_empty(), "no session tokens → no header, no lines");
    }

    #[test]
    fn savings_lines_starts_with_header() {
        let summary = StatusSummary {
            last_input_tokens: Some(500),
            last_output_tokens: Some(100),
            ..StatusSummary::default()
        };
        let rates = BudgetRates {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
        };
        let lines = savings_lines(&summary, rates, None, (0, 0));
        assert_eq!(
            format!("{}", lines[0]),
            "Savings",
            "first line is the header"
        );
    }

    #[test]
    fn savings_lines_session_dash_when_rates_unset() {
        let summary = StatusSummary {
            last_input_tokens: Some(500),
            last_output_tokens: Some(100),
            ..StatusSummary::default()
        };
        let rates = BudgetRates::default();
        let lines = savings_lines(&summary, rates, None, (0, 0));
        let row = format!("{}", lines[1]);
        assert!(row.starts_with("  Session:"), "session row: {row}");
        assert!(
            row.ends_with('—'),
            "value is the em-dash when rates unset: {row}"
        );
    }

    #[test]
    fn savings_lines_session_shows_dollars() {
        let summary = StatusSummary {
            last_input_tokens: Some(1_000_000),
            last_output_tokens: Some(500_000),
            ..StatusSummary::default()
        };
        let rates = BudgetRates {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
        };
        let lines = savings_lines(&summary, rates, None, (0, 0));
        // 1.0*3 + 0.5*15 = $10.50; right-aligned under the header.
        assert_eq!(format!("{}", lines[1]), "  Session:      $10.50");
        assert_eq!(lines.len(), 2, "header + session only");
    }

    #[test]
    fn savings_lines_shows_milestone_when_provided() {
        let summary = StatusSummary {
            last_input_tokens: Some(1_000_000),
            last_output_tokens: Some(500_000),
            ..StatusSummary::default()
        };
        let rates = BudgetRates {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
        };
        let lines = savings_lines(&summary, rates, Some((1_000_000, 500_000)), (0, 0));
        assert_eq!(
            lines.len(),
            3,
            "header + session + milestone, no project (0,0)"
        );
        assert!(
            format!("{}", lines[2]).contains("Milestone:"),
            "{:?}",
            lines
        );
    }

    #[test]
    fn savings_lines_shows_all_three_scopes_value_aligned() {
        let summary = StatusSummary {
            last_input_tokens: Some(500_000),
            last_output_tokens: Some(200_000),
            ..StatusSummary::default()
        };
        let rates = BudgetRates {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
        };
        let lines = savings_lines(
            &summary,
            rates,
            Some((2_000_000, 800_000)),
            (10_000_000, 4_000_000),
        );
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert_eq!(
            lines.len(),
            4,
            "header + session + milestone + project: {text:?}"
        );
        assert_eq!(text[0], "Savings");
        assert!(text[1].contains("Session:"), "{}", text[1]);
        assert!(text[2].contains("Milestone:"), "{}", text[2]);
        assert!(text[3].contains("Project:"), "{}", text[3]);
        // Alignment guarantee: all three scope rows share one width, so their
        // right-aligned values land in the same column.
        let widths: Vec<usize> = text[1..].iter().map(|s| s.chars().count()).collect();
        assert!(
            widths.iter().all(|&w| w == widths[0]),
            "scope rows must be equal width for value alignment: {widths:?}"
        );
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
