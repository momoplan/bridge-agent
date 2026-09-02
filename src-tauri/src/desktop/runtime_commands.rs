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
    version: String,
    source: String,
    checksum: String,
    archive_path: Option<String>,
) -> Result<managed_tool::ManagedToolStatus, String> {
    managed_tool::install_update(&source, &version, &checksum, archive_path.as_deref())
        .await
        .map_err(|err| err.to_string())?;
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

#[tauri::command]
pub(super) async fn test_local_app_capability(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
    app_id: String,
    method: String,
    arguments: Value,
    timeout_secs: Option<u64>,
) -> Result<InvokeResult, String> {
    let app_id = app_id.trim();
    let method = method.trim();
    if app_id.is_empty() {
        return Err("appId 不能为空".to_string());
    }
    if method.is_empty() {
        return Err("能力名不能为空".to_string());
    }

    let config_base_dir = resolve_config_base_dir(&state.config_path);
    let registry = ServiceRegistry::from_config_checked(&config, &config_base_dir)
        .await
        .map_err(|err| format!("构建本地应用运行环境失败: {err}"))?;
    let request_id = format!("desktop-local-app-test-{}", now_ms());

    Ok(registry
        .invoke_local_app(
            request_id,
            None,
            app_id,
            method,
            arguments,
            timeout_secs.filter(|value| *value > 0),
        )
        .await)
}

#[tauri::command]
pub(super) async fn list_logs(
    state: tauri::State<'_, DesktopState>,
    limit: Option<usize>,
) -> Result<Vec<bridge_agent::LogEntry>, String> {
    Ok(state.runtime.logs(limit.unwrap_or(200)).await)
}

#[tauri::command]
pub(super) fn set_runtime_log_streaming(state: tauri::State<'_, DesktopState>, enabled: bool) {
    state
        .runtime_log_streaming_requested
        .store(enabled, Ordering::SeqCst);
    let enabled = enabled && state.main_window_visible.load(Ordering::SeqCst);
    state.runtime_log_streaming.store(enabled, Ordering::SeqCst);
}

#[tauri::command]
pub(super) async fn clear_logs(state: tauri::State<'_, DesktopState>) -> Result<u64, String> {
    Ok(state.runtime.clear_logs().await)
}

#[tauri::command]
pub(super) async fn reset_example_config(
    state: tauri::State<'_, DesktopState>,
) -> Result<ConfigDocument, String> {
    clear_relay_credentials(&state.config_path).map_err(|err| err.to_string())?;
    let config = AgentConfig::example();
    save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
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
pub(super) async fn recover_invalid_config(
    state: tauri::State<'_, DesktopState>,
) -> Result<ConfigRecoveryDocument, String> {
    let recovery = reset_invalid_config(&state.config_path).map_err(|err| err.to_string())?;
    let manifest_preview =
        manifest_preview_json(&recovery.config).map_err(|err| err.to_string())?;
    let runtime = state.runtime.snapshot().await;
    Ok(ConfigRecoveryDocument {
        config_path: state.config_path.display().to_string(),
        archived_path: recovery
            .archived_path
            .map(|path| path.display().to_string()),
        manifest_preview,
        config: config_for_ui(&recovery.config)?,
        runtime,
    })
}

#[tauri::command]
pub(super) fn open_in_browser(url: String) -> Result<(), String> {
    open::that(url).map_err(|err| err.to_string())
}

pub(super) fn describe_upstream_http_failure(
    status: reqwest::StatusCode,
    content_type: &str,
    body: &str,
) -> String {
    let trimmed = body.trim();
    if let Some(description) = bridge_agent::describe_cmodel_http_outcome(
        status,
        trimmed.as_bytes(),
        "platform authorization",
    ) {
        return description;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            return format!("HTTP {status}: {message}");
        }
    }

    let lower_content_type = content_type.to_ascii_lowercase();
    let lower_body_start = trimmed
        .chars()
        .take(80)
        .collect::<String>()
        .to_ascii_lowercase();
    if lower_content_type.contains("text/html")
        || lower_body_start.starts_with("<!doctype html")
        || lower_body_start.starts_with("<html")
    {
        return format!(
            "HTTP {status}: 平台授权接口返回了 HTML 错误页，可能是网关路由、服务异常或请求体超过平台限制。请确认 Base URL 为 https://api.baijimu.com/lowcode3，并检查平台授权服务日志。"
        );
    }

    if lower_content_type.contains("xml") || lower_body_start.starts_with("<?xml") {
        if let (Some(code), Some(message)) = (
            extract_xml_tag(trimmed, "Code"),
            extract_xml_tag(trimmed, "Message"),
        ) {
            return format!("HTTP {status}: {code} - {message}");
        }
        return format!(
            "HTTP {status}: 平台授权接口返回了 XML 错误，请检查 Baijimu Base URL 和网关路由。"
        );
    }

    if trimmed.is_empty() {
        return format!("HTTP {status}: 空响应");
    }
    format!("HTTP {status}: {}", truncate_for_error(trimmed, 240))
}

pub(super) fn extract_xml_tag(body: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    let after_start = body.split_once(&start)?.1;
    let value = after_start.split_once(&end)?.0.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

pub(super) fn truncate_for_error(value: &str, limit: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= limit {
        return compact;
    }
    let prefix = compact.chars().take(limit).collect::<String>();
    format!("{prefix}...")
}

#[tauri::command]
pub(super) fn open_in_edge(url: String) -> Result<(), String> {
    open_url_in_edge(&url)
}

#[cfg(windows)]
pub(super) fn open_url_in_edge(url: &str) -> Result<(), String> {
    let mut candidates = Vec::new();
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        candidates
            .push(PathBuf::from(program_files).join("Microsoft\\Edge\\Application\\msedge.exe"));
    }
    if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
        candidates.push(
            PathBuf::from(program_files_x86).join("Microsoft\\Edge\\Application\\msedge.exe"),
        );
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        candidates
            .push(PathBuf::from(local_app_data).join("Microsoft\\Edge\\Application\\msedge.exe"));
    }

    for candidate in candidates {
        if candidate.is_file() {
            let mut edge = Command::new(candidate);
            configure_desktop_command(&mut edge);
            edge.arg(url)
                .spawn()
                .map_err(|err| format!("打开 Microsoft Edge 失败: {err}"))?;
            return Ok(());
        }
    }

    let mut edge = Command::new("msedge");
    configure_desktop_command(&mut edge);
    edge.arg(url)
        .spawn()
        .map_err(|err| format!("未找到 Microsoft Edge，请复制授权链接后手动粘贴到浏览器: {err}"))?;
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn open_url_in_edge(url: &str) -> Result<(), String> {
    open::that(url).map_err(|err| err.to_string())
}

pub(super) fn forward_runtime_events(
    app: tauri::AppHandle,
    runtime: AgentRuntimeManager,
    runtime_log_streaming: Arc<AtomicBool>,
) {
    let mut events = runtime.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(RuntimeEvent::SnapshotChanged(snapshot)) => {
                    let _ = app.emit(RUNTIME_SNAPSHOT_EVENT, snapshot);
                }
                Ok(RuntimeEvent::LogAppended(entry)) => {
                    if runtime_log_streaming.load(Ordering::SeqCst) {
                        let _ = app.emit(RUNTIME_LOG_EVENT, entry);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let snapshot = runtime.snapshot().await;
                    let _ = app.emit(RUNTIME_SNAPSHOT_EVENT, snapshot);
                    if runtime_log_streaming.load(Ordering::SeqCst) {
                        let logs = runtime.logs(200).await;
                        let _ = app.emit(RUNTIME_LOGS_SNAPSHOT_EVENT, logs);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_official_cmodel_failure_details() {
        let body = r#"{
            "contractVersion": "1.0.0",
            "errorCode": "WORKSPACE_AUTHORIZATION_REQUIRED",
            "value": "不能展示的遗留字段",
            "data": {
                "message": "当前工作区尚未授权此设备",
                "retryable": false
            }
        }"#;

        let description = describe_upstream_http_failure(
            reqwest::StatusCode::FORBIDDEN,
            "application/json",
            body,
        );

        assert_eq!(
            description,
            "HTTP 403 Forbidden: WORKSPACE_AUTHORIZATION_REQUIRED: 当前工作区尚未授权此设备"
        );
        assert!(!description.contains("遗留字段"));
    }

    #[test]
    fn reports_invalid_cmodel_candidate_as_protocol_error() {
        let body = r#"{
            "contractVersion": "1.0.0",
            "errorCode": "WORKSPACE_AUTHORIZATION_REQUIRED",
            "data": {"message": "缺少 retryable"}
        }"#;

        let description = describe_upstream_http_failure(
            reqwest::StatusCode::FORBIDDEN,
            "application/json",
            body,
        );

        assert!(description.contains("CModel 协议错误"));
        assert!(description.contains("retryable"));
    }

    #[test]
    fn keeps_non_cmodel_json_message_fallback() {
        let description = describe_upstream_http_failure(
            reqwest::StatusCode::BAD_GATEWAY,
            "application/json",
            r#"{"message":"upstream unavailable"}"#,
        );

        assert_eq!(description, "HTTP 502 Bad Gateway: upstream unavailable");
    }
}
