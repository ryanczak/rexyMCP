use clap::{Parser, Subcommand};
use rexymcp_executor::config::Config;
use rexymcp_executor::health;
use std::path::PathBuf;

mod cap;
mod dashboard;
mod doctor;
mod init;
mod log_query;
mod roots;
mod runner;
mod runs;
mod scorecard;
mod scorecard_cli;
mod server;
mod status;

#[derive(Parser)]
#[command(name = env!("CARGO_PKG_NAME"), version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Check connectivity to the configured LLM endpoint
    Health {
        /// Path to the config file
        #[arg(long)]
        config: Option<PathBuf>,

        /// Override the base URL from config
        #[arg(long)]
        base_url: Option<String>,
    },
    /// Execute a phase against a target repository
    RunPhase {
        /// Path to the config file
        #[arg(long)]
        config: PathBuf,

        /// Path to the phase-doc markdown file
        #[arg(long)]
        phase_doc: PathBuf,

        /// Path to the target repository root
        #[arg(long)]
        repo: PathBuf,

        /// Override the model ID from config
        #[arg(long)]
        model: Option<String>,
    },
    /// Run the MCP stdio server
    Serve {
        /// Path to the config file
        #[arg(long)]
        config: PathBuf,
    },
    /// Report the latest progress of a phase from its session log
    Status {
        /// Path to the target repository root
        #[arg(long)]
        repo: PathBuf,

        /// Select a specific session by a substring of its log filename
        /// (default: the most recently modified log)
        #[arg(long)]
        session: Option<String>,

        /// Emit the status as JSON instead of a human summary
        #[arg(long)]
        json: bool,
    },
    /// List individual PhaseRun records with their per-run statistics
    Runs {
        /// Path to the config file
        #[arg(long)]
        config: PathBuf,

        /// Restrict to one model (exact match)
        #[arg(long)]
        model: Option<String>,

        /// Restrict to runs whose tags contain this tag; repeat for AND
        #[arg(long = "tag")]
        tags: Vec<String>,

        /// Max rows (most recent first); 0 = no limit
        #[arg(long, default_value_t = 20)]
        limit: usize,

        /// Override the telemetry phase_runs.jsonl path
        #[arg(long)]
        telemetry_path: Option<PathBuf>,

        /// Emit JSON instead of a human table
        #[arg(long)]
        json: bool,
    },
    /// Aggregate runs into a model × settings competency matrix
    Scorecard {
        /// Path to the config file
        #[arg(long)]
        config: PathBuf,

        /// Restrict to one model (exact match)
        #[arg(long)]
        model: Option<String>,

        /// Restrict to runs whose tags contain this tag; repeat for AND
        #[arg(long = "tag")]
        tags: Vec<String>,

        /// Drop buckets with fewer than this many runs
        #[arg(long, default_value_t = 0)]
        min_runs: usize,

        /// Override the telemetry phase_runs.jsonl path
        #[arg(long)]
        telemetry_path: Option<PathBuf>,

        /// Emit JSON instead of a human table
        #[arg(long)]
        json: bool,
    },
    /// Scaffold rexymcp.toml in the target directory
    Init {
        /// Directory to initialise (default: current directory).
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        /// Overwrite existing files without prompting.
        #[arg(long)]
        force: bool,
    },
    /// Live dashboard — tails the active session log and refreshes continuously
    Dashboard {
        /// Target repo root (where `.rexymcp/sessions/` lives)
        #[arg(long)]
        repo: PathBuf,

        /// Session id to watch; omit to auto-select the most-recently-modified log
        #[arg(long)]
        session: Option<String>,

        /// Path to the config file
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Report whether the configured toolchain + verifier enhancers are on PATH
    Doctor {
        /// Path to the config file
        #[arg(long)]
        config: Option<PathBuf>,

        /// Emit the report as JSON instead of a human summary
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let Some(command) = cli.command else {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    };

    match command {
        Commands::Health { config, base_url } => {
            let config_path = config.unwrap_or_else(|| PathBuf::from("rexymcp.toml"));
            let mut cfg = Config::load_with_env(&config_path)?;

            if let Some(url) = base_url {
                cfg.executor.base_url = url;
            }

            let result = health::check(&cfg.executor).await;

            if result.reachable {
                println!("{}", result.base_url);
                if result.models.is_empty() {
                    println!("(no models reported)");
                } else {
                    for model in &result.models {
                        println!("{model}");
                    }
                }
                Ok(())
            } else {
                eprintln!("unreachable: {}", result.base_url);
                std::process::exit(1);
            }
        }
        Commands::RunPhase {
            config,
            phase_doc,
            repo,
            model,
        } => {
            let cfg = Config::load_with_env(&config)?;

            let standards_path = repo.join("docs/dev/STANDARDS.md");
            let standards = std::fs::read_to_string(&standards_path).unwrap_or_default();

            let project_id =
                rexymcp_executor::config::Config::load(&repo.join("rexymcp.toml"))
                    .ok()
                    .and_then(|c| c.project.id);

            let result = runner::run_phase(&runner::RunPhaseConfig {
                cfg: &cfg,
                phase_doc_path: &phase_doc,
                repo_path: &repo,
                standards: &standards,
                model_override: model.as_deref(),
                telemetry_dir: None,
                progress: None,
                project_id,
                test_client: None,
            })
            .await?;

            println!(
                "{}",
                serde_json::to_string_pretty(&result).unwrap_or_else(|e| {
                    format!("{{\"error\": \"failed to serialize PhaseResult: {}\"}}", e)
                })
            );
            Ok(())
        }
        Commands::Init { dir, force } => {
            init::run(&dir, force)?;
            Ok(())
        }
        Commands::Serve { config } => {
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "<unknown>".to_string());
            eprintln!(
                "rexymcp serve: starting MCP stdio server (version {}, cwd={}, config={}, config_exists={})",
                env!("CARGO_PKG_VERSION"),
                cwd,
                config.display(),
                config.exists()
            );
            let server = server::RexyMcpServer {
                config_path: config,
            };
            let transport = rmcp::transport::stdio();
            let _running = rmcp::serve_server(server, transport)
                .await
                .map_err(|e| anyhow::anyhow!("MCP server failed: {}", e))?;
            tokio::signal::ctrl_c()
                .await
                .map_err(|e| anyhow::anyhow!("failed to wait for signal: {}", e))?;
            Ok(())
        }
        Commands::Status {
            repo,
            session,
            json,
        } => {
            let summary = match status::load_status(&repo, session.as_deref()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&summary).unwrap_or_else(|e| {
                        format!("{{\"error\": \"failed to serialize status: {}\"}}", e)
                    })
                );
            } else {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                println!("{}", status::format_status(&summary, now_ms));
            }
            Ok(())
        }
        Commands::Runs {
            config,
            model,
            tags,
            limit,
            telemetry_path,
            json,
        } => {
            let filter = runs::RunsFilter {
                model: model.as_deref(),
                tags: &tags,
                limit,
            };

            let selected = match runs::load_runs(&config, telemetry_path.as_deref(), &filter) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&selected).unwrap_or_else(|e| {
                        format!("{{\"error\": \"failed to serialize runs: {}\"}}", e)
                    })
                );
            } else {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                println!("{}", runs::format_runs(&selected, now_ms));
            }
            Ok(())
        }
        Commands::Scorecard {
            config,
            model,
            tags,
            min_runs,
            telemetry_path,
            json,
        } => {
            let filter = scorecard::ScorecardFilter {
                model: model.as_deref(),
                tags: &tags,
                min_runs,
            };

            let rows = match scorecard_cli::load_settings_scorecard(
                &config,
                telemetry_path.as_deref(),
                &filter,
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&rows).unwrap_or_else(|e| {
                        format!("{{\"error\": \"failed to serialize scorecard: {}\"}}", e)
                    })
                );
            } else {
                println!("{}", scorecard_cli::format_settings_scorecard(&rows));
            }
            Ok(())
        }
        Commands::Dashboard {
            repo,
            session,
            config,
        } => {
            let config_path = config.unwrap_or_else(|| PathBuf::from("rexymcp.toml"));
            let cfg = Config::load_with_env(&config_path)?;
            let d = &cfg.dashboard;
            let rates = d
                .saved_model
                .as_deref()
                .and_then(dashboard::model_rates)
                .unwrap_or(dashboard::BudgetRates {
                    input_per_mtok: d.saved_input_per_mtok,
                    output_per_mtok: d.saved_output_per_mtok,
                });
            let telemetry_dir = cfg.telemetry.dir.as_deref();
            let project_id = rexymcp_executor::config::Config::load(&repo.join("rexymcp.toml"))
                .ok()
                .and_then(|c| c.project.id);
            dashboard::run_dashboard(&repo, session.as_deref(), rates, telemetry_dir, project_id)
                .unwrap_or_else(|e| {
                    eprintln!("dashboard error: {e}");
                    std::process::exit(1);
                });
            Ok(())
        }
        Commands::Doctor { config, json } => {
            let config_path = config.unwrap_or_else(|| PathBuf::from("rexymcp.toml"));
            let cfg = Config::load_with_env(&config_path)?;
            let ok = doctor::run(&cfg.commands, json);
            if ok {
                Ok(())
            } else {
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Commands};
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn cli_parse_run_phase_with_all_args() {
        let cli = Cli::try_parse_from([
            "rexymcp",
            "run-phase",
            "--config",
            "rexymcp.toml",
            "--phase-doc",
            "docs/dev/milestones/M5-mcp-server/phase-01-phase-runner.md",
            "--repo",
            "/tmp/repo",
            "--model",
            "qwen2.5-coder",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::RunPhase {
                config,
                phase_doc,
                repo,
                model,
            }) => {
                assert_eq!(config, PathBuf::from("rexymcp.toml"));
                assert_eq!(
                    phase_doc,
                    PathBuf::from("docs/dev/milestones/M5-mcp-server/phase-01-phase-runner.md")
                );
                assert_eq!(repo, PathBuf::from("/tmp/repo"));
                assert_eq!(model.as_deref(), Some("qwen2.5-coder"));
            }
            _ => panic!("expected RunPhase"),
        }
    }

    #[test]
    fn cli_parse_run_phase_model_optional() {
        let cli = Cli::try_parse_from([
            "rexymcp",
            "run-phase",
            "--config",
            "rexymcp.toml",
            "--phase-doc",
            "phase-doc.md",
            "--repo",
            "/tmp/repo",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::RunPhase { model, .. }) => {
                assert_eq!(model, None);
            }
            _ => panic!("expected RunPhase"),
        }
    }

    #[test]
    fn cli_parse_run_phase_missing_required_arg_fails() {
        let result = Cli::try_parse_from(["rexymcp", "run-phase", "--config", "rexymcp.toml"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_parse_serve_with_config() {
        let cli = Cli::try_parse_from(["rexymcp", "serve", "--config", "rexymcp.toml"]).unwrap();

        match cli.command {
            Some(Commands::Serve { config }) => {
                assert_eq!(config, PathBuf::from("rexymcp.toml"));
            }
            _ => panic!("expected Serve"),
        }
    }

    #[test]
    fn cli_parse_serve_missing_config_fails() {
        let result = Cli::try_parse_from(["rexymcp", "serve"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_parse_status_with_repo_only() {
        let cli = Cli::try_parse_from(["rexymcp", "status", "--repo", "/tmp/repo"]).unwrap();
        match cli.command {
            Some(Commands::Status {
                repo,
                session,
                json,
            }) => {
                assert_eq!(repo, PathBuf::from("/tmp/repo"));
                assert_eq!(session, None);
                assert!(!json);
            }
            _ => panic!("expected Status"),
        }
    }

    #[test]
    fn cli_parse_status_with_session_and_json() {
        let cli = Cli::try_parse_from([
            "rexymcp",
            "status",
            "--repo",
            "/tmp/repo",
            "--session",
            "abc123",
            "--json",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Status { session, json, .. }) => {
                assert_eq!(session.as_deref(), Some("abc123"));
                assert!(json);
            }
            _ => panic!("expected Status"),
        }
    }

    #[test]
    fn cli_parse_status_missing_repo_fails() {
        let result = Cli::try_parse_from(["rexymcp", "status"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_parse_runs_collects_filters() {
        let cli = Cli::try_parse_from([
            "rexymcp",
            "runs",
            "--config",
            "rexymcp.toml",
            "--model",
            "qwen",
            "--tag",
            "rust",
            "--tag",
            "feature",
            "--limit",
            "5",
            "--json",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Runs {
                config,
                model,
                tags,
                limit,
                json,
                ..
            }) => {
                assert_eq!(config, PathBuf::from("rexymcp.toml"));
                assert_eq!(model.as_deref(), Some("qwen"));
                assert_eq!(tags, ["rust", "feature"]);
                assert_eq!(limit, 5);
                assert!(json);
            }
            _ => panic!("expected Runs"),
        }
    }

    #[test]
    fn cli_parse_scorecard_collects_filters() {
        let cli = Cli::try_parse_from([
            "rexymcp",
            "scorecard",
            "--config",
            "rexymcp.toml",
            "--model",
            "qwen",
            "--tag",
            "rust",
            "--min-runs",
            "3",
            "--json",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Scorecard {
                config,
                model,
                tags,
                min_runs,
                json,
                ..
            }) => {
                assert_eq!(config, PathBuf::from("rexymcp.toml"));
                assert_eq!(model.as_deref(), Some("qwen"));
                assert_eq!(tags, ["rust"]);
                assert_eq!(min_runs, 3);
                assert!(json);
            }
            _ => panic!("expected Scorecard"),
        }
    }

    #[test]
    fn cli_parse_dashboard_collects_args() {
        let cli = Cli::try_parse_from([
            "rexymcp",
            "dashboard",
            "--repo",
            "/some/path",
            "--session",
            "sess-123",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Dashboard { repo, session, .. }) => {
                assert_eq!(repo, PathBuf::from("/some/path"));
                assert_eq!(session.as_deref(), Some("sess-123"));
            }
            _ => panic!("expected Dashboard"),
        }

        let cli2 = Cli::try_parse_from(["rexymcp", "dashboard", "--repo", "/p"]).unwrap();
        match cli2.command {
            Some(Commands::Dashboard {
                session, config, ..
            }) => {
                assert_eq!(session, None);
                assert_eq!(config, None);
            }
            _ => panic!("expected Dashboard"),
        }
    }

    #[test]
    fn cli_parse_doctor_with_config_and_json() {
        let cli = Cli::try_parse_from([
            "rexymcp",
            "doctor",
            "--config",
            "/some/rexymcp.toml",
            "--json",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Doctor { config, json }) => {
                assert_eq!(config, Some(PathBuf::from("/some/rexymcp.toml")));
                assert!(json);
            }
            _ => panic!("expected Doctor"),
        }

        let cli2 = Cli::try_parse_from(["rexymcp", "doctor"]).unwrap();
        match cli2.command {
            Some(Commands::Doctor { config, json }) => {
                assert_eq!(config, None);
                assert!(!json);
            }
            _ => panic!("expected Doctor"),
        }
    }
}
