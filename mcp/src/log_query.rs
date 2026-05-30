use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};

pub const SEARCH_DEFAULT_LIMIT: usize = 20;
pub const SEARCH_MAX_LIMIT: usize = 50;
pub const TAIL_DEFAULT_N: usize = 10;
pub const TAIL_MAX_N: usize = 50;

pub struct SearchFilter<'a> {
    pub event_type: Option<&'a str>,
    pub tool_name: Option<&'a str>,
    pub query_text: Option<&'a str>,
}

pub fn event_type_str(event: &SessionEvent) -> &'static str {
    match event {
        SessionEvent::SessionStart { .. } => "session_start",
        SessionEvent::Prompt { .. } => "prompt",
        SessionEvent::Completion { .. } => "completion",
        SessionEvent::Parsed { .. } => "parsed",
        SessionEvent::ParseFailed { .. } => "parse_failed",
        SessionEvent::ToolResult { .. } => "tool_result",
        SessionEvent::Verify { .. } => "verify",
        SessionEvent::HardFail { .. } => "hard_fail",
        SessionEvent::Progress { .. } => "progress",
        SessionEvent::SessionEnd { .. } => "session_end",
    }
}

fn matches_tool_name_filter(event: &SessionEvent, tool_name: &str) -> bool {
    match event {
        SessionEvent::Parsed { tool_call } => tool_call.name.contains(tool_name),
        SessionEvent::ToolResult { name, .. } => name.contains(tool_name),
        _ => false,
    }
}

pub fn search(
    records: &[SessionRecord],
    filter: &SearchFilter,
    limit: usize,
) -> Vec<SessionRecord> {
    let limit = if limit == 0 {
        SEARCH_DEFAULT_LIMIT
    } else {
        limit.min(SEARCH_MAX_LIMIT)
    };

    records
        .iter()
        .filter(|record| {
            if let Some(et) = filter.event_type
                && event_type_str(&record.event) != et
            {
                return false;
            }
            if let Some(tn) = filter.tool_name
                && !matches_tool_name_filter(&record.event, tn)
            {
                return false;
            }
            if let Some(qt) = filter.query_text {
                let json = serde_json::to_string(record);
                match json {
                    Ok(s) if !s.contains(qt) => return false,
                    _ => {}
                }
            }
            true
        })
        .take(limit)
        .cloned()
        .collect()
}

pub fn tail(records: &[SessionRecord], n: usize) -> Vec<SessionRecord> {
    let n = if n == 0 {
        TAIL_DEFAULT_N
    } else {
        n.min(TAIL_MAX_N)
    };

    if n >= records.len() {
        records.to_vec()
    } else {
        records[records.len() - n..].to_vec()
    }
}

pub fn get_turn(records: &[SessionRecord], turn: usize) -> Vec<SessionRecord> {
    records.iter().filter(|r| r.turn == turn).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::governor::verifier::{Diagnostic, Severity};
    use rexymcp_executor::parser::{Format, Origin, ParseFailure, ToolCall};
    use serde_json::json;

    fn make_record(event: SessionEvent, turn: usize) -> SessionRecord {
        SessionRecord {
            ts: 1_717_000_000_000 + turn as u64,
            turn,
            event,
        }
    }

    fn fixture_records() -> Vec<SessionRecord> {
        let tool_call = ToolCall {
            name: "read_file".into(),
            arguments: json!({ "path": "src/main.rs" }),
            origin: Origin::Extracted {
                format: Format::Hermes,
            },
        };
        let tool_call2 = ToolCall {
            name: "write_file".into(),
            arguments: json!({ "path": "src/lib.rs" }),
            origin: Origin::Extracted {
                format: Format::Hermes,
            },
        };
        let parse_failure = ParseFailure {
            raw: "bad".into(),
            detected_format: None,
            candidates: vec![],
            feedback: "no tool".into(),
        };
        let diag = Diagnostic {
            path: std::path::PathBuf::from("src/lib.rs"),
            line: 1,
            column: None,
            severity: Severity::Warning,
            message: "unused".into(),
            code: None,
        };
        vec![
            make_record(
                SessionEvent::SessionStart {
                    session_id: "s1".into(),
                    model: "test".into(),
                    phase: "p1".into(),
                },
                0,
            ),
            make_record(
                SessionEvent::Prompt {
                    rendered: "Do something".into(),
                },
                1,
            ),
            make_record(
                SessionEvent::Completion {
                    raw: "read_file src/main.rs".into(),
                },
                1,
            ),
            make_record(
                SessionEvent::Parsed {
                    tool_call: tool_call.clone(),
                },
                1,
            ),
            make_record(
                SessionEvent::ToolResult {
                    name: "read_file".into(),
                    succeeded: true,
                    output_preview: "fn main() {}".into(),
                },
                1,
            ),
            make_record(
                SessionEvent::Parsed {
                    tool_call: tool_call2.clone(),
                },
                2,
            ),
            make_record(
                SessionEvent::ToolResult {
                    name: "write_file".into(),
                    succeeded: true,
                    output_preview: "ok".into(),
                },
                2,
            ),
            make_record(
                SessionEvent::ParseFailed {
                    failure: parse_failure.clone(),
                },
                3,
            ),
            make_record(
                SessionEvent::Verify {
                    diagnostics: vec![diag.clone()],
                },
                3,
            ),
            make_record(
                SessionEvent::HardFail {
                    reason: "budget exceeded".into(),
                },
                3,
            ),
            make_record(
                SessionEvent::Progress {
                    turn: 3,
                    stage: "verify".into(),
                    files_changed: vec![],
                    message: "turn 3 done".into(),
                },
                3,
            ),
            make_record(
                SessionEvent::SessionEnd {
                    status: "fail".into(),
                    turns: 3,
                },
                3,
            ),
        ]
    }

    #[test]
    fn event_type_str_round_trips_all_variants() {
        let records = fixture_records();
        let expected = vec![
            "session_start",
            "prompt",
            "completion",
            "parsed",
            "tool_result",
            "parsed",
            "tool_result",
            "parse_failed",
            "verify",
            "hard_fail",
            "progress",
            "session_end",
        ];
        for (i, record) in records.iter().enumerate() {
            assert_eq!(
                event_type_str(&record.event),
                expected[i],
                "mismatch at record {}",
                i
            );
        }
    }

    #[test]
    fn search_no_filters_returns_first_limit_records() {
        let records = fixture_records();
        let filter = SearchFilter {
            event_type: None,
            tool_name: None,
            query_text: None,
        };
        let result = search(&records, &filter, 3);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].turn, 0);
        assert_eq!(result[1].turn, 1);
        assert_eq!(result[2].turn, 1);
    }

    #[test]
    fn search_event_type_filter() {
        let records = fixture_records();
        let filter = SearchFilter {
            event_type: Some("tool_result"),
            tool_name: None,
            query_text: None,
        };
        let result = search(&records, &filter, 50);
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0].event, SessionEvent::ToolResult { .. }));
        assert!(matches!(result[1].event, SessionEvent::ToolResult { .. }));
    }

    #[test]
    fn search_tool_name_filter_matches_parsed_and_tool_result() {
        let records = fixture_records();
        let filter = SearchFilter {
            event_type: None,
            tool_name: Some("read_file"),
            query_text: None,
        };
        let result = search(&records, &filter, 50);
        assert_eq!(result.len(), 2);
        assert!(matches!(result[0].event, SessionEvent::Parsed { .. }));
        assert!(matches!(result[1].event, SessionEvent::ToolResult { .. }));
    }

    #[test]
    fn search_tool_name_filter_rejects_non_tool_events() {
        let records = fixture_records();
        let filter = SearchFilter {
            event_type: None,
            tool_name: Some("read_file"),
            query_text: None,
        };
        let result = search(&records, &filter, 50);
        for r in &result {
            match &r.event {
                SessionEvent::Parsed { .. } | SessionEvent::ToolResult { .. } => {}
                _ => panic!(
                    "tool_name filter should only match Parsed or ToolResult, got {:?}",
                    r.event
                ),
            }
        }
    }

    #[test]
    fn search_query_text_substring() {
        let records = fixture_records();
        let filter = SearchFilter {
            event_type: None,
            tool_name: None,
            query_text: Some("fn main()"),
        };
        let result = search(&records, &filter, 50);
        assert!(!result.is_empty());
    }

    #[test]
    fn search_combined_filters_and() {
        let records = fixture_records();
        let filter = SearchFilter {
            event_type: Some("tool_result"),
            tool_name: Some("write_file"),
            query_text: None,
        };
        let result = search(&records, &filter, 50);
        assert_eq!(result.len(), 1);
        match &result[0].event {
            SessionEvent::ToolResult { name, .. } => {
                assert_eq!(name, "write_file");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn search_limit_clamped_to_max() {
        let mut records = fixture_records();
        while records.len() < 60 {
            records.push(make_record(
                SessionEvent::Prompt {
                    rendered: "pad".into(),
                },
                records.len(),
            ));
        }
        let filter = SearchFilter {
            event_type: None,
            tool_name: None,
            query_text: None,
        };
        let result = search(&records, &filter, 1000);
        assert_eq!(result.len(), SEARCH_MAX_LIMIT);
    }

    #[test]
    fn search_limit_zero_uses_default() {
        let mut records = fixture_records();
        while records.len() < 30 {
            records.push(make_record(
                SessionEvent::Prompt {
                    rendered: "pad".into(),
                },
                records.len(),
            ));
        }
        let filter = SearchFilter {
            event_type: None,
            tool_name: None,
            query_text: None,
        };
        let result = search(&records, &filter, 0);
        assert_eq!(result.len(), SEARCH_DEFAULT_LIMIT);
    }

    #[test]
    fn search_limit_small() {
        let records = fixture_records();
        let filter = SearchFilter {
            event_type: None,
            tool_name: None,
            query_text: None,
        };
        let result = search(&records, &filter, 2);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn tail_returns_last_n_in_order() {
        let records = fixture_records();
        let result = tail(&records, 3);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].turn, 3);
        assert_eq!(result[1].turn, 3);
        assert_eq!(result[2].turn, 3);
    }

    #[test]
    fn tail_default_n_when_zero() {
        let records = fixture_records();
        let result = tail(&records, 0);
        assert_eq!(result.len(), TAIL_DEFAULT_N);
    }

    #[test]
    fn tail_clamped_to_max() {
        let records = fixture_records();
        let result = tail(&records, 1000);
        assert!(result.len() <= TAIL_MAX_N);
        assert_eq!(result.len(), records.len());
    }

    #[test]
    fn tail_more_than_available_returns_all() {
        let records = fixture_records();
        let result = tail(&records, 100);
        assert_eq!(result.len(), records.len());
    }

    #[test]
    fn get_turn_returns_all_events_for_turn() {
        let records = fixture_records();
        let result = get_turn(&records, 3);
        assert!(result.len() > 1);
        for r in &result {
            assert_eq!(r.turn, 3);
        }
    }

    #[test]
    fn get_turn_empty_when_no_records() {
        let records = fixture_records();
        let result = get_turn(&records, 999);
        assert!(result.is_empty());
    }

    #[test]
    fn get_turn_turn_zero() {
        let records = fixture_records();
        let result = get_turn(&records, 0);
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0].event, SessionEvent::SessionStart { .. }));
    }
}
