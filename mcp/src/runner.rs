use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rexymcp_executor::agent::command::CommandRunner;
use rexymcp_executor::agent::progress::ProgressCallback;
use rexymcp_executor::agent::verify::FileVerifier;
use rexymcp_executor::agent::{self, LoopDeps, PhaseInput};
use rexymcp_executor::ai::{AiClient, OpenAiClient, SamplingParams, ToolSchema};
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
    context_window: Option<usize>,
    project_id: Option<String>,
    resume: Option<&'a crate::resume::ResumeContext>,
}

/// Resolve the telemetry directory for a CLI-driven `run-phase` invocation:
/// `--no-telemetry` forces telemetry off regardless of config; otherwise
/// defer to `cfg.telemetry.dir`, matching the MCP `execute_phase` path
/// (`server.rs::execute_phase_inner_with_client`), which always telemeters
/// when `[telemetry] dir` is set.
pub fn resolve_telemetry_dir(cfg: &Config, no_telemetry: bool) -> Option<&Path> {
    if no_telemetry {
        None
    } else {
        cfg.telemetry.dir.as_deref()
    }
}

/// Collect non-fatal warnings about a phase run's inputs, for surfacing in
/// `PhaseResult.warnings`. A blank (whitespace-only or absent) STANDARDS
/// string or an unparsed Goal / Acceptance-criteria section each means the
/// executor is running degraded, and today that is silent.
pub fn collect_input_warnings(
    standards: &str,
    goal: &str,
    acceptance_criteria: &str,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if standards.trim().is_empty() {
        warnings.push(
            "STANDARDS.md is empty or missing at <repo>/docs/dev/STANDARDS.md — \
             the executor ran without a Definition of Done. Confirm the file \
             exists and is readable."
                .to_string(),
        );
    }
    if goal.trim().is_empty() {
        warnings.push(
            "Phase doc has no parseable '## Goal' section — the executor ran \
             without a stated goal. Confirm the heading is exactly '## Goal'."
                .to_string(),
        );
    }
    if acceptance_criteria.trim().is_empty() {
        warnings.push(
            "Phase doc has no parseable '## Acceptance criteria' section. \
             Confirm the heading is exactly '## Acceptance criteria'."
                .to_string(),
        );
    }
    warnings
}

/// Derive the milestone directory slug from a phase-doc path.
/// `…/milestones/M17-dashboard-polish-3/phase-09.md` → `Some("M17-dashboard-polish-3")`.
/// Returns `None` when the immediate parent does not look like `M<n>-…`.
fn milestone_id_from_path(path: &Path) -> Option<String> {
    let dir_name = path.parent()?.file_name()?.to_str()?;
    let rest = dir_name.strip_prefix('M')?;
    let has_num = rest
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false);
    if has_num {
        Some(dir_name.to_string())
    } else {
        None
    }
}

/// Register the full built-in tool set and derive schemas.
pub fn build_registry(
    scope: &Scope,
    bash_timeout_secs: u32,
    filter_output: bool,
    tasks: Option<Vec<rexymcp_executor::agent::tasks::Task>>,
) -> (rexymcp_executor::tools::ToolRegistry, Vec<ToolSchema>) {
    let mut registry = rexymcp_executor::tools::ToolRegistry::new();

    let mut tools: Vec<Arc<dyn tools::Tool>> = vec![
        tools::read_file(scope.clone()),
        tools::write_file(scope.clone()),
        tools::patch(scope.clone()),
        tools::find_files(scope.clone()),
        tools::search(scope.clone()),
        tools::symbols(scope.clone()),
        tools::bash_with_filter(scope.clone(), bash_timeout_secs, filter_output),
        tools::delete_file(scope.clone()),
        tools::move_file(scope.clone()),
        tools::patch_lines(scope.clone()),
    ];

    if let Some(t) = tasks {
        tools.push(tools::update_task(t));
    }

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

    let mut cfg = inp.cfg.clone();
    cfg.resolve_for_model(inp.model);

    let tasks = if cfg.executor.task_tracking {
        Some(rexymcp_executor::agent::tasks::seed_from_spec(&phase_doc))
    } else {
        None
    };

    let (registry, tool_schemas) = build_registry(&scope, 30, inp.cfg.context.output_filter, tasks);

    let budget = Budget::from_context(
        inp.cfg.budget.context_length,
        inp.cfg.budget.max_context_pct,
    );

    let input_warnings =
        collect_input_warnings(inp.standards, &fields.goal, &fields.acceptance_criteria);

    // Apply resume context: append preamble to phase_doc and set restored task states.
    let (phase_doc, resumed_task_states) = if let Some(resume) = inp.resume {
        (
            format!("{}\n\n{}", phase_doc, resume.preamble),
            Some(resume.task_states.clone()),
        )
    } else {
        (phase_doc, None)
    };

    let input = PhaseInput {
        standards: inp.standards.to_string(),
        phase_doc,
        goal: fields.goal,
        acceptance_criteria: fields.acceptance_criteria,
        phase,
        tags: fields.tags,
        phase_doc_path: inp.phase_doc_path.to_string_lossy().into_owned(),
        project_id: inp.project_id.clone(),
        milestone_id: milestone_id_from_path(inp.phase_doc_path),
        tier: cfg.executor.tier,
        resumed_task_states,
    };

    let session_id = generate_session_id();

    let deps = LoopDeps {
        client: seams.client,
        registry: &registry,
        tools: &tool_schemas,
        budget: &budget,
        max_turns: inp.cfg.budget.max_turns as usize,
        gate_retries: inp.cfg.budget.effective_gate_retries(inp.cfg.executor.tier),
        wall_clock_secs: inp.cfg.budget.wall_clock_secs,
        project_root: inp.repo_path,
        model: inp.model,
        session_id: &session_id,
        clock: seams.clock,
        verifier: seams.verifier,
        commands: &inp.cfg.commands,
        runner: seams.runner,
        generation_params: GenerationParams {
            temperature: cfg.executor.temperature,
            seed: cfg.executor.seed,
        },
        telemetry_dir: inp.telemetry_dir,
        progress: inp.progress,
        context_window: inp.context_window,
        governor: cfg.governor,
        task_tracking: cfg.executor.task_tracking,
    };

    let mut result = agent::execute_phase(&input, deps).await?;
    result.warnings.extend(input_warnings);

    let finalize_input = crate::finalize::FinalizeInput {
        phase_doc_path: inp.phase_doc_path,
        repo_root: inp.repo_path,
        result: &result,
        now_ms: (seams.clock)(),
        runner: seams.runner,
    };
    if let Err(e) = crate::finalize::finalize_complete(&finalize_input).await {
        result.warnings.push(format!("server finalize failed: {e}"));
    }
    Ok(result)
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
    /// UUID from the target project's `[project] id` in `rexymcp.toml`.
    pub project_id: Option<String>,
    /// Inject a test client. `None` → production `OpenAiClient`.
    pub test_client: Option<&'a dyn AiClient>,
    /// Resume context for `continue_phase`. `None` on a normal `execute_phase`.
    pub resume: Option<crate::resume::ResumeContext>,
}

/// Production wrapper — builds real seams + system clock, delegates.
pub async fn run_phase(inp: &RunPhaseConfig<'_>) -> rexymcp_executor::error::Result<PhaseResult> {
    let model = inp
        .model_override
        .map(str::to_string)
        .unwrap_or_else(|| inp.cfg.executor.model.clone());

    // Per-model overrides for the wire client's sampling. The loop deps resolve
    // independently in `run_phase_with` (see "Why two resolve calls" in the phase
    // doc) — `inp.cfg` is passed down unresolved.
    let mut client_cfg = inp.cfg.clone();
    client_cfg.resolve_for_model(&model);

    let prod_client = OpenAiClient::new(
        client_cfg.executor.api_key.clone().unwrap_or_default(),
        model.clone(),
        client_cfg.executor.base_url.clone(),
        std::time::Duration::from_secs(client_cfg.executor.first_token_timeout_secs),
        std::time::Duration::from_secs(client_cfg.executor.stream_idle_timeout_secs),
        SamplingParams {
            temperature: client_cfg.executor.temperature,
            seed: client_cfg.executor.seed,
            max_tokens: client_cfg.executor.max_tokens,
            enable_thinking: client_cfg.executor.enable_thinking,
        },
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
        model: &model,
        telemetry_dir: inp.telemetry_dir,
        progress: inp.progress,
        context_window: if inp.test_client.is_none() {
            rexymcp_executor::health::fetch_context_window(&inp.cfg.executor, &model).await
        } else {
            None
        },
        project_id: inp.project_id.clone(),
        resume: inp.resume.as_ref(),
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
        let (_registry, schemas) = build_registry(&scope, 30, true, None);

        assert_eq!(schemas.len(), 10);
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
                "bash",
                "delete_file",
                "move_file",
                "patch_lines"
            ]
        );

        let rf = schemas.iter().find(|s| s.name == "read_file").unwrap();
        assert!(!rf.parameters.is_null());
    }

    #[test]
    fn build_registry_includes_update_task_when_tasks_present() {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let tasks = vec![rexymcp_executor::agent::tasks::Task {
            id: "1".to_string(),
            title: "Test task".to_string(),
            state: rexymcp_executor::store::sessions::event::TaskState::Pending,
        }];
        let (_registry, schemas) = build_registry(&scope, 30, true, Some(tasks));

        assert_eq!(schemas.len(), 11);
        let names: Vec<_> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"update_task"));
    }

    #[test]
    fn build_registry_excludes_update_task_when_none() {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let (_registry, schemas) = build_registry(&scope, 30, true, None);

        assert_eq!(schemas.len(), 10);
        let names: Vec<_> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"update_task"));
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
            "# Phase 01: Test\n\n**Status:** review\n\n**Tags:** language=rust, kind=test, size=s\
             \n\n## Goal\n\nTest goal.\n\n## Acceptance criteria\n\n- [ ] It runs.\
             \n\n## Update Log\n\n<!-- entries appended below this line -->\
             \n\n### Update — 2026-01-01 00:00 (complete)\n\n**Summary:** done.\n",
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
            context_window: None,
            project_id: None,
            resume: None,
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

    #[tokio::test]
    async fn run_phase_with_finalizes_an_in_progress_doc_to_review() {
        let dir = TempDir::new().unwrap();
        let repo_dir = dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();

        let phase_doc_path = dir.path().join("phase-01-test.md");
        std::fs::write(
            &phase_doc_path,
            "# Phase 01: Test\n\n**Status:** in-progress\n\n**Tags:** language=rust, kind=test, size=s\
             \n\n## Goal\n\nTest goal.\n\n## Acceptance criteria\n\n- [ ] It runs.\
             \n\n## Update Log\n\n<!-- entries appended below this line -->\
             \n\n### Update — 2026-01-01 00:00 (started)\n",
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
            context_window: None,
            project_id: None,
            resume: None,
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
        let doc_after = std::fs::read_to_string(&phase_doc_path).unwrap();
        assert!(
            doc_after.contains("**Status:** review"),
            "finalize must flip the completed in-progress doc to review: {doc_after}"
        );
        assert!(
            doc_after.contains("(complete, server-authored)"),
            "finalize must append the server-authored completion entry: {doc_after}"
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
            context_window: None,
            project_id: None,
            resume: None,
        };
        let result = run_phase_with(&inp, &seams).await;

        assert!(result.is_err(), "should error on non-existent root");
    }

    // --- per-model override resolution wiring tests ---

    #[tokio::test]
    async fn run_phase_with_resolves_per_model_sampling_into_telemetry() {
        let dir = TempDir::new().unwrap();
        let repo_dir = dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();

        let phase_doc_path = dir.path().join("phase-01-test.md");
        std::fs::write(
            &phase_doc_path,
            "# Phase 01: Test\n\n**Tags:** language=rust, kind=test, size=s\n\n## Goal\n\nTest goal.\n\n## Acceptance criteria\n\n- [ ] It runs.\n",
        )
        .unwrap();

        let mut cfg = Config::default();
        cfg.executor.temperature = Some(0.8);
        cfg.models.insert(
            "override-model".into(),
            rexymcp_executor::config::ModelOverride {
                temperature: Some(0.2),
                seed: None,
                task_tracking: None,
                max_tokens: None,
                enable_thinking: None,
                identical_call_threshold: None,
                verifier_persistence_threshold: None,
                runaway_output_bytes: None,
                empty_completion_threshold: None,
                gate_feedback_repeat_threshold: None,
                oscillation_window: None,
                oscillation_distinct_max: None,
                output_window: None,
                output_window_bytes: None,
            },
        );
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
            model: "override-model",
            telemetry_dir: Some(dir.path()),
            progress: None,
            context_window: None,
            project_id: None,
            resume: None,
        };

        let result = run_phase_with(&inp, &seams).await;
        assert!(
            result.is_ok(),
            "run_phase_with should succeed: {:?}",
            result
        );

        let runs = rexymcp_executor::store::telemetry::read(&dir.path().join("phase_runs.jsonl"))
            .expect("telemetry should be readable");
        assert_eq!(runs.len(), 1, "exactly one phase run recorded");
        assert_eq!(
            runs[0].generation_params.temperature,
            Some(0.2),
            "temperature must be the per-model override (0.2), not the global (0.8)"
        );
    }

    #[tokio::test]
    async fn run_phase_with_records_configured_tier_in_telemetry() {
        let dir = TempDir::new().unwrap();
        let repo_dir = dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();

        let phase_doc_path = dir.path().join("phase-01-test.md");
        std::fs::write(
            &phase_doc_path,
            "# Phase 01: Test\n\n**Tags:** language=rust, kind=test, size=s\n\n## Goal\n\nTest goal.\n\n## Acceptance criteria\n\n- [ ] It runs.\n",
        )
        .unwrap();

        let mut cfg = Config::default();
        cfg.executor.tier = Some(rexymcp_executor::config::Tier::Medium);

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
            model: "m",
            telemetry_dir: Some(dir.path()),
            progress: None,
            context_window: None,
            project_id: None,
            resume: None,
        };

        let result = run_phase_with(&inp, &seams).await;
        assert!(result.is_ok(), "run_phase_with should succeed: {result:?}");

        let runs = rexymcp_executor::store::telemetry::read(&dir.path().join("phase_runs.jsonl"))
            .expect("telemetry should be readable");
        assert_eq!(runs.len(), 1, "exactly one phase run recorded");
        assert_eq!(
            runs[0].tier_telemetry.tier,
            Some(rexymcp_executor::config::Tier::Medium),
            "the configured tier must be recorded in the written telemetry"
        );
    }

    #[tokio::test]
    async fn run_phase_with_unknown_model_keeps_global_sampling() {
        let dir = TempDir::new().unwrap();
        let repo_dir = dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();

        let phase_doc_path = dir.path().join("phase-01-test.md");
        std::fs::write(
            &phase_doc_path,
            "# Phase 01: Test\n\n**Tags:** language=rust, kind=test, size=s\n\n## Goal\n\nTest goal.\n\n## Acceptance criteria\n\n- [ ] It runs.\n",
        )
        .unwrap();

        let mut cfg = Config::default();
        cfg.executor.temperature = Some(0.8);
        cfg.models.insert(
            "override-model".into(),
            rexymcp_executor::config::ModelOverride {
                temperature: Some(0.2),
                seed: None,
                task_tracking: None,
                max_tokens: None,
                enable_thinking: None,
                identical_call_threshold: None,
                verifier_persistence_threshold: None,
                runaway_output_bytes: None,
                empty_completion_threshold: None,
                gate_feedback_repeat_threshold: None,
                oscillation_window: None,
                oscillation_distinct_max: None,
                output_window: None,
                output_window_bytes: None,
            },
        );

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
            model: "different-model",
            telemetry_dir: Some(dir.path()),
            progress: None,
            context_window: None,
            project_id: None,
            resume: None,
        };

        let result = run_phase_with(&inp, &seams).await;
        assert!(
            result.is_ok(),
            "run_phase_with should succeed: {:?}",
            result
        );

        let runs = rexymcp_executor::store::telemetry::read(&dir.path().join("phase_runs.jsonl"))
            .expect("telemetry should be readable");
        assert_eq!(runs.len(), 1, "exactly one phase run recorded");
        assert_eq!(
            runs[0].generation_params.temperature,
            Some(0.8),
            "temperature must be the global (0.8) since 'different-model' has no [models] entry"
        );
    }

    // --- resolve_telemetry_dir tests ---

    #[test]
    fn resolve_telemetry_dir_defers_to_config_when_flag_absent() {
        let mut cfg = Config::default();
        cfg.telemetry.dir = Some(PathBuf::from("/tmp/telemetry"));
        assert_eq!(
            resolve_telemetry_dir(&cfg, false),
            Some(Path::new("/tmp/telemetry"))
        );

        let cfg_no_dir = Config::default();
        assert_eq!(resolve_telemetry_dir(&cfg_no_dir, false), None);
    }

    #[test]
    fn resolve_telemetry_dir_forces_none_when_flag_present() {
        let mut cfg = Config::default();
        cfg.telemetry.dir = Some(PathBuf::from("/tmp/telemetry"));
        assert_eq!(resolve_telemetry_dir(&cfg, true), None);

        let cfg_no_dir = Config::default();
        assert_eq!(resolve_telemetry_dir(&cfg_no_dir, true), None);
    }

    // --- collect_input_warnings ---

    #[test]
    fn collect_input_warnings_empty_when_all_present() {
        let warnings = collect_input_warnings("s", "g", "a");
        assert!(warnings.is_empty());
    }

    #[test]
    fn collect_input_warnings_flags_blank_standards() {
        let warnings = collect_input_warnings("", "g", "a");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("STANDARDS"));
    }

    #[test]
    fn collect_input_warnings_flags_whitespace_only_standards() {
        let warnings = collect_input_warnings("   \n", "g", "a");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("STANDARDS"));
    }

    #[test]
    fn collect_input_warnings_flags_blank_goal() {
        let warnings = collect_input_warnings("s", "", "a");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Goal"));
    }

    #[test]
    fn collect_input_warnings_flags_blank_criteria() {
        let warnings = collect_input_warnings("s", "g", "");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Acceptance criteria"));
    }

    #[test]
    fn collect_input_warnings_multiple_blank_inputs() {
        let warnings = collect_input_warnings("", "", "");
        assert_eq!(warnings.len(), 3);
    }

    // --- end-to-end: run_phase_with stamps input warnings ---

    #[tokio::test]
    async fn run_phase_with_stamps_input_warnings() {
        let dir = TempDir::new().unwrap();

        let repo_dir = dir.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::write(repo_dir.join(".gitkeep"), "").unwrap();

        let phase_doc_path = repo_dir.join("phase-01.md");
        std::fs::write(
            &phase_doc_path,
            "# Phase 01\n\n## Goal\n\nDo a thing.\n\n## Acceptance criteria\n\n- done\n",
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
            standards: "",
            model: "test-model",
            telemetry_dir: None,
            progress: None,
            context_window: None,
            project_id: None,
            resume: None,
        };

        let result = run_phase_with(&inp, &seams).await;

        assert!(
            result.is_ok(),
            "run_phase_with should succeed: {:?}",
            result
        );
        let phase_result = result.unwrap();

        assert!(
            !phase_result.warnings.is_empty(),
            "expected warnings when standards are blank"
        );
        let has_standards_warning = phase_result
            .warnings
            .iter()
            .any(|w| w.contains("STANDARDS"));
        assert!(
            has_standards_warning,
            "expected a STANDARDS warning in: {:?}",
            phase_result.warnings
        );
    }
}
