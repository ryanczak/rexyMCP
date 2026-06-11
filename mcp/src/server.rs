use std::path::{Path, PathBuf};

use rmcp::handler::server::ServerHandler;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ProgressNotificationParam, ProgressToken,
    RawContent,
};
use rmcp::service::{RequestContext, RoleServer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use rexymcp_executor::agent::progress::ProgressCallback;
use rexymcp_executor::ai::AiClient;

use crate::cap;
use crate::log_query;
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
pub struct ExecutorHealthParams {
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExecutePhaseOutput {
    pub result: serde_json::Value,
}

pub struct RexyMcpServer {
    pub config_path: PathBuf,
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
                .notify_progress(ProgressNotificationParam {
                    progress_token: token,
                    progress,
                    total: None,
                    message: Some(message),
                })
                .await;
        });
    }
}

/// Inner logic for `execute_phase` — extracted so it can be tested without
/// the rmcp macro wrapper.
pub(crate) async fn execute_phase_inner(
    config_path: &Path,
    params: &ExecutePhaseParams,
    progress: Option<&dyn ProgressCallback>,
) -> Result<ExecutePhaseOutput, String> {
    execute_phase_inner_with_client(config_path, params, progress, None).await
}

/// Testable variant that accepts an optional mock client.
pub(crate) async fn execute_phase_inner_with_client(
    config_path: &Path,
    params: &ExecutePhaseParams,
    progress: Option<&dyn ProgressCallback>,
    test_client: Option<&dyn AiClient>,
) -> Result<ExecutePhaseOutput, String> {
    let cfg = rexymcp_executor::config::Config::load_with_env(config_path)
        .map_err(|e| format!("failed to load config: {}", e))?;

    let phase_doc_path = PathBuf::from(&params.phase_doc_path);
    let repo_path = PathBuf::from(&params.repo_path);

    let standards_path = repo_path.join("docs/dev/STANDARDS.md");
    let standards = std::fs::read_to_string(&standards_path).unwrap_or_default();

    let telemetry_dir = cfg.telemetry.dir.as_deref();

    let result = runner::run_phase(&runner::RunPhaseConfig {
        cfg: &cfg,
        phase_doc_path: &phase_doc_path,
        repo_path: &repo_path,
        standards: &standards,
        model_override: params.model.as_deref(),
        telemetry_dir,
        progress,
        test_client,
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
    pub rows: Vec<scorecard::ScorecardRow>,
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

    let total_runs_considered = runs.len();

    let filter = scorecard::ScorecardFilter {
        tags: params.tags.as_deref().unwrap_or(&[]),
        model: params.model.as_deref(),
        min_runs: params.min_runs.unwrap_or(0),
    };

    let mut rows = scorecard::aggregate(&runs, &filter);

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

                let roots_list: Vec<String> = if context
                    .peer
                    .peer_info()
                    .is_some_and(|ci| ci.capabilities.roots.is_some())
                {
                    match context.peer.list_roots().await {
                        Ok(result) => result.roots.into_iter().map(|r| r.uri).collect(),
                        Err(_) => Vec::new(),
                    }
                } else {
                    Vec::new()
                };

                let project_dir = std::env::var_os("CLAUDE_PROJECT_DIR")
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

                let output =
                    execute_phase_inner(&config_path, &params, progress_callback.as_deref())
                        .await
                        .map_err(|e| rmcp::ErrorData::internal_error(e, None))?;

                let json_str = serde_json::to_string(&output.result).map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("serialization failed: {}", e), None)
                })?;

                Ok(CallToolResult::success(vec![Content::new(
                    RawContent::text(json_str),
                    None,
                )]))
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
        tools.insert(0, rmcp::model::Tool::new(
            "execute_phase",
            "Execute a phase against a target repository. Runs the local LLM through a tool-using loop, verifies edits, runs build/lint/test commands, and returns a structured PhaseResult. The repo_path is corroborated against the MCP client's roots/list and CLAUDE_PROJECT_DIR; a mismatch refuses the call.",
            rmcp::handler::server::tool::schema_for_type::<Parameters<ExecutePhaseParams>>(),
        ));
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
            Some(rmcp::model::Tool::new(
                "execute_phase",
                "Execute a phase against a target repository. Runs the local LLM through a tool-using loop, verifies edits, runs build/lint/test commands, and returns a structured PhaseResult. The repo_path is corroborated against the MCP client's roots/list and CLAUDE_PROJECT_DIR; a mismatch refuses the call.",
                rmcp::handler::server::tool::schema_for_type::<Parameters<ExecutePhaseParams>>(),
            ))
        } else {
            Self::tool_router().get(name).cloned()
        }
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
