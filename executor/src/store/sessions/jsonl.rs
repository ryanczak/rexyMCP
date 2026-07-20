use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::event::{SessionEvent, SessionRecord};

/// Thread-safe handle to a SessionLogger. Passed through call chains via clone.
pub type SessionLogHandle = Arc<Mutex<SessionLogger>>;

/// Convenience: create a SessionLogHandle from a log dir and session ID.
pub fn open_session_log(log_dir: &Path, session_id: &str) -> std::io::Result<SessionLogHandle> {
    let logger = SessionLogger::open(log_dir, session_id)?;
    Ok(Arc::new(Mutex::new(logger)))
}

/// Log a session event. Errors are intentionally discarded — session logging is
/// best-effort and must never affect loop behavior.
pub fn session_log(handle: &SessionLogHandle, ts: u64, turn: usize, event: SessionEvent) {
    let record = SessionRecord { ts, turn, event };
    if let Ok(mut logger) = handle.lock() {
        let _ = logger.log(&record);
    }
}

pub struct SessionLogger {
    writer: BufWriter<File>,
    path: PathBuf,
}

impl SessionLogger {
    pub fn open(log_dir: &Path, session_id: &str) -> std::io::Result<Self> {
        std::fs::create_dir_all(log_dir)?;
        let path = log_dir.join(format!("session-{session_id}.jsonl"));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            writer: BufWriter::new(file),
            path,
        })
    }

    pub fn log(&mut self, record: &SessionRecord) -> std::io::Result<()> {
        let line = serde_json::to_string(record).map_err(std::io::Error::other)?;
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn read_session_log(path: &Path) -> std::io::Result<Vec<SessionRecord>> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(e) => return Err(e),
    };
    let mut records = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<SessionRecord>(trimmed) {
            records.push(record);
        }
    }
    Ok(records)
}

pub fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{:08x}", secs as u32)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::*;
    use crate::governor::verifier::{Diagnostic, Severity};
    use crate::parser::{Format, ParseFailure, ToolCall};
    use crate::store::sessions::event::FileNumstat;
    use serde_json::json;

    fn make_record(event: SessionEvent, turn: usize) -> SessionRecord {
        SessionRecord {
            ts: 1_717_000_000_000,
            turn,
            event,
        }
    }

    #[test]
    fn session_event_round_trips_through_json() {
        let event = SessionEvent::ToolResult {
            name: "bash".into(),
            succeeded: true,
            output_preview: "ls -la".into(),
            output_bytes: 999,
        };
        let record = make_record(event, 5);
        let json = serde_json::to_string(&record).unwrap();
        let round_tripped: SessionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record.turn, round_tripped.turn);
        match &round_tripped.event {
            SessionEvent::ToolResult {
                name,
                succeeded,
                output_preview,
                output_bytes,
            } => {
                assert_eq!(name, "bash");
                assert!(*succeeded);
                assert_eq!(output_preview, "ls -la");
                assert_eq!(*output_bytes, 999);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn session_logger_creates_file() {
        let tmp = TempDir::new().unwrap();
        let logger = SessionLogger::open(tmp.path(), "abc12345").unwrap();
        assert!(logger.path().exists());
        assert!(
            logger
                .path()
                .to_string_lossy()
                .contains("session-abc12345.jsonl")
        );
    }

    #[test]
    fn session_logger_appends_lines() {
        let tmp = TempDir::new().unwrap();
        let mut logger = SessionLogger::open(tmp.path(), "test").unwrap();
        for i in 0..3 {
            let event = SessionEvent::Prompt {
                rendered: format!("msg {i}"),
            };
            logger.log(&make_record(event, i)).unwrap();
        }
        let content = std::fs::read_to_string(logger.path()).unwrap();
        let lines: Vec<_> = content.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn read_session_log_reads_written_records() {
        let tmp = TempDir::new().unwrap();
        let mut logger = SessionLogger::open(tmp.path(), "test").unwrap();
        let events = vec![
            SessionEvent::SessionStart {
                session_id: "s1".into(),
                model: "llama-3".into(),
                phase: "phase-01".into(),
            },
            SessionEvent::Prompt {
                rendered: "hello".into(),
            },
            SessionEvent::ToolResult {
                name: "read_file".into(),
                succeeded: true,
                output_preview: "fn main()".into(),
                output_bytes: 0,
            },
            SessionEvent::SessionEnd {
                status: "success".into(),
                turns: 1,
            },
        ];
        for (i, event) in events.into_iter().enumerate() {
            logger.log(&make_record(event, i)).unwrap();
        }
        let records = read_session_log(logger.path()).unwrap();
        assert_eq!(records.len(), 4);
        assert!(matches!(
            records[0].event,
            SessionEvent::SessionStart { .. }
        ));
        assert!(matches!(records[1].event, SessionEvent::Prompt { .. }));
        assert!(matches!(records[2].event, SessionEvent::ToolResult { .. }));
        assert!(matches!(records[3].event, SessionEvent::SessionEnd { .. }));
    }

    #[test]
    fn read_session_log_handles_partial_last_line() {
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("session-partial.jsonl");
        let mut file = File::create(&log_path).unwrap();
        let r1 = make_record(
            SessionEvent::Prompt {
                rendered: "one".into(),
            },
            1,
        );
        writeln!(file, "{}", serde_json::to_string(&r1).unwrap()).unwrap();
        let r2 = make_record(
            SessionEvent::Prompt {
                rendered: "two".into(),
            },
            2,
        );
        writeln!(file, "{}", serde_json::to_string(&r2).unwrap()).unwrap();
        let r3 = make_record(
            SessionEvent::Prompt {
                rendered: "three".into(),
            },
            3,
        );
        writeln!(file, "{}", serde_json::to_string(&r3).unwrap()).unwrap();
        let r4 = make_record(
            SessionEvent::Prompt {
                rendered: "four".into(),
            },
            4,
        );
        let partial = serde_json::to_string(&r4).unwrap();
        writeln!(file, "{}", &partial[..partial.len() - 2]).unwrap();
        drop(file);

        let records = read_session_log(&log_path).unwrap();
        assert_eq!(records.len(), 3);
    }

    #[test]
    fn read_session_log_returns_empty_for_missing_file() {
        let result = read_session_log(Path::new("/nonexistent/path.jsonl")).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn generate_session_id_is_8_chars_hex() {
        let id = generate_session_id();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn all_event_variants_serialize_with_event_type() {
        let tool_call = ToolCall {
            name: "read_file".into(),
            arguments: json!({ "path": "src/main.rs" }),
            origin: crate::parser::Origin::Extracted {
                format: Format::Hermes,
            },
        };
        let parse_failure = ParseFailure {
            raw: "bad input".into(),
            detected_format: Some(Format::LooseJson),
            candidates: vec![],
            feedback: "unknown tool".into(),
        };
        let diag = Diagnostic {
            path: std::path::PathBuf::from("src/lib.rs"),
            line: 5,
            column: Some(10),
            severity: Severity::Error,
            message: "cannot find function".into(),
            code: Some("E0425".into()),
        };
        let variants: Vec<SessionEvent> = vec![
            SessionEvent::SessionStart {
                session_id: "s1".into(),
                model: "llama-3".into(),
                phase: "phase-01".into(),
            },
            SessionEvent::Prompt {
                rendered: "hi".into(),
            },
            SessionEvent::Completion {
                raw: "hello".into(),
            },
            SessionEvent::Parsed {
                tool_call: tool_call.clone(),
            },
            SessionEvent::ParseFailed {
                failure: parse_failure.clone(),
            },
            SessionEvent::ToolResult {
                name: "bash".into(),
                succeeded: true,
                output_preview: "file.txt".into(),
                output_bytes: 0,
            },
            SessionEvent::Verify {
                diagnostics: vec![diag.clone()],
            },
            SessionEvent::HardFail {
                reason: "budget exceeded".into(),
            },
            SessionEvent::Progress {
                turn: 3,
                stage: "verify".into(),
                files_changed: vec![FileNumstat {
                    path: "src/lib.rs".into(),
                    added: 5,
                    removed: 2,
                }],
                message: "turn 3, stage verify".into(),
            },
            SessionEvent::SessionEnd {
                status: "success".into(),
                turns: 10,
            },
        ];
        for event in variants {
            let record = make_record(event, 0);
            let json = serde_json::to_string(&record).unwrap();
            assert!(
                json.contains("\"event_type\""),
                "missing event_type for variant"
            );
            let round_tripped: SessionRecord = serde_json::from_str(&json).unwrap();
            assert_eq!(record.turn, round_tripped.turn);
        }
    }

    #[test]
    fn session_log_handle_open_and_log() {
        let tmp = TempDir::new().unwrap();
        let handle = open_session_log(tmp.path(), "handle-test").unwrap();
        session_log(
            &handle,
            1_717_000_000_000,
            0,
            SessionEvent::Prompt {
                rendered: "hello from handle".into(),
            },
        );
        let records = read_session_log(handle.lock().unwrap().path()).unwrap();
        assert_eq!(records.len(), 1);
        match &records[0].event {
            SessionEvent::Prompt { rendered } => {
                assert_eq!(rendered, "hello from handle");
            }
            _ => panic!("expected Prompt"),
        }
    }

    #[test]
    fn parsed_variant_round_trips_tool_call() {
        let tool_call = ToolCall {
            name: "write_file".into(),
            arguments: json!({ "path": "x.rs", "content": "fn main() {}" }),
            origin: crate::parser::Origin::Repaired {
                format: Format::FencedJson,
                repairs: vec![crate::parser::RepairOp::NameFuzzyMatch {
                    from: "write_fle".into(),
                    to: "write_file".into(),
                }],
            },
        };
        let record = make_record(
            SessionEvent::Parsed {
                tool_call: tool_call.clone(),
            },
            2,
        );
        let json = serde_json::to_string(&record).unwrap();
        let rt: SessionRecord = serde_json::from_str(&json).unwrap();
        match rt.event {
            SessionEvent::Parsed { tool_call: tc } => {
                assert_eq!(tc.name, "write_file");
                assert_eq!(
                    tc.origin,
                    crate::parser::Origin::Repaired {
                        format: Format::FencedJson,
                        repairs: vec![crate::parser::RepairOp::NameFuzzyMatch {
                            from: "write_fle".into(),
                            to: "write_file".into(),
                        }],
                    }
                );
            }
            _ => panic!("expected Parsed"),
        }
    }

    #[test]
    fn parse_failed_variant_round_trips() {
        let failure = ParseFailure {
            raw: "bad".into(),
            detected_format: Some(Format::Yaml),
            candidates: vec![],
            feedback: "missing name".into(),
        };
        let record = make_record(
            SessionEvent::ParseFailed {
                failure: failure.clone(),
            },
            1,
        );
        let json = serde_json::to_string(&record).unwrap();
        let rt: SessionRecord = serde_json::from_str(&json).unwrap();
        match rt.event {
            SessionEvent::ParseFailed { failure: f } => {
                assert_eq!(f.raw, "bad");
                assert_eq!(f.detected_format, Some(Format::Yaml));
                assert_eq!(f.feedback, "missing name");
            }
            _ => panic!("expected ParseFailed"),
        }
    }

    #[test]
    fn verify_variant_round_trips_diagnostics() {
        let diag = Diagnostic {
            path: std::path::PathBuf::from("src/lib.rs"),
            line: 10,
            column: None,
            severity: Severity::Error,
            message: "mismatched types".into(),
            code: None,
        };
        let record = make_record(
            SessionEvent::Verify {
                diagnostics: vec![diag.clone()],
            },
            3,
        );
        let json = serde_json::to_string(&record).unwrap();
        let rt: SessionRecord = serde_json::from_str(&json).unwrap();
        match rt.event {
            SessionEvent::Verify { diagnostics } => {
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(diagnostics[0].line, 10);
                assert_eq!(diagnostics[0].message, "mismatched types");
            }
            _ => panic!("expected Verify"),
        }
    }

    #[test]
    fn progress_variant_round_trips_numstat() {
        let record = make_record(
            SessionEvent::Progress {
                turn: 5,
                stage: "dispatch".into(),
                files_changed: vec![
                    FileNumstat {
                        path: "src/a.rs".into(),
                        added: 10,
                        removed: 3,
                    },
                    FileNumstat {
                        path: "src/b.rs".into(),
                        added: 0,
                        removed: 1,
                    },
                ],
                message: "turn 5, stage dispatch".into(),
            },
            5,
        );
        let json = serde_json::to_string(&record).unwrap();
        let rt: SessionRecord = serde_json::from_str(&json).unwrap();
        match rt.event {
            SessionEvent::Progress {
                turn,
                stage,
                files_changed,
                message,
            } => {
                assert_eq!(turn, 5);
                assert_eq!(stage, "dispatch");
                assert_eq!(files_changed.len(), 2);
                assert_eq!(files_changed[0].added, 10);
                assert_eq!(files_changed[1].removed, 1);
                assert_eq!(message, "turn 5, stage dispatch");
            }
            _ => panic!("expected Progress"),
        }
    }

    #[test]
    fn session_log_discards_errors_on_locked_handle() {
        let tmp = TempDir::new().unwrap();
        let handle = open_session_log(tmp.path(), "poison-test").unwrap();
        let poisoned = Arc::clone(&handle);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _lock = poisoned.lock().unwrap();
            panic!("intentional panic inside lock");
        }));
        assert!(result.is_err());
        // Should not panic — errors are discarded.
        session_log(
            &handle,
            1_717_000_000_000,
            0,
            SessionEvent::Prompt {
                rendered: "after poison".into(),
            },
        );
    }

    #[test]
    fn tool_result_line_without_output_bytes_parses_default() {
        // Pre-phase-02 tool_result line: no output_bytes key
        let line = r#"{"ts":100,"turn":1,"event":{"event_type":"tool_result","name":"read_file","succeeded":true,"output_preview":"content"}}"#;
        let record: SessionRecord = serde_json::from_str(line).unwrap();
        match record.event {
            SessionEvent::ToolResult { output_bytes, .. } => {
                assert_eq!(output_bytes, 0);
            }
            _ => panic!("expected ToolResult"),
        }
    }
}
