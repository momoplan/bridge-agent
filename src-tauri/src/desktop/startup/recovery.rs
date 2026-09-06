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
