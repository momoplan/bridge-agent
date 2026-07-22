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
use std::ffi::{c_void, OsString};

#[cfg(windows)]
use std::fs;

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[cfg(windows)]
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::ptr::{null, null_mut};

#[cfg(windows)]
use std::sync::OnceLock;

#[cfg(windows)]
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::{env, os::windows::ffi::OsStrExt, slice};

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
use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};

#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
};

#[cfg(windows)]
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

#[cfg(windows)]
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};

#[cfg(windows)]
use windows_sys::Win32::System::RemoteDesktop::{
    WTSActive, WTSEnumerateSessionsW, WTSFreeMemory, WTSQueryUserToken, WTS_CURRENT_SERVER_HANDLE,
    WTS_SESSION_INFOW,
};

#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateProcessAsUserW, OpenProcess, SetEvent, WaitForMultipleObjects,
    CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, PROCESS_SYNCHRONIZE, STARTUPINFOW,
};

#[cfg(windows)]
const SERVICE_NAME: &str = "BridgeAgent";

#[cfg(windows)]
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

#[cfg(windows)]
const DESKTOP_PROCESS_NAME: &str = "bridge-agent-desktop.exe";

#[cfg(windows)]
const FALLBACK_CHECK_INTERVAL: Duration = Duration::from_secs(5);

#[cfg(windows)]
const CONFIG_RETRY_INTERVAL: Duration = Duration::from_secs(1);

#[cfg(windows)]
const DESKTOP_LAUNCH_RETRY_INTERVAL: Duration = Duration::from_secs(30);

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

    let shutdown_event = OwnedHandle(unsafe { CreateEventW(null(), 1, 0, null()) });
    if shutdown_event.0.is_null() {
        return Err(std::io::Error::last_os_error())
            .context("create Windows service shutdown event");
    }
    let shutdown_event_value = shutdown_event.0 as usize;
    let status_handle =
        service_control_handler::register(SERVICE_NAME, move |control| match control {
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if unsafe { SetEvent(shutdown_event_value as HANDLE) } == 0 {
                    tracing::error!(
                        error = %std::io::Error::last_os_error(),
                        "failed to signal Windows service shutdown"
                    );
                }
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        })
        .context("failed to register Windows service control handler")?;

    status_handle
        .set_service_status(start_pending_status(1))
        .context("failed to mark service as start pending")?;

    let result = run_service_loop(&config_path, shutdown_event.0, &status_handle);
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
    config_path: &Path,
    shutdown_event: HANDLE,
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

    runtime.block_on(run_runtime_supervisor(
        &manager,
        config_path,
        shutdown_event,
    ))?;

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
    config_path: &Path,
    shutdown_event: HANDLE,
) -> Result<()> {
    let mut applied_config: Option<ConfigFingerprint> = None;
    let mut desktop_handoff_active = false;
    let mut next_desktop_launch_attempt = Instant::now();
    let mut next_desktop_scan = Instant::now();
    let mut next_config_retry = None;
    let mut next_config_fallback_check = None;
    let mut config_check_requested = true;
    let mut desktop = None;
    let mut config_watch = match ConfigChangeWatch::new(config_path) {
        Ok(watch) => Some(watch),
        Err(err) => {
            tracing::warn!(
                error = %err,
                "config change notifications are unavailable; using a five-second metadata fallback"
            );
            next_config_fallback_check = Some(Instant::now());
            None
        }
    };

    loop {
        let now = Instant::now();
        let desktop_needs_scan = desktop
            .as_ref()
            .is_none_or(|process: &DesktopProcess| process.handle.is_none());
        if desktop_needs_scan && now >= next_desktop_scan {
            let desktop_was_running = desktop.is_some();
            match find_desktop_process() {
                Ok(found) => {
                    desktop = found;
                    if desktop_was_running && desktop.is_none() {
                        config_check_requested = true;
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "failed to inspect Windows desktop process state")
                }
            }
            next_desktop_scan = Instant::now() + FALLBACK_CHECK_INTERVAL;
        }

        if desktop.is_some() {
            if !desktop_handoff_active {
                manager
                    .stop()
                    .await
                    .context("failed to stop service runtime for desktop handoff")?;
                applied_config = None;
                desktop_handoff_active = true;
                tracing::info!("bridge-agent runtime handed off to the desktop client");
            }
        } else {
            desktop_handoff_active = false;

            if now >= next_desktop_launch_attempt {
                match active_user_session_id() {
                    Ok(Some(session_id)) => {
                        manager
                            .stop()
                            .await
                            .context("failed to release service runtime before desktop launch")?;
                        applied_config = None;
                        desktop_handoff_active = true;
                        match launch_desktop_in_session(session_id) {
                            Ok(launched) => {
                                tracing::info!(
                                    session_id = launched.session_id,
                                    "started bridge-agent desktop client in the active Windows session"
                                );
                                desktop = Some(launched.process);
                                next_desktop_launch_attempt =
                                    Instant::now() + DESKTOP_LAUNCH_RETRY_INTERVAL;
                                continue;
                            }
                            Err(err) => {
                                desktop_handoff_active = false;
                                config_check_requested = true;
                                tracing::warn!(
                                    error = %err,
                                    "failed to start bridge-agent desktop client in the active Windows session"
                                );
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            "failed to inspect the active Windows session"
                        );
                    }
                }
                next_desktop_launch_attempt = Instant::now() + DESKTOP_LAUNCH_RETRY_INTERVAL;
            }

            if next_config_retry.is_some_and(|deadline| now >= deadline) {
                config_check_requested = true;
                next_config_retry = None;
            }
            if next_config_fallback_check.is_some_and(|deadline| now >= deadline) {
                config_check_requested = true;
                next_config_fallback_check = Some(now + FALLBACK_CHECK_INTERVAL);
            }

            if config_check_requested {
                config_check_requested = false;
                match config_fingerprint(config_path) {
                    Ok(fingerprint) if applied_config.as_ref() != Some(&fingerprint) => {
                        let config = match bridge_agent::load_config(config_path) {
                            Ok(config) => config,
                            Err(err) => {
                                tracing::warn!(
                                    "waiting for a valid bridge-agent config at {}: {err:#}",
                                    config_path.display()
                                );
                                next_config_retry = Some(Instant::now() + CONFIG_RETRY_INTERVAL);
                                continue;
                            }
                        };
                        match config_fingerprint(config_path) {
                            Ok(verified) if verified == fingerprint => {}
                            Ok(_) => {
                                config_check_requested = true;
                                continue;
                            }
                            Err(err) => {
                                tracing::warn!(
                                    "failed to verify bridge-agent config {} after loading: {err}",
                                    config_path.display()
                                );
                                next_config_retry = Some(Instant::now() + CONFIG_RETRY_INTERVAL);
                                continue;
                            }
                        }

                        manager
                            .stop()
                            .await
                            .context("failed to stop runtime before applying service config")?;

                        if config_is_authorized(&config) {
                            match manager.start(config, config_path).await {
                                Ok(_) => {
                                    applied_config = Some(fingerprint);
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        "service runtime start deferred for {}: {err:#}",
                                        config_path.display()
                                    );
                                    next_config_retry =
                                        Some(Instant::now() + CONFIG_RETRY_INTERVAL);
                                }
                            }
                        } else {
                            applied_config = Some(fingerprint);
                            tracing::info!(
                                "bridge-agent service is waiting for desktop device authorization"
                            );
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(
                            "failed to inspect bridge-agent config {}: {err}",
                            config_path.display()
                        );
                        next_config_retry = Some(Instant::now() + CONFIG_RETRY_INTERVAL);
                    }
                }
            }
        }

        let mut wait_handles = vec![(shutdown_event, SupervisorEvent::Shutdown)];
        if desktop.is_none() {
            if let Some(watch) = config_watch.as_ref() {
                wait_handles.push((watch.0, SupervisorEvent::ConfigChanged));
            }
        } else if let Some(handle) = desktop.as_ref().and_then(|process| process.handle.as_ref()) {
            wait_handles.push((handle.0, SupervisorEvent::DesktopExited));
        }

        let wait_deadline = if desktop.is_some() {
            desktop
                .as_ref()
                .filter(|process| process.handle.is_none())
                .map(|_| next_desktop_scan)
        } else {
            [
                Some(next_desktop_scan),
                Some(next_desktop_launch_attempt),
                next_config_retry,
                next_config_fallback_check,
            ]
            .into_iter()
            .flatten()
            .min()
        };

        match wait_for_supervisor_event(wait_handles, wait_deadline).await? {
            SupervisorEvent::Shutdown => break,
            SupervisorEvent::ConfigChanged => {
                config_check_requested = true;
                if let Some(watch) = config_watch.as_ref() {
                    if let Err(err) = watch.rearm() {
                        tracing::warn!(
                            error = %err,
                            "config change notifications stopped; using a five-second metadata fallback"
                        );
                        config_watch = None;
                        next_config_fallback_check = Some(Instant::now());
                    }
                }
            }
            SupervisorEvent::DesktopExited => {
                desktop = None;
                next_desktop_scan = Instant::now();
                config_check_requested = true;
            }
            SupervisorEvent::Timeout => {}
        }
    }

    Ok(())
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
enum SupervisorEvent {
    Shutdown,
    ConfigChanged,
    DesktopExited,
    Timeout,
}

#[cfg(windows)]
async fn wait_for_supervisor_event(
    handles: Vec<(HANDLE, SupervisorEvent)>,
    deadline: Option<Instant>,
) -> Result<SupervisorEvent> {
    let timeout = deadline
        .map(|deadline| deadline.saturating_duration_since(Instant::now()))
        .map(duration_to_windows_timeout)
        .unwrap_or(u32::MAX);
    let raw_handles = handles
        .iter()
        .map(|(handle, _)| *handle as usize)
        .collect::<Vec<_>>();
    let wait_result = tokio::task::spawn_blocking(move || {
        let raw_handles = raw_handles
            .into_iter()
            .map(|handle| handle as HANDLE)
            .collect::<Vec<_>>();
        let result = unsafe {
            WaitForMultipleObjects(raw_handles.len() as u32, raw_handles.as_ptr(), 0, timeout)
        };
        if result == WAIT_FAILED {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(result)
        }
    })
    .await
    .context("join Windows supervisor wait")?
    .context("wait for Windows supervisor event")?;

    if wait_result == WAIT_TIMEOUT {
        return Ok(SupervisorEvent::Timeout);
    }
    let index = wait_result.saturating_sub(WAIT_OBJECT_0) as usize;
    handles
        .get(index)
        .map(|(_, event)| *event)
        .context("Windows supervisor wait returned an unknown handle")
}

#[cfg(windows)]
fn duration_to_windows_timeout(duration: Duration) -> u32 {
    if duration.is_zero() {
        return 0;
    }
    let milliseconds = duration.as_millis().saturating_add(1);
    milliseconds.min((u32::MAX - 1) as u128) as u32
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct ConfigFingerprint {
    creation_time: u64,
    last_write_time: u64,
    file_size: u64,
}

#[cfg(windows)]
fn config_fingerprint(config_path: &Path) -> std::io::Result<ConfigFingerprint> {
    let metadata = fs::metadata(config_path)?;
    Ok(ConfigFingerprint {
        creation_time: metadata.creation_time(),
        last_write_time: metadata.last_write_time(),
        file_size: metadata.file_size(),
    })
}

#[cfg(windows)]
struct ConfigChangeWatch(HANDLE);

#[cfg(windows)]
impl ConfigChangeWatch {
    fn new(config_path: &Path) -> Result<Self> {
        let watch_dir = config_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or(env::current_dir().context("resolve config watch directory")?);
        let watch_dir = wide_null(watch_dir.as_os_str());
        let handle = unsafe {
            FindFirstChangeNotificationW(
                watch_dir.as_ptr(),
                0,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_LAST_WRITE
                    | FILE_NOTIFY_CHANGE_SIZE,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error()).context("watch config directory");
        }
        Ok(Self(handle))
    }

    fn rearm(&self) -> Result<()> {
        if unsafe { FindNextChangeNotification(self.0) } == 0 {
            return Err(std::io::Error::last_os_error()).context("rearm config directory watch");
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for ConfigChangeWatch {
    fn drop(&mut self) {
        if self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                FindCloseChangeNotification(self.0);
            }
        }
    }
}

#[cfg(windows)]
struct DesktopProcess {
    handle: Option<OwnedHandle>,
}

#[cfg(windows)]
struct LaunchedDesktop {
    session_id: u32,
    process: DesktopProcess,
}

#[cfg(any(windows, test))]
fn config_is_authorized(config: &bridge_agent::AgentConfig) -> bool {
    config.platform.workspace_id.is_some() && !config.relay.token.trim().is_empty()
}

#[cfg(windows)]
fn find_desktop_process() -> Result<Option<DesktopProcess>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error()).context("enumerate Windows processes");
    }
    let snapshot = OwnedHandle(snapshot);

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut has_entry = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
    while has_entry {
        let end = entry
            .szExeFile
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(entry.szExeFile.len());
        let process_name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        if process_name.eq_ignore_ascii_case(DESKTOP_PROCESS_NAME) {
            let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, entry.th32ProcessID) };
            if handle.is_null() {
                tracing::warn!(
                    process_id = entry.th32ProcessID,
                    error = %std::io::Error::last_os_error(),
                    "desktop process is running but cannot be observed by handle"
                );
                return Ok(Some(DesktopProcess { handle: None }));
            }
            return Ok(Some(DesktopProcess {
                handle: Some(OwnedHandle(handle)),
            }));
        }
        has_entry = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
    }
    Ok(None)
}

#[cfg(windows)]
fn launch_desktop_in_session(session_id: u32) -> Result<LaunchedDesktop> {
    let service_executable = env::current_exe().context("resolve Windows service executable")?;
    let install_dir = service_executable
        .parent()
        .context("resolve Windows service install directory")?;
    let desktop_executable = install_dir.join(DESKTOP_PROCESS_NAME);
    if !desktop_executable.is_file() {
        anyhow::bail!(
            "desktop executable does not exist at {}",
            desktop_executable.display()
        );
    }

    let mut user_token: HANDLE = null_mut();
    if unsafe { WTSQueryUserToken(session_id, &mut user_token) } == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("query user token for Windows session {session_id}"));
    }
    let user_token = OwnedHandle(user_token);

    let mut environment: *mut c_void = null_mut();
    if unsafe { CreateEnvironmentBlock(&mut environment, user_token.0, 0) } == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("create user environment for Windows session {session_id}"));
    }
    let environment = OwnedEnvironmentBlock(environment);

    let application = wide_null(desktop_executable.as_os_str());
    let current_directory = wide_null(install_dir.as_os_str());
    let mut desktop_name = wide_null(std::ffi::OsStr::new("winsta0\\default"));
    let startup_info = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        lpDesktop: desktop_name.as_mut_ptr(),
        ..Default::default()
    };
    let mut process_info = PROCESS_INFORMATION::default();

    let created = unsafe {
        CreateProcessAsUserW(
            user_token.0,
            application.as_ptr(),
            null_mut(),
            null(),
            null(),
            0,
            CREATE_UNICODE_ENVIRONMENT,
            environment.0,
            current_directory.as_ptr(),
            &startup_info,
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "start {} in Windows session {session_id}",
                desktop_executable.display()
            )
        });
    }

    unsafe {
        CloseHandle(process_info.hThread);
    }
    Ok(LaunchedDesktop {
        session_id,
        process: DesktopProcess {
            handle: Some(OwnedHandle(process_info.hProcess)),
        },
    })
}

#[cfg(windows)]
fn active_user_session_id() -> Result<Option<u32>> {
    let mut sessions: *mut WTS_SESSION_INFOW = null_mut();
    let mut count = 0u32;
    if unsafe { WTSEnumerateSessionsW(WTS_CURRENT_SERVER_HANDLE, 0, 1, &mut sessions, &mut count) }
        == 0
    {
        return Err(std::io::Error::last_os_error()).context("enumerate active Windows sessions");
    }
    let sessions_guard = OwnedWtsMemory(sessions.cast());
    let result = select_active_user_session_id(
        unsafe { slice::from_raw_parts(sessions, count as usize) }
            .iter()
            .map(|session| (session.SessionId, session.State == WTSActive)),
    );
    drop(sessions_guard);
    Ok(result)
}

#[cfg(any(windows, test))]
fn select_active_user_session_id(sessions: impl IntoIterator<Item = (u32, bool)>) -> Option<u32> {
    sessions
        .into_iter()
        .find(|(session_id, active)| *session_id != 0 && *active)
        .map(|(session_id, _)| session_id)
}

#[cfg(windows)]
fn wide_null(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
struct OwnedHandle(HANDLE);

#[cfg(windows)]
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
struct OwnedEnvironmentBlock(*mut c_void);

#[cfg(windows)]
impl Drop for OwnedEnvironmentBlock {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                DestroyEnvironmentBlock(self.0);
            }
        }
    }
}

#[cfg(windows)]
struct OwnedWtsMemory(*mut c_void);

#[cfg(windows)]
impl Drop for OwnedWtsMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                WTSFreeMemory(self.0);
            }
        }
    }
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
    use super::{config_is_authorized, select_active_user_session_id};
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

    #[test]
    fn desktop_launch_targets_the_first_active_non_system_session() {
        assert_eq!(
            select_active_user_session_id([(0, true), (2, false), (3, true), (4, true)]),
            Some(3)
        );
        assert_eq!(select_active_user_session_id([(0, true), (2, false)]), None);
    }
}
