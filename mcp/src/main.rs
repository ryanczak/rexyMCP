use clap::{Parser, Subcommand};
use rexymcp_executor::config::Config;
use rexymcp_executor::health;
use std::path::PathBuf;

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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let Some(Commands::Health { config, base_url }) = cli.command else {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    };

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
