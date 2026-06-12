//! The `execute_phase` turn loop — the full turn cycle. Drives the local model
//! through budget-bounded turns (chat → drain events → native-or-parsed `ToolCall`
//! → read-before-edit gate → dispatch → post-edit verify → hard-fail check),
//! writing a redacted session log throughout, and on termination returns a
//! `PhaseResult` (diff, files_changed, command_outputs, briefing) and emits a
//! `PhaseRun` telemetry record. Terminates `complete` (model stops calling tools),
//! `hard_fail` (a hard-fail signal), or `budget_exceeded` (turn/context cap).

pub mod command;
pub mod contract;
pub mod progress;
pub mod prompt;
pub mod verify;

mod log;
mod metrics;
mod outcome;
pub mod tasks;
mod tools;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tokio::sync::mpsc;
use tokio::time::interval;

use crate::ai::AiClient;
use crate::ai::types::{AiEvent, Message, ToolSchema};
use crate::config::{CommandConfig, GovernorConfig};
use crate::context::budget::Budget;
use crate::context::compactor::compact;
use crate::error::{Error, Result};
use crate::governor::hard_fail::{HardFailSignal, ToolCallSnapshot, evaluate};
use crate::governor::scorer::Scorer;
use crate::governor::verifier::{Baseline, Diagnostic, VerifierResult};
use crate::parser::{Origin, ParseResult, ToolCall, parse};
use crate::phase::{CommandOutputs, PhaseResult};
use crate::security::redact::Redactor;
use crate::store::sessions::event::SessionEvent;
use crate::store::sessions::jsonl::{SessionLogHandle, open_session_log};
use crate::store::telemetry::{Gates, GenerationParams};
use crate::tools::ToolRegistry;
use command::{CommandRunner, run_command_set, run_post_write_hooks};
use log::{log_event, log_session_end};
use metrics::{RunMetrics, emit_phase_run};
use outcome::{budget_exceeded_result, build_artifacts, hard_fail_result, turns_line};
use progress::{EmitCtx, ProgressCallback, emit_progress};
use tools::{
    append_tool_exchange, assistant_text, dispatch, edit_target, evict_superseded_reads,
    output_preview, read_before_edit_refusal, record_mtime, redundant_read_reference,
    render_diagnostics, resolve_path, user_text,
};
use verify::FileVerifier;

/// Heartbeat period (seconds) for re-emitting `awaiting_model` while the model
/// call is in flight. Keeps `rexymcp status`'s `last_ts` fresh during prefill.
const HEARTBEAT_PERIOD: std::time::Duration = std::time::Duration::from_secs(15);

/// The prompt inputs and verbatim phase metadata the loop assembles into the
/// system prompt and the escalation briefing.
pub struct PhaseInput {
    pub standards: String,
    pub phase_doc: String,
    pub goal: String,
    pub acceptance_criteria: String,
    /// Short phase identifier (e.g. `"phase-07b"`) — used for the `SessionStart`
    /// record and the session-log filename.
    pub phase: String,
    /// Phase-doc tags (language / kind / size) for the `PhaseRun` record.
    pub tags: Vec<String>,
    /// Full absolute path to the phase doc, recorded in `PhaseRun` for
    /// milestone-aware savings queries.
    pub phase_doc_path: String,
}

/// The injected dependencies the loop drives — explicit, no globals. The `clock`
/// is injected (no real `Utc::now()`); session logging reads its `ts` from it.
pub struct LoopDeps<'a> {
    pub client: &'a dyn AiClient,
    pub registry: &'a ToolRegistry,
    pub tools: &'a [ToolSchema],
    pub budget: &'a Budget,
    pub max_turns: usize,
    pub project_root: &'a Path,
    /// Model identifier, for the `SessionStart` record.
    pub model: &'a str,
    /// Caller-provided session id (M5 uses `generate_session_id()`); the loop
    /// never generates it.
    pub session_id: &'a str,
    /// Epoch-millis clock for session-log record timestamps.
    pub clock: &'a (dyn Fn() -> u64 + Send + Sync),
    /// Post-edit verifier (injected so tests need not spawn a real compiler).
    pub verifier: &'a dyn FileVerifier,
    /// Final command set (`fmt`/`build`/`lint`/`test`), run on clean completion.
    pub commands: &'a CommandConfig,
    /// Runner for the final command set (injected so tests need not spawn one).
    pub runner: &'a dyn CommandRunner,
    /// Generation knobs recorded in the `PhaseRun` (M5 populates; default here).
    pub generation_params: GenerationParams,
    /// Cross-project telemetry dir for the `PhaseRun` record; `None` disables it.
    pub telemetry_dir: Option<&'a Path>,
    /// Endpoint-reported model context window (`max_model_len`); `None` if unknown.
    pub context_window: Option<usize>,
    /// Optional liveness callback. `None` disables progress entirely (no
    /// callback invocations, no `Progress` log events, no numstat
    /// computation). Best-effort when `Some`: a callback that panics is
    /// outside this contract; the loop assumes the callback is safe.
    pub progress: Option<&'a dyn ProgressCallback>,
    /// Governor hard-fail thresholds — read from `[governor]` in rexymcp.toml.
    pub governor: GovernorConfig,
    /// Whether to seed + emit the M12 Arc A task list. Read from
    /// `[executor] task_tracking` (default true). Off → zero `TaskUpdate`
    /// events, byte-identical to pre-06a behavior.
    pub task_tracking: bool,
}

/// Run the turn cycle until the model stops calling tools (`complete`) or the
/// turn/context budget is exhausted (`budget_exceeded`). Backend/infra failures
/// surface as `Err`; model-visible outcomes (parse failures, unknown/failed
/// tools) are fed back into the conversation and never error.
pub async fn execute_phase(input: &PhaseInput, deps: LoopDeps<'_>) -> Result<PhaseResult> {
    let seeded: Vec<crate::agent::tasks::Task> = if deps.task_tracking {
        tasks::seed_from_spec(&input.phase_doc)
    } else {
        Vec::new()
    };
    let system = format!(
        "{}{}{}",
        prompt::datetime_header((deps.clock)()),
        prompt::assemble_system_prompt(deps.commands, &input.standards, &input.phase_doc),
        prompt::task_section(&seeded),
    );
    let tools_opt = if deps.tools.is_empty() {
        None
    } else {
        Some(deps.tools)
    };

    let mut messages: Vec<Message> = Vec::new();
    let mut scorer = Scorer::new();
    let mut recent_tool_calls: VecDeque<ToolCallSnapshot> = VecDeque::new();
    let mut turns: usize = 0;

    // Governor feedback state (07c).
    let mut baseline = Baseline::new();
    let mut baselined_exts: HashSet<String> = HashSet::new();
    let mut recent_verifier_error_counts: Vec<usize> = Vec::new();
    let mut last_author_diagnostics: Vec<Diagnostic> = Vec::new();

    // Read-before-edit working set (07d): resolved path → mtime at last read/edit.
    let mut working_set: HashMap<PathBuf, SystemTime> = HashMap::new();

    // Completion-artifact state (07e): pre-edit content of each file the model
    // edits, captured before the first edit lands — the "before" side of the diff.
    let mut pre_edit_content: HashMap<PathBuf, Option<String>> = HashMap::new();

    // Telemetry accumulation (08): metrics folded into the PhaseRun at terminal.
    let mut metrics = RunMetrics::started_at((deps.clock)());

    // Step 1 (observability) — open the session log. Best-effort: `.ok()` drops a
    // setup failure on purpose (a non-writable repo must not fail the phase —
    // logging is a side effect that never changes what the loop returns). The
    // composed id puts both phase and session_id in the filename.
    let redactor = Redactor::new();
    let log_dir = deps.project_root.join(".rexymcp").join("sessions");
    let log_handle: Option<SessionLogHandle> =
        open_session_log(&log_dir, &format!("{}-{}", input.phase, deps.session_id)).ok();
    let log_path = log_handle
        .as_ref()
        .and_then(|h| h.lock().ok().map(|l| l.path().to_path_buf()));

    log_event(
        &log_handle,
        &redactor,
        deps.clock,
        0,
        SessionEvent::SessionStart {
            session_id: deps.session_id.to_string(),
            model: deps.model.to_string(),
            phase: input.phase.clone(),
        },
    );
    log_event(
        &log_handle,
        &redactor,
        deps.clock,
        0,
        SessionEvent::Prompt {
            rendered: system.clone(),
        },
    );

    // Task-tracking substrate (M12 Arc A). Gated by [executor] task_tracking
    // (06b): off → no seeding, byte-identical to pre-06a.
    if deps.task_tracking {
        for task in &seeded {
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                0,
                SessionEvent::TaskUpdate {
                    id: task.id.clone(),
                    title: task.title.clone(),
                    state: task.state,
                },
            );
        }
        if seeded.is_empty() {
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                0,
                SessionEvent::Progress {
                    turn: 0,
                    stage: "task_seeding".to_string(),
                    files_changed: vec![],
                    message: "task tracking is on but 0 tasks were seeded \
                        from ## Spec — no `N. ` list items or `### N.` \
                        subheadings found; Tasks panel will be empty"
                        .to_string(),
                },
            );
        }
    }

    loop {
        // Step 2 — budget: compact on overflow, give up if still over.
        if deps.budget.would_overflow(&system, &messages) {
            let report = compact(&mut messages, deps.budget, &system);
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                turns,
                SessionEvent::Compaction {
                    tokens_before: report.tokens_before,
                    tokens_after: report.tokens_after,
                    messages_signaturized: report.messages_signaturized,
                    messages_evicted: report.messages_evicted,
                },
            );
            if deps.budget.would_overflow(&system, &messages) {
                log_session_end(&log_handle, &redactor, deps.clock, "budget_exceeded", turns);
                emit_phase_run(
                    &deps,
                    input,
                    "budget_exceeded",
                    Gates::default(),
                    &metrics,
                    &scorer,
                    turns,
                );
                let artifacts = build_artifacts(
                    &pre_edit_content,
                    deps.project_root,
                    log_path.clone(),
                    "budget_exceeded",
                    turns,
                    CommandOutputs::default(),
                );
                return Ok(budget_exceeded_result(
                    input,
                    &recent_tool_calls,
                    deps.project_root,
                    "context budget exhausted".to_string(),
                    artifacts,
                ));
            }
        }

        // Step 3 — call the model and drain its event stream for this turn.
        let upcoming_turn = turns + 1;
        let (tx, mut rx) = mpsc::unbounded_channel::<AiEvent>();

        // Emit awaiting_model before the call so rexymcp status flips off the
        // previous turn's stage immediately.
        {
            let emit = EmitCtx {
                progress: deps.progress,
                log_handle: &log_handle,
                redactor: &redactor,
                clock: deps.clock,
                pre_edit_content: &pre_edit_content,
                project_root: deps.project_root,
                turn: upcoming_turn,
            };
            emit_progress(&emit, "awaiting_model".to_string());
        }

        // Drive the chat future concurrently with a heartbeat interval. Each tick
        // re-emits awaiting_model so last_ts stays fresh during a slow prefill.
        let chat_fut = deps.client.chat(&system, messages.clone(), tx, tools_opt);
        tokio::pin!(chat_fut);
        let mut heartbeat = interval(HEARTBEAT_PERIOD);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                result = &mut chat_fut => {
                    match result {
                        Ok(()) => {}
                        Err(e) if turns == 0 => {
                            return Err(Error::Backend(e.to_string()))
                        }
                        Err(e) => {
                            let signal =
                                HardFailSignal::BackendError { message: e.to_string() };
                            log_event(
                                &log_handle,
                                &redactor,
                                deps.clock,
                                turns,
                                SessionEvent::HardFail {
                                    reason: signal.describe(),
                                },
                            );
                            log_session_end(
                                &log_handle,
                                &redactor,
                                deps.clock,
                                "hard_fail",
                                turns,
                            );
                            emit_phase_run(
                                &deps,
                                input,
                                "hard_fail",
                                Gates::default(),
                                &metrics,
                                &scorer,
                                turns,
                            );
                            let artifacts = build_artifacts(
                                &pre_edit_content,
                                deps.project_root,
                                log_path.clone(),
                                "hard_fail",
                                turns,
                                CommandOutputs::default(),
                            );
                            return Ok(hard_fail_result(
                                input,
                                &recent_tool_calls,
                                deps.project_root,
                                Vec::new(),
                                signal,
                                artifacts,
                            ));
                        }
                    }
                    break;
                }
                _ = heartbeat.tick() => {
                    let emit = EmitCtx {
                        progress: deps.progress,
                        log_handle: &log_handle,
                        redactor: &redactor,
                        clock: deps.clock,
                        pre_edit_content: &pre_edit_content,
                        project_root: deps.project_root,
                        turn: upcoming_turn,
                    };
                    emit_progress(&emit, "awaiting_model".to_string());
                }
            }
        }

        let mut completion = String::new();
        let mut native_call: Option<ToolCall> = None;
        while let Some(event) = rx.recv().await {
            match event {
                AiEvent::Token(s) => completion.push_str(&s),
                AiEvent::ToolCallGeneric { name, args, .. } => {
                    if native_call.is_none() {
                        native_call = Some(ToolCall {
                            name,
                            arguments: args,
                            origin: Origin::Native,
                        });
                    }
                }
                AiEvent::Done(breakdown) => metrics.add_tokens(&breakdown),
                AiEvent::Completion {
                    finish_reason,
                    model,
                } => {
                    if let Some(m) = model {
                        metrics.served_model = Some(m);
                    }
                    if let Some(fr) = finish_reason {
                        metrics.total_finishes += 1;
                        if fr == "length" {
                            metrics.length_finishes += 1;
                        }
                    }
                }
                AiEvent::Error(e) => {
                    if turns == 0 {
                        log_session_end(&log_handle, &redactor, deps.clock, "error", turns);
                        return Err(Error::Backend(e));
                    }
                    let signal = HardFailSignal::BackendError { message: e.clone() };
                    log_event(
                        &log_handle,
                        &redactor,
                        deps.clock,
                        turns,
                        SessionEvent::HardFail {
                            reason: signal.describe(),
                        },
                    );
                    log_session_end(&log_handle, &redactor, deps.clock, "hard_fail", turns);
                    emit_phase_run(
                        &deps,
                        input,
                        "hard_fail",
                        Gates::default(),
                        &metrics,
                        &scorer,
                        turns,
                    );
                    let artifacts = build_artifacts(
                        &pre_edit_content,
                        deps.project_root,
                        log_path.clone(),
                        "hard_fail",
                        turns,
                        CommandOutputs::default(),
                    );
                    return Ok(hard_fail_result(
                        input,
                        &recent_tool_calls,
                        deps.project_root,
                        Vec::new(),
                        signal,
                        artifacts,
                    ));
                }
            }
        }
        turns += 1;
        {
            let emit = EmitCtx {
                progress: deps.progress,
                log_handle: &log_handle,
                redactor: &redactor,
                clock: deps.clock,
                pre_edit_content: &pre_edit_content,
                project_root: deps.project_root,
                turn: turns,
            };
            emit_progress(&emit, "turn_start".to_string());
        }
        log_event(
            &log_handle,
            &redactor,
            deps.clock,
            turns,
            SessionEvent::Completion {
                raw: completion.clone(),
            },
        );

        // Per-turn resource snapshot: cumulative tokens + context budget fraction.
        log_event(
            &log_handle,
            &redactor,
            deps.clock,
            turns,
            SessionEvent::Metrics {
                input_tokens: metrics.tokens.input_tokens,
                output_tokens: metrics.tokens.output_tokens,
                context_pct: deps.budget.fraction_used(&system, &messages),
                context_used: deps
                    .budget
                    .estimate(&system, &messages)
                    .min(u32::MAX as usize) as u32,
                context_window: if deps.budget.ceiling == usize::MAX {
                    0
                } else {
                    deps.budget.ceiling.min(u32::MAX as usize) as u32
                },
            },
        );

        // Step 4 — turn the output into a ToolCall (native event wins; otherwise
        // run the forgiving text parser).
        let tool_call = if let Some(tc) = native_call {
            tc
        } else {
            metrics.parse_attempts += 1;
            match parse(&completion, deps.registry) {
                ParseResult::NoToolCall => {
                    // A completion that is *only* a <think> block (empty after
                    // stripping) is not a clean exit — the model reasoned but
                    // emitted no action. Treat it as a recoverable parse failure
                    // so it gets feedback to emit a tool call. bug-executor-1.
                    let post_think = crate::parser::strip_think_blocks(&completion);
                    if post_think.trim().is_empty() && completion.contains("</think>") {
                        metrics.parse_failures += 1;
                        let failure = crate::parser::ParseFailure {
                            raw: completion.clone(),
                            detected_format: None,
                            candidates: vec![],
                            feedback: crate::parser::feedback::format_no_match(&completion),
                        };
                        log_event(
                            &log_handle,
                            &redactor,
                            deps.clock,
                            turns,
                            SessionEvent::ParseFailed {
                                failure: failure.clone(),
                            },
                        );
                        messages.push(assistant_text(&completion, turns));
                        messages.push(user_text(&failure.feedback, turns));
                        if turns >= deps.max_turns {
                            log_session_end(
                                &log_handle,
                                &redactor,
                                deps.clock,
                                "budget_exceeded",
                                turns,
                            );
                            emit_phase_run(
                                &deps,
                                input,
                                "budget_exceeded",
                                Gates::default(),
                                &metrics,
                                &scorer,
                                turns,
                            );
                            let artifacts = build_artifacts(
                                &pre_edit_content,
                                deps.project_root,
                                log_path.clone(),
                                "budget_exceeded",
                                turns,
                                CommandOutputs::default(),
                            );
                            return Ok(budget_exceeded_result(
                                input,
                                &recent_tool_calls,
                                deps.project_root,
                                turns_line(deps.max_turns),
                                artifacts,
                            ));
                        }
                        continue;
                    }
                    log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
                    // Step 8 — clean completion runs the final command set.
                    let emit = EmitCtx {
                        progress: deps.progress,
                        log_handle: &log_handle,
                        redactor: &redactor,
                        clock: deps.clock,
                        pre_edit_content: &pre_edit_content,
                        project_root: deps.project_root,
                        turn: turns,
                    };
                    let (command_outputs, gates) =
                        run_command_set(deps.runner, deps.commands, deps.project_root, &emit).await;
                    emit_phase_run(&deps, input, "complete", gates, &metrics, &scorer, turns);
                    let artifacts = build_artifacts(
                        &pre_edit_content,
                        deps.project_root,
                        log_path.clone(),
                        "complete",
                        turns,
                        command_outputs,
                    );
                    return Ok(PhaseResult::complete(artifacts));
                }
                ParseResult::Found(tc) => tc,
                ParseResult::Failed(failure) => {
                    metrics.parse_failures += 1;
                    log_event(
                        &log_handle,
                        &redactor,
                        deps.clock,
                        turns,
                        SessionEvent::ParseFailed {
                            failure: failure.clone(),
                        },
                    );
                    messages.push(assistant_text(&completion, turns));
                    messages.push(user_text(&failure.feedback, turns));
                    if turns >= deps.max_turns {
                        log_session_end(
                            &log_handle,
                            &redactor,
                            deps.clock,
                            "budget_exceeded",
                            turns,
                        );
                        emit_phase_run(
                            &deps,
                            input,
                            "budget_exceeded",
                            Gates::default(),
                            &metrics,
                            &scorer,
                            turns,
                        );
                        let artifacts = build_artifacts(
                            &pre_edit_content,
                            deps.project_root,
                            log_path.clone(),
                            "budget_exceeded",
                            turns,
                            CommandOutputs::default(),
                        );
                        return Ok(budget_exceeded_result(
                            input,
                            &recent_tool_calls,
                            deps.project_root,
                            turns_line(deps.max_turns),
                            artifacts,
                        ));
                    }
                    continue;
                }
            }
        };
        log_event(
            &log_handle,
            &redactor,
            deps.clock,
            turns,
            SessionEvent::Parsed {
                tool_call: tool_call.clone(),
            },
        );

        // An edit-class call's target path — resolved here (pre-dispatch) so the
        // baseline can be captured *before* the model's edit lands. Otherwise
        // `capture_baseline` would record the model's own new errors as ambient.
        let edit_path = edit_target(&tool_call, deps.project_root);

        // Step 4.5 — read-before-edit gate (07d). A refusal short-circuits the
        // edit: no baseline, no dispatch, no verify — but it is still a
        // model-visible failure that feeds back and counts toward hard-fail.
        // Redundant-read dedupe (M10 Arc B): a `read_file` of an unchanged file
        // whose content is still live in context returns a compact reference
        // instead of re-injecting it — reclaims context and attacks the
        // IdenticalToolCallRepetition stall. Safe: declines unless the mtime
        // matches AND a live prior whole-file read survives; ranged / force:true
        // reads always fall through to a real read.
        let dedupe =
            redundant_read_reference(&tool_call, &messages, &working_set, deps.project_root);

        let (succeeded, content, tool_meta) = if let Some((reference, _, _)) = &dedupe {
            (true, reference.clone(), None)
        } else {
            match read_before_edit_refusal(&tool_call, &working_set, deps.project_root) {
                Some(refusal) => (false, refusal, None),
                None => {
                    if let Some(path) = &edit_path
                        && let Some(ext) = path.extension().and_then(|e| e.to_str())
                        && !baselined_exts.contains(ext)
                    {
                        let captured = deps
                            .verifier
                            .capture_baseline(std::slice::from_ref(path))
                            .await;
                        baseline.signatures.extend(captured.signatures);
                        baselined_exts.insert(ext.to_string());
                    }
                    // 07e — capture the file's pre-edit content (the "before" side
                    // of the diff) the first time it is edited, before the edit lands.
                    if let Some(path) = &edit_path
                        && !pre_edit_content.contains_key(path)
                    {
                        pre_edit_content.insert(path.clone(), std::fs::read_to_string(path).ok());
                    }
                    // Step 5 — dispatch (native and text share this path).
                    {
                        let emit = EmitCtx {
                            progress: deps.progress,
                            log_handle: &log_handle,
                            redactor: &redactor,
                            clock: deps.clock,
                            pre_edit_content: &pre_edit_content,
                            project_root: deps.project_root,
                            turn: turns,
                        };
                        emit_progress(&emit, format!("tool:{}", tool_call.name));
                    }
                    dispatch(deps.registry, &tool_call).await
                }
            }
        };
        log_event(
            &log_handle,
            &redactor,
            deps.clock,
            turns,
            SessionEvent::ToolResult {
                name: tool_call.name.clone(),
                succeeded,
                output_preview: output_preview(&content),
            },
        );
        scorer.record(&tool_call.name, succeeded);
        metrics.total_calls += 1;
        if let Origin::Repaired { repairs, .. } = &tool_call.origin {
            metrics.total_repairs += repairs.len();
        }
        recent_tool_calls.push_back(ToolCallSnapshot {
            tool: tool_call.name.clone(),
            arguments: tool_call.arguments.clone(),
            succeeded,
        });
        append_tool_exchange(&mut messages, &tool_call, &content, turns);

        // Per-lever reclaim event (M10 Arc A): record how much the boundary
        // filter shrank this bash call's output. Emit only on a real reduction.
        if let Some(meta) = &tool_meta
            && let Some(of) = meta.get("output_filter")
            && let (Some(before), Some(after), Some(filter)) = (
                of.get("tokens_before").and_then(|v| v.as_u64()),
                of.get("tokens_after").and_then(|v| v.as_u64()),
                of.get("filter").and_then(|v| v.as_str()),
            )
            && after < before
        {
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                turns,
                SessionEvent::OutputFiltered {
                    tokens_before: before as usize,
                    tokens_after: after as usize,
                    filter: filter.to_string(),
                },
            );
        }

        // Model-driven task flip (M12 Arc A / phase-06c): the update_task tool
        // reports the flip in metadata; transcribe it to a TaskUpdate event.
        if let Some(meta) = &tool_meta
            && let Some(tu) = meta.get("task_update")
            && let (Some(id), Some(title)) = (
                tu.get("id").and_then(|v| v.as_str()),
                tu.get("title").and_then(|v| v.as_str()),
            )
            && let Some(state) = tu.get("state").and_then(|v| {
                serde_json::from_value::<crate::store::sessions::event::TaskState>(v.clone()).ok()
            })
        {
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                turns,
                SessionEvent::TaskUpdate {
                    id: id.to_string(),
                    title: title.to_string(),
                    state,
                },
            );
        }

        // Record the working set: a read makes a file patch-eligible; a successful
        // patch refreshes its mtime so a follow-up patch needs no re-read.
        if succeeded
            && (tool_call.name == "read_file" || tool_call.name == "patch")
            && let Some(path) = resolve_path(&tool_call, deps.project_root)
        {
            record_mtime(&mut working_set, &path);
        }

        // Superseded-read eviction (M10 Arc B): a successful edit makes every
        // prior read of this file stale. Replace those read results with a
        // re-read breadcrumb to reclaim context and remove the stale-content
        // hazard. Always safe — the read-before-edit gate forces a re-read.
        if succeeded && let Some(path) = &edit_path {
            let (reads_evicted, tokens_reclaimed) =
                evict_superseded_reads(&mut messages, path, turns, deps.project_root);
            if reads_evicted > 0 {
                log_event(
                    &log_handle,
                    &redactor,
                    deps.clock,
                    turns,
                    SessionEvent::ReadEvicted {
                        path: path.display().to_string(),
                        reads_evicted,
                        tokens_reclaimed,
                    },
                );
            }
        }

        // Per-lever reclaim event (M10 Arc B): record the deduped re-read.
        if let Some((_, tokens_saved, prior_turn)) = dedupe
            && let Some(path) = resolve_path(&tool_call, deps.project_root)
        {
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                turns,
                SessionEvent::ReadDeduped {
                    path: path.display().to_string(),
                    tokens_saved,
                    prior_turn,
                },
            );
        }

        // Post-write format hook (M9/phase-01). Runs the configured format
        // command after every successful edit-class turn, before the verifier,
        // so the on-disk file is always formatted when verify reads it.
        if succeeded
            && edit_path.is_some()
            && (deps.commands.format.is_some() || deps.commands.lint_fix.is_some())
        {
            {
                let emit = EmitCtx {
                    progress: deps.progress,
                    log_handle: &log_handle,
                    redactor: &redactor,
                    clock: deps.clock,
                    pre_edit_content: &pre_edit_content,
                    project_root: deps.project_root,
                    turn: turns,
                };
                emit_progress(&emit, "format".to_string());
            }
            run_post_write_hooks(deps.runner, deps.commands, deps.project_root).await;
        }

        // Step 6 — post-edit verify + retry feedback. Only for a successful
        // edit-class call (verifying after a failed edit is noise).
        if succeeded && let Some(path) = &edit_path {
            {
                let emit = EmitCtx {
                    progress: deps.progress,
                    log_handle: &log_handle,
                    redactor: &redactor,
                    clock: deps.clock,
                    pre_edit_content: &pre_edit_content,
                    project_root: deps.project_root,
                    turn: turns,
                };
                emit_progress(&emit, "verify".to_string());
            }
            match deps.verifier.verify(path).await {
                VerifierResult::Checked { diagnostics } => {
                    let (author, _ambient) = baseline.partition(&diagnostics);
                    let author: Vec<Diagnostic> = author.into_iter().cloned().collect();
                    log_event(
                        &log_handle,
                        &redactor,
                        deps.clock,
                        turns,
                        SessionEvent::Verify {
                            diagnostics: author.clone(),
                        },
                    );
                    recent_verifier_error_counts.push(author.len());
                    if author.is_empty() {
                        last_author_diagnostics.clear();
                    } else {
                        metrics.verifier_retries += 1;
                        messages.push(user_text(&render_diagnostics(&author), turns));
                        last_author_diagnostics = author;
                    }
                }
                VerifierResult::Unsupported => {}
                VerifierResult::Failed(msg) => {
                    messages.push(user_text(&format!("verifier failed: {msg}"), turns));
                }
                VerifierResult::Skipped(msg) => {
                    messages.push(user_text(&format!("verifier skipped: {msg}"), turns));
                }
            }
        }

        // Step 7 — hard-fail detection (repetition / persistent verifier failure /
        // runaway output). Checked before the turn cap so the specific cause wins.
        if let Some(signal) = evaluate(
            &recent_tool_calls,
            &recent_verifier_error_counts,
            Some((&tool_call.name, content.len())),
            &deps.governor,
        ) {
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                turns,
                SessionEvent::HardFail {
                    reason: signal.describe(),
                },
            );
            log_session_end(&log_handle, &redactor, deps.clock, "hard_fail", turns);
            emit_phase_run(
                &deps,
                input,
                "hard_fail",
                Gates::default(),
                &metrics,
                &scorer,
                turns,
            );
            let artifacts = build_artifacts(
                &pre_edit_content,
                deps.project_root,
                log_path.clone(),
                "hard_fail",
                turns,
                CommandOutputs::default(),
            );
            return Ok(hard_fail_result(
                input,
                &recent_tool_calls,
                deps.project_root,
                last_author_diagnostics,
                signal,
                artifacts,
            ));
        }

        // Step 9 — turn cap.
        if turns >= deps.max_turns {
            log_session_end(&log_handle, &redactor, deps.clock, "budget_exceeded", turns);
            emit_phase_run(
                &deps,
                input,
                "budget_exceeded",
                Gates::default(),
                &metrics,
                &scorer,
                turns,
            );
            let artifacts = build_artifacts(
                &pre_edit_content,
                deps.project_root,
                log_path.clone(),
                "budget_exceeded",
                turns,
                CommandOutputs::default(),
            );
            return Ok(budget_exceeded_result(
                input,
                &recent_tool_calls,
                deps.project_root,
                turns_line(deps.max_turns),
                artifacts,
            ));
        }
    }
}

#[cfg(test)]
mod tests;
