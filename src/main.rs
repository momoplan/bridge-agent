use anyhow::{Context, Result};
use bridge_agent::{
    default_config_path, ensure_config_exists, install_rustls_crypto_provider, load_config,
    save_config, AgentConfig, AgentRuntimeManager, ServiceConfig, ServiceRegistration,
};
use clap::{Parser, Subcommand};
use serde::Deserialize;
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
    RegisterService {
        #[arg(long)]
        file: PathBuf,
        #[arg(long, env = "WS_BRIDGE_CONFIG")]
        config: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        replace: bool,
    },
    UnregisterService {
        name: String,
        #[arg(long, env = "WS_BRIDGE_CONFIG")]
        config: Option<PathBuf>,
    },
    ListServices {
        #[arg(long, env = "WS_BRIDGE_CONFIG")]
        config: Option<PathBuf>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ServiceRegistrationFile {
    Public(ServiceRegistration),
    Raw(ServiceConfig),
}

#[tokio::main]
async fn main() -> Result<()> {
    install_rustls_crypto_provider()?;
    init_tracing();
    match Cli::parse().command {
        Command::Run { config } => run_command(config).await,
        Command::InitConfig { output, force } => init_config_command(output, force).await,
        Command::PrintExampleConfig => print_example_config(),
        Command::RegisterService {
            file,
            config,
            replace,
        } => register_service_command(file, config, replace).await,
        Command::UnregisterService { name, config } => {
            unregister_service_command(name, config).await
        }
        Command::ListServices { config } => list_services_command(config).await,
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

async fn register_service_command(
    file: PathBuf,
    config: Option<PathBuf>,
    replace: bool,
) -> Result<()> {
    let config_path = config.unwrap_or(default_config_path()?);
    ensure_config_exists(&config_path)?;
    let content = std::fs::read_to_string(&file)
        .with_context(|| format!("failed to read service registration {}", file.display()))?;
    let service_file: ServiceRegistrationFile =
        serde_json::from_str(&content).with_context(|| "failed to parse service registration")?;
    let mut service = match service_file {
        ServiceRegistrationFile::Public(registration) => registration.into_service_config()?,
        ServiceRegistrationFile::Raw(service) => service,
    };
    service.name = service.name.trim().to_string();
    if service.name.is_empty() {
        anyhow::bail!("service name cannot be empty");
    }

    let mut config = load_config(&config_path)?;
    match config
        .services
        .iter()
        .position(|candidate| candidate.name == service.name)
    {
        Some(index) if replace => config.services[index] = service.clone(),
        Some(_) => anyhow::bail!(
            "service `{}` already exists; pass --replace to overwrite",
            service.name
        ),
        None => config.services.push(service.clone()),
    }
    save_config(&config_path, &config)?;
    println!("{}", service.name);
    Ok(())
}

async fn unregister_service_command(name: String, config: Option<PathBuf>) -> Result<()> {
    let config_path = config.unwrap_or(default_config_path()?);
    ensure_config_exists(&config_path)?;
    let mut config = load_config(&config_path)?;
    let normalized = name.trim();
    let initial_len = config.services.len();
    config.services.retain(|service| service.name != normalized);
    if config.services.len() == initial_len {
        anyhow::bail!("service `{normalized}` is not registered");
    }
    save_config(&config_path, &config)?;
    println!("{normalized}");
    Ok(())
}

async fn list_services_command(config: Option<PathBuf>) -> Result<()> {
    let config_path = config.unwrap_or(default_config_path()?);
    ensure_config_exists(&config_path)?;
    let config = load_config(&config_path)?;
    println!("{}", serde_json::to_string_pretty(&config.services)?);
    Ok(())
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
