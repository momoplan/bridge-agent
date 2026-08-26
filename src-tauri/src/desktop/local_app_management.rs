use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConnectorManagementCommandError {
    pub(super) code: &'static str,
    pub(super) message: String,
    pub(super) lifecycle: Option<Box<ConnectorLifecycleSnapshot>>,
}

impl ConnectorManagementCommandError {
    pub(super) fn message(message: impl Into<String>) -> Self {
        Self {
            code: "connector_management_failed",
            message: message.into(),
            lifecycle: None,
        }
    }
}

impl From<Box<ConnectorManagementNotReady>> for ConnectorManagementCommandError {
    fn from(error: Box<ConnectorManagementNotReady>) -> Self {
        Self {
            code: error.code,
            message: error.message,
            lifecycle: Some(Box::new(error.lifecycle)),
        }
    }
}

#[tauri::command]
pub(super) async fn invoke_connector_management(
    state: tauri::State<'_, DesktopState>,
    id: String,
    operation: String,
    payload: Option<Value>,
) -> Result<Value, ConnectorManagementCommandError> {
    invoke_connector_management_with_context(
        &state.connector_lifecycles,
        &state.connector_processes,
        id,
        operation,
        payload,
    )
    .await
}

pub(super) async fn invoke_connector_management_with_context(
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    id: String,
    operation: String,
    payload: Option<Value>,
) -> Result<Value, ConnectorManagementCommandError> {
    let management_permit = connector_lifecycles
        .try_management_permit(id.trim())
        .map_err(ConnectorManagementCommandError::from)?;
    if connector_processes.managed_running(id.trim()).await == Some(false) {
        connector_lifecycles
            .observe(
                id.trim(),
                ConnectorLifecycleState::Stopped,
                ConnectorHealthState::Unhealthy,
                management_permit.lifecycle.observed_version.clone(),
                None,
                Some("宿主管理进程已退出".to_string()),
            )
            .map_err(ConnectorManagementCommandError::message)?;
        return Err(ConnectorManagementCommandError::from(
            connector_lifecycles
                .try_management_permit(id.trim())
                .err()
                .expect("stopped connector must reject management requests"),
        ));
    }
    let result = invoke_connector_management_request(id, operation, payload)
        .await
        .map_err(ConnectorManagementCommandError::message);
    drop(management_permit);
    result
}

pub(super) async fn invoke_connector_management_request(
    id: String,
    operation: String,
    payload: Option<Value>,
) -> Result<Value, String> {
    let request = resolve_connector_management_request(&id, &operation, payload.as_ref())?;
    send_connector_management_request(request, payload).await
}

struct ResolvedManagementRequest {
    url: String,
    method: String,
    token: String,
}

fn resolve_connector_management_request(
    id: &str,
    operation: &str,
    payload: Option<&Value>,
) -> Result<ResolvedManagementRequest, String> {
    let id = id.trim();
    let operation = operation.trim();
    let record = show_connector(id).map_err(|err| err.to_string())?;
    let management = record
        .manifest
        .management
        .as_ref()
        .ok_or_else(|| format!("应用 {} 没有声明本机管理接口", record.manifest.name))?;
    let operation_config = management
        .operations
        .get(operation)
        .ok_or_else(|| format!("应用 {} 没有声明管理操作 {operation}", record.manifest.name))?;
    if let Some(payload) = payload {
        let payload_size = serde_json::to_vec(payload)
            .map_err(|err| format!("序列化应用管理参数失败: {err}"))?
            .len();
        if payload_size > LOCAL_APP_UI_MAX_MANAGEMENT_PAYLOAD_BYTES {
            return Err(format!(
                "应用管理参数超过 {} 字节限制",
                LOCAL_APP_UI_MAX_MANAGEMENT_PAYLOAD_BYTES
            ));
        }
    }
    let management_url = reqwest::Url::parse(&management.base_url)
        .map_err(|err| format!("应用本机管理地址无效: {err}"))?;
    let management_host = management_url.host_str().unwrap_or_default();
    let management_host_is_loopback = management_host.eq_ignore_ascii_case("localhost")
        || management_host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if management_url.scheme() != "http" || !management_host_is_loopback {
        return Err("应用本机管理地址必须是 loopback HTTP".to_string());
    }
    if !management_url.username().is_empty()
        || management_url.password().is_some()
        || management_url.query().is_some()
        || management_url.fragment().is_some()
        || management_url.path() != "/"
    {
        return Err("应用本机管理地址必须是只包含 origin 的 URL".to_string());
    }
    if !matches!(operation_config.method.as_str(), "GET" | "POST")
        || !operation_config.path.starts_with("/management/")
        || operation_config.path.contains('?')
        || operation_config.path.contains('#')
    {
        return Err(format!("应用管理操作 {operation} 的声明不安全"));
    }
    let token_path = connector_management_token_path(id).map_err(|err| err.to_string())?;
    let token = fs::read_to_string(&token_path)
        .map_err(|err| format!("读取应用本机管理凭证失败 {}: {err}", token_path.display()))?;
    let token = token.trim();
    if token.len() < 32 {
        return Err(format!("应用本机管理凭证无效: {}", token_path.display()));
    }
    #[cfg(unix)]
    {
        let metadata = fs::metadata(&token_path).map_err(|err| err.to_string())?;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "应用本机管理凭证权限不安全: {}",
                token_path.display()
            ));
        }
    }

    Ok(ResolvedManagementRequest {
        url: format!(
            "{}{}",
            management.base_url.trim_end_matches('/'),
            operation_config.path
        ),
        method: operation_config.method.clone(),
        token: token.to_string(),
    })
}

async fn send_connector_management_request(
    resolved: ResolvedManagementRequest,
    payload: Option<Value>,
) -> Result<Value, String> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|err| format!("创建本机应用管理请求失败: {err}"))?;
    let request = match resolved.method.as_str() {
        "GET" => client.get(&resolved.url),
        "POST" => client
            .post(&resolved.url)
            .json(&payload.unwrap_or_else(|| serde_json::json!({}))),
        method => return Err(format!("不支持的本机应用管理方法: {method}")),
    };
    let response = request
        .bearer_auth(resolved.token)
        .send()
        .await
        .map_err(|err| format!("调用本机应用管理接口失败: {err}"))?;
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|size| size > LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES as u64)
    {
        return Err(format!(
            "本机应用管理响应超过 {} 字节限制",
            LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES
        ));
    }
    let body = response
        .bytes()
        .await
        .map_err(|err| format!("读取本机应用管理响应失败: {err}"))?;
    if body.len() > LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES {
        return Err(format!(
            "本机应用管理响应超过 {} 字节限制",
            LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES
        ));
    }
    let document: Value = serde_json::from_slice(&body)
        .map_err(|err| format!("本机应用管理接口返回了无效 JSON: {err}"))?;
    if !status.is_success() || document.get("ok").and_then(Value::as_bool) != Some(true) {
        let message = document
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("本机应用管理操作失败");
        return Err(format!("{message}（HTTP {status}）"));
    }
    Ok(document.get("data").cloned().unwrap_or(Value::Null))
}
