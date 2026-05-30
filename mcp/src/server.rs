use std::path::{Path, PathBuf};

use rmcp::handler::server::wrapper::{Json, Parameters};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::cap;
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
}
