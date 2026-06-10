use crate::store::sessions::event::TaskState;

/// One architect-seeded task. `id` is the Spec item's number ("1", "2", …);
/// `title` is its short name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub state: TaskState,
}

/// Parse the phase doc's `## Spec` section into a seeded task list, all
/// `Pending`. Pure; no I/O. Returns empty when there is no `## Spec` section
/// or it has no top-level numbered items.
pub fn seed_from_spec(phase_doc: &str) -> Vec<Task> {
    let Some(spec_start) = find_spec_section(phase_doc) else {
        return Vec::new();
    };

    let lines: Vec<&str> = phase_doc.lines().collect();
    let mut tasks = Vec::new();

    for line in lines.iter().skip(spec_start + 1) {
        // Stop at the next heading (trimmed line starts with '#')
        if line.trim().starts_with('#') {
            break;
        }
        if let Some(task) = parse_task_line(line) {
            tasks.push(task);
        }
    }

    tasks
}

/// Find the line index (0-based) of the first line whose trimmed text is
/// exactly "## Spec". Returns `None` if not found.
fn find_spec_section(phase_doc: &str) -> Option<usize> {
    phase_doc.lines().enumerate().find_map(|(i, line)| {
        if line.trim() == "## Spec" {
            Some(i)
        } else {
            None
        }
    })
}

/// Parse a single line as a top-level numbered task. Returns `None` if the
/// line is not a task (indented, not numbered, decimal-like, etc.).
fn parse_task_line(line: &str) -> Option<Task> {
    // Must start with a digit (no leading whitespace)
    let first_char = line.chars().next()?;
    if !first_char.is_ascii_digit() {
        return None;
    }

    // Must match `<digits>. <rest>` shape
    let (digits, rest) = line.split_once('.')?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // The character after '.' must be a space or tab
    if !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }

    let title = extract_title(rest);
    Some(Task {
        id: digits.to_string(),
        title,
        state: TaskState::Pending,
    })
}

/// Extract the task title from the remainder after `<digits>. `.
/// If the trimmed text starts with `**`, extract the bold span.
/// Otherwise, use the whole trimmed remainder.
fn extract_title(rest: &str) -> String {
    let trimmed = rest.trim_start();
    if let Some(after_open) = trimmed.strip_prefix("**")
        && let Some(title) = after_open.split_once("**")
    {
        return title.0.trim().to_string();
    }
    trimmed.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_top_level_numbered_items() {
        let doc = "## Spec\n\n1. **First task** — do this first\n2. Second task — do this second\n3. **Third** — last one\n";
        let tasks = seed_from_spec(doc);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].id, "1");
        assert_eq!(tasks[0].title, "First task");
        assert_eq!(tasks[1].id, "2");
        assert_eq!(tasks[1].title, "Second task — do this second");
        assert_eq!(tasks[2].id, "3");
        assert_eq!(tasks[2].title, "Third");
        for t in &tasks {
            assert_eq!(t.state, TaskState::Pending);
        }
    }

    #[test]
    fn seeds_bold_title_strips_to_bold_span() {
        let doc = "## Spec\n\n1. **Name** — rest of the line\n";
        let tasks = seed_from_spec(doc);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Name");
    }

    #[test]
    fn seeds_plain_title_keeps_whole_remainder() {
        let doc = "## Spec\n\n2. plain text\n";
        let tasks = seed_from_spec(doc);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "plain text");
    }

    #[test]
    fn ignores_indented_sub_items() {
        let doc = "## Spec\n\n1. **Parent task**\n    1. a sub-step\n    2. another sub-step\n2. **Next task**\n";
        let tasks = seed_from_spec(doc);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "1");
        assert_eq!(tasks[1].id, "2");
    }

    #[test]
    fn ignores_decimal_like_numbers() {
        let doc = "## Spec\n\n1. **Real task**\n1.5x speedup is expected\n2. **Another**\n";
        let tasks = seed_from_spec(doc);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "1");
        assert_eq!(tasks[1].id, "2");
    }

    #[test]
    fn ignores_items_outside_spec_section() {
        let doc = "1. **Before spec** — should not appear\n\n## Spec\n\n1. **In spec** — should appear\n\n## Other Section\n\n2. **After spec** — should not appear\n";
        let tasks = seed_from_spec(doc);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "1");
        assert_eq!(tasks[0].title, "In spec");
    }

    #[test]
    fn empty_when_no_spec_section() {
        let doc = "# No spec here\n\nSome random text.\n";
        let tasks = seed_from_spec(doc);
        assert!(tasks.is_empty());
    }

    #[test]
    fn parses_multi_digit_ids() {
        let doc = "## Spec\n\n10. Tenth item\n11. Eleventh item\n";
        let tasks = seed_from_spec(doc);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "10");
        assert_eq!(tasks[0].title, "Tenth item");
        assert_eq!(tasks[1].id, "11");
        assert_eq!(tasks[1].title, "Eleventh item");
    }
}
