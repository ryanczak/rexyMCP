use crate::ai::types::TokenBreakdown;
use crate::governor::scorer::Scorer;
use crate::store::sessions::jsonl::read_session_log;
use crate::store::telemetry::{self, Gates, PhaseRun};

use super::{LoopDeps, PhaseInput};

pub(super) struct RunMetrics {
    pub(super) parse_attempts: usize,
    pub(super) parse_failures: usize,
    pub(super) total_repairs: usize,
    pub(super) total_calls: usize,
    pub(super) verifier_retries: usize,
    pub(super) tokens: TokenBreakdown,
    pub(super) start_ms: u64,
    pub(super) served_model: Option<String>,
    pub(super) length_finishes: usize,
    pub(super) total_finishes: usize,
}

impl RunMetrics {
    pub(super) fn started_at(start_ms: u64) -> Self {
        Self {
            parse_attempts: 0,
            parse_failures: 0,
            total_repairs: 0,
            total_calls: 0,
            verifier_retries: 0,
            tokens: TokenBreakdown::default(),
            start_ms,
            served_model: None,
            length_finishes: 0,
            total_finishes: 0,
        }
    }

    pub(super) fn add_tokens(&mut self, b: &TokenBreakdown) {
        self.tokens.input_tokens = self.tokens.input_tokens.saturating_add(b.input_tokens);
        self.tokens.output_tokens = self.tokens.output_tokens.saturating_add(b.output_tokens);
        self.tokens.cache_read_tokens = self
            .tokens
            .cache_read_tokens
            .saturating_add(b.cache_read_tokens);
        self.tokens.cache_write_tokens = self
            .tokens
            .cache_write_tokens
            .saturating_add(b.cache_write_tokens);
    }
}

/// Build and append (best-effort) the per-phase `PhaseRun` telemetry record.
/// `tool_success_rate` is computed from the loop's `Scorer` — the consumer that
/// makes `scorer.record` load-bearing. A `None` telemetry dir or a write error is
/// swallowed: telemetry, like the session log, never changes what the loop returns.
pub(super) fn emit_phase_run(
    deps: &LoopDeps<'_>,
    input: &PhaseInput,
    status: &str,
    gates: Gates,
    metrics: &RunMetrics,
    scorer: &Scorer,
    turns: usize,
) {
    let Some(dir) = deps.telemetry_dir else {
        return;
    };

    let (mut successes, mut total) = (0u64, 0u64);
    for counts in scorer.counts.values() {
        successes += counts.successes as u64;
        total += counts.successes as u64 + counts.failures as u64;
    }
    let tool_success_rate = if total > 0 {
        successes as f64 / total as f64
    } else {
        0.0
    };
    let parse_failure_rate = if metrics.parse_attempts > 0 {
        metrics.parse_failures as f64 / metrics.parse_attempts as f64
    } else {
        0.0
    };
    let repairs_per_call = if metrics.total_calls > 0 {
        metrics.total_repairs as f64 / metrics.total_calls as f64
    } else {
        0.0
    };
    let now = (deps.clock)();
    let wall_clock_s = now.saturating_sub(metrics.start_ms) as f64 / 1000.0;

    // Aggregate the context-efficiency signal from the durable session log the
    // loop just wrote. Best-effort: a missing/unreadable log yields the default
    // (all zeros) — telemetry never fails the phase. The path must mirror what
    // `execute_phase` passed to `open_session_log` (see that call + `SessionLogger::open`).
    let log_path = deps
        .project_root
        .join(".rexymcp")
        .join("sessions")
        .join(format!("session-{}-{}.jsonl", input.phase, deps.session_id));
    let context_efficiency = read_session_log(&log_path)
        .map(|recs| telemetry::aggregate_context_efficiency(&recs))
        .unwrap_or_default();

    let run = PhaseRun {
        ts: now,
        model: deps.model.to_string(),
        generation_params: deps.generation_params.clone(),
        phase_id: input.phase.clone(),
        phase_doc_path: Some(input.phase_doc_path.clone()),
        tags: input.tags.clone(),
        status: status.to_string(),
        escalated: status != "complete",
        gates,
        parse_failure_rate,
        repairs_per_call,
        verifier_retries: metrics.verifier_retries,
        tool_success_rate,
        turns,
        wall_clock_s,
        tokens: metrics.tokens.clone(),
        warnings: None,
        bugs_filed: None,
        bounces_to_approval: None,
        architect_verdict: None,
        served_model: metrics.served_model.clone(),
        length_finish_rate: (metrics.total_finishes > 0)
            .then(|| metrics.length_finishes as f64 / metrics.total_finishes as f64),
        context_window: deps.context_window,
        context_efficiency,
    };
    let _ = telemetry::append(dir, &run);
}
