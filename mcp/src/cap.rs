use rexymcp_executor::phase::{CommandOutputs, PhaseResult};

/// Per-field byte budget for output capping. ~12.5K tokens at 4 bytes/token
/// heuristic, well under any reasonable MCP per-tool ceiling.
pub const MAX_FIELD_BYTES: usize = 50_000;

/// Truncate a string to at most `MAX_FIELD_BYTES` bytes on a UTF-8 character
/// boundary, appending the truncation marker when truncation occurs.
fn cap_string(s: String) -> String {
    if s.len() <= MAX_FIELD_BYTES {
        return s;
    }

    let elided = s.len() - MAX_FIELD_BYTES;
    let mut kept = String::with_capacity(MAX_FIELD_BYTES);
    for (i, c) in s.char_indices() {
        if i >= MAX_FIELD_BYTES {
            break;
        }
        kept.push(c);
    }
    kept.push_str(&format!("\n\n[truncated: {} bytes elided]", elided));
    kept
}

/// Cap every long-string field on `PhaseResult` so the MCP return value stays
/// within a per-field byte budget.
pub fn cap_phase_result(result: PhaseResult) -> PhaseResult {
    let diff = cap_string(result.diff);

    let update_log = cap_string(result.update_log);

    let command_outputs = CommandOutputs {
        format: result.command_outputs.format.map(cap_string),
        build: result.command_outputs.build.map(cap_string),
        lint: result.command_outputs.lint.map(cap_string),
        test: result.command_outputs.test.map(cap_string),
    };

    let briefing = result.briefing.map(|mut b| {
        b.working_files = b
            .working_files
            .into_iter()
            .map(|f| rexymcp_executor::phase::briefing::WorkingFile {
                path: f.path,
                content: cap_string(f.content),
            })
            .collect();
        // what_was_tried[].one_line is already capped to MAX_ATTEMPT_CHARS=200
        // upstream; do not double-cap.
        b
    });

    PhaseResult {
        status: result.status,
        files_changed: result.files_changed,
        diff,
        command_outputs,
        update_log,
        briefing,
        log_path: result.log_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::phase::PhaseStatus;
    use rexymcp_executor::phase::briefing::{Briefing, WorkingFile};

    fn long_string(len: usize) -> String {
        "x".repeat(len)
    }

    fn short_string() -> String {
        "short".to_string()
    }

    fn base_result() -> PhaseResult {
        PhaseResult {
            status: PhaseStatus::Complete,
            files_changed: vec![],
            diff: String::new(),
            command_outputs: CommandOutputs::default(),
            update_log: String::new(),
            briefing: None,
            log_path: None,
        }
    }

    #[test]
    fn caps_long_diff() {
        let mut r = base_result();
        r.diff = long_string(60_000);

        let capped = cap_phase_result(r);
        assert!(capped.diff.len() <= MAX_FIELD_BYTES + 100);
        assert!(capped.diff.contains("[truncated:"));
        assert!(capped.diff.contains("bytes elided]"));
    }

    #[test]
    fn caps_long_update_log() {
        let mut r = base_result();
        r.update_log = long_string(55_000);

        let capped = cap_phase_result(r);
        assert!(capped.update_log.len() <= MAX_FIELD_BYTES + 100);
        assert!(capped.update_log.contains("[truncated:"));
    }

    #[test]
    fn caps_long_command_output_build() {
        let mut r = base_result();
        r.command_outputs.build = Some(long_string(52_000));

        let capped = cap_phase_result(r);
        let build = capped.command_outputs.build.unwrap();
        assert!(build.len() <= MAX_FIELD_BYTES + 100);
        assert!(build.contains("[truncated:"));
    }

    #[test]
    fn caps_all_command_outputs() {
        let mut r = base_result();
        r.command_outputs.format = Some(long_string(51_000));
        r.command_outputs.build = Some(long_string(51_000));
        r.command_outputs.lint = Some(long_string(51_000));
        r.command_outputs.test = Some(long_string(51_000));

        let capped = cap_phase_result(r);
        assert!(
            capped
                .command_outputs
                .format
                .as_ref()
                .unwrap()
                .contains("[truncated:")
        );
        assert!(
            capped
                .command_outputs
                .build
                .as_ref()
                .unwrap()
                .contains("[truncated:")
        );
        assert!(
            capped
                .command_outputs
                .lint
                .as_ref()
                .unwrap()
                .contains("[truncated:")
        );
        assert!(
            capped
                .command_outputs
                .test
                .as_ref()
                .unwrap()
                .contains("[truncated:")
        );
    }

    #[test]
    fn caps_briefing_working_file_content() {
        let mut r = base_result();
        r.status = PhaseStatus::HardFail;
        r.briefing = Some(Briefing {
            goal: "g".to_string(),
            acceptance_criteria: "ac".to_string(),
            diagnostics: vec![],
            working_files: vec![WorkingFile {
                path: std::path::PathBuf::from("src/lib.rs"),
                content: long_string(60_000),
            }],
            what_was_tried: vec![],
            current_blocker: rexymcp_executor::phase::briefing::Blocker::BudgetExceeded,
            budget_remaining: "0".to_string(),
        });

        let capped = cap_phase_result(r);
        let wf = &capped.briefing.as_ref().unwrap().working_files[0];
        assert!(wf.content.len() <= MAX_FIELD_BYTES + 100);
        assert!(wf.content.contains("[truncated:"));
    }

    #[test]
    fn leaves_short_field_untouched() {
        let mut r = base_result();
        r.diff = short_string();

        let capped = cap_phase_result(r);
        assert_eq!(capped.diff, "short");
    }

    #[test]
    fn leaves_none_command_output_as_none() {
        let r = base_result();
        assert!(r.command_outputs.build.is_none());

        let capped = cap_phase_result(r);
        assert!(capped.command_outputs.build.is_none());
    }

    #[test]
    fn leaves_none_briefing_untouched() {
        let r = base_result();
        assert!(r.briefing.is_none());

        let capped = cap_phase_result(r);
        assert!(capped.briefing.is_none());
    }

    #[test]
    fn respects_utf8_char_boundaries() {
        let mut r = base_result();
        let multi_byte = "äöü".repeat(20_000);
        r.diff = multi_byte;

        let capped = cap_phase_result(r);
        assert!(
            std::str::from_utf8(capped.diff.as_bytes()).is_ok(),
            "capped diff must be valid UTF-8"
        );
        assert!(capped.diff.contains("[truncated:"));
    }

    #[test]
    fn truncation_marker_includes_elided_count() {
        let mut r = base_result();
        r.diff = long_string(52_000);

        let capped = cap_phase_result(r);
        assert!(capped.diff.contains("2000 bytes elided"));
    }

    #[test]
    fn does_not_double_cap_what_was_tried() {
        let mut r = base_result();
        r.status = PhaseStatus::HardFail;
        let summary = rexymcp_executor::phase::briefing::AttemptSummary {
            one_line: "Tried patch on src/lib.rs; succeeded.".to_string(),
        };
        r.briefing = Some(Briefing {
            goal: "g".to_string(),
            acceptance_criteria: "ac".to_string(),
            diagnostics: vec![],
            working_files: vec![],
            what_was_tried: vec![summary],
            current_blocker: rexymcp_executor::phase::briefing::Blocker::BudgetExceeded,
            budget_remaining: "0".to_string(),
        });

        let capped = cap_phase_result(r);
        let tried = &capped.briefing.as_ref().unwrap().what_was_tried[0];
        assert_eq!(tried.one_line, "Tried patch on src/lib.rs; succeeded.");
    }
}
