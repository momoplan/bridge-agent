use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AppVersionInfo {
    pub(super) current_version: String,
    pub(super) current_target: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AppUpdateStatus {
    pub(super) current_version: String,
    pub(super) latest_version: Option<String>,
    pub(super) update_available: bool,
    pub(super) force_update_required: bool,
    pub(super) minimum_supported_version: Option<String>,
    pub(super) force_update_message: Option<String>,
    pub(super) release_url: Option<String>,
    pub(super) release_name: Option<String>,
    pub(super) published_at: Option<String>,
    pub(super) current_target: String,
    pub(super) auto_download_available: bool,
    pub(super) asset_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StartupUpdateDecision {
    Continue,
    RequireUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdateCheckFailureKind {
    TemporarilyUnavailable,
    Configuration,
    InvalidResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UpdateCheckFailure {
    pub(super) kind: UpdateCheckFailureKind,
    pub(super) detail: String,
}

impl UpdateCheckFailure {
    pub(super) fn temporarily_unavailable(detail: impl Into<String>) -> Self {
        Self {
            kind: UpdateCheckFailureKind::TemporarilyUnavailable,
            detail: detail.into(),
        }
    }

    pub(super) fn configuration(detail: impl Into<String>) -> Self {
        Self {
            kind: UpdateCheckFailureKind::Configuration,
            detail: detail.into(),
        }
    }

    pub(super) fn invalid_response(detail: impl Into<String>) -> Self {
        Self {
            kind: UpdateCheckFailureKind::InvalidResponse,
            detail: detail.into(),
        }
    }

    pub(super) fn retryable(&self) -> bool {
        self.kind == UpdateCheckFailureKind::TemporarilyUnavailable
    }

    pub(super) fn health_status(&self) -> &'static str {
        if self.retryable() {
            "unavailable"
        } else {
            "degraded"
        }
    }
}

pub(super) fn startup_update_decision(status: &AppUpdateStatus) -> StartupUpdateDecision {
    if status.force_update_required {
        StartupUpdateDecision::RequireUpdate
    } else {
        StartupUpdateDecision::Continue
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AppUpdateInstallResult {
    pub(super) status: String,
    pub(super) version: String,
    pub(super) asset_name: Option<String>,
    pub(super) downloaded_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AppUpdateProgress {
    pub(super) phase: String,
    pub(super) message: String,
    pub(super) version: Option<String>,
    pub(super) asset_name: Option<String>,
    pub(super) downloaded_bytes: Option<u64>,
    pub(super) total_bytes: Option<u64>,
    pub(super) downloaded_path: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateReleaseResponse {
    #[serde(default, alias = "tag_name")]
    pub(super) tag_name: Option<String>,
    #[serde(default)]
    pub(super) version: Option<String>,
    #[serde(default, alias = "html_url")]
    pub(super) release_url: Option<String>,
    #[serde(default, alias = "name")]
    pub(super) release_name: Option<String>,
    #[serde(default, alias = "published_at")]
    pub(super) published_at: Option<String>,
    #[serde(default, alias = "update_available")]
    pub(super) update_available: Option<bool>,
    #[serde(default, alias = "force_update")]
    pub(super) force_update: Option<bool>,
    #[serde(
        default,
        alias = "minimum_supported_version",
        alias = "minSupportedVersion"
    )]
    pub(super) minimum_supported_version: Option<String>,
    #[serde(default, alias = "force_update_message")]
    pub(super) force_update_message: Option<String>,
    #[serde(default)]
    pub(super) assets: Vec<UpdateReleaseAsset>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateReleaseAsset {
    pub(super) name: String,
    #[serde(default)]
    pub(super) signature: Option<String>,
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    pub(super) fn AXIsProcessTrusted() -> bool;
    pub(super) fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
    pub(super) fn CGPreflightListenEventAccess() -> bool;
    pub(super) fn CGPreflightPostEventAccess() -> bool;
    pub(super) fn CGPreflightScreenCaptureAccess() -> bool;
    pub(super) fn CGRequestPostEventAccess() -> bool;
    pub(super) fn CGRequestScreenCaptureAccess() -> bool;
}

pub(super) async fn restart_agent_from_saved_config(
    state: &tauri::State<'_, DesktopState>,
) -> anyhow::Result<RuntimeSnapshot> {
    state.runtime.stop().await?;
    start_runtime_from_saved_config(&state.runtime, &state.config_path).await
}

pub(super) async fn start_runtime_from_saved_config(
    runtime: &AgentRuntimeManager,
    config_path: &Path,
) -> anyhow::Result<RuntimeSnapshot> {
    runtime.start_from_path(config_path).await
}

#[tauri::command]
pub(super) fn app_version() -> AppVersionInfo {
    AppVersionInfo {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        current_target: current_update_target(),
    }
}

#[tauri::command]
pub(super) fn open_app_uninstaller() -> Result<(), String> {
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

pub(super) async fn resolve_app_update_status() -> Result<AppUpdateStatus, UpdateCheckFailure> {
    let current_version = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|err| UpdateCheckFailure::configuration(format!("当前版本号无效: {err}")))?;
    let release = fetch_latest_release().await?;
    let latest_version = release_version(&release).map_err(UpdateCheckFailure::invalid_response)?;
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

pub(super) fn updater_ready_detail(status: &AppUpdateStatus) -> Option<String> {
    if status.update_available {
        status
            .latest_version
            .as_deref()
            .map(|version| format!("发现可选更新 {version}，启动后可随时安装"))
    } else {
        None
    }
}

pub(super) fn updater_required_detail(status: &AppUpdateStatus) -> String {
    let target = status
        .latest_version
        .as_deref()
        .or(status.minimum_supported_version.as_deref())
        .unwrap_or("最新版本");
    format!(
        "当前版本 {} 已停止支持，必须先升级到 {target}",
        status.current_version
    )
}

pub(super) fn apply_updater_health_status(
    startup_health: &StartupHealthManager,
    status: &AppUpdateStatus,
) {
    match startup_update_decision(status) {
        StartupUpdateDecision::RequireUpdate => startup_health.set_component(
            "updater",
            "官方更新器",
            "degraded",
            Some(updater_required_detail(status)),
        ),
        StartupUpdateDecision::Continue => startup_health.set_component(
            "updater",
            "官方更新器",
            "ready",
            updater_ready_detail(status),
        ),
    }
}

pub(super) fn apply_updater_failure_health(
    startup_health: &StartupHealthManager,
    failure: &UpdateCheckFailure,
    automatic_retry_pending: bool,
) {
    let detail = if failure.retryable() {
        let next_step = if automatic_retry_pending {
            "客户端已按离线模式启动，并将在后台自动重试"
        } else {
            "客户端可继续离线运行，请稍后重新检查"
        };
        format!("更新服务暂时不可用，{next_step}: {}", failure.detail)
    } else {
        failure.detail.clone()
    };
    startup_health.set_component(
        "updater",
        "官方更新器",
        failure.health_status(),
        Some(detail),
    );
}

#[tauri::command]
pub(super) async fn check_app_update(
    state: tauri::State<'_, DesktopState>,
) -> Result<AppUpdateStatus, String> {
    match resolve_app_update_status().await {
        Ok(status) => {
            apply_updater_health_status(&state.startup_health, &status);
            Ok(status)
        }
        Err(failure) => {
            apply_updater_failure_health(&state.startup_health, &failure, false);
            Err(failure.detail)
        }
    }
}

#[tauri::command]
pub(super) async fn install_app_update(
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

    let update_bytes =
        download_update_bytes(&app, &update, &update_version, asset_name.as_ref()).await?;

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

    install_downloaded_update(&update, &update_bytes, &state).await?;

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

include!("update/installation.rs");
