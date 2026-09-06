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
