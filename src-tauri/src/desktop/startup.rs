use super::*;

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[derive(Clone, Debug)]
pub(super) struct StartupDiagnostics {
    pub(super) primary_path: PathBuf,
    pub(super) fallback_path: PathBuf,
}

impl StartupDiagnostics {
    pub(super) fn bootstrap() -> Self {
        Self {
            primary_path: std::env::temp_dir().join(STARTUP_LOG_FILE_NAME),
            fallback_path: std::env::temp_dir().join(STARTUP_LOG_FILE_NAME),
        }
    }

    pub(super) fn for_config_path(config_path: &Path) -> Self {
        let primary_path = resolve_config_base_dir(config_path).join(STARTUP_LOG_FILE_NAME);
        Self {
            primary_path,
            fallback_path: std::env::temp_dir().join(STARTUP_LOG_FILE_NAME),
        }
    }

    pub(super) fn info(&self, message: impl AsRef<str>) {
        self.write("INFO", message.as_ref());
    }

    pub(super) fn warn(&self, message: impl AsRef<str>) {
        self.write("WARN", message.as_ref());
    }

    pub(super) fn error(&self, message: impl AsRef<str>) {
        self.write("ERROR", message.as_ref());
    }

    pub(super) fn write(&self, level: &str, message: &str) {
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

pub(super) fn append_startup_log_line(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())
}

pub(super) fn install_panic_diagnostics(diagnostics: StartupDiagnostics) {
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

pub(super) fn log_startup_environment(diagnostics: &StartupDiagnostics, config_path: &Path) {
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
pub(super) fn is_probably_macos_translocated_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path.starts_with("/private/var/folders/") || path.starts_with("/var/folders/")
}

#[cfg(not(target_os = "macos"))]
pub(super) fn is_probably_macos_translocated_path(_path: &Path) -> bool {
    false
}

pub(super) fn interactive_restart_marker_path(config_path: &Path) -> PathBuf {
    resolve_config_base_dir(config_path).join(INTERACTIVE_RESTART_MARKER_FILE_NAME)
}

pub(super) fn request_interactive_restart(config_path: &Path) -> Result<(), String> {
    let marker_path = interactive_restart_marker_path(config_path);
    if let Some(parent) = marker_path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建客户端重启状态目录失败: {err}"))?;
    }
    fs::write(&marker_path, b"interactive\n")
        .map_err(|err| format!("写入客户端前台重启标记失败: {err}"))
}

pub(super) fn consume_interactive_restart_request(config_path: &Path) -> Result<bool, String> {
    let marker_path = interactive_restart_marker_path(config_path);
    match fs::remove_file(&marker_path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(format!("读取客户端前台重启标记失败: {err}")),
    }
}

pub(super) fn prepare_config_for_auto_start_with<F>(
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

pub(super) fn prepare_config_for_auto_start(
    config_path: &Path,
) -> anyhow::Result<(AgentConfig, ConnectorSyncReport)> {
    prepare_config_for_auto_start_with(config_path, sync_installed_connectors_report)
}

pub(super) fn auto_start_agent(
    runtime: AgentRuntimeManager,
    connector_lifecycles: ConnectorLifecycleManager,
    connector_processes: ConnectorProcessManager,
    config_path: PathBuf,
    startup_health: StartupHealthManager,
    diagnostics: StartupDiagnostics,
) {
    startup_health.set_component("agent_runtime", "Agent 运行时", "starting", None);
    tauri::async_runtime::spawn(async move {
        let Some(config) = prepare_and_start_authorized_runtime(
            &runtime,
            &config_path,
            &startup_health,
            &diagnostics,
        )
        .await
        else {
            return;
        };
        start_automatic_connectors(
            &runtime,
            &connector_lifecycles,
            &connector_processes,
            &config_path,
            &diagnostics,
            &config,
        )
        .await;
    });
}

async fn prepare_and_start_authorized_runtime(
    runtime: &AgentRuntimeManager,
    config_path: &Path,
    startup_health: &StartupHealthManager,
    diagnostics: &StartupDiagnostics,
) -> Option<AgentConfig> {
    diagnostics.info(format!(
        "auto start preparing config at {}",
        config_path.display()
    ));
    let (config, connector_sync) = match prepare_config_for_auto_start(config_path) {
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
            return None;
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
        diagnostics.info("bridge-agent runtime auto start skipped: device is not authorized yet");
        startup_health.set_component(
            "agent_runtime",
            "Agent 运行时",
            "ready",
            Some("设备尚未授权，未自动连接".to_string()),
        );
        return None;
    }
    diagnostics.info("bridge-agent config loaded for auto start");
    diagnostics.info("automatic agent start requested");
    if let Err(err) = start_runtime_from_saved_config(runtime, config_path).await {
        diagnostics.error(format!(
            "failed to auto start bridge-agent runtime: {err:#}"
        ));
        startup_health.set_component(
            "agent_runtime",
            "Agent 运行时",
            "degraded",
            Some(err.to_string()),
        );
        return None;
    } else {
        diagnostics.info("automatic agent start request completed");
        if let Some(detail) = connector_sync_failure_detail {
            startup_health.set_component("agent_runtime", "Agent 运行时", "degraded", Some(detail));
        } else {
            startup_health.set_component("agent_runtime", "Agent 运行时", "ready", None);
        }
    }
    Some(config)
}

async fn start_automatic_connectors(
    runtime: &AgentRuntimeManager,
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    config_path: &Path,
    diagnostics: &StartupDiagnostics,
    config: &AgentConfig,
) {
    let automatic_app_ids = config
        .local_apps
        .iter()
        .filter(|app| bridge_agent::local_app_starts_automatically(app))
        .map(|app| app.app_id.clone())
        .collect::<Vec<_>>();
    for app_id in automatic_app_ids {
        diagnostics.info(format!(
            "automatic connector start requested: app_id={app_id}"
        ));
        let bundled_cli = bundled_baijimu_cli_path();
        match start_connector_with_lifecycle(
            connector_lifecycles,
            connector_processes,
            config_path,
            &app_id,
            "自动启动应用",
            bundled_cli.as_deref(),
        )
        .await
        {
            Ok(_) => diagnostics.info(format!(
                "automatic connector start completed: app_id={app_id}"
            )),
            Err(err) => diagnostics.warn(format!(
                "automatic connector start failed: app_id={app_id} error={err}"
            )),
        }
    }
    if let Err(err) = runtime.apply_capabilities_from_path(config_path).await {
        diagnostics.warn(format!(
            "failed to refresh capabilities after automatic connector startup: {err:#}"
        ));
    }
}

pub(super) fn config_is_authorized(config: &AgentConfig) -> bool {
    config.platform.workspace_id.is_some() && !config.relay.token.trim().is_empty()
}

pub(super) struct DesktopBusinessStartup {
    pub(super) app: tauri::AppHandle,
    pub(super) launch_mode: DesktopLaunchMode,
    pub(super) runtime: AgentRuntimeManager,
    pub(super) connector_lifecycles: ConnectorLifecycleManager,
    pub(super) connector_processes: ConnectorProcessManager,
    pub(super) config_path: PathBuf,
    pub(super) startup_health: StartupHealthManager,
    pub(super) diagnostics: StartupDiagnostics,
    pub(super) local_app_ui: Arc<RwLock<Option<LocalAppUiEndpoint>>>,
    pub(super) local_apps: LocalAppsChangeNotifier,
    pub(super) registered_services: RegisteredServiceMonitor,
    pub(super) registered_service_request_rx: RegisteredServiceMonitorReceiver,
}

pub(super) fn mark_business_startup_skipped(startup_health: &StartupHealthManager, reason: &str) {
    for (id, label) in [
        ("local_app_ui_server", "本地应用界面服务"),
        ("managed_cli", "Baijimu CLI"),
        ("agent_runtime", "Agent 运行时"),
    ] {
        startup_health.set_component(id, label, "skipped", Some(reason.to_string()));
    }
}

pub(super) async fn run_update_check_with_retry<F, Fut>(
    mut check: F,
    attempt_timeout: Duration,
    retry_delays: &[Duration],
    diagnostics: Option<&StartupDiagnostics>,
) -> Result<AppUpdateStatus, UpdateCheckFailure>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<AppUpdateStatus, UpdateCheckFailure>>,
{
    let total_attempts = retry_delays.len() + 1;
    for attempt_index in 0..total_attempts {
        let attempt = attempt_index + 1;
        let failure = match timeout(attempt_timeout, check()).await {
            Ok(Ok(status)) => return Ok(status),
            Ok(Err(failure)) => failure,
            Err(_) => UpdateCheckFailure::temporarily_unavailable(format!(
                "第 {attempt} 次更新检查在 {} 秒后超时",
                attempt_timeout.as_secs()
            )),
        };
        let Some(delay) = retry_delays.get(attempt_index) else {
            return Err(failure);
        };
        if !failure.retryable() {
            return Err(failure);
        }
        if let Some(diagnostics) = diagnostics {
            diagnostics.warn(format!(
                "startup update check attempt {attempt}/{total_attempts} failed; retrying in {}ms: {}",
                delay.as_millis(),
                failure.detail
            ));
        }
        tokio::time::sleep(*delay).await;
    }
    unreachable!("update retry loop always returns on its final attempt")
}

fn start_update_service_recovery(
    app: tauri::AppHandle,
    startup_health: StartupHealthManager,
    diagnostics: StartupDiagnostics,
) {
    tauri::async_runtime::spawn(async move {
        let mut last_failure = None;
        for (attempt_index, delay) in STARTUP_UPDATE_RECOVERY_DELAYS.iter().enumerate() {
            tokio::time::sleep(*delay).await;
            diagnostics.info(format!(
                "background update service recovery attempt {}/{} started",
                attempt_index + 1,
                STARTUP_UPDATE_RECOVERY_DELAYS.len()
            ));
            let result = timeout(STARTUP_UPDATE_ATTEMPT_TIMEOUT, resolve_app_update_status()).await;
            match result {
                Ok(Ok(status)) => {
                    diagnostics.info("background update service recovery completed");
                    apply_updater_health_status(&startup_health, &status);
                    if startup_update_decision(&status) == StartupUpdateDecision::RequireUpdate {
                        diagnostics.warn(
                            "required update discovered after offline-capable startup; showing the main window",
                        );
                        show_main_window(
                            &app,
                            Some(&diagnostics),
                            MainWindowOpenReason::RequiredUpdate,
                        );
                    }
                    return;
                }
                Ok(Err(failure)) => {
                    diagnostics.warn(format!(
                        "background update service recovery attempt {} failed: {}",
                        attempt_index + 1,
                        failure.detail
                    ));
                    if !failure.retryable() {
                        apply_updater_failure_health(&startup_health, &failure, false);
                        return;
                    }
                    last_failure = Some(failure);
                }
                Err(_) => {
                    let failure = UpdateCheckFailure::temporarily_unavailable(format!(
                        "后台更新检查在 {} 秒后超时",
                        STARTUP_UPDATE_ATTEMPT_TIMEOUT.as_secs()
                    ));
                    diagnostics.warn(format!(
                        "background update service recovery attempt {} timed out",
                        attempt_index + 1
                    ));
                    last_failure = Some(failure);
                }
            }
        }
        if let Some(failure) = last_failure {
            diagnostics.warn(
                "background update service recovery exhausted; keeping offline-capable status",
            );
            apply_updater_failure_health(&startup_health, &failure, false);
        }
    });
}

pub(super) fn start_desktop_business_after_update_gate(startup: DesktopBusinessStartup) {
    tauri::async_runtime::spawn(run_desktop_business_after_update_gate(startup));
}

async fn startup_update_gate_allows_business(
    app: &tauri::AppHandle,
    launch_mode: DesktopLaunchMode,
    startup_health: &StartupHealthManager,
    diagnostics: &StartupDiagnostics,
) -> bool {
    startup_health.set_component(
        "updater",
        "官方更新器",
        "starting",
        Some("正在启动任何配置或业务组件之前检查官方更新".to_string()),
    );
    diagnostics.info("startup update gate check started before configuration migration");
    let update_status = timeout(
        STARTUP_UPDATE_CHECK_TIMEOUT,
        run_update_check_with_retry(
            resolve_app_update_status,
            STARTUP_UPDATE_ATTEMPT_TIMEOUT,
            STARTUP_UPDATE_RETRY_DELAYS,
            Some(diagnostics),
        ),
    )
    .await;
    match update_status {
        Ok(Ok(status)) => match startup_update_decision(&status) {
            StartupUpdateDecision::RequireUpdate => {
                let detail = updater_required_detail(&status);
                diagnostics.warn(format!(
                    "startup update gate blocked configuration and business startup: {detail}"
                ));
                startup_health.set_component("updater", "官方更新器", "degraded", Some(detail));
                startup_health.set_component(
                    "config_migration",
                    "配置迁移",
                    "skipped",
                    Some("必须先完成客户端升级".to_string()),
                );
                startup_health.set_component(
                    "registered_service_monitor",
                    "服务状态监控",
                    "skipped",
                    Some("必须先完成客户端升级".to_string()),
                );
                mark_business_startup_skipped(startup_health, "必须先完成客户端升级");
                if launch_mode == DesktopLaunchMode::BackgroundAutostart {
                    show_main_window(app, Some(diagnostics), MainWindowOpenReason::RequiredUpdate);
                }
                return false;
            }
            StartupUpdateDecision::Continue => {
                diagnostics.info("startup update gate completed; configuration startup allowed");
                apply_updater_health_status(startup_health, &status);
            }
        },
        Ok(Err(failure)) => {
            diagnostics.warn(format!(
                "startup update gate check failed; continuing in offline-capable mode: {}",
                failure.detail
            ));
            apply_updater_failure_health(startup_health, &failure, failure.retryable());
            if failure.retryable() {
                start_update_service_recovery(
                    app.clone(),
                    startup_health.clone(),
                    diagnostics.clone(),
                );
            }
        }
        Err(_) => {
            diagnostics
                .warn("startup update gate check timed out; continuing in offline-capable mode");
            let failure = UpdateCheckFailure::temporarily_unavailable(format!(
                "启动更新检查在 {} 秒后超时",
                STARTUP_UPDATE_CHECK_TIMEOUT.as_secs()
            ));
            apply_updater_failure_health(startup_health, &failure, true);
            start_update_service_recovery(app.clone(), startup_health.clone(), diagnostics.clone());
        }
    }

    true
}

fn migrate_desktop_config(
    config_path: &Path,
    startup_health: &StartupHealthManager,
    diagnostics: &StartupDiagnostics,
) -> bool {
    diagnostics.info(format!(
        "configuration migration started after startup update gate: config={}",
        config_path.display()
    ));
    match migrate_legacy_config_before_startup(config_path) {
        Ok(true) => {
            diagnostics.info(format!(
                "legacy app ID configuration migration completed before startup: config={}",
                config_path.display()
            ));
            startup_health.set_component(
                "config_migration",
                "配置迁移",
                "ready",
                Some("旧版本本地应用配置已完成迁移".to_string()),
            );
            true
        }
        Ok(false) => {
            startup_health.set_component("config_migration", "配置迁移", "ready", None);
            true
        }
        Err(err) => {
            diagnostics.error(format!(
                "failed to migrate legacy app ID configuration before startup: {err:#}"
            ));
            startup_health.set_component(
                "config_migration",
                "配置迁移",
                "degraded",
                Some(format!("旧版本配置迁移失败: {err:#}")),
            );
            false
        }
    }
}

async fn run_desktop_business_after_update_gate(startup: DesktopBusinessStartup) {
    let DesktopBusinessStartup {
        app,
        launch_mode,
        runtime,
        connector_lifecycles,
        connector_processes,
        config_path,
        startup_health,
        diagnostics,
        local_app_ui,
        local_apps,
        registered_services,
        registered_service_request_rx,
    } = startup;

    if !startup_update_gate_allows_business(&app, launch_mode, &startup_health, &diagnostics).await
    {
        return;
    }

    let startup_migration_ready =
        migrate_desktop_config(&config_path, &startup_health, &diagnostics);
    if startup_migration_ready {
        start_runtime_monitor_with_tauri(
            app.clone(),
            config_path.clone(),
            connector_lifecycles.clone(),
            connector_processes.clone(),
            registered_service_request_rx,
        );
        startup_health.set_component("registered_service_monitor", "服务状态监控", "ready", None);
    } else {
        startup_health.set_component(
            "registered_service_monitor",
            "服务状态监控",
            "skipped",
            Some("配置迁移失败，未启动服务状态监控".to_string()),
        );
    }

    if startup_health.safe_mode() || !startup_migration_ready {
        let skip_reason = if startup_migration_ready {
            "安全模式下未自动启动"
        } else {
            "配置迁移失败，未自动启动"
        };
        mark_business_startup_skipped(&startup_health, skip_reason);
        return;
    }

    start_local_app_ui_server(
        local_app_ui,
        startup_health.clone(),
        LocalAppUiServerDependencies {
            diagnostics: diagnostics.clone(),
            config_path: config_path.clone(),
            runtime: runtime.clone(),
            connector_lifecycles: connector_lifecycles.clone(),
            connector_processes: connector_processes.clone(),
            registered_services: registered_services.clone(),
            local_apps,
        },
    );
    bootstrap_bundled_baijimu_cli(startup_health.clone(), diagnostics.clone());
    auto_start_agent(
        runtime,
        connector_lifecycles,
        connector_processes,
        config_path,
        startup_health,
        diagnostics,
    );
}

pub(super) fn install_bundled_baijimu_cli(diagnostics: &StartupDiagnostics) -> anyhow::Result<()> {
    if unified_app_id_managed_cli_root().is_dir() && !legacy_managed_cli_root().exists() {
        let skill_path = codex_skill::install_bundled()?;
        diagnostics.info(format!(
            "managed Baijimu CLI bootstrap skipped after unified app ID migration: root={} codex_skill={}",
            unified_app_id_managed_cli_root().display(),
            skill_path.display()
        ));
        return Ok(());
    }
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

pub(super) fn legacy_managed_cli_root() -> PathBuf {
    managed_cli_root_for_app_id("com.baijimu.cli")
}

pub(super) fn unified_app_id_managed_cli_root() -> PathBuf {
    managed_cli_root_for_app_id("baijimu-cli")
}

pub(super) fn managed_cli_root_for_app_id(app_id: &str) -> PathBuf {
    #[cfg(windows)]
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("Baijimu")
            .join("apps")
            .join(app_id);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("baijimu")
        .join("apps")
        .join(app_id)
}

pub(super) fn bootstrap_bundled_baijimu_cli(
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

pub(super) fn bundled_baijimu_cli_path() -> Option<PathBuf> {
    bundled_resource_binary_path(baijimu_cli_binary_name())
}

pub(super) fn bundled_resource_binary_path(binary_name: &str) -> Option<PathBuf> {
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

pub(super) fn baijimu_cli_binary_name() -> &'static str {
    if cfg!(windows) {
        "baijimu.exe"
    } else {
        "baijimu"
    }
}
