use std::path::{Path, PathBuf};

use rexymcp_executor::agent::command::CommandRunner;
use rexymcp_executor::agent::prompt::format_utc_datetime;
use rexymcp_executor::phase::{CommandOutputs, FileChange, PhaseResult, PhaseStatus};

/// Inputs for the server-authored completion finalize step.
pub struct FinalizeInput<'a> {
    pub phase_doc_path: &'a Path,
    pub repo_root: &'a Path,
    pub result: &'a PhaseResult,
    pub now_ms: u64,
    pub runner: &'a dyn CommandRunner,
    /// The resolved dispatch model (same value as `PhaseRun.model`). Written as
    /// the authoritative `**Executor:**` line — never the model's self-report.
    pub model: &'a str,
}

/// Server-authored bookkeeping for a completed phase. No-op (returns
/// `Ok(false)`) unless the result is `Complete` and the phase doc's
/// `**Status:**` line is a pre-review status (`todo` or `in-progress`).
/// On the active path: flip Status to `review`, append a baseline
/// completion entry, flip the sibling milestone README's phase-table
/// row, commit the doc changes as a separate `docs:` commit, and return
/// `Ok(true)`.
pub async fn finalize_complete(inp: &FinalizeInput<'_>) -> std::io::Result<bool> {
    if inp.result.status != PhaseStatus::Complete {
        return Ok(false);
    }
    let doc = std::fs::read_to_string(inp.phase_doc_path)?;
    if !status_is_pre_review(&doc) {
        return Ok(false);
    }

    let code_sha = git_head(inp.runner, inp.repo_root).await;
    let entry = baseline_entry(inp.result, inp.now_ms, &code_sha, inp.model);
    let flipped = flip_status_to_review(&doc);
    let new_doc = append_entry(&flipped, &entry);
    std::fs::write(inp.phase_doc_path, new_doc)?;

    let mut staged: Vec<PathBuf> = vec![inp.phase_doc_path.to_path_buf()];
    if let Some(readme) = inp.phase_doc_path.parent().map(|p| p.join("README.md"))
        && let Ok(readme_doc) = std::fs::read_to_string(&readme)
        && let Some(stem) = inp.phase_doc_path.file_name().and_then(|s| s.to_str())
        && let Some(updated) = flip_readme_row(&readme_doc, stem)
    {
        std::fs::write(&readme, updated)?;
        staged.push(readme);
    }

    git_commit_docs(inp.runner, inp.repo_root, &staged).await;
    Ok(true)
}

/// True iff `trimmed` is a pre-review status line (`todo` or `in-progress`),
/// with or without a trailing note. The space before the note is the delimiter,
/// so `**Status:** todoish` and `**Status:** in-progressish` do NOT match.
fn is_pre_review_status(trimmed: &str) -> bool {
    matches!(trimmed, "**Status:** todo" | "**Status:** in-progress")
        || (trimmed.starts_with("**Status:** todo ")
            || trimmed.starts_with("**Status:** in-progress "))
}

/// True iff some line, trimmed, is a pre-review status line (exact or with a
/// trailing note).
fn status_is_pre_review(doc: &str) -> bool {
    doc.lines().any(|line| is_pre_review_status(line.trim()))
}

/// Replace the single frontmatter line `**Status:** todo` or
/// `**Status:** in-progress` with `**Status:** review`, leaving everything
/// else byte-identical. If the status line carries a bounce note
/// `(bounced — …)`, the note is dropped — the canonical review line has no
/// note. Replaces only the first such line.
fn flip_status_to_review(doc: &str) -> String {
    let mut replaced = false;
    let mut result = String::with_capacity(doc.len());
    let mut first = true;
    for line in doc.lines() {
        if !first {
            result.push('\n');
        }
        first = false;
        if !replaced && is_pre_review_status(line.trim()) {
            replaced = true;
            let leading = line.len() - line.trim_start().len();
            result.push_str(&" ".repeat(leading));
            result.push_str("**Status:** review");
        } else {
            result.push_str(line);
        }
    }
    // Preserve trailing newline if present
    if doc.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Build the baseline completion entry.
fn baseline_entry(result: &PhaseResult, now_ms: u64, code_sha: &str, model: &str) -> String {
    let summary = if result.completion_summary.trim().is_empty() {
        "(no summary provided by executor)".to_string()
    } else {
        result.completion_summary.trim().to_string()
    };

    let gates = gate_line(&result.command_outputs);
    let command_tails = command_output_tails(&result.command_outputs);
    let files = files_changed_list(&result.files_changed);
    let stamp = format_utc_datetime(now_ms);

    format!(
        "### Update — {stamp} (complete, server-authored)\n\n\
         **Summary:** {summary}\n\n\
         **Executor:** {model}\n\n\
         **Gates:** {gates}\n\n\
         **Command output tails:**\n\n\
         ```\n{command_tails}\n```\n\n\
         **Files changed:**\n\n{files}\n\n\
         **Commit:** {code_sha}\n\n\
         **Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).\n"
    )
}

fn gate_line(outputs: &CommandOutputs) -> String {
    let fmt = gate_status(outputs.format.as_ref());
    let build = gate_status(outputs.build.as_ref());
    let lint = gate_status(outputs.lint.as_ref());
    let test = gate_status(outputs.test.as_ref());
    format!("format={fmt}, build={build}, lint={lint}, test={test}")
}

fn gate_status(tail: Option<&String>) -> &'static str {
    match tail {
        Some(_) => "run",
        None => "skipped",
    }
}

fn command_output_tails(outputs: &CommandOutputs) -> String {
    let mut sections: Vec<String> = Vec::new();
    if let Some(ref tail) = outputs.format {
        sections.push(format!("FORMAT\n{tail}"));
    }
    if let Some(ref tail) = outputs.build {
        sections.push(format!("BUILD\n{tail}"));
    }
    if let Some(ref tail) = outputs.lint {
        sections.push(format!("LINT\n{tail}"));
    }
    if let Some(ref tail) = outputs.test {
        sections.push(format!("TEST\n{tail}"));
    }
    if sections.is_empty() {
        "(no command output captured)".to_string()
    } else {
        sections.join("\n\n")
    }
}

fn files_changed_list(files: &[FileChange]) -> String {
    if files.is_empty() {
        "(none)".to_string()
    } else {
        files
            .iter()
            .map(|f| format!("- `{}` — {}", f.path.display(), f.change_summary))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Return `doc` with the entry appended at end of file, separated by a blank
/// line.
fn append_entry(doc: &str, entry: &str) -> String {
    format!("{}\n\n{}\n", doc.trim_end(), entry.trim_end())
}

/// Find the one table row that contains `phase_doc_filename` whose last table
/// cell (text between the final two `|`, trimmed) starts with `todo` or
/// `in-progress`. Replace that last cell with ` review ` (dropping any bounce
/// note). Return `None` if no such row.
pub fn flip_readme_row(readme_doc: &str, phase_doc_filename: &str) -> Option<String> {
    let mut found = false;
    let had_trailing_newline = readme_doc.ends_with('\n');
    let lines: Vec<String> = readme_doc
        .lines()
        .map(|line| {
            if !found && line.contains(phase_doc_filename) {
                // Find the last two `|` delimiters to isolate the last cell
                if let Some(last_pipe) = line.rfind('|') {
                    let before_last = &line[..last_pipe];
                    if let Some(second_last_pipe) = before_last.rfind('|') {
                        let last_cell_raw = &before_last[second_last_pipe + 1..last_pipe];
                        let last_cell = last_cell_raw.trim();
                        if last_cell.starts_with("todo") || last_cell.starts_with("in-progress") {
                            found = true;
                            let original_cell_width = last_cell_raw.chars().count();
                            let replacement = " review ";
                            let new_cell = if replacement.chars().count() >= original_cell_width {
                                replacement.to_string()
                            } else {
                                format!("{:width$}", replacement, width = original_cell_width)
                            };
                            format!(
                                "{}{}|{}",
                                &line[..second_last_pipe + 1],
                                new_cell,
                                &line[last_pipe + 1..]
                            )
                        } else {
                            line.to_string()
                        }
                    } else {
                        line.to_string()
                    }
                } else {
                    line.to_string()
                }
            } else {
                line.to_string()
            }
        })
        .collect();

    if found {
        let mut result = lines.join("\n");
        if had_trailing_newline {
            result.push('\n');
        }
        Some(result)
    } else {
        None
    }
}

/// Run `git rev-parse HEAD` via the runner in `repo_root`; return the trimmed
/// stdout on success, or `"unknown"` on failure.
async fn git_head(runner: &dyn CommandRunner, repo_root: &Path) -> String {
    match runner.run("git rev-parse HEAD", repo_root).await {
        cr if cr.success => cr.output.trim().to_string(),
        _ => "unknown".to_string(),
    }
}

/// Stage exactly `paths` and commit with a `docs:` message.
/// Ignores failures (best-effort).
async fn git_commit_docs(runner: &dyn CommandRunner, repo_root: &Path, paths: &[PathBuf]) {
    let path_args: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    let add_cmd = format!("git add -- {}", path_args.join(" "));
    let _ = runner.run(&add_cmd, repo_root).await;
    let _ = runner
        .run(
            "git commit -m \"docs: server-authored completion bookkeeping\"",
            repo_root,
        )
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rexymcp_executor::agent::command::CommandResult;
    use tempfile::TempDir;

    // --- Recording mock CommandRunner ---

    /// Captures every command it is asked to run. Returns canned stdout for
    /// `git rev-parse HEAD`.
    #[derive(Default)]
    pub struct RecordingRunner {
        pub commands: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingRunner {
        pub fn new() -> Self {
            Self {
                commands: std::sync::Mutex::new(Vec::new()),
            }
        }

        pub fn get_commands(&self) -> Vec<String> {
            self.commands.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CommandRunner for RecordingRunner {
        async fn run(&self, command: &str, _cwd: &Path) -> CommandResult {
            self.commands.lock().unwrap().push(command.to_string());
            if command == "git rev-parse HEAD" {
                CommandResult {
                    output: "abcdef1234567890\n".to_string(),
                    success: true,
                }
            } else {
                CommandResult {
                    output: String::new(),
                    success: true,
                }
            }
        }
    }

    // --- pre-review status ---

    #[test]
    fn pre_review_predicate_accepts_todo() {
        assert!(is_pre_review_status("**Status:** todo"));
        assert!(is_pre_review_status("**Status:** todo with a note"));
    }

    #[test]
    fn pre_review_predicate_accepts_in_progress() {
        assert!(is_pre_review_status("**Status:** in-progress"));
        assert!(is_pre_review_status(
            "**Status:** in-progress (bounced — see bugs/bug-04-1.md)"
        ));
    }

    #[test]
    fn pre_review_predicate_rejects_review_and_done() {
        assert!(!is_pre_review_status("**Status:** review"));
        assert!(!is_pre_review_status("**Status:** done"));
        assert!(!is_pre_review_status("**Status:** review (bounced — …)"));
        assert!(!is_pre_review_status("**Status:** done (bounced — …)"));
    }

    #[test]
    fn pre_review_predicate_rejects_lookalikes() {
        assert!(!is_pre_review_status("**Status:** todoish"));
        assert!(!is_pre_review_status("**Status:** in-progressish"));
        assert!(!is_pre_review_status("**Status:** todo-"));
        assert!(!is_pre_review_status("**Status:** in-progress-"));
    }

    #[test]
    fn pre_review_predicate_ignores_prose() {
        let doc = "This work is in-progress as of today.\n\n**Status:** review\n";
        assert!(!status_is_pre_review(doc));
    }

    #[test]
    fn pre_review_predicate_matches_todo_in_doc() {
        let doc = "# Phase 01\n\n**Status:** todo\n\n## Goal\n\nDo it.\n";
        assert!(status_is_pre_review(doc));
    }

    #[test]
    fn pre_review_predicate_matches_in_progress_in_doc() {
        let doc = "# Phase 01\n\n**Status:** in-progress\n\n## Goal\n\nDo it.\n";
        assert!(status_is_pre_review(doc));
    }

    // --- flip_status_to_review ---

    #[test]
    fn flip_status_to_review_flips_todo() {
        let doc = "# Phase 01\n\n**Status:** todo\n\n## Goal\n\nDo it.\n";
        let result = flip_status_to_review(doc);
        assert!(result.contains("**Status:** review"));
        assert!(!result.contains("**Status:** todo"));
        assert!(result.contains("## Goal"));
    }

    #[test]
    fn flip_status_to_review_changes_only_status_line() {
        let doc = "# Phase 01\n\n**Status:** in-progress\n\n## Goal\n\nDo it.\n";
        let result = flip_status_to_review(doc);
        assert!(result.contains("**Status:** review"));
        assert!(!result.contains("**Status:** in-progress"));
        assert!(result.contains("## Goal"));
    }

    #[test]
    fn flip_status_to_review_leaves_other_lines_byte_identical() {
        let doc = "# Phase 01\n\n**Status:** in-progress\n\n## Goal\n\nDo it.\n";
        let expected = "# Phase 01\n\n**Status:** review\n\n## Goal\n\nDo it.\n";
        assert_eq!(flip_status_to_review(doc), expected);
    }

    #[test]
    fn flip_status_to_review_drops_bounce_note() {
        let doc = "# Phase 04\n\n**Status:** in-progress (bounced — see bugs/bug-04-1.md)\n\n## Goal\n\nDo it.\n";
        let expected = "# Phase 04\n\n**Status:** review\n\n## Goal\n\nDo it.\n";
        assert_eq!(flip_status_to_review(doc), expected);
    }

    // --- flip_readme_row ---

    #[test]
    fn flip_readme_row_flips_matching_row_only() {
        let readme = "| 03a | Server-authored finalize ([phase-03a-server-authored-finalize.md](phase-03a-server-authored-finalize.md)) | in-progress |\n| 03b | Retire executor gate ([phase-03b-retire-gate.md](phase-03b-retire-gate.md)) | in-progress |\n";
        let result = flip_readme_row(readme, "phase-03a-server-authored-finalize.md");
        let updated = result.expect("should have found and flipped the row");
        let lines: Vec<&str> = updated.lines().collect();
        assert_eq!(
            lines[0],
            "| 03a | Server-authored finalize ([phase-03a-server-authored-finalize.md](phase-03a-server-authored-finalize.md)) | review      |"
        );
        assert!(!lines[0].contains("||"));
        assert!(
            !lines[0].contains("in-progress"),
            "status cell should not contain in-progress"
        );
        // The sibling row must still be in-progress
        assert!(
            lines[1].contains("| in-progress |"),
            "03b row should still be in-progress"
        );
    }

    #[test]
    fn flip_readme_row_flips_todo_cell() {
        let readme = "| 01 | Phase ([phase-01.md](phase-01.md)) | todo |\n";
        let result = flip_readme_row(readme, "phase-01.md");
        assert!(result.is_some());
        let new = result.unwrap();
        assert_eq!(
            new.lines().next().unwrap(),
            "| 01 | Phase ([phase-01.md](phase-01.md)) | review |"
        );
        assert!(!new.contains("||"));
        assert!(!new.contains("| todo |"));
    }

    #[test]
    fn flip_readme_row_returns_none_when_already_review() {
        let readme = "| 03a | Phase ([phase-03a.md](phase-03a.md)) | review |\n";
        let result = flip_readme_row(readme, "phase-03a.md");
        assert!(result.is_none());
    }

    /// The trimmed contents of a table row's last cell — the text between the
    /// final two `|`. Lets a test assert the cell's value without pinning the
    /// column's padding width, which legitimately varies per table.
    fn last_cell_of(line: &str) -> &str {
        line.rsplit('|').nth(1).unwrap_or("").trim()
    }

    #[test]
    fn flip_readme_row_returns_none_when_row_absent() {
        let readme = "| 01 | Phase ([phase-01.md](phase-01.md)) | in-progress |\n";
        let result = flip_readme_row(readme, "phase-99.md");
        assert!(result.is_none());
    }

    #[test]
    fn flip_readme_row_flips_bounced_row() {
        let readme = "| 04 | Fix ([phase-04.md](phase-04.md)) | in-progress (bounced, bug-04-1) |\n| 05 | Next ([phase-05.md](phase-05.md)) | review |\n";
        let result = flip_readme_row(readme, "phase-04.md");
        let updated = result.expect("should have found and flipped the bounced row");
        // The bounced row should now be review
        assert!(updated.contains("phase-04.md"));
        let lines: Vec<&str> = updated.lines().collect();
        // Assert the cell's structure, not just the presence of the word: a bare
        // `contains("review")` also passes against a mangled row (M32's lesson).
        assert_eq!(
            last_cell_of(lines[0]),
            "review",
            "bounced 04 row's status cell should be exactly review: {lines:?}"
        );
        assert!(!lines[0].contains("||"), "no doubled pipe in bounced row");
        // The sibling review row must be untouched
        // The sibling review row must be untouched — byte-for-byte.
        assert_eq!(
            lines[1], "| 05 | Next ([phase-05.md](phase-05.md)) | review |",
            "sibling review row must be untouched"
        );
    }

    #[test]
    fn flip_readme_row_emits_single_trailing_pipe() {
        let readme = "| 02 | Structured output ([phase-02-structured-tool-output.md](phase-02-structured-tool-output.md)) | in-progress |\n";
        let result = flip_readme_row(readme, "phase-02-structured-tool-output.md");
        let updated = result.expect("row should flip");
        assert_eq!(
            updated.lines().next().unwrap(),
            "| 02 | Structured output ([phase-02-structured-tool-output.md](phase-02-structured-tool-output.md)) | review      |"
        );
        assert!(
            !updated.contains("||"),
            "no doubled pipe anywhere: {updated}"
        );
    }

    // --- finalize_noop tests ---

    #[tokio::test]
    async fn finalize_noop_when_status_already_review() {
        let dir = TempDir::new().unwrap();
        let doc_path = dir.path().join("phase-01-test.md");
        std::fs::write(
            &doc_path,
            "# Phase 01\n\n**Status:** review\n\n## Update Log\n",
        )
        .unwrap();

        let runner = RecordingRunner::new();
        let result = PhaseResult::complete(rexymcp_executor::phase::Artifacts {
            files_changed: vec![],
            diff: String::new(),
            command_outputs: CommandOutputs::default(),
            update_log: String::new(),
            log_path: None,
            completion_summary: String::new(),
        });

        let inp = FinalizeInput {
            phase_doc_path: &doc_path,
            repo_root: dir.path(),
            result: &result,
            now_ms: 1000,
            runner: &runner,
            model: "test-model",
        };

        let did_finalize = finalize_complete(&inp).await.expect("should not error");
        assert!(!did_finalize, "should return false for already-review doc");

        // Doc should be byte-identical
        let after = std::fs::read_to_string(&doc_path).unwrap();
        assert_eq!(after, "# Phase 01\n\n**Status:** review\n\n## Update Log\n");

        // No git commit should have been issued
        let cmds = runner.get_commands();
        assert!(
            cmds.iter().all(|c| !c.starts_with("git commit")),
            "no git commit should run for dormant doc: {:?}",
            cmds
        );
    }

    #[tokio::test]
    async fn finalize_noop_when_result_not_complete() {
        let dir = TempDir::new().unwrap();
        let doc_path = dir.path().join("phase-01-test.md");
        std::fs::write(
            &doc_path,
            "# Phase 01\n\n**Status:** in-progress\n\n## Update Log\n",
        )
        .unwrap();

        let runner = RecordingRunner::new();
        let result = PhaseResult::hard_fail(
            rexymcp_executor::phase::Briefing {
                goal: "g".to_string(),
                acceptance_criteria: "ac".to_string(),
                diagnostics: vec![],
                working_files: vec![],
                what_was_tried: vec![],
                current_blocker: rexymcp_executor::phase::Blocker::BudgetExceeded,
                budget_remaining: "0".to_string(),
            },
            rexymcp_executor::phase::Artifacts {
                files_changed: vec![],
                diff: String::new(),
                command_outputs: CommandOutputs::default(),
                update_log: String::new(),
                log_path: None,
                completion_summary: String::new(),
            },
        );

        let inp = FinalizeInput {
            phase_doc_path: &doc_path,
            repo_root: dir.path(),
            result: &result,
            now_ms: 1000,
            runner: &runner,
            model: "test-model",
        };

        let did_finalize = finalize_complete(&inp).await.expect("should not error");
        assert!(!did_finalize, "should return false for HardFail result");

        // Doc should be byte-identical
        let after = std::fs::read_to_string(&doc_path).unwrap();
        assert!(after.contains("**Status:** in-progress"));
    }

    // --- finalize_flips_status_and_appends_entry ---

    #[tokio::test]
    async fn finalize_flips_status_and_appends_entry() {
        let dir = TempDir::new().unwrap();
        let doc_path = dir.path().join("phase-03a-server-authored-finalize.md");
        std::fs::write(
            &doc_path,
            "# Phase 03a\n\n**Status:** in-progress\n\n## Update Log\n\n<!-- entries appended below this line -->\n",
        )
        .unwrap();

        let runner = RecordingRunner::new();

        let result = PhaseResult::complete(rexymcp_executor::phase::Artifacts {
            files_changed: vec![
                FileChange {
                    path: PathBuf::from("src/lib.rs"),
                    change_summary: "+5 -2".to_string(),
                },
                FileChange {
                    path: PathBuf::from("src/util.rs"),
                    change_summary: "+10 -0".to_string(),
                },
            ],
            diff: String::new(),
            command_outputs: CommandOutputs {
                format: Some("clean".to_string()),
                build: Some(
                    "Finished `dev` [unoptimized + debuginfo] target(s) in 0.50s".to_string(),
                ),
                lint: None,
                test: Some("running 5 tests\nok".to_string()),
            },
            update_log: String::new(),
            log_path: None,
            completion_summary: "Implemented server-authored finalize.".to_string(),
        });

        let inp = FinalizeInput {
            phase_doc_path: &doc_path,
            repo_root: dir.path(),
            result: &result,
            now_ms: 999999,
            runner: &runner,
            model: "test-model",
        };

        let did_finalize = finalize_complete(&inp).await.expect("should not error");
        assert!(did_finalize, "should return true for active finalize");

        let after = std::fs::read_to_string(&doc_path).unwrap();

        // Status flipped
        assert!(after.contains("**Status:** review"));
        assert!(!after.contains("**Status:** in-progress"));

        // Entry appended
        assert!(after.contains("(complete, server-authored)"));
        assert!(after.contains("### Update — 1970-01-01 00:16 (complete, server-authored)"));
        assert!(
            !after.contains("ts="),
            "heading must use the WORKFLOW date stamp, not epoch-ms"
        );
        assert!(after.contains("Implemented server-authored finalize."));
        assert!(after.contains("src/lib.rs"));
        assert!(after.contains("src/util.rs"));
        assert!(after.contains("+5 -2"));
        assert!(after.contains("+10 -0"));
        assert!(after.contains("abcdef1234567890"));
        assert!(after.contains("FORMAT"));
        assert!(after.contains("BUILD"));
        assert!(after.contains("TEST"));
        // lint was None → skipped
        assert!(after.contains("lint=skipped"));
        // format was Some → run
        assert!(after.contains("format=run"));
    }

    // --- finalize_updates_matching_readme_row_only ---

    #[tokio::test]
    async fn finalize_updates_matching_readme_row_only() {
        let dir = TempDir::new().unwrap();
        let doc_path = dir.path().join("phase-03a-server-authored-finalize.md");
        std::fs::write(
            &doc_path,
            "# Phase 03a\n\n**Status:** in-progress\n\n## Update Log\n",
        )
        .unwrap();

        let readme_path = dir.path().join("README.md");
        std::fs::write(
            &readme_path,
            "| 03a | Server-authored finalize ([phase-03a-server-authored-finalize.md](phase-03a-server-authored-finalize.md)) | in-progress |\n| 03b | Retire executor gate ([phase-03b-retire-gate.md](phase-03b-retire-gate.md)) | in-progress |\n",
        )
        .unwrap();

        let runner = RecordingRunner::new();

        let result = PhaseResult::complete(rexymcp_executor::phase::Artifacts {
            files_changed: vec![],
            diff: String::new(),
            command_outputs: CommandOutputs::default(),
            update_log: String::new(),
            log_path: None,
            completion_summary: String::new(),
        });

        let inp = FinalizeInput {
            phase_doc_path: &doc_path,
            repo_root: dir.path(),
            result: &result,
            now_ms: 500,
            runner: &runner,
            model: "test-model",
        };

        let did_finalize = finalize_complete(&inp).await.expect("should not error");
        assert!(did_finalize);

        let readme_after = std::fs::read_to_string(&readme_path).unwrap();
        let lines: Vec<&str> = readme_after.lines().collect();
        assert_eq!(
            last_cell_of(lines[0]),
            "review",
            "03a row's status cell should be exactly review"
        );
        assert!(
            lines[1].contains("| in-progress |"),
            "03b row should still be in-progress"
        );
    }

    // --- finalize_stages_only_doc_paths ---

    #[tokio::test]
    async fn finalize_stages_only_doc_paths() {
        let dir = TempDir::new().unwrap();
        let doc_path = dir.path().join("phase-03a-server-authored-finalize.md");
        std::fs::write(
            &doc_path,
            "# Phase 03a\n\n**Status:** in-progress\n\n## Update Log\n",
        )
        .unwrap();

        let runner = RecordingRunner::new();

        let result = PhaseResult::complete(rexymcp_executor::phase::Artifacts {
            files_changed: vec![],
            diff: String::new(),
            command_outputs: CommandOutputs::default(),
            update_log: String::new(),
            log_path: None,
            completion_summary: String::new(),
        });

        let inp = FinalizeInput {
            phase_doc_path: &doc_path,
            repo_root: dir.path(),
            result: &result,
            now_ms: 500,
            runner: &runner,
            model: "test-model",
        };

        let _ = finalize_complete(&inp).await.expect("should not error");

        let cmds = runner.get_commands();
        // The git add command should reference the phase doc path
        let add_cmds: Vec<&String> = cmds.iter().filter(|c| c.starts_with("git add")).collect();
        assert!(!add_cmds.is_empty(), "should have git add command");
        for add_cmd in &add_cmds {
            assert!(
                !add_cmd.contains("git add -A"),
                "must not use 'git add -A', got: {add_cmd}"
            );
            assert!(
                add_cmd.contains("phase-03a-server-authored-finalize.md"),
                "git add must reference the phase doc: {add_cmd}"
            );
        }
    }

    // --- finalize_flips_bounced_status_and_appends_entry ---

    #[tokio::test]
    async fn finalize_flips_bounced_status_and_appends_entry() {
        let dir = TempDir::new().unwrap();
        let doc_path = dir.path().join("phase-04b-tolerates-bounced-status.md");
        let initial_doc = "# Phase 04b\n\n**Status:** in-progress (bounced — see bugs/bug-04-1.md)\n\n## Update Log\n\n<!-- entries appended below this line -->\n";
        std::fs::write(&doc_path, initial_doc).unwrap();

        let runner = RecordingRunner::new();

        let result = PhaseResult::complete(rexymcp_executor::phase::Artifacts {
            files_changed: vec![FileChange {
                path: PathBuf::from("mcp/src/finalize.rs"),
                change_summary: "+80 -10".to_string(),
            }],
            diff: String::new(),
            command_outputs: CommandOutputs::default(),
            update_log: String::new(),
            log_path: None,
            completion_summary: "Bounced-status finalize fix.".to_string(),
        });

        let inp = FinalizeInput {
            phase_doc_path: &doc_path,
            repo_root: dir.path(),
            result: &result,
            now_ms: 999999,
            runner: &runner,
            model: "test-model",
        };

        let did_finalize = finalize_complete(&inp).await.expect("should not error");
        assert!(did_finalize, "should return true for bounced finalize");

        let after = std::fs::read_to_string(&doc_path).unwrap();

        // Status flipped to clean review (bounce note dropped)
        assert!(
            after.contains("**Status:** review"),
            "status should be review: {after}"
        );
        assert!(
            !after.contains("**Status:** in-progress"),
            "no residual in-progress: {after}"
        );
        assert!(
            !after.contains("bounced"),
            "bounce note must be removed: {after}"
        );

        // Entry appended
        assert!(
            after.contains("(complete, server-authored)"),
            "server-authored entry present: {after}"
        );
    }

    #[test]
    fn baseline_entry_includes_executor_line_from_model() {
        let result = PhaseResult::complete(rexymcp_executor::phase::Artifacts {
            files_changed: vec![],
            diff: String::new(),
            command_outputs: CommandOutputs::default(),
            update_log: String::new(),
            log_path: None,
            completion_summary: "Phase complete.".to_string(),
        });
        let entry = baseline_entry(&result, 12345, "abc123", "Qwen/Qwen3.6-27B-FP8");
        assert!(
            entry.contains("**Executor:** Qwen/Qwen3.6-27B-FP8"),
            "entry must contain the Executor line with the dispatched model: {entry}"
        );
    }

    #[test]
    fn baseline_entry_executor_line_ignores_self_report() {
        let result = PhaseResult::complete(rexymcp_executor::phase::Artifacts {
            files_changed: vec![],
            diff: String::new(),
            command_outputs: CommandOutputs::default(),
            update_log: String::new(),
            log_path: None,
            completion_summary: "**Executor:** Claude Sonnet 4.5 — all done".to_string(),
        });
        let entry = baseline_entry(&result, 12345, "abc123", "Qwen/Qwen3.6-27B-FP8");

        // The authoritative Executor line must be the dispatched model
        // (skip the Summary line which may contain the self-report text)
        let executor_line = entry
            .lines()
            .find(|l| l.starts_with("**Executor:**"))
            .expect("entry must contain an Executor line");
        assert!(
            executor_line.contains("Qwen/Qwen3.6-27B-FP8"),
            "Executor line must carry the dispatched model, got: {executor_line}"
        );

        // The self-reported model must not appear as an Executor attribution line
        // (it may appear inside the Summary block, which is fine)
        let non_summary_lines: Vec<&str> = entry
            .lines()
            .filter(|l| !l.contains("**Summary:**"))
            .collect();
        for line in &non_summary_lines {
            assert!(
                !line.contains("Claude Sonnet 4.5"),
                "self-reported model must not appear outside the Summary block: {line}"
            );
        }
    }

    #[tokio::test]
    async fn finalize_complete_writes_executor_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docs/dev/milestones/M99-test")).unwrap();
        let doc_path = dir
            .path()
            .join("docs/dev/milestones/M99-test/phase-01-test.md");

        let doc = r#"# Phase 01: Test

**Milestone:** M99 — Test
**Status:** in-progress
**Depends on:** none

## Update Log

<!-- entries appended below this line -->
"#;
        std::fs::write(&doc_path, doc).unwrap();

        let result = PhaseResult::complete(rexymcp_executor::phase::Artifacts {
            files_changed: vec![],
            diff: String::new(),
            command_outputs: CommandOutputs {
                format: Some("ok".to_string()),
                build: Some("ok".to_string()),
                lint: Some("ok".to_string()),
                test: Some("ok".to_string()),
            },
            update_log: String::new(),
            log_path: None,
            completion_summary: "Done.".to_string(),
        });

        let runner = RecordingRunner::new();

        let inp = FinalizeInput {
            phase_doc_path: &doc_path,
            repo_root: dir.path(),
            result: &result,
            now_ms: 1000,
            runner: &runner,
            model: "Qwen/Qwen3.6-27B-FP8",
        };

        let did_finalize = finalize_complete(&inp).await.expect("should not error");
        assert!(did_finalize, "should finalize");

        let after = std::fs::read_to_string(&doc_path).unwrap();
        assert!(
            after.contains("**Executor:** Qwen/Qwen3.6-27B-FP8"),
            "written doc must contain the Executor line with the dispatched model: {after}"
        );
    }

    // --- append_entry fixes (M42 phase-01) ---

    #[test]
    fn append_entry_separates_with_blank_line() {
        assert_eq!(append_entry("a\n", "### E\n"), "a\n\n### E\n");
    }

    #[test]
    fn append_entry_ends_with_single_newline() {
        let result = append_entry("a\n", "### E\n");
        assert!(result.ends_with('\n'), "must end with newline");
        assert!(
            !result.ends_with("\n\n"),
            "must not end with double newline"
        );
    }

    #[test]
    fn append_entry_collapses_existing_trailing_blanks() {
        assert_eq!(append_entry("a\n\n\n", "### E\n"), "a\n\n### E\n");
    }

    // --- baseline_entry blank line before files list (M42 phase-01) ---

    #[test]
    fn baseline_entry_blank_line_before_files_list() {
        let result = PhaseResult::complete(rexymcp_executor::phase::Artifacts {
            files_changed: vec![FileChange {
                path: PathBuf::from("src/lib.rs"),
                change_summary: "+5 -2".to_string(),
            }],
            diff: String::new(),
            command_outputs: CommandOutputs::default(),
            update_log: String::new(),
            log_path: None,
            completion_summary: "Done.".to_string(),
        });
        let entry = baseline_entry(&result, 12345, "abc123", "test-model");
        assert!(
            entry.contains("**Files changed:**\n\n- "),
            "must have blank line before files list: {entry:?}"
        );
    }

    // --- flip_readme_row width preservation (M42 phase-01) ---

    #[test]
    fn flip_readme_row_preserves_cell_width() {
        let row =
            "| 02  | lexer (source → `Token[]`, scan errors)                 | todo        |\n";
        let result = flip_readme_row(row, "lexer");
        let updated = result.expect("should find and flip the row");
        let line = updated.lines().next().unwrap();
        // Same total char count as input line
        assert_eq!(
            line.chars().count(),
            row.trim_end().chars().count(),
            "char count must be preserved: input={} output={}",
            row.trim_end().chars().count(),
            line.chars().count()
        );
        // Last cell trims to "review"
        let last_cell = line.rsplit('|').nth(1).unwrap().trim();
        assert_eq!(last_cell, "review");
    }

    #[test]
    fn flip_readme_row_preserves_wide_in_progress_width() {
        let row = "| 03a | Server-authored finalize | in-progress |\n";
        let result = flip_readme_row(row, "Server-authored");
        let updated = result.expect("should find and flip the row");
        let line = updated.lines().next().unwrap();
        assert_eq!(
            line.chars().count(),
            row.trim_end().chars().count(),
            "char count must be preserved"
        );
        let last_cell = line.rsplit('|').nth(1).unwrap().trim();
        assert_eq!(last_cell, "review");
    }

    #[test]
    fn flip_readme_row_narrow_cell_does_not_truncate() {
        let row = "|04|thing|todo|\n";
        let result = flip_readme_row(row, "thing");
        let updated = result.expect("should find and flip the row");
        let line = updated.lines().next().unwrap();
        let last_cell = line.rsplit('|').nth(1).unwrap().trim();
        assert_eq!(
            last_cell, "review",
            "narrow cell must not truncate 'review': got '{last_cell}'"
        );
    }

    #[test]
    fn flip_readme_row_preserves_trailing_newline() {
        let row = "| 01 | Phase | todo |\n";
        let result = flip_readme_row(row, "Phase");
        let updated = result.expect("should find and flip the row");
        assert!(updated.ends_with('\n'), "must preserve trailing newline");
    }

    #[test]
    fn flip_readme_row_without_trailing_newline_stays_without() {
        let row = "| 01 | Phase | todo |";
        let result = flip_readme_row(row, "Phase");
        let updated = result.expect("should find and flip the row");
        assert!(
            !updated.ends_with('\n'),
            "must not add trailing newline when input lacks one"
        );
    }

    // --- golden round-trip: flip_status_to_review → append_entry (M42 phase-01) ---

    #[test]
    fn golden_roundtrip_flip_then_append_produces_wellformed_doc() {
        let doc = "# Phase 01: Well-formed bookkeeping output\n\n**Milestone:** M42 — Bookkeeping Format Hygiene\n**Status:** in-progress\n**Depends on:** none\n\n## Update Log\n\n<!-- entries appended below this line -->\n\n### Update — 2026-07-24 23:09 (started)\n\nStarted implementation by AI executor.\n";
        let entry = "### Update — 2026-07-24 23:16 (complete, server-authored)\n\n**Summary:** Done.\n\n**Acceptance criteria:** all ticked above.\n\n**Notes:** server-authored completion entry.\n";

        let after_flip = flip_status_to_review(doc);
        assert!(after_flip.contains("**Status:** review"));
        assert!(!after_flip.contains("**Status:** in-progress"));

        let final_doc = append_entry(&after_flip, entry);

        // The started entry and the complete entry must be separated by a blank line
        assert!(
            final_doc.contains("by AI executor.\n\n### Update — 2026-07-24 23:16"),
            "blank line must separate entries: {final_doc:?}"
        );

        // The doc must end with exactly one newline
        assert!(final_doc.ends_with('\n'), "must end with newline");
        assert!(
            !final_doc.ends_with("\n\n"),
            "must not end with double newline"
        );

        // The status line must be clean review (no residual in-progress)
        assert!(final_doc.contains("**Status:** review\n"));

        // Full expected output (byte-for-byte)
        let expected = "# Phase 01: Well-formed bookkeeping output\n\n**Milestone:** M42 — Bookkeeping Format Hygiene\n**Status:** review\n**Depends on:** none\n\n## Update Log\n\n<!-- entries appended below this line -->\n\n### Update — 2026-07-24 23:09 (started)\n\nStarted implementation by AI executor.\n\n### Update — 2026-07-24 23:16 (complete, server-authored)\n\n**Summary:** Done.\n\n**Acceptance criteria:** all ticked above.\n\n**Notes:** server-authored completion entry.\n";
        assert_eq!(final_doc, expected);
    }
}
