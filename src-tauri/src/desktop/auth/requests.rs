use super::*;
use bridge_agent::config::environment::api_base;
use serde::de::DeserializeOwned;

pub(super) async fn start(config: &AgentConfig) -> Result<BrowserAuthStartResponse, String> {
    let manifest = browser_auth_manifest_json(config).map_err(|err| err.to_string())?;
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
    request(config, "start", &payload, "启动浏览器授权失败").await
}

pub(super) async fn poll(
    config: &AgentConfig,
    device_code: &str,
) -> Result<RawBrowserAuthPollResponse, String> {
    request(
        config,
        "poll",
        &serde_json::json!({"deviceCode": device_code}),
        "轮询浏览器授权失败",
    )
    .await
}

fn endpoint(base_url: &str, operation: &str) -> Result<reqwest::Url, String> {
    let base = api_base(base_url);
    let mut url = reqwest::Url::parse(&base).map_err(|err| format!("平台地址无效: {err}"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("平台地址必须是无凭据、查询参数和片段的 HTTP(S) 地址".into());
    }
    url.path_segments_mut()
        .map_err(|_| "平台地址不能作为 API 路径基址".to_string())?
        .pop_if_empty()
        .extend([
            "device-service",
            "api",
            "external-workspace-device-auth",
            operation,
        ]);
    Ok(url)
}

async fn request<T: DeserializeOwned>(
    config: &AgentConfig,
    operation: &str,
    body: &impl Serialize,
    label: &str,
) -> Result<T, String> {
    let url = endpoint(&config.platform.base_url, operation)?;
    let response = Client::new()
        .post(url.clone())
        .json(body)
        .send()
        .await
        .map_err(|err| format!("{label} ({url}): {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.text().await.unwrap_or_default();
        let message = describe_upstream_http_failure(status, &content_type, &body);
        let error = format!("{label} ({url}): {message}");
        log::warn!("{error}");
        return Err(error);
    }
    response
        .json()
        .await
        .map_err(|err| format!("{label} ({url}): {err}"))
}

#[cfg(test)]
mod tests;
