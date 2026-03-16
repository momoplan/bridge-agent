use anyhow::{Context, Result};
use bridge_agent::{
    default_config_path, ensure_config_exists, install_rustls_crypto_provider, save_config,
    AgentConfig, AgentRuntimeManager,
};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "bridge-agent")]
#[command(about = "Local bridge agent with CLI and desktop runtime support")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Run {
        #[arg(long, env = "WS_BRIDGE_CONFIG")]
        config: Option<PathBuf>,
    },
    InitConfig {
        #[arg(long, env = "WS_BRIDGE_CONFIG")]
        output: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    PrintExampleConfig,
}

#[tokio::main]
async fn main() -> Result<()> {
    install_rustls_crypto_provider()?;
    init_tracing();
    match Cli::parse().command {
        Command::Run { config } => run_command(config).await,
        Command::InitConfig { output, force } => init_config_command(output, force).await,
        Command::PrintExampleConfig => print_example_config(),
    }
}

async fn run_command(config: Option<PathBuf>) -> Result<()> {
    let config_path = config.unwrap_or(default_config_path()?);
    ensure_config_exists(&config_path)?;

    let runtime = AgentRuntimeManager::new();
    runtime
        .start_from_path(&config_path)
        .await
        .with_context(|| format!("failed to start runtime from {}", config_path.display()))?;

    tracing::info!("bridge-agent started, press Ctrl+C to stop");
    tokio::signal::ctrl_c().await?;
    runtime.stop().await?;
    Ok(())
}

async fn init_config_command(output: Option<PathBuf>, force: bool) -> Result<()> {
    let path = output.unwrap_or(default_config_path()?);
    if path.exists() && !force {
        anyhow::bail!(
            "config file already exists at {}. pass --force to overwrite",
            path.display()
        );
    }

    save_config(&path, &AgentConfig::example())?;
    println!("{}", path.display());
    Ok(())
}

fn print_example_config() -> Result<()> {
    let payload = serde_json::to_string_pretty(&AgentConfig::example())?;
    println!("{payload}");
    Ok(())
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
