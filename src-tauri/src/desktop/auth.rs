use super::*;

#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BrowserAuthStartResponse {
    pub(super) device_code: String,
    pub(super) user_code: String,
    pub(super) verification_uri: String,
    pub(super) verification_uri_complete: String,
    pub(super) expires_in: i32,
    pub(super) interval: i32,
}

#[derive(Debug, Serialize)]
pub(super) struct BrowserAuthPollResponse {
    pub(super) status: String,
    pub(super) message: String,
    pub(super) config: Option<Value>,
    pub(super) runtime: Option<RuntimeSnapshot>,
}

pub(super) fn config_for_ui(config: &AgentConfig) -> Result<Value, String> {
    let mut value = serde_json::to_value(config).map_err(|err| err.to_string())?;
    value["relay"]["token"] = Value::String(String::new());
    value["credential_status"] = serde_json::json!({
        "relay_token_configured": !config.relay.token.trim().is_empty()
    });
    Ok(value)
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct RawBrowserAuthPollResponse {
    pub(super) status: String,
    pub(super) message: String,
    #[serde(rename = "authorizedPayload")]
    pub(super) authorized_payload: Option<AuthorizedPayload>,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct AuthorizedPayload {
    #[serde(rename = "workspaceId")]
    pub(super) workspace_id: u64,
    #[serde(rename = "deviceId")]
    pub(super) device_id: String,
    #[serde(rename = "relayWsUrl")]
    pub(super) relay_ws_url: String,
    #[serde(rename = "agentToken")]
    pub(super) agent_token: String,
    #[serde(rename = "issuedAtEpochSeconds")]
    pub(super) issued_at_epoch_seconds: Option<u64>,
    #[serde(rename = "expiresAtEpochSeconds")]
    pub(super) expires_at_epoch_seconds: Option<u64>,
    #[serde(rename = "localClientToken")]
    pub(super) local_client_token: Option<String>,
    #[serde(rename = "localClientTokenType")]
    pub(super) local_client_token_type: Option<String>,
    #[serde(rename = "localClientKeyId")]
    pub(super) local_client_key_id: Option<String>,
    #[serde(rename = "localClientUserId")]
    pub(super) local_client_user_id: Option<u64>,
    #[serde(default, rename = "localClientScopes")]
    pub(super) local_client_scopes: Vec<String>,
    #[serde(rename = "localClientIssuedAt")]
    pub(super) local_client_issued_at: Option<String>,
    #[serde(rename = "localClientExpiresAt")]
    pub(super) local_client_expires_at: Option<String>,
}

#[tauri::command]
pub(super) async fn start_browser_auth(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
) -> Result<BrowserAuthStartResponse, String> {
    let mut config = config;
    let normalized = config.normalize();
    let agent_id_changed = ensure_browser_auth_agent_id(&mut config);
    if normalized || agent_id_changed {
        save_agent_config(&state.config_path, &config).map_err(|err| err.to_string())?;
    }
    let client = Client::new();
    let manifest = browser_auth_manifest_json(&config).map_err(|err| err.to_string())?;
    let base_url = config.platform.base_url.trim_end_matches('/');
    let mut payload = serde_json::Map::new();
    if let Some(workspace_id) = config.platform.workspace_id {
        payload.insert("workspaceId".to_string(), serde_json::json!(workspace_id));
    }
    payload.insert(
        "deviceId".to_string(),
        serde_json::json!(config.relay.agent_id),
    );
    payload.insert(
        "deviceName".to_string(),
        serde_json::json!(config.device.name),
    );
    payload.insert(
        "deviceDescription".to_string(),
        serde_json::json!(config.device.description),
    );
    payload.insert("serviceManifest".to_string(), serde_json::json!(manifest));
    let response = client
        .post(format!(
            "{base_url}/api/external-workspace-device-auth/start"
        ))
        .json(&payload)
        .send()
        .await
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let payload = response.text().await.unwrap_or_default();
        return Err(format!(
            "启动浏览器授权失败: {}",
            describe_upstream_http_failure(status, &content_type, &payload)
        ));
    }

    let payload: BrowserAuthStartResponse = response.json().await.map_err(|err| err.to_string())?;
    open::that(payload.verification_uri_complete.clone()).map_err(|err| err.to_string())?;
    Ok(payload)
}

#[tauri::command]
pub(super) async fn poll_browser_auth(
    state: tauri::State<'_, DesktopState>,
    config: AgentConfig,
    device_code: String,
) -> Result<BrowserAuthPollResponse, CommandError> {
    let client = Client::new();
    let base_url = config.platform.base_url.trim_end_matches('/');
    let response = client
        .post(format!(
            "{base_url}/api/external-workspace-device-auth/poll"
        ))
        .json(&serde_json::json!({
            "deviceCode": device_code
        }))
        .send()
        .await
        .map_err(|err| command_error_message(err.to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let payload = response.text().await.unwrap_or_default();
        return Err(command_error_message(format!(
            "轮询浏览器授权失败: {}",
            describe_upstream_http_failure(status, &content_type, &payload)
        )));
    }

    let payload: RawBrowserAuthPollResponse = response
        .json()
        .await
        .map_err(|err| command_error_message(err.to_string()))?;
    if payload.status != "authorized" {
        return Ok(BrowserAuthPollResponse {
            status: payload.status,
            message: payload.message,
            config: None,
            runtime: None,
        });
    }

    let authorized = payload
        .authorized_payload
        .ok_or_else(|| command_error_message("授权成功但缺少 authorizedPayload"))?;
    let mut updated = config;
    apply_authorized_device_credentials(&mut updated, &authorized);
    save_agent_config(&state.config_path, &updated)
        .map_err(|err| command_error_message(err.to_string()))?;
    log_shared_cli_auth_result(&state.runtime, write_shared_cli_auth(&updated, &authorized))
        .await?;
    let runtime = restart_agent_from_saved_config(&state)
        .await
        .map_err(CommandError::from)?;

    Ok(BrowserAuthPollResponse {
        status: payload.status,
        message: payload.message,
        config: Some(config_for_ui(&updated).map_err(command_error_message)?),
        runtime: Some(runtime),
    })
}

async fn log_shared_cli_auth_result(
    runtime: &AgentRuntimeManager,
    result: anyhow::Result<Option<SharedCliAuthWriteResult>>,
) -> Result<(), CommandError> {
    match result {
        Ok(Some(result)) => {
            runtime
                .push_desktop_log(
                    "info",
                    &format!("shared CLI auth written to {}", result.path.display()),
                    LogMetadata::category("desktop_auth")
                        .event("shared_cli_auth")
                        .outcome("written"),
                )
                .await;
        }
        Ok(None) => {
            runtime
                .push_desktop_log(
                    "warn",
                    "authorized payload did not include a local client token; skipped shared CLI auth",
                    LogMetadata::category("desktop_auth")
                        .event("shared_cli_auth")
                        .outcome("skipped"),
                )
                .await;
        }
        Err(err) => {
            runtime
                .push_desktop_log(
                    "error",
                    &format!("failed to write shared CLI auth: {err:#}"),
                    LogMetadata::category("desktop_auth")
                        .event("shared_cli_auth")
                        .outcome("failed"),
                )
                .await;
            return Err(command_error_message(err.to_string()));
        }
    }
    Ok(())
}

pub(super) fn apply_authorized_device_credentials(
    config: &mut AgentConfig,
    authorized: &AuthorizedPayload,
) {
    config.platform.workspace_id = Some(authorized.workspace_id);
    config.relay.agent_id = authorized.device_id.clone();
    config.relay.url = authorized.relay_ws_url.clone();
    config.relay.token = authorized.agent_token.clone();
    config.relay.token_issued_at_epoch_seconds = authorized
        .issued_at_epoch_seconds
        .map(|value| value.to_string());
    config.relay.token_expires_at_epoch_seconds = authorized
        .expires_at_epoch_seconds
        .map(|value| value.to_string());
}

pub(super) struct SharedCliAuthWriteResult {
    pub(super) path: PathBuf,
}

pub(super) fn write_shared_cli_auth(
    config: &AgentConfig,
    authorized: &AuthorizedPayload,
) -> anyhow::Result<Option<SharedCliAuthWriteResult>> {
    let Some(local_client_token) = authorized.local_client_token.as_deref() else {
        return Ok(None);
    };
    if local_client_token.trim().is_empty() {
        return Ok(None);
    }

    let path = shared_cli_auth_path();
    write_shared_cli_auth_at(&path, config, authorized)?;
    Ok(Some(SharedCliAuthWriteResult { path }))
}

pub(super) fn write_shared_cli_auth_at(
    path: &Path,
    config: &AgentConfig,
    authorized: &AuthorizedPayload,
) -> anyhow::Result<()> {
    let local_client_token = validate_shared_cli_auth_payload(authorized)?;
    let mut document = load_shared_cli_auth_document(path)?;
    configure_shared_cli_environment(&mut document, config, authorized);
    let credentials = merged_shared_cli_credentials(&document, authorized, local_client_token);
    document["schemaVersion"] = serde_json::json!(2);
    document["credentials"] = Value::Array(credentials);
    if let Some(object) = document.as_object_mut() {
        object.remove("machineCredentials");
    }
    persist_shared_cli_auth(path, &document)
}

fn validate_shared_cli_auth_payload(authorized: &AuthorizedPayload) -> anyhow::Result<&str> {
    let token = authorized
        .local_client_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("authorized payload is missing local client token"))?;
    if !token.starts_with("lc_pat_") {
        anyhow::bail!("authorized payload local client token is not a Baijimu PAT");
    }
    if authorized
        .local_client_token_type
        .as_deref()
        .is_some_and(|token_type| !matches!(token_type, "pat" | "workspace_user_api_key"))
    {
        anyhow::bail!("authorized payload local client token type is not a PAT");
    }
    Ok(token)
}

fn load_shared_cli_auth_document(path: &Path) -> anyhow::Result<Value> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut document = if path.exists() {
        let content = fs::read_to_string(path)?;
        serde_json::from_str::<Value>(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    if !document.is_object() {
        document = serde_json::json!({});
    }
    Ok(document)
}

fn configure_shared_cli_environment(
    document: &mut Value,
    config: &AgentConfig,
    authorized: &AuthorizedPayload,
) {
    document["currentEnvironment"] = serde_json::json!("prod");
    document["currentWorkspaceId"] = serde_json::json!(authorized.workspace_id);
    if !document
        .get("environments")
        .map(|value| value.is_object())
        .unwrap_or(false)
    {
        document["environments"] = serde_json::json!({});
    }
    document["environments"]["prod"] = serde_json::json!({
        "baseUrl": config.platform.base_url.trim_end_matches('/'),
    });
}

fn merged_shared_cli_credentials(
    document: &Value,
    authorized: &AuthorizedPayload,
    local_client_token: &str,
) -> Vec<Value> {
    let mut credentials = document
        .get("credentials")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    if let Some(legacy_credentials) = document
        .get("machineCredentials")
        .and_then(|value| value.as_array())
    {
        credentials.extend(legacy_credentials.iter().cloned());
    }
    credentials = credentials
        .into_iter()
        .filter_map(normalize_shared_pat_credential)
        .collect();
    let mut seen_credentials = HashSet::new();
    credentials.retain(|credential| {
        let identity = credential
            .get("credentialId")
            .and_then(Value::as_str)
            .map(|value| format!("id:{value}"))
            .or_else(|| {
                credential
                    .get("token")
                    .and_then(Value::as_str)
                    .map(|value| format!("token:{value}"))
            });
        identity.is_none_or(|value| seen_credentials.insert(value))
    });
    credentials.retain(|item| {
        !credential_has_workspace(item, authorized.workspace_id)
            || item.get("clientId").and_then(|value| value.as_str())
                != Some(authorized.device_id.as_str())
    });
    credentials.push(serde_json::json!({
        "credentialId": authorized.local_client_key_id,
        "userId": authorized.local_client_user_id,
        "workspaceIds": [authorized.workspace_id],
        "clientId": authorized.device_id,
        "token": local_client_token,
        "tokenType": "pat",
        "subjectType": "user",
        "source": "bridge-agent",
        "scopes": if authorized.local_client_scopes.is_empty() {
            vec![
                "baijimu:agent-cli".to_string(),
                "partner:api".to_string(),
                format!("workspace:{}", authorized.workspace_id),
            ]
        } else {
            authorized.local_client_scopes.clone()
        },
        "issuedAt": authorized.local_client_issued_at,
        "issuedAtEpochSeconds": now_epoch_seconds(),
        "expiresAt": authorized.local_client_expires_at,
    }));
    credentials
}

fn persist_shared_cli_auth(path: &Path, document: &Value) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid shared CLI auth path: {}", path.display()))?;
    let mut temp_file = tempfile::NamedTempFile::new_in(parent)?;
    temp_file.write_all(&serde_json::to_vec_pretty(&document)?)?;
    #[cfg(unix)]
    temp_file
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    temp_file.as_file().sync_all()?;
    temp_file
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to atomically replace {}", path.display()))?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub(super) fn normalize_shared_pat_credential(mut credential: Value) -> Option<Value> {
    let object = credential.as_object_mut()?;
    if !object
        .get("token")
        .and_then(Value::as_str)
        .is_some_and(|token| token.starts_with("lc_pat_"))
    {
        return None;
    }
    let workspace_ids = object
        .get("workspaceIds")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| {
            object
                .get("workspaceId")
                .and_then(Value::as_u64)
                .map(|workspace_id| vec![Value::from(workspace_id)])
        })
        .unwrap_or_default();
    if object.get("credentialId").is_none() {
        if let Some(key_id) = object.get("keyId").cloned() {
            object.insert("credentialId".to_string(), key_id);
        }
    }
    object.insert("workspaceIds".to_string(), Value::Array(workspace_ids));
    object.insert("tokenType".to_string(), Value::String("pat".to_string()));
    object.insert("subjectType".to_string(), Value::String("user".to_string()));
    if object.get("source").is_none() {
        object.insert(
            "source".to_string(),
            Value::String("bridge-agent".to_string()),
        );
    }
    object.remove("workspaceId");
    object.remove("keyId");
    Some(credential)
}

pub(super) fn credential_has_workspace(credential: &Value, workspace_id: u64) -> bool {
    credential
        .get("workspaceIds")
        .and_then(Value::as_array)
        .is_some_and(|workspace_ids| {
            workspace_ids
                .iter()
                .any(|value| value.as_u64() == Some(workspace_id))
        })
}

pub(super) fn shared_cli_auth_path() -> PathBuf {
    if let Some(config_home) = std::env::var_os("BAIJIMU_CONFIG_HOME") {
        return PathBuf::from(config_home).join("baijimu").join("auth.json");
    }
    let home = shared_cli_home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("baijimu").join("auth.json")
}

pub(super) fn shared_cli_home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(user_profile) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(user_profile));
        }
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
}

pub(super) fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}
