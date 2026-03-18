#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bridge_agent::{
    AgentConfig, AgentRuntimeManager, RuntimeSnapshot, default_config_path, ensure_config_exists,
    install_rustls_crypto_provider, load_config as load_agent_config, manifest_preview_json,
    save_config as save_agent_config,
};
use reqwest::Client;
use serde::Serialize;
use std::path::PathBuf;

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
            start_browser_auth,
            poll_browser_auth
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
