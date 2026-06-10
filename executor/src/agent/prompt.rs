use crate::agent::tasks::Task;
use crate::config::CommandConfig;
use crate::store::sessions::event::TaskState;

use super::contract;

/// Assemble the executor system prompt from its three inputs, in the order the
/// architecture pins: the embedded executor contract, the project `STANDARDS.md`,
/// then the architect's (pre-injected) phase doc. The local model reads none of
/// these as files — they are composed in-process from strings the caller holds.
pub fn assemble_system_prompt(
    commands: &CommandConfig,
    standards: &str,
    phase_doc: &str,
) -> String {
    let contract_body = contract::assemble_executor_contract(commands);
    let mut out = String::new();
    out.push_str("# Executor contract\n\n");
    out.push_str(contract_body.trim_end());
    out.push_str("\n\n# Engineering standards\n\n");
    out.push_str(standards.trim_end());
    out.push_str("\n\n# Phase\n\n");
    out.push_str(phase_doc.trim_end());
    out.push('\n');
    out
}

/// Format epoch-millis (UTC) as `YYYY-MM-DD` using civil-from-days integer
/// arithmetic — no date dependency, deterministic, hermetic. Input is the
/// injected `clock` value (always ≥ 0), so no negative-era branch is needed.
fn format_utc_date(now_ms: u64) -> String {
    let days = (now_ms / 1_000) / 86_400; // whole days since 1970-01-01 (UTC)
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097; // day-of-era, [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day-of-year, [0, 365]
    let mp = (5 * doy + 2) / 153; // month-prime, [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

/// Format epoch-millis (UTC) as `HH:MM` (24-hour). The time-of-day is the
/// remainder of the day in seconds; pairs with `format_utc_date` to give the
/// model sub-day grounding without a date dependency.
fn format_utc_time(now_ms: u64) -> String {
    let secs_of_day = (now_ms / 1_000) % 86_400;
    let hours = secs_of_day / 3_600;
    let minutes = (secs_of_day % 3_600) / 60;
    format!("{hours:02}:{minutes:02}")
}

/// The one-line temporal-grounding header prepended to the system prompt. The
/// local model has no clock of its own; without this it stamps hallucinated
/// dates and times in its Update Log. Built from the injected `clock`, never
/// real wall-clock time, so it stays deterministic under test.
pub fn datetime_header(now_ms: u64) -> String {
    format!(
        "Today's date is {} {} (UTC).\n\n",
        format_utc_date(now_ms),
        format_utc_time(now_ms)
    )
}

/// Render a task-tracking section for the system prompt.
/// Returns empty string when `tasks` is empty (off / no Spec).
pub fn task_section(tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return String::new();
    }

    let mut out = String::from("# Task tracking\n\n");
    out.push_str(
        "Use the `update_task` tool to record progress on each tracked task as you work.\n",
    );
    out.push_str("Set a task `active` when you start it and `done` when it is complete.\n");
    out.push_str("Update tasks as you go — do not batch updates at the end.\n\n");

    for task in tasks {
        let state_str = match task.state {
            TaskState::Pending => "pending",
            TaskState::Active => "active",
            TaskState::Done => "done",
        };
        out.push_str(&format!("- [{}] {} — {}\n", state_str, task.id, task.title));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_utc_date_formats_midnight_epoch_millis() {
        assert_eq!(format_utc_date(1_780_963_200_000), "2026-06-09");
    }

    #[test]
    fn format_utc_date_truncates_time_of_day() {
        // 2026-06-09 13:45:30 UTC — time-of-day is dropped, not rolled forward
        assert_eq!(format_utc_date(1_781_012_730_000), "2026-06-09");
    }

    #[test]
    fn format_utc_date_handles_leap_day() {
        assert_eq!(format_utc_date(1_709_208_000_000), "2024-02-29");
    }

    #[test]
    fn format_utc_date_handles_epoch_zero() {
        assert_eq!(format_utc_date(0), "1970-01-01");
    }

    #[test]
    fn format_utc_date_does_not_roll_over_at_year_boundary() {
        // 2025-12-31 23:59:59 UTC — must stay 2025-12-31, not roll to 2026-01-01
        assert_eq!(format_utc_date(1_767_225_599_000), "2025-12-31");
    }

    #[test]
    fn format_utc_time_formats_midnight() {
        assert_eq!(format_utc_time(0), "00:00");
        assert_eq!(format_utc_time(1_780_963_200_000), "00:00");
    }

    #[test]
    fn format_utc_time_formats_midday() {
        // 2026-06-09 13:45:30 UTC
        assert_eq!(format_utc_time(1_781_012_730_000), "13:45");
    }

    #[test]
    fn format_utc_time_formats_late_evening_boundary() {
        // 2025-12-31 23:59:59 UTC
        assert_eq!(format_utc_time(1_767_225_599_000), "23:59");
    }

    #[test]
    fn datetime_header_contains_grounding_line() {
        let header = datetime_header(1_780_963_200_000);
        assert!(header.contains("Today's date is 2026-06-09 00:00 (UTC)."));
    }

    #[test]
    fn assembles_system_prompt_in_contract_standards_phase_order() {
        let commands = CommandConfig {
            format: Some("cargo fmt".to_string()),
            build: Some("cargo build".to_string()),
            lint: Some("cargo clippy".to_string()),
            test: Some("cargo test".to_string()),
            lint_fix: None,
        };
        let prompt = assemble_system_prompt(&commands, "STANDARDS_BODY", "PHASE_BODY");

        assert!(prompt.contains("cargo fmt"));
        assert!(prompt.contains("STANDARDS_BODY"));
        assert!(prompt.contains("PHASE_BODY"));

        let contract = prompt.find("cargo fmt").expect("contract present");
        let standards = prompt.find("STANDARDS_BODY").expect("standards present");
        let phase = prompt.find("PHASE_BODY").expect("phase present");

        assert!(
            contract < standards && standards < phase,
            "expected contract < standards < phase, got {contract}/{standards}/{phase}"
        );
    }

    #[test]
    fn system_prompt_includes_substituted_contract() {
        let commands = CommandConfig {
            format: Some("npm fmt".to_string()),
            build: Some("npm run build".to_string()),
            lint: Some("npm run lint".to_string()),
            test: Some("npm test".to_string()),
            lint_fix: None,
        };
        let prompt = assemble_system_prompt(&commands, "MY_STANDARDS", "MY_PHASE");

        assert!(prompt.contains("Executor Contract"));
        assert!(prompt.contains("npm fmt"));
        assert!(prompt.contains("npm run build"));
        assert!(prompt.contains("npm run lint"));
        assert!(prompt.contains("npm test"));
        assert!(prompt.contains("MY_STANDARDS"));
        assert!(prompt.contains("MY_PHASE"));
    }

    #[test]
    fn system_prompt_order_is_contract_then_standards_then_phase_doc() {
        let commands = CommandConfig::default();
        let prompt =
            assemble_system_prompt(&commands, "UNIQUE_STANDARDS_MARKER", "UNIQUE_PHASE_MARKER");

        let contract_pos = prompt
            .find("Executor Contract")
            .expect("contract section present");
        let standards_pos = prompt
            .find("UNIQUE_STANDARDS_MARKER")
            .expect("standards section present");
        let phase_pos = prompt
            .find("UNIQUE_PHASE_MARKER")
            .expect("phase section present");

        assert!(
            contract_pos < standards_pos && standards_pos < phase_pos,
            "expected contract < standards < phase, got {contract_pos}/{standards_pos}/{phase_pos}"
        );
    }

    #[test]
    fn task_section_lists_tasks_with_state() {
        let tasks = vec![
            Task {
                id: "1".to_string(),
                title: "First task".to_string(),
                state: TaskState::Pending,
            },
            Task {
                id: "2".to_string(),
                title: "Second task".to_string(),
                state: TaskState::Active,
            },
        ];
        let section = task_section(&tasks);
        assert!(section.contains("# Task tracking"));
        assert!(section.contains("First task"));
        assert!(section.contains("Second task"));
        assert!(section.contains("[pending] 1"));
        assert!(section.contains("[active] 2"));
    }

    #[test]
    fn task_section_empty_for_no_tasks() {
        assert_eq!(task_section(&[]), "");
    }
}
