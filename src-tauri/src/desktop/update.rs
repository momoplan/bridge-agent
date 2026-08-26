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

#[tauri::command]
pub(super) async fn check_app_update() -> Result<AppUpdateStatus, String> {
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

async fn download_update_bytes(
    app: &tauri::AppHandle,
    update: &tauri_plugin_updater::Update,
    version: &str,
    asset_name: Option<&String>,
) -> Result<Vec<u8>, String> {
    emit_app_update_progress(
        app,
        AppUpdateProgress {
            phase: "downloading".to_string(),
            message: "正在下载更新包".to_string(),
            version: Some(version.to_string()),
            asset_name: asset_name.cloned(),
            downloaded_bytes: Some(0),
            total_bytes: None,
            downloaded_path: None,
        },
    );
    let progress_app = app.clone();
    let progress_version = version.to_string();
    let progress_asset_name = asset_name.cloned();
    let mut downloaded_bytes = 0_u64;
    let mut last_progress_at = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    update
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
        .map_err(|err| format!("下载或校验官方更新失败: {err}"))
}

async fn install_downloaded_update(
    update: &tauri_plugin_updater::Update,
    update_bytes: &[u8],
    state: &tauri::State<'_, DesktopState>,
) -> Result<(), String> {
    let runtime_was_active = state.runtime.snapshot().await.status != RuntimeStatus::Stopped;
    state
        .runtime
        .stop()
        .await
        .map_err(|err| format!("安装更新前停止 Agent Runtime 失败: {err}"))?;
    let Err(install_err) = update.install(update_bytes) else {
        return Ok(());
    };
    if !runtime_was_active {
        return Err(format!("安装官方更新失败: {install_err}"));
    }
    match start_runtime_from_saved_config(&state.runtime, &state.config_path).await {
        Ok(_) => Err(format!(
            "安装官方更新失败，Agent Runtime 已恢复运行: {install_err}"
        )),
        Err(recovery_err) => Err(format!(
            "安装官方更新失败，且 Agent Runtime 恢复失败: install={install_err}; recovery={recovery_err}"
        )),
    }
}

pub(super) fn legacy_config_requires_unified_app_id_migration(
    config_path: &Path,
) -> anyhow::Result<bool> {
    let config_dir = resolve_config_base_dir(config_path);
    if config_dir.join(UNIFIED_APP_ID_MIGRATION_LEDGER).is_file() {
        return Ok(true);
    }
    if !config_path.is_file() {
        return Ok(false);
    }
    let content = fs::read(config_path)
        .with_context(|| format!("failed to read config {}", config_path.display()))?;
    let document: Value = serde_json::from_slice(&content)
        .with_context(|| format!("failed to parse config {}", config_path.display()))?;
    Ok(document
        .get("local_apps")
        .and_then(Value::as_array)
        .is_some_and(|apps| apps.iter().any(|app| app.get("connectorId").is_some())))
}

pub(super) fn migrate_legacy_config_before_startup(config_path: &Path) -> anyhow::Result<bool> {
    if !legacy_config_requires_unified_app_id_migration(config_path)? {
        return Ok(false);
    }
    let binary = bundled_unified_app_id_migration_path().with_context(|| {
        format!(
            "missing unified app ID migration artifact {}",
            unified_app_id_migration_binary_name()
        )
    })?;
    let config_dir = resolve_config_base_dir(config_path);
    let output = Command::new(&binary)
        .arg("--config-dir")
        .arg(&config_dir)
        .arg("--config")
        .arg(config_path)
        .arg("--host-already-stopped")
        .output()
        .with_context(|| format!("failed to start migration artifact {}", binary.display()))?;
    if output.status.success() {
        return Ok(true);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        anyhow::bail!(
            "unified app ID migration failed with exit status {}",
            output.status
        );
    }
    anyhow::bail!("unified app ID migration failed: {detail}")
}

pub(super) fn bundled_unified_app_id_migration_path() -> Option<PathBuf> {
    bundled_resource_binary_path(unified_app_id_migration_binary_name())
}

pub(super) fn unified_app_id_migration_binary_name() -> &'static str {
    if cfg!(windows) {
        "bridge-agent-unified-app-id-migration.exe"
    } else {
        UNIFIED_APP_ID_MIGRATION_BINARY
    }
}

pub(super) fn emit_app_update_progress(app: &tauri::AppHandle, progress: AppUpdateProgress) {
    let _ = app.emit(UPDATE_PROGRESS_EVENT, progress);
}

pub(super) fn parse_release_version(tag_name: &str) -> Result<Version, String> {
    let normalized = tag_name
        .trim()
        .strip_prefix("bridge-agent-v")
        .or_else(|| tag_name.trim().strip_prefix('v'))
        .unwrap_or(tag_name.trim());
    Version::parse(normalized).map_err(|err| err.to_string())
}

pub(super) fn configured_update_api_url() -> Result<String, String> {
    let Some(url) = option_env!("BRIDGE_AGENT_UPDATE_API_URL")
        .map(str::trim)
        .filter(|url| !url.is_empty())
    else {
        return Err("当前应用未配置更新服务地址，请使用正式发布包或重新构建客户端。".to_string());
    };
    Ok(url.to_string())
}

pub(super) fn release_page_url(release: &UpdateReleaseResponse) -> Option<String> {
    release
        .release_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn release_version(release: &UpdateReleaseResponse) -> Result<Version, String> {
    let raw_version = release
        .version
        .as_deref()
        .or(release.tag_name.as_deref())
        .ok_or_else(|| "更新服务未返回最新版本号".to_string())?;
    parse_release_version(raw_version).map_err(|err| format!("最新版本号无效: {err}"))
}

pub(super) fn release_force_update_required(
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

pub(super) async fn fetch_latest_release() -> Result<UpdateReleaseResponse, String> {
    let update_api_url = configured_update_api_url()?;
    let client = Client::builder()
        .connect_timeout(UPDATE_CONNECT_TIMEOUT)
        .timeout(STARTUP_UPDATE_CHECK_TIMEOUT)
        .build()
        .map_err(|err| format!("初始化更新检查客户端失败: {err}"))?;
    let response = client
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

pub(super) fn current_update_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

pub(super) fn select_tauri_updater_asset(
    release: &UpdateReleaseResponse,
) -> Option<&UpdateReleaseAsset> {
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
