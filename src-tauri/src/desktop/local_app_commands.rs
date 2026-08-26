use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DesktopPermissionStatus {
    pub(super) platform: String,
    pub(super) accessibility_granted: bool,
    pub(super) screen_recording_granted: bool,
    pub(super) accessibility_supported: bool,
    pub(super) screen_recording_supported: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StartRegisteredServiceResult {
    pub(super) service: String,
    pub(super) success: bool,
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) timed_out: bool,
}

#[tauri::command]
pub(super) fn desktop_permission_status() -> Result<DesktopPermissionStatus, String> {
    Ok(read_desktop_permission_status())
}

#[tauri::command]
pub(super) async fn registered_service_statuses(
    state: tauri::State<'_, DesktopState>,
) -> Result<Vec<RegisteredServiceStatus>, String> {
    state.registered_services.statuses().await
}

#[tauri::command]
pub(super) async fn local_app_runtime_statuses(
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
pub(super) fn connector_lifecycle_snapshots(
    state: tauri::State<'_, DesktopState>,
) -> Vec<ConnectorLifecycleSnapshot> {
    state.connector_lifecycles.list()
}

#[tauri::command]
pub(super) async fn start_registered_service(
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
pub(super) async fn stop_registered_service(
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

#[tauri::command]
pub(super) fn connector_app_ui_url(
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
        local_app_ui_host(&endpoint.token, &record.manifest.app_id),
        endpoint.port,
        endpoint.token,
        record.manifest.app_id
    ))
}

#[tauri::command]
pub(super) async fn list_connector_apps(
    _state: tauri::State<'_, DesktopState>,
) -> Result<Vec<ConnectorSummary>, String> {
    list_connectors().map_err(|err| err.to_string())
}

#[tauri::command]
pub(super) fn request_desktop_permission(
    permission: String,
) -> Result<DesktopPermissionStatus, String> {
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
pub(super) fn open_desktop_permission_settings(permission: String) -> Result<(), String> {
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

pub(super) fn read_desktop_permission_status() -> DesktopPermissionStatus {
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
