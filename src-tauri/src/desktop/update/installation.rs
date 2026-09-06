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

pub(super) async fn fetch_latest_release() -> Result<UpdateReleaseResponse, UpdateCheckFailure> {
    let update_api_url = configured_update_api_url().map_err(UpdateCheckFailure::configuration)?;
    let client = Client::builder()
        .connect_timeout(UPDATE_CONNECT_TIMEOUT)
        .timeout(STARTUP_UPDATE_CHECK_TIMEOUT)
        .build()
        .map_err(|err| {
            UpdateCheckFailure::configuration(format!(
                "初始化更新检查客户端失败: {}",
                format_error_chain(&err)
            ))
        })?;
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
        .map_err(|err| {
            let detail = format!("检查更新失败: {}", format_error_chain(&err));
            if err.is_builder() {
                UpdateCheckFailure::configuration(detail)
            } else {
                UpdateCheckFailure::temporarily_unavailable(detail)
            }
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let payload = response.text().await.unwrap_or_default();
        let detail = format!("检查更新失败 ({status}): {payload}");
        return if status.is_server_error()
            || status == reqwest::StatusCode::REQUEST_TIMEOUT
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            Err(UpdateCheckFailure::temporarily_unavailable(detail))
        } else {
            Err(UpdateCheckFailure::invalid_response(detail))
        };
    }

    response.json().await.map_err(|err| {
        UpdateCheckFailure::invalid_response(format!(
            "解析最新版本信息失败: {}",
            format_error_chain(&err)
        ))
    })
}

fn format_error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut messages = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        let message = cause.to_string();
        if messages.last() != Some(&message) {
            messages.push(message);
        }
        source = cause.source();
    }
    messages.join("；原因: ")
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
