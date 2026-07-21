use clap::{Parser, Subcommand};
use rexymcp_executor::config::Config;
use rexymcp_executor::health;
use std::path::PathBuf;

mod calibrate;
mod calibrate_governor;
mod cap;
mod dashboard;
mod doctor;
mod finalize;
mod harvest;
mod init;
mod jobs;
mod journal;
mod log_query;
mod profile;
mod profile_cli;
mod resume;
mod review;
mod roots;
mod runner;
mod runs;
mod scorecard;
mod scorecard_cli;
mod server;
mod status;
mod stop;
mod stop_watcher;

#[derive(Parser)]
#[command(name = env!("CARGO_PKG_NAME"), version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::ValueEnum, Clone, Copy)]
enum CalibrateArg {
    #[value(name = "LARGE")]
    Large,
    #[value(name = "MEDIUM")]
    Medium,
    #[value(name = "SMALL")]
    Small,
}

impl From<CalibrateArg> for rexymcp_executor::config::Tier {
    fn from(a: CalibrateArg) -> Self {
        match a {
            CalibrateArg::Large => Self::Large,
            CalibrateArg::Medium => Self::Medium,
            CalibrateArg::Small => Self::Small,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy)]
enum ByArg {
    #[value(name = "model")]
    Model,
    #[value(name = "tag")]
    Tag,
    #[value(name = "settings")]
    Settings,
}

impl From<ByArg> for scorecard::ScorecardDimension {
    fn from(a: ByArg) -> Self {
        match a {
            ByArg::Model => Self::Model,
            ByArg::Tag => Self::Tag,
            ByArg::Settings => Self::Settings,
        }
    }
}

#[derive(Subcommand)]
enum RunsCommand {
    /// Show one run's full detail by id (8-hex, or an unambiguous prefix)
    Show {
        /// Run id from the `ID` column of `rexymcp runs`
        id: String,
    },
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
    /// Set the executor capability tier and write tier-derived defaults to the
    /// config file
    Calibrate {
        /// Capability tier: LARGE (Deepseek/Qwen3.6+), MEDIUM (Qwen3.6-27B /
        /// Gemma4-31b), or SMALL (Qwen3.5-coder-12b / Gemma-12b)
        #[arg(value_enum)]
        tier: CalibrateArg,

        /// Path to the config file
        #[arg(long, default_value = "rexymcp.toml")]
        config: PathBuf,
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

        /// Skip writing a PhaseRun telemetry record for this run, even if
        /// [telemetry] dir is configured
        #[arg(long)]
        no_telemetry: bool,
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
    /// List individual PhaseRun records, or show one in detail
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

        /// Subcommand: `show <id>` drills into one run. Absent = list.
        #[command(subcommand)]
        command: Option<RunsCommand>,
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

        /// Bucket by this dimension (model | tag | settings)
        #[arg(long, value_enum, default_value = "settings")]
        by: ByArg,
    },
    Profile {
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

        /// Report tokens & cost to ship, per approved phase, instead of the
        /// model×tag capability table.
        #[arg(long)]
        cost: bool,
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
    /// Signal a running executor to stop — writes `.rexymcp/stop` in the target repo,
    /// which the serve-side watcher (or a blocking `run-phase`) sees and cancels.
    Stop {
        /// Target repo root (where `.rexymcp/` lives). Defaults to the current dir.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
    /// Record an architect review verdict as a PhaseReview annotation
    Review {
        /// Path to the config file
        #[arg(long)]
        config: PathBuf,

        /// Absolute path to the phase doc under review (primary fold key)
        #[arg(long)]
        phase_doc: Option<PathBuf>,

        /// Phase id label (e.g. phase-01); also the fallback fold key
        #[arg(long)]
        phase_id: String,

        /// Project id; defaults to [project].id from config when omitted
        #[arg(long)]
        project_id: Option<String>,

        /// The verdict string (e.g. approved_first_try, approved_after_1, escalated)
        #[arg(long)]
        verdict: String,

        /// Failure class from the canonical vocabulary; repeat for several
        #[arg(long = "failure-class")]
        failure_class: Vec<String>,

        /// Bounces to approval
        #[arg(long)]
        bounces: Option<u32>,

        /// Bugs filed during review
        #[arg(long)]
        bugs_filed: Option<u32>,

        /// Warnings noted during review
        #[arg(long)]
        warnings: Option<u32>,

        /// Override the telemetry phase_runs.jsonl path
        #[arg(long)]
        telemetry_path: Option<PathBuf>,
    },

    /// Record an architect loop activity as an ArchitectActivity journal record
    Journal {
        /// Path to the rexymcp config file
        #[arg(long)]
        config: PathBuf,

        /// Path to the phase doc (for phase_doc_path in the record)
        #[arg(long)]
        phase_doc: Option<PathBuf>,

        /// Phase identifier (e.g. "phase-02")
        #[arg(long)]
        phase_id: String,

        /// Project ID override (defaults to [project].id from config)
        #[arg(long)]
        project_id: Option<String>,

        /// Milestone directory slug (e.g. "M27-autonomous-escalation-loop")
        #[arg(long)]
        milestone_id: Option<String>,

        /// The activity kind (e.g. "draft", "dispatch", "review", "assist", "takeover", "boundary")
        #[arg(long)]
        activity: String,

        /// Free-text outcome (e.g. "complete", "hard_fail", "bounced")
        #[arg(long)]
        outcome: Option<String>,

        /// Architect model that performed the activity
        #[arg(long)]
        model: Option<String>,

        /// Override the telemetry phase_runs.jsonl path
        #[arg(long)]
        telemetry_path: Option<PathBuf>,
    },

    /// Harvest Claude Code transcript token usage onto journal activities
    Harvest {
        /// Path to the rexymcp config file
        #[arg(long)]
        config: PathBuf,

        /// Directory of Claude Code *.jsonl session transcripts
        #[arg(long)]
        transcript_dir: PathBuf,

        /// Project ID override (defaults to [project].id from config)
        #[arg(long)]
        project_id: Option<String>,

        /// Override the telemetry phase_runs.jsonl path
        #[arg(long)]
        telemetry_path: Option<PathBuf>,
    },
    /// Calibrate governor thresholds by replaying the session-log corpus
    CalibrateGovernor {
        /// Target repo root (where `.rexymcp/sessions/` lives)
        #[arg(long, default_value = ".")]
        repo: PathBuf,

        /// Override the sessions directory (default: `<repo>/.rexymcp/sessions`)
        #[arg(long)]
        sessions_dir: Option<PathBuf>,

        /// Restrict to one model (exact match)
        #[arg(long)]
        model: Option<String>,

        /// Novelty window size (default: 24)
        #[arg(long, default_value_t = 24)]
        novelty_window: usize,

        /// Drop per-model cells with fewer than this many samples (default: 0)
        #[arg(long, default_value_t = 0)]
        min_runs: usize,

        /// Emit JSON instead of a human table
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
        Commands::Calibrate { tier, config } => {
            calibrate::run(&calibrate::CalibrateArgs {
                tier: tier.into(),
                config_path: &config,
            })?;
            Ok(())
        }
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
            no_telemetry,
        } => {
            let cfg = Config::load_with_env(&config)?;

            let standards_path = repo.join("docs/dev/STANDARDS.md");
            let standards = std::fs::read_to_string(&standards_path).unwrap_or_default();

            let project_id = rexymcp_executor::config::Config::load(&repo.join("rexymcp.toml"))
                .ok()
                .and_then(|c| c.project.id);

            let (cancel_handle, cancel_signal) = rexymcp_executor::agent::CancelSignal::new();
            let stop_watcher = tokio::spawn(stop_watcher::watch_stop_sentinel_single(
                repo.clone(),
                cancel_handle,
                stop_watcher::STOP_POLL_INTERVAL,
            ));

            let result = runner::run_phase(&runner::RunPhaseConfig {
                cfg: &cfg,
                phase_doc_path: &phase_doc,
                repo_path: &repo,
                standards: &standards,
                model_override: model.as_deref(),
                telemetry_dir: runner::resolve_telemetry_dir(&cfg, no_telemetry),
                progress: None,
                project_id,
                resume: None,
                test_client: None,
                cancel: cancel_signal,
            })
            .await;

            stop_watcher.abort();
            let result = result?;

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
            let server = server::RexyMcpServer::new(config);
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
            command,
        } => {
            if let Some(RunsCommand::Show { id }) = command {
                let filter = runs::RunsFilter {
                    model: None,
                    tags: &[],
                    limit: 0,
                };
                let all = match runs::load_runs(&config, telemetry_path.as_deref(), &filter) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                };
                match runs::find_run_by_id(&all, &id) {
                    Ok(run) => {
                        let cfg = match rexymcp_executor::config::Config::load_with_env(&config) {
                            Ok(c) => c,
                            Err(e) => {
                                eprintln!("failed to load config: {e}");
                                std::process::exit(1);
                            }
                        };
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);
                        println!("{}", runs::format_run_detail(run, now_ms, &cfg));
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
                return Ok(());
            }
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
                let cfg = match rexymcp_executor::config::Config::load_with_env(&config) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("failed to load config: {e}");
                        std::process::exit(1);
                    }
                };
                println!("{}", runs::format_runs(&selected, now_ms, &cfg));
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
            by,
        } => {
            let filter = scorecard::ScorecardFilter {
                model: model.as_deref(),
                tags: &tags,
                min_runs,
            };

            let dimension: scorecard::ScorecardDimension = by.into();

            let rows = match scorecard_cli::load_scorecard(
                &config,
                telemetry_path.as_deref(),
                dimension,
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
                println!("{}", scorecard_cli::format_scorecard(&rows, dimension));
            }
            Ok(())
        }
        Commands::Profile {
            config,
            model,
            tags,
            min_runs,
            telemetry_path,
            json,
            cost,
        } => {
            if cost {
                let filter = scorecard::ScorecardFilter {
                    model: model.as_deref(),
                    tags: &tags,
                    min_runs,
                };
                let rows = match profile_cli::load_phase_costs(
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
                            format!("{{\"error\": \"failed to serialize phase costs: {}\"}}", e)
                        })
                    );
                } else {
                    let cfg = match rexymcp_executor::config::Config::load_with_env(&config) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("failed to load config: {e}");
                            std::process::exit(1);
                        }
                    };
                    println!("{}", profile_cli::format_phase_costs(&rows, &cfg));
                }
                return Ok(());
            }

            let filter = scorecard::ScorecardFilter {
                model: model.as_deref(),
                tags: &tags,
                min_runs,
            };

            let rows = match profile_cli::load_profiles(&config, telemetry_path.as_deref(), &filter)
            {
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
                        format!("{{\"error\": \"failed to serialize profile: {}\"}}", e)
                    })
                );
            } else {
                println!("{}", profile_cli::format_profiles(&rows));
            }
            Ok(())
        }
        Commands::Review {
            config,
            phase_doc,
            phase_id,
            project_id,
            verdict,
            failure_class,
            bounces,
            bugs_filed,
            warnings,
            telemetry_path,
        } => {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let args = review::ReviewArgs {
                phase_doc: phase_doc.as_deref(),
                phase_id: &phase_id,
                project_id: project_id.as_deref(),
                verdict: &verdict,
                failure_class: &failure_class,
                bounces,
                bugs_filed,
                warnings,
            };
            match review::record_review(&config, telemetry_path.as_deref(), now_ms, &args) {
                Ok(outcome) => {
                    for unknown in &outcome.unknown_classes {
                        eprintln!(
                            "warning: unknown failure class {:?} (recorded anyway); known classes: {:?}",
                            unknown,
                            rexymcp_executor::store::telemetry::FAILURE_CLASSES
                        );
                    }
                    println!(
                        "recorded review for {} -> {}",
                        phase_id,
                        outcome.path.display()
                    );
                    Ok(())
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Dashboard {
            repo,
            session,
            config,
        } => {
            let config_path = config.unwrap_or_else(|| PathBuf::from("rexymcp.toml"));
            let cfg = Config::load_with_env(&config_path)?;
            let (i, o) = cfg.dashboard.effective_rates();
            let rates = dashboard::BudgetRates {
                input_per_mtok: i,
                output_per_mtok: o,
                architect: cfg.architect.effective_architect_rates(),
            };
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
        Commands::Stop { repo } => {
            let path = stop::write_sentinel(&repo)?;
            println!("wrote stop sentinel: {}", path.display());
            println!("running executors in this repo will cancel within ~1s.");
            Ok(())
        }
        Commands::Journal {
            config,
            phase_doc,
            phase_id,
            project_id,
            milestone_id,
            activity,
            outcome,
            model,
            telemetry_path,
        } => {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let args = journal::JournalArgs {
                phase_doc: phase_doc.as_deref(),
                phase_id: &phase_id,
                project_id: project_id.as_deref(),
                milestone_id: milestone_id.as_deref(),
                activity: &activity,
                outcome: outcome.as_deref(),
                model: model.as_deref(),
            };
            match journal::record_activity(&config, telemetry_path.as_deref(), now_ms, &args) {
                Ok(outcome) => {
                    if let Some(ref unknown) = outcome.unknown_activity {
                        eprintln!(
                            "warning: unknown activity kind {:?} (recorded anyway); known activities: {:?}",
                            unknown,
                            rexymcp_executor::store::telemetry::ARCHITECT_ACTIVITIES
                        );
                    }
                    println!(
                        "recorded {} activity for {} -> {}",
                        activity,
                        phase_id,
                        outcome.path.display()
                    );
                    Ok(())
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Harvest {
            config,
            transcript_dir,
            project_id,
            telemetry_path,
        } => {
            let args = harvest::HarvestArgs {
                transcript_dir: &transcript_dir,
                project_id: project_id.as_deref(),
            };
            match harvest::harvest(&config, telemetry_path.as_deref(), &args) {
                Ok(o) => {
                    println!(
                        "harvested {} messages, enriched {} activities ({} unattributed) -> {}",
                        o.messages,
                        o.enriched,
                        o.unattributed,
                        o.path.display()
                    );
                    Ok(())
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::CalibrateGovernor {
            repo,
            sessions_dir,
            model,
            novelty_window,
            min_runs,
            json,
        } => {
            let sessions_dir =
                sessions_dir.unwrap_or_else(|| repo.join(".rexymcp").join("sessions"));
            let model_ref = model.as_deref();
            let args = calibrate_governor::CalibrateGovernorArgs {
                sessions_dir: &sessions_dir,
                model_filter: model_ref,
                novelty_window,
                min_runs,
                json,
            };
            println!("{}", calibrate_governor::run(&args));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CalibrateArg, Cli, Commands};
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
                no_telemetry,
            }) => {
                assert_eq!(config, PathBuf::from("rexymcp.toml"));
                assert_eq!(
                    phase_doc,
                    PathBuf::from("docs/dev/milestones/M5-mcp-server/phase-01-phase-runner.md")
                );
                assert_eq!(repo, PathBuf::from("/tmp/repo"));
                assert_eq!(model.as_deref(), Some("qwen2.5-coder"));
                assert!(!no_telemetry);
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
    fn cli_parse_run_phase_no_telemetry_flag_sets_true() {
        let cli = Cli::try_parse_from([
            "rexymcp",
            "run-phase",
            "--config",
            "rexymcp.toml",
            "--phase-doc",
            "phase-doc.md",
            "--repo",
            "/tmp/repo",
            "--no-telemetry",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::RunPhase { no_telemetry, .. }) => {
                assert!(no_telemetry);
            }
            _ => panic!("expected RunPhase"),
        }
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
    fn cli_parse_profile_collects_filters() {
        let cli = Cli::try_parse_from([
            "rexymcp",
            "profile",
            "--config",
            "rexymcp.toml",
            "--model",
            "qwen",
            "--tag",
            "rust",
            "--tag",
            "feature",
            "--min-runs",
            "3",
            "--json",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Profile {
                config,
                model,
                tags,
                min_runs,
                json,
                ..
            }) => {
                assert_eq!(config, PathBuf::from("rexymcp.toml"));
                assert_eq!(model.as_deref(), Some("qwen"));
                assert_eq!(tags, ["rust", "feature"]);
                assert_eq!(min_runs, 3);
                assert!(json);
            }
            _ => panic!("expected Profile"),
        }
    }

    #[test]
    fn cli_parse_profile_cost_sets_cost_flag() {
        let cli = Cli::try_parse_from(["rexymcp", "profile", "--config", "rexymcp.toml", "--cost"])
            .unwrap();

        match cli.command {
            Some(Commands::Profile { cost, .. }) => {
                assert!(cost);
            }
            _ => panic!("expected Profile"),
        }
    }

    #[test]
    fn cli_parse_profile_without_cost_flag() {
        let cli = Cli::try_parse_from(["rexymcp", "profile", "--config", "rexymcp.toml"]).unwrap();

        match cli.command {
            Some(Commands::Profile { cost, .. }) => {
                assert!(!cost);
            }
            _ => panic!("expected Profile"),
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

    #[test]
    fn cli_parse_calibrate_medium() {
        let cli = Cli::try_parse_from(["rexymcp", "calibrate", "MEDIUM"]).unwrap();
        match cli.command {
            Some(Commands::Calibrate { tier, config }) => {
                assert!(matches!(tier, CalibrateArg::Medium));
                assert_eq!(config, PathBuf::from("rexymcp.toml"));
            }
            _ => panic!("expected Calibrate"),
        }
    }

    #[test]
    fn cli_parse_calibrate_small_with_config() {
        let cli = Cli::try_parse_from([
            "rexymcp",
            "calibrate",
            "SMALL",
            "--config",
            "/path/rexymcp.toml",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Calibrate { tier, config }) => {
                assert!(matches!(tier, CalibrateArg::Small));
                assert_eq!(config, PathBuf::from("/path/rexymcp.toml"));
            }
            _ => panic!("expected Calibrate"),
        }
    }

    #[test]
    fn cli_parse_calibrate_missing_tier_fails() {
        let result = Cli::try_parse_from(["rexymcp", "calibrate"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_parse_runs_show_id() {
        let cli = Cli::try_parse_from([
            "rexymcp",
            "runs",
            "--config",
            "rexymcp.toml",
            "show",
            "a3f9c1e2",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Runs {
                command: Some(super::RunsCommand::Show { id }),
                ..
            }) => {
                assert_eq!(id, "a3f9c1e2");
            }
            _ => panic!("expected runs show"),
        }
    }

    #[test]
    fn cli_parse_bare_runs_is_list() {
        let cli = Cli::try_parse_from(["rexymcp", "runs", "--config", "rexymcp.toml"]).unwrap();
        match cli.command {
            Some(Commands::Runs { command: None, .. }) => {}
            _ => panic!("expected bare runs list"),
        }
    }
}
