use bridge_agent::services::local_app_runtime_service;
use bridge_agent::{LocalAppConfig, ServiceConfig, ServiceHealthCheck};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

const HEALTH_ERROR_RESPONSE_MAX_BYTES: usize = 8 * 1024;
pub(crate) const HEALTH_ERROR_MESSAGE_MAX_CHARS: usize = 512;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RegisteredServiceState {
    NotConfigured,
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisteredServiceStatus {
    pub(crate) service: String,
    pub(crate) status: RegisteredServiceState,
    pub(crate) detail: Option<String>,
    pub(crate) checked_at_ms: u64,
    pub(crate) health_check_configured: bool,
    pub(crate) start_command_configured: bool,
    pub(crate) stop_command_configured: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalAppRuntimeStatus {
    pub(crate) app_id: String,
    pub(crate) status: RegisteredServiceState,
    pub(crate) detail: Option<String>,
    pub(crate) checked_at_ms: u64,
    pub(crate) health_check_configured: bool,
    pub(crate) start_command_configured: bool,
    pub(crate) stop_command_configured: bool,
    pub(crate) process_managed: bool,
    pub(crate) process_running: Option<bool>,
}

pub(crate) async fn check_registered_service(
    client: &Client,
    service: ServiceConfig,
) -> RegisteredServiceStatus {
    let health_check_configured = service.health_check.is_some();
    let start_command_configured = service.start_command.is_some();
    let stop_command_configured = service.stop_command.is_some();
    let Some(health_check) = service.health_check else {
        return RegisteredServiceStatus {
            service: service.name,
            status: RegisteredServiceState::NotConfigured,
            detail: Some("没有注册 healthCheck".to_string()),
            checked_at_ms: now_ms(),
            health_check_configured,
            start_command_configured,
            stop_command_configured,
        };
    };

    match health_check {
        ServiceHealthCheck::Http {
            url,
            http_method,
            headers,
            timeout_secs,
            expect_status,
            body_contains,
        } => {
            let method = http_method
                .parse::<reqwest::Method>()
                .unwrap_or(reqwest::Method::GET);
            let mut request = client
                .request(method, &url)
                .timeout(Duration::from_secs(timeout_secs.unwrap_or(3).max(1)));
            for (key, value) in headers {
                request = request.header(key, value);
            }
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let expected_status = expect_status.unwrap_or(200);
                    if status.as_u16() != expected_status {
                        let detail = health_http_error_detail(response, expected_status).await;
                        return RegisteredServiceStatus {
                            service: service.name,
                            status: RegisteredServiceState::Unhealthy,
                            detail: Some(detail),
                            checked_at_ms: now_ms(),
                            health_check_configured,
                            start_command_configured,
                            stop_command_configured,
                        };
                    }
                    if let Some(expected_text) = body_contains
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        match response.text().await {
                            Ok(body) if body.contains(expected_text) => {}
                            Ok(_) => {
                                return RegisteredServiceStatus {
                                    service: service.name,
                                    status: RegisteredServiceState::Unhealthy,
                                    detail: Some("health 响应内容不符合 bodyContains".to_string()),
                                    checked_at_ms: now_ms(),
                                    health_check_configured,
                                    start_command_configured,
                                    stop_command_configured,
                                };
                            }
                            Err(err) => {
                                return RegisteredServiceStatus {
                                    service: service.name,
                                    status: RegisteredServiceState::Unknown,
                                    detail: Some(format!("读取 health 响应失败: {err}")),
                                    checked_at_ms: now_ms(),
                                    health_check_configured,
                                    start_command_configured,
                                    stop_command_configured,
                                };
                            }
                        }
                    }
                    RegisteredServiceStatus {
                        service: service.name,
                        status: RegisteredServiceState::Healthy,
                        detail: Some(format!("health HTTP {}", status.as_u16())),
                        checked_at_ms: now_ms(),
                        health_check_configured,
                        start_command_configured,
                        stop_command_configured,
                    }
                }
                Err(err) => RegisteredServiceStatus {
                    service: service.name,
                    status: RegisteredServiceState::Unhealthy,
                    detail: Some(format!("health 检查失败: {err}")),
                    checked_at_ms: now_ms(),
                    health_check_configured,
                    start_command_configured,
                    stop_command_configured,
                },
            }
        }
    }
}

async fn health_http_error_detail(mut response: reqwest::Response, expected_status: u16) -> String {
    let status = response.status().as_u16();
    let mut body = Vec::new();
    while body.len() < HEALTH_ERROR_RESPONSE_MAX_BYTES {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) | Err(_) => break,
        };
        let remaining = HEALTH_ERROR_RESPONSE_MAX_BYTES - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining {
            break;
        }
    }
    format_health_http_error(status, expected_status, &body)
}

pub(crate) fn format_health_http_error(status: u16, expected_status: u16, body: &[u8]) -> String {
    let prefix = format!("health HTTP {status}，期望 {expected_status}");
    let Some(message) = structured_health_error_message(body) else {
        return prefix;
    };
    format!("{prefix}：{message}")
}

fn structured_health_error_message(body: &[u8]) -> Option<String> {
    let payload = serde_json::from_slice::<Value>(body).ok()?;
    let message = [
        "/error/message",
        "/status/startup/error",
        "/status/startup/message",
        "/message",
    ]
    .into_iter()
    .find_map(|pointer| payload.pointer(pointer).and_then(Value::as_str))?;
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    Some(
        normalized
            .chars()
            .take(HEALTH_ERROR_MESSAGE_MAX_CHARS)
            .collect(),
    )
}

pub(crate) async fn check_local_app(
    client: &Client,
    app: LocalAppConfig,
    process_running: Option<bool>,
) -> LocalAppRuntimeStatus {
    let app_id = app.app_id.clone();
    let health_check_configured = app.health_check.is_some();
    let start_command_configured = app.start_command.is_some();
    let stop_command_configured = app.stop_command.is_some();
    let service = match local_app_runtime_service(&app) {
        Ok(service) => service,
        Err(err) => {
            return LocalAppRuntimeStatus {
                app_id,
                status: RegisteredServiceState::Unknown,
                detail: Some(format!("加载应用本机凭证失败: {err:#}")),
                checked_at_ms: now_ms(),
                health_check_configured,
                start_command_configured,
                stop_command_configured,
                process_managed: process_running.is_some(),
                process_running,
            };
        }
    };
    let mut status = check_registered_service(
        client,
        ServiceConfig {
            methods: Vec::new(),
            ..service
        },
    )
    .await;
    apply_managed_process_status(&mut status, process_running);
    LocalAppRuntimeStatus {
        app_id,
        status: status.status,
        detail: status.detail,
        checked_at_ms: status.checked_at_ms,
        health_check_configured: status.health_check_configured,
        start_command_configured: status.start_command_configured,
        stop_command_configured: status.stop_command_configured,
        process_managed: process_running.is_some(),
        process_running,
    }
}

pub(crate) fn inactive_local_app_status(
    app: LocalAppConfig,
    process_running: Option<bool>,
) -> LocalAppRuntimeStatus {
    LocalAppRuntimeStatus {
        app_id: app.app_id,
        status: RegisteredServiceState::Unhealthy,
        detail: Some("应用尚未由 Bridge Agent 启动".to_string()),
        checked_at_ms: now_ms(),
        health_check_configured: app.health_check.is_some(),
        start_command_configured: app.start_command.is_some(),
        stop_command_configured: app.stop_command.is_some(),
        process_managed: process_running.is_some(),
        process_running,
    }
}

pub(crate) fn apply_managed_process_status(
    status: &mut RegisteredServiceStatus,
    process_running: Option<bool>,
) {
    if status.health_check_configured {
        return;
    }
    match process_running {
        Some(true) => {
            status.status = RegisteredServiceState::Healthy;
            status.detail = Some("宿主管理进程正在运行".to_string());
        }
        Some(false) => {
            status.status = RegisteredServiceState::Unhealthy;
            status.detail = Some("宿主管理进程未运行".to_string());
        }
        None => {}
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
