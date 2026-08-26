use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConnectorAppInstallDocument {
    pub(super) install: ConnectorInstallResult,
    pub(super) start: Option<ConnectorStartResult>,
    pub(super) setup: Option<Value>,
    pub(super) config: ConfigDocument,
}

#[tauri::command]
pub(super) async fn check_connector_app_update(
    state: tauri::State<'_, DesktopState>,
    app_id: String,
) -> Result<ConnectorAppUpdateStatus, String> {
    let app_id = app_id.trim();
    if app_id.is_empty() {
        return Err("应用 ID 不能为空".to_string());
    }
    let installed = show_connector(app_id).map_err(|err| err.to_string())?;
    if installed.review_status != "PUBLISHED" {
        return Err("该应用版本未在公开市场发布，请使用原注册来源同步".to_string());
    }
    let market_app = fetch_market_connector_apps(&state.config_path)
        .await?
        .into_iter()
        .find(|app| app.app_id == app_id)
        .ok_or_else(|| "市场中找不到该应用".to_string())?;
    validate_market_host_compatibility(&market_app)?;
    validate_market_app_identity(&market_app, app_id)?;
    let checksum = required_market_checksum(&market_app)?;
    let resolved_source =
        resolve_connector_source(&market_app.source, false, Some(&checksum), None).await?;
    let latest_manifest =
        load_connector_manifest(resolved_source.path()).map_err(|err| err.to_string())?;
    if latest_manifest.app_id != installed.manifest.app_id {
        return Err(format!(
            "更新来源应用 ID 不匹配：当前 `{}`，来源 `{}`",
            installed.manifest.app_id, latest_manifest.app_id
        ));
    }
    if latest_manifest.version != market_app.version {
        return Err(format!(
            "市场版本与安装包清单不匹配：市场 `{}`，安装包 `{}`",
            market_app.version, latest_manifest.version
        ));
    }

    Ok(ConnectorAppUpdateStatus {
        app_id: installed.manifest.app_id,
        name: latest_manifest.name,
        current_version: installed.manifest.version.clone(),
        latest_version: latest_manifest.version.clone(),
        update_available: connector_version_is_newer(
            &latest_manifest.version,
            &installed.manifest.version,
        ),
        source: market_app.source,
    })
}

#[tauri::command]
pub(super) fn start_connector_app_install(
    state: tauri::State<'_, DesktopState>,
    request: StartConnectorAppInstallRequest,
    on_event: tauri::ipc::Channel<LocalAppInstallTask>,
) -> Result<LocalAppInstallTask, String> {
    let StartConnectorAppInstallRequest {
        operation,
        replace,
        app_id,
        name,
        version,
        accept_unreviewed,
    } = request;
    let identity = RegisteredAppVersionIdentity::parse(app_id, version)?;
    let display_name = name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| identity.app_id.clone());
    let task = state.local_app_install_tasks.create(
        operation,
        Some(identity.app_id.clone()),
        display_name,
        Some(identity.version.to_string()),
    )?;
    let manager = state.local_app_install_tasks.clone();
    let reporter = LocalAppInstallProgressReporter {
        manager,
        task_id: task.task_id.clone(),
        on_event: Some(on_event),
    };
    reporter.send(task.clone());
    let config_path = state.config_path.clone();
    let runtime = state.runtime.clone();
    let connector_lifecycles = state.connector_lifecycles.clone();
    let connector_processes = state.connector_processes.clone();
    let registered_services = state.registered_services.clone();
    let local_apps = state.local_apps.clone();
    tauri::async_runtime::spawn(async move {
        let result = install_connector_app_with_context(
            &config_path,
            &runtime,
            &connector_lifecycles,
            &connector_processes,
            &registered_services,
            ConnectorInstallOptions {
                identity,
                replace,
                start: true,
                accept_unreviewed,
                progress: Some(reporter.clone()),
            },
        )
        .await;
        match result {
            Ok(document) => {
                reporter.update(|task| {
                    task.app_id = Some(document.install.app_id.clone());
                    task.name = document.install.name.clone();
                    task.version = Some(document.install.version.clone());
                    task.phase = LocalAppInstallTaskPhase::Succeeded;
                    task.progress_percent = Some(100);
                    task.downloaded_bytes = None;
                    task.total_bytes = None;
                    task.message = task.operation.succeeded_message().to_string();
                    task.error = None;
                });
                local_apps.notify(
                    match operation {
                        LocalAppInstallTaskOperation::Install => LocalAppsChangeOperation::Install,
                        LocalAppInstallTaskOperation::Upgrade => LocalAppsChangeOperation::Upgrade,
                        LocalAppInstallTaskOperation::Sync => LocalAppsChangeOperation::Sync,
                    },
                    &document.install.app_id,
                );
            }
            Err(error) => {
                reporter.update(|task| {
                    task.phase = LocalAppInstallTaskPhase::Failed;
                    task.progress_percent = None;
                    task.downloaded_bytes = None;
                    task.total_bytes = None;
                    task.message = task.operation.failed_message().to_string();
                    task.error = Some(error);
                });
            }
        }
    });
    Ok(task)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StartConnectorAppInstallRequest {
    pub(super) operation: LocalAppInstallTaskOperation,
    pub(super) replace: bool,
    pub(super) app_id: String,
    pub(super) name: Option<String>,
    pub(super) version: String,
    pub(super) accept_unreviewed: bool,
}

#[tauri::command]
pub(super) fn list_connector_app_install_tasks(
    state: tauri::State<'_, DesktopState>,
) -> Vec<LocalAppInstallTask> {
    state.local_app_install_tasks.list()
}

struct PreparedConnectorInstall {
    options: ConnectorInstallOptions,
    resolved_source: ResolvedConnectorSource,
    candidate_manifest: ConnectorManifest,
    existing: Option<ConnectorSummary>,
    restart_after_replace: bool,
    bundled_cli: Option<PathBuf>,
    provenance: ConnectorInstallProvenance,
    operation_kind: ConnectorOperationKind,
}

struct ConnectorInstallExecution<'a> {
    config_path: &'a Path,
    runtime_manager: &'a AgentRuntimeManager,
    connector_lifecycles: &'a ConnectorLifecycleManager,
    connector_processes: &'a ConnectorProcessManager,
    registered_services: &'a RegisteredServiceMonitor,
}

pub(super) async fn install_connector_app_with_context(
    config_path: &Path,
    runtime_manager: &AgentRuntimeManager,
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    registered_services: &RegisteredServiceMonitor,
    options: ConnectorInstallOptions,
) -> Result<ConnectorAppInstallDocument, String> {
    let prepared = prepare_connector_install(config_path, connector_processes, options).await?;
    let operation = connector_lifecycles
        .begin(
            &prepared.candidate_manifest.app_id,
            prepared.operation_kind,
            Some(prepared.candidate_manifest.version.clone()),
            if prepared.operation_kind == ConnectorOperationKind::Upgrade {
                "正在切换应用版本"
            } else {
                "正在安装应用"
            },
        )
        .await?;
    let result = execute_connector_install(
        config_path,
        runtime_manager,
        connector_lifecycles,
        connector_processes,
        registered_services,
        &operation,
        &prepared,
    )
    .await;
    finish_connector_install_lifecycle(
        config_path,
        connector_lifecycles,
        connector_processes,
        operation,
        &prepared.candidate_manifest,
        result,
    )
    .await
}

async fn prepare_connector_install(
    config_path: &Path,
    connector_processes: &ConnectorProcessManager,
    options: ConnectorInstallOptions,
) -> Result<PreparedConnectorInstall, String> {
    if let Some(progress) = options.progress.as_ref() {
        progress.report(
            LocalAppInstallTaskPhase::Resolving,
            Some(5),
            "正在解析平台注册版本",
        );
    }
    ensure_config_exists(config_path).map_err(|err| err.to_string())?;
    let registered = fetch_registered_install_source(config_path, &options.identity).await?;
    ensure_registered_install_is_accepted(&registered, options.accept_unreviewed)?;
    let resolved_source = resolve_connector_source(
        &registered.source,
        false,
        Some(&registered.checksum),
        options.progress.as_ref(),
    )
    .await?;
    if let Some(progress) = options.progress.as_ref() {
        progress.report(
            LocalAppInstallTaskPhase::Verifying,
            Some(60),
            "正在校验应用清单与平台身份",
        );
    }
    let candidate_manifest =
        load_connector_manifest(resolved_source.path()).map_err(|err| err.to_string())?;
    if let Some(progress) = options.progress.as_ref() {
        progress.identity(
            &candidate_manifest.app_id,
            &candidate_manifest.name,
            &candidate_manifest.version,
        );
    }
    validate_registered_candidate_identity(&registered, &candidate_manifest)?;
    let bundled_cli = bundled_baijimu_cli_path();
    managed_tool_dependency::ensure_ready(
        &candidate_manifest,
        bridge_agent::ConnectorManagedToolDependencyPhase::Install,
        bundled_cli.as_deref(),
    )
    .await
    .map_err(|err| format!("应用依赖检查失败: {err:#}"))?;
    let existing = list_connectors()
        .map_err(|err| err.to_string())?
        .into_iter()
        .find(|connector| connector.app_id == candidate_manifest.app_id);
    let restart_after_replace = if options.replace {
        match existing.as_ref() {
            Some(connector) => {
                connector_local_app_is_healthy(config_path, &connector.app_id, connector_processes)
                    .await?
            }
            None => false,
        }
    } else {
        false
    };
    let provenance = ConnectorInstallProvenance::registered(
        &registered.source,
        &registered.review_status,
        &registered.checksum,
    )
    .map_err(|err| err.to_string())?;
    let operation_kind = if existing.is_some() && options.replace {
        ConnectorOperationKind::Upgrade
    } else {
        ConnectorOperationKind::Install
    };
    Ok(PreparedConnectorInstall {
        options,
        resolved_source,
        candidate_manifest,
        existing,
        restart_after_replace,
        bundled_cli,
        provenance,
        operation_kind,
    })
}

fn validate_registered_candidate_identity(
    registered: &RegisteredInstallSource,
    candidate: &ConnectorManifest,
) -> Result<(), String> {
    if candidate.app_id == registered.identity.app_id
        && candidate.version == registered.identity.version.to_string()
    {
        return Ok(());
    }
    Err(format!(
        "注册版本与安装包清单不匹配：注册 `{}@{}`，安装包 `{}@{}`",
        registered.identity.app_id,
        registered.identity.version,
        candidate.app_id,
        candidate.version
    ))
}

async fn execute_connector_install(
    config_path: &Path,
    runtime_manager: &AgentRuntimeManager,
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    registered_services: &RegisteredServiceMonitor,
    operation: &ConnectorLifecycleOperation,
    prepared: &PreparedConnectorInstall,
) -> Result<ConnectorAppInstallDocument, String> {
    report_install_progress(
        prepared.options.progress.as_ref(),
        LocalAppInstallTaskPhase::Installing,
        72,
        "正在安装并注册应用",
    );
    connector_lifecycles.advance(
        &prepared.candidate_manifest.app_id,
        &operation.id,
        prepared.operation_kind.lifecycle(),
        "正在安装并注册应用",
        Some(72),
    )?;
    let install = install_connector_candidate(config_path, connector_processes, prepared).await?;
    let execution = ConnectorInstallExecution {
        config_path,
        runtime_manager,
        connector_lifecycles,
        connector_processes,
        registered_services,
    };
    start_and_refresh_installed_connector(&execution, operation, prepared, install).await
}

fn report_install_progress(
    progress: Option<&LocalAppInstallProgressReporter>,
    phase: LocalAppInstallTaskPhase,
    percent: u8,
    message: &str,
) {
    if let Some(progress) = progress {
        progress.report(phase, Some(percent), message);
    }
}

async fn install_connector_candidate(
    config_path: &Path,
    connector_processes: &ConnectorProcessManager,
    prepared: &PreparedConnectorInstall,
) -> Result<ConnectorInstallResult, String> {
    if prepared.options.replace {
        if let Some(connector) = prepared.existing.as_ref() {
            connector_processes
                .stop_if_managed(&connector.app_id, config_path)
                .await?;
        }
    }
    match install_connector_from_path_with_provenance(
        prepared.resolved_source.path(),
        config_path,
        prepared.options.replace,
        prepared.provenance.clone(),
    ) {
        Ok(install) => Ok(install),
        Err(err) if prepared.restart_after_replace => {
            start_connector_and_wait(
                connector_processes,
                config_path,
                &prepared.candidate_manifest.app_id,
                "恢复旧版应用",
                prepared.bundled_cli.as_deref(),
            )
            .await
            .map_err(|restart_err| {
                format!("应用升级失败: {err:#}；恢复旧版进程也失败: {restart_err:#}")
            })?;
            Err(err.to_string())
        }
        Err(err) => Err(err.to_string()),
    }
}

async fn start_and_refresh_installed_connector(
    execution: &ConnectorInstallExecution<'_>,
    operation: &ConnectorLifecycleOperation,
    prepared: &PreparedConnectorInstall,
    install: ConnectorInstallResult,
) -> Result<ConnectorAppInstallDocument, String> {
    let should_start = prepared.options.start || prepared.restart_after_replace;
    let started = if should_start {
        report_install_progress(
            prepared.options.progress.as_ref(),
            LocalAppInstallTaskPhase::Starting,
            88,
            "应用已安装，正在启动并检查运行状态",
        );
        execution.connector_lifecycles.advance(
            &prepared.candidate_manifest.app_id,
            &operation.id,
            ConnectorLifecycleState::Starting,
            "应用已安装，正在启动并检查运行状态",
            Some(88),
        )?;
        Some(
            start_connector_and_wait(
                execution.connector_processes,
                execution.config_path,
                &install.app_id,
                "启动新版应用",
                prepared.bundled_cli.as_deref(),
            )
            .await
            .map_err(|err| format!("新版应用已安装，但启动失败: {err}"))?,
        )
    } else {
        None
    };
    report_install_progress(
        prepared.options.progress.as_ref(),
        LocalAppInstallTaskPhase::Finalizing,
        96,
        "正在刷新本地应用能力",
    );
    execution.connector_lifecycles.advance(
        &prepared.candidate_manifest.app_id,
        &operation.id,
        if should_start {
            ConnectorLifecycleState::Starting
        } else {
            prepared.operation_kind.lifecycle()
        },
        "正在刷新本地应用能力",
        Some(96),
    )?;
    let runtime = execution
        .runtime_manager
        .apply_capabilities_from_path(execution.config_path)
        .await
        .map_err(|err| err.to_string())?;
    execution.registered_services.request_refresh();
    let config = load_agent_config(execution.config_path).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    Ok(ConnectorAppInstallDocument {
        install,
        start: started,
        setup: None,
        config: ConfigDocument {
            config_path: execution.config_path.display().to_string(),
            manifest_preview,
            config: config_for_ui(&config)?,
            runtime,
        },
    })
}

async fn finish_connector_install_lifecycle(
    config_path: &Path,
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    operation: ConnectorLifecycleOperation,
    candidate: &ConnectorManifest,
    result: Result<ConnectorAppInstallDocument, String>,
) -> Result<ConnectorAppInstallDocument, String> {
    match result {
        Ok(document) => {
            if document.start.is_some() {
                connector_lifecycles.complete_ready(
                    operation,
                    Some(document.install.version.clone()),
                    connector_processes.managed_pid(&candidate.app_id).await,
                    "应用已启动并通过就绪检查",
                )?;
            } else {
                connector_lifecycles.complete_stopped(
                    operation,
                    Some(document.install.version.clone()),
                    "应用已安装，等待启动",
                )?;
            }
            Ok(document)
        }
        Err(error) => {
            let recovered =
                connector_local_app_is_healthy(config_path, &candidate.app_id, connector_processes)
                    .await
                    .unwrap_or(false);
            if recovered {
                let observed_version = show_connector(&candidate.app_id)
                    .ok()
                    .map(|record| record.manifest.version);
                connector_lifecycles.complete_ready(
                    operation,
                    observed_version,
                    connector_processes.managed_pid(&candidate.app_id).await,
                    "升级失败，已恢复原运行版本",
                )?;
            } else {
                connector_lifecycles.fail(operation, &error)?;
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub(super) async fn start_connector_app(
    state: tauri::State<'_, DesktopState>,
    app_id: String,
) -> Result<ConnectorStartResult, String> {
    let bundled_cli = bundled_baijimu_cli_path();
    let result = start_connector_with_lifecycle(
        &state.connector_lifecycles,
        &state.connector_processes,
        &state.config_path,
        app_id.trim(),
        "启动应用",
        bundled_cli.as_deref(),
    )
    .await;
    state.registered_services.request_refresh();
    result
}

#[tauri::command]
pub(super) async fn stop_connector_app(
    state: tauri::State<'_, DesktopState>,
    app_id: String,
) -> Result<ConnectorStartResult, String> {
    let result = stop_connector_with_lifecycle(
        &state.connector_lifecycles,
        &state.connector_processes,
        &state.config_path,
        app_id.trim(),
        "停止应用",
    )
    .await;
    state.registered_services.request_refresh();
    result
}

#[tauri::command]
pub(super) async fn uninstall_connector_app(
    state: tauri::State<'_, DesktopState>,
    app_id: String,
    force: Option<bool>,
) -> Result<ConfigDocument, ConnectorUninstallCommandError> {
    let app_id = app_id.trim().to_string();
    let document = uninstall_connector_app_with_context(
        &state.config_path,
        &state.runtime,
        &state.connector_lifecycles,
        &state.connector_processes,
        &state.registered_services,
        app_id.clone(),
        force.unwrap_or(false),
    )
    .await?;
    state
        .local_apps
        .notify(LocalAppsChangeOperation::Uninstall, &app_id);
    Ok(document)
}

pub(super) async fn uninstall_connector_app_with_context(
    config_path: &Path,
    runtime_manager: &AgentRuntimeManager,
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    registered_services: &RegisteredServiceMonitor,
    app_id: String,
    force: bool,
) -> Result<ConfigDocument, ConnectorUninstallCommandError> {
    let app_id = app_id.trim().to_string();
    let operation = connector_lifecycles
        .begin(
            &app_id,
            ConnectorOperationKind::Uninstall,
            None,
            "正在停止并卸载应用",
        )
        .await
        .map_err(|message| ConnectorUninstallCommandError::Failed { message })?;
    let result = async {
        let managed_stop = connector_processes
        .stop_if_managed(&app_id, config_path)
        .await;
    if let Err(error) = managed_stop {
        if !force {
            return Err(ConnectorUninstallCommandError::StopFailed { message: error });
        }
        log::warn!(
            "continuing explicit forced uninstall for connector `{}` after host-managed stop failed: {}",
            app_id,
            error
        );
    }
    uninstall_connector_with_options(&app_id, config_path, ConnectorUninstallOptions { force })
        .map_err(|error| {
        let stop_failed = is_connector_package_stop_error(&error);
        let message = format!("{error:#}");
        if stop_failed && !force {
            ConnectorUninstallCommandError::StopFailed { message }
        } else {
            ConnectorUninstallCommandError::Failed { message }
        }
    })?;
    let runtime = runtime_manager
        .apply_capabilities_from_path(config_path)
        .await
        .map_err(|error| ConnectorUninstallCommandError::Failed {
            message: error.to_string(),
        })?;
    registered_services.request_refresh();
    let config =
        load_agent_config(config_path).map_err(|error| ConnectorUninstallCommandError::Failed {
            message: format!("{error:#}"),
        })?;
    let manifest_preview =
        manifest_preview_json(&config).map_err(|error| ConnectorUninstallCommandError::Failed {
            message: format!("{error:#}"),
        })?;
    Ok(ConfigDocument {
        config_path: config_path.display().to_string(),
        manifest_preview,
        config: config_for_ui(&config)
            .map_err(|message| ConnectorUninstallCommandError::Failed { message })?,
        runtime,
    })
    }
    .await;
    match result {
        Ok(document) => {
            connector_lifecycles
                .complete_absent(operation)
                .map_err(|message| ConnectorUninstallCommandError::Failed { message })?;
            Ok(document)
        }
        Err(error) => {
            let _ = connector_lifecycles.fail(operation, error.message());
            Err(error)
        }
    }
}

pub(super) async fn run_start_command(
    service: String,
    start_command: ServiceStartCommand,
) -> Result<StartRegisteredServiceResult, String> {
    match start_command {
        ServiceStartCommand::ShellCommand {
            command,
            cwd,
            mut env,
            timeout_secs,
        } => {
            if command.is_empty() || command[0].trim().is_empty() {
                return Err(format!("服务 `{service}` 的启动命令为空"));
            }
            enrich_user_command_environment(command.first().map(String::as_str), &mut env);
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

            let stdout_capture = tempfile::NamedTempFile::new()
                .map_err(|err| format!("创建服务 `{service}` 标准输出文件失败: {err}"))?;
            let stderr_capture = tempfile::NamedTempFile::new()
                .map_err(|err| format!("创建服务 `{service}` 标准错误文件失败: {err}"))?;
            process
                .stdout(std::process::Stdio::from(stdout_capture.reopen().map_err(
                    |err| format!("打开服务 `{service}` 标准输出文件失败: {err}"),
                )?))
                .stderr(std::process::Stdio::from(stderr_capture.reopen().map_err(
                    |err| format!("打开服务 `{service}` 标准错误文件失败: {err}"),
                )?));

            let timeout_secs = timeout_secs.unwrap_or(15).max(1);
            let mut child = process
                .spawn()
                .map_err(|err| format!("启动服务 `{service}` 失败: {err}"))?;
            let (status, timed_out) = match timeout(Duration::from_secs(timeout_secs), child.wait())
                .await
            {
                Ok(Ok(status)) => (Some(status), false),
                Ok(Err(err)) => return Err(format!("等待服务 `{service}` 启动命令失败: {err}")),
                Err(_) => {
                    let _ = child.start_kill();
                    let _ = timeout(Duration::from_secs(3), child.wait()).await;
                    (None, true)
                }
            };
            let stdout = read_lifecycle_capture(&service, "stdout", &stdout_capture)?;
            let mut stderr = read_lifecycle_capture(&service, "stderr", &stderr_capture)?;
            if timed_out {
                if !stderr.is_empty() {
                    stderr.push('\n');
                }
                stderr.push_str(&format!("timed out after {timeout_secs}s"));
            }
            Ok(StartRegisteredServiceResult {
                service,
                success: status.as_ref().is_some_and(|status| status.success()),
                exit_code: status.and_then(|status| status.code()),
                stdout,
                stderr,
                timed_out,
            })
        }
    }
}

pub(super) fn read_lifecycle_capture(
    service: &str,
    stream_name: &str,
    capture: &tempfile::NamedTempFile,
) -> Result<String, String> {
    let file = capture
        .reopen()
        .map_err(|err| format!("读取服务 `{service}` {stream_name} 文件失败: {err}"))?;
    let mut bytes = Vec::new();
    file.take(LIFECYCLE_OUTPUT_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("收集服务 `{service}` {stream_name} 失败: {err}"))?;
    if bytes.len() as u64 > LIFECYCLE_OUTPUT_MAX_BYTES {
        bytes.truncate(LIFECYCLE_OUTPUT_MAX_BYTES as usize);
        bytes.extend_from_slice(b"\n[output truncated by Bridge Agent]");
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConnectorAppUpdateStatus {
    pub(super) app_id: String,
    pub(super) name: String,
    pub(super) current_version: String,
    pub(super) latest_version: String,
    pub(super) update_available: bool,
    pub(super) source: String,
}
