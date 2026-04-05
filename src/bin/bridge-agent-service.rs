use anyhow::Result;

#[cfg(windows)]
use anyhow::Context;

#[cfg(windows)]
use bridge_agent::{
    default_config_path, ensure_config_exists, install_rustls_crypto_provider,
    windows_service_config_path, AgentRuntimeManager,
};

#[cfg(windows)]
use clap::Parser;

#[cfg(windows)]
use std::ffi::OsString;

#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
use std::sync::mpsc;

#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use windows_service::define_windows_service;

#[cfg(windows)]
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};

#[cfg(windows)]
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};

#[cfg(windows)]
use windows_service::service_dispatcher;

#[cfg(windows)]
const SERVICE_NAME: &str = "BridgeAgent";

#[cfg(windows)]
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

#[cfg(windows)]
#[derive(Debug, Parser)]
#[command(name = "bridge-agent-service")]
#[command(about = "Windows service host for bridge-agent")]
struct ConsoleArgs {
    #[arg(long, env = "WS_BRIDGE_CONFIG")]
    config: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    console: bool,
}

#[cfg(windows)]
#[derive(Debug, Parser)]
struct ServiceLaunchArgs {
    #[arg(long, env = "WS_BRIDGE_CONFIG")]
    config: Option<PathBuf>,
}

#[cfg(windows)]
define_windows_service!(ffi_service_main, service_main);

#[cfg(windows)]
fn main() -> Result<()> {
    install_rustls_crypto_provider()?;
    init_tracing();

    let args = ConsoleArgs::parse();
    if args.console {
        return run_console(args.config);
    }

    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("failed to start Windows service dispatcher")?;
    Ok(())
}

#[cfg(not(windows))]
fn main() -> Result<()> {
    anyhow::bail!("bridge-agent-service only supports Windows")
}

#[cfg(windows)]
fn service_main(arguments: Vec<OsString>) {
    if let Err(err) = run_service(arguments) {
        eprintln!("bridge-agent-service failed: {err:#}");
    }
}

#[cfg(windows)]
fn run_service(arguments: Vec<OsString>) -> Result<()> {
    let service_args = ServiceLaunchArgs::try_parse_from(
        std::iter::once(OsString::from("bridge-agent-service")).chain(arguments),
    )
    .context("failed to parse Windows service launch arguments")?;
    let config_path = resolve_service_config_path(service_args.config)?;

    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let status_handle =
        service_control_handler::register(SERVICE_NAME, move |control| match control {
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = shutdown_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        })
        .context("failed to register Windows service control handler")?;

    status_handle
        .set_service_status(start_pending_status(1))
        .context("failed to mark service as start pending")?;

    let result = run_service_loop(&config_path, shutdown_rx, &status_handle);
    let exit_code = if result.is_ok() {
        ServiceExitCode::Win32(0)
    } else {
        ServiceExitCode::ServiceSpecific(1)
    };

    status_handle
        .set_service_status(stopped_status(exit_code))
        .context("failed to mark service as stopped")?;

    result
}

#[cfg(windows)]
fn run_service_loop(
    config_path: &PathBuf,
    shutdown_rx: mpsc::Receiver<()>,
    status_handle: &service_control_handler::ServiceStatusHandle,
) -> Result<()> {
    ensure_config_exists(config_path).with_context(|| {
        format!(
            "failed to ensure Windows service config exists at {}",
            config_path.display()
        )
    })?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build Tokio runtime for Windows service")?;
    let manager = AgentRuntimeManager::new();

    runtime
        .block_on(manager.start_from_path(config_path))
        .with_context(|| format!("failed to start runtime from {}", config_path.display()))?;

    status_handle
        .set_service_status(running_status())
        .context("failed to mark service as running")?;

    shutdown_rx.recv().ok();

    status_handle
        .set_service_status(stop_pending_status())
        .context("failed to mark service as stop pending")?;

    runtime
        .block_on(manager.stop())
        .context("failed to stop runtime during service shutdown")?;

    Ok(())
}

#[cfg(windows)]
fn run_console(config: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_service_config_path(config)?;
    ensure_config_exists(&config_path).with_context(|| {
        format!(
            "failed to ensure config exists at {}",
            config_path.display()
        )
    })?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build Tokio runtime for console mode")?;
    let manager = AgentRuntimeManager::new();

    runtime
        .block_on(manager.start_from_path(&config_path))
        .with_context(|| format!("failed to start runtime from {}", config_path.display()))?;

    println!(
        "bridge-agent-service running in console mode with config {}",
        config_path.display()
    );
    runtime.block_on(async {
        tokio::signal::ctrl_c()
            .await
            .context("failed to wait for Ctrl+C in console mode")?;
        manager.stop().await.context("failed to stop runtime")?;
        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

#[cfg(windows)]
fn resolve_service_config_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(path) = windows_service_config_path() {
        return Ok(path);
    }
    default_config_path()
}

#[cfg(windows)]
fn start_pending_status(checkpoint: u32) -> ServiceStatus {
    ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    }
}

#[cfg(windows)]
fn running_status() -> ServiceStatus {
    ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }
}

#[cfg(windows)]
fn stop_pending_status() -> ServiceStatus {
    ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::StopPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 1,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    }
}

#[cfg(windows)]
fn stopped_status(exit_code: ServiceExitCode) -> ServiceStatus {
    ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }
}

#[cfg(windows)]
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
