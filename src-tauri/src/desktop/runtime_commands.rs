use super::*;

#[derive(Debug, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub(super) enum CommandError {
    RuntimeAlreadyRunning { conflict: Box<RuntimeLockConflict> },
    Message { message: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub(super) enum ConnectorUninstallCommandError {
    #[serde(rename = "connector_uninstall_stop_failed")]
    StopFailed { message: String },
    #[serde(rename = "connector_uninstall_failed")]
    Failed { message: String },
}

impl ConnectorUninstallCommandError {
    pub(super) fn message(&self) -> &str {
        match self {
            Self::StopFailed { message } | Self::Failed { message } => message,
        }
    }
}

impl From<anyhow::Error> for CommandError {
    fn from(err: anyhow::Error) -> Self {
        if let Some(conflict) = err.downcast_ref::<RuntimeLockConflict>() {
            return Self::RuntimeAlreadyRunning {
                conflict: Box::new(conflict.clone()),
            };
        }
        Self::Message {
            message: err.to_string(),
        }
    }
}

pub(super) fn command_error_message(message: impl Into<String>) -> CommandError {
    CommandError::Message {
        message: message.into(),
    }
}

#[derive(Serialize)]
pub(super) struct ConfigDocument {
    pub(super) config_path: String,
    pub(super) manifest_preview: String,
    pub(super) config: Value,
    pub(super) runtime: RuntimeSnapshot,
}

#[derive(Serialize)]
pub(super) struct ConfigRecoveryDocument {
    config_path: String,
    archived_path: Option<String>,
    manifest_preview: String,
    config: Value,
    runtime: RuntimeSnapshot,
}

#[tauri::command]
pub(super) async fn baijimu_cli_status() -> Result<managed_tool::ManagedToolStatus, String> {
    let source = bundled_baijimu_cli_path();
    managed_tool::inspect(source.as_deref()).map_err(|err| err.to_string())
}

#[tauri::command]
pub(super) async fn install_baijimu_cli_update(
    state: tauri::State<'_, DesktopState>,
    install_source: local_app_contract::InstallSource,
) -> Result<managed_tool::ManagedToolStatus, String> {
    let consumer = market_consumer::market_consumer(&state.config_path).await?;
    let listing = consumer
        .reader
        .version(&consumer.credential, &install_source)
        .await
        .map_err(|error| error.to_string())?;
    validate_market_host_compatibility(&market_listing_presentation(listing.clone())?)?;
    let artifact = bridge_agent::market_distribution::select_artifact(
        &listing.frozen_version,
        normalized_platform(),
        std::env::consts::ARCH,
    )
    .map_err(|error| error.to_string())?
    .ok_or("当前平台没有可用的工具制品")?;
    if artifact.size_bytes > 128 * 1024 * 1024 {
        return Err("工具制品超过 128 MiB".into());
    }
    let mut response = consumer
        .reader
        .artifact(&consumer.credential, &install_source, artifact.artifact_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "读取工具制品失败")? {
        if bytes.len() as u64 + chunk.len() as u64 > artifact.size_bytes {
            return Err("工具制品大小不符".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let manifest: Value = serde_json::from_str(listing.frozen_version.content.manifest.as_json())
        .map_err(|error| error.to_string())?;
    let manifest_artifact = manifest
        .get("artifacts")
        .and_then(Value::as_array)
        .and_then(|artifacts| {
            artifacts.iter().find(|entry| {
                entry.get("platform").and_then(Value::as_str) == Some(artifact.platform.as_str())
                    && entry
                        .get("arch")
                        .and_then(Value::as_str)
                        .unwrap_or("universal")
                        == artifact.architecture
            })
        });
    let archive_path = manifest_artifact
        .as_ref()
        .and_then(|value| value.get("archivePath"))
        .and_then(Value::as_str);
    managed_tool::install_market_package(&bytes, artifact, install_source, &listing, archive_path)
        .map_err(|error| error.to_string())?;
    codex_skill::install_bundled().map_err(|err| err.to_string())?;
    let bundled = bundled_baijimu_cli_path();
    managed_tool::inspect(bundled.as_deref()).map_err(|err| err.to_string())
}

#[tauri::command]
pub(super) async fn rollback_baijimu_cli() -> Result<managed_tool::ManagedToolStatus, String> {
    managed_tool::rollback().map_err(|err| err.to_string())?;
    codex_skill::install_bundled().map_err(|err| err.to_string())?;
    let bundled = bundled_baijimu_cli_path();
    managed_tool::inspect(bundled.as_deref()).map_err(|err| err.to_string())
}

#[tauri::command]
pub(super) async fn load_config(
    state: tauri::State<'_, DesktopState>,
) -> Result<ConfigDocument, String> {
    ensure_config_exists(&state.config_path).map_err(|err| format!("{err:#}"))?;
    let config = load_agent_config(&state.config_path).map_err(|err| format!("{err:#}"))?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| format!("{err:#}"))?;
    let runtime = state.runtime.snapshot().await;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config: config_for_ui(&config)?,
        runtime,
    })
}

#[tauri::command]
pub(super) async fn python_runtime_status(
    state: tauri::State<'_, DesktopState>,
    python_path: Option<String>,
) -> Result<bridge_agent::PythonRuntimeStatus, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let mut config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    if let Some(path) = python_path {
        config.runtime.python_path = if path.trim().is_empty() {
            None
        } else {
            Some(path)
        };
    }
    Ok(inspect_python_runtime(&config.runtime))
}

#[tauri::command]
pub(super) async fn save_config(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
) -> Result<ConfigDocument, String> {
    let previous = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    if !previous.relay.token.is_empty()
        && (bridge_agent::config::environment::api_base(&previous.platform.base_url)
            != bridge_agent::config::environment::api_base(&config.platform.base_url)
            || previous.platform.workspace_id != config.platform.workspace_id
            || previous.platform.environment_key != config.platform.environment_key)
    {
        return Err("切换环境或工作区请先完成浏览器授权，原连接尚未修改".into());
    }
    save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    let runtime = state.runtime.snapshot().await;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config: config_for_ui(&config)?,
        runtime,
    })
}

#[tauri::command]
pub(super) async fn save_service(
    state: tauri::State<'_, DesktopState>,
    service_index: usize,
    service: ServiceConfig,
    apply_to_runtime: bool,
) -> Result<ConfigDocument, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let mut config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    if service_index > config.services.len() {
        return Err(format!("服务索引 {service_index} 已超出当前配置范围"));
    }
    if service_index == config.services.len() {
        config.services.push(service);
    } else {
        config.services[service_index] = service;
    }
    save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    let runtime = if apply_to_runtime {
        state
            .runtime
            .apply_capabilities_from_path(&state.config_path)
            .await
            .map_err(|err| err.to_string())?
    } else {
        state.runtime.snapshot().await
    };
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config: config_for_ui(&config)?,
        runtime,
    })
}

#[tauri::command]
pub(super) async fn delete_service(
    state: tauri::State<'_, DesktopState>,
    service_index: usize,
    apply_to_runtime: bool,
) -> Result<ConfigDocument, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    let mut config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    if service_index >= config.services.len() {
        return Err(format!("服务索引 {service_index} 已超出当前配置范围"));
    }
    config.services.remove(service_index);
    save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    let runtime = if apply_to_runtime {
        state
            .runtime
            .apply_capabilities_from_path(&state.config_path)
            .await
            .map_err(|err| err.to_string())?
    } else {
        state.runtime.snapshot().await
    };
    let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
    let manifest_preview = manifest_preview_json(&config).map_err(|err| err.to_string())?;
    Ok(ConfigDocument {
        config_path: state.config_path.display().to_string(),
        manifest_preview,
        config: config_for_ui(&config)?,
        runtime,
    })
}

#[tauri::command]
pub(super) async fn start_agent(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
) -> Result<RuntimeSnapshot, CommandError> {
    state
        .startup_health
        .diagnostics
        .info("manual agent start requested from desktop UI");
    save_agent_config(&state.config_path, &config).map_err(|err| CommandError::Message {
        message: err.to_string(),
    })?;
    state
        .runtime
        .start_from_path(&state.config_path)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub(super) async fn stop_agent(
    state: tauri::State<'_, DesktopState>,
) -> Result<RuntimeSnapshot, String> {
    state.runtime.stop().await.map_err(|err| err.to_string())
}

#[tauri::command]
pub(super) async fn stop_conflicting_runtime(
    lock_path: String,
    pid: u32,
    agent_id: String,
    config_path: String,
) -> Result<(), CommandError> {
    terminate_runtime_lock_owner(Path::new(&lock_path), pid, &agent_id, &config_path)
        .map_err(CommandError::from)
}

#[tauri::command]
pub(super) async fn runtime_snapshot(
    state: tauri::State<'_, DesktopState>,
) -> Result<RuntimeSnapshot, String> {
    Ok(state.runtime.snapshot().await)
}

#[tauri::command]
pub(super) async fn apply_saved_config_to_runtime(
    state: tauri::State<'_, DesktopState>,
) -> Result<RuntimeSnapshot, String> {
    ensure_config_exists(&state.config_path).map_err(|err| err.to_string())?;
    state
        .runtime
        .apply_capabilities_from_path(&state.config_path)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub(super) async fn test_capability(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
    service: String,
    method: String,
    arguments: Value,
    timeout_secs: Option<u64>,
) -> Result<InvokeResult, String> {
    let service = service.trim();
    let method = method.trim();
    if service.is_empty() {
        return Err("服务名不能为空".to_string());
    }
    if method.is_empty() {
        return Err("能力名不能为空".to_string());
    }

    let config_base_dir = resolve_config_base_dir(&state.config_path);
    let registry = ServiceRegistry::from_config_checked(&config, &config_base_dir)
        .await
        .map_err(|err| format!("构建本地能力运行环境失败: {err}"))?;
    let request_id = format!("desktop-test-{}", now_ms());

    Ok(registry
        .invoke(
            request_id,
            service,
            method,
            arguments,
            timeout_secs.filter(|value| *value > 0),
        )
        .await)
}

include!("runtime_commands/capabilities.rs");
