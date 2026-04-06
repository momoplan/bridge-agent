#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bridge_agent::{
    AgentConfig, AgentRuntimeManager, RuntimeSnapshot, default_config_path, ensure_config_exists,
    install_rustls_crypto_provider, load_config as load_agent_config, manifest_preview_json,
    save_config as save_agent_config,
};
use reqwest::Client;
use semver::Version;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

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

const GITHUB_LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/momoplan/bridge-agent/releases/latest";
const GITHUB_LATEST_RELEASE_PAGE: &str = "https://github.com/momoplan/bridge-agent/releases/latest";
const GITHUB_API_ACCEPT: &str = "application/vnd.github+json";
const UPDATE_USER_AGENT: &str = concat!("bridge-agent-desktop/", env!("CARGO_PKG_VERSION"));

struct DesktopState {
    runtime: AgentRuntimeManager,
    config_path: PathBuf,
}

#[derive(Serialize)]
struct ConfigDocument {
    config_path: String,
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

#[derive(Debug, serde::Deserialize)]
struct GithubReleaseResponse {
    tag_name: String,
    html_url: String,
    name: Option<String>,
    published_at: Option<String>,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, serde::Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
    fn CGPreflightScreenCaptureAccess() -> bool;
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
async fn start_agent(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
) -> Result<RuntimeSnapshot, String> {
    save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    state
        .runtime
        .start_from_path(&state.config_path)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn stop_agent(state: tauri::State<'_, DesktopState>) -> Result<RuntimeSnapshot, String> {
    state.runtime.stop().await.map_err(|err| err.to_string())
}

#[tauri::command]
async fn runtime_snapshot(
    state: tauri::State<'_, DesktopState>,
) -> Result<RuntimeSnapshot, String> {
    Ok(state.runtime.snapshot().await)
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
fn open_in_browser(url: String) -> Result<(), String> {
    open::that(url).map_err(|err| err.to_string())
}

#[tauri::command]
fn desktop_permission_status() -> Result<DesktopPermissionStatus, String> {
    Ok(read_desktop_permission_status())
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
            "screen_recording" => "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
            "accessibility" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
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
async fn start_browser_auth(config: AgentConfig) -> Result<BrowserAuthStartResponse, String> {
    let client = Client::new();
    let manifest = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    let base_url = config.platform.base_url.trim_end_matches('/');
    let mut payload = serde_json::Map::new();
    if let Some(workspace_id) = config.platform.workspace_id {
        payload.insert("workspaceId".to_string(), serde_json::json!(workspace_id));
    }
    payload.insert("deviceId".to_string(), serde_json::json!(config.relay.agent_id));
    payload.insert("deviceName".to_string(), serde_json::json!(config.device.name));
    payload.insert(
        "deviceDescription".to_string(),
        serde_json::json!(config.device.description)
    );
    payload.insert("serviceManifest".to_string(), serde_json::json!(manifest));
    let response = client
        .post(format!("{base_url}/api/external-workspace-device-auth/start"))
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
        .post(format!("{base_url}/api/external-workspace-device-auth/poll"))
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

    let payload: RawBrowserAuthPollResponse = response.json().await.map_err(|err| err.to_string())?;
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
    let latest_version = parse_release_version(&release.tag_name)
        .map_err(|err| format!("最新版本号无效: {err}"))?;
    let preferred_asset = select_release_asset(&release);
    let release_url = if release.html_url.trim().is_empty() {
        GITHUB_LATEST_RELEASE_PAGE.to_string()
    } else {
        release.html_url.clone()
    };
    let release_name = release.name.clone();
    let published_at = release.published_at.clone();
    let asset_name = preferred_asset.map(|asset| asset.name.clone());
    let auto_download_available = preferred_asset.is_some();

    Ok(AppUpdateStatus {
        current_version: current_version.to_string(),
        latest_version: Some(latest_version.to_string()),
        update_available: latest_version > current_version,
        release_url,
        release_name,
        published_at,
        current_target: current_update_target(),
        auto_download_available,
        asset_name,
    })
}

#[tauri::command]
async fn install_app_update() -> Result<AppUpdateInstallResult, String> {
    let current_version = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|err| format!("当前版本号无效: {err}"))?;
    let release = fetch_latest_release().await?;
    let latest_version = parse_release_version(&release.tag_name)
        .map_err(|err| format!("最新版本号无效: {err}"))?;
    let release_url = if release.html_url.trim().is_empty() {
        GITHUB_LATEST_RELEASE_PAGE.to_string()
    } else {
        release.html_url.clone()
    };

    if latest_version <= current_version {
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
        .get(&asset.browser_download_url)
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
    std::fs::write(&download_path, bytes.as_ref()).map_err(|err| format!("写入更新文件失败: {err}"))?;
    make_asset_ready_to_open(&download_path)?;
    open::that(&download_path).map_err(|err| format!("打开安装包失败: {err}"))?;

    Ok(AppUpdateInstallResult {
        status: "downloaded".to_string(),
        version: latest_version.to_string(),
        asset_name: Some(asset.name.clone()),
        downloaded_path: Some(download_path.display().to_string()),
        release_url,
    })
}

fn parse_release_version(tag_name: &str) -> Result<Version, String> {
    let normalized = tag_name
        .trim()
        .strip_prefix("bridge-agent-v")
        .or_else(|| tag_name.trim().strip_prefix('v'))
        .unwrap_or(tag_name.trim());
    Version::parse(normalized).map_err(|err| err.to_string())
}

async fn fetch_latest_release() -> Result<GithubReleaseResponse, String> {
    let response = Client::new()
        .get(GITHUB_LATEST_RELEASE_API)
        .header(reqwest::header::USER_AGENT, UPDATE_USER_AGENT)
        .header(reqwest::header::ACCEPT, GITHUB_API_ACCEPT)
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

fn select_release_asset(release: &GithubReleaseResponse) -> Option<&GithubReleaseAsset> {
    let preferred_names = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => vec!["_universal.dmg", ".dmg"],
        ("windows", "x86_64") => vec!["_x64_en-US.msi", ".msi", ".exe"],
        ("linux", "x86_64") => vec!["_amd64.AppImage", ".AppImage", "_amd64.deb", ".deb"],
        _ => Vec::new(),
    };

    for suffix in preferred_names {
        if let Some(asset) = release.assets.iter().find(|asset| asset.name.ends_with(suffix)) {
            return Some(asset);
        }
    }

    None
}

fn verify_asset_digest(asset: &GithubReleaseAsset, bytes: &[u8]) -> Result<(), String> {
    let Some(expected_digest) = asset.digest.as_deref() else {
        return Ok(());
    };
    let Some(expected_hash) = expected_digest.strip_prefix("sha256:") else {
        return Ok(());
    };
    let actual_hash = format!("{:x}", Sha256::digest(bytes));
    if actual_hash != expected_hash.to_ascii_lowercase() {
        return Err(format!("更新文件校验失败: {}", asset.name));
    }
    Ok(())
}

fn resolve_update_download_path(asset_name: &str) -> Result<PathBuf, String> {
    let base_dir = dirs::download_dir().unwrap_or_else(|| std::env::temp_dir().join("bridge-agent-downloads"));
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

fn read_desktop_permission_status() -> DesktopPermissionStatus {
    #[cfg(target_os = "macos")]
    {
        DesktopPermissionStatus {
            platform: "macos".to_string(),
            accessibility_granted: unsafe { AXIsProcessTrusted() },
            screen_recording_granted: unsafe { CGPreflightScreenCaptureAccess() },
            accessibility_supported: true,
            screen_recording_supported: true,
        }
    }

    #[cfg(not(target_os = "macos"))]
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

#[cfg(target_os = "macos")]
fn prompt_accessibility_permission() {
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::true_value();
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
    let _ = unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef().cast()) };
}

fn main() {
    install_rustls_crypto_provider().expect("failed to install rustls provider");
    let config_path = default_config_path().expect("failed to determine default config path");
    tauri::Builder::default()
        .manage(DesktopState {
            runtime: AgentRuntimeManager::new(),
            config_path,
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            start_agent,
            stop_agent,
            runtime_snapshot,
            list_logs,
            clear_logs,
            reset_example_config,
            open_in_browser,
            desktop_permission_status,
            request_desktop_permission,
            open_desktop_permission_settings,
            start_browser_auth,
            poll_browser_auth,
            check_app_update,
            install_app_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
