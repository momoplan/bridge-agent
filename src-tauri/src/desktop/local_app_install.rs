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
    let selected = installed
        .install_source
        .as_ref()
        .ok_or("该安装记录缺少可验证的来源身份，不能自动匹配公开市场")?;
    let consumer = market_consumer::market_consumer(&state.config_path).await?;
    let (market_key, listing_id, source) = match selected {
        local_app_contract::InstallSource::Market {
            market_key,
            listing_id,
            source,
            ..
        } => (market_key, listing_id, source),
        _ => return Err("该应用属于环境安装来源".into()),
    };
    let listing = consumer
        .listings()
        .await?
        .into_iter()
        .find(|listing| {
            &listing.market_key == market_key
                && &listing.listing_id == listing_id
                && listing.frozen_version.source.application == source.application
        })
        .ok_or("市场中找不到该来源的应用")?;
    let upgrade = bridge_agent::market_distribution::select_upgrade(Some(selected), &listing)
        .map_err(|error| error.to_string())?;
    let latest_version = listing.frozen_version.source.version.to_string();
    Ok(ConnectorAppUpdateStatus {
        app_id: installed.manifest.app_id,
        name: installed.manifest.name,
        current_version: installed.manifest.version,
        latest_version,
        update_available: upgrade.is_some(),
        source: String::new(),
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
        install_source,
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
                install_source,
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
    pub(super) install_source: Option<local_app_contract::InstallSource>,
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
    let (resolved_source, provenance) = resolve_install_package(config_path, &options).await?;
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
    provenance
        .validate_manifest(&candidate_manifest)
        .map_err(|error| error.to_string())?;
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

include!("local_app_install/execution.rs");

async fn resolve_install_package(
    config_path: &Path,
    options: &ConnectorInstallOptions,
) -> Result<(ResolvedConnectorSource, ConnectorInstallProvenance), String> {
    Ok(match options.install_source.as_ref() {
        Some(selection @ local_app_contract::InstallSource::Market { .. }) => {
            if options.accept_unreviewed {
                return Err("公开市场安装不能同时声明未审核来源".into());
            }
            market_consumer::resolve_market_install(
                config_path,
                &options.identity,
                selection,
                options.progress.as_ref(),
            )
            .await?
        }
        Some(_) => return Err("环境安装来源必须由环境注册服务解析".into()),
        None => {
            if !options.accept_unreviewed {
                return Err("公开市场安装需要完整 installSource，请重新选择市场条目".into());
            }
            let registered =
                fetch_registered_install_source(config_path, &options.identity, true).await?;
            ensure_registered_install_is_accepted(&registered, true)?;
            let resolved = resolve_connector_source(
                &registered.source,
                false,
                Some(&registered.checksum),
                options.progress.as_ref(),
            )
            .await?;
            let manifest =
                load_connector_manifest(resolved.path()).map_err(|error| error.to_string())?;
            validate_registered_candidate_identity(&registered, &manifest)?;
            let provenance = ConnectorInstallProvenance::registered(
                &registered.source,
                &registered.review_status,
                &registered.checksum,
            )
            .map_err(|error| error.to_string())?;
            (resolved, provenance)
        }
    })
}
