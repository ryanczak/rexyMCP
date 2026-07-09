//! Resume-assembly helpers for `continue_phase`.
//!
//! Builds a briefing-seeded resume context from a prior session log, the
//! current uncommitted diff, and architect guidance. The resulting
//! `ResumeContext` is threaded through the runner so a resumed phase picks up
//! where the prior run stopped without rehydrating the transcript.

use std::collections::HashMap;
use std::path::Path;

use rexymcp_executor::agent::command::CommandRunner;
use rexymcp_executor::store::sessions::event::{SessionEvent, TaskState};
use rexymcp_executor::store::sessions::jsonl::read_session_log;

/// Cap on the uncommitted diff injected into the resume preamble.
/// Mirrors the executor's `MAX_DIFF_CHARS` constant.
const MAX_RESUME_DIFF_CHARS: usize = 50_000;

/// Carrier for resume context, threaded through `RunPhaseConfig` / `AssemblyInput`.
#[derive(Debug, Default, Clone)]
pub struct ResumeContext {
    /// Markdown appended to the phase doc in the system prompt.
    pub preamble: String,
    /// Restored task states (id → state) for `PhaseInput.resumed_task_states`.
    pub task_states: HashMap<String, TaskState>,
}

/// Restore task states from a prior run's session log.
///
/// Reads `SessionEvent::TaskUpdate` records and folds them into a map with
/// last-write-wins semantics. Returns an empty map when the path is `None`,
/// missing, or unreadable.
pub fn restore_task_states(prior_log_path: Option<&Path>) -> HashMap<String, TaskState> {
    let Some(path) = prior_log_path else {
        return HashMap::new();
    };
    let records = match read_session_log(path) {
        Ok(r) => r,
        Err(_) => return HashMap::new(),
    };
    let mut states = HashMap::new();
    for record in &records {
        if let SessionEvent::TaskUpdate { id, state, .. } = &record.event {
            states.insert(id.clone(), *state);
        }
    }
    states
}

/// Compute the uncommitted working-tree diff vs HEAD.
///
/// Runs `git --no-pager diff HEAD` via the runner. On success returns the
/// stdout truncated to `MAX_RESUME_DIFF_CHARS` characters; on failure returns
/// an empty string.
pub async fn current_diff(runner: &dyn CommandRunner, repo_root: &Path) -> String {
    let result = runner.run("git --no-pager diff HEAD", repo_root).await;
    if result.success {
        let out = result.output;
        if out.chars().count() > MAX_RESUME_DIFF_CHARS {
            out.chars().take(MAX_RESUME_DIFF_CHARS).collect()
        } else {
            out
        }
    } else {
        String::new()
    }
}

/// Render the `# Resume context` markdown block.
///
/// **Seed-safety is load-bearing:** this text is fed to `seed_from_spec`, so it
/// must NOT contain any pattern the seeder parses as a task — no `N. **bold**`
/// list items and no `### N.` / `### Task N` headings. Uses `##` sub-headings
/// and renders task progress as a plain bullet list.
pub fn render_preamble(
    guidance: &str,
    diff: &str,
    task_states: &HashMap<String, TaskState>,
) -> String {
    let diff_block: String = if diff.is_empty() {
        "```\n(no uncommitted changes)\n```".to_string()
    } else {
        format!("```\n{}\n```", diff)
    };

    let task_progress = if task_states.is_empty() {
        "(no prior task state recorded)".to_string()
    } else {
        let mut entries: Vec<String> = task_states
            .iter()
            .map(|(id, state)| {
                let state_str = match state {
                    TaskState::Pending => "pending",
                    TaskState::Active => "active",
                    TaskState::Done => "done",
                };
                format!("- {} ({})", id, state_str)
            })
            .collect();
        entries.sort();
        entries.join("\n")
    };

    format!(
        "\n\n# Resume context\n\n\
         You are RESUMING this phase. A prior executor run did not finish; you are\n\
         continuing its work, not starting over. Build on what is already on disk. Do\n\
         not redo tasks already marked done.\n\n\
         ## Architect guidance\n\n{}\n\n\
         ## Work already on disk (uncommitted diff vs HEAD)\n\n{}\n\n\
         ## Prior task progress\n\n{}",
        guidance, diff_block, task_progress
    )
}

/// Build a complete `ResumeContext` from the architect's guidance, prior log,
/// and repo state.
pub async fn build_resume_context(
    guidance: &str,
    prior_log_path: Option<&Path>,
    repo_root: &Path,
    runner: &dyn CommandRunner,
) -> ResumeContext {
    let task_states = restore_task_states(prior_log_path);
    let diff = current_diff(runner, repo_root).await;
    let preamble = render_preamble(guidance, &diff, &task_states);
    ResumeContext {
        preamble,
        task_states,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_task_states_folds_last_write_wins() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("test.jsonl");
        // Write proper SessionRecord objects (read_session_log expects {ts, turn, event}).
        let line1 = serde_json::json!({
            "ts": 1000,
            "turn": 0,
            "event": {
                "event_type": "task_update",
                "id": "1",
                "title": "First task",
                "state": "pending"
            }
        });
        let line2 = serde_json::json!({
            "ts": 2000,
            "turn": 5,
            "event": {
                "event_type": "task_update",
                "id": "1",
                "title": "First task",
                "state": "done"
            }
        });
        let line3 = serde_json::json!({
            "ts": 1000,
            "turn": 0,
            "event": {
                "event_type": "task_update",
                "id": "2",
                "title": "Second task",
                "state": "active"
            }
        });
        std::fs::write(&log_path, format!("{}\n{}\n{}\n", line1, line2, line3)).unwrap();

        let states = restore_task_states(Some(&log_path));
        assert_eq!(states.len(), 2);
        assert_eq!(
            *states.get("1").unwrap(),
            TaskState::Done,
            "later record should win"
        );
        assert_eq!(*states.get("2").unwrap(), TaskState::Active);
    }

    #[test]
    fn restore_task_states_empty_for_missing_log() {
        let states = restore_task_states(None);
        assert!(states.is_empty());

        let states = restore_task_states(Some(Path::new("/no/such/path.jsonl")));
        assert!(states.is_empty());
    }

    #[test]
    fn resume_preamble_seeds_no_tasks() {
        let states: HashMap<String, TaskState> = [
            ("1".to_string(), TaskState::Done),
            ("2".to_string(), TaskState::Active),
        ]
        .into_iter()
        .collect();
        let preamble = render_preamble(
            "1. do a thing\n2. **bold task** — should not seed\n- plain bullet",
            "some diff content",
            &states,
        );
        // Test preamble in isolation — it has no ## Spec section, so trivially seeds nothing.
        let tasks = rexymcp_executor::agent::tasks::seed_from_spec(&preamble);
        assert!(
            tasks.is_empty(),
            "preamble in isolation must seed zero tasks: {:?}",
            tasks
        );

        // Test preamble appended to a real phase doc with a ## Spec section —
        // the real risk: the preamble must not extend the Spec section or inject
        // phantom tasks.
        let full_doc = format!(
            "\
## Spec

1. **Real task one** — do this

## Acceptance criteria

- something

{}",
            preamble
        );
        let tasks = rexymcp_executor::agent::tasks::seed_from_spec(&full_doc);
        assert_eq!(
            tasks.len(),
            1,
            "appended preamble must not inject phantom tasks: {:?}",
            tasks
        );
        assert_eq!(tasks[0].id, "1");
        assert_eq!(tasks[0].title, "Real task one");
    }

    #[test]
    fn render_preamble_includes_all_sections() {
        let states: HashMap<String, TaskState> =
            [("1".to_string(), TaskState::Done)].into_iter().collect();
        let preamble = render_preamble("fix the bug", "diff here", &states);
        assert!(preamble.contains("# Resume context"));
        assert!(preamble.contains("## Architect guidance"));
        assert!(preamble.contains("fix the bug"));
        assert!(preamble.contains("## Work already on disk"));
        assert!(preamble.contains("diff here"));
        assert!(preamble.contains("## Prior task progress"));
        assert!(preamble.contains("- 1 (done)"));
    }

    #[test]
    fn render_preamble_empty_diff_shows_placeholder() {
        let preamble = render_preamble("guidance", "", &HashMap::new());
        assert!(preamble.contains("(no uncommitted changes)"));
    }

    #[test]
    fn render_preamble_empty_states_shows_placeholder() {
        let preamble = render_preamble("guidance", "diff", &HashMap::new());
        assert!(preamble.contains("(no prior task state recorded)"));
    }
}
