use rexymcp_executor::store::sessions::event::SessionEvent;

pub(crate) const FILTER_ITEM_COUNT: usize = 14;

/// Per-event-type visibility toggles for the Activity pane.
/// All enabled by default except `progress` (too noisy).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ActivityFilter {
    pub(crate) session: bool,
    pub(crate) prompt: bool,
    pub(crate) completion: bool,
    pub(crate) tool_call: bool,
    pub(crate) parse_failed: bool,
    pub(crate) verify: bool,
    pub(crate) hard_fail: bool,
    pub(crate) progress: bool,
    pub(crate) metrics: bool,
    pub(crate) compaction: bool,
    pub(crate) output_filtered: bool,
    pub(crate) read_evicted: bool,
    pub(crate) read_deduped: bool,
    pub(crate) task_update: bool,
}

impl Default for ActivityFilter {
    fn default() -> Self {
        Self {
            session: true,
            prompt: true,
            completion: true,
            tool_call: true,
            parse_failed: true,
            verify: true,
            hard_fail: true,
            progress: false,
            metrics: true,
            compaction: true,
            output_filtered: true,
            read_evicted: true,
            read_deduped: true,
            task_update: true,
        }
    }
}

impl ActivityFilter {
    pub(crate) fn allows(&self, event: &SessionEvent) -> bool {
        match event {
            SessionEvent::SessionStart { .. } | SessionEvent::SessionEnd { .. } => self.session,
            SessionEvent::Prompt { .. } => self.prompt,
            SessionEvent::Completion { .. } => self.completion,
            SessionEvent::Parsed { .. } => self.tool_call,
            SessionEvent::ParseFailed { .. } => self.parse_failed,
            SessionEvent::ToolResult { .. } => self.tool_call,
            SessionEvent::Verify { .. } => self.verify,
            SessionEvent::HardFail { .. } => self.hard_fail,
            SessionEvent::Progress { .. } => self.progress,
            SessionEvent::Metrics { .. } => self.metrics,
            SessionEvent::Compaction { .. } => self.compaction,
            SessionEvent::OutputFiltered { .. } => self.output_filtered,
            SessionEvent::ReadEvicted { .. } => self.read_evicted,
            SessionEvent::ReadDeduped { .. } => self.read_deduped,
            SessionEvent::TaskUpdate { .. } => self.task_update,
        }
    }

    pub(crate) fn toggle(&mut self, index: usize) {
        match index {
            0 => self.session = !self.session,
            1 => self.prompt = !self.prompt,
            2 => self.completion = !self.completion,
            3 => self.tool_call = !self.tool_call,
            4 => self.parse_failed = !self.parse_failed,
            5 => self.verify = !self.verify,
            6 => self.hard_fail = !self.hard_fail,
            7 => self.progress = !self.progress,
            8 => self.metrics = !self.metrics,
            9 => self.compaction = !self.compaction,
            10 => self.output_filtered = !self.output_filtered,
            11 => self.read_evicted = !self.read_evicted,
            12 => self.read_deduped = !self.read_deduped,
            13 => self.task_update = !self.task_update,
            _ => {}
        }
    }

    pub(crate) fn is_enabled(&self, index: usize) -> bool {
        match index {
            0 => self.session,
            1 => self.prompt,
            2 => self.completion,
            3 => self.tool_call,
            4 => self.parse_failed,
            5 => self.verify,
            6 => self.hard_fail,
            7 => self.progress,
            8 => self.metrics,
            9 => self.compaction,
            10 => self.output_filtered,
            11 => self.read_evicted,
            12 => self.read_deduped,
            13 => self.task_update,
            _ => false,
        }
    }

    pub(crate) fn item_label(index: usize) -> &'static str {
        match index {
            0 => "session start/end",
            1 => "prompt",
            2 => "completion",
            3 => "tool call",
            4 => "parse fail",
            5 => "verify",
            6 => "hard fail",
            7 => "progress",
            8 => "metrics",
            9 => "compaction",
            10 => "output filtered",
            11 => "read evicted",
            12 => "read deduped",
            13 => "task update",
            _ => "?",
        }
    }
}

/// Filter panel UI state — open/closed, cursor position, current settings.
#[derive(Clone, Debug, Default)]
pub(crate) struct FilterState {
    pub(crate) open: bool,
    pub(crate) cursor: usize,
    pub(crate) filter: ActivityFilter,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};

    fn rec(ts: u64, turn: usize, event: SessionEvent) -> SessionRecord {
        SessionRecord { ts, turn, event }
    }

    fn progress_event(turn: usize, stage: &str) -> SessionEvent {
        SessionEvent::Progress {
            turn,
            stage: stage.into(),
            files_changed: vec![],
            message: format!("turn={turn} stage={stage} +0/-0 files=0"),
        }
    }

    #[test]
    fn filter_default_disables_progress() {
        let f = ActivityFilter::default();
        assert!(!f.progress, "progress should be disabled by default");
        assert!(f.session);
        assert!(f.prompt);
        assert!(f.completion);
        assert!(f.tool_call);
        assert!(f.parse_failed);
        assert!(f.verify);
        assert!(f.hard_fail);
        assert!(f.metrics);
        assert!(f.compaction);
        assert!(f.output_filtered);
        assert!(f.read_evicted);
        assert!(f.read_deduped);
        assert!(f.task_update);
    }

    #[test]
    fn filter_allows_progress_when_enabled() {
        let f = ActivityFilter {
            progress: true,
            ..Default::default()
        };
        let progress_rec = rec(100, 4, progress_event(4, "verify"));
        assert!(f.allows(&progress_rec.event));
    }

    #[test]
    fn filter_blocks_progress_by_default() {
        let f = ActivityFilter::default();
        let progress_rec = rec(100, 4, progress_event(4, "verify"));
        assert!(!f.allows(&progress_rec.event));
    }

    #[test]
    fn filter_toggle_flips_field() {
        let mut f = ActivityFilter::default();
        assert!(!f.progress);
        f.toggle(7); // progress is now index 7
        assert!(f.progress);
        f.toggle(7);
        assert!(!f.progress);
    }

    #[test]
    fn filter_cursor_wraps_forward() {
        let mut fs = FilterState::default();
        fs.cursor = FILTER_ITEM_COUNT - 1;
        fs.cursor = (fs.cursor + 1) % FILTER_ITEM_COUNT;
        assert_eq!(fs.cursor, 0);
    }

    #[test]
    fn filter_cursor_wraps_backward() {
        let mut fs = FilterState::default();
        fs.cursor = 0;
        fs.cursor = (fs.cursor + FILTER_ITEM_COUNT - 1) % FILTER_ITEM_COUNT;
        assert_eq!(fs.cursor, FILTER_ITEM_COUNT - 1);
    }

    #[test]
    fn filter_merges_tool_result_into_tool_call() {
        let parsed = SessionEvent::Parsed {
            tool_call: rexymcp_executor::parser::ToolCall {
                name: "read_file".into(),
                arguments: serde_json::json!({}),
                origin: rexymcp_executor::parser::Origin::Native,
            },
        };
        let result = SessionEvent::ToolResult {
            name: "read_file".into(),
            succeeded: true,
            output_preview: "content".into(),
        };

        // With tool_call off, both Parsed and ToolResult are hidden
        let f = ActivityFilter {
            tool_call: false,
            ..Default::default()
        };
        assert!(
            !f.allows(&parsed),
            "Parsed should be hidden when tool_call is false"
        );
        assert!(
            !f.allows(&result),
            "ToolResult should be hidden when tool_call is false"
        );

        // With tool_call on, both are shown
        let f = ActivityFilter {
            tool_call: true,
            ..Default::default()
        };
        assert!(
            f.allows(&parsed),
            "Parsed should be shown when tool_call is true"
        );
        assert!(
            f.allows(&result),
            "ToolResult should be shown when tool_call is true"
        );
    }

    #[test]
    fn filter_has_no_tool_result_item() {
        assert_eq!(FILTER_ITEM_COUNT, 14);
        for i in 0..FILTER_ITEM_COUNT {
            assert_ne!(
                ActivityFilter::item_label(i),
                "tool result",
                "index {i} should not be 'tool result'"
            );
        }
        assert_eq!(ActivityFilter::item_label(3), "tool call");
        assert_eq!(ActivityFilter::item_label(7), "progress");
    }
}
