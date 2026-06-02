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
    /// Restrict to benchmark runs of this suite. Wins over `production_only`
    /// when both are set.
    pub bench_suite: Option<String>,
    /// When `Some(true)` (and `bench_suite` unset), restrict to production runs.
    pub production_only: Option<bool>,
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

    let source = if let Some(suite) = params.bench_suite.clone() {
        scorecard::SourceFilter::Suite(suite)
    } else if params.production_only == Some(true) {
        scorecard::SourceFilter::ProductionOnly
    } else {
        scorecard::SourceFilter::Any
    };

    let filter = scorecard::ScorecardFilter {
        tags: params.tags.as_deref().unwrap_or(&[]),
        model: params.model.as_deref(),
        min_runs: params.min_runs.unwrap_or(0),
        source,
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
        description = "Aggregate the cross-project PhaseRun telemetry into a model × tag competency matrix. Returns per-bucket gates pass rate, reliability means (parse-failure / repairs / tool-success / verifier-retries), efficiency (turns / wall-clock), escalation rate, and supervision metrics (approved_first_try_rate, bounces_to_approval_mean). Filter by tags (AND semantics, exact match), model, min_runs, or run provenance with production_only or bench_suite (bench_suite wins). Output capped at 500 rows."
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
                    roots::Corroboration::Matched(_) => {
                        let _ = ();
                    }
                    roots::Corroboration::NoSources => {
                        eprintln!(
                            "execute_phase: no roots or CLAUDE_PROJECT_DIR available for \
                             corroboration of repo_path={}",
                            repo_path.display()
                        );
                    }
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
                // Diagnostic: whether the client opted into live progress. The
                // MCP spec only permits notifications/progress for a request
                // that carried a progressToken, so a "no token" line here
                // explains absent live status (the logged Progress records and
                // `rexymcp status` are unaffected — they don't need the token).
                eprintln!(
                    "execute_phase: client progress_token {}",
                    if progress_token.is_some() {
                        "present (live notifications enabled)"
                    } else {
                        "absent (no live notifications; use `rexymcp status`)"
                    }
                );
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_test_config(temp_dir: &TempDir) -> PathBuf {
        let config_path = temp_dir.path().join("rexymcp.toml");
        std::fs::write(
            &config_path,
            r#"[executor]
provider = "openai"
model = "test-model"
base_url = "http://127.0.0.1:1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
escalation_slots = 1
"#,
        )
        .unwrap();
        config_path
    }

    fn make_phase_doc(temp_dir: &TempDir) -> PathBuf {
        let phase_path = temp_dir.path().join("phase-01-test.md");
        std::fs::write(
            &phase_path,
            "# Phase 01: Test\n\n**Tags:** language=rust, kind=test, size=s\n\n## Goal\n\nTest goal.\n\n## Acceptance criteria\n\n- [ ] It runs.\n",
        )
        .unwrap();
        phase_path
    }

    #[tokio::test]
    async fn executor_health_returns_unreachable_for_bad_url() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = make_test_config(&temp_dir);

        let params = ExecutorHealthParams { base_url: None };
        let health = executor_health_inner(&config_path, &params).await;

        assert!(health.is_ok(), "expected Ok, got Err: {:?}", health);
        let health = health.unwrap();
        assert!(!health.reachable);
        assert!(health.models.is_empty());
    }

    #[tokio::test]
    async fn executor_health_applies_base_url_override() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = make_test_config(&temp_dir);

        let params = ExecutorHealthParams {
            base_url: Some("http://127.0.0.1:99999".to_string()),
        };
        let health = executor_health_inner(&config_path, &params).await;

        assert!(health.is_ok());
        let health = health.unwrap();
        assert!(!health.reachable);
        assert_eq!(health.base_url, "http://127.0.0.1:99999");
    }

    #[tokio::test]
    async fn execute_phase_returns_error_for_missing_phase_doc() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = make_test_config(&temp_dir);

        let params = ExecutePhaseParams {
            phase_doc_path: "/nonexistent/phase.md".to_string(),
            repo_path: temp_dir.path().to_str().unwrap().to_string(),
            model: None,
        };
        let result = execute_phase_inner(&config_path, &params, None).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_phase_returns_error_for_missing_repo() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = make_test_config(&temp_dir);
        let phase_path = make_phase_doc(&temp_dir);

        let params = ExecutePhaseParams {
            phase_doc_path: phase_path.to_str().unwrap().to_string(),
            repo_path: "/nonexistent/repo".to_string(),
            model: None,
        };
        let result = execute_phase_inner(&config_path, &params, None).await;

        assert!(result.is_err());
    }

    fn write_fixture_log(temp_dir: &TempDir) -> PathBuf {
        let log_path = temp_dir.path().join("session-abcd1234.jsonl");
        let lines = [
            r#"{"ts":1717000000000,"turn":0,"event":{"event_type":"session_start","session_id":"s1","model":"test","phase":"p1"}}"#,
            r#"{"ts":1717000001000,"turn":1,"event":{"event_type":"prompt","rendered":"Do something useful."}}"#,
            r#"{"ts":1717000002000,"turn":1,"event":{"event_type":"completion","raw":"read_file src/main.rs"}}"#,
            r#"{"ts":1717000003000,"turn":1,"event":{"event_type":"tool_result","name":"read_file","succeeded":true,"output_preview":"fn main() {}"}}"#,
            r#"{"ts":1717000004000,"turn":2,"event":{"event_type":"tool_result","name":"write_file","succeeded":true,"output_preview":"wrote 10 bytes"}}"#,
            r#"{"ts":1717000005000,"turn":2,"event":{"event_type":"session_end","status":"success","turns":2}}"#,
        ];
        std::fs::write(&log_path, lines.join("\n") + "\n").unwrap();
        log_path
    }

    #[test]
    fn executor_log_search_returns_matching_records() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = write_fixture_log(&temp_dir);

        let params = ExecutorLogSearchParams {
            log_path: log_path.to_str().unwrap().to_string(),
            event_type: Some("tool_result".to_string()),
            tool_name: None,
            query_text: None,
            limit: None,
        };
        let result = executor_log_search_inner(&params).unwrap();

        let records = result.records.as_array().unwrap();
        assert_eq!(records.len(), 2);
        assert!(!result.truncated);
    }

    #[test]
    fn executor_log_search_filter_by_tool_name() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = write_fixture_log(&temp_dir);

        let params = ExecutorLogSearchParams {
            log_path: log_path.to_str().unwrap().to_string(),
            event_type: None,
            tool_name: Some("read_file".to_string()),
            query_text: None,
            limit: None,
        };
        let result = executor_log_search_inner(&params).unwrap();

        let records = result.records.as_array().unwrap();
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn executor_log_search_filter_by_query_text() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = write_fixture_log(&temp_dir);

        let params = ExecutorLogSearchParams {
            log_path: log_path.to_str().unwrap().to_string(),
            event_type: None,
            tool_name: None,
            query_text: Some("fn main()".to_string()),
            limit: None,
        };
        let result = executor_log_search_inner(&params).unwrap();

        let records = result.records.as_array().unwrap();
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn executor_log_search_returns_empty_for_missing_file() {
        let params = ExecutorLogSearchParams {
            log_path: "/nonexistent/session.jsonl".to_string(),
            event_type: None,
            tool_name: None,
            query_text: None,
            limit: None,
        };
        let result = executor_log_search_inner(&params).unwrap();

        let records = result.records.as_array().unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn executor_log_tail_returns_last_n_records() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = write_fixture_log(&temp_dir);

        let params = ExecutorLogTailParams {
            log_path: log_path.to_str().unwrap().to_string(),
            n: Some(3),
        };
        let result = executor_log_tail_inner(&params).unwrap();

        let records = result.records.as_array().unwrap();
        assert_eq!(records.len(), 3);
        assert!(result.truncated);
    }

    #[test]
    fn executor_log_tail_default_n() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = write_fixture_log(&temp_dir);

        let params = ExecutorLogTailParams {
            log_path: log_path.to_str().unwrap().to_string(),
            n: None,
        };
        let result = executor_log_tail_inner(&params).unwrap();

        let records = result.records.as_array().unwrap();
        assert_eq!(records.len(), 6);
        assert!(!result.truncated);
    }

    #[test]
    fn executor_log_tail_clamped_to_max() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = write_fixture_log(&temp_dir);

        let params = ExecutorLogTailParams {
            log_path: log_path.to_str().unwrap().to_string(),
            n: Some(1000),
        };
        let result = executor_log_tail_inner(&params).unwrap();

        let records = result.records.as_array().unwrap();
        assert!(records.len() <= log_query::TAIL_MAX_N);
    }

    #[test]
    fn executor_log_tail_returns_empty_for_missing_file() {
        let params = ExecutorLogTailParams {
            log_path: "/nonexistent/session.jsonl".to_string(),
            n: None,
        };
        let result = executor_log_tail_inner(&params).unwrap();

        let records = result.records.as_array().unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn get_turn_returns_all_events_for_turn() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = write_fixture_log(&temp_dir);

        let params = GetTurnParams {
            log_path: log_path.to_str().unwrap().to_string(),
            turn: 1,
        };
        let result = get_turn_inner(&params).unwrap();

        let records = result.records.as_array().unwrap();
        assert_eq!(records.len(), 3);
        assert!(!result.truncated);
    }

    #[test]
    fn get_turn_empty_when_no_records() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = write_fixture_log(&temp_dir);

        let params = GetTurnParams {
            log_path: log_path.to_str().unwrap().to_string(),
            turn: 999,
        };
        let result = get_turn_inner(&params).unwrap();

        let records = result.records.as_array().unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn get_turn_returns_empty_for_missing_file() {
        let params = GetTurnParams {
            log_path: "/nonexistent/session.jsonl".to_string(),
            turn: 1,
        };
        let result = get_turn_inner(&params).unwrap();

        let records = result.records.as_array().unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn get_turn_uncapped_vs_tail_capped() {
        let temp_dir = TempDir::new().unwrap();
        let huge = "H".repeat(100_000);
        let log_path = temp_dir.path().join("session-huge.jsonl");
        let line = serde_json::json!({
            "ts": 1717000000000i64,
            "turn": 1,
            "event": {
                "event_type": "prompt",
                "rendered": huge.clone()
            }
        });
        std::fs::write(&log_path, format!("{}\n", line)).unwrap();

        let turn_params = GetTurnParams {
            log_path: log_path.to_str().unwrap().to_string(),
            turn: 1,
        };
        let turn_result = get_turn_inner(&turn_params).unwrap();
        let turn_records = turn_result.records.as_array().unwrap();
        let rendered = turn_records[0]["event"]["rendered"].as_str().unwrap();
        assert_eq!(rendered.len(), 100_000, "get_turn must not cap");

        let tail_params = ExecutorLogTailParams {
            log_path: log_path.to_str().unwrap().to_string(),
            n: Some(1),
        };
        let tail_result = executor_log_tail_inner(&tail_params).unwrap();
        let tail_records = tail_result.records.as_array().unwrap();
        let rendered = tail_records[0]["event"]["rendered"].as_str().unwrap();
        assert!(rendered.len() < 100_000, "tail must cap");
        assert!(
            rendered.contains("[truncated:"),
            "tail must include truncation marker"
        );
    }

    #[test]
    fn executor_log_search_directory_path_returns_error() {
        let temp_dir = TempDir::new().unwrap();

        let params = ExecutorLogSearchParams {
            log_path: temp_dir.path().to_str().unwrap().to_string(),
            event_type: None,
            tool_name: None,
            query_text: None,
            limit: None,
        };
        let result = executor_log_search_inner(&params);

        assert!(result.is_err());
    }

    fn make_config_with_telemetry(temp_dir: &TempDir) -> PathBuf {
        let config_path = temp_dir.path().join("rexymcp.toml");
        let telemetry_dir = temp_dir.path().join("telemetry");
        let telemetry_dir_str = telemetry_dir.to_str().unwrap();
        std::fs::write(
            &config_path,
            format!(
                r#"[executor]
provider = "openai"
model = "test-model"
base_url = "http://127.0.0.1:1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
escalation_slots = 1

[telemetry]
dir = "{}"
"#,
                telemetry_dir_str
            ),
        )
        .unwrap();
        config_path
    }

    fn write_telemetry_fixture(temp_dir: &TempDir) -> PathBuf {
        let telemetry_dir = temp_dir.path().join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();
        let path = telemetry_dir.join("phase_runs.jsonl");
        let lines = [
            r#"{"ts":1717000000000,"model":"m1","generation_params":{"temperature":null,"seed":null},"phase_id":"p1","tags":["rust","feature"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.1,"repairs_per_call":0.5,"verifier_retries":2,"tool_success_rate":0.9,"turns":7,"wall_clock_s":12.5,"tokens":{"prompt":0,"completion":0,"total":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}"#,
            r#"{"ts":1717000001000,"model":"m2","generation_params":{"temperature":null,"seed":null},"phase_id":"p2","tags":["rust","bugfix"],"status":"complete","escalated":true,"gates":{"fmt":true,"build":true,"lint":false,"test":true},"parse_failure_rate":0.2,"repairs_per_call":1.0,"verifier_retries":3,"tool_success_rate":0.8,"turns":10,"wall_clock_s":20.0,"tokens":{"prompt":0,"completion":0,"total":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":1,"architect_verdict":"rejected"}"#,
        ];
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    #[test]
    fn model_scorecard_success_via_config_telemetry_dir() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = make_config_with_telemetry(&temp_dir);
        write_telemetry_fixture(&temp_dir);

        let params = ModelScorecardParams {
            tags: None,
            model: None,
            min_runs: None,
            telemetry_path: None,
            bench_suite: None,
            production_only: None,
        };
        let result = model_scorecard_inner(&config_path, &params).unwrap();

        assert_eq!(result.total_runs_considered, 2);
        assert!(!result.truncated);
        assert!(!result.rows.is_empty());
    }

    #[test]
    fn model_scorecard_success_via_telemetry_path_override() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = make_config_with_telemetry(&temp_dir);
        let fixture = write_telemetry_fixture(&temp_dir);

        let params = ModelScorecardParams {
            tags: None,
            model: None,
            min_runs: None,
            telemetry_path: Some(fixture.to_str().unwrap().to_string()),
            bench_suite: None,
            production_only: None,
        };
        let result = model_scorecard_inner(&config_path, &params).unwrap();

        assert_eq!(result.total_runs_considered, 2);
        assert!(!result.rows.is_empty());
    }

    #[test]
    fn model_scorecard_telemetry_path_override_takes_precedence() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = make_config_with_telemetry(&temp_dir);
        write_telemetry_fixture(&temp_dir);

        let alt_dir = temp_dir.path().join("alt_telemetry");
        std::fs::create_dir_all(&alt_dir).unwrap();
        let alt_path = alt_dir.join("phase_runs.jsonl");
        let line = r#"{"ts":1717000002000,"model":"m3","generation_params":{"temperature":null,"seed":null},"phase_id":"p3","tags":["go"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{"prompt":0,"completion":0,"total":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}"#;
        std::fs::write(&alt_path, format!("{}\n", line)).unwrap();

        let params = ModelScorecardParams {
            tags: None,
            model: None,
            min_runs: None,
            telemetry_path: Some(alt_path.to_str().unwrap().to_string()),
            bench_suite: None,
            production_only: None,
        };
        let result = model_scorecard_inner(&config_path, &params).unwrap();

        assert_eq!(result.total_runs_considered, 1);
        assert_eq!(result.rows[0].model, "m3");
    }

    #[test]
    fn model_scorecard_telemetry_disabled_returns_error() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("rexymcp.toml");
        std::fs::write(
            &config_path,
            r#"[executor]
provider = "openai"
model = "test-model"
base_url = "http://127.0.0.1:1"

[commands]

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
escalation_slots = 1
"#,
        )
        .unwrap();

        let params = ModelScorecardParams {
            tags: None,
            model: None,
            min_runs: None,
            telemetry_path: None,
            bench_suite: None,
            production_only: None,
        };
        let result = model_scorecard_inner(&config_path, &params);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("telemetry disabled"));
    }

    #[test]
    fn model_scorecard_missing_file_returns_empty() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = make_config_with_telemetry(&temp_dir);

        let params = ModelScorecardParams {
            tags: None,
            model: None,
            min_runs: None,
            telemetry_path: None,
            bench_suite: None,
            production_only: None,
        };
        let result = model_scorecard_inner(&config_path, &params).unwrap();

        assert_eq!(result.total_runs_considered, 0);
        assert!(result.rows.is_empty());
        assert!(!result.truncated);
    }

    #[test]
    fn model_scorecard_malformed_jsonl_survivors_contribute() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = make_config_with_telemetry(&temp_dir);
        let telemetry_dir = temp_dir.path().join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();
        let path = telemetry_dir.join("phase_runs.jsonl");
        let good_line = r#"{"ts":1717000000000,"model":"m1","generation_params":{"temperature":null,"seed":null},"phase_id":"p1","tags":["rust"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.1,"repairs_per_call":0.5,"verifier_retries":2,"tool_success_rate":0.9,"turns":7,"wall_clock_s":12.5,"tokens":{"prompt":0,"completion":0,"total":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}"#;
        std::fs::write(&path, format!("GARBAGE LINE\n{}\n", good_line)).unwrap();

        let params = ModelScorecardParams {
            tags: None,
            model: None,
            min_runs: None,
            telemetry_path: None,
            bench_suite: None,
            production_only: None,
        };
        let result = model_scorecard_inner(&config_path, &params).unwrap();

        assert_eq!(result.total_runs_considered, 1);
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].model, "m1");
    }

    #[test]
    fn model_scorecard_truncated_flag_when_over_max_rows() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = make_config_with_telemetry(&temp_dir);
        let telemetry_dir = temp_dir.path().join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();
        let path = telemetry_dir.join("phase_runs.jsonl");

        let mut lines = Vec::new();
        for i in 0..scorecard::MAX_ROWS + 10 {
            let tag = format!("tag{}", i);
            lines.push(format!(
                r#"{{"ts":1717000000000,"model":"m1","generation_params":{{"temperature":null,"seed":null}},"phase_id":"p{}","tags":["{}"],"status":"complete","escalated":false,"gates":{{"fmt":true,"build":true,"lint":true,"test":true}},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{{"prompt":0,"completion":0,"total":0}},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}}"#,
                i, tag
            ));
        }
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();

        let params = ModelScorecardParams {
            tags: None,
            model: None,
            min_runs: None,
            telemetry_path: None,
            bench_suite: None,
            production_only: None,
        };
        let result = model_scorecard_inner(&config_path, &params).unwrap();

        assert_eq!(result.rows.len(), scorecard::MAX_ROWS);
        assert!(result.truncated);
    }

    #[test]
    fn scorecard_params_bench_suite_takes_precedence() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = make_config_with_telemetry(&temp_dir);
        let telemetry_dir = temp_dir.path().join("telemetry");
        std::fs::create_dir_all(&telemetry_dir).unwrap();
        let path = telemetry_dir.join("phase_runs.jsonl");

        // Three runs: production (None), smoke benchmark, perf benchmark — all
        // with the same tag so they'd land in the same bucket without source
        // filtering.
        let lines = [
            r#"{"ts":1717000000000,"model":"m1","generation_params":{"temperature":null,"seed":null},"phase_id":"p1","tags":["rust"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{"prompt":0,"completion":0,"total":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}"#,
            r#"{"ts":1717000001000,"model":"m1","generation_params":{"temperature":null,"seed":null},"phase_id":"p2","tags":["rust"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.1,"repairs_per_call":0.5,"verifier_retries":2,"tool_success_rate":0.9,"turns":7,"wall_clock_s":12.5,"tokens":{"prompt":0,"completion":0,"total":0},"bench_suite":"smoke","warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}"#,
            r#"{"ts":1717000002000,"model":"m1","generation_params":{"temperature":null,"seed":null},"phase_id":"p3","tags":["rust"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.5,"repairs_per_call":1.0,"verifier_retries":5,"tool_success_rate":0.5,"turns":20,"wall_clock_s":100.0,"tokens":{"prompt":0,"completion":0,"total":0},"bench_suite":"perf","warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}"#,
        ];
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();

        // Both bench_suite and production_only set — bench_suite wins.
        let params = ModelScorecardParams {
            tags: None,
            model: None,
            min_runs: None,
            telemetry_path: None,
            bench_suite: Some("smoke".to_string()),
            production_only: Some(true),
        };
        let result = model_scorecard_inner(&config_path, &params).unwrap();

        // Only the smoke run should appear — production and perf are excluded.
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].tag, "rust");
        assert_eq!(result.rows[0].n_runs, 1);
        // Verify it's the smoke run's data (parse_failure_rate 0.1), not
        // production (0.0) or perf (0.5).
        assert!((result.rows[0].parse_failure_rate_mean - 0.1).abs() < f64::EPSILON);
    }

    // --- Progress forwarding tests ---

    #[test]
    fn progress_notifier_maps_fields_correctly() {
        use rexymcp_executor::agent::progress::ProgressEvent;
        use rmcp::model::NumberOrString;

        let event = ProgressEvent {
            turn: 4,
            stage: "tool:patch".to_string(),
            files_changed: vec![],
            message: "turn=4 stage=tool:patch +12/-3 files=1".to_string(),
        };

        let params = ProgressNotificationParam {
            progress_token: ProgressToken(NumberOrString::Number(42)),
            progress: event.turn as f64,
            total: None,
            message: Some(event.message.clone()),
        };

        assert_eq!(params.progress, 4.0);
        assert!(params.total.is_none());
        assert_eq!(
            params.message.as_deref(),
            Some("turn=4 stage=tool:patch +12/-3 files=1")
        );
    }

    #[test]
    fn progress_notifier_fire_and_forget_does_not_panic() {
        use rexymcp_executor::agent::progress::ProgressEvent;

        let event = ProgressEvent {
            turn: 1,
            stage: "turn_start".to_string(),
            files_changed: vec![],
            message: "turn=1 stage=turn_start +0/-0 files=0".to_string(),
        };

        let callback = |e: &ProgressEvent| {
            let _ = e;
        };
        callback(&event);
    }

    // --- Wrapper-level integration tests (bug-05b-1, issue 2) ---

    /// Capture callback for server-level tests. Implements `ProgressCallback`
    /// so it can be threaded through `execute_phase_inner` → `runner::run_phase`
    /// → `LoopDeps.progress` and inspected after the call.
    struct CaptureCallback {
        events: std::sync::Mutex<Vec<rexymcp_executor::agent::progress::ProgressEvent>>,
    }

    impl CaptureCallback {
        fn new() -> Self {
            Self {
                events: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn events(&self) -> Vec<rexymcp_executor::agent::progress::ProgressEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    impl ProgressCallback for CaptureCallback {
        fn on_progress(&self, event: &rexymcp_executor::agent::progress::ProgressEvent) {
            self.events.lock().unwrap().push(event.clone());
        }
    }

    #[tokio::test]
    async fn execute_phase_inner_forwards_progress_to_loop() {
        use rexymcp_executor::ai::AiEvent;
        use rexymcp_executor::ai::testing::MockAiClientScript;

        let temp_dir = TempDir::new().unwrap();
        let repo_dir = temp_dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();

        let config_path = make_test_config(&temp_dir);
        let phase_path = make_phase_doc(&temp_dir);

        let path = repo_dir.join("f.txt");
        std::fs::write(&path, "hello").unwrap();
        let path_str = path.to_string_lossy().to_string();

        let client = MockAiClientScript::new(vec![
            vec![AiEvent::ToolCallGeneric {
                id: "call1".to_string(),
                name: "read_file".to_string(),
                args: serde_json::json!({ "path": path_str }),
                thought_signature: None,
            }],
            vec![AiEvent::Token("done".to_string())],
        ]);

        let params = ExecutePhaseParams {
            phase_doc_path: phase_path.to_str().unwrap().to_string(),
            repo_path: repo_dir.to_str().unwrap().to_string(),
            model: None,
        };

        let capture = CaptureCallback::new();
        let result =
            execute_phase_inner_with_client(&config_path, &params, Some(&capture), Some(&client))
                .await;

        assert!(
            result.is_ok(),
            "execute_phase_inner should succeed: {:?}",
            result
        );

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
    }

    #[tokio::test]
    async fn execute_phase_inner_with_none_captures_nothing() {
        use rexymcp_executor::ai::AiEvent;
        use rexymcp_executor::ai::testing::MockAiClientScript;

        let temp_dir = TempDir::new().unwrap();
        let repo_dir = temp_dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();

        let config_path = make_test_config(&temp_dir);
        let phase_path = make_phase_doc(&temp_dir);

        let client = MockAiClientScript::new(vec![vec![AiEvent::Token("done".to_string())]]);

        let params = ExecutePhaseParams {
            phase_doc_path: phase_path.to_str().unwrap().to_string(),
            repo_path: repo_dir.to_str().unwrap().to_string(),
            model: None,
        };

        let result =
            execute_phase_inner_with_client(&config_path, &params, None, Some(&client)).await;

        assert!(
            result.is_ok(),
            "execute_phase_inner should succeed: {:?}",
            result
        );
    }
}
