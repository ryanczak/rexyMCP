use std::path::{Path, PathBuf};

use rmcp::handler::server::ServerHandler;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ProgressNotificationParam, ProgressToken,
};
use rmcp::service::{RequestContext, RoleServer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use rexymcp_executor::agent::CancelSignal;
use rexymcp_executor::agent::progress::ProgressCallback;
use rexymcp_executor::ai::AiClient;
use rexymcp_executor::config::Config;
use rexymcp_executor::phase::CancelReason;

use crate::cap;
use crate::log_query;
use crate::profile;
use crate::resume;
use crate::roots;
use crate::runner;
use crate::scorecard;

// Per-tool timeout is enforced client-side by Claude Code via .mcp.json
// per-server config (M6), not by the server itself.

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ExecutePhaseParams {
    pub phase_doc_path: String,
    pub repo_path: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ContinuePhaseParams {
    pub phase_doc_path: String,
    pub repo_path: String,
    pub guidance: String,
    pub prior_log_path: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ExecutorHealthParams {
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExecutePhaseOutput {
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetRunStatusParams {
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
/// The immediate `execute_phase` response — the spawned run's handle,
/// polled to completion via `get_run_status`.
pub(crate) struct SpawnedRun {
    pub(crate) run_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GetRunStatusOutput {
    pub run_id: String,
    /// One of: "running", "done", "failed", "unknown".
    pub state: String,
    /// The terminal PhaseResult JSON when state == "done"; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Infra error string when state == "failed"; absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StopPhaseParams {
    /// The `run_id` returned by `execute_phase`.
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StopPhaseOutput {
    /// `true` if a run with that id existed and its cancel was fired; `false`
    /// for an unknown `run_id`.
    pub stopped: bool,
}

/// Inner logic for `get_run_status` — takes the registry + a timeout so it is
/// hermetically testable without the rmcp wrapper.
pub(crate) async fn get_run_status_inner(
    registry: &crate::jobs::JobRegistry,
    params: &GetRunStatusParams,
    timeout: std::time::Duration,
) -> GetRunStatusOutput {
    let run_id = params.run_id.clone();
    match registry.await_terminal(&run_id, timeout).await {
        None => GetRunStatusOutput {
            run_id,
            state: "unknown".into(),
            result: None,
            error: None,
        },
        Some(crate::jobs::RunState::Running) => GetRunStatusOutput {
            run_id,
            state: "running".into(),
            result: None,
            error: None,
        },
        Some(crate::jobs::RunState::Complete(json)) => GetRunStatusOutput {
            run_id,
            state: "done".into(),
            result: Some(json),
            error: None,
        },
        Some(crate::jobs::RunState::Failed(e)) => GetRunStatusOutput {
            run_id,
            state: "failed".into(),
            result: None,
            error: Some(e),
        },
    }
}

pub struct RexyMcpServer {
    pub config_path: PathBuf,
    pub runs: std::sync::Arc<crate::jobs::JobRegistry>,
}

impl RexyMcpServer {
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config_path,
            runs: std::sync::Arc::new(crate::jobs::JobRegistry::new()),
        }
    }
}

/// A `ProgressCallback` that fires MCP `notifications/progress` via the
/// rmcp peer captured at request time.
pub(crate) struct McpProgressNotifier {
    peer: rmcp::service::Peer<RoleServer>,
    progress_token: ProgressToken,
}

impl ProgressCallback for McpProgressNotifier {
    fn on_progress(&self, event: &rexymcp_executor::agent::progress::ProgressEvent) {
        let token = self.progress_token.clone();
        let peer = self.peer.clone();
        let progress = event.turn as f64;
        let message = event.message.clone();
        tokio::spawn(async move {
            let _ = peer
                .notify_progress(
                    ProgressNotificationParam::new(token, progress).with_message(message),
                )
                .await;
        });
    }
}

/// Build a hand-rolled tool's success result: `structured_content` plus the
/// spec-recommended back-compat text block (`CallToolResult::structured`
/// emits both from one `Value`).
fn structured_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, rmcp::ErrorData> {
    let json = serde_json::to_value(value).map_err(|e| {
        rmcp::ErrorData::internal_error(format!("serialization failed: {}", e), None)
    })?;
    Ok(CallToolResult::structured(json))
}

/// Inner logic for `execute_phase` — extracted so it can be tested without
/// the rmcp macro wrapper.
pub(crate) async fn execute_phase_inner(
    config_path: &Path,
    params: &ExecutePhaseParams,
    progress: Option<&dyn ProgressCallback>,
    cancel: CancelSignal,
) -> Result<ExecutePhaseOutput, String> {
    execute_phase_inner_with_client(config_path, params, progress, None, cancel).await
}

/// Testable variant that accepts an optional mock client.
pub(crate) async fn execute_phase_inner_with_client(
    config_path: &Path,
    params: &ExecutePhaseParams,
    progress: Option<&dyn ProgressCallback>,
    test_client: Option<&dyn AiClient>,
    cancel: CancelSignal,
) -> Result<ExecutePhaseOutput, String> {
    let cfg = rexymcp_executor::config::Config::load_with_env(config_path)
        .map_err(|e| format!("failed to load config: {}", e))?;

    let phase_doc_path = PathBuf::from(&params.phase_doc_path);
    let repo_path = PathBuf::from(&params.repo_path);

    let standards_path = repo_path.join("docs/dev/STANDARDS.md");
    let standards = std::fs::read_to_string(&standards_path).unwrap_or_default();

    let telemetry_dir = cfg.telemetry.dir.as_deref();

    let project_id = rexymcp_executor::config::Config::load(&repo_path.join("rexymcp.toml"))
        .ok()
        .and_then(|c| c.project.id);

    let result = runner::run_phase(&runner::RunPhaseConfig {
        cfg: &cfg,
        phase_doc_path: &phase_doc_path,
        repo_path: &repo_path,
        standards: &standards,
        model_override: params.model.as_deref(),
        telemetry_dir,
        progress,
        project_id,
        test_client,
        resume: None,
        cancel,
    })
    .await
    .map_err(|e| e.to_string())?;

    let capped = cap::cap_phase_result(result);

    let json = serde_json::to_value(&capped)
        .map_err(|e| format!("failed to serialize PhaseResult: {}", e))?;

    Ok(ExecutePhaseOutput { result: json })
}

/// Inner logic for `continue_phase` — resumes a failed phase from a fresh
/// briefing-seeded context.
pub(crate) async fn continue_phase_inner(
    config_path: &Path,
    params: &ContinuePhaseParams,
    progress: Option<&dyn ProgressCallback>,
) -> Result<ExecutePhaseOutput, String> {
    let cfg = rexymcp_executor::config::Config::load_with_env(config_path)
        .map_err(|e| format!("failed to load config: {}", e))?;

    let phase_doc_path = PathBuf::from(&params.phase_doc_path);
    let repo_path = PathBuf::from(&params.repo_path);

    let standards_path = repo_path.join("docs/dev/STANDARDS.md");
    let standards = std::fs::read_to_string(&standards_path).unwrap_or_default();

    let telemetry_dir = cfg.telemetry.dir.as_deref();

    let project_id = rexymcp_executor::config::Config::load(&repo_path.join("rexymcp.toml"))
        .ok()
        .and_then(|c| c.project.id);

    let ctx = resume::build_resume_context(
        &params.guidance,
        params.prior_log_path.as_deref().map(Path::new),
        &repo_path,
        &rexymcp_executor::agent::command::RealCommandRunner,
    )
    .await;

    let result = runner::run_phase(&runner::RunPhaseConfig {
        cfg: &cfg,
        phase_doc_path: &phase_doc_path,
        repo_path: &repo_path,
        standards: &standards,
        model_override: params.model.as_deref(),
        telemetry_dir,
        progress,
        project_id,
        test_client: None,
        resume: Some(ctx),
        cancel: CancelSignal::never(),
    })
    .await
    .map_err(|e| e.to_string())?;

    let capped = cap::cap_phase_result(result);

    let json = serde_json::to_value(&capped)
        .map_err(|e| format!("failed to serialize PhaseResult: {}", e))?;

    Ok(ExecutePhaseOutput { result: json })
}

/// Inner logic for `executor_health` — extracted so it can be tested without
/// the rmcp macro wrapper.
pub(crate) async fn executor_health_inner(
    config_path: &Path,
    params: &ExecutorHealthParams,
) -> Result<rexymcp_executor::health::Health, String> {
    let mut cfg = rexymcp_executor::config::Config::load_with_env(config_path)
        .map_err(|e| format!("failed to load config: {}", e))?;

    if let Some(url) = &params.base_url {
        cfg.executor.base_url = url.clone();
    }

    let health = rexymcp_executor::health::check(&cfg.executor).await;
    Ok(health)
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ExecutorLogSearchParams {
    pub log_path: String,
    pub event_type: Option<String>,
    pub tool_name: Option<String>,
    pub query_text: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ExecutorLogTailParams {
    pub log_path: String,
    pub n: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetTurnParams {
    pub log_path: String,
    pub turn: usize,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LogQueryOutput {
    /// The matching records as a JSON array. Each record is a serialized
    /// SessionRecord. Wrapped in serde_json::Value so SessionRecord doesn't
    /// need JsonSchema (mirrors ExecutePhaseOutput's approach — see phase-02).
    pub records: serde_json::Value,
    /// True when the result was clipped by a per-tool count cap, so Claude
    /// knows to refine its query if it cares.
    pub truncated: bool,
}

/// Inner logic for `executor_log_search`.
pub(crate) fn executor_log_search_inner(
    params: &ExecutorLogSearchParams,
) -> Result<LogQueryOutput, String> {
    let path = PathBuf::from(&params.log_path);
    let records = rexymcp_executor::store::sessions::jsonl::read_session_log(&path)
        .map_err(|e| format!("failed to read session log: {}", e))?;

    let limit = params.limit.unwrap_or(log_query::SEARCH_DEFAULT_LIMIT);

    let filter = log_query::SearchFilter {
        event_type: params.event_type.as_deref(),
        tool_name: params.tool_name.as_deref(),
        query_text: params.query_text.as_deref(),
    };

    let matched_count = {
        let filtered = log_query::search(&records, &filter, usize::MAX);
        filtered.len()
    };

    let results = log_query::search(&records, &filter, limit);
    let capped_results: Vec<_> = results.into_iter().map(cap::cap_session_record).collect();

    let truncated = capped_results.len() < matched_count;

    let json = serde_json::to_value(&capped_results)
        .map_err(|e| format!("failed to serialize records: {}", e))?;

    Ok(LogQueryOutput {
        records: json,
        truncated,
    })
}

/// Inner logic for `executor_log_tail`.
pub(crate) fn executor_log_tail_inner(
    params: &ExecutorLogTailParams,
) -> Result<LogQueryOutput, String> {
    let path = PathBuf::from(&params.log_path);
    let records = rexymcp_executor::store::sessions::jsonl::read_session_log(&path)
        .map_err(|e| format!("failed to read session log: {}", e))?;

    let n = params.n.unwrap_or(log_query::TAIL_DEFAULT_N);

    let total = records.len();
    let results = log_query::tail(&records, n);
    let capped_results: Vec<_> = results.into_iter().map(cap::cap_session_record).collect();

    let truncated = capped_results.len() < total;

    let json = serde_json::to_value(&capped_results)
        .map_err(|e| format!("failed to serialize records: {}", e))?;

    Ok(LogQueryOutput {
        records: json,
        truncated,
    })
}

/// Inner logic for `get_turn`.
pub(crate) fn get_turn_inner(params: &GetTurnParams) -> Result<LogQueryOutput, String> {
    let path = PathBuf::from(&params.log_path);
    let records = rexymcp_executor::store::sessions::jsonl::read_session_log(&path)
        .map_err(|e| format!("failed to read session log: {}", e))?;

    let results = log_query::get_turn(&records, params.turn);

    let json = serde_json::to_value(&results)
        .map_err(|e| format!("failed to serialize records: {}", e))?;

    Ok(LogQueryOutput {
        records: json,
        truncated: false,
    })
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ModelScorecardParams {
    /// Tags the run must contain (AND-ed). Empty = no filter.
    pub tags: Option<Vec<String>>,
    /// Restrict to one model. `None` = all models.
    pub model: Option<String>,
    /// Drop buckets with fewer than this many runs. `None` = 0.
    pub min_runs: Option<usize>,
    /// Override the cross-project `phase_runs.jsonl` path. `None` = resolve
    /// from `cfg.telemetry.dir`.
    pub telemetry_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ModelScorecardOutput {
    pub rows: Vec<scorecard::ScorecardBucket>,
    pub total_runs_considered: usize,
    /// True iff the row count was clipped by `MAX_ROWS`.
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ModelProfileParams {
    /// Tags the run must contain (AND-ed). Empty = no filter.
    pub tags: Option<Vec<String>>,
    pub model: Option<String>,
    pub min_runs: Option<usize>,
    /// Override the cross-project `phase_runs.jsonl` path. `None` = resolve
    /// from `cfg.telemetry.dir`.
    pub telemetry_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ModelProfileOutput {
    pub rows: Vec<profile::ModelProfile>,
    pub total_runs_considered: usize,
    /// True iff the row count was clipped by `MAX_ROWS`.
    pub truncated: bool,
}

/// Inner logic for `model_scorecard`.
pub(crate) fn model_scorecard_inner(
    config_path: &Path,
    params: &ModelScorecardParams,
) -> Result<ModelScorecardOutput, String> {
    let cfg = rexymcp_executor::config::Config::load_with_env(config_path)
        .map_err(|e| format!("failed to load config: {}", e))?;

    let telemetry_file = if let Some(ref p) = params.telemetry_path {
        PathBuf::from(p)
    } else if let Some(ref dir) = cfg.telemetry.dir {
        dir.join("phase_runs.jsonl")
    } else {
        return Err(
            "telemetry disabled: cfg.telemetry.dir not set and no telemetry_path provided"
                .to_string(),
        );
    };

    let runs =
        rexymcp_executor::store::telemetry::read(&telemetry_file).map_err(|e| e.to_string())?;
    let reviews = rexymcp_executor::store::telemetry::read_reviews(&telemetry_file)
        .map_err(|e| e.to_string())?;
    let runs = rexymcp_executor::store::telemetry::fold_reviews(runs, &reviews);

    let total_runs_considered = runs.len();

    let filter = scorecard::ScorecardFilter {
        tags: params.tags.as_deref().unwrap_or(&[]),
        model: params.model.as_deref(),
        min_runs: params.min_runs.unwrap_or(0),
    };

    let mut rows =
        scorecard::aggregate_scorecard(&runs, scorecard::ScorecardDimension::Tag, &filter);

    let truncated = rows.len() > scorecard::MAX_ROWS;
    if truncated {
        rows.truncate(scorecard::MAX_ROWS);
    }

    Ok(ModelScorecardOutput {
        rows,
        total_runs_considered,
        truncated,
    })
}

/// Inner logic for `model_profile`.
pub(crate) fn model_profile_inner(
    config_path: &Path,
    params: &ModelProfileParams,
) -> Result<ModelProfileOutput, String> {
    let cfg =
        Config::load_with_env(config_path).map_err(|e| format!("failed to load config: {}", e))?;

    let telemetry_file = if let Some(ref p) = params.telemetry_path {
        PathBuf::from(p)
    } else if let Some(ref dir) = cfg.telemetry.dir {
        dir.join("phase_runs.jsonl")
    } else {
        return Err(
            "telemetry disabled: cfg.telemetry.dir not set and no telemetry_path provided"
                .to_string(),
        );
    };

    let runs =
        rexymcp_executor::store::telemetry::read(&telemetry_file).map_err(|e| e.to_string())?;
    let reviews = rexymcp_executor::store::telemetry::read_reviews(&telemetry_file)
        .map_err(|e| e.to_string())?;

    let total_runs_considered = runs.len();

    let filter = scorecard::ScorecardFilter {
        tags: params.tags.as_deref().unwrap_or(&[]),
        model: params.model.as_deref(),
        min_runs: params.min_runs.unwrap_or(0),
    };

    // aggregate_profiles folds internally — pass raw runs + reviews (no fold_reviews).
    let mut rows = profile::aggregate_profiles(&runs, &reviews, &filter);

    let truncated = rows.len() > scorecard::MAX_ROWS;
    if truncated {
        rows.truncate(scorecard::MAX_ROWS);
    }

    Ok(ModelProfileOutput {
        rows,
        total_runs_considered,
        truncated,
    })
}

#[rmcp::tool_router]
impl RexyMcpServer {
    #[rmcp::tool(
        description = "Check connectivity to the configured LLM endpoint and list available models."
    )]
    async fn executor_health(
        &self,
        Parameters(params): Parameters<ExecutorHealthParams>,
    ) -> Result<Json<rexymcp_executor::health::Health>, String> {
        executor_health_inner(&self.config_path, &params)
            .await
            .map(Json)
    }

    #[rmcp::tool(
        description = "Search the session JSONL log for matching records. Filters by event_type (exact match on snake_case discriminant), tool_name (substring match on Parsed/ToolResult events only), and query_text (substring match on serialized JSON). All filters AND together. Results are capped per-record and limited in count (default 20, max 50). Substring matching only, not regex."
    )]
    async fn executor_log_search(
        &self,
        Parameters(params): Parameters<ExecutorLogSearchParams>,
    ) -> Result<Json<LogQueryOutput>, String> {
        executor_log_search_inner(&params).map(Json)
    }

    #[rmcp::tool(
        description = "Return the last N records from the session JSONL log, each capped per-field. Default N is 10, max is 50. The log_path is the path from PhaseResult.log_path (returned by execute_phase). No path confinement — the caller (architect) is trusted."
    )]
    async fn executor_log_tail(
        &self,
        Parameters(params): Parameters<ExecutorLogTailParams>,
    ) -> Result<Json<LogQueryOutput>, String> {
        executor_log_tail_inner(&params).map(Json)
    }

    #[rmcp::tool(
        description = "Return all records for a single turn number, uncapped per-field. This is the one escape hatch for raw detail, scoped to one turn. The log_path is the path from PhaseResult.log_path (returned by execute_phase)."
    )]
    async fn get_turn(
        &self,
        Parameters(params): Parameters<GetTurnParams>,
    ) -> Result<Json<LogQueryOutput>, String> {
        get_turn_inner(&params).map(Json)
    }

    #[rmcp::tool(
        description = "Aggregate the cross-project PhaseRun telemetry into a model × tag competency matrix. Returns per-bucket gates pass rate, reliability means (parse-failure / repairs / tool-success / verifier-retries), efficiency (turns / wall-clock), escalation rate, and supervision metrics (approved_first_try_rate, bounces_to_approval_mean). Filter by tags (AND semantics, exact match), model, or min_runs. Output capped at 500 rows."
    )]
    async fn model_scorecard(
        &self,
        Parameters(params): Parameters<ModelScorecardParams>,
    ) -> Result<Json<ModelScorecardOutput>, String> {
        model_scorecard_inner(&self.config_path, &params).map(Json)
    }

    #[rmcp::tool(
        description = "Aggregate the cross-project PhaseRun telemetry into a per-(model, tag) capability profile. Returns strengths (gate-pass rate, approved-first-try rate, reliability means) and ranked failure classes with counts. Non-attributable classes (spec_bug, infra_blip) are separated from the model's real weaknesses. Filterable by tags (AND semantics), model, min_runs. Output capped at 500 rows."
    )]
    async fn model_profile(
        &self,
        Parameters(params): Parameters<ModelProfileParams>,
    ) -> Result<Json<ModelProfileOutput>, String> {
        model_profile_inner(&self.config_path, &params).map(Json)
    }

    #[rmcp::tool(
        description = "Poll a spawned execute_phase run by run_id. Bounded long-poll (~15s): returns {state:\"running\"} while the run is in flight, {state:\"done\", result: PhaseResult} once it completes / hard-fails / is cancelled, {state:\"failed\", error} on an infrastructure error, or {state:\"unknown\"} for an unrecognized run_id. Re-poll while running."
    )]
    async fn get_run_status(
        &self,
        Parameters(params): Parameters<GetRunStatusParams>,
    ) -> Result<Json<GetRunStatusOutput>, String> {
        let out =
            get_run_status_inner(&self.runs, &params, crate::jobs::RUN_STATUS_POLL_TIMEOUT).await;
        Ok(Json(out))
    }

    #[rmcp::tool(
        description = "Stop a spawned execute_phase run by run_id: fires the run's cooperative cancel signal so it aborts at the next turn boundary (or mid model-stream) and returns a PhaseResult with status \"cancelled\", cancellation.reason \"claude_stop\", and the partial diff (working tree left dirty). Returns {stopped:true} if the run_id was known, {stopped:false} if not. The cancel is cooperative and asynchronous — poll get_run_status to observe the terminal cancelled result."
    )]
    async fn stop_phase(
        &self,
        Parameters(params): Parameters<StopPhaseParams>,
    ) -> Result<Json<StopPhaseOutput>, String> {
        let stopped = self
            .runs
            .request_stop(&params.run_id, CancelReason::ClaudeStop);
        Ok(Json(StopPhaseOutput { stopped }))
    }
}

impl ServerHandler for RexyMcpServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        // Declare the `tools` capability in the initialize handshake. Without
        // this, the default ServerInfo advertises no capabilities and a
        // spec-compliant client (Claude Code) never calls tools/list, so the
        // tools appear missing even though list_tools/call_tool are wired up.
        let mut info = rmcp::model::ServerInfo::default();
        info.capabilities = rmcp::model::ServerCapabilities::builder()
            .enable_tools()
            .build();
        info
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, rmcp::ErrorData>>
    + rmcp::service::MaybeSendFuture
    + '_ {
        let router = Self::tool_router();
        let config_path = self.config_path.clone();
        let runs = self.runs.clone();

        async move {
            if request.name == "execute_phase" {
                let params: ExecutePhaseParams = serde_json::from_value(serde_json::Value::Object(
                    request.arguments.unwrap_or_default(),
                ))
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(
                        format!("invalid execute_phase parameters: {}", e),
                        None,
                    )
                })?;

                let repo_path = PathBuf::from(&params.repo_path);

                let roots_list: Vec<String> = Vec::new();

                let project_dir = std::env::var_os("CLAUDE_PROJECT_DIR")
                    .or_else(|| std::env::var_os("ANTIGRAVITY_PROJECT_DIR"))
                    .map(PathBuf::from)
                    .filter(|p| !p.as_os_str().is_empty());

                match roots::corroborate(&repo_path, &roots_list, project_dir.as_deref()) {
                    roots::Corroboration::Matched(_) | roots::Corroboration::NoSources => {}
                    roots::Corroboration::Mismatch { .. } => {
                        return Err(rmcp::ErrorData::invalid_params(
                            roots::format_mismatch_error(
                                &repo_path,
                                &roots_list,
                                project_dir.as_deref(),
                            ),
                            None,
                        ));
                    }
                }

                let progress_token = request.meta.as_ref().and_then(|m| m.get_progress_token());
                // The MCP spec only permits notifications/progress for a request that carried
                // a progressToken; without one, live progress can't fire (the logged Progress
                // records and `rexymcp status` are unaffected — they don't need the token).
                let progress_callback: Option<Box<dyn ProgressCallback>> =
                    progress_token.map(|token| {
                        Box::new(McpProgressNotifier {
                            peer: context.peer.clone(),
                            progress_token: token,
                        }) as Box<dyn ProgressCallback>
                    });

                let run_id = crate::jobs::new_run_id();
                let (cancel_handle, cancel_signal) = CancelSignal::new();
                let config_path_owned = config_path.clone();
                let params_owned = params.clone();
                let work = async move {
                    execute_phase_inner(
                        &config_path_owned,
                        &params_owned,
                        progress_callback.as_deref(),
                        cancel_signal,
                    )
                    .await
                    .map(|o| o.result)
                };
                crate::jobs::spawn_run(runs.clone(), run_id.clone(), cancel_handle, work);
                tokio::spawn(crate::stop_watcher::watch_stop_sentinel(
                    repo_path.clone(),
                    runs.clone(),
                    run_id.clone(),
                    crate::stop_watcher::STOP_POLL_INTERVAL,
                ));

                structured_result(&SpawnedRun { run_id })
            } else if request.name == "continue_phase" {
                let params: ContinuePhaseParams = serde_json::from_value(
                    serde_json::Value::Object(request.arguments.unwrap_or_default()),
                )
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(
                        format!("invalid continue_phase parameters: {}", e),
                        None,
                    )
                })?;

                let repo_path = PathBuf::from(&params.repo_path);

                let roots_list: Vec<String> = Vec::new();

                let project_dir = std::env::var_os("CLAUDE_PROJECT_DIR")
                    .or_else(|| std::env::var_os("ANTIGRAVITY_PROJECT_DIR"))
                    .map(PathBuf::from)
                    .filter(|p| !p.as_os_str().is_empty());

                match roots::corroborate(&repo_path, &roots_list, project_dir.as_deref()) {
                    roots::Corroboration::Matched(_) | roots::Corroboration::NoSources => {}
                    roots::Corroboration::Mismatch { .. } => {
                        return Err(rmcp::ErrorData::invalid_params(
                            roots::format_mismatch_error(
                                &repo_path,
                                &roots_list,
                                project_dir.as_deref(),
                            ),
                            None,
                        ));
                    }
                }

                let progress_token = request.meta.as_ref().and_then(|m| m.get_progress_token());
                let progress_callback: Option<Box<dyn ProgressCallback>> =
                    progress_token.map(|token| {
                        Box::new(McpProgressNotifier {
                            peer: context.peer.clone(),
                            progress_token: token,
                        }) as Box<dyn ProgressCallback>
                    });

                let output =
                    continue_phase_inner(&config_path, &params, progress_callback.as_deref())
                        .await
                        .map_err(|e| rmcp::ErrorData::internal_error(e, None))?;

                structured_result(&output.result)
            } else {
                let ctx = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
                router.call(ctx).await
            }
        }
    }

    async fn list_tools(
        &self,
        request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let mut tools = Self::tool_router().list_all();
        tools.insert(0, execute_phase_tool());
        tools.insert(1, continue_phase_tool());
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        let next_cursor = request.and_then(|r| r.cursor);
        Ok(rmcp::model::ListToolsResult {
            tools,
            next_cursor,
            meta: None,
        })
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        if name == "execute_phase" {
            Some(execute_phase_tool())
        } else if name == "continue_phase" {
            Some(continue_phase_tool())
        } else {
            Self::tool_router().get(name).cloned()
        }
    }
}

fn execute_phase_tool() -> rmcp::model::Tool {
    let tool = rmcp::model::Tool::new(
        "execute_phase",
        "Execute a phase against a target repository. Spawns the run inside the serve process and returns { run_id } immediately; poll it to completion with get_run_status. The repo_path is corroborated against the MCP client's roots/list and CLAUDE_PROJECT_DIR; a mismatch refuses the call.",
        rmcp::handler::server::tool::schema_for_type::<Parameters<ExecutePhaseParams>>(),
    );
    match rmcp::handler::server::tool::schema_for_output::<SpawnedRun>() {
        Ok(schema) => tool.with_raw_output_schema(schema),
        Err(_) => tool,
    }
}

fn continue_phase_tool() -> rmcp::model::Tool {
    let tool = rmcp::model::Tool::new(
        "continue_phase",
        "Resume a non-complete phase from a fresh briefing-seeded context. The architect provides distilled guidance and optionally the prior run's session log path; the tool restores task states and appends a resume preamble to the phase doc. The repo_path is corroborated against the MCP client's roots/list and CLAUDE_PROJECT_DIR; a mismatch refuses the call.",
        rmcp::handler::server::tool::schema_for_type::<Parameters<ContinuePhaseParams>>(),
    );
    match rmcp::handler::server::tool::schema_for_output::<rexymcp_executor::phase::PhaseResult>() {
        Ok(schema) => tool.with_raw_output_schema(schema),
        Err(_) => tool,
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
