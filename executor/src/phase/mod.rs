pub mod briefing;
pub mod result;

pub use briefing::{
    AttemptSummary, Blocker, Briefing, MAX_ATTEMPT_CHARS, MAX_WORKING_FILES, WorkingFile,
    collect_working_files, summarize_attempts,
};
pub use result::{
    Artifacts, CancelReason, Cancellation, CommandOutputs, FileChange, PhaseResult, PhaseStatus,
};
