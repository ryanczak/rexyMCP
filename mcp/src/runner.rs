use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rexymcp_executor::agent::command::CommandRunner;
use rexymcp_executor::agent::progress::ProgressCallback;
use rexymcp_executor::agent::verify::FileVerifier;
use rexymcp_executor::agent::{self, LoopDeps, PhaseInput};
use rexymcp_executor::ai::{AiClient, OpenAiClient, ToolSchema};
use rexymcp_executor::config::Config;
use rexymcp_executor::context::budget::Budget;
use rexymcp_executor::phase::PhaseResult;
use rexymcp_executor::security::Scope;
use rexymcp_executor::store::sessions::jsonl::generate_session_id;
use rexymcp_executor::store::telemetry::GenerationParams;
use rexymcp_executor::tools;

/// Parsed fields from a phase-doc markdown file.
pub struct PhaseDocFields {
    pub goal: String,
    pub acceptance_criteria: String,
    pub tags: Vec<String>,
}

/// Extract a section body from a markdown string.
/// Matches a heading line that starts with `## ` followed by `heading_name`,
/// then collects everything up to the next `## ` / `# ` line or EOF.
fn extract_section(markdown: &str, heading: &str) -> String {
    let prefix = format!("## {}", heading);
    let lines = markdown.lines();
    let mut found = false;
    let mut body = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        if !found {
            if trimmed.starts_with(&prefix) {
                found = true;
            }
        } else if trimmed.starts_with('#') {
            break;
        } else {
            body.push(line);
        }
    }

    body.join("\n").trim().to_string()
}

/// Parse `**Tags:**` line from the frontmatter into individual tags.
fn parse_tags(markdown: &str) -> Vec<String> {
    let tags_line = markdown.lines().find(|l| l.contains("**Tags:**"));
    match tags_line {
        Some(line) => {
            let after = line.split("**Tags:**").nth(1).unwrap_or("");
            after
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        None => Vec::new(),
    }
}

/// Parse a phase-doc markdown string into structured fields.
pub fn parse_phase_doc(markdown: &str) -> PhaseDocFields {
    PhaseDocFields {
        goal: extract_section(markdown, "Goal"),
        acceptance_criteria: extract_section(markdown, "Acceptance criteria"),
        tags: parse_tags(markdown),
    }
}

/// Derive a short phase id from a file path stem.
/// `phase-01-phase-runner.md` → `"phase-01"`; non-matching → whole stem.
pub fn derive_phase_id(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

    if let Some(after) = stem.strip_prefix("phase-") {
        let digits_end = after
            .char_indices()
            .find(|(_, c)| !c.is_ascii_digit())
            .map(|(i, _)| i)
            .unwrap_or(after.len());
        if digits_end > 0 {
            return stem[..6 + digits_end].to_string();
        }
    }

    stem.to_string()
}

/// Injected seams for the testable inner assembler.
struct Seams<'a> {
    client: &'a dyn AiClient,
    verifier: &'a dyn FileVerifier,
    runner: &'a dyn CommandRunner,
    clock: &'a (dyn Fn() -> u64 + Send + Sync),
}

/// Non-seam inputs for the assembler.
struct AssemblyInput<'a> {
    cfg: &'a Config,
    phase_doc_path: &'a Path,
    repo_path: &'a Path,
    standards: &'a str,
    model: &'a str,
    telemetry_dir: Option<&'a Path>,
    progress: Option<&'a dyn ProgressCallback>,
}

/// Register the full built-in tool set and derive schemas.
pub fn build_registry(
    scope: &Scope,
    bash_timeout_secs: u32,
) -> (rexymcp_executor::tools::ToolRegistry, Vec<ToolSchema>) {
    let mut registry = rexymcp_executor::tools::ToolRegistry::new();

    let tools: Vec<Arc<dyn tools::Tool>> = vec![
        tools::read_file(scope.clone()),
        tools::write_file(scope.clone()),
        tools::patch(scope.clone()),
        tools::find_files(scope.clone()),
        tools::search(scope.clone()),
        tools::symbols(scope.clone()),
        tools::bash(scope.clone(), bash_timeout_secs),
    ];

    for tool in &tools {
        registry.register(tool.clone());
    }

    let schemas: Vec<ToolSchema> = tools
        .iter()
        .map(|tool| ToolSchema {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            parameters: tool.schema(),
        })
        .collect();

    (registry, schemas)
}

/// Inner assembler — takes injected seams, fully hermetic-testable.
async fn run_phase_with(
    inp: &AssemblyInput<'_>,
    seams: &Seams<'_>,
) -> rexymcp_executor::error::Result<PhaseResult> {
    let phase_doc = std::fs::read_to_string(inp.phase_doc_path)?;
    let fields = parse_phase_doc(&phase_doc);

    let phase = derive_phase_id(inp.phase_doc_path);

    let scope = Scope::new(inp.repo_path)
        .map_err(|e| rexymcp_executor::error::Error::Internal(format!("scope error: {}", e)))?;

    let (registry, tool_schemas) = build_registry(&scope, 30);

    let budget = Budget::from_context(
        inp.cfg.budget.context_length,
        inp.cfg.budget.max_context_pct,
    );

    let input = PhaseInput {
        standards: inp.standards.to_string(),
        phase_doc,
        goal: fields.goal,
        acceptance_criteria: fields.acceptance_criteria,
        phase,
        tags: fields.tags,
    };

    let session_id = generate_session_id();

    let deps = LoopDeps {
        client: seams.client,
        registry: &registry,
        tools: &tool_schemas,
        budget: &budget,
        max_turns: inp.cfg.budget.max_turns as usize,
        project_root: inp.repo_path,
        model: inp.model,
        session_id: &session_id,
        clock: seams.clock,
        verifier: seams.verifier,
        commands: &inp.cfg.commands,
        runner: seams.runner,
        generation_params: GenerationParams {
            temperature: inp.cfg.executor.temperature,
            seed: inp.cfg.executor.seed,
        },
        telemetry_dir: inp.telemetry_dir,
        progress: inp.progress,
    };

    agent::execute_phase(&input, deps).await
}

/// Configuration parameters for `run_phase`, grouped to stay under
/// clippy's argument limit (same pattern as `AssemblyInput` / `Seams`).
pub struct RunPhaseConfig<'a> {
    pub cfg: &'a Config,
    pub phase_doc_path: &'a Path,
    pub repo_path: &'a Path,
    pub standards: &'a str,
    pub model_override: Option<&'a str>,
    pub telemetry_dir: Option<&'a Path>,
    pub progress: Option<&'a dyn ProgressCallback>,
    /// Inject a test client. `None` → production `OpenAiClient`.
    pub test_client: Option<&'a dyn AiClient>,
}

/// Production wrapper — builds real seams + system clock, delegates.
pub async fn run_phase(inp: &RunPhaseConfig<'_>) -> rexymcp_executor::error::Result<PhaseResult> {
    let model = inp.model_override.unwrap_or(&inp.cfg.executor.model);

    let prod_client = OpenAiClient::new(
        inp.cfg.executor.api_key.clone().unwrap_or_default(),
        model.to_string(),
        inp.cfg.executor.base_url.clone(),
        std::time::Duration::from_secs(inp.cfg.executor.first_token_timeout_secs),
        std::time::Duration::from_secs(inp.cfg.executor.stream_idle_timeout_secs),
        inp.cfg.executor.temperature,
        inp.cfg.executor.seed,
    );

    let client: &dyn AiClient = match inp.test_client {
        Some(c) => c,
        None => &prod_client,
    };

    let verifier = rexymcp_executor::agent::verify::RealVerifier;
    let runner = rexymcp_executor::agent::command::RealCommandRunner;

    let clock = || {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    };

    let seams = Seams {
        client,
        verifier: &verifier,
        runner: &runner,
        clock: &clock,
    };

    let assembly = AssemblyInput {
        cfg: inp.cfg,
        phase_doc_path: inp.phase_doc_path,
        repo_path: inp.repo_path,
        standards: inp.standards,
        model,
        telemetry_dir: inp.telemetry_dir,
        progress: inp.progress,
    };

    run_phase_with(&assembly, &seams).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rexymcp_executor::agent::command::{CommandResult, CommandRunner};
    use rexymcp_executor::agent::verify::FileVerifier;
    use rexymcp_executor::ai::testing::MockAiClient;
    use rexymcp_executor::governor::verifier::{
        Baseline as GovBaseline, VerifierResult as GovVerifierResult,
    };
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    // --- Noop verifier/runner for integration test ---

    struct NoopVerifier;

    #[async_trait]
    impl FileVerifier for NoopVerifier {
        async fn verify(&self, _path: &Path) -> GovVerifierResult {
            GovVerifierResult::Checked {
                diagnostics: vec![],
            }
        }
        async fn capture_baseline(&self, _paths: &[PathBuf]) -> GovBaseline {
            GovBaseline::default()
        }
    }

    struct NoopRunner;

    #[async_trait]
    impl CommandRunner for NoopRunner {
        async fn run(&self, _command: &str, _cwd: &Path) -> CommandResult {
            CommandResult {
                output: String::new(),
                success: true,
            }
        }
    }

    // --- parse_phase_doc tests ---

    #[test]
    fn parse_positive_fixture() {
        let md = "# Phase 01: Example\n\n**Tags:** language=rust, kind=feature, size=m\n\n## Goal\n\nDo the thing.\n\n## Acceptance criteria\n\n- [ ] It works.\n\n## Out of scope\n\nNothing.\n";

        let fields = parse_phase_doc(md);
        assert_eq!(fields.goal, "Do the thing.");
        assert_eq!(fields.acceptance_criteria, "- [ ] It works.");
        assert_eq!(fields.tags, vec!["language=rust", "kind=feature", "size=m"]);
    }

    #[test]
    fn parse_missing_goal_yields_empty() {
        let md = "# Phase 01\n\n## Acceptance criteria\n\n- [ ] It works.\n";
        let fields = parse_phase_doc(md);
        assert_eq!(fields.goal, "");
        assert_eq!(fields.acceptance_criteria, "- [ ] It works.");
    }

    #[test]
    fn parse_missing_tags_yields_empty_vec() {
        let md = "# Phase 01\n\n## Goal\n\nDo it.\n";
        let fields = parse_phase_doc(md);
        assert_eq!(fields.tags, Vec::<String>::new());
    }

    #[test]
    fn parse_spaced_tags_line_splits_cleanly() {
        let md = "**Tags:**   language=rust  ,   kind=feature  ,  size=m  \n\n## Goal\n\nDo it.\n";
        let fields = parse_phase_doc(md);
        assert_eq!(fields.tags, vec!["language=rust", "kind=feature", "size=m"]);
    }

    #[test]
    fn parse_goal_followed_immediately_by_next_heading() {
        let md = "## Goal\n\n## Acceptance criteria\n\n- [ ] It works.\n";
        let fields = parse_phase_doc(md);
        assert_eq!(fields.goal, "");
    }

    // --- derive_phase_id tests ---

    #[test]
    fn derive_phase_id_standard() {
        assert_eq!(
            derive_phase_id(Path::new("phase-01-phase-runner.md")),
            "phase-01"
        );
    }

    #[test]
    fn derive_phase_id_non_matching() {
        assert_eq!(derive_phase_id(Path::new("weird-name.md")), "weird-name");
    }

    // --- build_registry tests ---

    #[test]
    fn build_registry_has_seven_tools() {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let (_registry, schemas) = build_registry(&scope, 30);

        assert_eq!(schemas.len(), 7);
        let names: Vec<_> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "read_file",
                "write_file",
                "patch",
                "find_files",
                "search",
                "symbols",
                "bash"
            ]
        );

        let rf = schemas.iter().find(|s| s.name == "read_file").unwrap();
        assert!(!rf.parameters.is_null());
    }

    // --- run_phase_with integration test ---

    #[tokio::test]
    async fn run_phase_with_assembles_and_returns_result() {
        let dir = TempDir::new().unwrap();
        let repo_dir = dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();

        let phase_doc_path = dir.path().join("phase-01-test.md");
        std::fs::write(
            &phase_doc_path,
            "# Phase 01: Test\n\n**Tags:** language=rust, kind=test, size=s\n\n## Goal\n\nTest goal.\n\n## Acceptance criteria\n\n- [ ] It runs.\n",
        )
        .unwrap();

        let cfg = Config::default();

        let mock = MockAiClient::new(vec!["Done.".to_string()]);

        let clock = || 1234567890u64;

        let seams = Seams {
            client: &mock,
            verifier: &NoopVerifier,
            runner: &NoopRunner,
            clock: &clock,
        };

        let inp = AssemblyInput {
            cfg: &cfg,
            phase_doc_path: &phase_doc_path,
            repo_path: &repo_dir,
            standards: "standards",
            model: "test-model",
            telemetry_dir: None,
            progress: None,
        };

        let result = run_phase_with(&inp, &seams).await;

        assert!(
            result.is_ok(),
            "run_phase_with should succeed: {:?}",
            result
        );
        let phase_result = result.unwrap();
        assert_eq!(
            phase_result.status,
            rexymcp_executor::phase::PhaseStatus::Complete
        );
    }

    // --- negative: non-existent root ---

    #[tokio::test]
    async fn run_phase_with_fails_on_nonexistent_root() {
        let dir = TempDir::new().unwrap();
        let phase_doc_path = dir.path().join("phase-01-test.md");
        std::fs::write(&phase_doc_path, "# Phase 01\n\n## Goal\n\nTest.\n").unwrap();

        let cfg = Config::default();
        let mock = MockAiClient::new(vec![]);
        let clock = || 0u64;

        let seams = Seams {
            client: &mock,
            verifier: &NoopVerifier,
            runner: &NoopRunner,
            clock: &clock,
        };

        let nonexistent = dir.path().join("does_not_exist_repo");
        let inp = AssemblyInput {
            cfg: &cfg,
            phase_doc_path: &phase_doc_path,
            repo_path: &nonexistent,
            standards: "",
            model: "model",
            telemetry_dir: None,
            progress: None,
        };
        let result = run_phase_with(&inp, &seams).await;

        assert!(result.is_err(), "should error on non-existent root");
    }
}
