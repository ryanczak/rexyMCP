//! The final-command-set seam. The loop depends on this trait, not on
//! `tokio::process` directly, so tests inject a deterministic mock instead of
//! spawning real subprocesses (`cargo build`, `npm test`, …).

use std::path::Path;

use async_trait::async_trait;

/// Outcome of a final-command-set run: the captured output (for
/// `PhaseResult.command_outputs`) and whether it exited successfully (for the
/// `PhaseRun` gate booleans).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub output: String,
    pub success: bool,
}

/// Runs one shell command in a working directory and reports its combined
/// stdout+stderr plus exit success. Capturing the result is the whole job — a
/// command that fails to spawn or exits non-zero yields `success: false`, never an
/// error to the loop.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, command: &str, cwd: &Path) -> CommandResult;
}

/// Production runner: `sh -c <command>` via `tokio::process`, stdout then stderr.
pub struct RealCommandRunner;

#[async_trait]
impl CommandRunner for RealCommandRunner {
    async fn run(&self, command: &str, cwd: &Path) -> CommandResult {
        match tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(cwd)
            .output()
            .await
        {
            Ok(out) => {
                let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&out.stderr);
                if !stderr.is_empty() {
                    combined.push_str(&stderr);
                }
                CommandResult {
                    output: combined,
                    success: out.status.success(),
                }
            }
            Err(e) => CommandResult {
                output: format!("failed to run `{command}`: {e}"),
                success: false,
            },
        }
    }
}

use crate::config::CommandConfig;
use crate::phase::CommandOutputs;
use crate::store::telemetry::Gates;

use crate::store::sessions::event::TaskState;
use std::collections::HashMap;

use super::progress::{EmitCtx, emit_progress};

/// Tail cap on each captured final-command-set output.
pub(super) const MAX_COMMAND_TAIL_CHARS: usize = 4_000;

pub(super) async fn run_command_set(
    runner: &dyn CommandRunner,
    commands: &CommandConfig,
    cwd: &Path,
    ctx: &EmitCtx<'_>,
) -> (CommandOutputs, Gates) {
    if commands.format.is_some() {
        emit_progress(ctx, "command:fmt".to_string());
    }
    let (format, fmt_ok) = run_one(runner, commands.format.as_deref(), cwd).await;
    if commands.build.is_some() {
        emit_progress(ctx, "command:build".to_string());
    }
    let (build, build_ok) = run_one(runner, commands.build.as_deref(), cwd).await;
    if commands.lint.is_some() {
        emit_progress(ctx, "command:lint".to_string());
    }
    let (lint, lint_ok) = run_one(runner, commands.lint.as_deref(), cwd).await;
    if commands.test.is_some() {
        emit_progress(ctx, "command:test".to_string());
    }
    let (test, test_ok) = run_one(runner, commands.test.as_deref(), cwd).await;
    (
        CommandOutputs {
            format,
            build,
            lint,
            test,
        },
        Gates {
            fmt: fmt_ok,
            build: build_ok,
            lint: lint_ok,
            test: test_ok,
        },
    )
}

pub(super) fn gate_failure_feedback(gates: &Gates, outputs: &CommandOutputs) -> Option<String> {
    let mut sections: Vec<String> = Vec::new();
    if gates.fmt == Some(false) {
        sections.push(format!(
            "FORMAT failed:\n{}",
            outputs.format.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if gates.build == Some(false) {
        sections.push(format!(
            "BUILD failed:\n{}",
            outputs.build.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if gates.lint == Some(false) {
        sections.push(format!(
            "LINT failed:\n{}",
            outputs.lint.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if gates.test == Some(false) {
        sections.push(format!(
            "TEST failed:\n{}",
            outputs.test.as_deref().unwrap_or("(no output captured)")
        ));
    }
    if sections.is_empty() {
        return None;
    }
    Some(format!(
        "Pre-completion gate check failed — the phase is not done yet. \
         Fix the issues below, then re-emit your completion signal.\n\n{}",
        sections.join("\n\n")
    ))
}

/// Task-coverage gate symmetric with `gate_failure_feedback`. Returns `Some(msg)`
/// when `seeded` is non-empty and any task's current state is not `Done`; `None`
/// otherwise. The `states` map is kept in sync by the loop as `update_task` calls
/// land — an absent key means the task was never touched (treated as Pending).
pub(super) fn task_coverage_feedback(
    seeded: &[super::tasks::Task],
    states: &HashMap<String, TaskState>,
) -> Option<String> {
    if seeded.is_empty() {
        return None;
    }
    let incomplete: Vec<&super::tasks::Task> = seeded
        .iter()
        .filter(|t| states.get(&t.id) != Some(&TaskState::Done))
        .collect();
    if incomplete.is_empty() {
        return None;
    }
    let list = incomplete
        .iter()
        .map(|t| {
            let label = match states.get(&t.id) {
                Some(TaskState::Active) => "active",
                _ => "pending",
            };
            format!("  Task {} ({}): {}", t.id, t.title, label)
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "Pre-completion task check: the following spec tasks are not yet marked done:\n{}\n\n\
         Call update_task(id, state=\"done\") for each completed task, \
         then re-signal completion.",
        list
    ))
}

/// Bookkeeping gate: checks that the executor updated the phase doc's `**Status:**`
/// line and wrote at least one Update Log entry before declaring done. Re-reads the
/// phase doc from disk so in-session edits are visible. Returns `Some(msg)` when
/// either check fails; `None` when both pass or when the file cannot be read
/// (IO failures here are not a bookkeeping problem and should not block completion).
pub(super) fn bookkeeping_feedback(phase_doc_path: &std::path::Path) -> Option<String> {
    let content = match std::fs::read_to_string(phase_doc_path) {
        Ok(c) => c,
        Err(_) => return None,
    };

    let mut issues: Vec<String> = Vec::new();

    let status_still_open = content.lines().any(|line| {
        let l = line.trim();
        l.starts_with("**Status:**") && (l.contains("todo") || l.contains("in-progress"))
    });
    if status_still_open {
        issues.push(
            "The `**Status:**` line in the phase doc frontmatter still reads `todo` or \
             `in-progress`. Change it to `review` before signalling completion \
             (patch the phase doc's Status line)."
                .to_string(),
        );
    }

    let has_update_log_entry = content.contains("### Update");
    if !has_update_log_entry {
        issues.push(
            "The phase doc's `## Update Log` has no entries. Fill in the completion \
             entry using the `### Update — YYYY-MM-DD HH:MM (complete)` template from \
             WORKFLOW.md, including: Summary, Acceptance criteria, Commands output, \
             End-to-end verification, Files changed, New tests, Commits."
                .to_string(),
        );
    }

    if issues.is_empty() {
        return None;
    }

    Some(format!(
        "Pre-completion bookkeeping check failed — the phase is not done yet. \
         Fix the items below, update the phase doc, then re-signal completion.\n\n{}",
        issues
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{}. {}", i + 1, s))
            .collect::<Vec<_>>()
            .join("\n\n")
    ))
}

pub(super) async fn run_post_write_hooks(
    runner: &dyn CommandRunner,
    commands: &CommandConfig,
    cwd: &Path,
) {
    if let Some(cmd) = commands.lint_fix.as_deref() {
        let _ = runner.run(cmd, cwd).await;
    }
    if let Some(cmd) = commands.format.as_deref() {
        let _ = runner.run(cmd, cwd).await;
    }
}

async fn run_one(
    runner: &dyn CommandRunner,
    command: Option<&str>,
    cwd: &Path,
) -> (Option<String>, Option<bool>) {
    match command {
        Some(cmd) => {
            let CommandResult { output, success } = runner.run(cmd, cwd).await;
            (Some(tail(&output, MAX_COMMAND_TAIL_CHARS)), Some(success))
        }
        None => (None, None),
    }
}

fn tail(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count > max_chars {
        s.chars().skip(count - max_chars).collect()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tasks::Task;
    use crate::phase::CommandOutputs;
    use crate::store::sessions::event::TaskState;
    use crate::store::telemetry::Gates;
    use std::collections::HashMap;

    fn outputs_with(format: &str, build: &str, lint: &str, test: &str) -> CommandOutputs {
        CommandOutputs {
            format: Some(format.to_string()),
            build: Some(build.to_string()),
            lint: Some(lint.to_string()),
            test: Some(test.to_string()),
        }
    }

    #[test]
    fn gate_failure_feedback_returns_none_when_all_pass() {
        let gates = Gates {
            fmt: Some(true),
            build: Some(true),
            lint: Some(true),
            test: Some(true),
        };
        assert!(gate_failure_feedback(&gates, &outputs_with("ok", "ok", "ok", "ok")).is_none());
    }

    #[test]
    fn gate_failure_feedback_returns_none_when_no_commands_configured() {
        // Gates::default() is all None — unconfigured commands are not failures.
        assert!(gate_failure_feedback(&Gates::default(), &CommandOutputs::default()).is_none());
    }

    #[test]
    fn gate_failure_feedback_includes_failing_gates_and_omits_passing() {
        let gates = Gates {
            fmt: Some(false),
            build: Some(false),
            lint: Some(true),
            test: Some(false),
        };
        let outputs = outputs_with("fmt diff here", "build errors", "lint ok", "test failed");
        let msg = gate_failure_feedback(&gates, &outputs).expect("should be Some");
        assert!(msg.contains("FORMAT failed"), "missing FORMAT section");
        assert!(msg.contains("fmt diff here"), "missing FORMAT output");
        assert!(msg.contains("BUILD failed"), "missing BUILD section");
        assert!(msg.contains("build errors"), "missing BUILD output");
        assert!(!msg.contains("LINT"), "LINT should not appear (it passed)");
        assert!(msg.contains("TEST failed"), "missing TEST section");
    }

    fn task(id: &str, title: &str) -> Task {
        Task {
            id: id.to_string(),
            title: title.to_string(),
            state: TaskState::Pending,
        }
    }

    #[test]
    fn task_coverage_feedback_returns_none_when_no_tasks_seeded() {
        assert!(task_coverage_feedback(&[], &HashMap::new()).is_none());
    }

    #[test]
    fn task_coverage_feedback_returns_none_when_all_tasks_done() {
        let seeded = vec![task("1", "Foo"), task("2", "Bar")];
        let states = HashMap::from([
            ("1".to_string(), TaskState::Done),
            ("2".to_string(), TaskState::Done),
        ]);
        assert!(task_coverage_feedback(&seeded, &states).is_none());
    }

    #[test]
    fn task_coverage_feedback_lists_pending_task_by_id_and_title() {
        let seeded = vec![task("1", "Update the status header")];
        let states = HashMap::new(); // absent = pending
        let msg = task_coverage_feedback(&seeded, &states).expect("should be Some");
        assert!(
            msg.contains("Task 1 (Update the status header): pending"),
            "expected pending task listing, got: {msg}"
        );
    }

    #[test]
    fn task_coverage_feedback_labels_active_task() {
        let seeded = vec![task("3", "Wire the config")];
        let states = HashMap::from([("3".to_string(), TaskState::Active)]);
        let msg = task_coverage_feedback(&seeded, &states).expect("should be Some");
        assert!(
            msg.contains("Task 3 (Wire the config): active"),
            "expected active label, got: {msg}"
        );
    }

    #[test]
    fn task_coverage_feedback_omits_done_tasks_from_list() {
        let seeded = vec![task("1", "Done task"), task("2", "Pending task")];
        let states = HashMap::from([("1".to_string(), TaskState::Done)]);
        let msg = task_coverage_feedback(&seeded, &states).expect("should be Some");
        assert!(
            !msg.contains("Done task"),
            "done task must not appear: {msg}"
        );
        assert!(
            msg.contains("Pending task"),
            "pending task must appear: {msg}"
        );
    }

    fn write_phase_doc(dir: &tempfile::TempDir, content: &str) -> std::path::PathBuf {
        let path = dir.path().join("phase-01-test.md");
        std::fs::write(&path, content).unwrap();
        path
    }

    const GOOD_DOC: &str = "\
# Phase 01: Test\n\
\n\
**Status:** review\n\
\n\
## Update Log\n\
\n\
<!-- entries appended below this line -->\n\
\n\
### Update — 2026-06-17 10:00 (complete)\n\
\n\
**Summary:** done.\n";

    #[test]
    fn bookkeeping_feedback_returns_none_when_status_is_review_and_log_has_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_phase_doc(&dir, GOOD_DOC);
        assert!(bookkeeping_feedback(&path).is_none());
    }

    #[test]
    fn bookkeeping_feedback_flags_todo_status() {
        let dir = tempfile::tempdir().unwrap();
        let doc = GOOD_DOC.replace("**Status:** review", "**Status:** todo");
        let path = write_phase_doc(&dir, &doc);
        let msg = bookkeeping_feedback(&path).expect("should be Some");
        assert!(msg.contains("Status"), "expected Status mention: {msg}");
        assert!(msg.contains("review"), "expected review instruction: {msg}");
    }

    #[test]
    fn bookkeeping_feedback_flags_in_progress_status() {
        let dir = tempfile::tempdir().unwrap();
        let doc = GOOD_DOC.replace("**Status:** review", "**Status:** in-progress");
        let path = write_phase_doc(&dir, &doc);
        let msg = bookkeeping_feedback(&path).expect("should be Some");
        assert!(msg.contains("Status"), "expected Status mention: {msg}");
    }

    #[test]
    fn bookkeeping_feedback_flags_missing_update_log_entry() {
        let dir = tempfile::tempdir().unwrap();
        let doc = "\
# Phase 01: Test\n\
\n\
**Status:** review\n\
\n\
## Update Log\n\
\n\
<!-- entries appended below this line -->\n";
        let path = write_phase_doc(&dir, doc);
        let msg = bookkeeping_feedback(&path).expect("should be Some");
        assert!(msg.contains("Update Log"), "expected Update Log mention: {msg}");
    }

    #[test]
    fn bookkeeping_feedback_flags_both_issues_together() {
        let dir = tempfile::tempdir().unwrap();
        let doc = "\
# Phase 01: Test\n\
\n\
**Status:** todo\n\
\n\
## Update Log\n\
\n\
<!-- entries appended below this line -->\n";
        let path = write_phase_doc(&dir, doc);
        let msg = bookkeeping_feedback(&path).expect("should be Some");
        assert!(msg.contains("Status"), "expected Status mention: {msg}");
        assert!(msg.contains("Update Log"), "expected Update Log mention: {msg}");
    }

    #[test]
    fn bookkeeping_feedback_returns_none_on_unreadable_file() {
        let path = std::path::Path::new("/nonexistent/phase-01-test.md");
        assert!(bookkeeping_feedback(path).is_none());
    }
}
