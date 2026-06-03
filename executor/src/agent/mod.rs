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

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use similar::{ChangeTag, TextDiff};
use tokio::sync::mpsc;
use tokio::time::interval;

use crate::ai::AiClient;
use crate::ai::next_tool_id;
use crate::ai::types::{
    AiEvent, Message, TokenBreakdown, ToolCall as AiToolCall, ToolResult as AiToolResult,
    ToolSchema,
};
use crate::config::CommandConfig;
use crate::context::budget::Budget;
use crate::context::compactor::compact;
use crate::error::{Error, Result};
use crate::governor::hard_fail::{HardFailSignal, ToolCallSnapshot, evaluate};
use crate::governor::scorer::Scorer;
use crate::governor::verifier::{Baseline, Diagnostic, Severity, VerifierResult};
use crate::parser::{Origin, ParseResult, ToolCall, parse};
use crate::phase::{
    Artifacts, Blocker, Briefing, CommandOutputs, FileChange, PhaseResult, collect_working_files,
    summarize_attempts,
};
use crate::security::redact::Redactor;
use crate::store::sessions::event::SessionEvent;
use crate::store::sessions::jsonl::{SessionLogHandle, open_session_log, session_log};
use crate::store::telemetry::{self, Gates, GenerationParams, PhaseRun};
use crate::tools::ToolRegistry;
use command::{CommandResult, CommandRunner};
use progress::{ProgressCallback, ProgressEvent};
use verify::FileVerifier;

/// Preview cap for a tool result's `output_preview` in the session log — enough
/// to triage a failure, not the full (possibly huge) output.
const OUTPUT_PREVIEW_CHARS: usize = 500;

/// Cap on the combined unified diff returned in `PhaseResult.diff`.
const MAX_DIFF_CHARS: usize = 50_000;

/// Tail cap on each captured final-command-set output.
const MAX_COMMAND_TAIL_CHARS: usize = 4_000;

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
}

/// Run the turn cycle until the model stops calling tools (`complete`) or the
/// turn/context budget is exhausted (`budget_exceeded`). Backend/infra failures
/// surface as `Err`; model-visible outcomes (parse failures, unknown/failed
/// tools) are fed back into the conversation and never error.
pub async fn execute_phase(input: &PhaseInput, deps: LoopDeps<'_>) -> Result<PhaseResult> {
    let system = prompt::assemble_system_prompt(deps.commands, &input.standards, &input.phase_doc);
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
        let (succeeded, content) =
            match read_before_edit_refusal(&tool_call, &working_set, deps.project_root) {
                Some(refusal) => (false, refusal),
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

        // Record the working set: a read makes a file patch-eligible; a successful
        // patch refreshes its mtime so a follow-up patch needs no re-read.
        if succeeded
            && (tool_call.name == "read_file" || tool_call.name == "patch")
            && let Some(path) = resolve_path(&tool_call, deps.project_root)
        {
            record_mtime(&mut working_set, &path);
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
            }
        }

        // Step 7 — hard-fail detection (repetition / persistent verifier failure /
        // runaway output). Checked before the turn cap so the specific cause wins.
        if let Some(signal) = evaluate(
            &recent_tool_calls,
            &recent_verifier_error_counts,
            Some((&tool_call.name, content.len())),
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

/// Redact an event (round-tripping its JSON through the redactor so every string
/// field is covered) and write it best-effort. A `None` handle (the log failed to
/// open) is a silent no-op — logging never changes loop behavior.
fn log_event(
    handle: &Option<SessionLogHandle>,
    redactor: &Redactor,
    clock: &dyn Fn() -> u64,
    turn: usize,
    event: SessionEvent,
) {
    let Some(handle) = handle else {
        return;
    };
    session_log(handle, clock(), turn, redact_event(redactor, event));
}

fn log_session_end(
    handle: &Option<SessionLogHandle>,
    redactor: &Redactor,
    clock: &dyn Fn() -> u64,
    status: &str,
    turns: usize,
) {
    log_event(
        handle,
        redactor,
        clock,
        turns,
        SessionEvent::SessionEnd {
            status: status.to_string(),
            turns,
        },
    );
}

/// Round-trip an event through the redactor: serialize → redact the JSON →
/// deserialize. This redacts every string the event carries (prompt, completion,
/// tool output, the nested `ParseFailure` / `ToolCall` payloads) in one pass; the
/// `[REDACTED:<kind>]` markers are JSON-safe, so the parse round-trips. On the
/// can't-happen serde failure, fall back to the un-redacted event's structure
/// only after redaction was attempted — but serialization of these types is
/// effectively infallible, so this is a safety net, not a swallow.
fn redact_event(redactor: &Redactor, event: SessionEvent) -> SessionEvent {
    let Ok(json) = serde_json::to_string(&event) else {
        return event;
    };
    let redacted = redactor.redact(&json);
    serde_json::from_str(&redacted).unwrap_or(event)
}

fn output_preview(content: &str) -> String {
    if content.chars().count() > OUTPUT_PREVIEW_CHARS {
        content.chars().take(OUTPUT_PREVIEW_CHARS).collect()
    } else {
        content.to_string()
    }
}

/// Resolve a tool call's `"path"` argument against the project root. `None` if
/// the call has no string `"path"`.
fn resolve_path(tool_call: &ToolCall, project_root: &Path) -> Option<PathBuf> {
    let path = PathBuf::from(tool_call.arguments.get("path").and_then(|v| v.as_str())?);
    Some(if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    })
}

/// The file an edit-class (`write_file` / `patch`) call targets, resolved against
/// the project root. `None` for non-edit calls or calls missing a `"path"` arg.
fn edit_target(tool_call: &ToolCall, project_root: &Path) -> Option<PathBuf> {
    if tool_call.name != "write_file" && tool_call.name != "patch" {
        return None;
    }
    resolve_path(tool_call, project_root)
}

/// The read-before-edit gate (07d). Refuse a `patch` on a file the model has not
/// read this session, or one whose on-disk mtime no longer matches what was read.
/// `None` = allowed. Pure over `working_set` so the mtime-mismatch case is
/// unit-testable without mid-session filesystem hooks. `patch`-only — `write_file`
/// (whole-file create/overwrite) is not gated.
fn read_before_edit_refusal(
    tool_call: &ToolCall,
    working_set: &HashMap<PathBuf, SystemTime>,
    project_root: &Path,
) -> Option<String> {
    if tool_call.name != "patch" {
        return None;
    }
    let path = resolve_path(tool_call, project_root)?;
    match working_set.get(&path) {
        None => Some(format!(
            "refusing to patch {}: you have not read it this session. Use read_file on it first.",
            path.display()
        )),
        Some(recorded) => {
            let current = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
            match current {
                Some(now) if now == *recorded => None,
                _ => Some(format!(
                    "refusing to patch {}: it changed on disk since you read it. Re-read it with read_file first.",
                    path.display()
                )),
            }
        }
    }
}

/// Record (or refresh) a file's mtime in the working set. Best-effort — a file
/// that can't be stat'd is simply not recorded.
fn record_mtime(working_set: &mut HashMap<PathBuf, SystemTime>, path: &Path) {
    if let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) {
        working_set.insert(path.to_path_buf(), modified);
    }
}

/// Render author diagnostics into a retry message the model can act on.
fn render_diagnostics(diagnostics: &[Diagnostic]) -> String {
    let mut out =
        String::from("The verifier found errors you introduced. Fix them and continue:\n");
    for d in diagnostics {
        let col = d.column.map(|c| format!(":{c}")).unwrap_or_default();
        let severity = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
            Severity::Help => "help",
        };
        out.push_str(&format!(
            "- {}:{}{col} {severity}: {}\n",
            d.path.display(),
            d.line,
            d.message,
        ));
    }
    out
}

fn hard_fail_result(
    input: &PhaseInput,
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    project_root: &Path,
    diagnostics: Vec<Diagnostic>,
    signal: HardFailSignal,
    artifacts: Artifacts,
) -> PhaseResult {
    let briefing = Briefing {
        goal: input.goal.clone(),
        acceptance_criteria: input.acceptance_criteria.clone(),
        diagnostics,
        working_files: collect_working_files(recent_tool_calls, project_root),
        what_was_tried: summarize_attempts(recent_tool_calls),
        current_blocker: Blocker::HardFail(signal),
        budget_remaining: "halted on hard-fail".to_string(),
    };
    PhaseResult::hard_fail(briefing, artifacts)
}

/// Dispatch a tool call through the registry. Returns `(succeeded, content)`
/// where `content` is the message fed back to the model. A missing tool or an
/// execution error is a model-visible failure, not an `Err`.
async fn dispatch(registry: &ToolRegistry, tc: &ToolCall) -> (bool, String) {
    match registry.get(&tc.name) {
        None => (false, format!("error: unknown tool '{}'", tc.name)),
        Some(tool) => match tool.execute(tc.arguments.clone()).await {
            Ok(result) => match result.error {
                Some(error) => (false, error),
                None => (true, result.output),
            },
            Err(e) => (false, format!("tool execution failed: {e}")),
        },
    }
}

fn append_tool_exchange(messages: &mut Vec<Message>, tc: &ToolCall, content: &str, turn: usize) {
    let id = next_tool_id();
    let arguments = serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string());
    messages.push(Message {
        role: "assistant".to_string(),
        content: String::new(),
        tool_calls: Some(vec![AiToolCall {
            id: id.clone(),
            name: tc.name.clone(),
            arguments,
            thought_signature: None,
        }]),
        tool_results: None,
        turn: Some(turn),
    });
    messages.push(Message {
        role: "tool".to_string(),
        content: String::new(),
        tool_calls: None,
        tool_results: Some(vec![AiToolResult {
            tool_call_id: id,
            tool_name: tc.name.clone(),
            content: content.to_string(),
        }]),
        turn: Some(turn),
    });
}

fn assistant_text(content: &str, turn: usize) -> Message {
    Message {
        role: "assistant".to_string(),
        content: content.to_string(),
        tool_calls: None,
        tool_results: None,
        turn: Some(turn),
    }
}

fn user_text(content: &str, turn: usize) -> Message {
    Message {
        role: "user".to_string(),
        content: content.to_string(),
        tool_calls: None,
        tool_results: None,
        turn: Some(turn),
    }
}

fn turns_line(max_turns: usize) -> String {
    format!("0 of {max_turns} turns remaining")
}

fn budget_exceeded_result(
    input: &PhaseInput,
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    project_root: &Path,
    budget_remaining: String,
    artifacts: Artifacts,
) -> PhaseResult {
    let briefing = Briefing {
        goal: input.goal.clone(),
        acceptance_criteria: input.acceptance_criteria.clone(),
        diagnostics: Vec::new(),
        working_files: collect_working_files(recent_tool_calls, project_root),
        what_was_tried: summarize_attempts(recent_tool_calls),
        current_blocker: Blocker::BudgetExceeded,
        budget_remaining,
    };
    PhaseResult::budget_exceeded(briefing, artifacts)
}

/// Build the artifacts common to every terminal return: the unified diff +
/// `files_changed` of what the model edited, the update-log line, the log path,
/// and the (status-specific) command outputs.
fn build_artifacts(
    pre_edit_content: &HashMap<PathBuf, Option<String>>,
    project_root: &Path,
    log_path: Option<PathBuf>,
    status: &str,
    turns: usize,
    command_outputs: CommandOutputs,
) -> Artifacts {
    let (diff, files_changed) = build_diff(pre_edit_content, project_root);
    Artifacts {
        files_changed,
        diff,
        command_outputs,
        update_log: format!("Executor run: {status} after {turns} turn(s)."),
        log_path,
    }
}

/// Render the combined unified diff (capped) and the `files_changed` summary from
/// the pre-edit snapshots. Files whose content is unchanged (e.g. an edit later
/// reverted) are omitted. Deterministic order (sorted by path).
fn build_diff(
    pre_edit_content: &HashMap<PathBuf, Option<String>>,
    project_root: &Path,
) -> (String, Vec<FileChange>) {
    let mut paths: Vec<&PathBuf> = pre_edit_content.keys().collect();
    paths.sort();

    let mut diff = String::new();
    let mut files_changed = Vec::new();
    for path in paths {
        let before = pre_edit_content
            .get(path)
            .and_then(|b| b.clone())
            .unwrap_or_default();
        let after = std::fs::read_to_string(path).unwrap_or_default();
        if before == after {
            continue;
        }
        let rel = path.strip_prefix(project_root).unwrap_or(path);
        let rel_str = rel.display().to_string();
        let text_diff = TextDiff::from_lines(&before, &after);

        let mut added = 0usize;
        let mut removed = 0usize;
        for change in text_diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Insert => added += 1,
                ChangeTag::Delete => removed += 1,
                ChangeTag::Equal => {}
            }
        }

        if diff.chars().count() < MAX_DIFF_CHARS {
            diff.push_str(
                &text_diff
                    .unified_diff()
                    .header(&rel_str, &rel_str)
                    .to_string(),
            );
            if diff.chars().count() > MAX_DIFF_CHARS {
                diff = diff.chars().take(MAX_DIFF_CHARS).collect();
                diff.push_str("\n… (diff truncated)\n");
            }
        }
        files_changed.push(FileChange {
            path: rel.to_path_buf(),
            change_summary: format!("+{added} -{removed}"),
        });
    }
    (diff, files_changed)
}

/// Shared context for progress emission, avoiding repeated parameter passing.
struct EmitCtx<'a> {
    progress: Option<&'a dyn ProgressCallback>,
    log_handle: &'a Option<SessionLogHandle>,
    redactor: &'a Redactor,
    clock: &'a (dyn Fn() -> u64 + Send + Sync),
    pre_edit_content: &'a HashMap<PathBuf, Option<String>>,
    project_root: &'a Path,
    turn: usize,
}

/// Emit a progress event. The two consumers are independent: the
/// `SessionEvent::Progress` record is always logged (so `rexymcp status` and
/// Claude's post-return log queries can see liveness even when no live watcher
/// is attached), while the live callback fires only when one is present. The
/// log write self-gates on the session-log handle, so this is a no-op only
/// when there is neither a handle nor a callback.
fn emit_progress(ctx: &EmitCtx<'_>, stage: String) {
    let numstat = progress::numstat_from_pre_edit(ctx.pre_edit_content, ctx.project_root);
    let message = progress::format_message(ctx.turn, &stage, &numstat);

    if let Some(cb) = ctx.progress {
        cb.on_progress(&ProgressEvent {
            turn: ctx.turn,
            stage: stage.clone(),
            files_changed: numstat.clone(),
            message: message.clone(),
        });
    }

    log_event(
        ctx.log_handle,
        ctx.redactor,
        ctx.clock,
        ctx.turn,
        SessionEvent::Progress {
            turn: ctx.turn,
            stage,
            files_changed: numstat,
            message,
        },
    );
}

/// Run the configured final command set in `cwd`, tail-capping each output and
/// recording pass/fail. A `None`-configured command stays `None` in both outputs.
async fn run_command_set(
    runner: &dyn CommandRunner,
    commands: &CommandConfig,
    cwd: &Path,
    ctx: &EmitCtx<'_>,
) -> (CommandOutputs, Gates) {
    if commands.format.is_some() {
        emit_progress(ctx, "command:fmt".to_string());
    }
    let (format, fmt_ok) = run_one(runner, commands.format.as_deref(), cwd).await;
    if commands.build.is_some() {
        emit_progress(ctx, "command:build".to_string());
    }
    let (build, build_ok) = run_one(runner, commands.build.as_deref(), cwd).await;
    if commands.lint.is_some() {
        emit_progress(ctx, "command:lint".to_string());
    }
    let (lint, lint_ok) = run_one(runner, commands.lint.as_deref(), cwd).await;
    if commands.test.is_some() {
        emit_progress(ctx, "command:test".to_string());
    }
    let (test, test_ok) = run_one(runner, commands.test.as_deref(), cwd).await;
    (
        CommandOutputs {
            format,
            build,
            lint,
            test,
        },
        Gates {
            fmt: fmt_ok,
            build: build_ok,
            lint: lint_ok,
            test: test_ok,
        },
    )
}

async fn run_one(
    runner: &dyn CommandRunner,
    command: Option<&str>,
    cwd: &Path,
) -> (Option<String>, Option<bool>) {
    match command {
        Some(cmd) => {
            let CommandResult { output, success } = runner.run(cmd, cwd).await;
            (Some(tail(&output, MAX_COMMAND_TAIL_CHARS)), Some(success))
        }
        None => (None, None),
    }
}

fn tail(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count > max_chars {
        s.chars().skip(count - max_chars).collect()
    } else {
        s.to_string()
    }
}

/// Telemetry counters accumulated across the turn cycle, folded into the
/// `PhaseRun` at the terminal return.
struct RunMetrics {
    parse_attempts: usize,
    parse_failures: usize,
    total_repairs: usize,
    total_calls: usize,
    verifier_retries: usize,
    tokens: TokenBreakdown,
    start_ms: u64,
    served_model: Option<String>,
    length_finishes: usize,
    total_finishes: usize,
}

impl RunMetrics {
    fn started_at(start_ms: u64) -> Self {
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

    fn add_tokens(&mut self, b: &TokenBreakdown) {
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
fn emit_phase_run(
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

    let run = PhaseRun {
        ts: now,
        model: deps.model.to_string(),
        generation_params: deps.generation_params.clone(),
        phase_id: input.phase.clone(),
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
    };
    let _ = telemetry::append(dir, &run);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::testing::{MockAiClientScript, MockCall};
    use crate::ai::types::TokenBreakdown;
    use crate::phase::PhaseStatus;
    use crate::security::scope::Scope;
    use crate::tools::{patch, read_file, write_file};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    fn registry_over(scope: Scope) -> ToolRegistry {
        let mut r = ToolRegistry::new();
        r.register(read_file(scope.clone()));
        r.register(write_file(scope.clone()));
        r.register(patch(scope));
        r
    }

    const SESSION_ID: &str = "testsid";
    const PHASE_SLUG: &str = "phase-07b";

    fn clock_zero() -> u64 {
        0
    }

    fn input() -> PhaseInput {
        PhaseInput {
            standards: "STANDARDS".to_string(),
            phase_doc: "PHASE".to_string(),
            goal: "make it compile".to_string(),
            acceptance_criteria: "cargo build passes".to_string(),
            phase: PHASE_SLUG.to_string(),
            tags: vec!["rust".to_string(), "feature".to_string()],
        }
    }

    /// A verifier that is never expected to fire (existing non-edit tests). If an
    /// edit-class call ever reaches it, `verify` returns `Unsupported` so it stays
    /// inert rather than spawning a real compiler.
    struct NoopVerifier;

    #[async_trait::async_trait]
    impl FileVerifier for NoopVerifier {
        async fn verify(&self, _path: &Path) -> VerifierResult {
            VerifierResult::Unsupported
        }
        async fn capture_baseline(&self, _paths: &[PathBuf]) -> Baseline {
            Baseline::new()
        }
    }

    /// A command runner for runs with no commands configured (`EMPTY_COMMANDS`),
    /// where `run` is never actually reached; returns an empty success if it is.
    struct NoopRunner;

    #[async_trait::async_trait]
    impl CommandRunner for NoopRunner {
        async fn run(&self, _command: &str, _cwd: &Path) -> CommandResult {
            CommandResult {
                output: String::new(),
                success: true,
            }
        }
    }

    const EMPTY_COMMANDS: CommandConfig = CommandConfig {
        format: None,
        build: None,
        lint: None,
        test: None,
    };

    fn deps<'a>(
        client: &'a dyn AiClient,
        registry: &'a ToolRegistry,
        budget: &'a Budget,
        max_turns: usize,
        root: &'a Path,
    ) -> LoopDeps<'a> {
        LoopDeps {
            client,
            registry,
            tools: &[],
            budget,
            max_turns,
            project_root: root,
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_zero,
            verifier: &NoopVerifier,
            commands: &EMPTY_COMMANDS,
            runner: &NoopRunner,
            generation_params: GenerationParams {
                temperature: None,
                seed: None,
            },
            telemetry_dir: None,
            progress: None,
            context_window: None,
        }
    }

    /// The on-disk log path for a run driven by `deps()` over `root`.
    fn log_path(root: &Path) -> std::path::PathBuf {
        root.join(".rexymcp")
            .join("sessions")
            .join(format!("session-{PHASE_SLUG}-{SESSION_ID}.jsonl"))
    }

    fn token(s: &str) -> AiEvent {
        AiEvent::Token(s.to_string())
    }

    fn native(name: &str, args: serde_json::Value) -> AiEvent {
        AiEvent::ToolCallGeneric {
            id: "tc_x".to_string(),
            name: name.to_string(),
            args,
            thought_signature: None,
        }
    }

    #[tokio::test]
    async fn no_tool_call_first_turn_completes_immediately() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("All done, nothing to call.")]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        assert!(result.briefing.is_none());
        assert_eq!(client.calls().len(), 1);
    }

    #[tokio::test]
    async fn think_only_completion_is_not_complete() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![
            vec![token("<think>I will read the file</think>\n\n")],
            vec![token("All done.")],
        ]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        assert_eq!(client.calls().len(), 2);
    }

    #[tokio::test]
    async fn think_only_completion_at_budget_is_budget_exceeded() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![
            vec![token("<think>plan</think>\n\n")],
            vec![token("<think>still thinking</think>\n")],
        ]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 2, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::BudgetExceeded);
    }

    #[tokio::test]
    async fn complete_result_has_no_briefing() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert!(result.briefing.is_none());
    }

    #[tokio::test]
    async fn tool_call_then_no_tool_call_completes() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![token("now I'm done")],
        ]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        assert_eq!(client.calls().len(), 2);
    }

    #[tokio::test]
    async fn native_tool_call_event_dispatches_as_origin_native() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        // Always emit a native call; cap at 1 turn so we can inspect the snapshot.
        let client =
            MockAiClientScript::new(vec![vec![native("read_file", json!({ "path": path }))]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 1, dir.path()))
            .await
            .unwrap();

        // The native call was dispatched (it succeeded reading the file), recorded
        // in what_was_tried via the snapshot path.
        assert_eq!(result.status, PhaseStatus::BudgetExceeded);
        let briefing = result.briefing.unwrap();
        assert_eq!(briefing.what_was_tried.len(), 1);
        assert!(briefing.what_was_tried[0].one_line.contains("read_file"));
        assert!(briefing.what_was_tried[0].one_line.contains("succeeded"));
    }

    #[tokio::test]
    async fn native_tool_call_skips_text_parser() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        // Completion text contains a *different* (malformed-name) call; the native
        // event must win and the text must not be parsed/dispatched.
        let client = MockAiClientScript::new(vec![vec![
            token("{\"name\":\"write_file\",\"arguments\":{}}"),
            native("read_file", json!({ "path": path })),
        ]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 1, dir.path()))
            .await
            .unwrap();

        let briefing = result.briefing.unwrap();
        assert_eq!(briefing.what_was_tried.len(), 1);
        assert!(briefing.what_was_tried[0].one_line.contains("read_file"));
    }

    #[tokio::test]
    async fn native_unknown_tool_feeds_failure_not_err() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![
            vec![native("does_not_exist", json!({}))],
            vec![token("giving up")],
        ]);
        let budget = Budget::new(1_000_000);

        // Returns Ok (model-visible failure), reaches Complete on the next turn.
        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        // The unknown-tool feedback was placed in the conversation the second call saw.
        let second_call_messages = &client.calls()[1].messages;
        let has_unknown = second_call_messages.iter().any(|m| {
            m.tool_results
                .as_ref()
                .map(|trs| trs.iter().any(|t| t.content.contains("unknown tool")))
                .unwrap_or(false)
        });
        assert!(
            has_unknown,
            "expected an unknown-tool failure fed back to the model"
        );
    }

    #[tokio::test]
    async fn text_tool_call_is_parsed_and_dispatched() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let hermes = format!(
            "<tool_call>{{\"name\":\"read_file\",\"arguments\":{{\"path\":\"{}\"}}}}</tool_call>",
            path.replace('\\', "\\\\")
        );
        let client = MockAiClientScript::new(vec![vec![token(&hermes)]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 1, dir.path()))
            .await
            .unwrap();

        let briefing = result.briefing.unwrap();
        assert_eq!(briefing.what_was_tried.len(), 1);
        assert!(briefing.what_was_tried[0].one_line.contains("read_file"));
    }

    #[tokio::test]
    async fn parse_failure_feeds_feedback_and_continues() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        // First turn: a malformed call (unknown tool name, valid JSON) → Failed.
        // Second turn: plain text → Complete.
        let client = MockAiClientScript::new(vec![
            vec![token("{\"name\":\"nonexistent\",\"arguments\":{}}")],
            vec![token("ok, stopping")],
        ]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        // The second call's messages include a user message carrying parser feedback.
        let second = &client.calls()[1].messages;
        let has_user_feedback = second
            .iter()
            .any(|m| m.role == "user" && !m.content.is_empty());
        assert!(
            has_user_feedback,
            "expected parser feedback fed back as a user message"
        );
    }

    #[tokio::test]
    async fn turn_cap_returns_budget_exceeded_with_briefing() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path.clone() }))],
            vec![native("read_file", json!({ "path": path.clone() }))],
            vec![native("read_file", json!({ "path": path }))],
        ]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 2, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::BudgetExceeded);
        let briefing = result.briefing.unwrap();
        assert!(matches!(briefing.current_blocker, Blocker::BudgetExceeded));
        assert!(!briefing.what_was_tried.is_empty());
    }

    #[tokio::test]
    async fn budget_briefing_carries_goal_and_attempts() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client =
            MockAiClientScript::new(vec![vec![native("read_file", json!({ "path": path }))]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 1, dir.path()))
            .await
            .unwrap();

        let briefing = result.briefing.unwrap();
        assert_eq!(briefing.goal, "make it compile");
        assert!(!briefing.what_was_tried.is_empty());
    }

    #[tokio::test]
    async fn budget_overflow_after_compaction_returns_budget_exceeded() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        // A ceiling of 1 token — the system prompt alone overflows and cannot be
        // compacted away (system is never evicted), so the loop gives up before
        // ever calling the model.
        let client = MockAiClientScript::new(vec![vec![token("unused")]]);
        let budget = Budget::new(1);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::BudgetExceeded);
        assert_eq!(
            client.calls().len(),
            0,
            "must not call the model when over budget"
        );
        assert!(result.briefing.is_some());
    }

    #[tokio::test]
    async fn budget_with_headroom_runs_without_compaction() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
    }

    #[tokio::test]
    async fn tool_outcomes_distinguish_success_and_failure() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("does_not_exist", json!({}))],
            vec![native("read_file", json!({ "path": path }))],
        ]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 2, dir.path()))
            .await
            .unwrap();

        let tried = result.briefing.unwrap().what_was_tried;
        assert_eq!(tried.len(), 2);
        assert!(tried[0].one_line.contains("failed"));
        assert!(tried[1].one_line.contains("succeeded"));
    }

    // Turn-0 case: backend error before any work stays Err (nothing to preserve).
    #[tokio::test]
    async fn ai_event_error_propagates_as_err() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![
            token("partial"),
            AiEvent::Error("backend exploded".to_string()),
            AiEvent::Done(TokenBreakdown::default()),
        ]]);
        let budget = Budget::new(1_000_000);

        let result =
            execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path())).await;

        assert!(
            result.is_err(),
            "AiEvent::Error must surface as Err, not a PhaseResult"
        );
    }

    // ── M7-01: backend error degradation ──────────────────────────────────

    /// Mock that returns `Err` from `chat()` on a configured call number.
    /// Used to exercise the `chat_fut` error path (site A).
    struct MockAiClientChatError {
        error_on_call: Arc<Mutex<usize>>,
        events: Arc<Mutex<VecDeque<Vec<AiEvent>>>>,
        calls: Arc<Mutex<Vec<MockCall>>>,
    }

    impl MockAiClientChatError {
        fn new(events: Vec<Vec<AiEvent>>, error_on_call: usize) -> Self {
            Self {
                error_on_call: Arc::new(Mutex::new(error_on_call)),
                events: Arc::new(Mutex::new(events.into_iter().collect())),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait::async_trait]
    impl AiClient for MockAiClientChatError {
        async fn chat(
            &self,
            system_prompt: &str,
            messages: Vec<Message>,
            tx: mpsc::UnboundedSender<AiEvent>,
            tools: Option<&[ToolSchema]>,
        ) -> anyhow::Result<()> {
            let call_idx = {
                let mut calls = self.calls.lock().unwrap();
                let idx = calls.len();
                calls.push(MockCall {
                    system_prompt: system_prompt.to_string(),
                    messages,
                    tool_count: tools.map(|t| t.len()).unwrap_or(0),
                });
                idx
            };
            let error_on = *self.error_on_call.lock().unwrap();
            if call_idx == error_on {
                return Err(anyhow::anyhow!("transient backend failure"));
            }
            let events = self.events.lock().unwrap().pop_front();
            if let Some(evts) = events {
                for e in evts {
                    let _ = tx.send(e);
                }
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn backend_error_after_progress_degrades_to_hard_fail() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        // Turn 0: a tool call (read_file) → loop continues with turns=1.
        // Turn 1: chat() returns Err → should degrade to hard_fail.
        let client = MockAiClientChatError::new(
            vec![vec![native("read_file", json!({ "path": path }))]],
            1, // second chat() call (index 1) returns error
        );
        let budget = Budget::new(1_000_000);

        let result =
            execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path())).await;

        assert!(
            result.is_ok(),
            "backend error after progress must degrade to Ok(hard_fail), not Err"
        );
        let phase_result = result.unwrap();
        assert_eq!(phase_result.status, PhaseStatus::HardFail);
        assert!(phase_result.briefing.is_some());
        let briefing = phase_result.briefing.unwrap();
        assert!(
            matches!(
                briefing.current_blocker,
                Blocker::HardFail(HardFailSignal::BackendError { .. })
            ),
            "expected BackendError hard-fail signal, got {:?}",
            briefing.current_blocker
        );
    }

    #[tokio::test]
    async fn ai_event_error_after_progress_degrades_to_hard_fail() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        // Turn 0: a tool call (read_file) → loop continues with turns=1.
        // Turn 1: AiEvent::Error in the stream → should degrade to hard_fail.
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![
                token("starting second"),
                AiEvent::Error("mid-phase error".to_string()),
                AiEvent::Done(TokenBreakdown::default()),
            ],
        ]);
        let budget = Budget::new(1_000_000);

        let result =
            execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path())).await;

        assert!(
            result.is_ok(),
            "AiEvent::Error after progress must degrade to Ok(hard_fail), not Err"
        );
        let phase_result = result.unwrap();
        assert_eq!(phase_result.status, PhaseStatus::HardFail);
        assert!(phase_result.briefing.is_some());
        let briefing = phase_result.briefing.unwrap();
        assert!(
            matches!(
                briefing.current_blocker,
                Blocker::HardFail(HardFailSignal::BackendError { .. })
            ),
            "expected BackendError hard-fail signal, got {:?}",
            briefing.current_blocker
        );
    }

    // ── 07b: session log ──────────────────────────────────────────────────

    use crate::store::sessions::jsonl::read_session_log;

    fn records(root: &Path) -> Vec<crate::store::sessions::event::SessionRecord> {
        read_session_log(&log_path(root)).unwrap()
    }

    #[tokio::test]
    async fn creates_log_file_named_with_phase_and_session_id() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let path = log_path(dir.path());
        assert!(path.exists(), "log file not created");
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(name.contains(PHASE_SLUG) && name.contains(SESSION_ID));
    }

    #[tokio::test]
    async fn logs_session_start_first_then_prompt() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let recs = records(dir.path());
        assert!(matches!(recs[0].event, SessionEvent::SessionStart { .. }));
        assert!(matches!(recs[1].event, SessionEvent::Prompt { .. }));
        match &recs[0].event {
            SessionEvent::SessionStart {
                session_id,
                model,
                phase,
            } => {
                assert_eq!(session_id, SESSION_ID);
                assert_eq!(model, "test-model");
                assert_eq!(phase, PHASE_SLUG);
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn logs_completion_parsed_and_tool_result_for_dispatched_turn() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![token("done")],
        ]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        // Progress and Metrics records are logged unconditionally and interleave
        // with the turn events; filter them out to assert the turn-event sequence.
        let kinds: Vec<&str> = records(dir.path())
            .iter()
            .map(|r| event_kind(&r.event))
            .filter(|k| *k != "progress" && *k != "metrics")
            .collect();
        // SessionStart, Prompt, then turn 1: Completion, Parsed, ToolResult, then
        // turn 2 Completion, then SessionEnd.
        assert_eq!(kinds[0], "session_start");
        assert_eq!(kinds[1], "prompt");
        assert_eq!(kinds[2], "completion");
        assert_eq!(kinds[3], "parsed");
        assert_eq!(kinds[4], "tool_result");
        assert_eq!(*kinds.last().unwrap(), "session_end");
    }

    #[tokio::test]
    async fn logs_parse_failed_for_malformed_turn() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![
            vec![token("{\"name\":\"nonexistent\",\"arguments\":{}}")],
            vec![token("stopping")],
        ]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert!(
            records(dir.path())
                .iter()
                .any(|r| matches!(r.event, SessionEvent::ParseFailed { .. })),
            "expected a ParseFailed event"
        );
    }

    #[tokio::test]
    async fn logs_session_end_complete_on_clean_finish() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let recs = records(dir.path());
        match &recs.last().unwrap().event {
            SessionEvent::SessionEnd { status, turns } => {
                assert_eq!(status, "complete");
                assert_eq!(*turns, 1);
            }
            other => panic!("expected SessionEnd, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn logs_session_end_budget_exceeded_on_turn_cap() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path.clone() }))],
            vec![native("read_file", json!({ "path": path }))],
        ]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 1, dir.path()))
            .await
            .unwrap();

        match &records(dir.path()).last().unwrap().event {
            SessionEvent::SessionEnd { status, .. } => assert_eq!(status, "budget_exceeded"),
            other => panic!("expected SessionEnd, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn redacts_secret_in_tool_output_before_writing() {
        const SECRET: &str = "sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("creds.txt"), SECRET).unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("creds.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![token("done")],
        ]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let raw = std::fs::read_to_string(log_path(dir.path())).unwrap();
        assert!(!raw.contains(SECRET), "secret leaked into the session log");
        assert!(
            raw.contains("[REDACTED:openai_key]"),
            "redaction marker missing"
        );
    }

    #[tokio::test]
    async fn redacts_secret_in_completion_before_writing() {
        const SECRET: &str = "sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client =
            MockAiClientScript::new(vec![vec![token(&format!("here is the key {SECRET} ok"))]]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let raw = std::fs::read_to_string(log_path(dir.path())).unwrap();
        assert!(!raw.contains(SECRET), "secret leaked via completion");
        assert!(raw.contains("[REDACTED:openai_key]"));
    }

    #[tokio::test]
    async fn logging_failure_does_not_change_result() {
        let dir = TempDir::new().unwrap();
        // Pre-create `.rexymcp` as a *file* so the sessions dir cannot be created
        // and the log fails to open — the run must still complete normally.
        std::fs::write(dir.path().join(".rexymcp"), "not a dir").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        assert!(!log_path(dir.path()).exists());
    }

    #[tokio::test]
    async fn injected_clock_sets_record_ts() {
        const TS: u64 = 1_717_000_000_000;
        fn clock_fixed() -> u64 {
            TS
        }
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);
        let d = LoopDeps {
            client: &client,
            registry: &registry,
            tools: &[],
            budget: &budget,
            max_turns: 8,
            project_root: dir.path(),
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_fixed,
            verifier: &NoopVerifier,
            commands: &EMPTY_COMMANDS,
            runner: &NoopRunner,
            generation_params: GenerationParams::default(),
            telemetry_dir: None,
            progress: None,
            context_window: None,
        };

        execute_phase(&input(), d).await.unwrap();

        let recs = records(dir.path());
        assert!(!recs.is_empty());
        assert!(recs.iter().all(|r| r.ts == TS));
    }

    fn event_kind(event: &SessionEvent) -> &'static str {
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
            SessionEvent::Metrics { .. } => "metrics",
            SessionEvent::Compaction { .. } => "compaction",
        }
    }

    // ── 07c: verifier retry + hard-fail ───────────────────────────────────

    use crate::governor::verifier::{Baseline as Bl, Diagnostic, Severity};

    /// Verifier mock: pops a scripted `VerifierResult` per `verify` call (an
    /// exhausted script yields `Unsupported`), returns a configured baseline, and
    /// records the paths it was asked to verify.
    struct MockFileVerifier {
        results: Mutex<VecDeque<VerifierResult>>,
        baseline: Bl,
        verified: Mutex<Vec<PathBuf>>,
    }

    impl MockFileVerifier {
        fn new(results: Vec<VerifierResult>) -> Self {
            Self {
                results: Mutex::new(results.into_iter().collect()),
                baseline: Bl::new(),
                verified: Mutex::new(Vec::new()),
            }
        }

        fn with_baseline(mut self, baseline: Bl) -> Self {
            self.baseline = baseline;
            self
        }

        fn verified_paths(&self) -> Vec<PathBuf> {
            self.verified.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl FileVerifier for MockFileVerifier {
        async fn verify(&self, path: &Path) -> VerifierResult {
            self.verified.lock().unwrap().push(path.to_path_buf());
            self.results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(VerifierResult::Unsupported)
        }
        async fn capture_baseline(&self, _paths: &[PathBuf]) -> Bl {
            self.baseline.clone()
        }
    }

    fn diag(message: &str) -> Diagnostic {
        Diagnostic {
            path: PathBuf::from("src/lib.rs"),
            line: 7,
            column: Some(3),
            severity: Severity::Error,
            message: message.to_string(),
            code: Some("E0425".to_string()),
        }
    }

    fn checked(diagnostics: Vec<Diagnostic>) -> VerifierResult {
        VerifierResult::Checked { diagnostics }
    }

    /// A loop run over `dir` driving `client` with an injected `verifier`.
    async fn run_with_verifier(
        dir: &TempDir,
        client: &dyn AiClient,
        verifier: &dyn FileVerifier,
        max_turns: usize,
    ) -> PhaseResult {
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let budget = Budget::new(1_000_000);
        let d = LoopDeps {
            client,
            registry: &registry,
            tools: &[],
            budget: &budget,
            max_turns,
            project_root: dir.path(),
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_zero,
            verifier,
            commands: &EMPTY_COMMANDS,
            runner: &NoopRunner,
            generation_params: GenerationParams::default(),
            telemetry_dir: None,
            progress: None,
            context_window: None,
        };
        execute_phase(&input(), d).await.unwrap()
    }

    /// A native `write_file` call writing `body` to `name` under the temp root.
    fn write_call(dir: &TempDir, name: &str, body: &str) -> AiEvent {
        let path = dir.path().join(name).to_string_lossy().to_string();
        native("write_file", json!({ "path": path, "content": body }))
    }

    #[tokio::test]
    async fn edit_class_call_runs_verifier() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "fn a() {}")],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![checked(vec![])]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        assert_eq!(verifier.verified_paths().len(), 1);
        assert!(
            verifier.verified_paths()[0].ends_with("a.rs"),
            "verifier should have run on the edited file"
        );
    }

    #[tokio::test]
    async fn non_edit_call_does_not_run_verifier() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![checked(vec![])]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        assert!(verifier.verified_paths().is_empty());
    }

    #[tokio::test]
    async fn clean_verify_produces_no_retry_message() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "fn a() {}")],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![checked(vec![])]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        // The second model call's messages carry no verifier-retry user message.
        let second = &client.calls()[1].messages;
        assert!(
            !second
                .iter()
                .any(|m| m.role == "user" && m.content.contains("verifier found errors")),
            "clean verify must not feed a retry message"
        );
    }

    #[tokio::test]
    async fn author_diagnostics_fed_back_as_retry() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "fn a() { bork }")],
            vec![token("ok I'll fix it")],
        ]);
        let verifier = MockFileVerifier::new(vec![checked(vec![diag("cannot find value `bork`")])]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        let second = &client.calls()[1].messages;
        assert!(
            second
                .iter()
                .any(|m| m.role == "user" && m.content.contains("cannot find value `bork`")),
            "author diagnostic should be fed back as a retry message"
        );
    }

    #[tokio::test]
    async fn ambient_diagnostics_not_fed_back() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "fn a() {}")],
            vec![token("done")],
        ]);
        // The same diagnostic is in the baseline → ambient → must not feed back.
        let ambient = diag("pre-existing error");
        let mut bl = Bl::new();
        bl.record(&ambient);
        let verifier = MockFileVerifier::new(vec![checked(vec![ambient])]).with_baseline(bl);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        let second = &client.calls()[1].messages;
        assert!(
            !second
                .iter()
                .any(|m| m.role == "user" && m.content.contains("pre-existing error")),
            "ambient (baseline) diagnostics must not be fed back"
        );
    }

    #[tokio::test]
    async fn unsupported_verify_is_skipped() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "notes.md", "# hi")],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![VerifierResult::Unsupported]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        // No Verify event logged for an unsupported language.
        let has_verify = records(dir.path())
            .iter()
            .any(|r| matches!(r.event, SessionEvent::Verify { .. }));
        assert!(!has_verify, "Unsupported must not log a Verify event");
    }

    #[tokio::test]
    async fn verifier_failed_appends_notice_not_err() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "fn a() {}")],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![VerifierResult::Failed("spawn failed".into())]);

        let result = run_with_verifier(&dir, &client, &verifier, 8).await;

        assert_eq!(result.status, PhaseStatus::Complete);
        let second = &client.calls()[1].messages;
        assert!(
            second
                .iter()
                .any(|m| m.content.contains("verifier failed: spawn failed")),
            "a verifier infra failure should append a notice, not error"
        );
    }

    #[tokio::test]
    async fn persistent_verifier_failure_trips_hard_fail() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "v1")],
            vec![write_call(&dir, "a.rs", "v2")],
            vec![write_call(&dir, "a.rs", "v3")],
            vec![token("unreached")],
        ]);
        // Three consecutive Checked-with-author verifier runs.
        let verifier = MockFileVerifier::new(vec![
            checked(vec![diag("err1")]),
            checked(vec![diag("err2")]),
            checked(vec![diag("err3")]),
        ]);

        let result = run_with_verifier(&dir, &client, &verifier, 10).await;

        assert_eq!(result.status, PhaseStatus::HardFail);
        let briefing = result.briefing.unwrap();
        assert!(matches!(
            briefing.current_blocker,
            Blocker::HardFail(HardFailSignal::VerifierFailurePersistent { .. })
        ));
        assert!(
            !briefing.diagnostics.is_empty(),
            "hard-fail briefing must carry the current diagnostics"
        );
    }

    #[tokio::test]
    async fn identical_tool_call_repetition_trips_hard_fail() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let mk = || native("read_file", json!({ "path": path }));
        let client = MockAiClientScript::new(vec![
            vec![mk()],
            vec![mk()],
            vec![mk()],
            vec![token("unreached")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_with_verifier(&dir, &client, &verifier, 10).await;

        assert_eq!(result.status, PhaseStatus::HardFail);
        assert!(matches!(
            result.briefing.unwrap().current_blocker,
            Blocker::HardFail(HardFailSignal::IdenticalToolCallRepetition { .. })
        ));
    }

    #[tokio::test]
    async fn runaway_output_trips_hard_fail() {
        let dir = TempDir::new().unwrap();
        // A file larger than the runaway threshold; reading it overflows the cap.
        let big = "x".repeat(110 * 1024);
        std::fs::write(dir.path().join("big.txt"), &big).unwrap();
        let path = dir.path().join("big.txt").to_string_lossy().to_string();
        let client =
            MockAiClientScript::new(vec![vec![native("read_file", json!({ "path": path }))]]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_with_verifier(&dir, &client, &verifier, 10).await;

        assert_eq!(result.status, PhaseStatus::HardFail);
        assert!(matches!(
            result.briefing.unwrap().current_blocker,
            Blocker::HardFail(HardFailSignal::RunawayOutput { .. })
        ));
    }

    #[tokio::test]
    async fn hard_fail_logs_hardfail_then_session_end() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let mk = || native("read_file", json!({ "path": path }));
        let client = MockAiClientScript::new(vec![vec![mk()], vec![mk()], vec![mk()]]);
        let verifier = MockFileVerifier::new(vec![]);

        run_with_verifier(&dir, &client, &verifier, 10).await;

        let kinds: Vec<&str> = records(dir.path())
            .iter()
            .map(|r| event_kind(&r.event))
            .collect();
        let hf = kinds.iter().position(|k| *k == "hard_fail").unwrap();
        let se = kinds.iter().position(|k| *k == "session_end").unwrap();
        assert!(hf < se, "HardFail must be logged before SessionEnd");
        match &records(dir.path()).last().unwrap().event {
            SessionEvent::SessionEnd { status, .. } => assert_eq!(status, "hard_fail"),
            other => panic!("expected SessionEnd, got {other:?}"),
        }
    }

    // ── 07d: read-before-edit ─────────────────────────────────────────────

    fn patch_call(path: &str, old: &str, new: &str) -> ToolCall {
        ToolCall {
            name: "patch".to_string(),
            arguments: json!({ "path": path, "old_str": old, "new_str": new }),
            origin: Origin::Native,
        }
    }

    #[test]
    fn gate_allows_non_patch_calls() {
        let root = Path::new("/repo");
        let ws = HashMap::new();
        let write = ToolCall {
            name: "write_file".to_string(),
            arguments: json!({ "path": "a.rs", "content": "x" }),
            origin: Origin::Native,
        };
        let read = ToolCall {
            name: "read_file".to_string(),
            arguments: json!({ "path": "a.rs" }),
            origin: Origin::Native,
        };
        assert!(read_before_edit_refusal(&write, &ws, root).is_none());
        assert!(read_before_edit_refusal(&read, &ws, root).is_none());
    }

    #[test]
    fn gate_refuses_patch_of_unread_file() {
        let root = Path::new("/repo");
        let ws = HashMap::new();
        let call = patch_call("a.rs", "x", "y");
        let refusal = read_before_edit_refusal(&call, &ws, root).expect("should refuse");
        assert!(refusal.contains("have not read"));
    }

    #[test]
    fn gate_allows_patch_of_read_unchanged_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "fn a() {}").unwrap();
        let mtime = std::fs::metadata(&file).unwrap().modified().unwrap();
        let mut ws = HashMap::new();
        ws.insert(file.clone(), mtime);
        let call = patch_call(file.to_str().unwrap(), "a", "b");
        assert!(read_before_edit_refusal(&call, &ws, dir.path()).is_none());
    }

    #[test]
    fn gate_refuses_patch_when_mtime_changed() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "fn a() {}").unwrap();
        // Working set holds a stale mtime (epoch) — current differs → refuse.
        let mut ws = HashMap::new();
        ws.insert(file.clone(), SystemTime::UNIX_EPOCH);
        let call = patch_call(file.to_str().unwrap(), "a", "b");
        let refusal = read_before_edit_refusal(&call, &ws, dir.path()).expect("should refuse");
        assert!(refusal.contains("changed on disk"));
    }

    #[tokio::test]
    async fn patch_without_prior_read_is_refused() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("t.txt");
        std::fs::write(&file, "original").unwrap();
        let path = file.to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native(
                "patch",
                json!({ "path": path, "old_str": "original", "new_str": "edited" }),
            )],
            vec![token("ok")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "original",
            "refused patch must not modify the file"
        );
        let second = &client.calls()[1].messages;
        assert!(
            second.iter().any(|m| m
                .tool_results
                .as_ref()
                .is_some_and(|trs| trs.iter().any(|t| t.content.contains("have not read")))),
            "refusal should be fed back to the model"
        );
    }

    #[tokio::test]
    async fn patch_after_reading_is_allowed() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("t.txt");
        std::fs::write(&file, "original").unwrap();
        let path = file.to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![native(
                "patch",
                json!({ "path": path, "old_str": "original", "new_str": "edited" }),
            )],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "edited",
            "patch after a read should apply"
        );
    }

    #[tokio::test]
    async fn write_file_without_read_is_allowed() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("new.txt");
        let path = file.to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native(
                "write_file",
                json!({ "path": path, "content": "fresh" }),
            )],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);

        run_with_verifier(&dir, &client, &verifier, 8).await;

        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "fresh",
            "write_file is not gated by read-before-edit"
        );
    }

    #[tokio::test]
    async fn repeated_refused_patch_trips_hard_fail() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("t.txt");
        std::fs::write(&file, "original").unwrap();
        let path = file.to_string_lossy().to_string();
        let mk = || {
            native(
                "patch",
                json!({ "path": path, "old_str": "original", "new_str": "x" }),
            )
        };
        let client = MockAiClientScript::new(vec![vec![mk()], vec![mk()], vec![mk()]]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_with_verifier(&dir, &client, &verifier, 10).await;

        assert_eq!(result.status, PhaseStatus::HardFail);
        assert!(matches!(
            result.briefing.unwrap().current_blocker,
            Blocker::HardFail(HardFailSignal::IdenticalToolCallRepetition { .. })
        ));
    }

    // ── 07e: completion artifacts ─────────────────────────────────────────

    /// Records which commands ran and returns scripted output; commands named in
    /// `failing` return `success: false` (for gate tests).
    struct MockCommandRunner {
        ran: Mutex<Vec<String>>,
        output: String,
        failing: HashSet<String>,
    }

    impl MockCommandRunner {
        fn new(output: &str) -> Self {
            Self {
                ran: Mutex::new(Vec::new()),
                output: output.to_string(),
                failing: HashSet::new(),
            }
        }
        fn failing(mut self, command: &str) -> Self {
            self.failing.insert(command.to_string());
            self
        }
        fn ran(&self) -> Vec<String> {
            self.ran.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl CommandRunner for MockCommandRunner {
        async fn run(&self, command: &str, _cwd: &Path) -> CommandResult {
            self.ran.lock().unwrap().push(command.to_string());
            CommandResult {
                output: self.output.clone(),
                success: !self.failing.contains(command),
            }
        }
    }

    /// Full run with injectable command runner + command config + telemetry dir.
    async fn run_full(
        dir: &TempDir,
        client: &dyn AiClient,
        verifier: &dyn FileVerifier,
        runner: &dyn CommandRunner,
        commands: &CommandConfig,
        telemetry_dir: Option<&Path>,
        max_turns: usize,
    ) -> PhaseResult {
        run_full_with_context_window(
            dir,
            client,
            verifier,
            runner,
            commands,
            telemetry_dir,
            max_turns,
            None,
        )
        .await
    }

    /// Same as `run_full` but with an explicit `context_window` value.
    #[allow(clippy::too_many_arguments)]
    async fn run_full_with_context_window(
        dir: &TempDir,
        client: &dyn AiClient,
        verifier: &dyn FileVerifier,
        runner: &dyn CommandRunner,
        commands: &CommandConfig,
        telemetry_dir: Option<&Path>,
        max_turns: usize,
        context_window: Option<usize>,
    ) -> PhaseResult {
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let budget = Budget::new(1_000_000);
        let d = LoopDeps {
            client,
            registry: &registry,
            tools: &[],
            budget: &budget,
            max_turns,
            project_root: dir.path(),
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_zero,
            verifier,
            commands,
            runner,
            generation_params: GenerationParams::default(),
            telemetry_dir,
            progress: None,
            context_window,
        };
        execute_phase(&input(), d).await.unwrap()
    }

    #[tokio::test]
    async fn diff_and_files_changed_for_edited_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("t.txt");
        std::fs::write(&file, "original\n").unwrap();
        let path = file.to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![native(
                "patch",
                json!({ "path": path, "old_str": "original", "new_str": "edited" }),
            )],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            None,
            8,
        )
        .await;

        assert!(result.diff.contains("-original"), "diff: {}", result.diff);
        assert!(result.diff.contains("+edited"));
        assert_eq!(result.files_changed.len(), 1);
        assert!(result.files_changed[0].path.ends_with("t.txt"));
        assert!(result.files_changed[0].change_summary.contains('+'));
    }

    #[tokio::test]
    async fn new_file_diff_is_all_added() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("new.txt");
        let path = file.to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native(
                "write_file",
                json!({ "path": path, "content": "line1\nline2\n" }),
            )],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            None,
            8,
        )
        .await;

        assert!(result.diff.contains("+line1"));
        assert!(result.diff.contains("+line2"));
        assert!(!result.diff.contains("-line"));
        assert_eq!(result.files_changed.len(), 1);
    }

    #[tokio::test]
    async fn unchanged_file_is_absent_from_files_changed() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("t.txt");
        std::fs::write(&file, "same\n").unwrap();
        let path = file.to_string_lossy().to_string();
        // write_file with identical content → no net change.
        let client = MockAiClientScript::new(vec![
            vec![native(
                "write_file",
                json!({ "path": path, "content": "same\n" }),
            )],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            None,
            8,
        )
        .await;

        assert!(
            result.files_changed.is_empty(),
            "an unchanged file must not appear in files_changed"
        );
        assert!(result.diff.is_empty());
    }

    #[tokio::test]
    async fn clean_completion_runs_configured_commands() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let verifier = MockFileVerifier::new(vec![]);
        let runner = MockCommandRunner::new("ok-output");
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: Some("cargo test".to_string()),
        };

        let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

        assert_eq!(result.status, PhaseStatus::Complete);
        let ran = runner.ran();
        assert_eq!(
            ran,
            vec!["cargo build".to_string(), "cargo test".to_string()]
        );
        assert_eq!(result.command_outputs.build.as_deref(), Some("ok-output"));
        assert_eq!(result.command_outputs.test.as_deref(), Some("ok-output"));
        assert!(result.command_outputs.format.is_none());
        assert!(result.command_outputs.lint.is_none());
    }

    #[tokio::test]
    async fn command_output_is_tail_capped() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let verifier = MockFileVerifier::new(vec![]);
        let big = "y".repeat(MAX_COMMAND_TAIL_CHARS + 500);
        let runner = MockCommandRunner::new(&big);
        let commands = CommandConfig {
            format: None,
            build: Some("b".to_string()),
            lint: None,
            test: None,
        };

        let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

        assert_eq!(
            result
                .command_outputs
                .build
                .as_deref()
                .map(|s| s.chars().count()),
            Some(MAX_COMMAND_TAIL_CHARS)
        );
    }

    #[tokio::test]
    async fn hard_fail_does_not_run_command_set() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let mk = || native("read_file", json!({ "path": path }));
        let client = MockAiClientScript::new(vec![vec![mk()], vec![mk()], vec![mk()]]);
        let verifier = MockFileVerifier::new(vec![]);
        let runner = MockCommandRunner::new("should-not-run");
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: None,
        };

        let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 10).await;

        assert_eq!(result.status, PhaseStatus::HardFail);
        assert!(
            runner.ran().is_empty(),
            "command set must not run on hard-fail"
        );
        assert!(result.command_outputs.build.is_none());
    }

    #[tokio::test]
    async fn complete_result_reports_log_path() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            None,
            8,
        )
        .await;

        assert_eq!(result.log_path, Some(log_path(dir.path())));
    }

    #[tokio::test]
    async fn log_path_is_none_when_log_unopened() {
        let dir = TempDir::new().unwrap();
        // `.rexymcp` as a file → sessions dir can't be created → log doesn't open.
        std::fs::write(dir.path().join(".rexymcp"), "x").unwrap();
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            None,
            8,
        )
        .await;

        assert_eq!(result.status, PhaseStatus::Complete);
        assert!(result.log_path.is_none());
    }

    // ── 08: PhaseRun telemetry ────────────────────────────────────────────

    fn read_runs(telem: &Path) -> Vec<PhaseRun> {
        crate::store::telemetry::read(&telem.join("phase_runs.jsonl")).unwrap()
    }

    #[tokio::test]
    async fn run_appends_one_phase_run_line() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let verifier = MockFileVerifier::new(vec![]);

        run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            8,
        )
        .await;

        let runs = read_runs(&telem);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "complete");
        assert!(!runs[0].escalated);
    }

    #[tokio::test]
    async fn telemetry_none_dir_is_noop_and_completes() {
        let dir = TempDir::new().unwrap();
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            None,
            8,
        )
        .await;

        assert_eq!(result.status, PhaseStatus::Complete);
    }

    #[tokio::test]
    async fn telemetry_write_failure_does_not_change_result() {
        let dir = TempDir::new().unwrap();
        // Telemetry "dir" is actually a file → create_dir_all fails → append errs.
        let telem_file = dir.path().join("telem_is_a_file");
        std::fs::write(&telem_file, "x").unwrap();
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let verifier = MockFileVerifier::new(vec![]);

        let result = run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem_file),
            8,
        )
        .await;

        assert_eq!(result.status, PhaseStatus::Complete);
    }

    #[tokio::test]
    async fn hard_fail_run_is_escalated() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let mk = || native("read_file", json!({ "path": path }));
        let client = MockAiClientScript::new(vec![vec![mk()], vec![mk()], vec![mk()]]);
        let verifier = MockFileVerifier::new(vec![]);

        run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            10,
        )
        .await;

        let runs = read_runs(&telem);
        assert_eq!(runs[0].status, "hard_fail");
        assert!(runs[0].escalated);
    }

    #[tokio::test]
    async fn gates_populated_on_complete_from_exit_status() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let verifier = MockFileVerifier::new(vec![]);
        let runner = MockCommandRunner::new("out").failing("cargo test");
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: Some("cargo test".to_string()),
        };

        run_full(
            &dir,
            &client,
            &verifier,
            &runner,
            &commands,
            Some(&telem),
            8,
        )
        .await;

        let gates = read_runs(&telem)[0].gates.clone();
        assert_eq!(gates.build, Some(true));
        assert_eq!(gates.test, Some(false));
        assert_eq!(gates.fmt, None);
        assert_eq!(gates.lint, None);
    }

    #[tokio::test]
    async fn gates_none_on_hard_fail() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let mk = || native("read_file", json!({ "path": path }));
        let client = MockAiClientScript::new(vec![vec![mk()], vec![mk()], vec![mk()]]);
        let verifier = MockFileVerifier::new(vec![]);
        let runner = MockCommandRunner::new("out");
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: None,
        };

        run_full(
            &dir,
            &client,
            &verifier,
            &runner,
            &commands,
            Some(&telem),
            10,
        )
        .await;

        let gates = read_runs(&telem)[0].gates.clone();
        assert_eq!(gates.build, None, "no gate should be set on hard-fail");
        assert!(runner.ran().is_empty());
    }

    #[tokio::test]
    async fn tool_success_rate_reflects_scorer() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        // one failure (unknown tool) + one success (read_file), then complete.
        let client = MockAiClientScript::new(vec![
            vec![native("does_not_exist", json!({}))],
            vec![native("read_file", json!({ "path": path }))],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);

        run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            8,
        )
        .await;

        let rate = read_runs(&telem)[0].tool_success_rate;
        assert!((rate - 0.5).abs() < 1e-9, "expected 0.5, got {rate}");
    }

    #[tokio::test]
    async fn parse_failure_rate_counts_only_parse_attempts() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        // native turn (no parse), then a malformed text turn (parse fail), then a
        // plain-text turn (parse attempt, NoToolCall) → attempts=2, failures=1.
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![token("{\"name\":\"nonexistent\",\"arguments\":{}}")],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);

        run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            8,
        )
        .await;

        let rate = read_runs(&telem)[0].parse_failure_rate;
        assert!((rate - 0.5).abs() < 1e-9, "expected 0.5, got {rate}");
    }

    #[tokio::test]
    async fn repairs_per_call_counts_repaired_origin() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        // A close-typo tool name the parser fuzzy-repairs → Origin::Repaired.
        let hermes = format!(
            "<tool_call>{{\"name\":\"read_fil\",\"arguments\":{{\"path\":\"{}\"}}}}</tool_call>",
            path
        );
        let client = MockAiClientScript::new(vec![vec![token(&hermes)], vec![token("done")]]);
        let verifier = MockFileVerifier::new(vec![]);

        run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            8,
        )
        .await;

        assert!(
            read_runs(&telem)[0].repairs_per_call > 0.0,
            "a repaired call should count repairs"
        );
    }

    #[tokio::test]
    async fn verifier_retries_counts_author_failures() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "bad")],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![checked(vec![diag("err")])]);

        run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            8,
        )
        .await;

        assert_eq!(read_runs(&telem)[0].verifier_retries, 1);
    }

    #[tokio::test]
    async fn tokens_accumulate_across_done_events() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let tb = |n: u32| {
            AiEvent::Done(TokenBreakdown {
                input_tokens: n,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            })
        };
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path })), tb(10)],
            vec![token("done"), tb(5)],
        ]);
        let verifier = MockFileVerifier::new(vec![]);

        run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            8,
        )
        .await;

        assert_eq!(read_runs(&telem)[0].tokens.input_tokens, 15);
    }

    #[tokio::test]
    async fn logs_metrics_event_per_turn() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let tb = AiEvent::Done(TokenBreakdown {
            input_tokens: 42,
            output_tokens: 17,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        });
        let completion = AiEvent::Completion {
            finish_reason: Some("stop".into()),
            model: None,
        };
        let client = MockAiClientScript::new(vec![vec![token("done"), tb, completion]]);
        // Real, non-sentinel ceiling so context_pct is non-zero.
        // Must be large enough that the prompt doesn't overflow before turn 1.
        let budget = Budget::new(100_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let recs = records(dir.path());
        let metrics_recs: Vec<_> = recs
            .iter()
            .filter_map(|r| {
                if let SessionEvent::Metrics {
                    input_tokens,
                    output_tokens,
                    context_pct,
                } = &r.event
                {
                    Some((*input_tokens, *output_tokens, *context_pct))
                } else {
                    None
                }
            })
            .collect();

        assert!(
            !metrics_recs.is_empty(),
            "expected at least one Metrics record, got {} total records: {:?}",
            recs.len(),
            recs.iter()
                .map(|r| event_kind(&r.event))
                .collect::<Vec<_>>()
        );
        let (in_tok, out_tok, ctx_pct) = metrics_recs.last().unwrap();
        assert_eq!(*in_tok, 42, "input_tokens mismatch");
        assert_eq!(*out_tok, 17, "output_tokens mismatch");
        assert!(
            *ctx_pct > 0.0,
            "context_pct should be > 0 with a real ceiling and non-empty messages"
        );
    }

    #[tokio::test]
    async fn logs_compaction_event_when_budget_overflows() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hi").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        // Tiny budget so overflow fires on turn 0 (system prompt alone is
        // hundreds of tokens). The model is never called.
        let client = MockAiClientScript::new(vec![]);
        let budget = Budget::new(10);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let recs = records(dir.path());
        let compaction_recs: Vec<_> = recs
            .iter()
            .filter_map(|r| {
                if let SessionEvent::Compaction {
                    tokens_before,
                    tokens_after,
                    ..
                } = &r.event
                {
                    Some((*tokens_before, *tokens_after))
                } else {
                    None
                }
            })
            .collect();

        assert!(
            !compaction_recs.is_empty(),
            "expected at least one Compaction record, got {} total records: {:?}",
            recs.len(),
            recs.iter()
                .map(|r| event_kind(&r.event))
                .collect::<Vec<_>>()
        );

        let (tokens_before, tokens_after) = compaction_recs.first().unwrap();
        assert!(*tokens_before > 0, "tokens_before should be > 0");
        assert!(
            *tokens_before >= *tokens_after,
            "tokens_before ({}) should be >= tokens_after ({})",
            tokens_before,
            tokens_after
        );
    }

    #[tokio::test]
    async fn wall_clock_zero_under_constant_clock() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let verifier = MockFileVerifier::new(vec![]);

        run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            8,
        )
        .await;

        assert_eq!(read_runs(&telem)[0].wall_clock_s, 0.0);
    }

    // ── 05a: progress callback ────────────────────────────────────────────

    use crate::agent::progress::ProgressEvent;

    /// Captures progress events into a `Mutex<Vec<ProgressEvent>>` for test
    /// inspection. Implements `ProgressCallback` so it can be held by `LoopDeps`
    /// without a closure lifetime issue.
    struct CaptureCallback {
        events: std::sync::Mutex<Vec<ProgressEvent>>,
    }

    impl CaptureCallback {
        fn new() -> Self {
            Self {
                events: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn events(&self) -> Vec<ProgressEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl ProgressCallback for CaptureCallback {
        fn on_progress(&self, event: &ProgressEvent) {
            self.events.lock().unwrap().push(event.clone());
        }
    }

    /// Helper: build LoopDeps with a progress callback that captures into
    /// `capture`. Uses NoopVerifier + NoopRunner + empty commands + no telemetry.
    fn deps_with_progress_simple<'a>(
        client: &'a dyn AiClient,
        registry: &'a ToolRegistry,
        budget: &'a Budget,
        max_turns: usize,
        root: &'a Path,
        capture: &'a CaptureCallback,
    ) -> LoopDeps<'a> {
        LoopDeps {
            client,
            registry,
            tools: &[],
            budget,
            max_turns,
            project_root: root,
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_zero,
            verifier: &NoopVerifier,
            commands: &EMPTY_COMMANDS,
            runner: &NoopRunner,
            generation_params: GenerationParams {
                temperature: None,
                seed: None,
            },
            telemetry_dir: None,
            progress: Some(capture),
            context_window: None,
        }
    }

    /// Builder for LoopDeps with a progress callback, allowing per-test overrides.
    struct DepsBuilder<'a> {
        client: &'a dyn AiClient,
        registry: &'a ToolRegistry,
        budget: &'a Budget,
        max_turns: usize,
        root: &'a Path,
        capture: &'a CaptureCallback,
        verifier: &'a dyn FileVerifier,
        commands: &'a CommandConfig,
        runner: &'a dyn CommandRunner,
        telemetry_dir: Option<&'a Path>,
    }

    impl<'a> DepsBuilder<'a> {
        fn new(
            client: &'a dyn AiClient,
            registry: &'a ToolRegistry,
            budget: &'a Budget,
            max_turns: usize,
            root: &'a Path,
            capture: &'a CaptureCallback,
        ) -> Self {
            Self {
                client,
                registry,
                budget,
                max_turns,
                root,
                capture,
                verifier: &NoopVerifier,
                commands: &EMPTY_COMMANDS,
                runner: &NoopRunner,
                telemetry_dir: None,
            }
        }
        fn verifier(mut self, v: &'a dyn FileVerifier) -> Self {
            self.verifier = v;
            self
        }
        fn commands(mut self, c: &'a CommandConfig) -> Self {
            self.commands = c;
            self
        }
        fn runner(mut self, r: &'a dyn CommandRunner) -> Self {
            self.runner = r;
            self
        }
        fn build(self) -> LoopDeps<'a> {
            LoopDeps {
                client: self.client,
                registry: self.registry,
                tools: &[],
                budget: self.budget,
                max_turns: self.max_turns,
                project_root: self.root,
                model: "test-model",
                session_id: SESSION_ID,
                clock: &clock_zero,
                verifier: self.verifier,
                commands: self.commands,
                runner: self.runner,
                generation_params: GenerationParams {
                    temperature: None,
                    seed: None,
                },
                telemetry_dir: self.telemetry_dir,
                progress: Some(self.capture),
                context_window: None,
            }
        }
    }

    #[tokio::test]
    async fn progress_none_still_logs_progress_records() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        // The session-log Progress records are independent of the live
        // callback: `rexymcp status` and Claude's post-return log queries must
        // see liveness even when no live watcher (progress token) is attached.
        let recs = records(dir.path());
        assert!(
            recs.iter()
                .any(|r| matches!(r.event, SessionEvent::Progress { .. })),
            "progress: None must still produce Progress log entries"
        );
    }

    #[tokio::test]
    async fn progress_some_emits_turn_start_and_tool() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let path = dir.path().join("f.txt").to_string_lossy().to_string();
        let capture = CaptureCallback::new();
        let client = MockAiClientScript::new(vec![
            vec![native("read_file", json!({ "path": path }))],
            vec![token("done")],
        ]);
        let budget = Budget::new(1_000_000);

        execute_phase(
            &input(),
            deps_with_progress_simple(&client, &registry, &budget, 8, dir.path(), &capture),
        )
        .await
        .unwrap();

        let events = capture.events();
        let stages: Vec<&str> = events.iter().map(|e| e.stage.as_str()).collect();
        assert!(
            stages.contains(&"turn_start"),
            "expected a turn_start event, got: {:?}",
            stages
        );
        assert!(
            stages.contains(&"tool:read_file"),
            "expected a tool:read_file event, got: {:?}",
            stages
        );
        assert!(
            stages.contains(&"tool:read_file"),
            "expected a tool:read_file event, got: {:?}",
            stages
        );

        // Also check the session log has matching Progress entries.
        let recs = records(dir.path());
        let progress_count = recs
            .iter()
            .filter(|r| matches!(r.event, SessionEvent::Progress { .. }))
            .count();
        assert!(
            progress_count >= events.len(),
            "log should have at least as many Progress entries as callback events"
        );
    }

    #[tokio::test]
    async fn progress_emits_verify_after_edit_class_tool() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let capture = CaptureCallback::new();
        let client = MockAiClientScript::new(vec![
            vec![write_call(&dir, "a.rs", "fn a() {}")],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![checked(vec![])]);
        let budget = Budget::new(1_000_000);

        let d = DepsBuilder::new(&client, &registry, &budget, 8, dir.path(), &capture)
            .verifier(&verifier)
            .build();
        execute_phase(&input(), d).await.unwrap();

        let events = capture.events();
        let stages: Vec<&str> = events.iter().map(|e| e.stage.as_str()).collect();
        assert!(
            stages.contains(&"verify"),
            "expected a verify event after edit-class tool, got: {:?}",
            stages
        );
    }

    #[tokio::test]
    async fn progress_emits_commands_on_clean_completion() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let capture = CaptureCallback::new();
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let verifier = MockFileVerifier::new(vec![]);
        let runner = MockCommandRunner::new("ok");
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: Some("cargo test".to_string()),
        };
        let budget = Budget::new(1_000_000);

        let d = DepsBuilder::new(&client, &registry, &budget, 8, dir.path(), &capture)
            .verifier(&verifier)
            .commands(&commands)
            .runner(&runner)
            .build();
        execute_phase(&input(), d).await.unwrap();

        let events = capture.events();
        let stages: Vec<&str> = events.iter().map(|e| e.stage.as_str()).collect();
        assert!(
            stages.contains(&"command:build"),
            "expected command:build, got: {:?}",
            stages
        );
        assert!(
            stages.contains(&"command:test"),
            "expected command:test, got: {:?}",
            stages
        );
        assert!(
            !stages.contains(&"command:fmt"),
            "fmt was not configured, must not emit"
        );
    }

    #[tokio::test]
    #[should_panic(expected = "panic in progress callback")]
    async fn callback_panic_is_not_caught() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        struct PanicCallback;
        impl ProgressCallback for PanicCallback {
            fn on_progress(&self, _event: &ProgressEvent) {
                panic!("panic in progress callback");
            }
        }

        let d = LoopDeps {
            client: &client,
            registry: &registry,
            tools: &[],
            budget: &budget,
            max_turns: 8,
            project_root: dir.path(),
            model: "test-model",
            session_id: SESSION_ID,
            clock: &clock_zero,
            verifier: &NoopVerifier,
            commands: &EMPTY_COMMANDS,
            runner: &NoopRunner,
            generation_params: GenerationParams {
                temperature: None,
                seed: None,
            },
            telemetry_dir: None,
            progress: Some(&PanicCallback),
            context_window: None,
        };
        execute_phase(&input(), d).await.unwrap();
    }

    #[tokio::test]
    async fn progress_independent_of_log_write_failure() {
        let dir = TempDir::new().unwrap();
        // `.rexymcp` as a file → sessions dir can't be created → log doesn't open.
        std::fs::write(dir.path().join(".rexymcp"), "x").unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let capture = CaptureCallback::new();
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        execute_phase(
            &input(),
            deps_with_progress_simple(&client, &registry, &budget, 8, dir.path(), &capture),
        )
        .await
        .unwrap();

        let events = capture.events();
        assert!(
            !events.is_empty(),
            "callback should still receive events even when log dir is unwritable"
        );
        assert!(
            events.iter().any(|e| e.stage == "turn_start"),
            "expected turn_start event despite log failure"
        );
    }

    // ── 07b: awaiting_model heartbeat ─────────────────────────────────────

    use crate::ai::testing::MockAiClientPending;

    use tokio::sync::Notify;

    #[tokio::test]
    async fn awaiting_model_emitted_before_model_call() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("done")]]);
        let budget = Budget::new(1_000_000);

        execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        let recs = records(dir.path());
        let awaiting: Vec<_> = recs
            .iter()
            .filter(|r| {
                matches!(
                    &r.event,
                    SessionEvent::Progress { stage, .. } if stage == "awaiting_model"
                )
            })
            .collect();

        assert!(
            !awaiting.is_empty(),
            "expected at least one awaiting_model Progress record"
        );
        let first_awaiting_idx = recs
            .iter()
            .position(|r| {
                matches!(
                    &r.event,
                    SessionEvent::Progress { stage, .. } if stage == "awaiting_model"
                )
            })
            .unwrap();
        let completion_idx = recs
            .iter()
            .position(|r| matches!(&r.event, SessionEvent::Completion { .. }))
            .unwrap();
        assert!(
            first_awaiting_idx < completion_idx,
            "awaiting_model must be logged before Completion"
        );
        if let SessionEvent::Progress { turn, .. } = &awaiting[0].event {
            assert_eq!(*turn, 1, "awaiting_model should be for turn 1 (upcoming)");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_reemits_awaiting_model_while_in_flight() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);

        let gate = Arc::new(Notify::new());
        let client = MockAiClientPending::new(vec![token("done")], gate.clone());
        let budget = Budget::new(1_000_000);

        let inp = input();
        let dir_path = dir.path().to_path_buf();
        let dir_path2 = dir_path.clone();

        let handle = tokio::spawn(async move {
            execute_phase(&inp, deps(&client, &registry, &budget, 8, &dir_path)).await
        });

        // Advance time by 3 heartbeat periods, yielding between each so the
        // loop processes the tick and writes its record.
        for _ in 0..3 {
            tokio::time::advance(HEARTBEAT_PERIOD).await;
            tokio::task::yield_now().await;
        }

        // While chat is still in flight, the session log should have
        // 1 pre-call + 3 heartbeat = 4 awaiting_model records.
        let recs_mid = records(&dir_path2);
        let awaiting_mid = recs_mid
            .iter()
            .filter(|r| {
                matches!(
                    &r.event,
                    SessionEvent::Progress { stage, .. } if stage == "awaiting_model"
                )
            })
            .count();

        gate.notify_one();
        handle.await.unwrap().unwrap();

        assert_eq!(
            awaiting_mid, 4,
            "expected exactly 4 awaiting_model records (1 pre-call + 3 ticks), got {awaiting_mid}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn heartbeat_stops_when_model_responds() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);

        let gate = Arc::new(Notify::new());
        let client = MockAiClientPending::new(vec![token("done")], gate.clone());
        let budget = Budget::new(1_000_000);

        let inp = input();
        let dir_path = dir.path().to_path_buf();
        let dir_path2 = dir_path.clone();

        let handle = tokio::spawn(async move {
            execute_phase(&inp, deps(&client, &registry, &budget, 8, &dir_path)).await
        });

        // Advance 2 heartbeat periods, yielding between each.
        for _ in 0..2 {
            tokio::time::advance(HEARTBEAT_PERIOD).await;
            tokio::task::yield_now().await;
        }

        let recs_before = records(&dir_path2);
        let count_before = recs_before
            .iter()
            .filter(|r| {
                matches!(
                    &r.event,
                    SessionEvent::Progress { stage, .. } if stage == "awaiting_model"
                )
            })
            .count();

        // Release the gate so chat resolves.
        gate.notify_one();
        let result = handle.await.unwrap().unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);

        // Advance more time — no new awaiting_model records should appear.
        for _ in 0..3 {
            tokio::time::advance(HEARTBEAT_PERIOD).await;
            tokio::task::yield_now().await;
        }

        let recs_after = records(&dir_path2);
        let count_after = recs_after
            .iter()
            .filter(|r| {
                matches!(
                    &r.event,
                    SessionEvent::Progress { stage, .. } if stage == "awaiting_model"
                )
            })
            .count();

        assert_eq!(
            count_before, count_after,
            "no new awaiting_model records should appear after chat resolves"
        );
    }

    // ── 09: Chat-stream provenance (phase-05b) ─────────────────────────

    #[tokio::test]
    async fn length_finish_rate_is_fraction_of_length_finishes() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");

        // Single turn emitting two Completion events (mock allows arbitrary event sequences)
        let turn1 = vec![
            AiEvent::Completion {
                finish_reason: Some("length".into()),
                model: Some("served-x".into()),
            },
            AiEvent::Completion {
                finish_reason: Some("stop".into()),
                model: None,
            },
            AiEvent::Done(TokenBreakdown::default()),
            AiEvent::Token("done".into()),
        ];
        let client = MockAiClientScript::new(vec![turn1]);
        let verifier = MockFileVerifier::new(vec![]);

        run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            8,
        )
        .await;

        let runs = read_runs(&telem);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].length_finish_rate, Some(0.5));
        assert_eq!(runs[0].served_model, Some("served-x".into()));
    }

    #[tokio::test]
    async fn length_finish_rate_none_when_no_finish_reasons() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");

        // No Completion events at all — only Token + Done
        let turn1 = vec![
            AiEvent::Token("done".into()),
            AiEvent::Done(TokenBreakdown::default()),
        ];
        let client = MockAiClientScript::new(vec![turn1]);
        let verifier = MockFileVerifier::new(vec![]);

        run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            8,
        )
        .await;

        let runs = read_runs(&telem);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].length_finish_rate, None);
        assert_eq!(runs[0].served_model, None);
    }

    #[tokio::test]
    async fn served_model_recorded_from_completion() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");

        let turn1 = vec![
            AiEvent::Completion {
                finish_reason: None,
                model: Some("served-model-v2".into()),
            },
            AiEvent::Done(TokenBreakdown::default()),
            AiEvent::Token("done".into()),
        ];
        let client = MockAiClientScript::new(vec![turn1]);
        let verifier = MockFileVerifier::new(vec![]);

        run_full(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            8,
        )
        .await;

        let runs = read_runs(&telem);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].served_model, Some("served-model-v2".into()));
    }

    #[tokio::test]
    async fn context_window_recorded_from_loop_deps() {
        let dir = TempDir::new().unwrap();
        let telem = dir.path().join("telem");

        let turn1 = vec![
            AiEvent::Completion {
                finish_reason: None,
                model: Some("served-model-v2".into()),
            },
            AiEvent::Done(TokenBreakdown::default()),
            AiEvent::Token("done".into()),
        ];
        let client = MockAiClientScript::new(vec![turn1]);
        let verifier = MockFileVerifier::new(vec![]);

        run_full_with_context_window(
            &dir,
            &client,
            &verifier,
            &NoopRunner,
            &EMPTY_COMMANDS,
            Some(&telem),
            8,
            Some(262_144),
        )
        .await;

        let runs = read_runs(&telem);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].context_window, Some(262_144));
    }
}
