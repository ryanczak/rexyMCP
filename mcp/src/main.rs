use clap::{Parser, Subcommand};
use rexymcp_executor::config::Config;
use rexymcp_executor::health;
use std::path::PathBuf;

mod cap;
mod log_query;
mod roots;
mod runner;
mod scorecard;
mod server;

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

            let result = runner::run_phase(&runner::RunPhaseConfig {
                cfg: &cfg,
                phase_doc_path: &phase_doc,
                repo_path: &repo,
                standards: &standards,
                model_override: model.as_deref(),
                telemetry_dir: None,
                progress: None,
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
}
