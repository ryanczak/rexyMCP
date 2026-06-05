use std::sync::OnceLock;

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::{SyntaxReference, SyntaxSet};

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

pub(crate) fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

/// Max content lines shown per record before collapsing the rest.
pub(crate) const TRANSCRIPT_CONTENT_MAX_LINES: usize = 20;

/// Detect a syntax definition from content alone (no filename available).
/// Returns `None` when no language can be confidently identified.
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

/// True when `content` looks like unified diff output.
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

/// Render unified diff content with line-level background colors.
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
pub(crate) fn highlighted_body_lines(content: &str) -> Vec<Line<'static>> {
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
pub(crate) fn plain_body_lines(content: &str, color: Color) -> Vec<Line<'static>> {
    body_lines(content)
        .into_iter()
        .map(|l| Line::from(Span::styled(l, Style::new().fg(color))))
        .collect()
}

/// Split `body` on newlines into indented display lines.
pub(crate) fn body_lines(body: &str) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
