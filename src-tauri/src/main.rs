#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bridge_agent::{
    default_config_path, ensure_browser_auth_agent_id, ensure_config_exists,
    install_connector_from_path, install_rustls_crypto_provider, list_connectors,
    load_config as load_agent_config, load_connector_manifest, manifest_preview_json,
    reset_invalid_config, save_config as save_agent_config, show_connector, start_connector,
    stop_connector, terminate_runtime_lock_owner, uninstall_connector, AgentConfig,
    AgentRuntimeManager, ConnectorInstallRecord, ConnectorInstallResult, ConnectorStartResult,
    ConnectorSummary, RuntimeLockConflict, RuntimeSnapshot, ServiceConfig, ServiceHealthCheck,
    ServiceStartCommand,
};
use reqwest::Client;
use semver::Version;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};
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
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSApplication;
#[cfg(target_os = "macos")]
use tauri::ActivationPolicy;

const UPDATE_USER_AGENT: &str = concat!("bridge-agent-desktop/", env!("CARGO_PKG_VERSION"));
const TRAY_ID: &str = "bridge-agent";
const TRAY_MENU_SHOW: &str = "show";
const TRAY_MENU_QUIT: &str = "quit";

struct DesktopState {
    runtime: AgentRuntimeManager,
    config_path: PathBuf,
    quitting: Arc<AtomicBool>,
}

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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdateStatus {
    current_version: String,
    latest_version: Option<String>,
    update_available: bool,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorAppInstallDocument {
    install: ConnectorInstallResult,
    config: ConfigDocument,
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
    #[serde(default)]
    assets: Vec<UpdateReleaseAsset>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateReleaseAsset {
    name: String,
    #[serde(alias = "download_url", alias = "browser_download_url")]
    download_url: String,
    digest: Option<String>,
    sha256: Option<String>,
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

#[tauri::command]
async fn list_connector_apps() -> Result<Vec<ConnectorSummary>, String> {
    list_connectors().map_err(|err| err.to_string())
}

#[tauri::command]
async fn show_connector_app(id: String) -> Result<ConnectorInstallRecord, String> {
    show_connector(id.trim()).map_err(|err| err.to_string())
}

#[tauri::command]
async fn check_connector_app_update(
    id: String,
    source: String,
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
    let resolved_source = resolve_connector_source(source).await?;
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
) -> Result<ConnectorAppInstallDocument, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let source = source.trim();
    if source.is_empty() {
        return Err("安装来源不能为空".to_string());
    }

    let resolved_source = resolve_connector_source(source).await?;
    let install = install_connector_from_path(resolved_source.path(), &state.config_path, replace)
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
        return Ok(());
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
    let manifest = manifest_preview_json(&config).map_err(|err| err.to_string())?;
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
        let payload = response.text().await.unwrap_or_default();
        return Err(format!("启动浏览器授权失败: {payload}"));
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
) -> Result<BrowserAuthPollResponse, String> {
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
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        let payload = response.text().await.unwrap_or_default();
        return Err(format!("轮询浏览器授权失败: {payload}"));
    }

    let payload: RawBrowserAuthPollResponse =
        response.json().await.map_err(|err| err.to_string())?;
    if payload.status != "authorized" {
        return Ok(BrowserAuthPollResponse {
            status: payload.status,
            message: payload.message,
            config: None,
        });
    }

    let authorized = payload
        .authorized_payload
        .ok_or_else(|| "授权成功但缺少 authorizedPayload".to_string())?;
    let mut updated = config;
    updated.platform.workspace_id = Some(authorized.workspace_id);
    updated.relay.agent_id = authorized.device_id;
    updated.relay.url = authorized.relay_ws_url;
    updated.relay.token = authorized.agent_token;
    save_agent_config(&state.config_path, &updated).map_err(|err| err.to_string())?;

    Ok(BrowserAuthPollResponse {
        status: payload.status,
        message: payload.message,
        config: Some(updated),
    })
}

#[tauri::command]
async fn check_app_update() -> Result<AppUpdateStatus, String> {
    let current_version = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|err| format!("当前版本号无效: {err}"))?;
    let release = fetch_latest_release().await?;
    let latest_version = release_version(&release)?;
    let preferred_asset = select_release_asset(&release);
    let release_url = release_page_url(&release);
    let release_name = release.release_name.clone();
    let published_at = release.published_at.clone();
    let asset_name = preferred_asset.map(|asset| asset.name.clone());
    let auto_download_available = preferred_asset.is_some();
    let update_available = release
        .update_available
        .unwrap_or(latest_version > current_version);

    Ok(AppUpdateStatus {
        current_version: current_version.to_string(),
        latest_version: Some(latest_version.to_string()),
        update_available,
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
    let current_version = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|err| format!("当前版本号无效: {err}"))?;
    let release = fetch_latest_release().await?;
    let latest_version = release_version(&release)?;
    let release_url = release_page_url(&release);

    let update_available = release
        .update_available
        .unwrap_or(latest_version > current_version);
    if !update_available || latest_version <= current_version {
        return Ok(AppUpdateInstallResult {
            status: "up_to_date".to_string(),
            version: current_version.to_string(),
            asset_name: None,
            downloaded_path: None,
            release_url,
        });
    }

    let asset = select_release_asset(&release).ok_or_else(|| {
        format!(
            "当前平台 {} 暂不支持自动下载更新，请打开发布页手工下载。",
            current_update_target()
        )
    })?;
    let response = Client::new()
        .get(&asset.download_url)
        .header(reqwest::header::USER_AGENT, UPDATE_USER_AGENT)
        .send()
        .await
        .map_err(|err| format!("下载更新失败: {err}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let payload = response.text().await.unwrap_or_default();
        return Err(format!("下载更新失败 ({status}): {payload}"));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("读取更新文件失败: {err}"))?;
    verify_asset_digest(asset, bytes.as_ref())?;

    let download_path = resolve_update_download_path(&asset.name)?;
    if let Some(parent) = download_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("创建更新目录失败: {err}"))?;
    }
    std::fs::write(&download_path, bytes.as_ref())
        .map_err(|err| format!("写入更新文件失败: {err}"))?;
    make_asset_ready_to_open(&download_path)?;

    #[cfg(target_os = "macos")]
    {
        schedule_macos_app_update(&app, &download_path)?;
        let app_to_exit = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1200));
            app_to_exit.exit(0);
        });

        return Ok(AppUpdateInstallResult {
            status: "downloaded".to_string(),
            version: latest_version.to_string(),
            asset_name: Some(asset.name.clone()),
            downloaded_path: Some(download_path.display().to_string()),
            release_url,
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        open::that(&download_path).map_err(|err| format!("打开安装包失败: {err}"))?;

        Ok(AppUpdateInstallResult {
            status: "downloaded".to_string(),
            version: latest_version.to_string(),
            asset_name: Some(asset.name.clone()),
            downloaded_path: Some(download_path.display().to_string()),
            release_url,
        })
    }
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

fn select_release_asset(release: &UpdateReleaseResponse) -> Option<&UpdateReleaseAsset> {
    let preferred_names = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => vec!["_universal.dmg", ".dmg"],
        ("windows", "x86_64") => vec!["_x64_en-US.msi", ".msi", ".exe"],
        ("linux", "x86_64") => vec!["_amd64.AppImage", ".AppImage", "_amd64.deb", ".deb"],
        _ => Vec::new(),
    };

    for suffix in preferred_names {
        if let Some(asset) = release
            .assets
            .iter()
            .find(|asset| asset.name.ends_with(suffix))
        {
            return Some(asset);
        }
    }

    None
}

fn verify_asset_digest(asset: &UpdateReleaseAsset, bytes: &[u8]) -> Result<(), String> {
    let Some(expected_hash) = expected_asset_sha256(asset) else {
        return Ok(());
    };
    let actual_hash = format!("{:x}", Sha256::digest(bytes));
    if actual_hash != expected_hash.to_ascii_lowercase() {
        return Err(format!("更新文件校验失败: {}", asset.name));
    }
    Ok(())
}

fn expected_asset_sha256(asset: &UpdateReleaseAsset) -> Option<&str> {
    if let Some(sha256) = asset.sha256.as_deref() {
        let sha256 = sha256.trim();
        if !sha256.is_empty() {
            return Some(sha256);
        }
    }
    asset
        .digest
        .as_deref()
        .and_then(|digest| digest.trim().strip_prefix("sha256:"))
        .map(str::trim)
        .filter(|sha256| !sha256.is_empty())
}

fn resolve_update_download_path(asset_name: &str) -> Result<PathBuf, String> {
    let base_dir =
        dirs::download_dir().unwrap_or_else(|| std::env::temp_dir().join("bridge-agent-downloads"));
    let path = base_dir.join("Bridge Agent Updates").join(asset_name);
    if path.as_os_str().is_empty() {
        return Err("无法确定更新文件保存路径".to_string());
    }
    Ok(path)
}

fn make_asset_ready_to_open(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("AppImage"))
    {
        let mut permissions = std::fs::metadata(path)
            .map_err(|err| format!("读取更新文件权限失败: {err}"))?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions)
            .map_err(|err| format!("设置更新文件权限失败: {err}"))?;
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn schedule_macos_app_update(app: &tauri::AppHandle, dmg_path: &Path) -> Result<(), String> {
    let current_bundle = current_macos_app_bundle()
        .ok_or_else(|| "无法确定当前 macOS 应用包路径，不能自动替换更新。".to_string())?;
    let target_bundle = if current_bundle.starts_with("/Volumes") {
        PathBuf::from("/Applications").join(
            current_bundle
                .file_name()
                .ok_or_else(|| "无法确定当前 macOS 应用包名称。".to_string())?,
        )
    } else {
        current_bundle
    };
    let app_name = target_bundle
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Bridge Agent")
        .to_string();
    let bundle_identifier = app.config().identifier.clone();
    let process_id = std::process::id().to_string();

    let script_path = std::env::temp_dir().join(format!(
        "bridge-agent-update-{}-{}.sh",
        process_id,
        env!("CARGO_PKG_VERSION")
    ));
    std::fs::write(&script_path, macos_update_script())
        .map_err(|err| format!("写入 macOS 更新脚本失败: {err}"))?;
    let mut permissions = std::fs::metadata(&script_path)
        .map_err(|err| format!("读取 macOS 更新脚本权限失败: {err}"))?
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions)
        .map_err(|err| format!("设置 macOS 更新脚本权限失败: {err}"))?;

    Command::new("/bin/sh")
        .arg(&script_path)
        .arg(dmg_path)
        .arg(&target_bundle)
        .arg(app_name)
        .arg(process_id)
        .arg(bundle_identifier)
        .spawn()
        .map_err(|err| format!("启动 macOS 更新安装器失败: {err}"))?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn current_macos_app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        if ancestor
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn macos_update_script() -> &'static str {
    r#"#!/bin/sh
set -u

DMG_PATH="$1"
TARGET_APP="$2"
APP_NAME="$3"
APP_PID="$4"
BUNDLE_IDENTIFIER="$5"
LOG_DIR="$HOME/Library/Logs"
LOG_FILE="$LOG_DIR/Bridge Agent Updater.log"

mkdir -p "$LOG_DIR"
exec >> "$LOG_FILE" 2>&1

echo "[$(date '+%Y-%m-%d %H:%M:%S')] starting update from $DMG_PATH to $TARGET_APP"

for _ in $(seq 1 60); do
  if ! kill -0 "$APP_PID" 2>/dev/null; then
    break
  fi
  sleep 1
done

if kill -0 "$APP_PID" 2>/dev/null; then
  /usr/bin/osascript -e "tell application id \"$BUNDLE_IDENTIFIER\" to quit" >/dev/null 2>&1 || true
  for _ in $(seq 1 20); do
    if ! kill -0 "$APP_PID" 2>/dev/null; then
      break
    fi
    sleep 1
  done
fi

if kill -0 "$APP_PID" 2>/dev/null; then
  echo "application is still running; aborting update"
  exit 1
fi

ATTACH_OUTPUT="$(/usr/bin/hdiutil attach "$DMG_PATH" -nobrowse -readonly)"
VOLUME="$(printf '%s\n' "$ATTACH_OUTPUT" | /usr/bin/awk '/\/Volumes\// {print substr($0, index($0, "/Volumes/")); exit}')"

cleanup() {
  if [ -n "${VOLUME:-}" ]; then
    /usr/bin/hdiutil detach "$VOLUME" -quiet >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

if [ -z "${VOLUME:-}" ] || [ ! -d "$VOLUME" ]; then
  echo "failed to mount update dmg"
  exit 1
fi

SOURCE_APP="$(/usr/bin/find "$VOLUME" -maxdepth 1 -name "*.app" -type d | /usr/bin/head -n 1)"
if [ -z "$SOURCE_APP" ] || [ ! -d "$SOURCE_APP" ]; then
  echo "no .app bundle found in update dmg"
  exit 1
fi

install_without_privilege() {
  /bin/mkdir -p "$(/usr/bin/dirname "$TARGET_APP")" &&
  /bin/rm -rf "$TARGET_APP" &&
  /usr/bin/ditto "$SOURCE_APP" "$TARGET_APP"
}

install_with_privilege() {
  /usr/bin/osascript - "$SOURCE_APP" "$TARGET_APP" <<'OSA'
on run argv
  set sourceApp to item 1 of argv
  set targetApp to item 2 of argv
  do shell script "/bin/rm -rf " & quoted form of targetApp & " && /usr/bin/ditto " & quoted form of sourceApp & " " & quoted form of targetApp with administrator privileges
end run
OSA
}

if ! install_without_privilege; then
  echo "normal install failed; requesting administrator privilege"
  install_with_privilege
fi

/usr/bin/xattr -dr com.apple.quarantine "$TARGET_APP" >/dev/null 2>&1 || true
/usr/bin/open "$TARGET_APP"

echo "[$(date '+%Y-%m-%d %H:%M:%S')] update installed and relaunched $APP_NAME"
"#
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
}

impl ResolvedConnectorSource {
    fn path(&self) -> &Path {
        match self {
            Self::Local(path) => path.as_path(),
            Self::Git { path, .. } => path.as_path(),
        }
    }
}

async fn resolve_connector_source(source: &str) -> Result<ResolvedConnectorSource, String> {
    if is_git_connector_source(source) {
        let temp_dir = tempfile::tempdir().map_err(|err| err.to_string())?;
        let checkout_path = temp_dir.path().join("connector");
        let output = Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                source,
                checkout_path.to_string_lossy().as_ref(),
            ])
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

fn is_git_connector_source(source: &str) -> bool {
    let value = source.trim();
    value.starts_with("https://")
        || value.starts_with("http://")
        || value.starts_with("git@")
        || value.ends_with(".git")
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

#[cfg(target_os = "macos")]
fn prompt_accessibility_permission() {
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::true_value();
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
    let _ = unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef().cast()) };
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, TRAY_MENU_SHOW, "打开 Bridge Agent", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_MENU_QUIT, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let icon = app.default_window_icon().cloned();

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Bridge Agent")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_MENU_SHOW => show_main_window(app),
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
            } => show_main_window(tray.app_handle()),
            _ => {}
        });

    if let Some(icon) = icon {
        tray = tray.icon(icon);
    }

    tray.build(app)?;
    Ok(())
}

fn show_main_window(app: &tauri::AppHandle) {
    activate_app(app);
    if let Some(window) = app.get_webview_window("main") {
        restore_main_window(&window);
    }

    for delay_ms in [120, 400, 900] {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            activate_app(&app);
            if let Some(window) = app.get_webview_window("main") {
                restore_main_window(&window);
            }
        });
    }
}

fn restore_main_window(window: &tauri::WebviewWindow) {
    if let Err(err) = window.show() {
        eprintln!("failed to show main window: {err}");
    }
    if let Err(err) = window.unminimize() {
        eprintln!("failed to unminimize main window: {err}");
    }
    if let Err(err) = window.set_focus() {
        eprintln!("failed to focus main window: {err}");
    }
}

fn hide_to_tray(window: &tauri::Window) {
    if let Err(err) = window.hide() {
        eprintln!("failed to hide main window: {err}");
    }
    hide_dock_icon(window.app_handle());
}

#[cfg(target_os = "macos")]
fn activate_app(app: &tauri::AppHandle) {
    if let Err(err) = app.set_activation_policy(ActivationPolicy::Regular) {
        eprintln!("failed to set regular activation policy: {err}");
    }
    if let Err(err) = app.set_dock_visibility(true) {
        eprintln!("failed to show dock icon: {err}");
    }
    if let Err(err) = app.run_on_main_thread(move || {
        if let Some(mtm) = MainThreadMarker::new() {
            let ns_app = NSApplication::sharedApplication(mtm);
            ns_app.activate();
            #[allow(deprecated)]
            ns_app.activateIgnoringOtherApps(true);
        }
    }) {
        eprintln!("failed to activate app on main thread: {err}");
    }
}

#[cfg(not(target_os = "macos"))]
fn activate_app(_app: &tauri::AppHandle) {}

#[cfg(target_os = "macos")]
fn hide_dock_icon(app: &tauri::AppHandle) {
    if let Err(err) = app.set_dock_visibility(false) {
        eprintln!("failed to hide dock icon: {err}");
    }
    if let Err(err) = app.set_activation_policy(ActivationPolicy::Accessory) {
        eprintln!("failed to set accessory activation policy: {err}");
    }
}

#[cfg(not(target_os = "macos"))]
fn hide_dock_icon(_app: &tauri::AppHandle) {}

fn quit_app(app: &tauri::AppHandle) {
    let state = app.state::<DesktopState>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        app.exit(0);
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

fn auto_start_agent(runtime: AgentRuntimeManager, config_path: PathBuf) {
    tauri::async_runtime::spawn(async move {
        if let Err(err) = ensure_config_exists(&config_path) {
            eprintln!("failed to prepare bridge-agent config: {err:#}");
            return;
        }
        if let Err(err) = runtime.start_from_path(&config_path).await {
            eprintln!("failed to auto start bridge-agent runtime: {err:#}");
        }
    });
}

fn main() {
    install_rustls_crypto_provider().expect("failed to install rustls provider");
    let config_path = default_config_path().expect("failed to determine default config path");
    let runtime = AgentRuntimeManager::new();
    let quitting = Arc::new(AtomicBool::new(false));
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        .manage(DesktopState {
            runtime: runtime.clone(),
            config_path: config_path.clone(),
            quitting: Arc::clone(&quitting),
        })
        .setup(move |app| {
            setup_tray(app)?;
            auto_start_agent(runtime.clone(), config_path.clone());
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
            list_logs,
            clear_logs,
            reset_example_config,
            recover_invalid_config,
            open_in_browser,
            desktop_permission_status,
            registered_service_statuses,
            start_registered_service,
            stop_registered_service,
            list_connector_apps,
            show_connector_app,
            check_connector_app_update,
            install_connector_app,
            start_connector_app,
            stop_connector_app,
            uninstall_connector_app,
            request_desktop_permission,
            open_desktop_permission_settings,
            start_browser_auth,
            poll_browser_auth,
            check_app_update,
            install_app_update
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            tauri::RunEvent::Ready => {
                show_main_window(app);
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                show_main_window(app);
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
