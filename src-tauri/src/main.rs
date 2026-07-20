#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod managed_tool;

use axum::{
    body::Body,
    extract::{Path as AxumPath, State as AxumState},
    http::{header, HeaderMap, Response as HttpResponse, StatusCode},
    response::Response as AxumResponse,
    routing::get,
    Router,
};
use bridge_agent::config::resolve_config_base_dir;
use bridge_agent::logging::LogMetadata;
use bridge_agent::protocol::InvokeResult;
use bridge_agent::services::ServiceRegistry;
use bridge_agent::{
    browser_auth_manifest_json, connector_management_token_path, default_config_path,
    ensure_browser_auth_agent_id, ensure_config_exists, format_connector_sync_failures,
    install_connector_from_path_with_source_reference, install_rustls_crypto_provider,
    list_connectors, load_config as load_agent_config, load_connector_manifest,
    manifest_preview_json, reset_invalid_config, resolve_connector_ui_asset,
    resolve_connector_ui_entry, save_config as save_agent_config, show_connector, start_connector,
    stop_connector, sync_installed_connectors_report, terminate_runtime_lock_owner,
    uninstall_connector, AgentConfig, AgentRuntimeManager, ConnectorInstallRecord,
    ConnectorInstallResult, ConnectorStartResult, ConnectorSummary, RuntimeLockConflict,
    RuntimeSnapshot, ServiceConfig, ServiceHealthCheck, ServiceStartCommand,
};
use reqwest::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt as _;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};
use tauri_plugin_updater::UpdaterExt;
use tokio::process::Command as AsyncCommand;
use tokio::time::timeout;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(target_os = "macos")]
use core_foundation::base::TCFType;
#[cfg(target_os = "macos")]
use core_foundation::boolean::CFBoolean;
#[cfg(target_os = "macos")]
use core_foundation::dictionary::CFDictionary;
#[cfg(target_os = "macos")]
use core_foundation::string::CFString;

const UPDATE_USER_AGENT: &str = concat!("bridge-agent-desktop/", env!("CARGO_PKG_VERSION"));
const UPDATE_PROGRESS_EVENT: &str = "app-update-progress";
const CONNECTOR_MANIFEST_FILE: &str = "connector.json";
const LOCAL_APP_UI_BRIDGE_ASSET: &str = "__baijimu_bridge.js";
const LOCAL_APP_UI_MAX_MANAGEMENT_PAYLOAD_BYTES: usize = 1024 * 1024;
const LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const TRAY_ID: &str = "bridge-agent";
const TRAY_MENU_SHOW: &str = "show";
const TRAY_MENU_QUIT: &str = "quit";
const STARTUP_LOG_FILE_NAME: &str = "bridge-agent-desktop-startup.log";
const STARTUP_STATE_FILE_NAME: &str = "bridge-agent-desktop-startup-state.json";
const SAFE_MODE_FAILURE_THRESHOLD: u32 = 2;
#[cfg(windows)]
const WINDOWS_CREATE_NO_WINDOW: u32 = 0x08000000;
#[cfg(windows)]
const WINDOWS_SERVICE_HANDOFF_RETRIES: usize = 40;
#[cfg(windows)]
const WINDOWS_SERVICE_HANDOFF_RETRY_DELAY: Duration = Duration::from_millis(250);

struct DesktopState {
    runtime: AgentRuntimeManager,
    config_path: PathBuf,
    quitting: Arc<AtomicBool>,
    local_app_ui: Arc<RwLock<Option<LocalAppUiEndpoint>>>,
    startup_health: StartupHealthManager,
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
    state_path: PathBuf,
    diagnostics: StartupDiagnostics,
}

impl StartupHealthManager {
    fn begin(
        config_path: &Path,
        diagnostics: StartupDiagnostics,
        forced_safe_mode: bool,
        bootstrap_failure: Option<String>,
    ) -> Self {
        let base_dir = resolve_config_base_dir(config_path);
        let state_path = base_dir.join(STARTUP_STATE_FILE_NAME);
        let previous = fs::read(&state_path)
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
        let startup_log_path = diagnostics.primary_path.display().to_string();
        let manager = Self {
            inner: Arc::new(Mutex::new(StartupHealthSnapshot {
                safe_mode,
                forced_safe_mode,
                consecutive_failures,
                frontend_ready: false,
                startup_log_path,
                components: Vec::new(),
            })),
            state_path,
            diagnostics,
        };
        manager.set_component("desktop_shell", "桌面基础壳", "starting", None);
        if let Some(detail) = bootstrap_failure {
            manager.set_component("configuration_path", "配置目录", "degraded", Some(detail));
        } else {
            manager.set_component("configuration_path", "配置目录", "ready", None);
        }
        if safe_mode {
            manager.diagnostics.warn(format!(
                "safe mode enabled: forced={forced_safe_mode} consecutive_failures={consecutive_failures}"
            ));
        }
        manager.write_pending_state();
        manager
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

    fn set_component(&self, id: &str, label: &str, status: &str, detail: Option<String>) {
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
        }
        self.write_ready_state()?;
        self.diagnostics
            .info("frontend readiness handshake completed");
        Ok(self.snapshot())
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
    token: String,
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
    config: AgentConfig,
    runtime: RuntimeSnapshot,
}

#[derive(Serialize)]
struct ConfigRecoveryDocument {
    config_path: String,
    archived_path: Option<String>,
    manifest_preview: String,
    config: AgentConfig,
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
    config: Option<AgentConfig>,
    runtime: Option<RuntimeSnapshot>,
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
    #[serde(rename = "localClientToken")]
    local_client_token: Option<String>,
    #[serde(rename = "localClientTokenType")]
    local_client_token_type: Option<String>,
    #[serde(rename = "localClientKeyId")]
    local_client_key_id: Option<String>,
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
    release_url: String,
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
    release_url: String,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum RegisteredServiceState {
    NotConfigured,
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Serialize)]
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
    #[serde(default)]
    manifest: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorAppInstallDocument {
    install: ConnectorInstallResult,
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
    let bundled = bundled_baijimu_cli_path();
    managed_tool::inspect(bundled.as_deref()).map_err(|err| err.to_string())
}

#[tauri::command]
async fn rollback_baijimu_cli() -> Result<managed_tool::ManagedToolStatus, String> {
    managed_tool::rollback().map_err(|err| err.to_string())?;
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
        config,
        runtime,
    })
}

#[tauri::command]
async fn save_config(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
) -> Result<ConfigDocument, String> {
    save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    let runtime = state.runtime.snapshot().await;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config,
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
        config,
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
        config,
        runtime,
    })
}

#[tauri::command]
async fn start_agent(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
) -> Result<RuntimeSnapshot, CommandError> {
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
async fn list_logs(
    state: tauri::State<'_, DesktopState>,
    limit: Option<usize>,
) -> Result<Vec<bridge_agent::LogEntry>, String> {
    Ok(state.runtime.logs(limit.unwrap_or(200)).await)
}

#[tauri::command]
async fn clear_logs(state: tauri::State<'_, DesktopState>) -> Result<(), String> {
    state.runtime.clear_logs().await;
    Ok(())
}

#[tauri::command]
async fn reset_example_config(
    state: tauri::State<'_, DesktopState>,
) -> Result<ConfigDocument, String> {
    let config = AgentConfig::example();
    save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    let runtime = state.runtime.snapshot().await;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config,
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
        config: recovery.config,
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
            "HTTP {status}: 平台授权接口返回了 HTML 错误页，可能是网关路由、服务异常或请求体超过平台限制。请确认 Base URL 为 https://baijimu.com/lowcode3，并检查平台授权服务日志。"
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
            Command::new(candidate)
                .arg(url)
                .spawn()
                .map_err(|err| format!("打开 Microsoft Edge 失败: {err}"))?;
            return Ok(());
        }
    }

    Command::new("msedge")
        .arg(url)
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
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
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
    run_start_command(service_config.name, start_command).await
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
    run_start_command(service_config.name, stop_command).await
}

fn start_local_app_ui_server(
    endpoint: Arc<RwLock<Option<LocalAppUiEndpoint>>>,
    startup_health: StartupHealthManager,
    diagnostics: StartupDiagnostics,
) {
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
        let token = uuid::Uuid::new_v4().simple().to_string();
        match endpoint.write() {
            Ok(mut value) => {
                *value = Some(LocalAppUiEndpoint {
                    port,
                    token: token.clone(),
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
        let state = LocalAppUiHttpState { token };
        let router = Router::new()
            .route("/{token}/{connector_id}/", get(local_app_ui_entry_handler))
            .route(
                "/{token}/{connector_id}/{*asset_path}",
                get(local_app_ui_asset_handler),
            )
            .with_state(state);
        if let Err(err) = axum::serve(listener, router).await {
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
    if token != state.token || !local_app_ui_request_host_matches(headers, token, connector_id) {
        return local_app_ui_error(StatusCode::NOT_FOUND, "not found");
    }
    if asset_path == Some(LOCAL_APP_UI_BRIDGE_ASSET) {
        return local_app_ui_response(
            StatusCode::OK,
            "application/javascript; charset=utf-8",
            LOCAL_APP_UI_BRIDGE_SCRIPT.as_bytes().to_vec(),
        );
    }

    let record = match show_connector(connector_id) {
        Ok(record) => record,
        Err(_) => return local_app_ui_error(StatusCode::NOT_FOUND, "application not found"),
    };
    let Some(ui) = record.manifest.ui.as_ref() else {
        return local_app_ui_error(StatusCode::NOT_FOUND, "application UI not found");
    };
    let package_path = Path::new(&record.package_path);
    let resolved = match resolve_connector_ui_asset(package_path, ui, asset_path) {
        Ok(path) => path,
        Err(_) => return local_app_ui_error(StatusCode::NOT_FOUND, "asset not found"),
    };
    let mut body = match tokio::fs::read(&resolved).await {
        Ok(body) => body,
        Err(_) => return local_app_ui_error(StatusCode::NOT_FOUND, "asset not found"),
    };
    if asset_path.is_none() {
        body = match inject_local_app_ui_bridge(body) {
            Ok(body) => body,
            Err(message) => return local_app_ui_error(StatusCode::UNPROCESSABLE_ENTITY, &message),
        };
    }
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
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<ConnectorSummary>, String> {
    let report =
        sync_installed_connectors_report(&state.config_path).map_err(|err| err.to_string())?;
    if !report.failures.is_empty() {
        state
            .runtime
            .push_desktop_log(
                "warn",
                &format_connector_sync_failures(&report.failures),
                LogMetadata::category("connector").outcome("sync_failed"),
            )
            .await;
    }
    list_connectors().map_err(|err| err.to_string())
}

#[tauri::command]
async fn list_market_connector_apps(
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<MarketConnectorApp>, String> {
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let base_url = config.platform.base_url.trim_end_matches('/');
    let platform = normalized_platform();
    let arch = std::env::consts::ARCH;
    let url = format!("{base_url}/api/local-app-market/apps?platform={platform}&arch={arch}");
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

#[tauri::command]
async fn invoke_connector_management(
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
    id: String,
    source: String,
    checksum: Option<String>,
    allow_git: Option<bool>,
) -> Result<ConnectorAppUpdateStatus, String> {
    let connector_id = id.trim();
    if connector_id.is_empty() {
        return Err("应用 ID 不能为空".to_string());
    }
    let source = source.trim();
    if source.is_empty() {
        return Err("更新来源不能为空".to_string());
    }

    let installed = show_connector(connector_id).map_err(|err| err.to_string())?;
    let resolved_source =
        resolve_connector_source(source, allow_git.unwrap_or(true), checksum.as_deref()).await?;
    let latest_manifest =
        load_connector_manifest(resolved_source.path()).map_err(|err| err.to_string())?;
    if latest_manifest.id != installed.manifest.id {
        return Err(format!(
            "更新来源应用 ID 不匹配：当前 `{}`，来源 `{}`",
            installed.manifest.id, latest_manifest.id
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
        source: source.to_string(),
    })
}

#[tauri::command]
async fn install_connector_app(
    state: tauri::State<'_, DesktopState>,
    source: String,
    replace: bool,
    checksum: Option<String>,
    allow_git: Option<bool>,
) -> Result<ConnectorAppInstallDocument, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let source = source.trim();
    if source.is_empty() {
        return Err("安装来源不能为空".to_string());
    }

    let allow_git = allow_git.unwrap_or(true);
    if !allow_git
        && connector_archive_kind(source).is_some()
        && checksum.as_deref().is_none_or(str::is_empty)
    {
        return Err("市场本地应用发布包必须提供 SHA-256 checksum".to_string());
    }
    let resolved_source = resolve_connector_source(source, allow_git, checksum.as_deref()).await?;
    let install = install_connector_from_path_with_source_reference(
        resolved_source.path(),
        &state.config_path,
        replace,
        Some(source),
    )
    .map_err(|err| err.to_string())?;
    let runtime = state
        .runtime
        .apply_capabilities_from_path(&state.config_path)
        .await
        .map_err(|err| err.to_string())?;
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    Ok(ConnectorAppInstallDocument {
        install,
        config: ConfigDocument {
            config_path: state.config_path.display().to_string(),
            manifest_preview,
            config,
            runtime,
        },
    })
}

#[tauri::command]
async fn start_connector_app(
    state: tauri::State<'_, DesktopState>,
    id: String,
) -> Result<ConnectorStartResult, String> {
    start_connector(id.trim(), &state.config_path).map_err(|err| err.to_string())
}

#[tauri::command]
async fn stop_connector_app(
    state: tauri::State<'_, DesktopState>,
    id: String,
) -> Result<ConnectorStartResult, String> {
    stop_connector(id.trim(), &state.config_path).map_err(|err| err.to_string())
}

#[tauri::command]
async fn uninstall_connector_app(
    state: tauri::State<'_, DesktopState>,
    id: String,
) -> Result<ConfigDocument, String> {
    uninstall_connector(id.trim(), &state.config_path).map_err(|err| err.to_string())?;
    let runtime = state
        .runtime
        .apply_capabilities_from_path(&state.config_path)
        .await
        .map_err(|err| err.to_string())?;
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config,
        runtime,
    })
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
    updated.platform.workspace_id = Some(authorized.workspace_id);
    updated.relay.agent_id = authorized.device_id.clone();
    updated.relay.url = authorized.relay_ws_url.clone();
    updated.relay.token = authorized.agent_token.clone();
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
        config: Some(updated),
        runtime: Some(runtime),
    })
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut document = if path.exists() {
        let content = fs::read_to_string(&path)?;
        serde_json::from_str::<Value>(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    if !document.is_object() {
        document = serde_json::json!({});
    }

    document["currentEnvironment"] = serde_json::json!("prod");
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
        .get("machineCredentials")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    credentials.retain(|item| {
        item.get("workspaceId").and_then(|value| value.as_u64()) != Some(authorized.workspace_id)
            || item.get("clientId").and_then(|value| value.as_str())
                != Some(authorized.device_id.as_str())
    });
    credentials.push(serde_json::json!({
        "workspaceId": authorized.workspace_id,
        "clientId": authorized.device_id,
        "keyId": authorized.local_client_key_id,
        "token": local_client_token,
        "tokenType": authorized.local_client_token_type.as_deref().unwrap_or("workspace_user_api_key"),
        "issuedAtEpochSeconds": now_epoch_seconds(),
    }));
    document["machineCredentials"] = Value::Array(credentials);

    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, serde_json::to_vec_pretty(&document)?)?;
    #[cfg(unix)]
    fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp_path, &path)?;
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(Some(SharedCliAuthWriteResult { path }))
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
    start_runtime_after_windows_service_handoff(&state.runtime, &state.config_path).await
}

async fn start_runtime_after_windows_service_handoff(
    runtime: &AgentRuntimeManager,
    config_path: &Path,
) -> anyhow::Result<RuntimeSnapshot> {
    #[cfg(windows)]
    {
        for attempt in 0..=WINDOWS_SERVICE_HANDOFF_RETRIES {
            match runtime.start_from_path(config_path).await {
                Ok(snapshot) => return Ok(snapshot),
                Err(err)
                    if attempt < WINDOWS_SERVICE_HANDOFF_RETRIES
                        && runtime_lock_is_owned_by_windows_service(&err) =>
                {
                    tokio::time::sleep(WINDOWS_SERVICE_HANDOFF_RETRY_DELAY).await;
                }
                Err(err) => return Err(err),
            }
        }
        unreachable!("Windows service handoff loop must return")
    }

    #[cfg(not(windows))]
    {
        runtime.start_from_path(config_path).await
    }
}

#[cfg(windows)]
fn runtime_lock_is_owned_by_windows_service(err: &anyhow::Error) -> bool {
    let Some(conflict) = err.downcast_ref::<RuntimeLockConflict>() else {
        return false;
    };

    [
        conflict.process.name.as_deref(),
        conflict.process.executable_path.as_deref(),
        conflict.process.command_line.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(process_value_is_windows_service)
}

#[cfg(any(windows, test))]
fn process_value_is_windows_service(value: &str) -> bool {
    value
        .trim()
        .trim_matches('"')
        .rsplit(['/', '\\'])
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("bridge-agent-service.exe"))
        || value
            .to_ascii_lowercase()
            .contains("bridge-agent-service.exe")
}

#[tauri::command]
fn app_version() -> AppVersionInfo {
    AppVersionInfo {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        current_target: current_update_target(),
    }
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
    app.restart();
}

#[tauri::command]
fn open_startup_log(state: tauri::State<'_, DesktopState>) -> Result<(), String> {
    let path = state.startup_health.snapshot().startup_log_path;
    open::that(path).map_err(|err| format!("打开启动日志失败: {err}"))
}

#[tauri::command]
fn report_frontend_bootstrap_event(state: tauri::State<'_, DesktopState>, message: String) {
    state
        .startup_health
        .diagnostics
        .info(format!("frontend bootstrap: {message}"));
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
async fn install_app_update(app: tauri::AppHandle) -> Result<AppUpdateInstallResult, String> {
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

    let release_url = configured_release_page_url().unwrap_or_default();
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
            release_url,
        });
    };
    let update_version = update.version.to_string();
    let asset_name = update
        .download_url
        .path_segments()
        .and_then(|segments| segments.last())
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
    let install_app = app.clone();
    let install_version = update_version.clone();
    let install_asset_name = asset_name.clone();
    update
        .download_and_install(
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
            move || {
                emit_app_update_progress(
                    &install_app,
                    AppUpdateProgress {
                        phase: "installing".to_string(),
                        message: "更新包签名校验通过，正在安装".to_string(),
                        version: Some(install_version),
                        asset_name: install_asset_name,
                        downloaded_bytes: None,
                        total_bytes: None,
                        downloaded_path: None,
                    },
                );
            },
        )
        .await
        .map_err(|err| format!("下载或安装官方更新失败: {err}"))?;

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
        release_url,
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

fn configured_release_page_url() -> Option<String> {
    option_env!("BRIDGE_AGENT_RELEASE_PAGE_URL")
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned)
}

fn release_page_url(release: &UpdateReleaseResponse) -> String {
    release
        .release_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned)
        .or_else(configured_release_page_url)
        .unwrap_or_default()
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
                        return RegisteredServiceStatus {
                            service: service.name,
                            status: RegisteredServiceState::Unhealthy,
                            detail: Some(format!(
                                "health HTTP {}，期望 {}",
                                status.as_u16(),
                                expected_status
                            )),
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

            let timeout_secs = timeout_secs.unwrap_or(15).max(1);
            match timeout(Duration::from_secs(timeout_secs), process.output()).await {
                Ok(Ok(output)) => Ok(StartRegisteredServiceResult {
                    service,
                    success: output.status.success(),
                    exit_code: output.status.code(),
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                    timed_out: false,
                }),
                Ok(Err(err)) => Err(format!("启动服务 `{service}` 失败: {err}")),
                Err(_) => Ok(StartRegisteredServiceResult {
                    service,
                    success: false,
                    exit_code: None,
                    stdout: String::new(),
                    stderr: format!("timed out after {timeout_secs}s"),
                    timed_out: true,
                }),
            }
        }
    }
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
) -> Result<ResolvedConnectorSource, String> {
    let (source, git_revision) = split_source_revision(source);
    if let Some(archive_url) =
        connector_archive_download_url(&source, git_revision.as_deref(), allow_git)?
    {
        return resolve_connector_archive_source(&archive_url, expected_checksum).await;
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
        }
    }
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
) -> Result<ResolvedConnectorSource, String> {
    let kind = connector_archive_kind(archive_url)
        .ok_or_else(|| "本地应用下载源必须是 .zip、.tar.gz 或 .tgz 文件。".to_string())?;
    let response = Client::new()
        .get(archive_url)
        .header(reqwest::header::USER_AGENT, UPDATE_USER_AGENT)
        .send()
        .await
        .map_err(|err| format!("下载本地应用失败: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let payload = response.text().await.unwrap_or_default();
        return Err(format!("下载本地应用失败 ({status}): {payload}"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("读取本地应用下载包失败: {err}"))?;
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

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("百积木")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_MENU_SHOW => show_main_window(app, None),
            TRAY_MENU_QUIT => quit_app(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => show_main_window(tray.app_handle(), None),
            _ => {}
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

fn show_main_window(app: &tauri::AppHandle, diagnostics: Option<&StartupDiagnostics>) {
    if app_is_quitting(app) {
        if let Some(diagnostics) = diagnostics {
            diagnostics.info("skipping main window restore because app is quitting");
        }
        return;
    }
    show_dock_icon(app, diagnostics);
    if let Some(window) = app.get_webview_window("main") {
        restore_main_window(&window, diagnostics);
    } else if let Some(diagnostics) = diagnostics {
        diagnostics.warn("main window is unavailable during show request");
    }

    for delay_ms in [120, 400, 900] {
        let app = app.clone();
        let diagnostics = diagnostics.cloned();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            if app_is_quitting(&app) {
                if let Some(diagnostics) = diagnostics.as_ref() {
                    diagnostics.info(format!(
                        "skipping {delay_ms}ms main window restore retry because app is quitting"
                    ));
                }
                return;
            }
            if let Some(diagnostics) = diagnostics.as_ref() {
                run_ui_action(
                    diagnostics,
                    &format!("{delay_ms}ms main window restore retry"),
                    || restore_main_window_once(&app, Some(diagnostics), delay_ms),
                );
            } else {
                restore_main_window_once(&app, None, delay_ms);
            }
        });
    }
}

fn app_is_quitting(app: &tauri::AppHandle) -> bool {
    app.try_state::<DesktopState>()
        .is_some_and(|state| state.quitting.load(Ordering::SeqCst))
}

fn run_ui_action(diagnostics: &StartupDiagnostics, label: &str, action: impl FnOnce()) {
    diagnostics.info(format!("{label} started"));
    if panic::catch_unwind(AssertUnwindSafe(action)).is_err() {
        diagnostics.error(format!("{label} panicked; continuing"));
    } else {
        diagnostics.info(format!("{label} completed"));
    }
}

fn show_main_window_deferred(
    app: tauri::AppHandle,
    diagnostics: StartupDiagnostics,
    reason: &'static str,
    delay_ms: u64,
) {
    diagnostics.info(format!(
        "deferring show main window for {reason} by {delay_ms}ms"
    ));
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        run_ui_action(&diagnostics, reason, || {
            show_main_window(&app, Some(&diagnostics));
        });
    });
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

fn restore_main_window_once(
    app: &tauri::AppHandle,
    diagnostics: Option<&StartupDiagnostics>,
    delay_ms: u64,
) {
    show_dock_icon(app, diagnostics);
    if let Some(window) = app.get_webview_window("main") {
        restore_main_window(&window, diagnostics);
    } else if let Some(diagnostics) = diagnostics {
        diagnostics.warn(format!(
            "main window is unavailable during {delay_ms}ms restore retry"
        ));
    }
}

fn hide_to_tray(window: &tauri::Window) {
    if let Err(err) = window.hide() {
        eprintln!("failed to hide main window: {err}");
    }
    hide_dock_icon(window.app_handle());
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
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = runtime.stop().await {
            eprintln!("failed to stop runtime before exit: {err:#}");
        }
        app.exit(0);
    });
}

fn auto_start_agent(
    runtime: AgentRuntimeManager,
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
        if let Err(err) = ensure_config_exists(&config_path) {
            diagnostics.error(format!(
                "failed to prepare bridge-agent config at {}: {err:#}",
                config_path.display()
            ));
            startup_health.set_component(
                "agent_runtime",
                "Agent 运行时",
                "degraded",
                Some(format!("配置初始化失败: {err}")),
            );
            return;
        }
        match load_agent_config(&config_path) {
            Ok(config) if !config_is_authorized(&config) => {
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
            Ok(_) => diagnostics.info("bridge-agent config loaded for auto start"),
            Err(err) => {
                diagnostics.error(format!(
                    "failed to load bridge-agent config before auto start from {}: {err:#}",
                    config_path.display()
                ));
                startup_health.set_component(
                    "agent_runtime",
                    "Agent 运行时",
                    "degraded",
                    Some(format!("配置加载失败: {err}")),
                );
                return;
            }
        }
        if let Err(err) = start_runtime_after_windows_service_handoff(&runtime, &config_path).await
        {
            diagnostics.error(format!(
                "failed to auto start bridge-agent runtime: {err:#}"
            ));
            startup_health.set_component(
                "agent_runtime",
                "Agent 运行时",
                "degraded",
                Some(err.to_string()),
            );
        } else {
            diagnostics.info("bridge-agent runtime auto start completed");
            startup_health.set_component("agent_runtime", "Agent 运行时", "ready", None);
        }
    });
}

fn config_is_authorized(config: &AgentConfig) -> bool {
    config.platform.workspace_id.is_some() && !config.relay.token.trim().is_empty()
}

fn install_bundled_baijimu_cli(diagnostics: &StartupDiagnostics) -> anyhow::Result<()> {
    let source = bundled_baijimu_cli_path();
    let status = managed_tool::bootstrap_bundled(source.as_deref())?;
    diagnostics.info(format!(
        "managed baijimu CLI bootstrap completed: state={} version={} launcher={}",
        status.state,
        status.installed_version.as_deref().unwrap_or("unknown"),
        status.launcher_path
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
    let forced_safe_mode = std::env::args().any(|arg| arg == "--safe-mode");
    let startup_health = StartupHealthManager::begin(
        &config_path,
        diagnostics.clone(),
        forced_safe_mode,
        config_path_failure,
    );
    if let Some(detail) = crypto_provider_failure {
        startup_health.set_component("crypto_provider", "网络加密组件", "degraded", Some(detail));
    } else {
        startup_health.set_component("crypto_provider", "网络加密组件", "ready", None);
    }

    let runtime = AgentRuntimeManager::new();
    let quitting = Arc::new(AtomicBool::new(false));
    let local_app_ui = Arc::new(RwLock::new(None));
    let single_instance_diagnostics = diagnostics.clone();
    let setup_diagnostics = diagnostics.clone();
    let page_load_diagnostics = diagnostics.clone();
    let setup_health = startup_health.clone();
    let setup_local_app_ui = Arc::clone(&local_app_ui);
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(
            move |app, _argv, _cwd| {
                let diagnostics = single_instance_diagnostics.clone();
                run_ui_action(&diagnostics, "single instance show main window", || {
                    show_main_window(app, Some(&diagnostics));
                });
            },
        ))
        .manage(DesktopState {
            runtime: runtime.clone(),
            config_path: config_path.clone(),
            quitting: Arc::clone(&quitting),
            local_app_ui,
            startup_health: startup_health.clone(),
        })
        .on_page_load(move |webview, payload| {
            page_load_diagnostics.info(format!(
                "webview page load {:?}: label={} url={}",
                payload.event(),
                webview.label(),
                payload.url()
            ));
        })
        .setup(move |app| {
            setup_diagnostics.info("tauri setup started");
            #[cfg(debug_assertions)]
            if std::env::var_os("BRIDGE_AGENT_OPEN_DEVTOOLS").is_some() {
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }
            setup_health.set_component("updater", "官方更新器", "ready", None);
            if let Err(err) = setup_tray(app, &setup_diagnostics) {
                setup_diagnostics.error(format!(
                    "failed to setup tray; continuing without tray icon: {err:#}"
                ));
                setup_health.set_component("tray", "系统托盘", "degraded", Some(err.to_string()));
            } else {
                setup_health.set_component("tray", "系统托盘", "ready", None);
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
                    setup_diagnostics.clone(),
                );
                bootstrap_bundled_baijimu_cli(setup_health.clone(), setup_diagnostics.clone());
                auto_start_agent(
                    runtime.clone(),
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
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            save_service,
            delete_service,
            start_agent,
            stop_agent,
            stop_conflicting_runtime,
            runtime_snapshot,
            apply_saved_config_to_runtime,
            test_capability,
            list_logs,
            clear_logs,
            reset_example_config,
            recover_invalid_config,
            open_in_browser,
            open_in_edge,
            desktop_permission_status,
            registered_service_statuses,
            start_registered_service,
            stop_registered_service,
            list_connector_apps,
            connector_app_ui_url,
            list_market_connector_apps,
            show_connector_app,
            invoke_connector_management,
            check_connector_app_update,
            install_connector_app,
            start_connector_app,
            stop_connector_app,
            uninstall_connector_app,
            request_desktop_permission,
            open_desktop_permission_settings,
            start_browser_auth,
            poll_browser_auth,
            app_version,
            get_startup_health,
            mark_frontend_ready,
            restart_in_normal_mode,
            open_startup_log,
            report_frontend_bootstrap_event,
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
                startup_health.set_component(
                    "desktop_shell",
                    "桌面基础壳",
                    "starting",
                    Some("等待前端就绪确认".to_string()),
                );
                show_main_window_deferred(
                    app.clone(),
                    diagnostics.clone(),
                    "ready show main window",
                    700,
                );
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                diagnostics.info("tauri reopen event received");
                show_main_window_deferred(
                    app.clone(),
                    diagnostics.clone(),
                    "reopen show main window",
                    120,
                );
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
    fn windows_service_handoff_recognizes_process_probe_values() {
        assert!(process_value_is_windows_service("bridge-agent-service.exe"));
        assert!(process_value_is_windows_service(
            r#"C:\Program Files\Baijimu\bridge-agent-service.exe" --config agent-config.json"#
        ));
        assert!(!process_value_is_windows_service(
            "bridge-agent-desktop.exe"
        ));
    }

    #[test]
    fn shared_cli_auth_path_should_live_under_home_config() {
        let path = shared_cli_auth_path();

        assert!(path.ends_with(Path::new(".config").join("baijimu").join("auth.json")));
        assert!(path.is_absolute() || std::env::var_os("HOME").is_none());
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
        assert!(LOCAL_APP_UI_BRIDGE_SCRIPT.contains("window.addEventListener(\"pageshow\", announceReady)"));
        assert!(LOCAL_APP_UI_BRIDGE_SCRIPT.contains("message.type === HELLO_TYPE"));
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

        let health = StartupHealthManager::begin(
            &config_path,
            StartupDiagnostics::for_config_path(&config_path),
            false,
            None,
        );

        assert!(health.safe_mode());
        assert_eq!(
            health.snapshot().consecutive_failures,
            SAFE_MODE_FAILURE_THRESHOLD
        );
    }
}
