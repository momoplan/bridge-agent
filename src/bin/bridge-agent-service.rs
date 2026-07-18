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
use std::fs;

#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
use std::sync::{
    mpsc::{self, TryRecvError},
    OnceLock,
};

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
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};

#[cfg(windows)]
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

#[cfg(windows)]
const SERVICE_NAME: &str = "BridgeAgent";

#[cfg(windows)]
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

#[cfg(windows)]
const DESKTOP_PROCESS_NAME: &str = "bridge-agent-desktop.exe";

#[cfg(windows)]
const CONFIG_POLL_INTERVAL: Duration = Duration::from_millis(500);

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
#[cfg(windows)]
define_windows_service!(ffi_service_main, service_main);

#[cfg(windows)]
static SERVICE_CONFIG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

#[cfg(windows)]
fn main() -> Result<()> {
    install_rustls_crypto_provider()?;
    init_tracing();

    let args = ConsoleArgs::parse();
    if args.console {
        return run_console(args.config);
    }

    SERVICE_CONFIG_PATH
        .set(args.config)
        .map_err(|_| anyhow::anyhow!("Windows service config was already initialized"))?;

    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("failed to start Windows service dispatcher")?;
    Ok(())
}

#[cfg(not(windows))]
fn main() -> Result<()> {
    anyhow::bail!("bridge-agent-service only supports Windows")
}

#[cfg(windows)]
fn service_main(_arguments: Vec<OsString>) {
    let config = SERVICE_CONFIG_PATH.get().cloned().flatten();
    if let Err(err) = run_service(config) {
        eprintln!("bridge-agent-service failed: {err:#}");
    }
}

#[cfg(windows)]
fn run_service(config: Option<PathBuf>) -> Result<()> {
    let config_path = resolve_service_config_path(config)?;

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

    status_handle
        .set_service_status(running_status())
        .context("failed to mark service as running")?;

    runtime.block_on(run_runtime_supervisor(&manager, config_path, &shutdown_rx))?;

    status_handle
        .set_service_status(stop_pending_status())
        .context("failed to mark service as stop pending")?;

    runtime
        .block_on(manager.stop())
        .context("failed to stop runtime during service shutdown")?;

    Ok(())
}

#[cfg(windows)]
async fn run_runtime_supervisor(
    manager: &AgentRuntimeManager,
    config_path: &PathBuf,
    shutdown_rx: &mpsc::Receiver<()>,
) -> Result<()> {
    let mut applied_config: Option<Vec<u8>> = None;
    let mut desktop_handoff_active = false;

    loop {
        match shutdown_rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }

        if desktop_process_running() {
            if !desktop_handoff_active {
                manager
                    .stop()
                    .await
                    .context("failed to stop service runtime for desktop handoff")?;
                applied_config = None;
                desktop_handoff_active = true;
                tracing::info!("bridge-agent runtime handed off to the desktop client");
            }
            tokio::time::sleep(CONFIG_POLL_INTERVAL).await;
            continue;
        }
        desktop_handoff_active = false;

        match fs::read(config_path) {
            Ok(contents) if applied_config.as_deref() != Some(contents.as_slice()) => {
                let config = match bridge_agent::load_config(config_path) {
                    Ok(config) => config,
                    Err(err) => {
                        tracing::warn!(
                            "waiting for a valid bridge-agent config at {}: {err:#}",
                            config_path.display()
                        );
                        tokio::time::sleep(CONFIG_POLL_INTERVAL).await;
                        continue;
                    }
                };

                manager
                    .stop()
                    .await
                    .context("failed to stop runtime before applying service config")?;

                if config_is_authorized(&config) {
                    match manager.start(config, config_path).await {
                        Ok(_) => applied_config = Some(contents),
                        Err(err) => {
                            tracing::warn!(
                                "service runtime start deferred for {}: {err:#}",
                                config_path.display()
                            );
                        }
                    }
                } else {
                    applied_config = Some(contents);
                    tracing::info!(
                        "bridge-agent service is waiting for desktop device authorization"
                    );
                }
            }
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(
                    "failed to read bridge-agent config {}: {err}",
                    config_path.display()
                );
            }
        }

        tokio::time::sleep(CONFIG_POLL_INTERVAL).await;
    }

    Ok(())
}

#[cfg(any(windows, test))]
fn config_is_authorized(config: &bridge_agent::AgentConfig) -> bool {
    config.platform.workspace_id.is_some() && !config.relay.token.trim().is_empty()
}

#[cfg(windows)]
fn desktop_process_running() -> bool {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        tracing::warn!(
            "failed to enumerate Windows processes: {}",
            std::io::Error::last_os_error()
        );
        return false;
    }

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut found = false;
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while has_entry {
        let end = entry
            .szExeFile
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(entry.szExeFile.len());
        let process_name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        if process_name.eq_ignore_ascii_case(DESKTOP_PROCESS_NAME) {
            found = true;
            break;
        }
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }

    unsafe {
        CloseHandle(snapshot);
    }
    found
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

#[cfg(test)]
mod tests {
    use super::config_is_authorized;
    use bridge_agent::AgentConfig;

    #[test]
    fn service_waits_until_device_authorization_is_complete() {
        let mut config = AgentConfig::example();
        assert!(!config_is_authorized(&config));

        config.platform.workspace_id = Some(1390);
        assert!(!config_is_authorized(&config));

        config.relay.token = "device-token".to_string();
        assert!(config_is_authorized(&config));
    }
}
