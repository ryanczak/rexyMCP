use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};

use super::filter::ActivityFilter;
use super::highlight::{highlighted_body_lines, plain_body_lines};

/// Max chars of free-text content shown per transcript line in 10a (10b expands
/// to full multi-line). Keeps one record = one line.
pub(crate) const TRANSCRIPT_PREVIEW_MAX: usize = 100;

pub(crate) const SPINNER_FRAMES: &[&str] = &[
    "🐕       🧠",
    " 🐕     🧠",
    "  🐕   🧠   ",
    "   🐕 🧠  ",
    "    🐕🧠 ",
    "  🧠🐕💨",
    " 🧠🐕",
    "🧠🐕",
    "🐕",
];

/// Build all transcript lines for the given records, in chronological order.
/// Filters records through `filter`, appends a spinner frame when `spinner` is
/// `Some`. Returns a placeholder when all records are filtered out.
pub(crate) fn transcript_lines(
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

/// Render one record as one or more transcript lines (header + optional body),
/// styled by event type. Completion and ToolResult expand their content across
/// multiple lines; all other events are a single styled header line.
pub(crate) fn record_lines(rec: &SessionRecord) -> Vec<Line<'static>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::highlight::TRANSCRIPT_CONTENT_MAX_LINES;
    use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};

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

    /// Join a record's rendered lines into one string for content assertions.
    fn record_text(rec: &SessionRecord) -> String {
        record_lines(rec)
            .iter()
            .map(|l| format!("{l}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn transcript_lines_empty_placeholder() {
        let lines = transcript_lines(&[], &ActivityFilter::default(), None);
        let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        assert!(text.iter().any(|s| s.contains("no activity")));
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
        assert_eq!(last, "🐕       🧠");
    }

    #[test]
    fn spinner_frame_cycles_through_all_frames() {
        let records = vec![rec(100, 0, start_event())];
        for (i, expected) in SPINNER_FRAMES.iter().enumerate() {
            let lines = transcript_lines(&records, &ActivityFilter::default(), Some(i));
            let last = format!("{}", lines.last().unwrap());
            assert_eq!(last, *expected, "frame {i} mismatch");
        }
        // Index 9 wraps to frame 0 (9 frames total, 9 % 9 == 0)
        let lines = transcript_lines(&records, &ActivityFilter::default(), Some(9));
        let last = format!("{}", lines.last().unwrap());
        assert_eq!(last, SPINNER_FRAMES[0], "frame 9 should wrap to 0");
    }

    #[test]
    fn spinner_absent_when_none() {
        let records = vec![rec(100, 0, start_event())];
        let lines = transcript_lines(&records, &ActivityFilter::default(), None);
        let last = format!("{}", lines.last().unwrap());
        assert!(!last.contains("🐕"), "spinner should not appear: {last}");
    }

    #[test]
    fn spinner_appended_to_empty_records() {
        let lines = transcript_lines(&[], &ActivityFilter::default(), Some(3));
        assert_eq!(lines.len(), 2);
        assert_eq!(format!("{}", lines[0]), "(no activity yet)");
        assert_eq!(format!("{}", lines[1]), "   🐕 🧠  ");
    }
}
