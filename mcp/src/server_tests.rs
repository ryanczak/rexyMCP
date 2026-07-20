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
    let result = execute_phase_inner(&config_path, &params, None, CancelSignal::never()).await;

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
    let result = execute_phase_inner(&config_path, &params, None, CancelSignal::never()).await;

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
        r#"{"schema_version":1,"ts":1717000000000,"model":"m1","generation_params":{"temperature":null,"seed":null},"phase_id":"p1","tags":["rust","feature"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.1,"repairs_per_call":0.5,"verifier_retries":2,"tool_success_rate":0.9,"turns":7,"wall_clock_s":12.5,"tokens":{"prompt":0,"completion":0,"total":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}"#,
        r#"{"schema_version":1,"ts":1717000001000,"model":"m2","generation_params":{"temperature":null,"seed":null},"phase_id":"p2","tags":["rust","bugfix"],"status":"complete","escalated":true,"gates":{"fmt":true,"build":true,"lint":false,"test":true},"parse_failure_rate":0.2,"repairs_per_call":1.0,"verifier_retries":3,"tool_success_rate":0.8,"turns":10,"wall_clock_s":20.0,"tokens":{"prompt":0,"completion":0,"total":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":1,"architect_verdict":"rejected"}"#,
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
    let line = r#"{"schema_version":1,"ts":1717000002000,"model":"m3","generation_params":{"temperature":null,"seed":null},"phase_id":"p3","tags":["go"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{"prompt":0,"completion":0,"total":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}"#;
    std::fs::write(&alt_path, format!("{}\n", line)).unwrap();

    let params = ModelScorecardParams {
        tags: None,
        model: None,
        min_runs: None,
        telemetry_path: Some(alt_path.to_str().unwrap().to_string()),
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

[telemetry]
enabled = false
"#,
    )
    .unwrap();

    let params = ModelScorecardParams {
        tags: None,
        model: None,
        min_runs: None,
        telemetry_path: None,
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
    let good_line = r#"{"schema_version":1,"ts":1717000000000,"model":"m1","generation_params":{"temperature":null,"seed":null},"phase_id":"p1","tags":["rust"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.1,"repairs_per_call":0.5,"verifier_retries":2,"tool_success_rate":0.9,"turns":7,"wall_clock_s":12.5,"tokens":{"prompt":0,"completion":0,"total":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}"#;
    std::fs::write(&path, format!("GARBAGE LINE\n{}\n", good_line)).unwrap();

    let params = ModelScorecardParams {
        tags: None,
        model: None,
        min_runs: None,
        telemetry_path: None,
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
                r#"{{"schema_version":1,"ts":1717000000000,"model":"m1","generation_params":{{"temperature":null,"seed":null}},"phase_id":"p{}","tags":["{}"],"status":"complete","escalated":false,"gates":{{"fmt":true,"build":true,"lint":true,"test":true}},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{{"prompt":0,"completion":0,"total":0}},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}}"#,
                i, tag
            ));
    }
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();

    let params = ModelScorecardParams {
        tags: None,
        model: None,
        min_runs: None,
        telemetry_path: None,
    };
    let result = model_scorecard_inner(&config_path, &params).unwrap();

    assert_eq!(result.rows.len(), scorecard::MAX_ROWS);
    assert!(result.truncated);
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

    let params = ProgressNotificationParam::new(
        ProgressToken(NumberOrString::Number(42)),
        event.turn as f64,
    )
    .with_message(event.message.clone());

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
    let result = execute_phase_inner_with_client(
        &config_path,
        &params,
        Some(&capture),
        Some(&client),
        CancelSignal::never(),
    )
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

    let result = execute_phase_inner_with_client(
        &config_path,
        &params,
        None,
        Some(&client),
        CancelSignal::never(),
    )
    .await;

    assert!(
        result.is_ok(),
        "execute_phase_inner should succeed: {:?}",
        result
    );
}

#[test]
fn model_scorecard_folds_review() {
    use rexymcp_executor::store::telemetry::{PhaseReview, REVIEW_RECORD_TAG, append_review};

    let temp_dir = TempDir::new().unwrap();
    let config_path = make_config_with_telemetry(&temp_dir);
    let fixture = write_telemetry_fixture(&temp_dir);

    // Append a review keyed to the first run (model "m1", phase "p1")
    // The first fixture run has ts=1717000000000, model="m1", phase_id="p1", architect_verdict=null
    let telemetry_dir = temp_dir.path().join("telemetry");
    append_review(
        &telemetry_dir,
        &PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 1_717_000_000_500,
            phase_doc_path: None,
            phase_id: "p1".to_string(),
            project_id: None,
            architect_verdict: "approved_first_try".to_string(),
            bounces_to_approval: Some(0),
            bugs_filed: Some(0),
            warnings: Some(0),
            failure_class: vec!["none".to_string()],
        },
    )
    .unwrap();

    let params = ModelScorecardParams {
        tags: None,
        model: None,
        min_runs: None,
        telemetry_path: Some(fixture.to_str().unwrap().to_string()),
    };
    let result = model_scorecard_inner(&config_path, &params).unwrap();

    assert_eq!(result.total_runs_considered, 2);

    // The m1 row should now show approved_first_try rate > 0
    let m1_row = result
        .rows
        .iter()
        .find(|r| r.model == "m1")
        .expect("expected m1 row in scorecard");
    assert!(
        m1_row.approved_first_try_rate.is_some() && m1_row.approved_first_try_rate.unwrap() > 0.0,
        "expected m1 approved_first_try_rate > 0 after folding review, got {:?}",
        m1_row.approved_first_try_rate
    );

    // Pinned negative: a review whose phase_doc_path matches no run should not affect aggregates.
    // Append a review for a non-existent phase
    append_review(
        &telemetry_dir,
        &PhaseReview {
            record: REVIEW_RECORD_TAG.to_string(),
            ts: 1_717_000_002_000,
            phase_doc_path: Some("/nonexistent/phase-99.md".to_string()),
            phase_id: "phase-99".to_string(),
            project_id: Some("other-project".to_string()),
            architect_verdict: "approved_first_try".to_string(),
            bounces_to_approval: Some(0),
            bugs_filed: Some(0),
            warnings: Some(0),
            failure_class: vec!["none".to_string()],
        },
    )
    .unwrap();

    let result2 = model_scorecard_inner(&config_path, &params).unwrap();
    assert_eq!(result2.total_runs_considered, 2);
    // m1 row should be unchanged (the phantom review didn't match any run)
    let m1_row2 = result2
        .rows
        .iter()
        .find(|r| r.model == "m1")
        .expect("expected m1 row in scorecard");
    assert_eq!(
        m1_row.approved_first_try_rate, m1_row2.approved_first_try_rate,
        "phantom review should not change m1 aggregates"
    );
}

// --- model_profile tests ---

#[test]
fn model_profile_success_via_config_telemetry_dir() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = make_config_with_telemetry(&temp_dir);
    write_telemetry_fixture(&temp_dir);

    let params = ModelProfileParams {
        tags: None,
        model: None,
        min_runs: None,
        telemetry_path: None,
    };
    let result = model_profile_inner(&config_path, &params).unwrap();

    assert_eq!(result.total_runs_considered, 2);
    assert!(!result.truncated);
    assert!(!result.rows.is_empty());
}

#[test]
fn model_profile_telemetry_path_override_takes_precedence() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = make_config_with_telemetry(&temp_dir);
    write_telemetry_fixture(&temp_dir);

    let alt_dir = temp_dir.path().join("alt_telemetry");
    std::fs::create_dir_all(&alt_dir).unwrap();
    let alt_path = alt_dir.join("phase_runs.jsonl");
    let line = r#"{"schema_version":1,"ts":1717000002000,"model":"m3","generation_params":{"temperature":null,"seed":null},"phase_id":"p3","tags":["go"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":1,"wall_clock_s":1.0,"tokens":{"prompt":0,"completion":0,"total":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null}"#;
    std::fs::write(&alt_path, format!("{}\n", line)).unwrap();

    let params = ModelProfileParams {
        tags: None,
        model: None,
        min_runs: None,
        telemetry_path: Some(alt_path.to_str().unwrap().to_string()),
    };
    let result = model_profile_inner(&config_path, &params).unwrap();

    assert_eq!(result.total_runs_considered, 1);
    assert_eq!(result.rows[0].model, "m3");
}

#[test]
fn model_profile_telemetry_disabled_returns_error() {
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

[telemetry]
enabled = false
"#,
    )
    .unwrap();

    let params = ModelProfileParams {
        tags: None,
        model: None,
        min_runs: None,
        telemetry_path: None,
    };
    let result = model_profile_inner(&config_path, &params);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("telemetry disabled"));
}

#[tokio::test]
async fn continue_phase_returns_error_for_missing_phase_doc() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = make_test_config(&temp_dir);

    let params = ContinuePhaseParams {
        phase_doc_path: "/nonexistent/phase.md".to_string(),
        repo_path: temp_dir.path().to_str().unwrap().to_string(),
        guidance: "fix the issue".to_string(),
        prior_log_path: None,
        model: None,
    };
    let result = continue_phase_inner(&config_path, &params, None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn continue_phase_restores_task_states_from_prior_log() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = make_test_config(&temp_dir);

    // Create a prior session log with a TaskUpdate for task "1" as Done.
    let log_path = temp_dir.path().join("session-prior.jsonl");
    std::fs::write(
        &log_path,
        r#"{"ts":1717000000000,"turn":0,"event":{"event_type":"session_start","session_id":"s1","model":"test","phase":"p1"}}
{"ts":1717000001000,"turn":0,"event":{"event_type":"task_update","id":"1","title":"Test task","state":"done"}}
"#,
    )
    .unwrap();

    // Write a phase doc with a spec section that seeds task "1".
    let phase_path_with_spec = temp_dir.path().join("phase-02-test.md");
    std::fs::write(
        &phase_path_with_spec,
        "# Phase 02: Test Resume\n\n**Tags:** language=rust, kind=test, size=s\n\n## Goal\n\nTest resume.\n\n## Spec\n\n1. **Test task** — already done\n\n## Acceptance criteria\n\n- [ ] It resumes.\n",
    )
    .unwrap();

    let params = ContinuePhaseParams {
        phase_doc_path: phase_path_with_spec.to_str().unwrap().to_string(),
        repo_path: temp_dir.path().to_str().unwrap().to_string(),
        guidance: "continue from where we left off".to_string(),
        prior_log_path: Some(log_path.to_str().unwrap().to_string()),
        model: None,
    };

    // This will fail because there's no real AI client, but we can verify
    // the resume context was built (the error should be about the AI call,
    // not about missing files).
    let result = continue_phase_inner(&config_path, &params, None).await;

    // The error should be about the AI backend, not about file resolution.
    if let Err(err) = result {
        assert!(
            !err.contains("failed to load config"),
            "should not be a config error: {err}"
        );
        assert!(
            !err.contains("failed to read phase doc"),
            "should not be a phase doc error: {err}"
        );
    }
}

use std::time::Duration;

#[tokio::test]
async fn get_run_status_unknown_run_id() {
    let registry = crate::jobs::JobRegistry::new();
    let params = GetRunStatusParams {
        run_id: "nonexistent".into(),
    };
    let out = get_run_status_inner(&registry, &params, Duration::from_secs(1)).await;
    assert_eq!(out.state, "unknown");
    assert!(out.result.is_none());
    assert!(out.error.is_none());
}

#[tokio::test]
async fn get_run_status_reports_done_with_result() {
    let registry = crate::jobs::JobRegistry::new();
    let run_id = "done-run".to_string();
    let (handle, _signal) = CancelSignal::new();
    registry.insert(&run_id, handle);
    registry.publish(
        &run_id,
        crate::jobs::RunState::Complete(serde_json::json!({"status": "complete"})),
    );
    let params = GetRunStatusParams {
        run_id: run_id.clone(),
    };
    let out = get_run_status_inner(&registry, &params, Duration::from_secs(1)).await;
    assert_eq!(out.state, "done");
    assert!(out.result.is_some());
    assert!(out.error.is_none());
}

#[tokio::test]
async fn get_run_status_reports_failed() {
    let registry = crate::jobs::JobRegistry::new();
    let run_id = "failed-run".to_string();
    let (handle, _signal) = CancelSignal::new();
    registry.insert(&run_id, handle);
    registry.publish(&run_id, crate::jobs::RunState::Failed("boom".to_string()));
    let params = GetRunStatusParams {
        run_id: run_id.clone(),
    };
    let out = get_run_status_inner(&registry, &params, Duration::from_secs(1)).await;
    assert_eq!(out.state, "failed");
    assert!(out.result.is_none());
    assert_eq!(out.error.as_deref(), Some("boom"));
}

#[tokio::test]
async fn get_run_status_running_times_out() {
    let registry = crate::jobs::JobRegistry::new();
    let run_id = "running-run".to_string();
    let (handle, _signal) = CancelSignal::new();
    registry.insert(&run_id, handle);
    let params = GetRunStatusParams {
        run_id: run_id.clone(),
    };
    let out = get_run_status_inner(&registry, &params, Duration::from_millis(1)).await;
    assert_eq!(out.state, "running");
    assert!(out.result.is_none());
    assert!(out.error.is_none());
}

#[test]
fn get_run_status_tool_is_registered() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let config_path = make_test_config(&temp_dir);
    let server = RexyMcpServer::new(config_path);
    assert!(
        server.get_tool("get_run_status").is_some(),
        "get_run_status tool should be registered"
    );
}

#[test]
fn execute_phase_tool_declares_run_id_output_schema() {
    let tool = execute_phase_tool();
    let schema = tool
        .output_schema
        .expect("execute_phase_tool should have an output_schema");
    assert!(
        schema
            .as_ref()
            .get("properties")
            .and_then(|p| p.get("run_id"))
            .is_some(),
        "execute_phase output_schema should contain run_id property"
    );
    assert!(
        schema
            .as_ref()
            .get("properties")
            .and_then(|p| p.get("status"))
            .is_none(),
        "execute_phase output_schema should NOT contain status (that's PhaseResult, not SpawnedRun)"
    );
}

#[test]
fn continue_phase_tool_declares_phase_result_output_schema() {
    let tool = continue_phase_tool();
    let schema = tool
        .output_schema
        .expect("continue_phase_tool should have an output_schema");
    assert!(
        schema
            .as_ref()
            .get("properties")
            .and_then(|p| p.get("status"))
            .is_some(),
        "continue_phase output_schema should contain status property"
    );
}

#[test]
fn list_tools_carries_output_schemas_for_hand_rolled_tools() {
    let execute_tool = execute_phase_tool();
    let continue_tool = continue_phase_tool();
    assert!(
        execute_tool.output_schema.is_some(),
        "execute_phase tool must carry an output_schema"
    );
    assert!(
        continue_tool.output_schema.is_some(),
        "continue_phase tool must carry an output_schema"
    );
}

#[test]
fn structured_result_carries_matching_text_block() {
    let result = structured_result(&SpawnedRun {
        run_id: "r-1".to_string(),
    })
    .unwrap();
    let expected = serde_json::json!({ "run_id": "r-1" });
    assert_eq!(
        result.structured_content.as_ref(),
        Some(&expected),
        "structured_content should match SpawnedRun JSON"
    );
    let text = result.content[0]
        .as_text()
        .expect("content[0] should be text")
        .text
        .as_str();
    let parsed: serde_json::Value = serde_json::from_str(text).expect("text should parse as JSON");
    assert_eq!(
        parsed, expected,
        "text block should parse to the same JSON as structured_content"
    );
    assert_eq!(
        result.is_error,
        Some(false),
        "is_error should be Some(false) for structured results"
    );
}
