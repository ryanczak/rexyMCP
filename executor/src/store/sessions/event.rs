use serde::{Deserialize, Serialize};

/// One file's line-change summary for progress heartbeats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileNumstat {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// A single JSONL record — one line in the session log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub ts: u64,
    pub turn: usize,
    pub event: SessionEvent,
}

/// Turn-cycle event kinds. Serialized with `event_type` discriminant so each
/// JSONL line carries a tag the M5 query tools can grep for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum SessionEvent {
    SessionStart {
        session_id: String,
        model: String,
        phase: String,
    },
    Prompt {
        rendered: String,
    },
    Completion {
        raw: String,
    },
    Parsed {
        tool_call: crate::parser::ToolCall,
    },
    ParseFailed {
        failure: crate::parser::ParseFailure,
    },
    ToolResult {
        name: String,
        succeeded: bool,
        output_preview: String,
    },
    Verify {
        diagnostics: Vec<crate::governor::verifier::Diagnostic>,
    },
    HardFail {
        reason: String,
    },
    Progress {
        turn: usize,
        stage: String,
        files_changed: Vec<FileNumstat>,
        message: String,
    },
    SessionEnd {
        status: String,
        turns: usize,
    },
}
