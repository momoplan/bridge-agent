#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod codex_skill;
mod connector_lifecycle;
mod connector_process;
mod macos_installation;
mod managed_tool;
mod managed_tool_dependency;
mod window_layout;

use connector_lifecycle::{
    ConnectorHealthState, ConnectorLifecycleManager, ConnectorLifecycleSnapshot,
    ConnectorLifecycleState, ConnectorManagementNotReady, ConnectorOperationKind,
};
use connector_process::ConnectorProcessManager;

use anyhow::Context as _;
use axum::{
    body::Body,
    extract::{Path as AxumPath, Query as AxumQuery, State as AxumState},
    http::{header, HeaderMap, Response as HttpResponse, StatusCode},
    response::{IntoResponse, Response as AxumResponse},
    routing::{get, post},
    Json, Router,
};
use bridge_agent::config::resolve_config_base_dir;
use bridge_agent::connector::{
    ConnectorDatabaseContract, ConnectorEventContract, ConnectorMethodContract, ConnectorPermission,
};
use bridge_agent::logging::LogMetadata;
use bridge_agent::protocol::InvokeResult;
use bridge_agent::services::ServiceRegistry;
use bridge_agent::{
    browser_auth_manifest_json, clear_relay_credentials, connector_icon_data_url,
    connector_management_token_path, default_config_path, ensure_browser_auth_agent_id,
    ensure_config_exists, format_connector_sync_failures, inspect_python_runtime,
    install_connector_from_path_with_provenance, install_rustls_crypto_provider,
    is_connector_package_stop_error, list_connectors, load_config as load_agent_config,
    load_connector_manifest, manifest_preview_json, reset_invalid_config,
    resolve_connector_ui_asset, resolve_connector_ui_entry, save_config as save_agent_config,
    show_connector, sync_installed_connector, sync_installed_connectors_report,
    terminate_runtime_lock_owner, uninstall_connector_with_options, AgentConfig,
    AgentRuntimeManager, ConnectorIcon, ConnectorInstallProvenance, ConnectorInstallRecord,
    ConnectorInstallResult, ConnectorStartResult, ConnectorSummary, ConnectorSyncReport,
    ConnectorTrustLevel, ConnectorUninstallOptions, LocalAppConfig, RuntimeEvent,
    RuntimeLockConflict, RuntimeSnapshot, RuntimeStatus, ServiceConfig, ServiceHealthCheck,
    ServiceStartCommand,
};
use reqwest::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_updater::UpdaterExt;
use tauri_plugin_window_state::StateFlags;
use tokio::process::Command as AsyncCommand;
use tokio::time::timeout;
use window_layout::{fit_main_window_to_work_area, WindowLayoutOutcome, WindowLayoutPolicy};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt as _;

#[cfg(target_os = "macos")]
use core_foundation::base::TCFType;
#[cfg(target_os = "macos")]
use core_foundation::boolean::CFBoolean;
#[cfg(target_os = "macos")]
use core_foundation::dictionary::CFDictionary;
#[cfg(target_os = "macos")]
use core_foundation::string::CFString;

const UPDATE_USER_AGENT: &str = concat!("bridge-agent-desktop/", env!("CARGO_PKG_VERSION"));
const CONNECTOR_DOWNLOAD_USER_AGENT: &str = concat!(
    "Baijimu-Connector-Installer/",
    env!("CARGO_PKG_VERSION"),
    " Wget/1.21.4"
);
const UPDATE_PROGRESS_EVENT: &str = "app-update-progress";
const RUNTIME_SNAPSHOT_EVENT: &str = "runtime-snapshot-changed";
const RUNTIME_LOG_EVENT: &str = "runtime-log-appended";
const RUNTIME_LOGS_SNAPSHOT_EVENT: &str = "runtime-logs-snapshot";
const MAIN_WINDOW_VISIBILITY_EVENT: &str = "main-window-visibility-changed";
const STARTUP_HEALTH_EVENT: &str = "startup-health-changed";
const REGISTERED_SERVICES_EVENT: &str = "registered-services-changed";
const LOCAL_APP_RUNTIME_EVENT: &str = "local-app-runtime-changed";
const LOCAL_APPS_CHANGED_EVENT: &str = "local-apps-changed";
const LOCAL_APP_INSTALL_TASK_EVENT: &str = "local-app-install-task-changed";
const HOST_CAPABILITY_CONNECTOR_SETUP_V1: &str = "connector.setup.v1";
const HOST_CAPABILITY_CONNECTOR_PROCESS_HOST_MANAGED_V1: &str = "connector.process.host-managed.v1";
const HOST_CAPABILITY_CONNECTOR_MANAGED_TOOL_DEPENDENCIES_V1: &str =
    "connector.managed-tool-dependencies.v1";
const HOST_CAPABILITY_CONNECTOR_PRESENTATION_ICON_V1: &str = "connector.presentation.icon.v1";
const LOCAL_APP_HOST_CAPABILITIES: &[&str] = &[
    HOST_CAPABILITY_CONNECTOR_SETUP_V1,
    HOST_CAPABILITY_CONNECTOR_PROCESS_HOST_MANAGED_V1,
    HOST_CAPABILITY_CONNECTOR_MANAGED_TOOL_DEPENDENCIES_V1,
    HOST_CAPABILITY_CONNECTOR_PRESENTATION_ICON_V1,
];
const REGISTERED_SERVICES_MONITOR_INTERVAL: Duration = Duration::from_secs(30);
const CONNECTOR_MANIFEST_FILE: &str = "connector.json";
const LOCAL_APP_UI_BRIDGE_ASSET: &str = "__baijimu_bridge.js";
const LOCAL_APP_UI_MAX_MANAGEMENT_PAYLOAD_BYTES: usize = 1024 * 1024;
const LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const HEALTH_ERROR_RESPONSE_MAX_BYTES: usize = 8 * 1024;
const HEALTH_ERROR_MESSAGE_MAX_CHARS: usize = 512;
const LIFECYCLE_OUTPUT_MAX_BYTES: u64 = 1024 * 1024;
const TRAY_ID: &str = "bridge-agent";
const TRAY_MENU_SHOW: &str = "show";
const TRAY_MENU_QUIT: &str = "quit";
const QUIT_RUNNING_INSTANCE_ARG: &str = "--quit-running-instance";
const AUTOSTART_BACKGROUND_ARG: &str = "--background-autostart";
const STARTUP_LOG_FILE_NAME: &str = "bridge-agent-desktop-startup.log";
const STARTUP_STATE_FILE_NAME: &str = "bridge-agent-desktop-startup-state.json";
const INTERACTIVE_RESTART_MARKER_FILE_NAME: &str = "bridge-agent-desktop-interactive-restart";
const LOCAL_APP_CONTROL_FILE_NAME: &str = "local-app-control.json";
const SAFE_MODE_FAILURE_THRESHOLD: u32 = 2;
#[cfg(windows)]
const WINDOWS_CREATE_NO_WINDOW: u32 = 0x08000000;

fn configure_desktop_command(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
    #[cfg(not(windows))]
    let _ = command;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesktopLaunchMode {
    Interactive,
    BackgroundAutostart,
}

impl DesktopLaunchMode {
    fn from_args<I, S>(args: I, interactive_restart_requested: bool) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        if !interactive_restart_requested
            && args
                .into_iter()
                .any(|arg| arg.as_ref() == OsStr::new(AUTOSTART_BACKGROUND_ARG))
        {
            Self::BackgroundAutostart
        } else {
            Self::Interactive
        }
    }

    fn should_show_main_window(self) -> bool {
        matches!(self, Self::Interactive)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainWindowOpenReason {
    InteractiveStartup,
    SecondaryLaunch,
    TrayMenu,
    TrayIcon,
    #[cfg(target_os = "macos")]
    MacosReopen,
}

impl MainWindowOpenReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::InteractiveStartup => "interactive_startup",
            Self::SecondaryLaunch => "secondary_launch",
            Self::TrayMenu => "tray_menu",
            Self::TrayIcon => "tray_icon",
            #[cfg(target_os = "macos")]
            Self::MacosReopen => "macos_reopen",
        }
    }
}

fn desktop_window_state_flags() -> StateFlags {
    StateFlags::SIZE
        | StateFlags::POSITION
        | StateFlags::MAXIMIZED
        | StateFlags::DECORATIONS
        | StateFlags::FULLSCREEN
}

#[cfg(any(target_os = "macos", test))]
fn should_restore_main_window_on_macos_reopen(has_visible_windows: bool) -> bool {
    !has_visible_windows
}

struct DesktopState {
    runtime: AgentRuntimeManager,
    connector_lifecycles: ConnectorLifecycleManager,
    connector_processes: ConnectorProcessManager,
    config_path: PathBuf,
    quitting: Arc<AtomicBool>,
    local_app_ui: Arc<RwLock<Option<LocalAppUiEndpoint>>>,
    local_apps: LocalAppsChangeNotifier,
    local_app_install_tasks: LocalAppInstallTaskManager,
    startup_health: StartupHealthManager,
    registered_services: RegisteredServiceMonitor,
    runtime_log_streaming_requested: Arc<AtomicBool>,
    runtime_log_streaming: Arc<AtomicBool>,
    main_window_visible: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LocalAppInstallTaskPhase {
    Queued,
    Resolving,
    Downloading,
    Verifying,
    Installing,
    Starting,
    Finalizing,
    Succeeded,
    Failed,
}

impl LocalAppInstallTaskPhase {
    fn is_active(self) -> bool {
        !matches!(self, Self::Succeeded | Self::Failed)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalAppInstallTask {
    task_id: String,
    connector_id: Option<String>,
    market_app_id: Option<String>,
    name: String,
    version: Option<String>,
    phase: LocalAppInstallTaskPhase,
    progress_percent: Option<u8>,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
    message: String,
    error: Option<String>,
    created_at_epoch_ms: u64,
    updated_at_epoch_ms: u64,
}

#[derive(Clone, Default)]
struct LocalAppInstallTaskManager {
    tasks: Arc<RwLock<BTreeMap<String, LocalAppInstallTask>>>,
    event_app: Arc<Mutex<Option<tauri::AppHandle>>>,
}

impl LocalAppInstallTaskManager {
    fn attach_event_app(&self, app: tauri::AppHandle) {
        if let Ok(mut current) = self.event_app.lock() {
            *current = Some(app);
        }
    }

    fn create(
        &self,
        connector_id: Option<String>,
        market_app_id: Option<String>,
        name: String,
        version: Option<String>,
    ) -> Result<LocalAppInstallTask, String> {
        let mut tasks = self
            .tasks
            .write()
            .map_err(|_| "本地应用安装任务状态锁已损坏".to_string())?;
        if let Some(existing) = tasks.values().find(|task| {
            task.phase.is_active()
                && ((market_app_id.is_some() && task.market_app_id == market_app_id)
                    || (connector_id.is_some() && task.connector_id == connector_id))
        }) {
            return Err(format!("应用 {} 已在安装中", existing.name));
        }
        let timestamp = now_ms();
        let task = LocalAppInstallTask {
            task_id: uuid::Uuid::new_v4().to_string(),
            connector_id,
            market_app_id,
            name,
            version,
            phase: LocalAppInstallTaskPhase::Queued,
            progress_percent: Some(0),
            downloaded_bytes: None,
            total_bytes: None,
            message: "等待开始安装".to_string(),
            error: None,
            created_at_epoch_ms: timestamp,
            updated_at_epoch_ms: timestamp,
        };
        tasks.insert(task.task_id.clone(), task.clone());
        drop(tasks);
        self.emit(&task);
        Ok(task)
    }

    fn update(
        &self,
        task_id: &str,
        update: impl FnOnce(&mut LocalAppInstallTask),
    ) -> Option<LocalAppInstallTask> {
        let mut tasks = self.tasks.write().ok()?;
        let task = tasks.get_mut(task_id)?;
        update(task);
        task.updated_at_epoch_ms = now_ms();
        let snapshot = task.clone();
        drop(tasks);
        self.emit(&snapshot);
        Some(snapshot)
    }

    fn list(&self) -> Vec<LocalAppInstallTask> {
        self.tasks
            .read()
            .map(|tasks| tasks.values().cloned().collect())
            .unwrap_or_default()
    }

    fn emit(&self, task: &LocalAppInstallTask) {
        let app = self
            .event_app
            .lock()
            .ok()
            .and_then(|current| current.clone());
        if let Some(app) = app {
            if let Err(err) = app.emit(LOCAL_APP_INSTALL_TASK_EVENT, task.clone()) {
                log::warn!(
                    "failed to emit local app install task: task_id={} error={err}",
                    task.task_id
                );
            }
        }
    }
}

#[derive(Clone)]
struct LocalAppInstallProgressReporter {
    manager: LocalAppInstallTaskManager,
    task_id: String,
}

impl LocalAppInstallProgressReporter {
    fn report(
        &self,
        phase: LocalAppInstallTaskPhase,
        progress_percent: Option<u8>,
        message: impl Into<String>,
    ) {
        let message = message.into();
        self.manager.update(&self.task_id, |task| {
            task.phase = phase;
            task.progress_percent = progress_percent;
            task.message = message;
            task.error = None;
            task.downloaded_bytes = None;
            task.total_bytes = None;
        });
    }

    fn download(&self, downloaded_bytes: u64, total_bytes: Option<u64>) {
        self.manager.update(&self.task_id, |task| {
            task.phase = LocalAppInstallTaskPhase::Downloading;
            task.downloaded_bytes = Some(downloaded_bytes);
            task.total_bytes = total_bytes;
            task.progress_percent = total_bytes.filter(|total| *total > 0).map(|total| {
                let download_percent = downloaded_bytes.saturating_mul(100) / total;
                (10 + download_percent.saturating_mul(45) / 100).min(55) as u8
            });
            task.message = match total_bytes {
                Some(total) => format!(
                    "正在下载应用包（{} / {}）",
                    format_byte_count(downloaded_bytes),
                    format_byte_count(total)
                ),
                None => format!("正在下载应用包（{}）", format_byte_count(downloaded_bytes)),
            };
        });
    }

    fn identity(&self, connector_id: &str, name: &str, version: &str) {
        self.manager.update(&self.task_id, |task| {
            task.connector_id = Some(connector_id.to_string());
            task.name = name.to_string();
            task.version = Some(version.to_string());
        });
    }
}

fn format_byte_count(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    const KIB: u64 = 1024;
    if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct LocalAppsChangedEvent {
    revision: u64,
    operation: LocalAppsChangeOperation,
    connector_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LocalAppsChangeOperation {
    Install,
    Sync,
    Uninstall,
}

#[derive(Clone, Default)]
struct LocalAppsChangeNotifier {
    revision: Arc<AtomicU64>,
    event_app: Arc<Mutex<Option<tauri::AppHandle>>>,
}

impl LocalAppsChangeNotifier {
    fn attach_event_app(&self, app: tauri::AppHandle) {
        if let Ok(mut current) = self.event_app.lock() {
            *current = Some(app);
        }
    }

    fn notify(
        &self,
        operation: LocalAppsChangeOperation,
        connector_id: &str,
    ) -> LocalAppsChangedEvent {
        let event = LocalAppsChangedEvent {
            revision: self.revision.fetch_add(1, Ordering::SeqCst) + 1,
            operation,
            connector_id: connector_id.to_string(),
        };
        let app = self
            .event_app
            .lock()
            .ok()
            .and_then(|current| current.clone());
        if let Some(app) = app {
            if let Err(err) = app.emit(LOCAL_APPS_CHANGED_EVENT, event.clone()) {
                log::warn!(
                    "failed to emit local apps changed event: operation={:?} connector_id={} error={err}",
                    event.operation,
                    event.connector_id
                );
            }
        }
        event
    }
}

#[derive(Clone)]
struct RegisteredServiceMonitor {
    request_tx: tokio::sync::mpsc::UnboundedSender<RegisteredServiceMonitorRequest>,
}

impl RegisteredServiceMonitor {
    fn request_refresh(&self) {
        let _ = self
            .request_tx
            .send(RegisteredServiceMonitorRequest::Refresh);
    }

    async fn statuses(&self) -> Result<Vec<RegisteredServiceStatus>, String> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.request_tx
            .send(RegisteredServiceMonitorRequest::RefreshAndRespond(reply_tx))
            .map_err(|_| "本地应用健康监控已停止".to_string())?;
        reply_rx
            .await
            .map_err(|_| "本地应用健康监控未返回结果".to_string())?
    }
}

enum RegisteredServiceMonitorRequest {
    Refresh,
    RefreshAndRespond(tokio::sync::oneshot::Sender<Result<Vec<RegisteredServiceStatus>, String>>),
}

#[derive(Debug, Clone)]
struct LocalAppUiEndpoint {
    port: u16,
    token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupComponentHealth {
    id: String,
    label: String,
    status: String,
    detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupHealthSnapshot {
    revision: u64,
    safe_mode: bool,
    forced_safe_mode: bool,
    consecutive_failures: u32,
    frontend_ready: bool,
    startup_log_path: String,
    components: Vec<StartupComponentHealth>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistentStartupState {
    pending: bool,
    consecutive_failures: u32,
    version: Option<String>,
    started_at_ms: Option<u64>,
    ready_at_ms: Option<u64>,
}

#[derive(Clone)]
struct StartupHealthManager {
    inner: Arc<Mutex<StartupHealthSnapshot>>,
    event_app: Arc<Mutex<Option<tauri::AppHandle>>>,
    state_path: PathBuf,
    diagnostics: StartupDiagnostics,
}

impl StartupHealthManager {
    fn new(config_path: &Path, diagnostics: StartupDiagnostics) -> Self {
        let base_dir = resolve_config_base_dir(config_path);
        Self {
            inner: Arc::new(Mutex::new(StartupHealthSnapshot {
                revision: 0,
                safe_mode: false,
                forced_safe_mode: false,
                consecutive_failures: 0,
                frontend_ready: false,
                startup_log_path: diagnostics.primary_path.display().to_string(),
                components: Vec::new(),
            })),
            event_app: Arc::new(Mutex::new(None)),
            state_path: base_dir.join(STARTUP_STATE_FILE_NAME),
            diagnostics,
        }
    }

    fn begin_primary(&self, forced_safe_mode: bool, bootstrap_failure: Option<String>) {
        let previous = fs::read(&self.state_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<PersistentStartupState>(&bytes).ok())
            .unwrap_or_default();
        let consecutive_failures = if previous.pending {
            previous.consecutive_failures.saturating_add(1)
        } else {
            0
        };
        let safe_mode = forced_safe_mode
            || bootstrap_failure.is_some()
            || consecutive_failures >= SAFE_MODE_FAILURE_THRESHOLD;
        {
            let mut health = match self.inner.lock() {
                Ok(health) => health,
                Err(poisoned) => poisoned.into_inner(),
            };
            *health = StartupHealthSnapshot {
                revision: 0,
                safe_mode,
                forced_safe_mode,
                consecutive_failures,
                frontend_ready: false,
                startup_log_path: self.diagnostics.primary_path.display().to_string(),
                components: Vec::new(),
            };
        }
        self.set_component("desktop_shell", "桌面基础壳", "starting", None);
        if let Some(detail) = bootstrap_failure {
            self.set_component("configuration_path", "配置目录", "degraded", Some(detail));
        } else {
            self.set_component("configuration_path", "配置目录", "ready", None);
        }
        if safe_mode {
            self.diagnostics.warn(format!(
                "safe mode enabled: forced={forced_safe_mode} consecutive_failures={consecutive_failures}"
            ));
        }
        self.write_pending_state();
    }

    fn snapshot(&self) -> StartupHealthSnapshot {
        match self.inner.lock() {
            Ok(health) => health.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn safe_mode(&self) -> bool {
        match self.inner.lock() {
            Ok(health) => health.safe_mode,
            Err(poisoned) => poisoned.into_inner().safe_mode,
        }
    }

    fn attach_event_app(&self, app: tauri::AppHandle) {
        match self.event_app.lock() {
            Ok(mut target) => *target = Some(app),
            Err(poisoned) => *poisoned.into_inner() = Some(app),
        }
        self.emit_snapshot(self.snapshot());
    }

    fn emit_snapshot(&self, snapshot: StartupHealthSnapshot) {
        let app = match self.event_app.lock() {
            Ok(target) => target.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        if let Some(app) = app {
            let _ = app.emit(STARTUP_HEALTH_EVENT, snapshot);
        }
    }

    fn set_component(&self, id: &str, label: &str, status: &str, detail: Option<String>) {
        let snapshot = {
            let mut health = match self.inner.lock() {
                Ok(health) => health,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(component) = health.components.iter_mut().find(|item| item.id == id) {
                component.label = label.to_string();
                component.status = status.to_string();
                component.detail = detail;
            } else {
                health.components.push(StartupComponentHealth {
                    id: id.to_string(),
                    label: label.to_string(),
                    status: status.to_string(),
                    detail,
                });
            }
            health.revision = health.revision.saturating_add(1);
            health.clone()
        };
        self.emit_snapshot(snapshot);
    }

    fn mark_frontend_ready(&self) -> Result<StartupHealthSnapshot, String> {
        {
            let mut health = self
                .inner
                .lock()
                .map_err(|_| "启动状态锁已损坏".to_string())?;
            health.frontend_ready = true;
            if let Some(component) = health
                .components
                .iter_mut()
                .find(|item| item.id == "desktop_shell")
            {
                component.status = "ready".to_string();
                component.detail = None;
            }
            health.revision = health.revision.saturating_add(1);
        }
        let snapshot = self.snapshot();
        self.emit_snapshot(snapshot.clone());
        self.write_ready_state()?;
        self.diagnostics
            .info("frontend readiness handshake completed");
        Ok(snapshot)
    }

    fn reset_for_normal_restart(&self) -> Result<(), String> {
        write_startup_state(
            &self.state_path,
            &PersistentStartupState {
                pending: false,
                consecutive_failures: 0,
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                started_at_ms: None,
                ready_at_ms: Some(now_ms()),
            },
        )
    }

    fn write_pending_state(&self) {
        let snapshot = self.snapshot();
        if let Err(err) = write_startup_state(
            &self.state_path,
            &PersistentStartupState {
                pending: true,
                consecutive_failures: snapshot.consecutive_failures,
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                started_at_ms: Some(now_ms()),
                ready_at_ms: None,
            },
        ) {
            self.diagnostics
                .warn(format!("failed to persist startup pending state: {err}"));
        }
    }

    fn write_ready_state(&self) -> Result<(), String> {
        write_startup_state(
            &self.state_path,
            &PersistentStartupState {
                pending: false,
                consecutive_failures: 0,
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                started_at_ms: None,
                ready_at_ms: Some(now_ms()),
            },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesktopAutostartPolicy {
    SkipDevelopmentBuild,
    EnableForDesktop,
}

fn desktop_autostart_policy(debug_assertions: bool) -> DesktopAutostartPolicy {
    if debug_assertions {
        DesktopAutostartPolicy::SkipDevelopmentBuild
    } else {
        DesktopAutostartPolicy::EnableForDesktop
    }
}

fn configure_desktop_autostart(
    app: &tauri::App,
    startup_health: &StartupHealthManager,
    diagnostics: &StartupDiagnostics,
) {
    match desktop_autostart_policy(cfg!(debug_assertions)) {
        DesktopAutostartPolicy::SkipDevelopmentBuild => startup_health.set_component(
            "autostart",
            "登录启动",
            "skipped",
            Some("开发构建不注册系统登录项".to_string()),
        ),
        DesktopAutostartPolicy::EnableForDesktop => {
            let was_enabled = match app.autolaunch().is_enabled() {
                Ok(enabled) => Some(enabled),
                Err(err) => {
                    diagnostics.warn(format!(
                        "failed to inspect autostart before refreshing registration: {err:#}"
                    ));
                    None
                }
            };
            match app.autolaunch().enable() {
                Ok(()) => {
                    diagnostics.info(format!(
                        "official autostart integration {} with background launch argument",
                        if was_enabled == Some(true) {
                            "refreshed"
                        } else {
                            "enabled"
                        }
                    ));
                    startup_health.set_component("autostart", "登录启动", "ready", None);
                }
                Err(err) => {
                    diagnostics.error(format!("failed to register autostart: {err:#}"));
                    startup_health.set_component(
                        "autostart",
                        "登录启动",
                        "degraded",
                        Some(err.to_string()),
                    );
                }
            }
        }
    }
}

fn write_startup_state(path: &Path, state: &PersistentStartupState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建启动状态目录失败: {err}"))?;
    }
    let temporary_path = path.with_extension("json.tmp");
    let bytes =
        serde_json::to_vec_pretty(state).map_err(|err| format!("序列化启动状态失败: {err}"))?;
    fs::write(&temporary_path, bytes).map_err(|err| format!("写入启动状态失败: {err}"))?;
    match fs::rename(&temporary_path, path) {
        Ok(()) => Ok(()),
        Err(first_err) if path.exists() => {
            fs::remove_file(path)
                .map_err(|err| format!("替换启动状态失败（rename: {first_err}; remove: {err}）"))?;
            fs::rename(&temporary_path, path).map_err(|err| format!("提交启动状态失败: {err}"))
        }
        Err(err) => Err(format!("提交启动状态失败: {err}")),
    }
}

#[derive(Clone)]
struct LocalAppUiHttpState {
    ui_token: String,
    control_token: String,
    diagnostics: StartupDiagnostics,
    config_path: PathBuf,
    runtime: AgentRuntimeManager,
    connector_lifecycles: ConnectorLifecycleManager,
    connector_processes: ConnectorProcessManager,
    registered_services: RegisteredServiceMonitor,
    local_apps: LocalAppsChangeNotifier,
}

#[derive(Clone)]
struct LocalAppUiServerDependencies {
    diagnostics: StartupDiagnostics,
    config_path: PathBuf,
    runtime: AgentRuntimeManager,
    connector_lifecycles: ConnectorLifecycleManager,
    connector_processes: ConnectorProcessManager,
    registered_services: RegisteredServiceMonitor,
    local_apps: LocalAppsChangeNotifier,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalAppControlDiscovery {
    schema_version: u32,
    pid: u32,
    base_url: String,
    token: String,
    started_at_epoch_ms: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalAppControlInstallRequest {
    #[serde(default)]
    source: String,
    #[serde(default)]
    replace: bool,
    checksum: Option<String>,
    allow_git: Option<bool>,
    market_app_id: Option<String>,
    #[serde(default)]
    accept_untrusted: bool,
    #[serde(default)]
    start: bool,
}

#[derive(Clone)]
struct ConnectorInstallOptions {
    source: String,
    replace: bool,
    checksum: Option<String>,
    allow_git: Option<bool>,
    market_app_id: Option<String>,
    start: bool,
    progress: Option<LocalAppInstallProgressReporter>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalAppControlManagementRequest {
    payload: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
struct LocalAppControlUninstallQuery {
    #[serde(default)]
    force: bool,
}

const LOCAL_APP_UI_BRIDGE_SCRIPT: &str = r#"(() => {
  const REQUEST_TYPE = "baijimu:local-app:invoke";
  const RESPONSE_TYPE = "baijimu:local-app:response";
  const READY_TYPE = "baijimu:local-app:ready";
  const HELLO_TYPE = "baijimu:local-app:hello";
  const pending = new Map();
  let sequence = 0;

  const announceReady = () => {
    window.parent.postMessage({ type: READY_TYPE, version: 1 }, "*");
  };

  window.addEventListener("message", (event) => {
    if (event.source !== window.parent) return;
    const message = event.data;
    if (message && message.type === HELLO_TYPE && message.version === 1) {
      announceReady();
      return;
    }
    if (!message || message.type !== RESPONSE_TYPE || message.version !== 1) return;
    const request = pending.get(message.requestId);
    if (!request) return;
    pending.delete(message.requestId);
    clearTimeout(request.timeout);
    if (message.ok) request.resolve(message.data);
    else request.reject(new Error(message.error || "本地应用管理操作失败"));
  });

  const api = Object.freeze({
    version: 1,
    invoke(operation, payload = null) {
      if (typeof operation !== "string" || !/^[A-Za-z0-9._-]{1,128}$/.test(operation)) {
        return Promise.reject(new Error("management operation 名称无效"));
      }
      const requestId = `${Date.now().toString(36)}-${(++sequence).toString(36)}`;
      return new Promise((resolve, reject) => {
        const timeout = window.setTimeout(() => {
          pending.delete(requestId);
          reject(new Error("本地应用管理操作超时"));
        }, 65000);
        pending.set(requestId, { resolve, reject, timeout });
        window.parent.postMessage({
          type: REQUEST_TYPE,
          version: 1,
          requestId,
          operation,
          payload
        }, "*");
      });
    }
  });

  Object.defineProperty(window, "baijimuLocalApp", {
    value: api,
    configurable: false,
    enumerable: true,
    writable: false
  });
  announceReady();
  window.addEventListener("pageshow", announceReady);
})();
"#;

#[derive(Debug, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
enum CommandError {
    RuntimeAlreadyRunning { conflict: RuntimeLockConflict },
    Message { message: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
enum ConnectorUninstallCommandError {
    #[serde(rename = "connector_uninstall_stop_failed")]
    StopFailed { message: String },
    #[serde(rename = "connector_uninstall_failed")]
    Failed { message: String },
}

impl ConnectorUninstallCommandError {
    fn message(&self) -> &str {
        match self {
            Self::StopFailed { message } | Self::Failed { message } => message,
        }
    }
}

impl From<anyhow::Error> for CommandError {
    fn from(err: anyhow::Error) -> Self {
        if let Some(conflict) = err.downcast_ref::<RuntimeLockConflict>() {
            return Self::RuntimeAlreadyRunning {
                conflict: conflict.clone(),
            };
        }
        Self::Message {
            message: err.to_string(),
        }
    }
}

fn command_error_message(message: impl Into<String>) -> CommandError {
    CommandError::Message {
        message: message.into(),
    }
}

#[derive(Serialize)]
struct ConfigDocument {
    config_path: String,
    manifest_preview: String,
    config: Value,
    runtime: RuntimeSnapshot,
}

#[derive(Serialize)]
struct ConfigRecoveryDocument {
    config_path: String,
    archived_path: Option<String>,
    manifest_preview: String,
    config: Value,
    runtime: RuntimeSnapshot,
}

#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserAuthStartResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: i32,
    interval: i32,
}

#[derive(Debug, Serialize)]
struct BrowserAuthPollResponse {
    status: String,
    message: String,
    config: Option<Value>,
    runtime: Option<RuntimeSnapshot>,
}

fn config_for_ui(config: &AgentConfig) -> Result<Value, String> {
    let mut value = serde_json::to_value(config).map_err(|err| err.to_string())?;
    value["relay"]["token"] = Value::String(String::new());
    value["credential_status"] = serde_json::json!({
        "relay_token_configured": !config.relay.token.trim().is_empty()
    });
    Ok(value)
}

#[derive(Debug, serde::Deserialize)]
struct RawBrowserAuthPollResponse {
    status: String,
    message: String,
    #[serde(rename = "authorizedPayload")]
    authorized_payload: Option<AuthorizedPayload>,
}

#[derive(Debug, serde::Deserialize)]
struct AuthorizedPayload {
    #[serde(rename = "workspaceId")]
    workspace_id: u64,
    #[serde(rename = "deviceId")]
    device_id: String,
    #[serde(rename = "relayWsUrl")]
    relay_ws_url: String,
    #[serde(rename = "agentToken")]
    agent_token: String,
    #[serde(rename = "issuedAtEpochSeconds")]
    issued_at_epoch_seconds: Option<u64>,
    #[serde(rename = "expiresAtEpochSeconds")]
    expires_at_epoch_seconds: Option<u64>,
    #[serde(rename = "localClientToken")]
    local_client_token: Option<String>,
    #[serde(rename = "localClientTokenType")]
    local_client_token_type: Option<String>,
    #[serde(rename = "localClientKeyId")]
    local_client_key_id: Option<String>,
    #[serde(rename = "localClientUserId")]
    local_client_user_id: Option<u64>,
    #[serde(default, rename = "localClientScopes")]
    local_client_scopes: Vec<String>,
    #[serde(rename = "localClientIssuedAt")]
    local_client_issued_at: Option<String>,
    #[serde(rename = "localClientExpiresAt")]
    local_client_expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppVersionInfo {
    current_version: String,
    current_target: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdateStatus {
    current_version: String,
    latest_version: Option<String>,
    update_available: bool,
    force_update_required: bool,
    minimum_supported_version: Option<String>,
    force_update_message: Option<String>,
    release_url: Option<String>,
    release_name: Option<String>,
    published_at: Option<String>,
    current_target: String,
    auto_download_available: bool,
    asset_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdateInstallResult {
    status: String,
    version: String,
    asset_name: Option<String>,
    downloaded_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdateProgress {
    phase: String,
    message: String,
    version: Option<String>,
    asset_name: Option<String>,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
    downloaded_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopPermissionStatus {
    platform: String,
    accessibility_granted: bool,
    screen_recording_granted: bool,
    accessibility_supported: bool,
    screen_recording_supported: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RegisteredServiceState {
    NotConfigured,
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisteredServiceStatus {
    service: String,
    status: RegisteredServiceState,
    detail: Option<String>,
    checked_at_ms: u64,
    health_check_configured: bool,
    start_command_configured: bool,
    stop_command_configured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalAppRuntimeStatus {
    connector_id: String,
    status: RegisteredServiceState,
    detail: Option<String>,
    checked_at_ms: u64,
    health_check_configured: bool,
    start_command_configured: bool,
    stop_command_configured: bool,
    process_managed: bool,
    process_running: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartRegisteredServiceResult {
    service: String,
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorAppUpdateStatus {
    connector_id: String,
    name: String,
    current_version: String,
    latest_version: String,
    update_available: bool,
    source: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MarketConnectorApp {
    id: String,
    connector_id: String,
    application_type: String,
    name: String,
    description: String,
    source: String,
    checksum: Option<String>,
    archive_path: Option<String>,
    risk: String,
    risk_level: String,
    capability: String,
    version: String,
    published_at: Option<String>,
    icon_data_url: Option<String>,
    release_notes: Vec<String>,
    configuration_declaration: String,
    interface_declaration: String,
    database_declaration: String,
    config_schema: Option<Value>,
    database: Option<ConnectorDatabaseContract>,
    methods: Vec<ConnectorMethodContract>,
    events: Vec<ConnectorEventContract>,
    method_names: Vec<String>,
    event_names: Vec<String>,
    permissions: Vec<ConnectorPermission>,
    compatible: bool,
    compatibility_message: Option<String>,
    minimum_host_version: Option<String>,
    required_host_capabilities: Vec<String>,
    missing_host_capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLocalAppMarketResponse<T> {
    error_code: Option<String>,
    value: Option<String>,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMarketConnectorApp {
    id: String,
    connector_id: String,
    name: String,
    description: String,
    risk: String,
    risk_level: Option<String>,
    capability: String,
    latest_version: RawMarketConnectorVersion,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMarketConnectorVersion {
    version: String,
    source: String,
    source_type: Option<String>,
    revision: Option<String>,
    checksum: Option<String>,
    published_at: Option<String>,
    #[serde(default)]
    manifest: Value,
    #[serde(default)]
    compatibility: Option<RawMarketHostCompatibility>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMarketHostCompatibility {
    compatible: bool,
    message: Option<String>,
    minimum_host_version: Option<String>,
    #[serde(default)]
    required_capabilities: Vec<String>,
    #[serde(default)]
    missing_capabilities: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorAppInstallDocument {
    install: ConnectorInstallResult,
    start: Option<ConnectorStartResult>,
    setup: Option<Value>,
    config: ConfigDocument,
}

#[tauri::command]
async fn baijimu_cli_status() -> Result<managed_tool::ManagedToolStatus, String> {
    let source = bundled_baijimu_cli_path();
    managed_tool::inspect(source.as_deref()).map_err(|err| err.to_string())
}

#[tauri::command]
async fn install_baijimu_cli_update(
    version: String,
    source: String,
    checksum: String,
    archive_path: Option<String>,
) -> Result<managed_tool::ManagedToolStatus, String> {
    managed_tool::install_update(&source, &version, &checksum, archive_path.as_deref())
        .await
        .map_err(|err| err.to_string())?;
    codex_skill::install_bundled().map_err(|err| err.to_string())?;
    let bundled = bundled_baijimu_cli_path();
    managed_tool::inspect(bundled.as_deref()).map_err(|err| err.to_string())
}

#[tauri::command]
async fn rollback_baijimu_cli() -> Result<managed_tool::ManagedToolStatus, String> {
    managed_tool::rollback().map_err(|err| err.to_string())?;
    codex_skill::install_bundled().map_err(|err| err.to_string())?;
    let bundled = bundled_baijimu_cli_path();
    managed_tool::inspect(bundled.as_deref()).map_err(|err| err.to_string())
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateReleaseResponse {
    #[serde(default, alias = "tag_name")]
    tag_name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default, alias = "html_url")]
    release_url: Option<String>,
    #[serde(default, alias = "name")]
    release_name: Option<String>,
    #[serde(default, alias = "published_at")]
    published_at: Option<String>,
    #[serde(default, alias = "update_available")]
    update_available: Option<bool>,
    #[serde(default, alias = "force_update")]
    force_update: Option<bool>,
    #[serde(
        default,
        alias = "minimum_supported_version",
        alias = "minSupportedVersion"
    )]
    minimum_supported_version: Option<String>,
    #[serde(default, alias = "force_update_message")]
    force_update_message: Option<String>,
    #[serde(default)]
    assets: Vec<UpdateReleaseAsset>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateReleaseAsset {
    name: String,
    #[serde(default)]
    signature: Option<String>,
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[tauri::command]
async fn load_config(state: tauri::State<'_, DesktopState>) -> Result<ConfigDocument, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    let runtime = state.runtime.snapshot().await;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config: config_for_ui(&config)?,
        runtime,
    })
}

#[tauri::command]
async fn python_runtime_status(
    state: tauri::State<'_, DesktopState>,
    python_path: Option<String>,
) -> Result<bridge_agent::PythonRuntimeStatus, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let mut config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    if let Some(path) = python_path {
        config.runtime.python_path = if path.trim().is_empty() {
            None
        } else {
            Some(path)
        };
    }
    Ok(inspect_python_runtime(&config.runtime))
}

#[tauri::command]
async fn save_config(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
) -> Result<ConfigDocument, String> {
    save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    let runtime = state.runtime.snapshot().await;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config: config_for_ui(&config)?,
        runtime,
    })
}

#[tauri::command]
async fn save_service(
    state: tauri::State<'_, DesktopState>,
    service_index: usize,
    service: ServiceConfig,
    apply_to_runtime: bool,
) -> Result<ConfigDocument, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let mut config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    if service_index > config.services.len() {
        return Err(format!("服务索引 {service_index} 已超出当前配置范围"));
    }
    if service_index == config.services.len() {
        config.services.push(service);
    } else {
        config.services[service_index] = service;
    }
    save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    let runtime = if apply_to_runtime {
        state
            .runtime
            .apply_capabilities_from_path(&state.config_path)
            .await
            .map_err(|err| err.to_string())?
    } else {
        state.runtime.snapshot().await
    };
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config: config_for_ui(&config)?,
        runtime,
    })
}

#[tauri::command]
async fn delete_service(
    state: tauri::State<'_, DesktopState>,
    service_index: usize,
    apply_to_runtime: bool,
) -> Result<ConfigDocument, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let mut config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    if service_index >= config.services.len() {
        return Err(format!("服务索引 {service_index} 已超出当前配置范围"));
    }
    config.services.remove(service_index);
    save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    let runtime = if apply_to_runtime {
        state
            .runtime
            .apply_capabilities_from_path(&state.config_path)
            .await
            .map_err(|err| err.to_string())?
    } else {
        state.runtime.snapshot().await
    };
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config: config_for_ui(&config)?,
        runtime,
    })
}

#[tauri::command]
async fn start_agent(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
) -> Result<RuntimeSnapshot, CommandError> {
    state
        .startup_health
        .diagnostics
        .info("manual agent start requested from desktop UI");
    save_agent_config(&state.config_path, &config).map_err(|err| CommandError::Message {
        message: err.to_string(),
    })?;
    state
        .runtime
        .start_from_path(&state.config_path)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
async fn stop_agent(state: tauri::State<'_, DesktopState>) -> Result<RuntimeSnapshot, String> {
    state.runtime.stop().await.map_err(|err| err.to_string())
}

#[tauri::command]
async fn stop_conflicting_runtime(
    lock_path: String,
    pid: u32,
    agent_id: String,
    config_path: String,
) -> Result<(), CommandError> {
    terminate_runtime_lock_owner(Path::new(&lock_path), pid, &agent_id, &config_path)
        .map_err(CommandError::from)
}

#[tauri::command]
async fn runtime_snapshot(
    state: tauri::State<'_, DesktopState>,
) -> Result<RuntimeSnapshot, String> {
    Ok(state.runtime.snapshot().await)
}

#[tauri::command]
async fn apply_saved_config_to_runtime(
    state: tauri::State<'_, DesktopState>,
) -> Result<RuntimeSnapshot, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    state
        .runtime
        .apply_capabilities_from_path(&state.config_path)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn test_capability(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
    service: String,
    method: String,
    arguments: Value,
    timeout_secs: Option<u64>,
) -> Result<InvokeResult, String> {
    let service = service.trim();
    let method = method.trim();
    if service.is_empty() {
        return Err("服务名不能为空".to_string());
    }
    if method.is_empty() {
        return Err("能力名不能为空".to_string());
    }

    let config_base_dir = resolve_config_base_dir(&state.config_path);
    let registry = ServiceRegistry::from_config_checked(&config, &config_base_dir)
        .await
        .map_err(|err| format!("构建本地能力运行环境失败: {err}"))?;
    let request_id = format!("desktop-test-{}", now_ms());

    Ok(registry
        .invoke(
            request_id,
            service,
            method,
            arguments,
            timeout_secs.filter(|value| *value > 0),
        )
        .await)
}

#[tauri::command]
async fn test_local_app_capability(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
    connector_id: String,
    method: String,
    arguments: Value,
    timeout_secs: Option<u64>,
) -> Result<InvokeResult, String> {
    let connector_id = connector_id.trim();
    let method = method.trim();
    if connector_id.is_empty() {
        return Err("connectorId 不能为空".to_string());
    }
    if method.is_empty() {
        return Err("能力名不能为空".to_string());
    }

    let config_base_dir = resolve_config_base_dir(&state.config_path);
    let registry = ServiceRegistry::from_config_checked(&config, &config_base_dir)
        .await
        .map_err(|err| format!("构建本地应用运行环境失败: {err}"))?;
    let request_id = format!("desktop-local-app-test-{}", now_ms());

    Ok(registry
        .invoke_local_app(
            request_id,
            None,
            connector_id,
            method,
            arguments,
            timeout_secs.filter(|value| *value > 0),
        )
        .await)
}

#[tauri::command]
async fn list_logs(
    state: tauri::State<'_, DesktopState>,
    limit: Option<usize>,
) -> Result<Vec<bridge_agent::LogEntry>, String> {
    Ok(state.runtime.logs(limit.unwrap_or(200)).await)
}

#[tauri::command]
fn set_runtime_log_streaming(state: tauri::State<'_, DesktopState>, enabled: bool) {
    state
        .runtime_log_streaming_requested
        .store(enabled, Ordering::SeqCst);
    let enabled = enabled && state.main_window_visible.load(Ordering::SeqCst);
    state.runtime_log_streaming.store(enabled, Ordering::SeqCst);
}

#[tauri::command]
async fn clear_logs(state: tauri::State<'_, DesktopState>) -> Result<u64, String> {
    Ok(state.runtime.clear_logs().await)
}

#[tauri::command]
async fn reset_example_config(
    state: tauri::State<'_, DesktopState>,
) -> Result<ConfigDocument, String> {
    clear_relay_credentials(&state.config_path).map_err(|err| err.to_string())?;
    let config = AgentConfig::example();
    save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    let runtime = state.runtime.snapshot().await;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config: config_for_ui(&config)?,
        runtime,
    })
}

#[tauri::command]
async fn recover_invalid_config(
    state: tauri::State<'_, DesktopState>,
) -> Result<ConfigRecoveryDocument, String> {
    let recovery = reset_invalid_config(&state.config_path).map_err(|err| err.to_string())?;
    let manifest_preview =
        manifest_preview_json(&recovery.config).map_err(|err| err.to_string())?;
    let runtime = state.runtime.snapshot().await;
    Ok(ConfigRecoveryDocument {
        config_path: state.config_path.display().to_string(),
        archived_path: recovery
            .archived_path
            .map(|path| path.display().to_string()),
        manifest_preview,
        config: config_for_ui(&recovery.config)?,
        runtime,
    })
}

#[tauri::command]
fn open_in_browser(url: String) -> Result<(), String> {
    open::that(url).map_err(|err| err.to_string())
}

fn describe_upstream_http_failure(
    status: reqwest::StatusCode,
    content_type: &str,
    body: &str,
) -> String {
    let trimmed = body.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(message) = value.get("value").and_then(Value::as_str) {
            return format!("HTTP {status}: {message}");
        }
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            return format!("HTTP {status}: {message}");
        }
    }

    let lower_content_type = content_type.to_ascii_lowercase();
    let lower_body_start = trimmed
        .chars()
        .take(80)
        .collect::<String>()
        .to_ascii_lowercase();
    if lower_content_type.contains("text/html")
        || lower_body_start.starts_with("<!doctype html")
        || lower_body_start.starts_with("<html")
    {
        return format!(
            "HTTP {status}: 平台授权接口返回了 HTML 错误页，可能是网关路由、服务异常或请求体超过平台限制。请确认 Base URL 为 https://api.baijimu.com/lowcode3，并检查平台授权服务日志。"
        );
    }

    if lower_content_type.contains("xml") || lower_body_start.starts_with("<?xml") {
        if let (Some(code), Some(message)) = (
            extract_xml_tag(trimmed, "Code"),
            extract_xml_tag(trimmed, "Message"),
        ) {
            return format!("HTTP {status}: {code} - {message}");
        }
        return format!(
            "HTTP {status}: 平台授权接口返回了 XML 错误，请检查 Baijimu Base URL 和网关路由。"
        );
    }

    if trimmed.is_empty() {
        return format!("HTTP {status}: 空响应");
    }
    format!("HTTP {status}: {}", truncate_for_error(trimmed, 240))
}

fn extract_xml_tag(body: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    let after_start = body.split_once(&start)?.1;
    let value = after_start.split_once(&end)?.0.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn truncate_for_error(value: &str, limit: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= limit {
        return compact;
    }
    let prefix = compact.chars().take(limit).collect::<String>();
    format!("{prefix}...")
}

#[tauri::command]
fn open_in_edge(url: String) -> Result<(), String> {
    open_url_in_edge(&url)
}

#[cfg(windows)]
fn open_url_in_edge(url: &str) -> Result<(), String> {
    let mut candidates = Vec::new();
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        candidates
            .push(PathBuf::from(program_files).join("Microsoft\\Edge\\Application\\msedge.exe"));
    }
    if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
        candidates.push(
            PathBuf::from(program_files_x86).join("Microsoft\\Edge\\Application\\msedge.exe"),
        );
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        candidates
            .push(PathBuf::from(local_app_data).join("Microsoft\\Edge\\Application\\msedge.exe"));
    }

    for candidate in candidates {
        if candidate.is_file() {
            let mut edge = Command::new(candidate);
            configure_desktop_command(&mut edge);
            edge.arg(url)
                .spawn()
                .map_err(|err| format!("打开 Microsoft Edge 失败: {err}"))?;
            return Ok(());
        }
    }

    let mut edge = Command::new("msedge");
    configure_desktop_command(&mut edge);
    edge.arg(url)
        .spawn()
        .map_err(|err| format!("未找到 Microsoft Edge，请复制授权链接后手动粘贴到浏览器: {err}"))?;
    Ok(())
}

#[cfg(not(windows))]
fn open_url_in_edge(url: &str) -> Result<(), String> {
    open::that(url).map_err(|err| err.to_string())
}

#[tauri::command]
fn desktop_permission_status() -> Result<DesktopPermissionStatus, String> {
    Ok(read_desktop_permission_status())
}

#[tauri::command]
async fn registered_service_statuses(
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<RegisteredServiceStatus>, String> {
    state.registered_services.statuses().await
}

#[tauri::command]
async fn local_app_runtime_statuses(
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<LocalAppRuntimeStatus>, String> {
    collect_local_app_runtime_statuses(
        &state.config_path,
        &state.connector_lifecycles,
        &state.connector_processes,
    )
    .await
}

#[tauri::command]
fn connector_lifecycle_snapshots(
    state: tauri::State<'_, DesktopState>,
) -> Vec<ConnectorLifecycleSnapshot> {
    state.connector_lifecycles.list()
}

async fn collect_local_app_runtime_statuses(
    config_path: &Path,
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
) -> Result<Vec<LocalAppRuntimeStatus>, String> {
    ensure_config_exists(config_path).map_err(|err| err.to_string())?;
    let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|err| err.to_string())?;
    let mut statuses = Vec::with_capacity(config.local_apps.len());
    for app in config.local_apps {
        let process_running = connector_processes.managed_running(&app.connector_id).await;
        let runtime_active = connector_processes.runtime_active(&app.connector_id).await;
        let connector_id = app.connector_id.clone();
        let status = if runtime_active {
            check_local_app(&client, app, process_running).await
        } else {
            inactive_local_app_status(app, process_running)
        };
        let version = show_connector(&connector_id)
            .ok()
            .map(|record| record.manifest.version);
        let pid = connector_processes.managed_pid(&connector_id).await;
        let (lifecycle, health) = match (runtime_active, status.status) {
            (false, _) => (
                ConnectorLifecycleState::Stopped,
                ConnectorHealthState::Unhealthy,
            ),
            (true, RegisteredServiceState::Healthy) => (
                ConnectorLifecycleState::Ready,
                ConnectorHealthState::Healthy,
            ),
            (true, RegisteredServiceState::Unhealthy) => (
                ConnectorLifecycleState::Degraded,
                ConnectorHealthState::Unhealthy,
            ),
            (true, RegisteredServiceState::NotConfigured) => (
                ConnectorLifecycleState::Ready,
                ConnectorHealthState::NotConfigured,
            ),
            (true, RegisteredServiceState::Unknown) => (
                ConnectorLifecycleState::Recovering,
                ConnectorHealthState::Unknown,
            ),
        };
        connector_lifecycles.observe(
            &connector_id,
            lifecycle,
            health,
            version,
            pid,
            status.detail.clone(),
        )?;
        statuses.push(status);
    }
    Ok(statuses)
}

async fn collect_registered_service_statuses(
    config_path: &Path,
) -> Result<Vec<RegisteredServiceStatus>, String> {
    ensure_config_exists(config_path).map_err(|err| err.to_string())?;
    let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|err| err.to_string())?;
    let mut statuses = Vec::new();

    for service in config.services {
        if service.health_check.is_none() && service.start_command.is_none() {
            continue;
        }
        statuses.push(check_registered_service(&client, service).await);
    }

    Ok(statuses)
}

fn registered_service_statuses_changed(
    previous: &[RegisteredServiceStatus],
    current: &[RegisteredServiceStatus],
) -> bool {
    previous.len() != current.len()
        || previous.iter().zip(current).any(|(left, right)| {
            left.service != right.service
                || left.status != right.status
                || left.detail != right.detail
                || left.health_check_configured != right.health_check_configured
                || left.start_command_configured != right.start_command_configured
                || left.stop_command_configured != right.stop_command_configured
        })
}

fn local_app_runtime_statuses_changed(
    previous: &[LocalAppRuntimeStatus],
    current: &[LocalAppRuntimeStatus],
) -> bool {
    previous.len() != current.len()
        || previous.iter().zip(current).any(|(left, right)| {
            left.connector_id != right.connector_id
                || left.status != right.status
                || left.detail != right.detail
                || left.health_check_configured != right.health_check_configured
                || left.start_command_configured != right.start_command_configured
                || left.stop_command_configured != right.stop_command_configured
                || left.process_managed != right.process_managed
                || left.process_running != right.process_running
        })
}

fn start_registered_service_monitor(
    app: tauri::AppHandle,
    config_path: PathBuf,
    connector_lifecycles: ConnectorLifecycleManager,
    connector_processes: ConnectorProcessManager,
    mut request_rx: tokio::sync::mpsc::UnboundedReceiver<RegisteredServiceMonitorRequest>,
) {
    tauri::async_runtime::spawn(async move {
        let mut previous: Option<Vec<RegisteredServiceStatus>> = None;
        let mut previous_local_apps: Option<Vec<LocalAppRuntimeStatus>> = None;
        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + REGISTERED_SERVICES_MONITOR_INTERVAL,
            REGISTERED_SERVICES_MONITOR_INTERVAL,
        );
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let mut responders = Vec::new();
            tokio::select! {
                _ = interval.tick() => {}
                _ = connector_processes.changed() => {}
                request = request_rx.recv() => {
                    match request {
                        Some(RegisteredServiceMonitorRequest::Refresh) => {}
                        Some(RegisteredServiceMonitorRequest::RefreshAndRespond(reply)) => {
                            responders.push(reply);
                        }
                        None => break,
                    }
                }
            }

            while let Ok(request) = request_rx.try_recv() {
                if let RegisteredServiceMonitorRequest::RefreshAndRespond(reply) = request {
                    responders.push(reply);
                }
            }

            let result = collect_registered_service_statuses(&config_path).await;
            match result.as_ref() {
                Ok(current) => {
                    let changed = previous
                        .as_deref()
                        .is_none_or(|last| registered_service_statuses_changed(last, current));
                    if changed {
                        log::debug!("registered service status changed");
                    }
                    if responders.is_empty() {
                        let _ = app.emit(REGISTERED_SERVICES_EVENT, current.clone());
                    }
                    previous = Some(current.clone());
                }
                Err(err) => {
                    log::warn!("failed to refresh registered service statuses: {err}");
                }
            }
            for responder in responders {
                let _ = responder.send(result.clone());
            }

            match collect_local_app_runtime_statuses(
                &config_path,
                &connector_lifecycles,
                &connector_processes,
            )
            .await
            {
                Ok(current) => {
                    let changed = previous_local_apps
                        .as_deref()
                        .is_none_or(|last| local_app_runtime_statuses_changed(last, &current));
                    if changed {
                        log::debug!("local app runtime status changed");
                        let _ = app.emit(LOCAL_APP_RUNTIME_EVENT, current.clone());
                    }
                    previous_local_apps = Some(current);
                }
                Err(err) => {
                    log::warn!("failed to refresh local app runtime statuses: {err}");
                }
            }
        }
    });
}

#[tauri::command]
async fn start_registered_service(
    state: tauri::State<'_, DesktopState>,
    service: String,
) -> Result<StartRegisteredServiceResult, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let requested_service = service.trim();
    if requested_service.is_empty() {
        return Err("服务名不能为空".to_string());
    }
    let service_config = config
        .services
        .into_iter()
        .find(|candidate| candidate.name == requested_service)
        .ok_or_else(|| format!("服务 `{requested_service}` 未注册"))?;
    let Some(start_command) = service_config.start_command else {
        return Err(format!("服务 `{requested_service}` 没有注册启动命令"));
    };
    let result = run_start_command(service_config.name, start_command).await;
    state.registered_services.request_refresh();
    result
}

#[tauri::command]
async fn stop_registered_service(
    state: tauri::State<'_, DesktopState>,
    service: String,
) -> Result<StartRegisteredServiceResult, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let requested_service = service.trim();
    if requested_service.is_empty() {
        return Err("服务名不能为空".to_string());
    }
    let service_config = config
        .services
        .into_iter()
        .find(|candidate| candidate.name == requested_service)
        .ok_or_else(|| format!("服务 `{requested_service}` 未注册"))?;
    let Some(stop_command) = service_config.stop_command else {
        return Err(format!("服务 `{requested_service}` 没有注册停止命令"));
    };
    let result = run_start_command(service_config.name, stop_command).await;
    state.registered_services.request_refresh();
    result
}

fn start_local_app_ui_server(
    endpoint: Arc<RwLock<Option<LocalAppUiEndpoint>>>,
    startup_health: StartupHealthManager,
    dependencies: LocalAppUiServerDependencies,
) {
    let LocalAppUiServerDependencies {
        diagnostics,
        config_path,
        runtime,
        connector_lifecycles,
        connector_processes,
        registered_services,
        local_apps,
    } = dependencies;
    startup_health.set_component("local_app_ui_server", "本地应用界面服务", "starting", None);
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await
        {
            Ok(listener) => listener,
            Err(err) => {
                let detail = format!("无法监听本机端口: {err}");
                diagnostics.error(format!("failed to start local app UI server: {detail}"));
                startup_health.set_component(
                    "local_app_ui_server",
                    "本地应用界面服务",
                    "degraded",
                    Some(detail),
                );
                return;
            }
        };
        let port = match listener.local_addr() {
            Ok(address) => address.port(),
            Err(err) => {
                let detail = format!("无法读取监听地址: {err}");
                diagnostics.error(format!("failed to start local app UI server: {detail}"));
                startup_health.set_component(
                    "local_app_ui_server",
                    "本地应用界面服务",
                    "degraded",
                    Some(detail),
                );
                return;
            }
        };
        let ui_token = uuid::Uuid::new_v4().simple().to_string();
        let control_token = uuid::Uuid::new_v4().simple().to_string();
        match endpoint.write() {
            Ok(mut value) => {
                *value = Some(LocalAppUiEndpoint {
                    port,
                    token: ui_token.clone(),
                });
            }
            Err(_) => {
                let detail = "本地应用界面状态锁已损坏".to_string();
                diagnostics.error(&detail);
                startup_health.set_component(
                    "local_app_ui_server",
                    "本地应用界面服务",
                    "degraded",
                    Some(detail),
                );
                return;
            }
        }
        startup_health.set_component(
            "local_app_ui_server",
            "本地应用界面服务",
            "ready",
            Some(format!("127.0.0.1:{port}")),
        );
        diagnostics.info(format!("local app UI server listening on 127.0.0.1:{port}"));
        let state = LocalAppUiHttpState {
            ui_token,
            control_token: control_token.clone(),
            diagnostics: diagnostics.clone(),
            config_path: config_path.clone(),
            runtime,
            connector_lifecycles,
            connector_processes,
            registered_services,
            local_apps,
        };
        let control_path = local_app_control_discovery_path(&config_path);
        if let Err(err) = write_local_app_control_discovery(&control_path, port, &control_token) {
            diagnostics.error(format!(
                "failed to publish local app control endpoint: {err}"
            ));
            startup_health.set_component(
                "local_app_ui_server",
                "本地应用界面服务",
                "degraded",
                Some(err),
            );
            return;
        }
        let router = Router::new()
            .route("/api/v1/status", get(local_app_control_status_handler))
            .route(
                "/api/v1/local-app-market",
                get(local_app_control_market_handler),
            )
            .route("/api/v1/local-apps", get(local_app_control_list_handler))
            .route(
                "/api/v1/local-apps/install",
                post(local_app_control_install_handler),
            )
            .route(
                "/api/v1/local-apps/{connector_id}",
                get(local_app_control_show_handler).delete(local_app_control_uninstall_handler),
            )
            .route(
                "/api/v1/local-apps/{connector_id}/start",
                post(local_app_control_start_handler),
            )
            .route(
                "/api/v1/local-apps/{connector_id}/stop",
                post(local_app_control_stop_handler),
            )
            .route(
                "/api/v1/local-apps/{connector_id}/sync",
                post(local_app_control_sync_handler),
            )
            .route(
                "/api/v1/local-apps/{connector_id}/management/{operation}",
                post(local_app_control_management_handler),
            )
            .route("/{token}/{connector_id}/", get(local_app_ui_entry_handler))
            .route(
                "/{token}/{connector_id}/{*asset_path}",
                get(local_app_ui_asset_handler),
            )
            .with_state(state);
        let serve_result = axum::serve(listener, router).await;
        let _ = fs::remove_file(&control_path);
        if let Err(err) = serve_result {
            diagnostics.error(format!("local app UI server stopped: {err:#}"));
            if let Ok(mut value) = endpoint.write() {
                *value = None;
            }
            startup_health.set_component(
                "local_app_ui_server",
                "本地应用界面服务",
                "degraded",
                Some(format!("服务已停止: {err}")),
            );
        }
    });
}

fn local_app_control_discovery_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LOCAL_APP_CONTROL_FILE_NAME)
}

fn write_local_app_control_discovery(path: &Path, port: u16, token: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "无法确定本机应用控制文件目录".to_string())?;
    fs::create_dir_all(parent).map_err(|err| format!("创建本机应用控制目录失败: {err}"))?;
    let document = LocalAppControlDiscovery {
        schema_version: 1,
        pid: std::process::id(),
        base_url: format!("http://127.0.0.1:{port}/api/v1"),
        token: token.to_string(),
        started_at_epoch_ms: now_ms(),
    };
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|err| format!("序列化本机应用控制地址失败: {err}"))?;
    let temporary_path = path.with_extension("json.tmp");
    fs::write(&temporary_path, bytes).map_err(|err| format!("写入本机应用控制地址失败: {err}"))?;
    #[cfg(unix)]
    fs::set_permissions(&temporary_path, fs::Permissions::from_mode(0o600))
        .map_err(|err| format!("保护本机应用控制地址失败: {err}"))?;
    if path.exists() {
        fs::remove_file(path).map_err(|err| format!("替换本机应用控制地址失败: {err}"))?;
    }
    fs::rename(&temporary_path, path).map_err(|err| format!("提交本机应用控制地址失败: {err}"))?;
    Ok(())
}

fn local_app_control_is_authorized(state: &LocalAppUiHttpState, headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .is_some_and(|value| value == state.control_token)
}

fn local_app_control_error(status: StatusCode, message: impl Into<String>) -> AxumResponse {
    (
        status,
        Json(serde_json::json!({
            "ok": false,
            "error": { "message": message.into() }
        })),
    )
        .into_response()
}

fn local_app_control_success<T: Serialize>(value: T) -> AxumResponse {
    Json(serde_json::json!({ "ok": true, "data": value })).into_response()
}

fn local_app_control_result<T: Serialize>(result: Result<T, String>) -> AxumResponse {
    match result {
        Ok(value) => local_app_control_success(value),
        Err(err) => local_app_control_error(StatusCode::BAD_REQUEST, err),
    }
}

async fn local_app_control_status_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = async {
        let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
        Ok::<_, String>(serde_json::json!({
            "schemaVersion": 1,
            "pid": std::process::id(),
            "version": env!("CARGO_PKG_VERSION"),
            "configPath": state.config_path.display().to_string(),
            "authorized": config_is_authorized(&config),
            "workspaceId": config.platform.workspace_id,
            "relayTokenConfigured": !config.relay.token.trim().is_empty(),
            "runtime": state.runtime.snapshot().await
        }))
    }
    .await;
    local_app_control_result(result)
}

async fn local_app_control_market_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    local_app_control_result(fetch_market_connector_apps(&state.config_path).await)
}

async fn local_app_control_list_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = async {
        let apps = list_connectors().map_err(|err| err.to_string())?;
        let services = state.registered_services.statuses().await?;
        Ok::<_, String>(serde_json::json!({
            "apps": apps,
            "syncFailures": [],
            "services": services,
            "lifecycles": state.connector_lifecycles.list(),
            "runtime": state.runtime.snapshot().await
        }))
    }
    .await;
    local_app_control_result(result)
}

async fn local_app_control_show_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath(connector_id): AxumPath<String>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = async {
        let record = show_connector(connector_id.trim()).map_err(|err| err.to_string())?;
        let process_running = state
            .connector_processes
            .managed_running(&record.manifest.id)
            .await;
        let status =
            connector_local_app_status(&state.config_path, &record.manifest.id, process_running)
                .await?;
        Ok::<_, String>(serde_json::json!({
            "app": record,
            "status": status,
            "lifecycle": state.connector_lifecycles.list().into_iter()
                .find(|snapshot| snapshot.connector_id == connector_id.trim()),
            "runtime": state.runtime.snapshot().await
        }))
    }
    .await;
    local_app_control_result(result)
}

async fn local_app_control_install_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    headers: HeaderMap,
    Json(request): Json<LocalAppControlInstallRequest>,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let market_install = request
        .market_app_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    if !market_install && !request.accept_untrusted {
        return local_app_control_error(
            StatusCode::FORBIDDEN,
            "本地目录、Git 或直接下载来源未经平台验证；检查来源后传 acceptUntrusted=true",
        );
    }
    let result = async {
        let document = install_connector_app_with_context(
            &state.config_path,
            &state.runtime,
            &state.connector_lifecycles,
            &state.connector_processes,
            &state.registered_services,
            ConnectorInstallOptions {
                source: request.source,
                replace: request.replace,
                checksum: request.checksum,
                allow_git: request.allow_git,
                market_app_id: request.market_app_id,
                start: request.start,
                progress: None,
            },
        )
        .await?;
        state.local_apps.notify(
            LocalAppsChangeOperation::Install,
            &document.install.connector_id,
        );
        Ok::<_, String>(document)
    }
    .await;
    local_app_control_result(result)
}

async fn local_app_control_start_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath(connector_id): AxumPath<String>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = start_connector_with_lifecycle(
        &state.connector_lifecycles,
        &state.connector_processes,
        &state.config_path,
        connector_id.trim(),
        "启动应用",
    )
    .await;
    state.registered_services.request_refresh();
    local_app_control_result(result)
}

async fn local_app_control_stop_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath(connector_id): AxumPath<String>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = stop_connector_with_lifecycle(
        &state.connector_lifecycles,
        &state.connector_processes,
        &state.config_path,
        connector_id.trim(),
        "停止应用",
    )
    .await;
    state.registered_services.request_refresh();
    local_app_control_result(result)
}

async fn local_app_control_sync_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath(connector_id): AxumPath<String>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = async {
        let record = show_connector(connector_id.trim()).map_err(|err| err.to_string())?;
        let source = record
            .source_reference
            .clone()
            .unwrap_or_else(|| record.source_path.clone());
        let document = install_connector_app_with_context(
            &state.config_path,
            &state.runtime,
            &state.connector_lifecycles,
            &state.connector_processes,
            &state.registered_services,
            ConnectorInstallOptions {
                source,
                replace: true,
                checksum: record.source_checksum,
                allow_git: Some(true),
                market_app_id: record.market_app_id,
                start: true,
                progress: None,
            },
        )
        .await?;
        state.local_apps.notify(
            LocalAppsChangeOperation::Sync,
            &document.install.connector_id,
        );
        Ok::<_, String>(document)
    }
    .await;
    local_app_control_result(result)
}

async fn local_app_control_management_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath((connector_id, operation)): AxumPath<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<LocalAppControlManagementRequest>,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    match invoke_connector_management_with_context(
        &state.connector_lifecycles,
        &state.connector_processes,
        connector_id,
        operation,
        request.payload,
    )
    .await
    {
        Ok(value) => local_app_control_success(value),
        Err(error) => {
            let status = if error.code == "connector_not_ready" {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            (
                status,
                Json(serde_json::json!({ "ok": false, "error": error })),
            )
                .into_response()
        }
    }
}

async fn local_app_control_uninstall_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath(connector_id): AxumPath<String>,
    AxumQuery(query): AxumQuery<LocalAppControlUninstallQuery>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = async {
        let connector_id = connector_id.trim().to_string();
        let document = uninstall_connector_app_with_context(
            &state.config_path,
            &state.runtime,
            &state.connector_lifecycles,
            &state.connector_processes,
            &state.registered_services,
            connector_id.clone(),
            query.force,
        )
        .await
        .map_err(|error| error.message().to_string())?;
        state
            .local_apps
            .notify(LocalAppsChangeOperation::Uninstall, &connector_id);
        Ok::<_, String>(document)
    }
    .await;
    local_app_control_result(result)
}

async fn local_app_ui_entry_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath((token, connector_id)): AxumPath<(String, String)>,
    headers: HeaderMap,
) -> AxumResponse {
    serve_local_app_ui_asset(&state, &token, &connector_id, None, &headers).await
}

async fn local_app_ui_asset_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath((token, connector_id, asset_path)): AxumPath<(String, String, String)>,
    headers: HeaderMap,
) -> AxumResponse {
    serve_local_app_ui_asset(&state, &token, &connector_id, Some(&asset_path), &headers).await
}

async fn serve_local_app_ui_asset(
    state: &LocalAppUiHttpState,
    token: &str,
    connector_id: &str,
    asset_path: Option<&str>,
    headers: &HeaderMap,
) -> AxumResponse {
    let asset_kind = match asset_path {
        None => "entry",
        Some(LOCAL_APP_UI_BRIDGE_ASSET) => "bridge",
        Some(_) => "asset",
    };
    if token != state.ui_token || !local_app_ui_request_host_matches(headers, token, connector_id) {
        state.diagnostics.warn(format!(
            "local app UI request: connector_id={connector_id} asset_kind={asset_kind} outcome=rejected reason=invalid_endpoint"
        ));
        return local_app_ui_error(StatusCode::NOT_FOUND, "not found");
    }
    if asset_path == Some(LOCAL_APP_UI_BRIDGE_ASSET) {
        state.diagnostics.info(format!(
            "local app UI request: connector_id={connector_id} asset_kind=bridge outcome=served status=200"
        ));
        return local_app_ui_response(
            StatusCode::OK,
            "application/javascript; charset=utf-8",
            LOCAL_APP_UI_BRIDGE_SCRIPT.as_bytes().to_vec(),
        );
    }

    let record = match show_connector(connector_id) {
        Ok(record) => record,
        Err(_) => {
            state.diagnostics.warn(format!(
                "local app UI request: connector_id={connector_id} asset_kind={asset_kind} outcome=rejected reason=application_not_found"
            ));
            return local_app_ui_error(StatusCode::NOT_FOUND, "application not found");
        }
    };
    let Some(ui) = record.manifest.ui.as_ref() else {
        state.diagnostics.warn(format!(
            "local app UI request: connector_id={connector_id} asset_kind={asset_kind} outcome=rejected reason=ui_not_declared"
        ));
        return local_app_ui_error(StatusCode::NOT_FOUND, "application UI not found");
    };
    let package_path = Path::new(&record.package_path);
    let resolved = match resolve_connector_ui_asset(package_path, ui, asset_path) {
        Ok(path) => path,
        Err(_) => {
            state.diagnostics.warn(format!(
                "local app UI request: connector_id={connector_id} asset_kind={asset_kind} outcome=rejected reason=asset_not_found"
            ));
            return local_app_ui_error(StatusCode::NOT_FOUND, "asset not found");
        }
    };
    let mut body = match tokio::fs::read(&resolved).await {
        Ok(body) => body,
        Err(_) => {
            state.diagnostics.warn(format!(
                "local app UI request: connector_id={connector_id} asset_kind={asset_kind} outcome=rejected reason=asset_read_failed"
            ));
            return local_app_ui_error(StatusCode::NOT_FOUND, "asset not found");
        }
    };
    if asset_path.is_none() {
        body = match inject_local_app_ui_bridge(body) {
            Ok(body) => body,
            Err(message) => {
                state.diagnostics.warn(format!(
                    "local app UI request: connector_id={connector_id} asset_kind=entry outcome=rejected reason=bridge_injection_failed"
                ));
                return local_app_ui_error(StatusCode::UNPROCESSABLE_ENTITY, &message);
            }
        };
    }
    state.diagnostics.info(format!(
        "local app UI request: connector_id={connector_id} asset_kind={asset_kind} outcome=served status=200"
    ));
    local_app_ui_response(StatusCode::OK, local_app_ui_content_type(&resolved), body)
}

fn local_app_ui_host(token: &str, connector_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.update([0]);
    hasher.update(connector_id.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("app-{}.localhost", &digest[..20])
}

fn local_app_ui_request_host_matches(headers: &HeaderMap, token: &str, connector_id: &str) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let host_without_port = host.split_once(':').map_or(host, |(host, _)| host);
    host_without_port.eq_ignore_ascii_case(&local_app_ui_host(token, connector_id))
}

fn inject_local_app_ui_bridge(body: Vec<u8>) -> Result<Vec<u8>, String> {
    let html = String::from_utf8(body)
        .map_err(|_| "application UI entry must be UTF-8 HTML".to_string())?;
    let script = format!(r#"<script src="./{LOCAL_APP_UI_BRIDGE_ASSET}"></script>"#);
    let lower = html.to_ascii_lowercase();
    let injected = if let Some(index) = lower.find("</head>") {
        format!("{}{}{}", &html[..index], script, &html[index..])
    } else {
        format!("{script}{html}")
    };
    Ok(injected.into_bytes())
}

fn local_app_ui_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn local_app_ui_error(status: StatusCode, message: &str) -> AxumResponse {
    local_app_ui_response(
        status,
        "text/plain; charset=utf-8",
        message.as_bytes().to_vec(),
    )
}

fn local_app_ui_response(
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
) -> AxumResponse {
    HttpResponse::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'none'; object-src 'none'; base-uri 'self'; form-action 'none'; frame-src 'none'; frame-ancestors tauri://localhost http://tauri.localhost http://localhost:1420",
        )
        .body(Body::from(body))
        .unwrap_or_else(|_| HttpResponse::new(Body::empty()))
}

#[tauri::command]
fn connector_app_ui_url(
    id: String,
    state: tauri::State<'_, DesktopState>,
) -> Result<String, String> {
    let id = id.trim();
    let record = show_connector(id).map_err(|err| err.to_string())?;
    let ui = record
        .manifest
        .ui
        .as_ref()
        .ok_or_else(|| format!("应用 {} 没有声明内嵌界面", record.manifest.name))?;
    resolve_connector_ui_entry(Path::new(&record.package_path), ui)
        .map_err(|err| err.to_string())?;
    let endpoint = state
        .local_app_ui
        .read()
        .map_err(|_| "本地应用界面状态锁已损坏".to_string())?
        .clone()
        .ok_or_else(|| "本地应用界面服务当前不可用，请在诊断页查看启动状态".to_string())?;
    Ok(format!(
        "http://{}:{}/{}/{}/",
        local_app_ui_host(&endpoint.token, &record.manifest.id),
        endpoint.port,
        endpoint.token,
        record.manifest.id
    ))
}

#[tauri::command]
async fn list_connector_apps(
    _state: tauri::State<'_, DesktopState>,
) -> Result<Vec<ConnectorSummary>, String> {
    list_connectors().map_err(|err| err.to_string())
}

#[tauri::command]
async fn list_market_connector_apps(
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<MarketConnectorApp>, String> {
    fetch_market_connector_apps(&state.config_path).await
}

async fn fetch_market_connector_apps(
    config_path: &Path,
) -> Result<Vec<MarketConnectorApp>, String> {
    let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
    let base_url = config.platform.base_url.trim_end_matches('/');
    let platform = normalized_platform();
    let arch = std::env::consts::ARCH;
    let mut url = reqwest::Url::parse(&format!("{base_url}/api/local-app-market/apps"))
        .map_err(|err| format!("localApp 市场地址无效: {err}"))?;
    url.query_pairs_mut()
        .append_pair("platform", platform)
        .append_pair("arch", arch)
        .append_pair("hostVersion", env!("CARGO_PKG_VERSION"))
        .append_pair("hostCapabilities", &LOCAL_APP_HOST_CAPABILITIES.join(","));
    let response = Client::new()
        .get(url)
        .send()
        .await
        .map_err(|err| format!("请求 localApp 市场失败: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("请求 localApp 市场失败: HTTP {status} {body}"));
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|err| format!("解析 localApp 市场响应失败: {err}"))?;
    let raw_apps: Vec<RawMarketConnectorApp> = if payload.get("data").is_some() {
        let wrapped: RawLocalAppMarketResponse<Vec<RawMarketConnectorApp>> =
            serde_json::from_value(payload)
                .map_err(|err| format!("解析 lowcode localApp 市场响应失败: {err}"))?;
        if wrapped
            .error_code
            .as_deref()
            .is_some_and(|code| code != "0")
        {
            return Err(format!(
                "lowcode localApp 市场返回失败: {}",
                wrapped.value.unwrap_or_else(|| "未知错误".to_string())
            ));
        }
        wrapped.data.unwrap_or_default()
    } else {
        serde_json::from_value(payload)
            .map_err(|err| format!("解析 local-app-market 响应失败: {err}"))?
    };
    Ok(raw_apps.into_iter().map(MarketConnectorApp::from).collect())
}

#[tauri::command]
async fn show_connector_app(id: String) -> Result<ConnectorInstallRecord, String> {
    show_connector(id.trim()).map_err(|err| err.to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorManagementCommandError {
    code: &'static str,
    message: String,
    lifecycle: Option<ConnectorLifecycleSnapshot>,
}

impl ConnectorManagementCommandError {
    fn message(message: impl Into<String>) -> Self {
        Self {
            code: "connector_management_failed",
            message: message.into(),
            lifecycle: None,
        }
    }
}

impl From<Box<ConnectorManagementNotReady>> for ConnectorManagementCommandError {
    fn from(error: Box<ConnectorManagementNotReady>) -> Self {
        Self {
            code: error.code,
            message: error.message,
            lifecycle: Some(error.lifecycle),
        }
    }
}

#[tauri::command]
async fn invoke_connector_management(
    state: tauri::State<'_, DesktopState>,
    id: String,
    operation: String,
    payload: Option<Value>,
) -> Result<Value, ConnectorManagementCommandError> {
    invoke_connector_management_with_context(
        &state.connector_lifecycles,
        &state.connector_processes,
        id,
        operation,
        payload,
    )
    .await
}

async fn invoke_connector_management_with_context(
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    id: String,
    operation: String,
    payload: Option<Value>,
) -> Result<Value, ConnectorManagementCommandError> {
    let management_permit = connector_lifecycles
        .try_management_permit(id.trim())
        .map_err(ConnectorManagementCommandError::from)?;
    if connector_processes.managed_running(id.trim()).await == Some(false) {
        connector_lifecycles
            .observe(
                id.trim(),
                ConnectorLifecycleState::Stopped,
                ConnectorHealthState::Unhealthy,
                management_permit.lifecycle.observed_version.clone(),
                None,
                Some("宿主管理进程已退出".to_string()),
            )
            .map_err(ConnectorManagementCommandError::message)?;
        return Err(ConnectorManagementCommandError::from(
            connector_lifecycles
                .try_management_permit(id.trim())
                .err()
                .expect("stopped connector must reject management requests"),
        ));
    }
    let result = invoke_connector_management_request(id, operation, payload)
        .await
        .map_err(ConnectorManagementCommandError::message);
    drop(management_permit);
    result
}

async fn invoke_connector_management_request(
    id: String,
    operation: String,
    payload: Option<Value>,
) -> Result<Value, String> {
    let id = id.trim();
    let operation = operation.trim();
    let record = show_connector(id).map_err(|err| err.to_string())?;
    let management = record
        .manifest
        .management
        .as_ref()
        .ok_or_else(|| format!("应用 {} 没有声明本机管理接口", record.manifest.name))?;
    let operation_config = management
        .operations
        .get(operation)
        .ok_or_else(|| format!("应用 {} 没有声明管理操作 {operation}", record.manifest.name))?;
    if let Some(payload) = payload.as_ref() {
        let payload_size = serde_json::to_vec(payload)
            .map_err(|err| format!("序列化应用管理参数失败: {err}"))?
            .len();
        if payload_size > LOCAL_APP_UI_MAX_MANAGEMENT_PAYLOAD_BYTES {
            return Err(format!(
                "应用管理参数超过 {} 字节限制",
                LOCAL_APP_UI_MAX_MANAGEMENT_PAYLOAD_BYTES
            ));
        }
    }
    let management_url = reqwest::Url::parse(&management.base_url)
        .map_err(|err| format!("应用本机管理地址无效: {err}"))?;
    let management_host = management_url.host_str().unwrap_or_default();
    let management_host_is_loopback = management_host.eq_ignore_ascii_case("localhost")
        || management_host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if management_url.scheme() != "http" || !management_host_is_loopback {
        return Err("应用本机管理地址必须是 loopback HTTP".to_string());
    }
    if !management_url.username().is_empty()
        || management_url.password().is_some()
        || management_url.query().is_some()
        || management_url.fragment().is_some()
        || management_url.path() != "/"
    {
        return Err("应用本机管理地址必须是只包含 origin 的 URL".to_string());
    }
    if !matches!(operation_config.method.as_str(), "GET" | "POST")
        || !operation_config.path.starts_with("/management/")
        || operation_config.path.contains('?')
        || operation_config.path.contains('#')
    {
        return Err(format!("应用管理操作 {operation} 的声明不安全"));
    }
    let token_path = connector_management_token_path(id).map_err(|err| err.to_string())?;
    let token = fs::read_to_string(&token_path)
        .map_err(|err| format!("读取应用本机管理凭证失败 {}: {err}", token_path.display()))?;
    let token = token.trim();
    if token.len() < 32 {
        return Err(format!("应用本机管理凭证无效: {}", token_path.display()));
    }
    #[cfg(unix)]
    {
        let metadata = fs::metadata(&token_path).map_err(|err| err.to_string())?;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "应用本机管理凭证权限不安全: {}",
                token_path.display()
            ));
        }
    }

    let base = management.base_url.trim_end_matches('/');
    let url = format!("{base}{}", operation_config.path);
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|err| format!("创建本机应用管理请求失败: {err}"))?;
    let request = match operation_config.method.as_str() {
        "GET" => client.get(&url),
        "POST" => client
            .post(&url)
            .json(&payload.unwrap_or_else(|| serde_json::json!({}))),
        method => return Err(format!("不支持的本机应用管理方法: {method}")),
    };
    let response = request
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| format!("调用本机应用管理接口失败: {err}"))?;
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|size| size > LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES as u64)
    {
        return Err(format!(
            "本机应用管理响应超过 {} 字节限制",
            LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES
        ));
    }
    let body = response
        .bytes()
        .await
        .map_err(|err| format!("读取本机应用管理响应失败: {err}"))?;
    if body.len() > LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES {
        return Err(format!(
            "本机应用管理响应超过 {} 字节限制",
            LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES
        ));
    }
    let document: Value = serde_json::from_slice(&body)
        .map_err(|err| format!("本机应用管理接口返回了无效 JSON: {err}"))?;
    if !status.is_success() || document.get("ok").and_then(Value::as_bool) != Some(true) {
        let message = document
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("本机应用管理操作失败");
        return Err(format!("{message}（HTTP {status}）"));
    }
    Ok(document.get("data").cloned().unwrap_or(Value::Null))
}

#[tauri::command]
async fn check_connector_app_update(
    state: tauri::State<'_, DesktopState>,
    id: String,
    market_app_id: String,
) -> Result<ConnectorAppUpdateStatus, String> {
    let connector_id = id.trim();
    if connector_id.is_empty() {
        return Err("应用 ID 不能为空".to_string());
    }
    let installed = show_connector(connector_id).map_err(|err| err.to_string())?;
    if installed.trust_level != ConnectorTrustLevel::PlatformTrusted {
        return Err("用户信任的应用不能静默切换到市场更新源，请从市场重新安装".to_string());
    }
    if installed.market_app_id.as_deref() != Some(market_app_id.trim()) {
        return Err("已安装应用的市场身份与更新来源不匹配".to_string());
    }
    let market_app = fetch_market_connector_apps(&state.config_path)
        .await?
        .into_iter()
        .find(|app| app.id == market_app_id.trim())
        .ok_or_else(|| "市场中找不到该应用".to_string())?;
    validate_market_host_compatibility(&market_app)?;
    validate_market_connector_identity(&market_app, connector_id)?;
    let checksum = required_market_checksum(&market_app)?;
    let resolved_source =
        resolve_connector_source(&market_app.source, false, Some(&checksum), None).await?;
    let latest_manifest =
        load_connector_manifest(resolved_source.path()).map_err(|err| err.to_string())?;
    if latest_manifest.id != installed.manifest.id {
        return Err(format!(
            "更新来源应用 ID 不匹配：当前 `{}`，来源 `{}`",
            installed.manifest.id, latest_manifest.id
        ));
    }
    if latest_manifest.version != market_app.version {
        return Err(format!(
            "市场版本与安装包清单不匹配：市场 `{}`，安装包 `{}`",
            market_app.version, latest_manifest.version
        ));
    }

    Ok(ConnectorAppUpdateStatus {
        connector_id: installed.manifest.id,
        name: latest_manifest.name,
        current_version: installed.manifest.version.clone(),
        latest_version: latest_manifest.version.clone(),
        update_available: connector_version_is_newer(
            &latest_manifest.version,
            &installed.manifest.version,
        ),
        source: market_app.source,
    })
}

#[tauri::command]
async fn install_connector_app(
    state: tauri::State<'_, DesktopState>,
    source: String,
    replace: bool,
    checksum: Option<String>,
    allow_git: Option<bool>,
    market_app_id: Option<String>,
) -> Result<ConnectorAppInstallDocument, String> {
    let document = install_connector_app_with_context(
        &state.config_path,
        &state.runtime,
        &state.connector_lifecycles,
        &state.connector_processes,
        &state.registered_services,
        ConnectorInstallOptions {
            source,
            replace,
            checksum,
            allow_git,
            market_app_id,
            start: true,
            progress: None,
        },
    )
    .await?;
    state.local_apps.notify(
        LocalAppsChangeOperation::Install,
        &document.install.connector_id,
    );
    Ok(document)
}

#[tauri::command]
fn start_connector_app_install(
    state: tauri::State<'_, DesktopState>,
    request: StartConnectorAppInstallRequest,
) -> Result<LocalAppInstallTask, String> {
    let StartConnectorAppInstallRequest {
        source,
        replace,
        checksum,
        allow_git,
        market_app_id,
        connector_id,
        name,
        version,
    } = request;
    let market_app_id = market_app_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let connector_id = connector_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let display_name = name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "自定义应用".to_string());
    let display_version = version
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let task = state.local_app_install_tasks.create(
        connector_id,
        market_app_id.clone(),
        display_name,
        display_version,
    )?;
    let manager = state.local_app_install_tasks.clone();
    let reporter = LocalAppInstallProgressReporter {
        manager: manager.clone(),
        task_id: task.task_id.clone(),
    };
    let task_id = task.task_id.clone();
    let config_path = state.config_path.clone();
    let runtime = state.runtime.clone();
    let connector_lifecycles = state.connector_lifecycles.clone();
    let connector_processes = state.connector_processes.clone();
    let registered_services = state.registered_services.clone();
    let local_apps = state.local_apps.clone();
    tauri::async_runtime::spawn(async move {
        let result = install_connector_app_with_context(
            &config_path,
            &runtime,
            &connector_lifecycles,
            &connector_processes,
            &registered_services,
            ConnectorInstallOptions {
                source,
                replace,
                checksum,
                allow_git,
                market_app_id,
                start: true,
                progress: Some(reporter),
            },
        )
        .await;
        match result {
            Ok(document) => {
                manager.update(&task_id, |task| {
                    task.connector_id = Some(document.install.connector_id.clone());
                    task.name = document.install.name.clone();
                    task.version = Some(document.install.version.clone());
                    task.phase = LocalAppInstallTaskPhase::Succeeded;
                    task.progress_percent = Some(100);
                    task.downloaded_bytes = None;
                    task.total_bytes = None;
                    task.message = "应用已安装，可进入应用完成初始化".to_string();
                    task.error = None;
                });
                local_apps.notify(
                    LocalAppsChangeOperation::Install,
                    &document.install.connector_id,
                );
            }
            Err(error) => {
                manager.update(&task_id, |task| {
                    task.phase = LocalAppInstallTaskPhase::Failed;
                    task.progress_percent = None;
                    task.downloaded_bytes = None;
                    task.total_bytes = None;
                    task.message = "应用安装失败".to_string();
                    task.error = Some(error);
                });
            }
        }
    });
    Ok(task)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartConnectorAppInstallRequest {
    source: String,
    replace: bool,
    checksum: Option<String>,
    allow_git: Option<bool>,
    market_app_id: Option<String>,
    connector_id: Option<String>,
    name: Option<String>,
    version: Option<String>,
}

#[tauri::command]
fn list_connector_app_install_tasks(
    state: tauri::State<'_, DesktopState>,
) -> Vec<LocalAppInstallTask> {
    state.local_app_install_tasks.list()
}

async fn install_connector_app_with_context(
    config_path: &Path,
    runtime_manager: &AgentRuntimeManager,
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    registered_services: &RegisteredServiceMonitor,
    options: ConnectorInstallOptions,
) -> Result<ConnectorAppInstallDocument, String> {
    let progress = options.progress.clone();
    if let Some(progress) = progress.as_ref() {
        progress.report(
            LocalAppInstallTaskPhase::Resolving,
            Some(5),
            "正在解析安装来源",
        );
    }
    ensure_config_exists(config_path).map_err(|err| err.to_string())?;
    let requested_source = options.source.trim();
    if requested_source.is_empty() && options.market_app_id.as_deref().is_none_or(str::is_empty) {
        return Err("安装来源不能为空".to_string());
    }

    let market_app = match options
        .market_app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(id) => Some(
            fetch_market_connector_apps(config_path)
                .await?
                .into_iter()
                .find(|app| app.id == id)
                .ok_or_else(|| "市场中找不到该应用".to_string())?,
        ),
        None => None,
    };
    let (resolved_source_text, resolved_checksum, resolved_allow_git) =
        if let Some(market_app) = market_app.as_ref() {
            validate_market_host_compatibility(market_app)?;
            if market_app.application_type != "connector" {
                return Err("该市场条目不是 Connector 应用".to_string());
            }
            (
                market_app.source.clone(),
                Some(required_market_checksum(market_app)?),
                false,
            )
        } else {
            (
                requested_source.to_string(),
                options.checksum.filter(|value| !value.trim().is_empty()),
                options.allow_git.unwrap_or(true),
            )
        };
    let resolved_source = resolve_connector_source(
        &resolved_source_text,
        resolved_allow_git,
        resolved_checksum.as_deref(),
        progress.as_ref(),
    )
    .await?;
    if let Some(progress) = progress.as_ref() {
        progress.report(
            LocalAppInstallTaskPhase::Verifying,
            Some(60),
            "正在校验应用清单与平台身份",
        );
    }
    let candidate_manifest =
        load_connector_manifest(resolved_source.path()).map_err(|err| err.to_string())?;
    if let Some(progress) = progress.as_ref() {
        progress.identity(
            &candidate_manifest.id,
            &candidate_manifest.name,
            &candidate_manifest.version,
        );
    }
    if let Some(market_app) = market_app.as_ref() {
        validate_market_connector_identity(market_app, &candidate_manifest.id)?;
        if candidate_manifest.version != market_app.version {
            return Err(format!(
                "市场版本与安装包清单不匹配：市场 `{}`，安装包 `{}`",
                market_app.version, candidate_manifest.version
            ));
        }
    }
    let bundled_cli = bundled_baijimu_cli_path();
    managed_tool_dependency::ensure_ready(
        &candidate_manifest,
        bridge_agent::ConnectorManagedToolDependencyPhase::Install,
        bundled_cli.as_deref(),
    )
    .await
    .map_err(|err| format!("应用依赖检查失败: {err:#}"))?;
    let existing = list_connectors()
        .map_err(|err| err.to_string())?
        .into_iter()
        .find(|connector| connector.id == candidate_manifest.id);
    let restart_after_replace = if options.replace {
        match existing.as_ref() {
            Some(connector) => {
                connector_local_app_is_healthy(config_path, &connector.id, connector_processes)
                    .await?
            }
            None => false,
        }
    } else {
        false
    };

    let provenance = match market_app.as_ref() {
        Some(market_app) => ConnectorInstallProvenance::platform_trusted(
            &resolved_source_text,
            &market_app.id,
            resolved_checksum.as_deref().unwrap_or_default(),
        )
        .map_err(|err| err.to_string())?,
        None => ConnectorInstallProvenance::user_trusted(Some(&resolved_source_text)),
    };
    let operation_kind = if existing.is_some() && options.replace {
        ConnectorOperationKind::Upgrade
    } else {
        ConnectorOperationKind::Install
    };
    let operation = connector_lifecycles
        .begin(
            &candidate_manifest.id,
            operation_kind,
            Some(candidate_manifest.version.clone()),
            if operation_kind == ConnectorOperationKind::Upgrade {
                "正在切换应用版本"
            } else {
                "正在安装应用"
            },
        )
        .await?;
    let operation_result = async {
        if let Some(progress) = progress.as_ref() {
            progress.report(
                LocalAppInstallTaskPhase::Installing,
                Some(72),
                "正在安装并注册应用",
            );
        }
        connector_lifecycles.advance(
            &candidate_manifest.id,
            &operation.id,
            operation_kind.lifecycle(),
            "正在安装并注册应用",
            Some(72),
        )?;
        if options.replace {
            if let Some(connector) = existing.as_ref() {
                connector_processes
                    .stop_if_managed(&connector.id, config_path)
                    .await?;
            }
        }
        let install = match install_connector_from_path_with_provenance(
            resolved_source.path(),
            config_path,
            options.replace,
            provenance,
        ) {
            Ok(install) => install,
            Err(err) => {
                if restart_after_replace {
                    if let Err(restart_err) = start_connector_and_wait(
                        connector_processes,
                        config_path,
                        &candidate_manifest.id,
                        "恢复旧版应用",
                    )
                    .await
                    {
                        return Err(format!(
                            "应用升级失败: {err:#}；恢复旧版进程也失败: {restart_err:#}"
                        ));
                    }
                }
                return Err(err.to_string());
            }
        };

        let should_start = options.start || restart_after_replace;
        let started = if should_start {
            if let Some(progress) = progress.as_ref() {
                progress.report(
                    LocalAppInstallTaskPhase::Starting,
                    Some(88),
                    "应用已安装，正在启动并检查运行状态",
                );
            }
            connector_lifecycles.advance(
                &candidate_manifest.id,
                &operation.id,
                ConnectorLifecycleState::Starting,
                "应用已安装，正在启动并检查运行状态",
                Some(88),
            )?;
            let started = start_connector_and_wait(
                connector_processes,
                config_path,
                &install.connector_id,
                "启动新版应用",
            )
            .await
            .map_err(|err| format!("新版应用已安装，但启动失败: {err}"))?;
            Some(started)
        } else {
            None
        };

        if let Some(progress) = progress.as_ref() {
            progress.report(
                LocalAppInstallTaskPhase::Finalizing,
                Some(96),
                "正在刷新本地应用能力",
            );
        }
        connector_lifecycles.advance(
            &candidate_manifest.id,
            &operation.id,
            if should_start {
                ConnectorLifecycleState::Starting
            } else {
                operation_kind.lifecycle()
            },
            "正在刷新本地应用能力",
            Some(96),
        )?;
        let runtime = runtime_manager
            .apply_capabilities_from_path(config_path)
            .await
            .map_err(|err| err.to_string())?;
        registered_services.request_refresh();
        let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
        let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
        Ok(ConnectorAppInstallDocument {
            install,
            start: started,
            setup: None,
            config: ConfigDocument {
                config_path: config_path.display().to_string(),
                manifest_preview,
                config: config_for_ui(&config)?,
                runtime,
            },
        })
    }
    .await;

    match operation_result {
        Ok(document) => {
            if document.start.is_some() {
                connector_lifecycles.complete_ready(
                    operation,
                    Some(document.install.version.clone()),
                    connector_processes
                        .managed_pid(&candidate_manifest.id)
                        .await,
                    "应用已启动并通过就绪检查",
                )?;
            } else {
                connector_lifecycles.complete_stopped(
                    operation,
                    Some(document.install.version.clone()),
                    "应用已安装，等待启动",
                )?;
            }
            Ok(document)
        }
        Err(error) => {
            let recovered = connector_local_app_is_healthy(
                config_path,
                &candidate_manifest.id,
                connector_processes,
            )
            .await
            .unwrap_or(false);
            if recovered {
                let observed_version = show_connector(&candidate_manifest.id)
                    .ok()
                    .map(|record| record.manifest.version);
                connector_lifecycles.complete_ready(
                    operation,
                    observed_version,
                    connector_processes
                        .managed_pid(&candidate_manifest.id)
                        .await,
                    "升级失败，已恢复原运行版本",
                )?;
            } else {
                connector_lifecycles.fail(operation, &error)?;
            }
            Err(error)
        }
    }
}

fn ensure_connector_lifecycle_command_succeeded(
    action: &str,
    result: &ConnectorStartResult,
) -> Result<(), String> {
    let failures = &result.lifecycle;
    if failures.configured && failures.exit_code == Some(0) {
        Ok(())
    } else {
        let detail = if !failures.configured {
            "命令未配置".to_string()
        } else if !failures.stderr.trim().is_empty() {
            failures.stderr.trim().to_string()
        } else {
            format!("退出码 {:?}", failures.exit_code)
        };
        Err(format!("{action}失败：{}: {detail}", failures.connector_id))
    }
}

async fn connector_local_app_is_healthy(
    config_path: &Path,
    connector_id: &str,
    connector_processes: &ConnectorProcessManager,
) -> Result<bool, String> {
    let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
    if !config
        .local_apps
        .iter()
        .any(|app| app.connector_id == connector_id)
    {
        // The install record is the source of truth and local_apps is derived state. Rebuild a
        // missing entry before deciding whether a running process must survive replacement.
        sync_installed_connector(config_path, connector_id).map_err(|err| err.to_string())?;
    }
    let process_running = connector_processes.managed_running(connector_id).await;
    let status = connector_local_app_status(config_path, connector_id, process_running).await?;
    Ok(status.status == RegisteredServiceState::Healthy)
}

async fn connector_local_app_status(
    config_path: &Path,
    connector_id: &str,
    process_running: Option<bool>,
) -> Result<LocalAppRuntimeStatus, String> {
    let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|err| err.to_string())?;
    let app = config
        .local_apps
        .into_iter()
        .find(|app| app.connector_id == connector_id)
        .ok_or_else(|| format!("本地应用 `{connector_id}` 不在当前配置中"))?;
    Ok(check_local_app(&client, app, process_running).await)
}

async fn wait_for_connector_health(
    config_path: &Path,
    connector_id: &str,
    expected_healthy: bool,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = connector_local_app_status(config_path, connector_id, None).await?;
        let matches = if expected_healthy {
            !status.health_check_configured || status.status == RegisteredServiceState::Healthy
        } else {
            !status.health_check_configured || status.status != RegisteredServiceState::Healthy
        };
        if matches {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let details = format!(
                "{}={:?} ({})",
                status.connector_id,
                status.status,
                status.detail.as_deref().unwrap_or("无详情")
            );
            let expected = if expected_healthy { "健康" } else { "停止" };
            return Err(format!("等待应用进入{expected}状态超时：{details}"));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn start_connector_and_wait(
    connector_processes: &ConnectorProcessManager,
    config_path: &Path,
    connector_id: &str,
    action: &str,
) -> Result<ConnectorStartResult, String> {
    let record = show_connector(connector_id).map_err(|err| err.to_string())?;
    let bundled_cli = bundled_baijimu_cli_path();
    let dependency_env = managed_tool_dependency::ensure_ready(
        &record.manifest,
        bridge_agent::ConnectorManagedToolDependencyPhase::Start,
        bundled_cli.as_deref(),
    )
    .await
    .map_err(|err| format!("{action}前的应用依赖检查失败: {err:#}"))?;
    let result = connector_processes
        .start(connector_id, config_path, dependency_env)
        .await?;
    let verification = ensure_connector_lifecycle_command_succeeded(action, &result);
    if let Err(error) = verification {
        return Err(cleanup_failed_connector_start(
            connector_processes,
            config_path,
            &result.connector_id,
            error,
        )
        .await);
    }
    if connector_processes
        .managed_running(&result.connector_id)
        .await
        == Some(false)
    {
        return Err(cleanup_failed_connector_start(
            connector_processes,
            config_path,
            &result.connector_id,
            format!("{action}失败：宿主管理进程已提前退出"),
        )
        .await);
    }
    if let Err(error) = wait_for_connector_health(config_path, &result.connector_id, true).await {
        return Err(cleanup_failed_connector_start(
            connector_processes,
            config_path,
            &result.connector_id,
            error,
        )
        .await);
    }
    Ok(result)
}

async fn start_connector_with_lifecycle(
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    config_path: &Path,
    connector_id: &str,
    action: &str,
) -> Result<ConnectorStartResult, String> {
    let version = show_connector(connector_id)
        .map_err(|error| error.to_string())?
        .manifest
        .version;
    let operation = connector_lifecycles
        .begin(
            connector_id,
            ConnectorOperationKind::Start,
            Some(version.clone()),
            action,
        )
        .await?;
    match start_connector_and_wait(connector_processes, config_path, connector_id, action).await {
        Ok(result) => {
            connector_lifecycles.complete_ready(
                operation,
                Some(version),
                connector_processes.managed_pid(connector_id).await,
                "应用已启动并通过就绪检查",
            )?;
            Ok(result)
        }
        Err(error) => {
            connector_lifecycles.fail(operation, &error)?;
            Err(error)
        }
    }
}

async fn cleanup_failed_connector_start(
    connector_processes: &ConnectorProcessManager,
    config_path: &Path,
    connector_id: &str,
    error: String,
) -> String {
    match connector_processes.stop(connector_id, config_path).await {
        Ok(_) => format!("{error}；已回收未通过启动验证的应用进程"),
        Err(cleanup_error) => {
            format!("{error}；回收未通过启动验证的应用进程也失败: {cleanup_error}")
        }
    }
}

async fn stop_connector_and_wait(
    connector_processes: &ConnectorProcessManager,
    config_path: &Path,
    connector_id: &str,
    action: &str,
) -> Result<ConnectorStartResult, String> {
    let result = connector_processes.stop(connector_id, config_path).await?;
    ensure_connector_lifecycle_command_succeeded(action, &result)?;
    wait_for_connector_health(config_path, &result.connector_id, false).await?;
    Ok(result)
}

async fn stop_connector_with_lifecycle(
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    config_path: &Path,
    connector_id: &str,
    action: &str,
) -> Result<ConnectorStartResult, String> {
    let version = show_connector(connector_id)
        .ok()
        .map(|record| record.manifest.version);
    let operation = connector_lifecycles
        .begin(
            connector_id,
            ConnectorOperationKind::Stop,
            version.clone(),
            action,
        )
        .await?;
    match stop_connector_and_wait(connector_processes, config_path, connector_id, action).await {
        Ok(result) => {
            connector_lifecycles.complete_stopped(operation, version, "应用已停止")?;
            Ok(result)
        }
        Err(error) => {
            connector_lifecycles.fail(operation, &error)?;
            Err(error)
        }
    }
}

#[tauri::command]
async fn start_connector_app(
    state: tauri::State<'_, DesktopState>,
    id: String,
) -> Result<ConnectorStartResult, String> {
    let result = start_connector_with_lifecycle(
        &state.connector_lifecycles,
        &state.connector_processes,
        &state.config_path,
        id.trim(),
        "启动应用",
    )
    .await;
    state.registered_services.request_refresh();
    result
}

#[tauri::command]
async fn stop_connector_app(
    state: tauri::State<'_, DesktopState>,
    id: String,
) -> Result<ConnectorStartResult, String> {
    let result = stop_connector_with_lifecycle(
        &state.connector_lifecycles,
        &state.connector_processes,
        &state.config_path,
        id.trim(),
        "停止应用",
    )
    .await;
    state.registered_services.request_refresh();
    result
}

#[tauri::command]
async fn uninstall_connector_app(
    state: tauri::State<'_, DesktopState>,
    id: String,
    force: Option<bool>,
) -> Result<ConfigDocument, ConnectorUninstallCommandError> {
    let connector_id = id.trim().to_string();
    let document = uninstall_connector_app_with_context(
        &state.config_path,
        &state.runtime,
        &state.connector_lifecycles,
        &state.connector_processes,
        &state.registered_services,
        connector_id.clone(),
        force.unwrap_or(false),
    )
    .await?;
    state
        .local_apps
        .notify(LocalAppsChangeOperation::Uninstall, &connector_id);
    Ok(document)
}

async fn uninstall_connector_app_with_context(
    config_path: &Path,
    runtime_manager: &AgentRuntimeManager,
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    registered_services: &RegisteredServiceMonitor,
    id: String,
    force: bool,
) -> Result<ConfigDocument, ConnectorUninstallCommandError> {
    let connector_id = id.trim().to_string();
    let operation = connector_lifecycles
        .begin(
            &connector_id,
            ConnectorOperationKind::Uninstall,
            None,
            "正在停止并卸载应用",
        )
        .await
        .map_err(|message| ConnectorUninstallCommandError::Failed { message })?;
    let result = async {
    let managed_stop = connector_processes
        .stop_if_managed(id.trim(), config_path)
        .await;
    if let Err(error) = managed_stop {
        if !force {
            return Err(ConnectorUninstallCommandError::StopFailed { message: error });
        }
        log::warn!(
            "continuing explicit forced uninstall for connector `{}` after host-managed stop failed: {}",
            id.trim(),
            error
        );
    }
    uninstall_connector_with_options(id.trim(), config_path, ConnectorUninstallOptions { force })
        .map_err(|error| {
        let stop_failed = is_connector_package_stop_error(&error);
        let message = format!("{error:#}");
        if stop_failed && !force {
            ConnectorUninstallCommandError::StopFailed { message }
        } else {
            ConnectorUninstallCommandError::Failed { message }
        }
    })?;
    let runtime = runtime_manager
        .apply_capabilities_from_path(config_path)
        .await
        .map_err(|error| ConnectorUninstallCommandError::Failed {
            message: error.to_string(),
        })?;
    registered_services.request_refresh();
    let config =
        load_agent_config(config_path).map_err(|error| ConnectorUninstallCommandError::Failed {
            message: format!("{error:#}"),
        })?;
    let manifest_preview =
        manifest_preview_json(&config).map_err(|error| ConnectorUninstallCommandError::Failed {
            message: format!("{error:#}"),
        })?;
    Ok(ConfigDocument {
        config_path: config_path.display().to_string(),
        manifest_preview,
        config: config_for_ui(&config)
            .map_err(|message| ConnectorUninstallCommandError::Failed { message })?,
        runtime,
    })
    }
    .await;
    match result {
        Ok(document) => {
            connector_lifecycles
                .complete_absent(operation)
                .map_err(|message| ConnectorUninstallCommandError::Failed { message })?;
            Ok(document)
        }
        Err(error) => {
            let _ = connector_lifecycles.fail(operation, error.message());
            Err(error)
        }
    }
}

#[tauri::command]
fn request_desktop_permission(permission: String) -> Result<DesktopPermissionStatus, String> {
    #[cfg(target_os = "macos")]
    {
        match permission.trim() {
            "screen_recording" => {
                let _ = unsafe { CGRequestScreenCaptureAccess() };
            }
            "accessibility" => {
                prompt_accessibility_permission();
                let _ = unsafe { CGRequestPostEventAccess() };
            }
            other => return Err(format!("不支持的权限类型: {other}")),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = permission;
    }

    Ok(read_desktop_permission_status())
}

#[tauri::command]
fn open_desktop_permission_settings(permission: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let target = match permission.trim() {
            "screen_recording" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
            "accessibility" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            "full_disk_access" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
            }
            other => return Err(format!("不支持的权限类型: {other}")),
        };

        if open::that(target).is_ok() {
            return Ok(());
        }

        open::that("x-apple.systempreferences:com.apple.preference.security")
            .or_else(|_| open::that("x-apple.systempreferences:"))
            .or_else(|_| open::that("System Settings"))
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = permission;
        Err("当前平台暂不支持打开桌面权限设置".to_string())
    }
}

#[tauri::command]
async fn start_browser_auth(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
) -> Result<BrowserAuthStartResponse, String> {
    let mut config = config;
    let normalized = config.normalize();
    let agent_id_changed = ensure_browser_auth_agent_id(&mut config);
    if normalized || agent_id_changed {
        save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    }
    let client = Client::new();
    let manifest = browser_auth_manifest_json(&config).map_err(|err| err.to_string())?;
    let base_url = config.platform.base_url.trim_end_matches('/');
    let mut payload = serde_json::Map::new();
    if let Some(workspace_id) = config.platform.workspace_id {
        payload.insert("workspaceId".to_string(), serde_json::json!(workspace_id));
    }
    payload.insert(
        "deviceId".to_string(),
        serde_json::json!(config.relay.agent_id),
    );
    payload.insert(
        "deviceName".to_string(),
        serde_json::json!(config.device.name),
    );
    payload.insert(
        "deviceDescription".to_string(),
        serde_json::json!(config.device.description),
    );
    payload.insert("serviceManifest".to_string(), serde_json::json!(manifest));
    let response = client
        .post(format!(
            "{base_url}/api/external-workspace-device-auth/start"
        ))
        .json(&payload)
        .send()
        .await
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let payload = response.text().await.unwrap_or_default();
        return Err(format!(
            "启动浏览器授权失败: {}",
            describe_upstream_http_failure(status, &content_type, &payload)
        ));
    }

    let payload: BrowserAuthStartResponse = response.json().await.map_err(|err| err.to_string())?;
    open::that(payload.verification_uri_complete.clone()).map_err(|err| err.to_string())?;
    Ok(payload)
}

#[tauri::command]
async fn poll_browser_auth(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
    device_code: String,
) -> Result<BrowserAuthPollResponse, CommandError> {
    let client = Client::new();
    let base_url = config.platform.base_url.trim_end_matches('/');
    let response = client
        .post(format!(
            "{base_url}/api/external-workspace-device-auth/poll"
        ))
        .json(&serde_json::json!({
            "deviceCode": device_code
        }))
        .send()
        .await
        .map_err(|err| command_error_message(err.to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let payload = response.text().await.unwrap_or_default();
        return Err(command_error_message(format!(
            "轮询浏览器授权失败: {}",
            describe_upstream_http_failure(status, &content_type, &payload)
        )));
    }

    let payload: RawBrowserAuthPollResponse = response
        .json()
        .await
        .map_err(|err| command_error_message(err.to_string()))?;
    if payload.status != "authorized" {
        return Ok(BrowserAuthPollResponse {
            status: payload.status,
            message: payload.message,
            config: None,
            runtime: None,
        });
    }

    let authorized = payload
        .authorized_payload
        .ok_or_else(|| command_error_message("授权成功但缺少 authorizedPayload"))?;
    let mut updated = config;
    apply_authorized_device_credentials(&mut updated, &authorized);
    save_agent_config(&state.config_path, &updated)
        .map_err(|err| command_error_message(err.to_string()))?;
    match write_shared_cli_auth(&updated, &authorized) {
        Ok(Some(result)) => {
            state
                .runtime
                .push_desktop_log(
                    "info",
                    &format!("shared CLI auth written to {}", result.path.display()),
                    LogMetadata::category("desktop_auth")
                        .event("shared_cli_auth")
                        .outcome("written"),
                )
                .await;
        }
        Ok(None) => {
            state
                .runtime
                .push_desktop_log(
                    "warn",
                    "authorized payload did not include a local client token; skipped shared CLI auth",
                    LogMetadata::category("desktop_auth")
                        .event("shared_cli_auth")
                        .outcome("skipped"),
                )
                .await;
        }
        Err(err) => {
            state
                .runtime
                .push_desktop_log(
                    "error",
                    &format!("failed to write shared CLI auth: {err:#}"),
                    LogMetadata::category("desktop_auth")
                        .event("shared_cli_auth")
                        .outcome("failed"),
                )
                .await;
            return Err(command_error_message(err.to_string()));
        }
    }
    let runtime = restart_agent_from_saved_config(&state)
        .await
        .map_err(CommandError::from)?;

    Ok(BrowserAuthPollResponse {
        status: payload.status,
        message: payload.message,
        config: Some(config_for_ui(&updated).map_err(command_error_message)?),
        runtime: Some(runtime),
    })
}

fn apply_authorized_device_credentials(config: &mut AgentConfig, authorized: &AuthorizedPayload) {
    config.platform.workspace_id = Some(authorized.workspace_id);
    config.relay.agent_id = authorized.device_id.clone();
    config.relay.url = authorized.relay_ws_url.clone();
    config.relay.token = authorized.agent_token.clone();
    config.relay.token_issued_at_epoch_seconds = authorized
        .issued_at_epoch_seconds
        .map(|value| value.to_string());
    config.relay.token_expires_at_epoch_seconds = authorized
        .expires_at_epoch_seconds
        .map(|value| value.to_string());
}

struct SharedCliAuthWriteResult {
    path: PathBuf,
}

fn write_shared_cli_auth(
    config: &AgentConfig,
    authorized: &AuthorizedPayload,
) -> anyhow::Result<Option<SharedCliAuthWriteResult>> {
    let Some(local_client_token) = authorized.local_client_token.as_deref() else {
        return Ok(None);
    };
    if local_client_token.trim().is_empty() {
        return Ok(None);
    }

    let path = shared_cli_auth_path();
    write_shared_cli_auth_at(&path, config, authorized)?;
    Ok(Some(SharedCliAuthWriteResult { path }))
}

fn write_shared_cli_auth_at(
    path: &Path,
    config: &AgentConfig,
    authorized: &AuthorizedPayload,
) -> anyhow::Result<()> {
    let local_client_token = authorized
        .local_client_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("authorized payload is missing local client token"))?;
    if !local_client_token.starts_with("lc_pat_") {
        anyhow::bail!("authorized payload local client token is not a Baijimu PAT");
    }
    if authorized
        .local_client_token_type
        .as_deref()
        .is_some_and(|token_type| !matches!(token_type, "pat" | "workspace_user_api_key"))
    {
        anyhow::bail!("authorized payload local client token type is not a PAT");
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut document = if path.exists() {
        let content = fs::read_to_string(path)?;
        serde_json::from_str::<Value>(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    if !document.is_object() {
        document = serde_json::json!({});
    }

    document["currentEnvironment"] = serde_json::json!("prod");
    document["currentWorkspaceId"] = serde_json::json!(authorized.workspace_id);
    if !document
        .get("environments")
        .map(|value| value.is_object())
        .unwrap_or(false)
    {
        document["environments"] = serde_json::json!({});
    }
    document["environments"]["prod"] = serde_json::json!({
        "baseUrl": config.platform.base_url.trim_end_matches('/'),
    });

    let mut credentials = document
        .get("credentials")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    if let Some(legacy_credentials) = document
        .get("machineCredentials")
        .and_then(|value| value.as_array())
    {
        credentials.extend(legacy_credentials.iter().cloned());
    }
    credentials = credentials
        .into_iter()
        .filter_map(normalize_shared_pat_credential)
        .collect();
    let mut seen_credentials = HashSet::new();
    credentials.retain(|credential| {
        let identity = credential
            .get("credentialId")
            .and_then(Value::as_str)
            .map(|value| format!("id:{value}"))
            .or_else(|| {
                credential
                    .get("token")
                    .and_then(Value::as_str)
                    .map(|value| format!("token:{value}"))
            });
        identity.is_none_or(|value| seen_credentials.insert(value))
    });
    credentials.retain(|item| {
        !credential_has_workspace(item, authorized.workspace_id)
            || item.get("clientId").and_then(|value| value.as_str())
                != Some(authorized.device_id.as_str())
    });
    credentials.push(serde_json::json!({
        "credentialId": authorized.local_client_key_id,
        "userId": authorized.local_client_user_id,
        "workspaceIds": [authorized.workspace_id],
        "clientId": authorized.device_id,
        "token": local_client_token,
        "tokenType": "pat",
        "subjectType": "user",
        "source": "bridge-agent",
        "scopes": if authorized.local_client_scopes.is_empty() {
            vec![
                "baijimu:agent-cli".to_string(),
                "partner:api".to_string(),
                format!("workspace:{}", authorized.workspace_id),
            ]
        } else {
            authorized.local_client_scopes.clone()
        },
        "issuedAt": authorized.local_client_issued_at,
        "issuedAtEpochSeconds": now_epoch_seconds(),
        "expiresAt": authorized.local_client_expires_at,
    }));
    document["schemaVersion"] = serde_json::json!(2);
    document["credentials"] = Value::Array(credentials);
    if let Some(object) = document.as_object_mut() {
        object.remove("machineCredentials");
    }

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid shared CLI auth path: {}", path.display()))?;
    let mut temp_file = tempfile::NamedTempFile::new_in(parent)?;
    temp_file.write_all(&serde_json::to_vec_pretty(&document)?)?;
    #[cfg(unix)]
    temp_file
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temp_file.as_file().sync_all()?;
    temp_file
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to atomically replace {}", path.display()))?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn normalize_shared_pat_credential(mut credential: Value) -> Option<Value> {
    let object = credential.as_object_mut()?;
    if !object
        .get("token")
        .and_then(Value::as_str)
        .is_some_and(|token| token.starts_with("lc_pat_"))
    {
        return None;
    }
    let workspace_ids = object
        .get("workspaceIds")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| {
            object
                .get("workspaceId")
                .and_then(Value::as_u64)
                .map(|workspace_id| vec![Value::from(workspace_id)])
        })
        .unwrap_or_default();
    if object.get("credentialId").is_none() {
        if let Some(key_id) = object.get("keyId").cloned() {
            object.insert("credentialId".to_string(), key_id);
        }
    }
    object.insert("workspaceIds".to_string(), Value::Array(workspace_ids));
    object.insert("tokenType".to_string(), Value::String("pat".to_string()));
    object.insert("subjectType".to_string(), Value::String("user".to_string()));
    if object.get("source").is_none() {
        object.insert(
            "source".to_string(),
            Value::String("bridge-agent".to_string()),
        );
    }
    object.remove("workspaceId");
    object.remove("keyId");
    Some(credential)
}

fn credential_has_workspace(credential: &Value, workspace_id: u64) -> bool {
    credential
        .get("workspaceIds")
        .and_then(Value::as_array)
        .is_some_and(|workspace_ids| {
            workspace_ids
                .iter()
                .any(|value| value.as_u64() == Some(workspace_id))
        })
}

fn shared_cli_auth_path() -> PathBuf {
    if let Some(config_home) = std::env::var_os("BAIJIMU_CONFIG_HOME") {
        return PathBuf::from(config_home).join("baijimu").join("auth.json");
    }
    let home = shared_cli_home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("baijimu").join("auth.json")
}

fn shared_cli_home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(user_profile) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(user_profile));
        }
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

async fn restart_agent_from_saved_config(
    state: &tauri::State<'_, DesktopState>,
) -> anyhow::Result<RuntimeSnapshot> {
    state.runtime.stop().await?;
    start_runtime_from_saved_config(&state.runtime, &state.config_path).await
}

async fn start_runtime_from_saved_config(
    runtime: &AgentRuntimeManager,
    config_path: &Path,
) -> anyhow::Result<RuntimeSnapshot> {
    runtime.start_from_path(config_path).await
}

#[tauri::command]
fn app_version() -> AppVersionInfo {
    AppVersionInfo {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        current_target: current_update_target(),
    }
}

#[tauri::command]
fn open_app_uninstaller() -> Result<(), String> {
    #[cfg(windows)]
    {
        let executable =
            std::env::current_exe().map_err(|err| format!("无法确定客户端安装目录: {err}"))?;
        let uninstaller = executable
            .parent()
            .ok_or_else(|| "无法确定客户端安装目录".to_string())?
            .join("bridge-agent-uninstaller.exe");
        if !uninstaller.is_file() {
            return Err(format!(
                "未找到百积木卸载器 {}，请先通过官方安装包修复安装",
                uninstaller.display()
            ));
        }
        let mut command = Command::new(&uninstaller);
        command.arg("--interactive");
        configure_desktop_command(&mut command);
        command
            .spawn()
            .map_err(|err| format!("启动百积木卸载器失败: {err}"))?;
        Ok(())
    }

    #[cfg(not(windows))]
    Err("当前平台请使用系统的软件包管理方式卸载百积木".to_string())
}

#[tauri::command]
fn get_startup_health(state: tauri::State<'_, DesktopState>) -> StartupHealthSnapshot {
    state.startup_health.snapshot()
}

#[tauri::command]
fn mark_frontend_ready(
    state: tauri::State<'_, DesktopState>,
) -> Result<StartupHealthSnapshot, String> {
    state.startup_health.mark_frontend_ready()
}

#[tauri::command]
fn restart_in_normal_mode(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), String> {
    if state.startup_health.snapshot().forced_safe_mode {
        return Err("当前进程由 --safe-mode 参数启动，请移除该参数后重新启动应用".to_string());
    }
    state.startup_health.reset_for_normal_restart()?;
    state
        .startup_health
        .diagnostics
        .info("normal mode restart requested from recovery UI");
    request_interactive_restart(&state.config_path)?;
    app.restart();
}

#[tauri::command]
fn open_startup_log(state: tauri::State<'_, DesktopState>) -> Result<(), String> {
    let path = state.startup_health.snapshot().startup_log_path;
    open::that(path).map_err(|err| format!("打开启动日志失败: {err}"))
}

#[tauri::command]
async fn check_app_update() -> Result<AppUpdateStatus, String> {
    let current_version = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|err| format!("当前版本号无效: {err}"))?;
    let release = fetch_latest_release().await?;
    let latest_version = release_version(&release)?;
    let preferred_asset = select_tauri_updater_asset(&release);
    let release_url = release_page_url(&release);
    let release_name = release.release_name.clone();
    let published_at = release.published_at.clone();
    let asset_name = preferred_asset.map(|asset| asset.name.clone());
    let auto_download_available = preferred_asset.is_some();
    let force_update_required = release_force_update_required(&release, &current_version);
    let update_available = force_update_required
        || release
            .update_available
            .unwrap_or(latest_version > current_version);

    Ok(AppUpdateStatus {
        current_version: current_version.to_string(),
        latest_version: Some(latest_version.to_string()),
        update_available,
        force_update_required,
        minimum_supported_version: release.minimum_supported_version.clone(),
        force_update_message: release.force_update_message.clone(),
        release_url,
        release_name,
        published_at,
        current_target: current_update_target(),
        auto_download_available,
        asset_name,
    })
}

#[tauri::command]
async fn install_app_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<AppUpdateInstallResult, String> {
    emit_app_update_progress(
        &app,
        AppUpdateProgress {
            phase: "checking".to_string(),
            message: "正在获取最新版本信息".to_string(),
            version: None,
            asset_name: None,
            downloaded_bytes: None,
            total_bytes: None,
            downloaded_path: None,
        },
    );

    let updater = app
        .updater()
        .map_err(|err| format!("初始化官方更新器失败: {err}"))?;
    let Some(update) = updater
        .check()
        .await
        .map_err(|err| format!("检查官方更新失败: {err}"))?
    else {
        return Ok(AppUpdateInstallResult {
            status: "up_to_date".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            asset_name: None,
            downloaded_path: None,
        });
    };
    let update_version = update.version.to_string();
    let asset_name = update
        .download_url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned);

    emit_app_update_progress(
        &app,
        AppUpdateProgress {
            phase: "downloading".to_string(),
            message: "正在下载更新包".to_string(),
            version: Some(update_version.clone()),
            asset_name: asset_name.clone(),
            downloaded_bytes: Some(0),
            total_bytes: None,
            downloaded_path: None,
        },
    );

    let progress_app = app.clone();
    let progress_version = update_version.clone();
    let progress_asset_name = asset_name.clone();
    let mut downloaded_bytes = 0_u64;
    let mut last_progress_at = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let update_bytes = update
        .download(
            move |chunk_length, total_bytes| {
                downloaded_bytes = downloaded_bytes.saturating_add(chunk_length as u64);
                if last_progress_at.elapsed() >= Duration::from_millis(250)
                    || total_bytes.is_some_and(|total| downloaded_bytes >= total)
                {
                    emit_app_update_progress(
                        &progress_app,
                        AppUpdateProgress {
                            phase: "downloading".to_string(),
                            message: "正在下载更新包".to_string(),
                            version: Some(progress_version.clone()),
                            asset_name: progress_asset_name.clone(),
                            downloaded_bytes: Some(downloaded_bytes),
                            total_bytes,
                            downloaded_path: None,
                        },
                    );
                    last_progress_at = Instant::now();
                }
            },
            || {},
        )
        .await
        .map_err(|err| format!("下载或校验官方更新失败: {err}"))?;

    emit_app_update_progress(
        &app,
        AppUpdateProgress {
            phase: "installing".to_string(),
            message: "更新包签名校验通过，正在停止 Agent 并安装".to_string(),
            version: Some(update_version.clone()),
            asset_name: asset_name.clone(),
            downloaded_bytes: None,
            total_bytes: None,
            downloaded_path: None,
        },
    );

    let runtime_was_active = state.runtime.snapshot().await.status != RuntimeStatus::Stopped;
    state
        .runtime
        .stop()
        .await
        .map_err(|err| format!("安装更新前停止 Agent Runtime 失败: {err}"))?;

    if let Err(install_err) = update.install(&update_bytes) {
        if !runtime_was_active {
            return Err(format!("安装官方更新失败: {install_err}"));
        }
        let recovery = start_runtime_from_saved_config(&state.runtime, &state.config_path).await;
        return Err(match recovery {
            Ok(_) => format!("安装官方更新失败，Agent Runtime 已恢复运行: {install_err}"),
            Err(recovery_err) => format!(
                "安装官方更新失败，且 Agent Runtime 恢复失败: install={install_err}; recovery={recovery_err}"
            ),
        });
    }

    emit_app_update_progress(
        &app,
        AppUpdateProgress {
            phase: "ready_to_install".to_string(),
            message: "更新已安装，应用即将重启".to_string(),
            version: Some(update_version.clone()),
            asset_name: asset_name.clone(),
            downloaded_bytes: None,
            total_bytes: None,
            downloaded_path: None,
        },
    );
    request_interactive_restart(&state.config_path).map_err(|err| {
        format!("更新已安装，但无法安排客户端以前台模式重启，请手动退出并重新打开百积木: {err}")
    })?;
    let app_to_restart = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        app_to_restart.restart();
    });

    Ok(AppUpdateInstallResult {
        status: "installed".to_string(),
        version: update_version,
        asset_name,
        downloaded_path: None,
    })
}

fn emit_app_update_progress(app: &tauri::AppHandle, progress: AppUpdateProgress) {
    let _ = app.emit(UPDATE_PROGRESS_EVENT, progress);
}

fn parse_release_version(tag_name: &str) -> Result<Version, String> {
    let normalized = tag_name
        .trim()
        .strip_prefix("bridge-agent-v")
        .or_else(|| tag_name.trim().strip_prefix('v'))
        .unwrap_or(tag_name.trim());
    Version::parse(normalized).map_err(|err| err.to_string())
}

fn configured_update_api_url() -> Result<String, String> {
    let Some(url) = option_env!("BRIDGE_AGENT_UPDATE_API_URL")
        .map(str::trim)
        .filter(|url| !url.is_empty())
    else {
        return Err("当前应用未配置更新服务地址，请使用正式发布包或重新构建客户端。".to_string());
    };
    Ok(url.to_string())
}

fn release_page_url(release: &UpdateReleaseResponse) -> Option<String> {
    release
        .release_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned)
}

fn release_version(release: &UpdateReleaseResponse) -> Result<Version, String> {
    let raw_version = release
        .version
        .as_deref()
        .or(release.tag_name.as_deref())
        .ok_or_else(|| "更新服务未返回最新版本号".to_string())?;
    parse_release_version(raw_version).map_err(|err| format!("最新版本号无效: {err}"))
}

fn release_force_update_required(
    release: &UpdateReleaseResponse,
    current_version: &Version,
) -> bool {
    if release.force_update.unwrap_or(false) {
        return true;
    }
    let Some(minimum_version) = release.minimum_supported_version.as_deref() else {
        return false;
    };
    parse_release_version(minimum_version)
        .map(|minimum_version| current_version < &minimum_version)
        .unwrap_or(false)
}

async fn fetch_latest_release() -> Result<UpdateReleaseResponse, String> {
    let update_api_url = configured_update_api_url()?;
    let response = Client::new()
        .get(update_api_url)
        .header(reqwest::header::USER_AGENT, UPDATE_USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json")
        .query(&[
            ("platform", std::env::consts::OS),
            ("arch", std::env::consts::ARCH),
            ("currentVersion", env!("CARGO_PKG_VERSION")),
        ])
        .send()
        .await
        .map_err(|err| format!("检查更新失败: {err}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let payload = response.text().await.unwrap_or_default();
        return Err(format!("检查更新失败 ({status}): {payload}"));
    }

    response
        .json()
        .await
        .map_err(|err| format!("解析最新版本信息失败: {err}"))
}

fn current_update_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn select_tauri_updater_asset(release: &UpdateReleaseResponse) -> Option<&UpdateReleaseAsset> {
    let suffixes: &[&str] = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => &[".app.tar.gz"],
        ("windows", "x86_64") => &["_x64_en-US.msi", ".msi"],
        ("windows", "aarch64") => &["_arm64_en-US.msi", ".msi"],
        ("linux", "x86_64") => &["_amd64.AppImage", ".AppImage"],
        _ => &[],
    };
    suffixes.iter().find_map(|suffix| {
        release.assets.iter().find(|asset| {
            asset.name.ends_with(suffix)
                && asset
                    .signature
                    .as_deref()
                    .is_some_and(|signature| !signature.trim().is_empty())
        })
    })
}

fn read_desktop_permission_status() -> DesktopPermissionStatus {
    #[cfg(target_os = "macos")]
    {
        let accessibility_granted = unsafe {
            AXIsProcessTrusted() || CGPreflightPostEventAccess() || CGPreflightListenEventAccess()
        };
        DesktopPermissionStatus {
            platform: "macos".to_string(),
            accessibility_granted,
            screen_recording_granted: unsafe { CGPreflightScreenCaptureAccess() },
            accessibility_supported: true,
            screen_recording_supported: true,
        }
    }

    #[cfg(windows)]
    {
        DesktopPermissionStatus {
            platform: "windows".to_string(),
            accessibility_granted: true,
            screen_recording_granted: true,
            accessibility_supported: true,
            screen_recording_supported: true,
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        DesktopPermissionStatus {
            platform: std::env::consts::OS.to_string(),
            accessibility_granted: false,
            screen_recording_granted: false,
            accessibility_supported: false,
            screen_recording_supported: false,
        }
    }
}

async fn check_registered_service(
    client: &Client,
    service: ServiceConfig,
) -> RegisteredServiceStatus {
    let health_check_configured = service.health_check.is_some();
    let start_command_configured = service.start_command.is_some();
    let stop_command_configured = service.stop_command.is_some();
    let Some(health_check) = service.health_check else {
        return RegisteredServiceStatus {
            service: service.name,
            status: RegisteredServiceState::NotConfigured,
            detail: Some("没有注册 healthCheck".to_string()),
            checked_at_ms: now_ms(),
            health_check_configured,
            start_command_configured,
            stop_command_configured,
        };
    };

    match health_check {
        ServiceHealthCheck::Http {
            url,
            http_method,
            headers,
            timeout_secs,
            expect_status,
            body_contains,
        } => {
            let method = http_method
                .parse::<reqwest::Method>()
                .unwrap_or(reqwest::Method::GET);
            let mut request = client
                .request(method, &url)
                .timeout(Duration::from_secs(timeout_secs.unwrap_or(3).max(1)));
            for (key, value) in headers {
                request = request.header(key, value);
            }
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let expected_status = expect_status.unwrap_or(200);
                    if status.as_u16() != expected_status {
                        let detail = health_http_error_detail(response, expected_status).await;
                        return RegisteredServiceStatus {
                            service: service.name,
                            status: RegisteredServiceState::Unhealthy,
                            detail: Some(detail),
                            checked_at_ms: now_ms(),
                            health_check_configured,
                            start_command_configured,
                            stop_command_configured,
                        };
                    }
                    if let Some(expected_text) = body_contains
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        match response.text().await {
                            Ok(body) if body.contains(expected_text) => {}
                            Ok(_) => {
                                return RegisteredServiceStatus {
                                    service: service.name,
                                    status: RegisteredServiceState::Unhealthy,
                                    detail: Some("health 响应内容不符合 bodyContains".to_string()),
                                    checked_at_ms: now_ms(),
                                    health_check_configured,
                                    start_command_configured,
                                    stop_command_configured,
                                };
                            }
                            Err(err) => {
                                return RegisteredServiceStatus {
                                    service: service.name,
                                    status: RegisteredServiceState::Unknown,
                                    detail: Some(format!("读取 health 响应失败: {err}")),
                                    checked_at_ms: now_ms(),
                                    health_check_configured,
                                    start_command_configured,
                                    stop_command_configured,
                                };
                            }
                        }
                    }
                    RegisteredServiceStatus {
                        service: service.name,
                        status: RegisteredServiceState::Healthy,
                        detail: Some(format!("health HTTP {}", status.as_u16())),
                        checked_at_ms: now_ms(),
                        health_check_configured,
                        start_command_configured,
                        stop_command_configured,
                    }
                }
                Err(err) => RegisteredServiceStatus {
                    service: service.name,
                    status: RegisteredServiceState::Unhealthy,
                    detail: Some(format!("health 检查失败: {err}")),
                    checked_at_ms: now_ms(),
                    health_check_configured,
                    start_command_configured,
                    stop_command_configured,
                },
            }
        }
    }
}

async fn health_http_error_detail(mut response: reqwest::Response, expected_status: u16) -> String {
    let status = response.status().as_u16();
    let mut body = Vec::new();
    while body.len() < HEALTH_ERROR_RESPONSE_MAX_BYTES {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) | Err(_) => break,
        };
        let remaining = HEALTH_ERROR_RESPONSE_MAX_BYTES - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining {
            break;
        }
    }
    format_health_http_error(status, expected_status, &body)
}

fn format_health_http_error(status: u16, expected_status: u16, body: &[u8]) -> String {
    let prefix = format!("health HTTP {status}，期望 {expected_status}");
    let Some(message) = structured_health_error_message(body) else {
        return prefix;
    };
    format!("{prefix}：{message}")
}

fn structured_health_error_message(body: &[u8]) -> Option<String> {
    let payload = serde_json::from_slice::<Value>(body).ok()?;
    let message = [
        "/error/message",
        "/status/startup/error",
        "/status/startup/message",
        "/message",
    ]
    .into_iter()
    .find_map(|pointer| payload.pointer(pointer).and_then(Value::as_str))?;
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    Some(
        normalized
            .chars()
            .take(HEALTH_ERROR_MESSAGE_MAX_CHARS)
            .collect(),
    )
}

async fn check_local_app(
    client: &Client,
    app: LocalAppConfig,
    process_running: Option<bool>,
) -> LocalAppRuntimeStatus {
    let connector_id = app.connector_id.clone();
    let mut status = check_registered_service(
        client,
        ServiceConfig {
            name: connector_id.clone(),
            description: app.description,
            enabled: app.enabled,
            health_check: app.health_check,
            start_command: app.start_command,
            stop_command: app.stop_command,
            methods: Vec::new(),
            events: Vec::new(),
        },
    )
    .await;
    apply_managed_process_status(&mut status, process_running);
    LocalAppRuntimeStatus {
        connector_id,
        status: status.status,
        detail: status.detail,
        checked_at_ms: status.checked_at_ms,
        health_check_configured: status.health_check_configured,
        start_command_configured: status.start_command_configured,
        stop_command_configured: status.stop_command_configured,
        process_managed: process_running.is_some(),
        process_running,
    }
}

fn inactive_local_app_status(
    app: LocalAppConfig,
    process_running: Option<bool>,
) -> LocalAppRuntimeStatus {
    LocalAppRuntimeStatus {
        connector_id: app.connector_id,
        status: RegisteredServiceState::Unhealthy,
        detail: Some("应用尚未由 Bridge Agent 启动".to_string()),
        checked_at_ms: now_ms(),
        health_check_configured: app.health_check.is_some(),
        start_command_configured: app.start_command.is_some(),
        stop_command_configured: app.stop_command.is_some(),
        process_managed: process_running.is_some(),
        process_running,
    }
}

fn apply_managed_process_status(
    status: &mut RegisteredServiceStatus,
    process_running: Option<bool>,
) {
    if status.health_check_configured {
        return;
    }
    match process_running {
        Some(true) => {
            status.status = RegisteredServiceState::Healthy;
            status.detail = Some("宿主管理进程正在运行".to_string());
        }
        Some(false) => {
            status.status = RegisteredServiceState::Unhealthy;
            status.detail = Some("宿主管理进程未运行".to_string());
        }
        None => {}
    }
}

async fn run_start_command(
    service: String,
    start_command: ServiceStartCommand,
) -> Result<StartRegisteredServiceResult, String> {
    match start_command {
        ServiceStartCommand::ShellCommand {
            command,
            cwd,
            env,
            timeout_secs,
        } => {
            if command.is_empty() || command[0].trim().is_empty() {
                return Err(format!("服务 `{service}` 的启动命令为空"));
            }
            let mut process = AsyncCommand::new(&command[0]);
            #[cfg(windows)]
            process.creation_flags(WINDOWS_CREATE_NO_WINDOW);
            process.args(command.iter().skip(1));
            if let Some(cwd) = cwd
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                process.current_dir(cwd);
            }
            process.envs(env);
            process.kill_on_drop(true);

            let stdout_capture = tempfile::NamedTempFile::new()
                .map_err(|err| format!("创建服务 `{service}` 标准输出文件失败: {err}"))?;
            let stderr_capture = tempfile::NamedTempFile::new()
                .map_err(|err| format!("创建服务 `{service}` 标准错误文件失败: {err}"))?;
            process
                .stdout(std::process::Stdio::from(stdout_capture.reopen().map_err(
                    |err| format!("打开服务 `{service}` 标准输出文件失败: {err}"),
                )?))
                .stderr(std::process::Stdio::from(stderr_capture.reopen().map_err(
                    |err| format!("打开服务 `{service}` 标准错误文件失败: {err}"),
                )?));

            let timeout_secs = timeout_secs.unwrap_or(15).max(1);
            let mut child = process
                .spawn()
                .map_err(|err| format!("启动服务 `{service}` 失败: {err}"))?;
            let (status, timed_out) = match timeout(Duration::from_secs(timeout_secs), child.wait())
                .await
            {
                Ok(Ok(status)) => (Some(status), false),
                Ok(Err(err)) => return Err(format!("等待服务 `{service}` 启动命令失败: {err}")),
                Err(_) => {
                    let _ = child.start_kill();
                    let _ = timeout(Duration::from_secs(3), child.wait()).await;
                    (None, true)
                }
            };
            let stdout = read_lifecycle_capture(&service, "stdout", &stdout_capture)?;
            let mut stderr = read_lifecycle_capture(&service, "stderr", &stderr_capture)?;
            if timed_out {
                if !stderr.is_empty() {
                    stderr.push('\n');
                }
                stderr.push_str(&format!("timed out after {timeout_secs}s"));
            }
            Ok(StartRegisteredServiceResult {
                service,
                success: status.as_ref().is_some_and(|status| status.success()),
                exit_code: status.and_then(|status| status.code()),
                stdout,
                stderr,
                timed_out,
            })
        }
    }
}

fn read_lifecycle_capture(
    service: &str,
    stream_name: &str,
    capture: &tempfile::NamedTempFile,
) -> Result<String, String> {
    let file = capture
        .reopen()
        .map_err(|err| format!("读取服务 `{service}` {stream_name} 文件失败: {err}"))?;
    let mut bytes = Vec::new();
    file.take(LIFECYCLE_OUTPUT_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("收集服务 `{service}` {stream_name} 失败: {err}"))?;
    if bytes.len() as u64 > LIFECYCLE_OUTPUT_MAX_BYTES {
        bytes.truncate(LIFECYCLE_OUTPUT_MAX_BYTES as usize);
        bytes.extend_from_slice(b"\n[output truncated by Bridge Agent]");
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

enum ResolvedConnectorSource {
    Local(PathBuf),
    Git {
        path: PathBuf,
        _temp_dir: tempfile::TempDir,
    },
    Archive {
        path: PathBuf,
        _temp_dir: tempfile::TempDir,
    },
}

impl ResolvedConnectorSource {
    fn path(&self) -> &Path {
        match self {
            Self::Local(path) => path.as_path(),
            Self::Git { path, .. } => path.as_path(),
            Self::Archive { path, .. } => path.as_path(),
        }
    }
}

async fn resolve_connector_source(
    source: &str,
    allow_git: bool,
    expected_checksum: Option<&str>,
    progress: Option<&LocalAppInstallProgressReporter>,
) -> Result<ResolvedConnectorSource, String> {
    let (source, git_revision) = split_source_revision(source);
    if let Some(archive_url) =
        connector_archive_download_url(&source, git_revision.as_deref(), allow_git)?
    {
        return resolve_connector_archive_source(&archive_url, expected_checksum, progress).await;
    }

    if is_git_connector_source(&source) {
        if !allow_git {
            return Err(
                "市场本地应用不能依赖本机 git，请将安装源发布为 .zip 或 .tar.gz 下载包。"
                    .to_string(),
            );
        }
        let temp_dir = tempfile::tempdir().map_err(|err| err.to_string())?;
        let checkout_path = temp_dir.path().join("connector");
        let mut command = Command::new("git");
        configure_desktop_command(&mut command);
        command.args(["clone", "--depth", "1"]);
        if let Some(revision) = git_revision.as_deref().filter(|value| !value.is_empty()) {
            command.args(["--branch", revision]);
        }
        let output = command
            .arg(&source)
            .arg(&checkout_path)
            .output()
            .map_err(|err| format!("执行 git clone 失败: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "下载本地应用失败: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        return Ok(ResolvedConnectorSource::Git {
            path: checkout_path,
            _temp_dir: temp_dir,
        });
    }

    let path = PathBuf::from(source);
    if !path.exists() {
        return Err(format!("本地路径不存在: {}", path.display()));
    }
    Ok(ResolvedConnectorSource::Local(path))
}

impl From<RawMarketConnectorApp> for MarketConnectorApp {
    fn from(value: RawMarketConnectorApp) -> Self {
        let host_compatibility = market_host_compatibility(
            &value.latest_version.manifest,
            value.latest_version.compatibility.as_ref(),
        );
        let application_type = value
            .latest_version
            .manifest
            .get("applicationType")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("connector")
            .to_string();
        let artifact = select_market_tool_artifact(&value.latest_version.manifest);
        let source = artifact
            .as_ref()
            .and_then(|artifact| artifact.get("source"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| market_connector_source(&value.latest_version));
        let checksum = artifact
            .as_ref()
            .and_then(|artifact| artifact.get("checksum"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or(value.latest_version.checksum.clone());
        let archive_path = artifact
            .as_ref()
            .and_then(|artifact| artifact.get("archivePath"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let release_notes = market_release_notes(&value.latest_version.manifest);
        let icon_data_url = value
            .latest_version
            .manifest
            .get("icon")
            .cloned()
            .and_then(|icon| serde_json::from_value::<ConnectorIcon>(icon).ok())
            .and_then(|icon| connector_icon_data_url(&icon).ok());
        let config_schema = value.latest_version.manifest.get("configSchema").cloned();
        let database = market_manifest_database(&value.latest_version.manifest);
        let methods = market_manifest_method_contracts(&value.latest_version.manifest);
        let events = market_manifest_event_contracts(&value.latest_version.manifest);
        let configuration_declaration = market_contract_declaration(
            &value.latest_version.manifest,
            "configuration",
            config_schema.is_some(),
            &application_type,
        );
        let interface_declaration = market_contract_declaration(
            &value.latest_version.manifest,
            "interfaces",
            !methods.is_empty() || !events.is_empty(),
            &application_type,
        );
        let database_declaration = market_contract_declaration(
            &value.latest_version.manifest,
            "database",
            database.is_some(),
            &application_type,
        );
        let method_names = methods.iter().map(|method| method.name.clone()).collect();
        let event_names = events.iter().map(|event| event.name.clone()).collect();
        let permissions = market_manifest_permissions(&value.latest_version.manifest);
        Self {
            id: value.id,
            connector_id: value.connector_id,
            application_type,
            name: value.name,
            description: value.description,
            source,
            checksum,
            archive_path,
            risk: value.risk,
            risk_level: value.risk_level.unwrap_or_else(|| "medium".to_string()),
            capability: value.capability,
            version: value.latest_version.version,
            published_at: value.latest_version.published_at,
            icon_data_url,
            release_notes,
            configuration_declaration,
            interface_declaration,
            database_declaration,
            config_schema,
            database,
            methods,
            events,
            method_names,
            event_names,
            permissions,
            compatible: host_compatibility.compatible,
            compatibility_message: host_compatibility.message,
            minimum_host_version: host_compatibility.minimum_host_version,
            required_host_capabilities: host_compatibility.required_capabilities,
            missing_host_capabilities: host_compatibility.missing_capabilities,
        }
    }
}

fn market_release_notes(manifest: &Value) -> Vec<String> {
    ["releaseNotes", "changes", "changelog"]
        .iter()
        .find_map(|field| {
            manifest
                .get(field)
                .and_then(normalized_market_release_notes)
        })
        .unwrap_or_default()
}

fn normalized_market_release_notes(value: &Value) -> Option<Vec<String>> {
    let values = match value {
        Value::String(value) => value.lines().map(str::to_string).collect::<Vec<_>>(),
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>(),
        _ => return None,
    };
    let values = values
        .into_iter()
        .map(|value| {
            value
                .trim()
                .trim_start_matches(['-', '*', '•'])
                .trim()
                .to_string()
        })
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn market_manifest_database(manifest: &Value) -> Option<ConnectorDatabaseContract> {
    manifest
        .get("database")
        .cloned()
        .and_then(|database| serde_json::from_value(database).ok())
}

fn market_contract_declaration(
    manifest: &Value,
    field: &str,
    contract_present: bool,
    application_type: &str,
) -> String {
    if contract_present {
        return "declared".to_string();
    }
    manifest
        .get("upgradeReview")
        .and_then(|review| review.get(field))
        .and_then(Value::as_str)
        .filter(|status| matches!(*status, "declared" | "not_applicable"))
        .map(str::to_string)
        .unwrap_or_else(|| {
            if application_type == "managed_tool" {
                "not_applicable".to_string()
            } else {
                "undeclared".to_string()
            }
        })
}

fn market_manifest_method_contracts(manifest: &Value) -> Vec<ConnectorMethodContract> {
    market_manifest_contract_entries(manifest, "methods")
        .into_iter()
        .filter_map(|entry| {
            let name = market_manifest_text(entry, "name")?;
            Some(ConnectorMethodContract {
                name,
                description: market_manifest_text(entry, "description").unwrap_or_default(),
                input_schema: entry
                    .get("inputSchema")
                    .or_else(|| entry.get("input_schema"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"})),
                response_mode: market_manifest_text(entry, "responseMode")
                    .or_else(|| market_manifest_text(entry, "response_mode"))
                    .unwrap_or_else(|| "cmodel".to_string()),
                path: market_manifest_text(entry, "path").unwrap_or_default(),
                http_method: market_manifest_text(entry, "httpMethod")
                    .or_else(|| market_manifest_text(entry, "http_method"))
                    .map(|method| method.to_uppercase())
                    .unwrap_or_else(|| "POST".to_string()),
            })
        })
        .collect()
}

fn market_manifest_event_contracts(manifest: &Value) -> Vec<ConnectorEventContract> {
    market_manifest_contract_entries(manifest, "events")
        .into_iter()
        .filter_map(|entry| {
            let name = market_manifest_text(entry, "name")?;
            Some(ConnectorEventContract {
                name,
                description: market_manifest_text(entry, "description").unwrap_or_default(),
                payload_schema: entry
                    .get("payloadSchema")
                    .or_else(|| entry.get("payload_schema"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"})),
            })
        })
        .collect()
}

fn market_manifest_contract_entries<'a>(manifest: &'a Value, field: &str) -> Vec<&'a Value> {
    let mut entries = manifest
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if let Some(services) = manifest.get("services").and_then(Value::as_array) {
        for service in services {
            entries.extend(
                service
                    .get(field)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            );
        }
    }
    entries
}

fn market_manifest_text(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn market_manifest_permissions(manifest: &Value) -> Vec<ConnectorPermission> {
    manifest
        .get("permissions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|permission| serde_json::from_value(permission.clone()).ok())
        .collect()
}

#[derive(Debug, Clone)]
struct MarketHostCompatibility {
    compatible: bool,
    message: Option<String>,
    minimum_host_version: Option<String>,
    required_capabilities: Vec<String>,
    missing_capabilities: Vec<String>,
}

fn market_host_compatibility(
    manifest: &Value,
    server: Option<&RawMarketHostCompatibility>,
) -> MarketHostCompatibility {
    let requirements = manifest.get("hostRequirements").and_then(Value::as_object);
    let minimum_host_version = requirements
        .and_then(|value| value.get("minimumVersion"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| server.and_then(|value| value.minimum_host_version.clone()));
    let required_capabilities = requirements
        .and_then(|value| value.get("capabilities"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|values| !values.is_empty())
        .or_else(|| server.map(|value| value.required_capabilities.clone()))
        .unwrap_or_default();
    let current_version = Version::parse(env!("CARGO_PKG_VERSION")).ok();
    let required_version = minimum_host_version
        .as_deref()
        .and_then(|value| Version::parse(value).ok());
    let version_compatible = match (required_version.as_ref(), current_version.as_ref()) {
        (Some(required), Some(current)) => current >= required,
        (Some(_), None) => false,
        (None, _) => true,
    };
    let supported_capabilities = LOCAL_APP_HOST_CAPABILITIES
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let missing_host_capabilities = required_capabilities
        .iter()
        .filter(|capability| !supported_capabilities.contains(capability.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let locally_compatible = version_compatible && missing_host_capabilities.is_empty();
    let compatible = locally_compatible && server.is_none_or(|value| value.compatible);
    let message = if compatible {
        None
    } else {
        server
            .and_then(|value| value.message.clone())
            .or_else(|| {
                (!version_compatible).then(|| {
                    format!(
                        "需要百积木客户端 {} 或更高版本，当前版本为 {}，请先升级客户端",
                        minimum_host_version.as_deref().unwrap_or("最新"),
                        env!("CARGO_PKG_VERSION")
                    )
                })
            })
            .or_else(|| {
                (!missing_host_capabilities.is_empty()).then(|| {
                    format!(
                        "当前百积木客户端缺少所需能力：{}，请先升级客户端",
                        missing_host_capabilities.join("、")
                    )
                })
            })
            .or_else(|| Some("当前百积木客户端不支持该应用版本，请先升级客户端".to_string()))
    };
    MarketHostCompatibility {
        compatible,
        message,
        minimum_host_version,
        required_capabilities,
        missing_capabilities: server
            .map(|value| value.missing_capabilities.clone())
            .filter(|values| !values.is_empty())
            .unwrap_or(missing_host_capabilities),
    }
}

fn validate_market_host_compatibility(market_app: &MarketConnectorApp) -> Result<(), String> {
    if market_app.compatible {
        return Ok(());
    }
    Err(market_app
        .compatibility_message
        .clone()
        .unwrap_or_else(|| "当前百积木客户端不支持该应用版本，请先升级客户端".to_string()))
}

fn validate_market_connector_identity(
    market_app: &MarketConnectorApp,
    connector_id: &str,
) -> Result<(), String> {
    if market_app.application_type != "connector" {
        return Err("该市场条目不是 Connector 应用".to_string());
    }
    if market_app.connector_id.trim() != connector_id.trim() {
        return Err(format!(
            "市场应用 ID 与安装包不匹配：市场 `{}`，安装包 `{}`",
            market_app.connector_id, connector_id
        ));
    }
    if !market_app.source.trim().starts_with("https://") {
        return Err("市场 Connector 安装源必须使用 HTTPS".to_string());
    }
    Ok(())
}

fn required_market_checksum(market_app: &MarketConnectorApp) -> Result<String, String> {
    let checksum = market_app
        .checksum
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "市场本地应用发布包必须提供 SHA-256 checksum".to_string())?;
    let digest = checksum.strip_prefix("sha256:").unwrap_or(checksum);
    if digest.len() != 64
        || !digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err("市场本地应用 SHA-256 checksum 格式无效".to_string());
    }
    Ok(format!("sha256:{}", digest.to_ascii_lowercase()))
}

fn select_market_tool_artifact(manifest: &Value) -> Option<Value> {
    let platform = normalized_platform();
    let arch = std::env::consts::ARCH;
    manifest
        .get("artifacts")?
        .as_array()?
        .iter()
        .find(|artifact| {
            let artifact_platform = artifact
                .get("platform")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();
            let artifact_arch = artifact
                .get("arch")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("universal");
            artifact_platform.eq_ignore_ascii_case(platform)
                && (artifact_arch.eq_ignore_ascii_case(arch)
                    || artifact_arch.eq_ignore_ascii_case("universal")
                    || (arch == "x86_64" && artifact_arch.eq_ignore_ascii_case("x64"))
                    || (arch == "aarch64" && artifact_arch.eq_ignore_ascii_case("arm64")))
        })
        .cloned()
}

fn market_connector_source(version: &RawMarketConnectorVersion) -> String {
    let source_type = version
        .source_type
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if source_type.eq_ignore_ascii_case("git") || is_git_connector_source(&version.source) {
        with_revision(&version.source, version.revision.as_deref())
    } else {
        version.source.trim().to_string()
    }
}

fn with_revision(source: &str, revision: Option<&str>) -> String {
    let source = source.trim();
    match revision.map(str::trim).filter(|value| !value.is_empty()) {
        Some(revision) if !source.contains('#') => format!("{source}#{revision}"),
        _ => source.to_string(),
    }
}

fn split_source_revision(source: &str) -> (String, Option<String>) {
    let source = source.trim();
    match source.rsplit_once('#') {
        Some((base, revision)) if !base.is_empty() && !revision.is_empty() => {
            (base.to_string(), Some(revision.to_string()))
        }
        _ => (source.to_string(), None),
    }
}

fn normalized_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        _ => std::env::consts::OS,
    }
}

fn is_git_connector_source(source: &str) -> bool {
    let value = source.trim();
    value.starts_with("git@")
        || value.ends_with(".git")
        || value.starts_with("ssh://")
        || value.starts_with("git://")
        || parse_https_git_repo(value, "github.com").is_some()
        || parse_https_git_repo(value, "gitee.com").is_some()
}

fn is_http_connector_source(source: &str) -> bool {
    let value = source.trim();
    value.starts_with("https://") || value.starts_with("http://")
}

#[derive(Clone, Copy)]
enum ConnectorArchiveKind {
    Zip,
    TarGz,
}

async fn resolve_connector_archive_source(
    archive_url: &str,
    expected_checksum: Option<&str>,
    progress: Option<&LocalAppInstallProgressReporter>,
) -> Result<ResolvedConnectorSource, String> {
    let kind = connector_archive_kind(archive_url)
        .ok_or_else(|| "本地应用下载源必须是 .zip、.tar.gz 或 .tgz 文件。".to_string())?;
    let mut response = Client::new()
        .get(archive_url)
        .header(reqwest::header::USER_AGENT, CONNECTOR_DOWNLOAD_USER_AGENT)
        .send()
        .await
        .map_err(|err| format!("下载本地应用失败: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let payload = response.text().await.unwrap_or_default();
        return Err(format!("下载本地应用失败 ({status}): {payload}"));
    }
    let total_bytes = response.content_length();
    let mut bytes = Vec::with_capacity(
        total_bytes
            .and_then(|size| usize::try_from(size).ok())
            .unwrap_or_default(),
    );
    if let Some(progress) = progress {
        progress.download(0, total_bytes);
    }
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| format!("读取本地应用下载包失败: {err}"))?
    {
        bytes.extend_from_slice(&chunk);
        if let Some(progress) = progress {
            progress.download(bytes.len() as u64, total_bytes);
        }
    }
    if let Some(progress) = progress {
        progress.report(
            LocalAppInstallTaskPhase::Verifying,
            Some(58),
            "下载完成，正在校验应用包",
        );
    }
    verify_connector_archive_checksum(bytes.as_ref(), expected_checksum)?;
    let temp_dir = tempfile::tempdir().map_err(|err| err.to_string())?;
    let extract_dir = temp_dir.path().join("connector-archive");
    fs::create_dir_all(&extract_dir).map_err(|err| format!("创建本地应用解压目录失败: {err}"))?;
    extract_connector_archive(bytes.as_ref(), kind, &extract_dir)?;
    let path = find_extracted_connector_root(&extract_dir)?;
    Ok(ResolvedConnectorSource::Archive {
        path,
        _temp_dir: temp_dir,
    })
}

fn verify_connector_archive_checksum(
    bytes: &[u8],
    expected_checksum: Option<&str>,
) -> Result<(), String> {
    let Some(expected) = expected_checksum
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let expected = expected
        .strip_prefix("sha256:")
        .unwrap_or(expected)
        .to_ascii_lowercase();
    if expected.len() != 64
        || !expected
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err("本地应用 SHA-256 checksum 格式无效".to_string());
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err(format!(
            "本地应用下载包 SHA-256 校验失败：期望 {expected}，实际 {actual}"
        ));
    }
    Ok(())
}

fn connector_archive_download_url(
    source: &str,
    revision: Option<&str>,
    allow_git: bool,
) -> Result<Option<String>, String> {
    if connector_archive_kind(source).is_some() {
        return Ok(Some(source.trim().to_string()));
    }
    if !is_http_connector_source(source) {
        return Ok(None);
    }
    if !is_git_connector_source(source) {
        return Err(
            "HTTP(S) 本地应用安装源必须是 .zip、.tar.gz、.tgz，或可转换为源码包的 GitHub/Gitee 仓库 URL。".to_string(),
        );
    }
    if allow_git {
        return Ok(None);
    }
    github_archive_url(source, revision)
        .or_else(|| gitee_archive_url(source, revision))
        .map(Some)
        .ok_or_else(|| {
            "市场本地应用不能依赖本机 git，请将安装源发布为 .zip 或 .tar.gz 下载包。".to_string()
        })
}

fn connector_archive_kind(source: &str) -> Option<ConnectorArchiveKind> {
    let source = source
        .split(['?', '#'])
        .next()
        .unwrap_or(source)
        .to_ascii_lowercase();
    if source.ends_with(".zip") {
        Some(ConnectorArchiveKind::Zip)
    } else if source.ends_with(".tar.gz") || source.ends_with(".tgz") {
        Some(ConnectorArchiveKind::TarGz)
    } else {
        None
    }
}

fn github_archive_url(source: &str, revision: Option<&str>) -> Option<String> {
    let (owner, repo) = parse_https_git_repo(source, "github.com")?;
    let revision = revision?.trim();
    if revision.is_empty() {
        return None;
    }
    Some(format!(
        "https://github.com/{owner}/{repo}/archive/{revision}.zip"
    ))
}

fn gitee_archive_url(source: &str, revision: Option<&str>) -> Option<String> {
    let (owner, repo) = parse_https_git_repo(source, "gitee.com")?;
    let revision = revision?.trim();
    if revision.is_empty() {
        return None;
    }
    Some(format!(
        "https://gitee.com/{owner}/{repo}/repository/archive/{revision}.zip"
    ))
}

fn parse_https_git_repo(source: &str, host: &str) -> Option<(String, String)> {
    let without_scheme = source
        .strip_prefix("https://")
        .or_else(|| source.strip_prefix("http://"))?;
    let path = without_scheme.strip_prefix(host)?.trim_start_matches('/');
    let mut parts = path.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim().trim_end_matches(".git");
    if parts.next().is_some() {
        return None;
    }
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

fn extract_connector_archive(
    bytes: &[u8],
    kind: ConnectorArchiveKind,
    destination: &Path,
) -> Result<(), String> {
    match kind {
        ConnectorArchiveKind::Zip => extract_connector_zip(bytes, destination),
        ConnectorArchiveKind::TarGz => extract_connector_tar_gz(bytes, destination),
    }
}

fn extract_connector_zip(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|err| format!("解析本地应用 zip 失败: {err}"))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("读取本地应用 zip 条目失败: {err}"))?;
        let Some(relative_path) = entry.enclosed_name() else {
            return Err("本地应用 zip 包含不安全路径。".to_string());
        };
        let target = destination.join(relative_path);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|err| format!("创建解压目录失败: {err}"))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("创建解压目录失败: {err}"))?;
        }
        let mut file =
            fs::File::create(&target).map_err(|err| format!("写入解压文件失败: {err}"))?;
        std::io::copy(&mut entry, &mut file).map_err(|err| format!("写入解压文件失败: {err}"))?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            let mut permissions = file
                .metadata()
                .map_err(|err| format!("读取解压文件权限失败: {err}"))?
                .permissions();
            permissions.set_mode(mode);
            fs::set_permissions(&target, permissions)
                .map_err(|err| format!("设置解压文件权限失败: {err}"))?;
        }
    }
    Ok(())
}

fn extract_connector_tar_gz(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let decoder = flate2::read::GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|err| format!("解析本地应用 tar.gz 失败: {err}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|err| format!("读取本地应用 tar.gz 条目失败: {err}"))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err("本地应用 tar.gz 包含不支持的链接文件。".to_string());
        }
        let relative_path = sanitize_archive_path(
            &entry
                .path()
                .map_err(|err| format!("读取 tar.gz 路径失败: {err}"))?,
        )?;
        let target = destination.join(relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("创建解压目录失败: {err}"))?;
        }
        entry
            .unpack(&target)
            .map_err(|err| format!("解压本地应用 tar.gz 失败: {err}"))?;
    }
    Ok(())
}

fn sanitize_archive_path(path: &Path) -> Result<PathBuf, String> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => clean.push(part),
            std::path::Component::CurDir => {}
            _ => return Err("本地应用压缩包包含不安全路径。".to_string()),
        }
    }
    if clean.as_os_str().is_empty() {
        return Err("本地应用压缩包包含空路径。".to_string());
    }
    Ok(clean)
}

fn find_extracted_connector_root(extract_dir: &Path) -> Result<PathBuf, String> {
    let mut manifests = Vec::new();
    collect_connector_manifests(extract_dir, &mut manifests)
        .map_err(|err| format!("查找本地应用清单失败: {err}"))?;
    match manifests.len() {
        0 => Err("下载包中没有找到 connector.json。".to_string()),
        1 => Ok(manifests
            .remove(0)
            .parent()
            .unwrap_or(extract_dir)
            .to_path_buf()),
        _ => Err("下载包中包含多个 connector.json，无法确定应用根目录。".to_string()),
    }
}

fn collect_connector_manifests(dir: &Path, manifests: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        if file_name.to_str().is_some_and(|name| name == "__MACOSX") {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_connector_manifests(&path, manifests)?;
        } else if file_type.is_file()
            && file_name
                .to_str()
                .is_some_and(|name| name == CONNECTOR_MANIFEST_FILE)
        {
            manifests.push(path);
        }
    }
    Ok(())
}

fn connector_version_is_newer(latest: &str, current: &str) -> bool {
    let latest = latest.trim().trim_start_matches('v');
    let current = current.trim().trim_start_matches('v');
    match (Version::parse(latest), Version::parse(current)) {
        (Ok(latest), Ok(current)) => latest > current,
        _ => latest != current,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[derive(Clone, Debug)]
struct StartupDiagnostics {
    primary_path: PathBuf,
    fallback_path: PathBuf,
}

impl StartupDiagnostics {
    fn bootstrap() -> Self {
        Self {
            primary_path: std::env::temp_dir().join(STARTUP_LOG_FILE_NAME),
            fallback_path: std::env::temp_dir().join(STARTUP_LOG_FILE_NAME),
        }
    }

    fn for_config_path(config_path: &Path) -> Self {
        let primary_path = resolve_config_base_dir(config_path).join(STARTUP_LOG_FILE_NAME);
        Self {
            primary_path,
            fallback_path: std::env::temp_dir().join(STARTUP_LOG_FILE_NAME),
        }
    }

    fn info(&self, message: impl AsRef<str>) {
        self.write("INFO", message.as_ref());
    }

    fn warn(&self, message: impl AsRef<str>) {
        self.write("WARN", message.as_ref());
    }

    fn error(&self, message: impl AsRef<str>) {
        self.write("ERROR", message.as_ref());
    }

    fn write(&self, level: &str, message: &str) {
        let line = format!("{} [{level}] {message}\n", now_ms());
        if append_startup_log_line(&self.primary_path, &line).is_err()
            && self.fallback_path != self.primary_path
        {
            let _ = append_startup_log_line(&self.fallback_path, &line);
        }
        eprint!("{line}");
        match level {
            "ERROR" => log::error!(target: "desktop_startup", "{message}"),
            "WARN" => log::warn!(target: "desktop_startup", "{message}"),
            _ => log::info!(target: "desktop_startup", "{message}"),
        }
    }
}

fn append_startup_log_line(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())
}

fn install_panic_diagnostics(diagnostics: StartupDiagnostics) {
    panic::set_hook(Box::new(move |panic_info| {
        let location = panic_info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown location".to_string());
        let payload = panic_info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| {
                panic_info
                    .payload()
                    .downcast_ref::<String>()
                    .map(String::as_str)
            })
            .unwrap_or("non-string panic payload");
        diagnostics.error(format!("panic at {location}: {payload}"));
    }));
}

fn log_startup_environment(diagnostics: &StartupDiagnostics, config_path: &Path) {
    diagnostics.info(format!(
        "starting 百积木 desktop version {}",
        env!("CARGO_PKG_VERSION")
    ));
    diagnostics.info(format!("config path: {}", config_path.display()));
    match std::env::current_exe() {
        Ok(path) => {
            diagnostics.info(format!("current exe: {}", path.display()));
            if is_probably_macos_translocated_path(&path) {
                diagnostics.warn(
                    "app appears to be running from /private/var/folders; move 百积木.app to /Applications and launch it there before collecting final diagnostics",
                );
            }
        }
        Err(err) => diagnostics.warn(format!("failed to determine current exe: {err}")),
    }
    match std::env::current_dir() {
        Ok(path) => diagnostics.info(format!("current dir: {}", path.display())),
        Err(err) => diagnostics.warn(format!("failed to determine current dir: {err}")),
    }
}

#[cfg(target_os = "macos")]
fn is_probably_macos_translocated_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.starts_with("/private/var/folders/") || path.starts_with("/var/folders/")
}

#[cfg(not(target_os = "macos"))]
fn is_probably_macos_translocated_path(_path: &Path) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn prompt_accessibility_permission() {
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::true_value();
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
    let _ = unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef().cast()) };
}

fn setup_tray(app: &tauri::App, diagnostics: &StartupDiagnostics) -> tauri::Result<()> {
    diagnostics.info("setting up tray icon");
    let show = MenuItem::with_id(app, TRAY_MENU_SHOW, "打开百积木", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_MENU_QUIT, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let icon = app.default_window_icon().cloned();
    let menu_diagnostics = diagnostics.clone();
    let tray_diagnostics = diagnostics.clone();

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("百积木")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            TRAY_MENU_SHOW => {
                show_main_window(app, Some(&menu_diagnostics), MainWindowOpenReason::TrayMenu)
            }
            TRAY_MENU_QUIT => quit_app(app),
            _ => {}
        })
        .on_tray_icon_event(move |tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(
                    tray.app_handle(),
                    Some(&tray_diagnostics),
                    MainWindowOpenReason::TrayIcon,
                );
            }
        });

    if let Some(icon) = icon {
        tray = tray.icon(icon);
    } else {
        diagnostics.warn("default window icon is unavailable; building tray without an icon");
    }

    tray.build(app)?;
    diagnostics.info("tray icon setup completed");
    Ok(())
}

#[cfg(target_os = "windows")]
fn setup_main_window_icon(app: &tauri::App) -> anyhow::Result<()> {
    let icon = app
        .default_window_icon()
        .cloned()
        .context("default bundled window icon is unavailable")?;
    let window = app
        .get_webview_window("main")
        .context("main webview window is unavailable")?;
    window
        .set_icon(icon)
        .context("failed to bind the bundled icon to the main Windows window")
}

#[cfg(not(target_os = "windows"))]
fn setup_main_window_icon(_app: &tauri::App) -> anyhow::Result<()> {
    Ok(())
}

fn show_main_window(
    app: &tauri::AppHandle,
    diagnostics: Option<&StartupDiagnostics>,
    reason: MainWindowOpenReason,
) {
    if app_is_quitting(app) {
        if let Some(diagnostics) = diagnostics {
            diagnostics.info(format!(
                "skipping main window open because app is quitting: reason={}",
                reason.as_str()
            ));
        }
        return;
    }
    if let Some(diagnostics) = diagnostics {
        diagnostics.info(format!(
            "main window open requested: reason={}",
            reason.as_str()
        ));
    }
    let Some(window) = app.get_webview_window("main") else {
        if let Some(diagnostics) = diagnostics {
            diagnostics.warn(format!(
                "main window is unavailable during open request: reason={}",
                reason.as_str()
            ));
        }
        return;
    };
    normalize_main_window_layout(&window, diagnostics, WindowLayoutPolicy::Full);
    if window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) {
        if let Some(diagnostics) = diagnostics {
            diagnostics.info(format!(
                "main window open skipped because it is already visible and focused: reason={}",
                reason.as_str()
            ));
        }
        return;
    }
    show_dock_icon(app, diagnostics);
    restore_main_window(&window, diagnostics);
    if let Some(diagnostics) = diagnostics {
        diagnostics.info(format!(
            "main window open completed: reason={} visible={} focused={}",
            reason.as_str(),
            window.is_visible().unwrap_or(false),
            window.is_focused().unwrap_or(false)
        ));
    }
}

fn normalize_main_window_layout(
    window: &tauri::WebviewWindow,
    diagnostics: Option<&StartupDiagnostics>,
    policy: WindowLayoutPolicy,
) {
    match fit_main_window_to_work_area(window, policy) {
        Ok(outcome @ WindowLayoutOutcome::Applied { .. }) => {
            if let Some(diagnostics) = diagnostics {
                diagnostics.info(format!("main window work-area normalization {outcome}"));
            }
        }
        Ok(_) => {}
        Err(err) => {
            if let Some(diagnostics) = diagnostics {
                diagnostics.warn(format!(
                    "failed to normalize main window to the monitor work area: {err:#}"
                ));
            }
        }
    }
}

fn app_is_quitting(app: &tauri::AppHandle) -> bool {
    app.try_state::<DesktopState>()
        .is_some_and(|state| state.quitting.load(Ordering::SeqCst))
}

fn restore_main_window(window: &tauri::WebviewWindow, diagnostics: Option<&StartupDiagnostics>) {
    run_window_action(
        diagnostics,
        "show main window",
        "main window show completed",
        || window.show(),
    );
    run_window_action(
        diagnostics,
        "unminimize main window",
        "main window unminimize completed",
        || window.unminimize(),
    );
    run_window_action(
        diagnostics,
        "focus main window",
        "main window focus completed",
        || window.set_focus(),
    );
    if window.is_visible().unwrap_or(false) {
        if let Some(state) = window.app_handle().try_state::<DesktopState>() {
            state.main_window_visible.store(true, Ordering::SeqCst);
            state.runtime_log_streaming.store(
                state.runtime_log_streaming_requested.load(Ordering::SeqCst),
                Ordering::SeqCst,
            );
        }
        let _ = window.app_handle().emit(MAIN_WINDOW_VISIBILITY_EVENT, true);
    }
}

fn run_window_action(
    diagnostics: Option<&StartupDiagnostics>,
    label: &str,
    success_message: &str,
    action: impl FnOnce() -> tauri::Result<()>,
) {
    match panic::catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(())) => {
            if let Some(diagnostics) = diagnostics {
                diagnostics.info(success_message);
            }
        }
        Ok(Err(err)) => {
            if let Some(diagnostics) = diagnostics {
                diagnostics.error(format!("failed to {label}: {err:#}"));
            }
            eprintln!("failed to {label}: {err}");
        }
        Err(_) => {
            if let Some(diagnostics) = diagnostics {
                diagnostics.error(format!("{label} panicked; skipping"));
            }
            eprintln!("{label} panicked; skipping");
        }
    }
}

fn hide_to_tray(window: &tauri::Window) {
    if let Some(state) = window.app_handle().try_state::<DesktopState>() {
        state.main_window_visible.store(false, Ordering::SeqCst);
        state.runtime_log_streaming.store(false, Ordering::SeqCst);
    }
    let _ = window
        .app_handle()
        .emit(MAIN_WINDOW_VISIBILITY_EVENT, false);
    if let Err(err) = window.hide() {
        eprintln!("failed to hide main window: {err}");
    }
    hide_dock_icon(window.app_handle());
}

fn prepare_background_startup(app: &tauri::AppHandle, diagnostics: &StartupDiagnostics) {
    if let Some(state) = app.try_state::<DesktopState>() {
        state.main_window_visible.store(false, Ordering::SeqCst);
        state.runtime_log_streaming.store(false, Ordering::SeqCst);
    }
    if let Some(window) = app.get_webview_window("main") {
        run_window_action(
            Some(diagnostics),
            "hide main window for background autostart",
            "background autostart main window hide completed",
            || window.hide(),
        );
    } else {
        diagnostics.warn("main window is unavailable while preparing background autostart");
    }
    hide_dock_icon(app);
    let _ = app.emit(MAIN_WINDOW_VISIBILITY_EVENT, false);
    diagnostics.info("background autostart prepared without showing or focusing the main window");
}

#[cfg(target_os = "macos")]
fn show_dock_icon(app: &tauri::AppHandle, diagnostics: Option<&StartupDiagnostics>) {
    if let Err(err) = app.set_dock_visibility(true) {
        if let Some(diagnostics) = diagnostics {
            diagnostics.error(format!("failed to show dock icon: {err:#}"));
        }
        eprintln!("failed to show dock icon: {err}");
    } else if let Some(diagnostics) = diagnostics {
        diagnostics.info("dock icon show completed");
    }
}

#[cfg(not(target_os = "macos"))]
fn show_dock_icon(_app: &tauri::AppHandle, _diagnostics: Option<&StartupDiagnostics>) {}

#[cfg(target_os = "macos")]
fn hide_dock_icon(app: &tauri::AppHandle) {
    if let Err(err) = app.set_dock_visibility(false) {
        eprintln!("failed to hide dock icon: {err}");
    }
}

#[cfg(not(target_os = "macos"))]
fn hide_dock_icon(_app: &tauri::AppHandle) {}

fn quit_app(app: &tauri::AppHandle) {
    let state = app.state::<DesktopState>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        eprintln!("quit requested while app is already quitting");
        return;
    }
    let runtime = state.runtime.clone();
    let connector_processes = state.connector_processes.clone();
    let config_path = state.config_path.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        for failure in connector_processes.stop_all(&config_path).await {
            eprintln!("failed to stop managed connector before exit: {failure}");
        }
        if let Err(err) = runtime.stop().await {
            eprintln!("failed to stop runtime before exit: {err:#}");
        }
        app.exit(0);
    });
}

fn quit_running_instance_requested(args: &[String]) -> bool {
    args.iter().any(|arg| arg == QUIT_RUNNING_INSTANCE_ARG)
}

fn background_autostart_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .any(|arg| arg.as_ref() == OsStr::new(AUTOSTART_BACKGROUND_ARG))
}

fn interactive_restart_marker_path(config_path: &Path) -> PathBuf {
    resolve_config_base_dir(config_path).join(INTERACTIVE_RESTART_MARKER_FILE_NAME)
}

fn request_interactive_restart(config_path: &Path) -> Result<(), String> {
    let marker_path = interactive_restart_marker_path(config_path);
    if let Some(parent) = marker_path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建客户端重启状态目录失败: {err}"))?;
    }
    fs::write(&marker_path, b"interactive\n")
        .map_err(|err| format!("写入客户端前台重启标记失败: {err}"))
}

fn consume_interactive_restart_request(config_path: &Path) -> Result<bool, String> {
    let marker_path = interactive_restart_marker_path(config_path);
    match fs::remove_file(&marker_path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(format!("读取客户端前台重启标记失败: {err}")),
    }
}

fn prepare_config_for_auto_start_with<F>(
    config_path: &Path,
    sync_installed_connectors: F,
) -> anyhow::Result<(AgentConfig, ConnectorSyncReport)>
where
    F: FnOnce(&Path) -> anyhow::Result<ConnectorSyncReport>,
{
    ensure_config_exists(config_path)?;
    let sync_report = sync_installed_connectors(config_path)?;
    let config = load_agent_config(config_path)?;
    Ok((config, sync_report))
}

fn prepare_config_for_auto_start(
    config_path: &Path,
) -> anyhow::Result<(AgentConfig, ConnectorSyncReport)> {
    prepare_config_for_auto_start_with(config_path, sync_installed_connectors_report)
}

fn auto_start_agent(
    runtime: AgentRuntimeManager,
    connector_lifecycles: ConnectorLifecycleManager,
    connector_processes: ConnectorProcessManager,
    config_path: PathBuf,
    startup_health: StartupHealthManager,
    diagnostics: StartupDiagnostics,
) {
    startup_health.set_component("agent_runtime", "Agent 运行时", "starting", None);
    tauri::async_runtime::spawn(async move {
        diagnostics.info(format!(
            "auto start preparing config at {}",
            config_path.display()
        ));
        let (config, connector_sync) = match prepare_config_for_auto_start(&config_path) {
            Ok(prepared) => prepared,
            Err(err) => {
                diagnostics.error(format!(
                    "failed to prepare bridge-agent config and installed connectors at {}: {err:#}",
                    config_path.display()
                ));
                startup_health.set_component(
                    "agent_runtime",
                    "Agent 运行时",
                    "degraded",
                    Some(format!("配置与本地应用同步失败: {err}")),
                );
                return;
            }
        };
        diagnostics.info(format!(
            "installed connector sync completed before runtime auto start: installed={} failures={}",
            connector_sync.summaries.len(),
            connector_sync.failures.len()
        ));
        let connector_sync_failure_detail = if connector_sync.failures.is_empty() {
            None
        } else {
            let detail = format_connector_sync_failures(&connector_sync.failures);
            diagnostics.warn(format!(
                "installed connector sync completed with failures before runtime auto start: {detail}"
            ));
            Some(detail)
        };
        if !config_is_authorized(&config) {
            diagnostics
                .info("bridge-agent runtime auto start skipped: device is not authorized yet");
            startup_health.set_component(
                "agent_runtime",
                "Agent 运行时",
                "ready",
                Some("设备尚未授权，未自动连接".to_string()),
            );
            return;
        }
        diagnostics.info("bridge-agent config loaded for auto start");
        diagnostics.info("automatic agent start requested");
        if let Err(err) = start_runtime_from_saved_config(&runtime, &config_path).await {
            diagnostics.error(format!(
                "failed to auto start bridge-agent runtime: {err:#}"
            ));
            startup_health.set_component(
                "agent_runtime",
                "Agent 运行时",
                "degraded",
                Some(err.to_string()),
            );
            return;
        } else {
            diagnostics.info("automatic agent start request completed");
            if let Some(detail) = connector_sync_failure_detail {
                startup_health.set_component(
                    "agent_runtime",
                    "Agent 运行时",
                    "degraded",
                    Some(detail),
                );
            } else {
                startup_health.set_component("agent_runtime", "Agent 运行时", "ready", None);
            }
        }

        let automatic_connector_ids = config
            .local_apps
            .iter()
            .filter(|app| bridge_agent::local_app_starts_automatically(app))
            .map(|app| app.connector_id.clone())
            .collect::<Vec<_>>();
        for connector_id in automatic_connector_ids {
            diagnostics.info(format!(
                "automatic connector start requested: connector_id={connector_id}"
            ));
            match start_connector_with_lifecycle(
                &connector_lifecycles,
                &connector_processes,
                &config_path,
                &connector_id,
                "自动启动应用",
            )
            .await
            {
                Ok(_) => diagnostics.info(format!(
                    "automatic connector start completed: connector_id={connector_id}"
                )),
                Err(err) => diagnostics.warn(format!(
                    "automatic connector start failed: connector_id={connector_id} error={err}"
                )),
            }
        }
        if let Err(err) = runtime.apply_capabilities_from_path(&config_path).await {
            diagnostics.warn(format!(
                "failed to refresh capabilities after automatic connector startup: {err:#}"
            ));
        }
    });
}

fn config_is_authorized(config: &AgentConfig) -> bool {
    config.platform.workspace_id.is_some() && !config.relay.token.trim().is_empty()
}

fn install_bundled_baijimu_cli(diagnostics: &StartupDiagnostics) -> anyhow::Result<()> {
    let source = bundled_baijimu_cli_path();
    let status = managed_tool::bootstrap_bundled(source.as_deref())?;
    let skill_path = codex_skill::install_bundled()?;
    diagnostics.info(format!(
        "managed baijimu CLI bootstrap completed: state={} version={} launcher={} codex_skill={}",
        status.state,
        status.installed_version.as_deref().unwrap_or("unknown"),
        status.launcher_path,
        skill_path.display()
    ));
    Ok(())
}

fn bootstrap_bundled_baijimu_cli(
    startup_health: StartupHealthManager,
    diagnostics: StartupDiagnostics,
) {
    startup_health.set_component("managed_cli", "Baijimu CLI", "starting", None);
    tauri::async_runtime::spawn_blocking(move || match install_bundled_baijimu_cli(&diagnostics) {
        Ok(()) => startup_health.set_component("managed_cli", "Baijimu CLI", "ready", None),
        Err(err) => {
            diagnostics.warn(format!(
                "failed to install bundled baijimu CLI; continuing without CLI install: {err:#}"
            ));
            startup_health.set_component(
                "managed_cli",
                "Baijimu CLI",
                "degraded",
                Some(err.to_string()),
            );
        }
    });
}

fn bundled_baijimu_cli_path() -> Option<PathBuf> {
    let binary_name = baijimu_cli_binary_name();
    let exe = std::env::current_exe().ok();
    let mut candidates = Vec::new();
    if let Some(exe) = exe.as_ref() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("resources").join("bin").join(binary_name));
            candidates.push(
                dir.join("..")
                    .join("Resources")
                    .join("resources")
                    .join("bin")
                    .join(binary_name),
            );
            candidates.push(
                dir.join("..")
                    .join("resources")
                    .join("bin")
                    .join(binary_name),
            );
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(
            cwd.join("src-tauri")
                .join("resources")
                .join("bin")
                .join(binary_name),
        );
        candidates.push(cwd.join("resources").join("bin").join(binary_name));
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn baijimu_cli_binary_name() -> &'static str {
    if cfg!(windows) {
        "baijimu.exe"
    } else {
        "baijimu"
    }
}

fn forward_runtime_events(
    app: tauri::AppHandle,
    runtime: AgentRuntimeManager,
    runtime_log_streaming: Arc<AtomicBool>,
) {
    let mut events = runtime.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(RuntimeEvent::SnapshotChanged(snapshot)) => {
                    let _ = app.emit(RUNTIME_SNAPSHOT_EVENT, snapshot);
                }
                Ok(RuntimeEvent::LogAppended(entry)) => {
                    if runtime_log_streaming.load(Ordering::SeqCst) {
                        let _ = app.emit(RUNTIME_LOG_EVENT, entry);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let snapshot = runtime.snapshot().await;
                    let _ = app.emit(RUNTIME_SNAPSHOT_EVENT, snapshot);
                    if runtime_log_streaming.load(Ordering::SeqCst) {
                        let logs = runtime.logs(200).await;
                        let _ = app.emit(RUNTIME_LOGS_SNAPSHOT_EVENT, logs);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn main() {
    let bootstrap_diagnostics = StartupDiagnostics::bootstrap();
    install_panic_diagnostics(bootstrap_diagnostics.clone());

    let crypto_provider_failure = install_rustls_crypto_provider().err().map(|err| {
        let detail = format!("failed to install rustls provider: {err:#}");
        bootstrap_diagnostics.error(&detail);
        detail
    });
    let (config_path, config_path_failure) = match default_config_path() {
        Ok(path) => (path, None),
        Err(err) => {
            let detail = format!("failed to determine default config path: {err:#}");
            bootstrap_diagnostics.error(&detail);
            (
                std::env::temp_dir()
                    .join("baijimu-recovery")
                    .join("agent-config.json"),
                Some(detail),
            )
        }
    };
    let diagnostics = StartupDiagnostics::for_config_path(&config_path);
    install_panic_diagnostics(diagnostics.clone());
    log_startup_environment(&diagnostics, &config_path);
    let macos_installation_required = macos_installation::required_for_current_executable();
    let process_args = std::env::args_os().collect::<Vec<_>>();
    let forced_safe_mode = process_args
        .iter()
        .any(|arg| arg == OsStr::new("--safe-mode"));
    let interactive_restart_requested = match consume_interactive_restart_request(&config_path) {
        Ok(requested) => requested,
        Err(err) => {
            diagnostics.warn(err);
            false
        }
    };
    let launch_mode = DesktopLaunchMode::from_args(
        process_args.iter().map(|arg| arg.as_os_str()),
        interactive_restart_requested,
    );
    diagnostics.info(format!(
        "desktop launch mode resolved: mode={} interactive_restart_requested={interactive_restart_requested}",
        match launch_mode {
            DesktopLaunchMode::Interactive => "interactive",
            DesktopLaunchMode::BackgroundAutostart => "background_autostart",
        }
    ));
    // The single-instance plugin runs before the application setup callback. Keep startup
    // health persistence deferred until setup so a secondary process can notify the primary
    // instance and exit without turning a healthy startup into a recorded failure.
    let startup_health = StartupHealthManager::new(&config_path, diagnostics.clone());

    let runtime = AgentRuntimeManager::new();
    let connector_lifecycles = ConnectorLifecycleManager::default();
    let connector_processes = ConnectorProcessManager::default();
    let (registered_service_request_tx, registered_service_request_rx) =
        tokio::sync::mpsc::unbounded_channel();
    let registered_services = RegisteredServiceMonitor {
        request_tx: registered_service_request_tx,
    };
    let local_apps = LocalAppsChangeNotifier::default();
    let local_app_install_tasks = LocalAppInstallTaskManager::default();
    let runtime_log_streaming_requested = Arc::new(AtomicBool::new(false));
    let runtime_log_streaming = Arc::new(AtomicBool::new(false));
    let main_window_visible = Arc::new(AtomicBool::new(false));
    let quitting = Arc::new(AtomicBool::new(false));
    let local_app_ui = Arc::new(RwLock::new(None));
    let single_instance_diagnostics = diagnostics.clone();
    let setup_diagnostics = diagnostics.clone();
    let page_load_diagnostics = diagnostics.clone();
    let window_event_diagnostics = diagnostics.clone();
    let setup_health = startup_health.clone();
    let setup_local_app_ui = Arc::clone(&local_app_ui);
    let setup_local_apps = local_apps.clone();
    let setup_local_app_install_tasks = local_app_install_tasks.clone();
    let setup_connector_lifecycles = connector_lifecycles.clone();
    let setup_connector_processes = connector_processes.clone();
    let page_load_runtime_log_streaming_requested = Arc::clone(&runtime_log_streaming_requested);
    let page_load_runtime_log_streaming = Arc::clone(&runtime_log_streaming);
    tauri::Builder::default()
        // Single Instance must be registered first. With its `deep-link` feature enabled it
        // forwards protocol URLs from secondary launches to the primary process before exit.
        .plugin(tauri_plugin_single_instance::init(
            move |app, argv, _cwd| {
                let diagnostics = single_instance_diagnostics.clone();
                if quit_running_instance_requested(&argv) {
                    diagnostics.info("quit requested by a secondary desktop process");
                    quit_app(app);
                    return;
                }
                if background_autostart_requested(argv.iter().map(String::as_str)) {
                    diagnostics.info(
                        "background autostart reached an existing instance; keeping main window state unchanged",
                    );
                    return;
                }
                show_main_window(
                    app,
                    Some(&diagnostics),
                    MainWindowOpenReason::SecondaryLaunch,
                );
            },
        ))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .max_file_size(5 * 1024 * 1024)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .build(),
        )
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("BaijimuBridgeAgent")
                .arg(AUTOSTART_BACKGROUND_ARG)
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(desktop_window_state_flags())
                .build(),
        )
        .manage(DesktopState {
            runtime: runtime.clone(),
            connector_lifecycles: connector_lifecycles.clone(),
            connector_processes: connector_processes.clone(),
            config_path: config_path.clone(),
            quitting: Arc::clone(&quitting),
            local_app_ui,
            local_apps: local_apps.clone(),
            local_app_install_tasks: local_app_install_tasks.clone(),
            startup_health: startup_health.clone(),
            registered_services: registered_services.clone(),
            runtime_log_streaming_requested: Arc::clone(&runtime_log_streaming_requested),
            runtime_log_streaming: Arc::clone(&runtime_log_streaming),
            main_window_visible,
        })
        .on_page_load(move |webview, payload| {
            if webview.label() == "main"
                && payload.event() == tauri::webview::PageLoadEvent::Started
            {
                page_load_runtime_log_streaming_requested.store(false, Ordering::SeqCst);
                page_load_runtime_log_streaming.store(false, Ordering::SeqCst);
            }
            page_load_diagnostics.info(format!(
                "webview page load {:?}: label={} url={}",
                payload.event(),
                webview.label(),
                payload.url()
            ));
        })
        .setup(move |app| {
            setup_diagnostics.info("tauri setup started");
            if macos_installation_required {
                setup_diagnostics.warn(
                    "macOS app is running outside an Applications directory; showing the installation reminder and stopping startup",
                );
                macos_installation::show_reminder(app.handle());
                return Ok(());
            }
            setup_health.begin_primary(forced_safe_mode, config_path_failure);
            if let Some(detail) = crypto_provider_failure {
                setup_health.set_component(
                    "crypto_provider",
                    "网络加密组件",
                    "degraded",
                    Some(detail),
                );
            } else {
                setup_health.set_component("crypto_provider", "网络加密组件", "ready", None);
            }
            setup_health.attach_event_app(app.handle().clone());
            setup_local_apps.attach_event_app(app.handle().clone());
            setup_local_app_install_tasks.attach_event_app(app.handle().clone());
            setup_connector_lifecycles.attach_event_app(app.handle().clone());
            forward_runtime_events(
                app.handle().clone(),
                runtime.clone(),
                Arc::clone(&runtime_log_streaming),
            );
            start_registered_service_monitor(
                app.handle().clone(),
                config_path.clone(),
                setup_connector_lifecycles.clone(),
                setup_connector_processes.clone(),
                registered_service_request_rx,
            );
            #[cfg(debug_assertions)]
            if std::env::var_os("BRIDGE_AGENT_OPEN_DEVTOOLS").is_some() {
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }
            setup_health.set_component("updater", "官方更新器", "ready", None);
            configure_desktop_autostart(app, &setup_health, &setup_diagnostics);
            if let Err(err) = setup_main_window_icon(app) {
                setup_diagnostics.error(format!(
                    "failed to setup the main window icon; Windows may show a generic taskbar icon: {err:#}"
                ));
                setup_health.set_component(
                    "window_icon",
                    "应用图标",
                    "degraded",
                    Some(err.to_string()),
                );
            } else {
                setup_diagnostics.info("main window icon setup completed");
                setup_health.set_component("window_icon", "应用图标", "ready", None);
            }
            if let Err(err) = setup_tray(app, &setup_diagnostics) {
                setup_diagnostics.error(format!(
                    "failed to setup tray; continuing without tray icon: {err:#}"
                ));
                setup_health.set_component("tray", "系统托盘", "degraded", Some(err.to_string()));
            } else {
                setup_health.set_component("tray", "系统托盘", "ready", None);
            }
            if launch_mode == DesktopLaunchMode::BackgroundAutostart {
                prepare_background_startup(app.handle(), &setup_diagnostics);
            }
            if setup_health.safe_mode() {
                for (id, label) in [
                    ("local_app_ui_server", "本地应用界面服务"),
                    ("managed_cli", "Baijimu CLI"),
                    ("agent_runtime", "Agent 运行时"),
                ] {
                    setup_health.set_component(
                        id,
                        label,
                        "skipped",
                        Some("安全模式下未自动启动".to_string()),
                    );
                }
            } else {
                start_local_app_ui_server(
                    Arc::clone(&setup_local_app_ui),
                    setup_health.clone(),
                    LocalAppUiServerDependencies {
                        diagnostics: setup_diagnostics.clone(),
                        config_path: config_path.clone(),
                        runtime: runtime.clone(),
                        connector_lifecycles: setup_connector_lifecycles.clone(),
                        connector_processes: setup_connector_processes.clone(),
                        registered_services: registered_services.clone(),
                        local_apps: setup_local_apps.clone(),
                    },
                );
                bootstrap_bundled_baijimu_cli(setup_health.clone(), setup_diagnostics.clone());
                auto_start_agent(
                    runtime.clone(),
                    setup_connector_lifecycles.clone(),
                    setup_connector_processes.clone(),
                    config_path.clone(),
                    setup_health.clone(),
                    setup_diagnostics.clone(),
                );
            }
            setup_diagnostics.info("tauri setup completed");
            Ok(())
        })
        .on_window_event(move |window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if quitting.load(Ordering::SeqCst) {
                    return;
                }
                api.prevent_close();
                hide_to_tray(window);
            }
            if window.label() == "main" {
                let Some(webview_window) = window.app_handle().get_webview_window("main") else {
                    return;
                };
                match event {
                    WindowEvent::ScaleFactorChanged { .. } => normalize_main_window_layout(
                        &webview_window,
                        Some(&window_event_diagnostics),
                        WindowLayoutPolicy::Full,
                    ),
                    WindowEvent::Resized(_) => normalize_main_window_layout(
                        &webview_window,
                        Some(&window_event_diagnostics),
                        WindowLayoutPolicy::OversizedOnly,
                    ),
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            python_runtime_status,
            save_config,
            save_service,
            delete_service,
            start_agent,
            stop_agent,
            stop_conflicting_runtime,
            runtime_snapshot,
            apply_saved_config_to_runtime,
            test_capability,
            test_local_app_capability,
            list_logs,
            set_runtime_log_streaming,
            clear_logs,
            reset_example_config,
            recover_invalid_config,
            open_in_browser,
            open_in_edge,
            desktop_permission_status,
            registered_service_statuses,
            local_app_runtime_statuses,
            connector_lifecycle_snapshots,
            start_registered_service,
            stop_registered_service,
            list_connector_apps,
            connector_app_ui_url,
            list_market_connector_apps,
            show_connector_app,
            invoke_connector_management,
            check_connector_app_update,
            install_connector_app,
            start_connector_app_install,
            list_connector_app_install_tasks,
            start_connector_app,
            stop_connector_app,
            uninstall_connector_app,
            request_desktop_permission,
            open_desktop_permission_settings,
            start_browser_auth,
            poll_browser_auth,
            app_version,
            open_app_uninstaller,
            get_startup_health,
            mark_frontend_ready,
            restart_in_normal_mode,
            open_startup_log,
            check_app_update,
            install_app_update,
            baijimu_cli_status,
            install_baijimu_cli_update,
            rollback_baijimu_cli
        ])
        .build(tauri::generate_context!())
        .unwrap_or_else(|err| {
            diagnostics.error(format!("error while building tauri application: {err:#}"));
            std::process::exit(1);
        })
        .run(move |app, event| match event {
            tauri::RunEvent::Ready => {
                diagnostics.info("tauri runtime ready");
                if macos_installation_required {
                    diagnostics.info(
                        "macOS installation reminder is active; suppressing the main window",
                    );
                    return;
                }
                startup_health.set_component(
                    "desktop_shell",
                    "桌面基础壳",
                    "starting",
                    Some("等待前端就绪确认".to_string()),
                );
                if launch_mode.should_show_main_window() {
                    show_main_window(
                        app,
                        Some(&diagnostics),
                        MainWindowOpenReason::InteractiveStartup,
                    );
                } else {
                    diagnostics.info(
                        "background autostart runtime ready; main window remains hidden and unfocused",
                    );
                }
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen {
                has_visible_windows,
                ..
            } => {
                diagnostics.info(format!(
                    "tauri reopen event received: has_visible_windows={has_visible_windows}"
                ));
                if should_restore_main_window_on_macos_reopen(has_visible_windows) {
                    show_main_window(
                        app,
                        Some(&diagnostics),
                        MainWindowOpenReason::MacosReopen,
                    );
                } else {
                    diagnostics.info(
                        "macOS already has a visible main window; skipping forced refocus",
                    );
                }
            }
            tauri::RunEvent::ExitRequested { api, .. } => {
                let state = app.state::<DesktopState>();
                if !state.quitting.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    quit_app(app);
                }
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_agent::ConnectorLifecycleResult;

    fn market_connector(checksum: Option<&str>) -> MarketConnectorApp {
        MarketConnectorApp {
            id: "market-app-1".to_string(),
            connector_id: "com.baijimu.connector.test".to_string(),
            application_type: "connector".to_string(),
            name: "Test Connector".to_string(),
            description: String::new(),
            source: "https://downloads.example.test/connector.zip".to_string(),
            checksum: checksum.map(str::to_string),
            archive_path: None,
            risk: String::new(),
            risk_level: "medium".to_string(),
            capability: String::new(),
            version: "1.0.0".to_string(),
            published_at: None,
            icon_data_url: None,
            release_notes: Vec::new(),
            configuration_declaration: "undeclared".to_string(),
            interface_declaration: "undeclared".to_string(),
            database_declaration: "undeclared".to_string(),
            config_schema: None,
            database: None,
            methods: Vec::new(),
            events: Vec::new(),
            method_names: Vec::new(),
            event_names: Vec::new(),
            permissions: Vec::new(),
            compatible: true,
            compatibility_message: None,
            minimum_host_version: None,
            required_host_capabilities: Vec::new(),
            missing_host_capabilities: Vec::new(),
        }
    }

    #[test]
    fn local_app_install_tasks_track_progress_and_reject_duplicate_active_installs() {
        let manager = LocalAppInstallTaskManager::default();
        let task = manager
            .create(
                Some("com.baijimu.connector.codex".to_string()),
                Some("codex".to_string()),
                "Codex".to_string(),
                Some("1.2.1".to_string()),
            )
            .unwrap();
        assert_eq!(task.phase, LocalAppInstallTaskPhase::Queued);
        assert!(manager
            .create(
                Some("com.baijimu.connector.codex".to_string()),
                Some("codex".to_string()),
                "Codex".to_string(),
                Some("1.2.1".to_string()),
            )
            .unwrap_err()
            .contains("已在安装中"));

        let reporter = LocalAppInstallProgressReporter {
            manager: manager.clone(),
            task_id: task.task_id.clone(),
        };
        reporter.download(50, Some(100));
        let downloading = manager.list().pop().unwrap();
        assert_eq!(downloading.phase, LocalAppInstallTaskPhase::Downloading);
        assert_eq!(downloading.progress_percent, Some(32));
        assert_eq!(downloading.downloaded_bytes, Some(50));
        assert_eq!(downloading.total_bytes, Some(100));

        manager.update(&task.task_id, |task| {
            task.phase = LocalAppInstallTaskPhase::Succeeded;
            task.progress_percent = Some(100);
        });
        assert!(manager
            .create(
                Some("com.baijimu.connector.codex".to_string()),
                Some("codex".to_string()),
                "Codex".to_string(),
                Some("1.2.1".to_string()),
            )
            .is_ok());
    }

    #[test]
    fn local_app_install_progress_formats_download_sizes() {
        assert_eq!(format_byte_count(512), "512 B");
        assert_eq!(format_byte_count(1536), "1.5 KB");
        assert_eq!(format_byte_count(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn connector_uninstall_errors_preserve_force_eligibility_for_the_frontend() {
        let stop_failed = serde_json::to_value(ConnectorUninstallCommandError::StopFailed {
            message: "stop failed".to_string(),
        })
        .unwrap();
        let uninstall_failed = serde_json::to_value(ConnectorUninstallCommandError::Failed {
            message: "directory locked".to_string(),
        })
        .unwrap();

        assert_eq!(
            stop_failed["code"],
            serde_json::json!("connector_uninstall_stop_failed")
        );
        assert_eq!(
            uninstall_failed["code"],
            serde_json::json!("connector_uninstall_failed")
        );
    }

    #[test]
    fn registered_desktop_commands_exactly_match_main_acl() {
        let backend = include_str!("main.rs");
        let permissions = include_str!("../permissions/main.toml");
        let handler_section = backend
            .split_once("tauri::generate_handler![")
            .and_then(|(_, rest)| rest.split_once("])"))
            .map(|(section, _)| section)
            .expect("desktop backend must register a Tauri command handler");
        let allow_section = permissions
            .split_once("commands.allow = [")
            .and_then(|(_, rest)| rest.split_once(']'))
            .map(|(section, _)| section)
            .expect("main ACL must define commands.allow");
        let registered = handler_section
            .lines()
            .map(str::trim)
            .map(|line| line.trim_end_matches(','))
            .filter(|line| !line.is_empty())
            .collect::<std::collections::BTreeSet<_>>();
        let allowed = allow_section
            .lines()
            .map(str::trim)
            .map(|line| line.trim_end_matches(',').trim_matches('"'))
            .filter(|line| !line.is_empty())
            .collect::<std::collections::BTreeSet<_>>();
        let missing = registered.difference(&allowed).copied().collect::<Vec<_>>();
        let stale = allowed.difference(&registered).copied().collect::<Vec<_>>();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "desktop command ACL drift: missing=[{}], stale=[{}]",
            missing.join(", "),
            stale.join(", ")
        );
    }

    #[test]
    fn market_connector_trust_requires_checksum_and_matching_identity() {
        let valid = market_connector(Some(&"a".repeat(64)));
        assert_eq!(
            required_market_checksum(&valid).unwrap(),
            format!("sha256:{}", "a".repeat(64))
        );
        assert!(validate_market_connector_identity(&valid, "com.baijimu.connector.test").is_ok());

        assert!(required_market_checksum(&market_connector(None)).is_err());
        assert!(required_market_checksum(&market_connector(Some("invalid"))).is_err());
        assert!(validate_market_connector_identity(&valid, "com.example.other").is_err());

        let mut insecure = valid;
        insecure.source = "http://downloads.example.test/connector.zip".to_string();
        assert!(
            validate_market_connector_identity(&insecure, "com.baijimu.connector.test").is_err()
        );
    }

    #[test]
    fn market_manifest_exposes_release_notes_and_update_shape() {
        let manifest = serde_json::json!({
            "releaseNotes": ["新增文件发送", "修复重连", ""],
            "configSchema": {"type": "object", "required": ["token"], "properties": {"token": {"type": "string"}}},
            "upgradeReview": {"configuration": "declared", "interfaces": "declared", "database": "declared"},
            "methods": [{"name": "message.send", "path": "/send", "httpMethod": "POST", "input_schema": {"type": "object"}}, {"name": "file.send"}],
            "events": [{"name": "message.received", "payload_schema": {"type": "object"}}],
            "database": {
                "engine": "sqlite",
                "schemaVersion": "2",
                "migrations": [{
                    "id": "002-add-status",
                    "fromVersion": "1",
                    "toVersion": "2",
                    "description": "新增状态字段",
                    "changes": [{"operation": "add_column", "target": "messages.status", "description": "新增状态", "destructive": false}],
                    "destructive": false,
                    "rollback": "automatic",
                    "downtime": "none"
                }]
            },
            "permissions": [{
                "id": "filesystem",
                "title": "文件读取",
                "description": "选择文件后读取内容",
                "platforms": ["macos"]
            }]
        });

        assert_eq!(
            market_release_notes(&manifest),
            vec!["新增文件发送".to_string(), "修复重连".to_string()]
        );
        let methods = market_manifest_method_contracts(&manifest);
        assert_eq!(
            methods
                .iter()
                .map(|method| method.name.as_str())
                .collect::<Vec<_>>(),
            vec!["message.send", "file.send"]
        );
        assert_eq!(methods[0].path, "/send");
        assert_eq!(
            market_manifest_event_contracts(&manifest)[0].name,
            "message.received"
        );
        let database = market_manifest_database(&manifest).unwrap();
        assert_eq!(database.schema_version, "2");
        assert_eq!(database.migrations[0].changes[0].target, "messages.status");
        assert_eq!(
            market_contract_declaration(&manifest, "database", true, "connector"),
            "declared"
        );
        assert_eq!(
            market_contract_declaration(&serde_json::json!({}), "database", false, "managed_tool"),
            "not_applicable"
        );
        assert_eq!(market_manifest_permissions(&manifest)[0].id, "filesystem");
    }

    #[test]
    fn market_manifest_accepts_multiline_legacy_release_notes() {
        let manifest = serde_json::json!({
            "changelog": "- 新增能力\n* 修复问题\n\n"
        });
        assert_eq!(
            market_release_notes(&manifest),
            vec!["新增能力".to_string(), "修复问题".to_string()]
        );
    }

    #[test]
    fn market_host_compatibility_checks_version_and_capabilities() {
        let setup = serde_json::json!({
            "hostRequirements": {
                "minimumVersion": env!("CARGO_PKG_VERSION"),
                "capabilities": ["connector.setup.v1"]
            }
        });
        assert!(market_host_compatibility(&setup, None).compatible);

        let future = serde_json::json!({
            "hostRequirements": {"minimumVersion": "99.0.0"}
        });
        let incompatible = market_host_compatibility(&future, None);
        assert!(!incompatible.compatible);
        assert!(incompatible.message.unwrap().contains("请先升级客户端"));

        let missing = serde_json::json!({
            "hostRequirements": {"capabilities": ["connector.unknown.v1"]}
        });
        assert!(!market_host_compatibility(&missing, None).compatible);
    }

    #[test]
    fn config_for_ui_redacts_relay_credentials_and_reports_status() {
        let mut config = AgentConfig::example();
        config.relay.token = "relay-secret".to_string();

        let value = config_for_ui(&config).unwrap();

        assert_eq!(value["relay"]["token"], "");
        assert_eq!(value["credential_status"]["relay_token_configured"], true);
        assert!(!value.to_string().contains("relay-secret"));
    }

    fn registered_status(
        status: RegisteredServiceState,
        checked_at_ms: u64,
    ) -> RegisteredServiceStatus {
        RegisteredServiceStatus {
            service: "local-app".to_string(),
            status,
            detail: None,
            checked_at_ms,
            health_check_configured: true,
            start_command_configured: true,
            stop_command_configured: true,
        }
    }

    #[test]
    fn registered_service_monitor_emits_only_meaningful_changes() {
        let previous = vec![registered_status(RegisteredServiceState::Healthy, 100)];
        let refreshed = vec![registered_status(RegisteredServiceState::Healthy, 200)];
        let unhealthy = vec![registered_status(RegisteredServiceState::Unhealthy, 300)];

        assert!(!registered_service_statuses_changed(&previous, &refreshed));
        assert!(registered_service_statuses_changed(&previous, &unhealthy));
    }

    fn local_app_status(
        status: RegisteredServiceState,
        process_running: Option<bool>,
    ) -> LocalAppRuntimeStatus {
        LocalAppRuntimeStatus {
            connector_id: "com.baijimu.connector.test".to_string(),
            status,
            detail: None,
            checked_at_ms: 100,
            health_check_configured: false,
            start_command_configured: true,
            stop_command_configured: true,
            process_managed: process_running.is_some(),
            process_running,
        }
    }

    #[test]
    fn inactive_connector_status_is_derived_without_a_health_probe() {
        let app = LocalAppConfig {
            connector_id: "com.baijimu.connector.inactive".to_string(),
            name: "Inactive Connector".to_string(),
            version: "1.0.0".to_string(),
            description: String::new(),
            enabled: true,
            health_check: Some(ServiceHealthCheck::Http {
                url: "http://127.0.0.1:9/health".to_string(),
                http_method: "GET".to_string(),
                headers: BTreeMap::new(),
                timeout_secs: Some(60),
                expect_status: Some(200),
                body_contains: None,
            }),
            start_command: None,
            stop_command: None,
            methods: Vec::new(),
            events: Vec::new(),
        };

        let status = inactive_local_app_status(app, None);

        assert_eq!(status.connector_id, "com.baijimu.connector.inactive");
        assert_eq!(status.status, RegisteredServiceState::Unhealthy);
        assert_eq!(
            status.detail.as_deref(),
            Some("应用尚未由 Bridge Agent 启动")
        );
        assert!(status.health_check_configured);
        assert!(!status.process_managed);
        assert_eq!(status.process_running, None);
    }

    #[test]
    fn host_managed_process_state_is_authoritative_without_health_check() {
        let mut running = registered_status(RegisteredServiceState::NotConfigured, 100);
        running.health_check_configured = false;
        apply_managed_process_status(&mut running, Some(true));
        assert_eq!(running.status, RegisteredServiceState::Healthy);
        assert_eq!(running.detail.as_deref(), Some("宿主管理进程正在运行"));

        let mut stopped = registered_status(RegisteredServiceState::NotConfigured, 100);
        stopped.health_check_configured = false;
        apply_managed_process_status(&mut stopped, Some(false));
        assert_eq!(stopped.status, RegisteredServiceState::Unhealthy);
        assert_eq!(stopped.detail.as_deref(), Some("宿主管理进程未运行"));
    }

    #[test]
    fn health_check_remains_authoritative_when_configured() {
        let mut unhealthy = registered_status(RegisteredServiceState::Unhealthy, 100);
        unhealthy.detail = Some("health HTTP 503".to_string());
        apply_managed_process_status(&mut unhealthy, Some(true));
        assert_eq!(unhealthy.status, RegisteredServiceState::Unhealthy);
        assert_eq!(unhealthy.detail.as_deref(), Some("health HTTP 503"));
    }

    #[test]
    fn health_error_detail_preserves_connector_readiness_root_cause() {
        let body = serde_json::to_vec(&serde_json::json!({
            "ok": false,
            "status": {
                "startup": {
                    "status": "failed",
                    "error": "同步用户级 CODEX_HOME 失败：Windows 环境广播超时"
                }
            },
            "error": {
                "code": "connector_initialization_failed",
                "message": "同步用户级 CODEX_HOME 失败：Windows 环境广播超时"
            }
        }))
        .unwrap();

        assert_eq!(
            format_health_http_error(503, 200, &body),
            "health HTTP 503，期望 200：同步用户级 CODEX_HOME 失败：Windows 环境广播超时"
        );
    }

    #[test]
    fn health_error_detail_ignores_unstructured_or_secret_fields() {
        let body = serde_json::to_vec(&serde_json::json!({
            "token": "must-not-be-rendered",
            "details": "arbitrary connector response"
        }))
        .unwrap();

        let detail = format_health_http_error(503, 200, &body);
        assert_eq!(detail, "health HTTP 503，期望 200");
        assert!(!detail.contains("must-not-be-rendered"));
    }

    #[test]
    fn health_error_detail_is_bounded_for_local_connector_responses() {
        let body = serde_json::to_vec(&serde_json::json!({
            "error": {"message": "x".repeat(HEALTH_ERROR_MESSAGE_MAX_CHARS + 100)}
        }))
        .unwrap();

        let detail = format_health_http_error(503, 200, &body);
        let rendered = detail.split_once('：').unwrap().1;
        assert_eq!(rendered.chars().count(), HEALTH_ERROR_MESSAGE_MAX_CHARS);
    }

    #[test]
    fn local_app_monitor_detects_process_lifecycle_changes() {
        let stopped = vec![local_app_status(
            RegisteredServiceState::Unhealthy,
            Some(false),
        )];
        let running = vec![local_app_status(
            RegisteredServiceState::Healthy,
            Some(true),
        )];
        assert!(local_app_runtime_statuses_changed(&stopped, &running));
        assert!(!local_app_runtime_statuses_changed(&running, &running));
    }

    #[test]
    fn local_app_change_notifications_have_monotonic_revisions_and_context() {
        let notifier = LocalAppsChangeNotifier::default();

        let installed = notifier.notify(
            LocalAppsChangeOperation::Install,
            "com.baijimu.connector.test",
        );
        let synced = notifier.notify(LocalAppsChangeOperation::Sync, "com.baijimu.connector.test");

        assert_eq!(installed.revision, 1);
        assert_eq!(installed.operation, LocalAppsChangeOperation::Install);
        assert_eq!(installed.connector_id, "com.baijimu.connector.test");
        assert_eq!(synced.revision, 2);
        assert_eq!(synced.operation, LocalAppsChangeOperation::Sync);
    }

    fn update_release_response(
        force_update: Option<bool>,
        minimum_supported_version: Option<&str>,
    ) -> UpdateReleaseResponse {
        UpdateReleaseResponse {
            tag_name: Some("bridge-agent-v0.1.72".to_string()),
            version: Some("0.1.72".to_string()),
            release_url: None,
            release_name: None,
            published_at: None,
            update_available: None,
            force_update,
            minimum_supported_version: minimum_supported_version.map(str::to_string),
            force_update_message: None,
            assets: Vec::new(),
        }
    }

    #[test]
    fn force_update_required_should_follow_minimum_supported_version() {
        let release = update_release_response(None, Some("0.1.72"));

        assert!(release_force_update_required(
            &release,
            &Version::parse("0.1.71").unwrap()
        ));
        assert!(!release_force_update_required(
            &release,
            &Version::parse("0.1.72").unwrap()
        ));
    }

    #[test]
    fn force_update_flag_should_override_version_comparison() {
        let release = update_release_response(Some(true), Some("0.1.70"));

        assert!(release_force_update_required(
            &release,
            &Version::parse("0.1.72").unwrap()
        ));
    }

    #[test]
    fn updater_asset_selection_requires_a_signature() {
        if matches!(std::env::consts::OS, "windows" | "linux") && std::env::consts::ARCH != "x86_64"
        {
            return;
        }
        let suffix = match std::env::consts::OS {
            "macos" => ".app.tar.gz",
            "windows" => ".msi",
            "linux" => ".AppImage",
            _ => return,
        };
        let mut release = update_release_response(None, None);
        release.assets = vec![
            UpdateReleaseAsset {
                name: format!("unsigned{suffix}"),
                signature: None,
            },
            UpdateReleaseAsset {
                name: format!("signed{suffix}"),
                signature: Some("minisign-signature".to_string()),
            },
        ];

        let selected = select_tauri_updater_asset(&release).expect("signed updater asset");
        assert_eq!(selected.name, format!("signed{suffix}"));
    }

    #[test]
    fn shared_cli_auth_path_should_live_under_home_config() {
        let path = shared_cli_auth_path();

        assert!(path.ends_with(Path::new(".config").join("baijimu").join("auth.json")));
        assert!(path.is_absolute() || std::env::var_os("HOME").is_none());
    }

    #[test]
    fn shared_cli_auth_sets_authorized_workspace_as_current_and_preserves_other_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("auth.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "currentEnvironment": "prod",
                "currentWorkspaceId": 1201,
                "environments": {
                    "prod": {"baseUrl": "https://baijimu.com"}
                },
                "machineCredentials": [{
                    "workspaceId": 1201,
                    "clientId": "old-device",
                    "token": "lc_pat_old",
                    "tokenType": "workspace_user_api_key",
                    "issuedAtEpochSeconds": 1
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let config = AgentConfig::example();
        let authorized = AuthorizedPayload {
            workspace_id: 1082,
            device_id: "wenya".to_string(),
            relay_ws_url: "wss://relay.example.test".to_string(),
            agent_token: "agent-token".to_string(),
            issued_at_epoch_seconds: Some(1_786_205_925),
            expires_at_epoch_seconds: Some(i64::MAX as u64),
            local_client_token: Some("lc_pat_workspace_1082".to_string()),
            local_client_token_type: Some("workspace_user_api_key".to_string()),
            local_client_key_id: Some("key-1082".to_string()),
            local_client_user_id: Some(433),
            local_client_scopes: vec![
                "baijimu:agent-cli".to_string(),
                "partner:api".to_string(),
                "workspace:1082".to_string(),
            ],
            local_client_issued_at: Some("2026-07-29 10:00:00".to_string()),
            local_client_expires_at: Some("2026-10-27 10:00:00".to_string()),
        };

        let mut authorized_config = config.clone();
        apply_authorized_device_credentials(&mut authorized_config, &authorized);
        assert_eq!(authorized_config.platform.workspace_id, Some(1082));
        assert_eq!(authorized_config.relay.agent_id, "wenya");
        assert_eq!(authorized_config.relay.token, "agent-token");
        assert_eq!(
            authorized_config.relay.token_expires_at_epoch_seconds,
            Some((i64::MAX as u64).to_string())
        );

        write_shared_cli_auth_at(&path, &config, &authorized).unwrap();

        let document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(document["currentWorkspaceId"], 1082);
        assert_eq!(document["schemaVersion"], 2);
        assert!(document.get("machineCredentials").is_none());
        assert_eq!(document["credentials"].as_array().unwrap().len(), 2);
        assert_eq!(document["credentials"][0]["workspaceIds"][0], 1201);
        assert_eq!(document["credentials"][0]["tokenType"], "pat");
        assert_eq!(document["credentials"][1]["workspaceIds"][0], 1082);
        assert_eq!(document["credentials"][1]["userId"], 433);
        assert_eq!(document["credentials"][1]["source"], "bridge-agent");
        assert_eq!(
            document["credentials"][1]["expiresAt"],
            "2026-10-27 10:00:00"
        );
        assert!(document["credentials"][1]["issuedAtEpochSeconds"]
            .as_u64()
            .is_some());
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn market_git_source_converts_to_github_archive() {
        let archive = connector_archive_download_url(
            "https://github.com/momoplan/wechat-bridge-collector.git",
            Some("v0.2.3"),
            false,
        )
        .unwrap();
        assert_eq!(
            archive.as_deref(),
            Some("https://github.com/momoplan/wechat-bridge-collector/archive/v0.2.3.zip")
        );
    }

    #[test]
    fn custom_git_source_keeps_git_clone_path() {
        let archive = connector_archive_download_url(
            "https://github.com/momoplan/wechat-bridge-collector.git",
            Some("v0.2.3"),
            true,
        )
        .unwrap();
        assert!(archive.is_none());
    }

    #[test]
    fn archive_source_downloads_directly() {
        let archive = connector_archive_download_url(
            "https://download.baijimu.com/connectors/wechat.zip",
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            archive.as_deref(),
            Some("https://download.baijimu.com/connectors/wechat.zip")
        );
    }

    #[test]
    fn connector_archive_checksum_is_required_to_match_exact_bytes() {
        let checksum = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify_connector_archive_checksum(b"hello", Some(checksum)).is_ok());
        assert!(verify_connector_archive_checksum(b"changed", Some(checksum)).is_err());
        assert!(verify_connector_archive_checksum(b"hello", Some("invalid")).is_err());
    }

    #[test]
    fn connector_upgrade_requires_every_lifecycle_command_to_succeed() {
        let success = ConnectorStartResult {
            connector_id: "com.baijimu.connector.test".to_string(),
            lifecycle: ConnectorLifecycleResult {
                connector_id: "com.baijimu.connector.test".to_string(),
                configured: true,
                exit_code: Some(0),
                stdout: "started".to_string(),
                stderr: String::new(),
            },
        };
        assert!(ensure_connector_lifecycle_command_succeeded("启动新版应用", &success).is_ok());

        let failure = ConnectorStartResult {
            connector_id: "com.baijimu.connector.test".to_string(),
            lifecycle: ConnectorLifecycleResult {
                connector_id: "com.baijimu.connector.test".to_string(),
                configured: false,
                exit_code: None,
                stdout: String::new(),
                stderr: "stop command is not configured".to_string(),
            },
        };
        let error =
            ensure_connector_lifecycle_command_succeeded("停止旧版应用", &failure).unwrap_err();
        assert!(error.contains("命令未配置"));
        assert!(error.contains("com.baijimu.connector.test"));
    }

    #[test]
    fn local_app_ui_bridge_is_injected_before_head_closes() {
        let html = b"<!doctype html><html><head><title>Settings</title></head><body></body></html>"
            .to_vec();

        let injected = String::from_utf8(inject_local_app_ui_bridge(html).unwrap()).unwrap();

        let bridge_index = injected.find(LOCAL_APP_UI_BRIDGE_ASSET).unwrap();
        let head_end_index = injected.to_ascii_lowercase().find("</head>").unwrap();
        assert!(bridge_index < head_end_index);
        assert_eq!(injected.matches(LOCAL_APP_UI_BRIDGE_ASSET).count(), 1);
    }

    #[test]
    fn local_app_ui_bridge_reannounces_ready_after_host_hello() {
        assert!(LOCAL_APP_UI_BRIDGE_SCRIPT.contains("baijimu:local-app:hello"));
        assert!(LOCAL_APP_UI_BRIDGE_SCRIPT.contains("announceReady();"));
        assert!(LOCAL_APP_UI_BRIDGE_SCRIPT
            .contains("window.addEventListener(\"pageshow\", announceReady)"));
        assert!(LOCAL_APP_UI_BRIDGE_SCRIPT.contains("message.type === HELLO_TYPE"));
    }

    #[test]
    fn macos_bundle_allows_loopback_assets_inside_webview() {
        let info_plist = include_str!("../Info.plist");
        assert!(info_plist.contains("NSAppTransportSecurity"));
        assert!(info_plist.contains("NSAllowsArbitraryLoadsInWebContent"));
        assert!(info_plist.contains("<true/>"));
    }

    #[test]
    fn local_app_ui_response_disables_direct_network_access() {
        let response = local_app_ui_response(
            StatusCode::OK,
            "text/html; charset=utf-8",
            b"<html></html>".to_vec(),
        );

        let csp = response
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("connect-src 'none'"));
        assert!(csp.contains("frame-ancestors tauri://localhost http://tauri.localhost"));
        assert_eq!(
            response
                .headers()
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .unwrap(),
            "nosniff"
        );
    }

    #[test]
    fn local_app_ui_hosts_are_isolated_per_connector() {
        let token = "0123456789abcdef0123456789abcdef";
        let first = local_app_ui_host(token, "com.baijimu.connector.first");
        let second = local_app_ui_host(token, "com.baijimu.connector.second");
        assert_ne!(first, second);
        assert!(first.ends_with(".localhost"));

        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, format!("{first}:32123").parse().unwrap());
        assert!(local_app_ui_request_host_matches(
            &headers,
            token,
            "com.baijimu.connector.first"
        ));
        assert!(!local_app_ui_request_host_matches(
            &headers,
            token,
            "com.baijimu.connector.second"
        ));
    }

    #[test]
    fn local_app_control_discovery_is_private_and_loopback_only() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(LOCAL_APP_CONTROL_FILE_NAME);
        let token = "0123456789abcdef0123456789abcdef";

        write_local_app_control_discovery(&path, 39100, token).unwrap();

        let document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(document["schemaVersion"], 1);
        assert_eq!(document["baseUrl"], "http://127.0.0.1:39100/api/v1");
        assert_eq!(document["token"], token);
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn repeated_incomplete_startups_enable_safe_mode() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("agent-config.json");
        let state_path = directory.path().join(STARTUP_STATE_FILE_NAME);
        write_startup_state(
            &state_path,
            &PersistentStartupState {
                pending: true,
                consecutive_failures: SAFE_MODE_FAILURE_THRESHOLD - 1,
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                started_at_ms: Some(now_ms()),
                ready_at_ms: None,
            },
        )
        .unwrap();

        let health = StartupHealthManager::new(
            &config_path,
            StartupDiagnostics::for_config_path(&config_path),
        );
        health.begin_primary(false, None);

        assert!(health.safe_mode());
        assert_eq!(
            health.snapshot().consecutive_failures,
            SAFE_MODE_FAILURE_THRESHOLD
        );
    }

    #[test]
    fn secondary_instance_construction_does_not_mutate_startup_state() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("agent-config.json");
        let state_path = directory.path().join(STARTUP_STATE_FILE_NAME);
        let previous = PersistentStartupState {
            pending: true,
            consecutive_failures: 1,
            version: Some("0.2.9".to_string()),
            started_at_ms: Some(123),
            ready_at_ms: None,
        };
        write_startup_state(&state_path, &previous).unwrap();
        let before = fs::read(&state_path).unwrap();

        let health = StartupHealthManager::new(
            &config_path,
            StartupDiagnostics::for_config_path(&config_path),
        );

        assert!(!health.safe_mode());
        assert_eq!(health.snapshot().consecutive_failures, 0);
        assert_eq!(fs::read(&state_path).unwrap(), before);
    }

    #[test]
    fn desktop_is_the_release_autostart_owner_on_every_supported_platform() {
        assert_eq!(
            desktop_autostart_policy(false),
            DesktopAutostartPolicy::EnableForDesktop
        );
        assert_eq!(
            desktop_autostart_policy(true),
            DesktopAutostartPolicy::SkipDevelopmentBuild
        );
    }

    #[test]
    fn desktop_launch_mode_distinguishes_background_autostart_from_user_launches() {
        assert_eq!(
            DesktopLaunchMode::from_args(["bridge-agent-desktop", AUTOSTART_BACKGROUND_ARG], false,),
            DesktopLaunchMode::BackgroundAutostart
        );
        assert_eq!(
            DesktopLaunchMode::from_args(["bridge-agent-desktop"], false),
            DesktopLaunchMode::Interactive
        );
        assert_eq!(
            DesktopLaunchMode::from_args(["bridge-agent-desktop", AUTOSTART_BACKGROUND_ARG], true,),
            DesktopLaunchMode::Interactive
        );
        assert!(!DesktopLaunchMode::BackgroundAutostart.should_show_main_window());
        assert!(DesktopLaunchMode::Interactive.should_show_main_window());
    }

    #[test]
    fn background_secondary_launch_does_not_request_an_interactive_window() {
        assert!(background_autostart_requested([
            "bridge-agent-desktop",
            AUTOSTART_BACKGROUND_ARG,
        ]));
        assert!(!background_autostart_requested([
            "bridge-agent-desktop",
            "--safe-mode",
        ]));
    }

    #[test]
    fn desktop_window_state_never_restores_visibility_or_focus() {
        let flags = desktop_window_state_flags();
        assert!(!flags.contains(StateFlags::VISIBLE));
        assert!(flags.contains(StateFlags::SIZE));
        assert!(flags.contains(StateFlags::POSITION));

        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["app"]["windows"][0]["visible"], false);
        assert!(config["app"]["windows"][0].get("minWidth").is_none());
        assert!(config["app"]["windows"][0].get("minHeight").is_none());
    }

    #[test]
    fn macos_reopen_only_restores_a_missing_visible_window() {
        assert!(should_restore_main_window_on_macos_reopen(false));
        assert!(!should_restore_main_window_on_macos_reopen(true));
    }

    #[test]
    fn interactive_restart_marker_is_consumed_once() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("agent-config.json");

        assert!(!consume_interactive_restart_request(&config_path).unwrap());
        request_interactive_restart(&config_path).unwrap();
        assert!(consume_interactive_restart_request(&config_path).unwrap());
        assert!(!consume_interactive_restart_request(&config_path).unwrap());
    }

    #[test]
    fn auto_start_rebuilds_installed_connectors_before_loading_runtime_config() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("agent-config.json");
        let config = AgentConfig::example();
        save_agent_config(&config_path, &config).unwrap();
        assert!(load_agent_config(&config_path)
            .unwrap()
            .local_apps
            .is_empty());

        let (prepared, report) = prepare_config_for_auto_start_with(&config_path, |path| {
            let mut synchronized = load_agent_config(path)?;
            synchronized.local_apps.push(LocalAppConfig {
                connector_id: "com.baijimu.connector.persisted".to_string(),
                name: "Persisted Connector".to_string(),
                version: "1.0.0".to_string(),
                description: "Installed before the host upgrade".to_string(),
                enabled: true,
                health_check: None,
                start_command: None,
                stop_command: None,
                methods: Vec::new(),
                events: vec![bridge_agent::EventConfig {
                    name: "changed".to_string(),
                    description: "Connector state changed".to_string(),
                    enabled: true,
                    payload_schema: serde_json::json!({
                        "type": "object",
                        "additionalProperties": true
                    }),
                }],
            });
            save_agent_config(path, &synchronized)?;
            Ok(ConnectorSyncReport {
                summaries: Vec::new(),
                failures: Vec::new(),
            })
        })
        .unwrap();

        assert!(report.failures.is_empty());
        assert_eq!(prepared.local_apps.len(), 1);
        assert_eq!(
            prepared.local_apps[0].connector_id,
            "com.baijimu.connector.persisted"
        );
    }

    #[test]
    fn quit_running_instance_flag_is_explicit() {
        assert!(quit_running_instance_requested(&[
            "bridge-agent-desktop.exe".to_string(),
            QUIT_RUNNING_INSTANCE_ARG.to_string(),
        ]));
        assert!(!quit_running_instance_requested(&[
            "bridge-agent-desktop.exe".to_string(),
            "--safe-mode".to_string(),
        ]));
    }
}
