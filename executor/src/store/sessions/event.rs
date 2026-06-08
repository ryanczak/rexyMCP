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
    /// Per-turn resource snapshot: cumulative token usage and the fraction of
    /// the context-window budget consumed going into this turn. `context_pct`
    /// is 0.0 when the ceiling is the "unmeasured" sentinel (`usize::MAX`).
    /// `context_used` / `context_window` are the raw token counts that make up
    /// `context_pct`; both are 0 when the budget is the unmeasured sentinel.
    Metrics {
        input_tokens: u32,
        output_tokens: u32,
        context_pct: f64,
        /// Estimated tokens currently occupying the context window.
        context_used: u32,
        /// Budget ceiling in tokens (0 = unmeasured / no real ceiling configured).
        context_window: u32,
    },
    /// Emitted each time the context compactor runs (on budget overflow at the
    /// top of a turn). Mirrors `CompactionReport`: token totals before/after and
    /// the message counts touched. Tokens freed = `tokens_before - tokens_after`.
    Compaction {
        tokens_before: usize,
        tokens_after: usize,
        messages_signaturized: usize,
        messages_evicted: usize,
    },
    /// Emitted once per `bash` call whose output the boundary filter (Arc A)
    /// shrank. `filter` is `"generic"` (phase-01 normalize+truncate) or
    /// `"cargo"` (phase-02 structured). Tokens reclaimed = `tokens_before -
    /// tokens_after` (chars/4 estimate, same heuristic as the budget).
    OutputFiltered {
        tokens_before: usize,
        tokens_after: usize,
        filter: String,
    },
    /// Emitted when a successful edit supersedes prior `read_file` results for a
    /// file (M10 Arc B). `reads_evicted` results were replaced by a re-read
    /// breadcrumb; `tokens_reclaimed` is the chars/4 estimate of context freed.
    ReadEvicted {
        path: String,
        reads_evicted: usize,
        tokens_reclaimed: usize,
    },
}
