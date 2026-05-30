use std::path::{Path, PathBuf};

use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::cap;
use crate::log_query;
use crate::runner;

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

/// Inner logic for `execute_phase` — extracted so it can be tested without
/// the rmcp macro wrapper.
pub(crate) async fn execute_phase_inner(
    config_path: &Path,
    params: &ExecutePhaseParams,
) -> Result<ExecutePhaseOutput, String> {
    let cfg = rexymcp_executor::config::Config::load_with_env(config_path)
        .map_err(|e| format!("failed to load config: {}", e))?;

    let phase_doc_path = PathBuf::from(&params.phase_doc_path);
    let repo_path = PathBuf::from(&params.repo_path);

    let standards_path = repo_path.join("docs/dev/STANDARDS.md");
    let standards = std::fs::read_to_string(&standards_path).unwrap_or_default();

    let executor_contract = "";

    let telemetry_dir = cfg.telemetry.dir.as_deref();

    let result = runner::run_phase(
        &cfg,
        &phase_doc_path,
        &repo_path,
        executor_contract,
        &standards,
        params.model.as_deref(),
        telemetry_dir,
    )
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

#[rmcp::tool_router(server_handler)]
impl RexyMcpServer {
    #[rmcp::tool(
        description = "Execute a phase against a target repository. Runs the local LLM through a tool-using loop, verifies edits, runs build/lint/test commands, and returns a structured PhaseResult."
    )]
    async fn execute_phase(
        &self,
        Parameters(params): Parameters<ExecutePhaseParams>,
    ) -> Result<Json<ExecutePhaseOutput>, String> {
        execute_phase_inner(&self.config_path, &params)
            .await
            .map(Json)
    }

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
        let result = execute_phase_inner(&config_path, &params).await;

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
        let result = execute_phase_inner(&config_path, &params).await;

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
}
