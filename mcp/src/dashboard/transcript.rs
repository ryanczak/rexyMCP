use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};

use super::filter::ActivityFilter;
use super::highlight::{
    completion_body_lines, highlighted_body_lines, highlighted_body_lines_for, plain_body_lines,
};

/// Max chars of free-text content shown per transcript line in 10a (10b expands
/// to full multi-line). Keeps one record = one line.
pub(crate) const TRANSCRIPT_PREVIEW_MAX: usize = 100;

/// Beautify a tool call's arguments into compact, dimmed body lines. `patch`
/// shows only the target path — its `old_str`/`new_str` are echoed as a unified
/// diff in the paired result, so repeating them on the call is noise. Every
/// other tool renders its scalar args as `key: value` previews, one per line,
/// with newlines/tabs flattened and long values truncated. Returns an empty Vec
/// when there is nothing worth showing (no args, or `patch` without a path).
fn tool_arg_lines(name: &str, args: &serde_json::Value) -> Vec<String> {
    let obj = match args {
        serde_json::Value::Object(m) if !m.is_empty() => m,
        _ => return Vec::new(),
    };
    if name == "patch" {
        return obj
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| vec![format!("path: {p}")])
            .unwrap_or_default();
    }
    obj.iter()
        .map(|(k, v)| format!("{k}: {}", arg_value_preview(v)))
        .collect()
}

/// One-line preview of a single JSON argument value: strings are flattened
/// (newlines/tabs → spaces) and truncated via `preview`; other scalars use their
/// compact JSON form so the rendered arg stays on one neat line.
fn arg_value_preview(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => preview(s),
        other => other.to_string(),
    }
}

/// Tracks the most-recent tool call so a following result can be paired with it.
struct PendingCall {
    name: String,
    path: Option<String>,
}

/// Build all transcript lines for the given records, in chronological order.
/// Filters records through `filter`. Returns a placeholder when all records are
/// filtered out.
pub(crate) fn transcript_lines(
    records: &[SessionRecord],
    filter: &ActivityFilter,
) -> Vec<Line<'static>> {
    let visible: Vec<_> = records.iter().filter(|r| filter.allows(&r.event)).collect();
    if visible.is_empty() {
        return vec![Line::from("(no activity yet)")];
    }
    let base_ts = records.first().map(|r| r.ts).unwrap_or(0);
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut pending: Option<PendingCall> = None;
    for r in &visible {
        let mut paired = false;
        let mut hint: Option<String> = None;
        match &r.event {
            SessionEvent::Parsed { tool_call } => {
                let path = if tool_call.name == "read_file" {
                    tool_call
                        .arguments
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                } else {
                    None
                };
                pending = Some(PendingCall {
                    name: tool_call.name.clone(),
                    path,
                });
            }
            SessionEvent::ToolResult { name, .. } => {
                if let Some(p) = &pending
                    && &p.name == name
                {
                    paired = true;
                    if name == "read_file" {
                        hint = p.path.clone();
                    }
                }
                pending = None;
            }
            _ => {}
        }

        let mut lines = record_lines_with_lang(r, hint.as_deref(), paired);

        if let Some(header) = lines.first_mut() {
            let mut spans = Vec::with_capacity(header.spans.len() + 1);
            spans.push(Span::styled(
                format!("[{}] ", relative_ts(r.ts, base_ts)),
                Style::new().fg(Color::Rgb(180, 150, 50)),
            ));
            spans.append(&mut header.spans);
            *header = Line::from(spans);
        }
        out.extend(lines);
    }
    out
}

/// Render one record as one or more transcript lines (header + optional body),
/// styled by event type. Completion and ToolResult expand their content across
/// multiple lines; all other events are a single styled header line.
#[cfg(test)]
fn record_lines(rec: &SessionRecord) -> Vec<Line<'static>> {
    record_lines_with_lang(rec, None, false)
}

/// As `record_lines`, but a `read_file` `ToolResult` body is highlighted using
/// the grammar for `path_hint`'s extension when provided. When `paired` is
/// `true`, the `ToolResult` header renders as a paired connector (`╰ [ok]`)
/// instead of the standalone `tool {name} [{status}]` form.
pub(crate) fn record_lines_with_lang(
    rec: &SessionRecord,
    path_hint: Option<&str>,
    paired: bool,
) -> Vec<Line<'static>> {
    // (header_summary, header_color, bold, body_lines)
    let (summary, color, bold, body): (String, Color, bool, Option<Vec<Line<'static>>>) = match &rec
        .event
    {
        SessionEvent::SessionStart { model, phase, .. } => (
            format!("session start — phase {phase}, model {model}"),
            Color::Cyan,
            false,
            None,
        ),
        SessionEvent::Prompt { rendered } => (
            format!("prompt ({} chars)", rendered.chars().count()),
            Color::Rgb(200, 200, 200),
            false,
            Some(plain_body_lines(rendered, Color::Rgb(200, 200, 200))),
        ),
        SessionEvent::Completion { raw } => (
            "completion:".to_string(),
            Color::Reset,
            false,
            Some(completion_body_lines(raw)),
        ),
        SessionEvent::Parsed { tool_call } => {
            let arg_lines = tool_arg_lines(&tool_call.name, &tool_call.arguments);
            let body = if arg_lines.is_empty() {
                None
            } else {
                Some(plain_body_lines(
                    &arg_lines.join("\n"),
                    Color::Rgb(128, 128, 128),
                ))
            };
            (
                format!("→ call {}", tool_call.name),
                Color::Blue,
                false,
                body,
            )
        }
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
            ..
        } => {
            let status = if *succeeded { "ok" } else { "FAIL" };
            let color = if *succeeded { Color::Green } else { Color::Red };
            let summary = if paired {
                format!("╰ [{status}]")
            } else {
                format!("tool {name} [{status}]")
            };
            (
                summary,
                color,
                false,
                Some(match path_hint {
                    Some(p) => highlighted_body_lines_for(output_preview, Some(p)),
                    None => highlighted_body_lines(output_preview),
                }),
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
        SessionEvent::Progress { stage, .. } => (
            format!("progress: {stage}"),
            Color::Rgb(200, 200, 200),
            false,
            None,
        ),
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
            Color::Rgb(200, 200, 200),
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
        SessionEvent::OutputFiltered {
            tokens_before,
            tokens_after,
            filter,
        } => (
            format!("filtered ({filter}): {tokens_before} → {tokens_after} tokens"),
            Color::Cyan,
            false,
            None,
        ),
        SessionEvent::ReadEvicted {
            reads_evicted,
            tokens_reclaimed,
            ..
        } => (
            format!("evicted {reads_evicted} stale read(s): -{tokens_reclaimed} tokens"),
            Color::Cyan,
            false,
            None,
        ),
        SessionEvent::ReadDeduped {
            tokens_saved,
            prior_turn,
            ..
        } => (
            format!("deduped re-read (already read turn {prior_turn}): -{tokens_saved} tokens"),
            Color::Cyan,
            false,
            None,
        ),
        SessionEvent::TaskUpdate { id, title, state } => (
            format!("task {id} [{state:?}]: {title}"),
            Color::Yellow,
            false,
            None,
        ),
        SessionEvent::NoveltySample {
            window,
            distinct_targets,
        } => (
            format!("novelty: {distinct_targets} distinct target(s) over {window} read-only calls"),
            Color::Cyan,
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

/// Relative timestamp for a transcript record: `+{humanized}` elapsed since the
/// session's first record (`base_ts`). `+0s` at the baseline; `saturating_sub`
/// guards a record that reads before the baseline (shouldn't happen — records are
/// chronological — but stays panic-free). Reuses the Session-panel duration
/// formatter so the buckets match (`5s` / `3m12s` / `1h04m`).
fn relative_ts(ts: u64, base_ts: u64) -> String {
    format!(
        "+{}",
        crate::status::humanize_age(ts.saturating_sub(base_ts))
    )
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
        let lines = transcript_lines(&[], &ActivityFilter::default());
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
            output_bytes: 0,
        };
        let t = record_text(&rec(300, 2, tool_ok));
        assert!(t.contains("[t2] tool read_file [ok]"));
        assert!(t.contains("file contents"));

        // ToolResult FAIL
        let tool_fail = SessionEvent::ToolResult {
            name: "bash".into(),
            succeeded: false,
            output_preview: "error output".into(),
            output_bytes: 0,
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
            output_bytes: 0,
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
        let lines = transcript_lines(&records, &ActivityFilter::default());
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn prompt_header_uses_soft_white() {
        let lines = record_lines(&rec(
            0,
            0,
            SessionEvent::Prompt {
                rendered: "hi".into(),
            },
        ));
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Rgb(200, 200, 200)));
    }

    #[test]
    fn progress_header_uses_soft_white() {
        let lines = record_lines(&rec(0, 0, progress_event(0, "build")));
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Rgb(200, 200, 200)));
    }

    #[test]
    fn metrics_header_uses_soft_white() {
        let lines = record_lines(&rec(
            0,
            0,
            SessionEvent::Metrics {
                input_tokens: 10,
                output_tokens: 5,
                context_pct: 0.0,
                context_used: 0,
                context_window: 0,
            },
        ));
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Rgb(200, 200, 200)));
    }

    #[test]
    fn prompt_body_shows_rendered_text_soft_white() {
        let lines = record_lines(&rec(
            0,
            0,
            SessionEvent::Prompt {
                rendered: "injected ctx".into(),
            },
        ));
        // 1 header + at least 1 body line
        assert!(
            lines.len() >= 2,
            "expected header + body, got {} lines",
            lines.len()
        );
        // Body line contains the rendered text and uses soft white
        let body_line = &lines[1];
        let text = format!("{}", body_line);
        assert!(
            text.contains("injected ctx"),
            "body should contain rendered text: {text}"
        );
        assert_eq!(
            body_line.spans[0].style.fg,
            Some(Color::Rgb(200, 200, 200)),
            "body span should be soft white"
        );
    }

    #[test]
    fn tool_call_args_render_dimmed() {
        let parsed = SessionEvent::Parsed {
            tool_call: rexymcp_executor::parser::ToolCall {
                name: "read_file".into(),
                arguments: serde_json::json!({ "path": "x.rs" }),
                origin: rexymcp_executor::parser::Origin::Native,
            },
        };
        let lines = record_lines(&rec(0, 0, parsed));
        // 1 header + at least 1 body line
        assert!(
            lines.len() >= 2,
            "expected header + body, got {} lines",
            lines.len()
        );
        // Collect body text and check for arg key/value
        let body_texts: Vec<String> = lines[1..].iter().map(|l| format!("{}", l)).collect();
        let full_body = body_texts.join("\n");
        assert!(
            full_body.contains("path"),
            "body should contain arg key: {full_body}"
        );
        assert!(
            full_body.contains("x.rs"),
            "body should contain arg value: {full_body}"
        );
        // First body span should be dim grey
        assert_eq!(
            lines[1].spans[0].style.fg,
            Some(Color::Rgb(128, 128, 128)),
            "body span should be dim grey"
        );
    }

    #[test]
    fn tool_call_patch_shows_only_path() {
        // patch's old_str/new_str appear as a diff in the paired result, so the
        // call body shows just the file being patched — not the replacement text.
        let parsed = SessionEvent::Parsed {
            tool_call: rexymcp_executor::parser::ToolCall {
                name: "patch".into(),
                arguments: serde_json::json!({
                    "path": "src/foo.rs",
                    "old_str": "fn old() {}",
                    "new_str": "fn new() {}",
                }),
                origin: rexymcp_executor::parser::Origin::Native,
            },
        };
        let lines = record_lines(&rec(0, 0, parsed));
        // 1 header + exactly 1 body line (the path).
        assert_eq!(
            lines.len(),
            2,
            "patch call should render header + one path line, got {} lines",
            lines.len()
        );
        let body = format!("{}", lines[1]);
        assert!(body.contains("src/foo.rs"), "body should show path: {body}");
        assert!(
            !body.contains("old()") && !body.contains("new()"),
            "patch body must not echo old_str/new_str: {body}"
        );
    }

    #[test]
    fn tool_call_args_flatten_multiline_values() {
        // A bash command with embedded newlines collapses to a single neat line.
        let parsed = SessionEvent::Parsed {
            tool_call: rexymcp_executor::parser::ToolCall {
                name: "bash".into(),
                arguments: serde_json::json!({ "command": "echo a\necho b" }),
                origin: rexymcp_executor::parser::Origin::Native,
            },
        };
        let lines = record_lines(&rec(0, 0, parsed));
        assert_eq!(
            lines.len(),
            2,
            "header + one flattened command line, got {} lines",
            lines.len()
        );
        let body = format!("{}", lines[1]);
        assert!(
            body.contains("command:"),
            "body should label command: {body}"
        );
        assert!(body.contains("echo a") && body.contains("echo b"), "{body}");
    }

    #[test]
    fn tool_call_empty_args_render_header_only() {
        // Empty object
        let parsed = SessionEvent::Parsed {
            tool_call: rexymcp_executor::parser::ToolCall {
                name: "read_file".into(),
                arguments: serde_json::json!({}),
                origin: rexymcp_executor::parser::Origin::Native,
            },
        };
        let lines = record_lines(&rec(0, 0, parsed));
        assert_eq!(
            lines.len(),
            1,
            "empty-object args should render header only, got {} lines",
            lines.len()
        );

        // Null arguments
        let parsed_null = SessionEvent::Parsed {
            tool_call: rexymcp_executor::parser::ToolCall {
                name: "read_file".into(),
                arguments: serde_json::Value::Null,
                origin: rexymcp_executor::parser::Origin::Native,
            },
        };
        let lines_null = record_lines(&rec(0, 0, parsed_null));
        assert_eq!(
            lines_null.len(),
            1,
            "null args should render header only, got {} lines",
            lines_null.len()
        );
    }

    #[test]
    fn prompt_body_caps_long_text() {
        let body: String = (0..TRANSCRIPT_CONTENT_MAX_LINES + 5)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = record_lines(&rec(0, 0, SessionEvent::Prompt { rendered: body }));
        // 1 header + TRANSCRIPT_CONTENT_MAX_LINES body + 1 overflow marker
        assert_eq!(lines.len(), 1 + TRANSCRIPT_CONTENT_MAX_LINES + 1);
        let last = format!("{}", lines[lines.len() - 1]);
        assert!(
            last.contains("more lines"),
            "last line should be the overflow marker: {last}"
        );
    }

    #[test]
    fn completion_record_styles_think_block() {
        let ev = SessionEvent::Completion {
            raw: "<think>reasoning here</think>the answer".into(),
        };
        let lines = record_lines(&rec(0, 0, ev));

        // Header line.
        let header = format!("{}", lines[0]);
        assert!(header.contains("completion:"));

        // Body lines (skip header).
        let body = &lines[1..];
        let think_line = body
            .iter()
            .find(|l| format!("{l}").contains("reasoning here"))
            .expect("should have a line containing 'reasoning here'");
        assert_eq!(
            think_line.spans[0].style.fg,
            Some(Color::Rgb(128, 128, 128)),
            "think line should be dim grey"
        );

        let answer_line = body
            .iter()
            .find(|l| format!("{l}").contains("the answer"))
            .expect("should have a line containing 'the answer'");
        // Answer line is now markdown-highlighted: first span is raw indent,
        // subsequent spans carry syntax colors.
        assert!(
            answer_line.spans.len() >= 2,
            "answer line should have indent + content spans, got {}",
            answer_line.spans.len()
        );
        assert!(
            !answer_line.spans[1]
                .style
                .add_modifier
                .contains(Modifier::ITALIC),
            "answer content span should not be italic"
        );

        // Markers should not appear in any body line.
        for line in body {
            let text = format!("{line}");
            assert!(
                !text.contains("<think>"),
                "marker should be removed: {text}"
            );
            assert!(
                !text.contains("</think>"),
                "marker should be removed: {text}"
            );
        }
    }

    #[test]
    fn relative_ts_formats_offset_from_base() {
        assert_eq!(relative_ts(1000, 1000), "+0s");
        assert_eq!(relative_ts(6000, 1000), "+5s");
        assert_eq!(relative_ts(193_000, 1000), "+3m12s");
        // saturating_sub guards underflow — no panic, no negative
        assert_eq!(relative_ts(500, 1000), "+0s");
    }

    #[test]
    fn transcript_lines_prefixes_relative_timestamp() {
        let records = vec![
            rec(1000, 0, start_event()),
            rec(4000, 1, progress_event(1, "verify")),
        ];
        let filter = ActivityFilter {
            progress: true,
            ..Default::default()
        };
        let lines = transcript_lines(&records, &filter);

        let first_header = format!("{}", lines[0]);
        assert!(
            first_header.contains("[+0s]"),
            "first record should have [+0s]: {first_header}"
        );
        assert!(
            first_header.contains("[t0]"),
            "first record should have [t0]: {first_header}"
        );

        // Second record header is at index 1 (no body lines for session_start or progress)
        let second_header = format!("{}", lines[1]);
        assert!(
            second_header.contains("[+3s]"),
            "second record should have [+3s] (4000-1000=3000ms): {second_header}"
        );
        assert!(
            second_header.contains("[t1]"),
            "second record should have [t1]: {second_header}"
        );
    }

    #[test]
    fn transcript_lines_timestamp_relative_to_first_record_not_first_visible() {
        // First record (prompt) is filtered out, but baseline is still records[0].ts
        let records = vec![
            rec(
                1000,
                0,
                SessionEvent::Prompt {
                    rendered: "ctx".into(),
                },
            ),
            rec(5000, 1, progress_event(1, "build")),
        ];
        let filter = ActivityFilter {
            prompt: false,
            progress: true,
            ..Default::default()
        };
        let lines = transcript_lines(&records, &filter);
        assert_eq!(lines.len(), 1, "only the progress record should be visible");
        let header = format!("{}", lines[0]);
        assert!(
            header.contains("[+4s]"),
            "visible record should show [+4s] (5000-1000), not [+0s]: {header}"
        );
    }

    #[test]
    fn transcript_lines_timestamp_only_on_header_not_body() {
        let records = vec![rec(
            1000,
            0,
            SessionEvent::Completion {
                raw: "alpha\nbeta".into(),
            },
        )];
        let lines = transcript_lines(&records, &ActivityFilter::default());

        // Count how many lines contain the timestamp token
        let ts_count = lines
            .iter()
            .filter(|l| format!("{l}").contains("[+0s]"))
            .count();
        assert_eq!(
            ts_count, 1,
            "exactly one line (the header) should contain [+0s], got {ts_count}"
        );

        // Body lines should not contain the timestamp
        for line in &lines[1..] {
            let text = format!("{line}");
            assert!(
                !text.contains("+0s"),
                "body line should not contain timestamp: {text}"
            );
        }
    }

    #[test]
    fn transcript_lines_timestamp_span_is_dull_yellow() {
        let records = vec![rec(1000, 0, start_event())];
        let lines = transcript_lines(&records, &ActivityFilter::default());
        let header = &lines[0];

        // First span is the timestamp gutter
        assert_eq!(
            header.spans[0].style.fg,
            Some(Color::Rgb(180, 150, 50)),
            "timestamp span should be dull yellow"
        );
        let ts_text = &header.spans[0].content;
        assert!(
            ts_text.to_string().starts_with("["),
            "timestamp span content should start with [: {ts_text}"
        );
        assert!(
            ts_text.to_string().contains("+"),
            "timestamp span content should contain +: {ts_text}"
        );

        // Second span is the original header text
        let header_text = &header.spans[1].content;
        assert!(
            header_text.to_string().contains("[t0]"),
            "second span should contain original header: {header_text}"
        );
    }

    #[test]
    fn record_lines_delegates_to_with_lang_none() {
        let rec = rec(
            0,
            0,
            SessionEvent::ToolResult {
                name: "bash".to_string(),
                succeeded: true,
                output_preview: "echo hello".to_string(),
                output_bytes: 0,
            },
        );
        let lines = record_lines(&rec);
        let lines_with_lang = record_lines_with_lang(&rec, None, false);
        assert_eq!(
            lines.len(),
            lines_with_lang.len(),
            "delegated call should produce same line count"
        );
        for (a, b) in lines.iter().zip(lines_with_lang.iter()) {
            assert_eq!(
                format!("{a}"),
                format!("{b}"),
                "delegated call should produce identical rendered text"
            );
        }
    }

    #[test]
    fn transcript_lines_highlights_read_file_by_extension() {
        let records = vec![
            rec(
                0,
                0,
                SessionEvent::Parsed {
                    tool_call: rexymcp_executor::parser::ToolCall {
                        name: "read_file".to_string(),
                        arguments: serde_json::json!({"path": "foo.py"}),
                        origin: rexymcp_executor::parser::Origin::Native,
                    },
                },
            ),
            rec(
                1000,
                0,
                SessionEvent::ToolResult {
                    name: "read_file".to_string(),
                    succeeded: true,
                    output_preview: "def f():\n    pass".to_string(),
                    output_bytes: 0,
                },
            ),
        ];
        let lines = transcript_lines(&records, &ActivityFilter::default());

        // Find the tool-result body lines (lines containing "def f()")
        let code_line = lines
            .iter()
            .find(|l| format!("{l}").contains("def f()"))
            .expect("should have a line with 'def f()'");

        // With Python grammar, the code line should have multiple spans
        assert!(
            code_line.spans.len() > 1,
            "Python-highlighted line should have multiple spans, got {}",
            code_line.spans.len()
        );
    }

    #[test]
    fn transcript_lines_read_file_without_call_falls_back() {
        let records = vec![rec(
            0,
            0,
            SessionEvent::ToolResult {
                name: "read_file".to_string(),
                succeeded: true,
                output_preview: "def f():\n    pass".to_string(),
                output_bytes: 0,
            },
        )];
        // Should not panic — falls back to content detection
        let lines = transcript_lines(&records, &ActivityFilter::default());
        assert!(
            lines.iter().any(|l| format!("{l}").contains("read_file")),
            "should have a read_file tool result line"
        );
    }

    #[test]
    fn transcript_lines_pairs_call_and_result() {
        let records = vec![
            rec(
                0,
                0,
                SessionEvent::Parsed {
                    tool_call: rexymcp_executor::parser::ToolCall {
                        name: "read_file".into(),
                        arguments: serde_json::json!({"path": "foo.rs"}),
                        origin: rexymcp_executor::parser::Origin::Native,
                    },
                },
            ),
            rec(
                1,
                0,
                SessionEvent::ToolResult {
                    name: "read_file".into(),
                    succeeded: true,
                    output_preview: "fn x(){}".into(),
                    output_bytes: 0,
                },
            ),
        ];
        let lines = transcript_lines(&records, &ActivityFilter::default());
        let rendered: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();

        // The call header should contain the call name
        let call_header = rendered
            .iter()
            .find(|l| l.contains("→ call read_file"))
            .expect("should have a call header line");
        assert!(
            !call_header.is_empty(),
            "call header should not be empty: {call_header}"
        );

        // The result header should contain the paired connector and not the tool name
        let result_header = rendered
            .iter()
            .find(|l| l.contains("╰ [ok]"))
            .expect("should have a paired result header line");
        assert!(
            !result_header.contains("tool read_file"),
            "result header should not contain 'tool read_file': {result_header}"
        );
    }

    #[test]
    fn transcript_lines_orphan_result_is_standalone() {
        let records = vec![rec(
            0,
            0,
            SessionEvent::ToolResult {
                name: "read_file".into(),
                succeeded: true,
                output_preview: "content".into(),
                output_bytes: 0,
            },
        )];
        let lines = transcript_lines(&records, &ActivityFilter::default());
        let rendered: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        let header = &rendered[0];
        assert!(
            header.contains("tool read_file [ok]"),
            "orphan result should render standalone: {header}"
        );
        assert!(
            !header.contains("╰"),
            "orphan result should not contain paired connector: {header}"
        );
    }

    #[test]
    fn transcript_lines_pairs_only_matching_name() {
        let records = vec![
            rec(
                0,
                0,
                SessionEvent::Parsed {
                    tool_call: rexymcp_executor::parser::ToolCall {
                        name: "read_file".into(),
                        arguments: serde_json::json!({"path": "foo.rs"}),
                        origin: rexymcp_executor::parser::Origin::Native,
                    },
                },
            ),
            rec(
                1,
                0,
                SessionEvent::ToolResult {
                    name: "bash".into(),
                    succeeded: false,
                    output_preview: "error".into(),
                    output_bytes: 0,
                },
            ),
        ];
        let lines = transcript_lines(&records, &ActivityFilter::default());
        let rendered: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
        // The bash result should be standalone (names don't match)
        let result_header = rendered
            .iter()
            .find(|l| l.contains("tool bash [FAIL]"))
            .expect("should have a standalone bash result header line");
        assert!(
            !result_header.contains("╰"),
            "mismatched result should not contain paired connector: {result_header}"
        );
    }

    #[test]
    fn record_lines_tool_result_unpaired_unchanged() {
        let rec = rec(
            0,
            0,
            SessionEvent::ToolResult {
                name: "bash".into(),
                succeeded: false,
                output_preview: "error".into(),
                output_bytes: 0,
            },
        );
        let lines = record_lines(&rec);
        let header = format!("{}", lines[0]);
        assert!(
            header.contains("tool bash [FAIL]"),
            "unpaired result should render unchanged: {header}"
        );
    }
}
