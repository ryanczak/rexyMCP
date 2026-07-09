use std::path::{Path, PathBuf};

use rexymcp_executor::agent::command::CommandRunner;
use rexymcp_executor::phase::{CommandOutputs, FileChange, PhaseResult, PhaseStatus};

/// Inputs for the server-authored completion finalize step.
pub struct FinalizeInput<'a> {
    pub phase_doc_path: &'a Path,
    pub repo_root: &'a Path,
    pub result: &'a PhaseResult,
    pub now_ms: u64,
    pub runner: &'a dyn CommandRunner,
}

/// Server-authored bookkeeping for a completed phase. No-op (returns
/// `Ok(false)`) unless the result is `Complete` and the phase doc's
/// `**Status:**` line still reads `in-progress` — so this is inert while the
/// executor still authors its own bookkeeping (see phase-03b). On the active
/// path: flip Status to `review`, append a baseline completion entry, flip the
/// sibling milestone README's phase-table row, commit the doc changes as a
/// separate `docs:` commit, and return `Ok(true)`.
pub async fn finalize_complete(inp: &FinalizeInput<'_>) -> std::io::Result<bool> {
    if inp.result.status != PhaseStatus::Complete {
        return Ok(false);
    }
    let doc = std::fs::read_to_string(inp.phase_doc_path)?;
    if !status_is_in_progress(&doc) {
        return Ok(false);
    }

    let code_sha = git_head(inp.runner, inp.repo_root).await;
    let entry = baseline_entry(inp.result, inp.now_ms, &code_sha);
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

/// True iff some line, trimmed, equals `**Status:** in-progress`.
fn status_is_in_progress(doc: &str) -> bool {
    doc.lines()
        .any(|line| line.trim() == "**Status:** in-progress")
}

/// Replace the single frontmatter line `**Status:** in-progress` with
/// `**Status:** review`, leaving everything else byte-identical.
/// Replaces only the first such line.
fn flip_status_to_review(doc: &str) -> String {
    let mut replaced = false;
    let mut result = String::with_capacity(doc.len());
    let mut first = true;
    for line in doc.lines() {
        if !first {
            result.push('\n');
        }
        first = false;
        if !replaced && line.trim() == "**Status:** in-progress" {
            replaced = true;
            result.push_str(&line.replace("**Status:** in-progress", "**Status:** review"));
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
fn baseline_entry(result: &PhaseResult, now_ms: u64, code_sha: &str) -> String {
    let summary = if result.completion_summary.trim().is_empty() {
        "(no summary provided by executor)".to_string()
    } else {
        result.completion_summary.trim().to_string()
    };

    let gates = gate_line(&result.command_outputs);
    let command_tails = command_output_tails(&result.command_outputs);
    let files = files_changed_list(&result.files_changed);

    format!(
        "### Update — ts={now_ms} (complete, server-authored)\n\n\
         **Summary:** {summary}\n\n\
         **Gates:** {gates}\n\n\
         **Command output tails:**\n\n\
         ```\n{command_tails}\n```\n\n\
         **Files changed:**\n{files}\n\n\
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
    format!("{}\n{}\n", doc.trim_end(), entry)
}

/// Find the one table row that contains `phase_doc_filename` and ends (after
/// trimming) with `| in-progress |`; replace that row's trailing `| in-progress |`
/// with `| review |`. Return `None` if no such row.
pub fn flip_readme_row(readme_doc: &str, phase_doc_filename: &str) -> Option<String> {
    let mut found = false;
    let lines: Vec<String> = readme_doc
        .lines()
        .map(|line| {
            if !found
                && line.contains(phase_doc_filename)
                && line.trim().ends_with("| in-progress |")
            {
                found = true;
                line.trim_end().replace("| in-progress |", "| review |")
            } else {
                line.to_string()
            }
        })
        .collect();

    if found { Some(lines.join("\n")) } else { None }
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

    // --- status_is_in_progress ---

    #[test]
    fn status_is_in_progress_matches_exact_line() {
        assert!(status_is_in_progress("**Status:** in-progress"));
    }

    #[test]
    fn status_is_in_progress_rejects_review() {
        assert!(!status_is_in_progress("**Status:** review"));
    }

    #[test]
    fn status_is_in_progress_rejects_todo() {
        assert!(!status_is_in_progress("**Status:** todo"));
    }

    #[test]
    fn status_is_in_progress_rejects_done() {
        assert!(!status_is_in_progress("**Status:** done"));
    }

    #[test]
    fn status_is_in_progress_ignores_prose_containing_in_progress() {
        let doc = "This work is in-progress as of today.\n\n**Status:** review\n";
        assert!(!status_is_in_progress(doc));
    }

    // --- flip_status_to_review ---

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

    // --- flip_readme_row ---

    #[test]
    fn flip_readme_row_flips_matching_row_only() {
        let readme = "| 03a | Server-authored finalize ([phase-03a-server-authored-finalize.md](phase-03a-server-authored-finalize.md)) | in-progress |\n| 03b | Retire executor gate ([phase-03b-retire-gate.md](phase-03b-retire-gate.md)) | in-progress |\n";
        let result = flip_readme_row(readme, "phase-03a-server-authored-finalize.md");
        let updated = result.expect("should have found and flipped the row");
        assert!(updated.contains("phase-03a-server-authored-finalize.md"));
        assert!(updated.contains("| review |"));
        // The sibling row must still be in-progress
        assert!(updated.contains("phase-03b-retire-gate.md"));
        let lines: Vec<&str> = updated.lines().collect();
        assert!(lines[0].contains("| review |"), "03a row should be review");
        assert!(
            lines[1].contains("| in-progress |"),
            "03b row should still be in-progress"
        );
    }

    #[test]
    fn flip_readme_row_returns_none_when_already_review() {
        let readme = "| 03a | Phase ([phase-03a.md](phase-03a.md)) | review |\n";
        let result = flip_readme_row(readme, "phase-03a.md");
        assert!(result.is_none());
    }

    #[test]
    fn flip_readme_row_returns_none_when_row_absent() {
        let readme = "| 01 | Phase ([phase-01.md](phase-01.md)) | in-progress |\n";
        let result = flip_readme_row(readme, "phase-99.md");
        assert!(result.is_none());
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
        };

        let did_finalize = finalize_complete(&inp).await.expect("should not error");
        assert!(did_finalize, "should return true for active finalize");

        let after = std::fs::read_to_string(&doc_path).unwrap();

        // Status flipped
        assert!(after.contains("**Status:** review"));
        assert!(!after.contains("**Status:** in-progress"));

        // Entry appended
        assert!(after.contains("(complete, server-authored)"));
        assert!(after.contains("ts=999999"));
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
        };

        let did_finalize = finalize_complete(&inp).await.expect("should not error");
        assert!(did_finalize);

        let readme_after = std::fs::read_to_string(&readme_path).unwrap();
        let lines: Vec<&str> = readme_after.lines().collect();
        assert!(lines[0].contains("| review |"), "03a row should be review");
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
}
