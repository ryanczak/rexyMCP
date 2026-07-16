use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::governor::hard_fail::{HardFailSignal, ToolCallSnapshot};
use crate::governor::verifier::{Diagnostic, Severity};

/// Maximum files included in the working-set section — enough post-edit truth on
/// what the model just touched, small enough to stay token-efficient.
pub const MAX_WORKING_FILES: usize = 5;

/// Maximum characters per attempt summary line.
pub const MAX_ATTEMPT_CHARS: usize = 200;

/// One compressed 1–2 line description of a local-model attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AttemptSummary {
    pub one_line: String,
}

/// A file the model touched, with its post-edit content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkingFile {
    pub path: PathBuf,
    pub content: String,
}

/// What triggered the escalation — one of the two non-`Complete` statuses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum Blocker {
    HardFail(HardFailSignal),
    BudgetExceeded,
}

/// The escalation hand-off returned to Claude when a phase does not complete.
/// A fresh brief, not a transcript replay. Not redacted — Claude is the trusted
/// architect and needs the truth; redaction guards the on-disk session log only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Briefing {
    pub goal: String,
    pub acceptance_criteria: String,
    pub diagnostics: Vec<Diagnostic>,
    pub working_files: Vec<WorkingFile>,
    pub what_was_tried: Vec<AttemptSummary>,
    pub current_blocker: Blocker,
    pub budget_remaining: String,
}

impl Briefing {
    /// Render the canonical six-section markdown the architect reads.
    pub fn render(&self) -> String {
        let mut out = String::new();

        out.push_str("# Goal\n\n");
        push_block(&mut out, &self.goal);

        out.push_str("# Acceptance criteria\n\n");
        push_block(&mut out, &self.acceptance_criteria);

        out.push_str("# Current code state\n\n");
        out.push_str("## Diagnostics\n\n");
        if self.diagnostics.is_empty() {
            out.push_str("(none)\n\n");
        } else {
            for d in &self.diagnostics {
                out.push_str(&render_diagnostic(d));
                out.push('\n');
            }
            out.push('\n');
        }

        out.push_str("## Files in the working set\n\n");
        if self.working_files.is_empty() {
            out.push_str("(none)\n\n");
        } else {
            for file in &self.working_files {
                out.push_str(&format!("### {}\n\n```\n", file.path.display()));
                out.push_str(&file.content);
                if !file.content.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```\n\n");
            }
        }

        out.push_str("# What was tried\n\n");
        if self.what_was_tried.is_empty() {
            out.push_str("(no prior attempts)\n\n");
        } else {
            for (i, attempt) in self.what_was_tried.iter().enumerate() {
                out.push_str(&format!("{}. {}\n", i + 1, attempt.one_line));
            }
            out.push('\n');
        }

        out.push_str("# Current blocker\n\n");
        match &self.current_blocker {
            Blocker::HardFail(signal) => {
                out.push_str(&signal.describe());
                out.push('\n');
            }
            Blocker::BudgetExceeded => {
                out.push_str("The executor exhausted its turn/context budget before completing.\n");
            }
        }
        out.push('\n');

        out.push_str("# Budget remaining\n\n");
        push_block(&mut out, &self.budget_remaining);

        out
    }
}

/// Compress each recent tool call into a one-line attempt summary, truncated to
/// `MAX_ATTEMPT_CHARS`.
pub fn summarize_attempts(recent_tool_calls: &VecDeque<ToolCallSnapshot>) -> Vec<AttemptSummary> {
    recent_tool_calls
        .iter()
        .map(|snapshot| {
            let outcome = if snapshot.succeeded {
                "succeeded"
            } else {
                "failed"
            };
            let one_line = format!(
                "Tried {} {}; {outcome}.",
                snapshot.tool,
                compact_args(&snapshot.arguments),
            );
            AttemptSummary {
                one_line: truncate_with_ellipsis(one_line, MAX_ATTEMPT_CHARS),
            }
        })
        .collect()
}

/// Collect post-edit content of files the model touched via any file-mutating
/// tool (`patch`/`write_file`/`patch_lines`/`delete_file`/`move_file`, per
/// `tools::mutates_files`). Newest-first, deduped, capped at `MAX_WORKING_FILES`;
/// unreadable paths (e.g. a `delete_file` target) are skipped rather than failing
/// the whole briefing.
pub fn collect_working_files(
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    project_root: &Path,
) -> Vec<WorkingFile> {
    let mut seen: Vec<PathBuf> = Vec::new();
    for snapshot in recent_tool_calls.iter().rev() {
        if !crate::tools::mutates_files(&snapshot.tool) {
            continue;
        }
        // Most write tools name the file in `path`; `move_file` writes the new
        // content at `to` (its `from` no longer exists post-edit).
        let path_key = if snapshot.tool == "move_file" {
            "to"
        } else {
            "path"
        };
        let path = match snapshot.arguments.get(path_key).and_then(|v| v.as_str()) {
            Some(p) => PathBuf::from(p),
            None => continue,
        };
        if !seen.contains(&path) {
            seen.push(path);
            if seen.len() == MAX_WORKING_FILES {
                break;
            }
        }
    }

    let mut files = Vec::new();
    for relative in seen {
        let absolute = if relative.is_absolute() {
            relative.clone()
        } else {
            project_root.join(&relative)
        };
        if let Ok(content) = fs::read_to_string(&absolute) {
            files.push(WorkingFile {
                path: relative,
                content,
            });
        }
    }
    files
}

fn push_block(out: &mut String, body: &str) {
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
}

fn render_diagnostic(d: &Diagnostic) -> String {
    let col = d.column.map(|c| format!(":{c}")).unwrap_or_default();
    let code = d
        .code
        .as_ref()
        .map(|c| format!("[{c}] "))
        .unwrap_or_default();
    let sev = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Note => "note",
        Severity::Help => "help",
    };
    format!(
        "- {}:{}{col} {sev}: {code}{}",
        d.path.display(),
        d.line,
        d.message,
    )
}

fn compact_args(args: &serde_json::Value) -> String {
    if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
        return format!("on {path}");
    }
    let s = serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string());
    truncate_with_ellipsis(s, 100)
}

fn truncate_with_ellipsis(s: String, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        let mut t: String = s.chars().take(max_chars - 1).collect();
        t.push('\u{2026}');
        t
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(tool: &str, args: serde_json::Value, succeeded: bool) -> ToolCallSnapshot {
        ToolCallSnapshot {
            tool: tool.to_string(),
            arguments: args,
            succeeded,
        }
    }

    fn diag(severity: Severity, message: &str) -> Diagnostic {
        Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 42,
            column: Some(10),
            severity,
            message: message.to_string(),
            code: Some("E0425".to_string()),
        }
    }

    fn sample_briefing(blocker: Blocker) -> Briefing {
        Briefing {
            goal: "fix the thing".to_string(),
            acceptance_criteria: "tests pass".to_string(),
            diagnostics: vec![diag(Severity::Error, "cannot find `foo`")],
            working_files: vec![],
            what_was_tried: vec![],
            current_blocker: blocker,
            budget_remaining: "2 of 12 turns remaining".to_string(),
        }
    }

    #[test]
    fn render_emits_six_section_headers_in_order() {
        let rendered = sample_briefing(Blocker::BudgetExceeded).render();
        let headers = [
            "# Goal",
            "# Acceptance criteria",
            "# Current code state",
            "# What was tried",
            "# Current blocker",
            "# Budget remaining",
        ];
        let mut last = 0;
        for h in headers {
            let pos = rendered[last..]
                .find(h)
                .unwrap_or_else(|| panic!("missing header {h} after position {last}"));
            last += pos + h.len();
        }
    }

    #[test]
    fn render_code_state_has_diagnostics_and_files_subheaders() {
        let rendered = sample_briefing(Blocker::BudgetExceeded).render();
        assert!(rendered.contains("## Diagnostics"));
        assert!(rendered.contains("## Files in the working set"));
    }

    #[test]
    fn render_omits_todo_state() {
        let rendered = sample_briefing(Blocker::BudgetExceeded).render();
        assert!(
            !rendered.contains("TODO"),
            "briefing must not render a TODO section"
        );
    }

    #[test]
    fn render_hard_fail_blocker_uses_signal_describe() {
        let signal = HardFailSignal::IdenticalToolCallRepetition {
            tool: "patch".to_string(),
            consecutive_count: 3,
        };
        let expected = signal.describe();
        let rendered = sample_briefing(Blocker::HardFail(signal)).render();
        assert!(rendered.contains(&expected));
    }

    #[test]
    fn render_budget_exceeded_blocker_states_exhaustion() {
        let rendered = sample_briefing(Blocker::BudgetExceeded).render();
        assert!(rendered.contains("exhausted its turn/context budget"));
    }

    #[test]
    fn render_budget_remaining_echoes_the_line() {
        let rendered = sample_briefing(Blocker::BudgetExceeded).render();
        assert!(rendered.contains("2 of 12 turns remaining"));
    }

    #[test]
    fn render_diagnostic_includes_path_line_severity_message() {
        let rendered = sample_briefing(Blocker::BudgetExceeded).render();
        assert!(rendered.contains("src/lib.rs"));
        assert!(rendered.contains(":42"));
        assert!(rendered.contains("error"));
        assert!(rendered.contains("cannot find `foo`"));
    }

    #[test]
    fn summarize_attempts_marks_succeeded_and_failed() {
        let mut recent = VecDeque::new();
        recent.push_back(snap(
            "patch",
            serde_json::json!({"path": "src/lib.rs"}),
            true,
        ));
        recent.push_back(snap(
            "write_file",
            serde_json::json!({"path": "src/main.rs"}),
            false,
        ));
        let summaries = summarize_attempts(&recent);
        assert_eq!(summaries.len(), 2);
        assert!(summaries[0].one_line.contains("patch"));
        assert!(summaries[0].one_line.contains("succeeded"));
        assert!(summaries[1].one_line.contains("write_file"));
        assert!(summaries[1].one_line.contains("failed"));
    }

    #[test]
    fn summarize_attempts_truncates_long_summaries() {
        let mut recent = VecDeque::new();
        recent.push_back(snap(
            "patch",
            serde_json::json!({"path": "x".repeat(300)}),
            true,
        ));
        let summaries = summarize_attempts(&recent);
        assert!(summaries[0].one_line.chars().count() <= MAX_ATTEMPT_CHARS);
        assert!(summaries[0].one_line.ends_with('\u{2026}'));
    }

    #[test]
    fn collect_working_files_caps_at_five() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut recent = VecDeque::new();
        for i in 0..7 {
            let p = dir.path().join(format!("file_{i}.rs"));
            std::fs::write(&p, format!("content {i}")).unwrap();
            recent.push_back(snap(
                "patch",
                serde_json::json!({"path": p.to_str().unwrap()}),
                true,
            ));
        }
        let files = collect_working_files(&recent, dir.path());
        assert_eq!(files.len(), MAX_WORKING_FILES);
    }

    #[test]
    fn collect_working_files_dedupes_repeated_paths() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("single.rs");
        std::fs::write(&p, "content").unwrap();
        let mut recent = VecDeque::new();
        for _ in 0..3 {
            recent.push_back(snap(
                "patch",
                serde_json::json!({"path": p.to_str().unwrap()}),
                true,
            ));
        }
        let files = collect_working_files(&recent, dir.path());
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn collect_working_files_skips_unreadable() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut recent = VecDeque::new();
        recent.push_back(snap(
            "patch",
            serde_json::json!({"path": "/nonexistent/path.rs"}),
            true,
        ));
        let files = collect_working_files(&recent, dir.path());
        assert!(files.is_empty());
    }

    #[test]
    fn collect_working_files_includes_patch_lines_edits() {
        // Regression for issue #2: `patch_lines` mutates the file and must be
        // reported as a working file, not omitted from the briefing.
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("edited.rs");
        std::fs::write(&p, "post-edit content").unwrap();
        let mut recent = VecDeque::new();
        recent.push_back(snap(
            "patch_lines",
            serde_json::json!({"path": p.to_str().unwrap()}),
            true,
        ));
        let files = collect_working_files(&recent, dir.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "post-edit content");
    }

    #[test]
    fn collect_working_files_resolves_move_file_via_to_key() {
        // `move_file` names the surviving file in `to`, not `path`; the collector
        // must read the destination content.
        let dir = tempfile::TempDir::new().unwrap();
        let to = dir.path().join("moved.rs");
        std::fs::write(&to, "moved content").unwrap();
        let mut recent = VecDeque::new();
        recent.push_back(snap(
            "move_file",
            serde_json::json!({"from": "gone.rs", "to": to.to_str().unwrap()}),
            true,
        ));
        let files = collect_working_files(&recent, dir.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "moved content");
    }
}
