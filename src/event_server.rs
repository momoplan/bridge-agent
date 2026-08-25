use crate::config::{
    load_config, resolve_config_base_dir, save_config, AgentConfig, ServiceConfig,
    ServiceRegistration,
};
use crate::connector::{
    authorize_connector_asset_upload, authorize_connector_event, connector_declares_asset_upload,
};
use crate::logging::LogMetadata;
use crate::process_identity::is_bridge_agent_process_name;
use crate::protocol::LocalAppEventEmitted;
use crate::runtime::{LocalAppEventSubmission, RuntimeAuditLog, RuntimeRegistryUpdate};
use crate::services::ServiceRegistry;
#[cfg(windows)]
use crate::windows_process::{inspect_windows_process, terminate_windows_process};
#[cfg(windows)]
use crate::windows_tcp::find_windows_tcp_listener_pid;
#[cfg(test)]
use crate::windows_tcp::windows_listener_matches;
use anyhow::{Context, Result};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch, RwLock};
use tokio::time::{sleep, timeout};
use uuid::Uuid;

const PORT_RECLAIM_BIND_RETRIES: usize = 20;
const PORT_RECLAIM_RETRY_DELAY: Duration = Duration::from_millis(150);
const MAX_CONNECTOR_ASSET_BYTES: u64 = 5 * 1024 * 1024;
const LOCAL_APP_EVENT_FORWARD_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct LocalEventServer {
    bind: SocketAddr,
    listener: TcpListener,
    state: EventServerState,
}

#[derive(Clone)]
struct EventServerState {
    registry: Arc<RwLock<ServiceRegistry>>,
    event_tx: mpsc::Sender<LocalAppEventSubmission>,
    apply_tx: mpsc::UnboundedSender<RuntimeRegistryUpdate>,
    audit_tx: mpsc::UnboundedSender<RuntimeAuditLog>,
    config_path: PathBuf,
    event_enabled: bool,
    service_registration_enabled: bool,
    service_registration_token: Option<String>,
    upload_prepare_url: Option<String>,
    upload_timeout_secs: u64,
    relay_token: String,
    agent_id: String,
    workspace_id: Option<u64>,
    http_client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadLocalAppAssetRequest {
    connector_id: String,
    local_path: String,
    purpose: String,
    content_type: String,
    #[serde(default)]
    file_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadLocalAppAssetResponse {
    result_type: &'static str,
    asset_id: String,
    object_key: Option<String>,
    download_url: Option<String>,
    expires_at: Option<String>,
    mime_type: String,
    size_bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrepareAssetUploadRequest {
    agent_id: String,
    workspace_id: Option<u64>,
    purpose: String,
    content_type: String,
    file_name: String,
    size_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareAssetUploadResponse {
    file_id: String,
    upload_url: String,
    method: Option<String>,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, String>,
    object_key: Option<String>,
    download_url: Option<String>,
    expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmitLocalAppEventRequest {
    connector_id: String,
    event: String,
    #[serde(default)]
    payload: Value,
    #[serde(default)]
    event_id: Option<String>,
    #[serde(default)]
    occurred_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EmitLocalAppEventResponse {
    accepted: bool,
    persisted: bool,
    matched_subscription_count: usize,
    duplicate: bool,
    event_id: String,
    connector_id: String,
    event: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RegisterServiceRequest {
    Public(ServiceRegistration),
    Raw {
        service: ServiceConfig,
        #[serde(default)]
        replace: bool,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceManagementResponse {
    service: ServiceConfig,
    replaced: bool,
    runtime_applied: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteServiceResponse {
    service: String,
    deleted: bool,
    runtime_applied: bool,
}

struct EventApiError {
    status: StatusCode,
    message: String,
}

impl EventApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for EventApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": self.message
            })),
        )
            .into_response()
    }
}

impl LocalEventServer {
    pub(crate) async fn bind(
        config: &AgentConfig,
        config_path: PathBuf,
        registry: Arc<RwLock<ServiceRegistry>>,
        event_tx: mpsc::Sender<LocalAppEventSubmission>,
        apply_tx: mpsc::UnboundedSender<RuntimeRegistryUpdate>,
        audit_tx: mpsc::UnboundedSender<RuntimeAuditLog>,
    ) -> Result<Option<Self>> {
        if !config.runtime.event_server_enabled && !config.runtime.service_registration_enabled {
            let asset_upload_enabled = config
                .local_apps
                .iter()
                .filter(|app| app.enabled)
                .filter_map(|app| connector_declares_asset_upload(&app.connector_id).ok())
                .any(|declared| declared);
            if !asset_upload_enabled {
                return Ok(None);
            }
        }

        let bind: SocketAddr = config
            .runtime
            .event_server_bind
            .parse()
            .with_context(|| "runtime.event_server_bind must be a socket address")?;
        let listener = bind_event_listener(bind)
            .await
            .with_context(|| format!("failed to bind local event server on {bind}"))?;
        let bind = listener.local_addr()?;
        Ok(Some(Self {
            bind,
            listener,
            state: EventServerState {
                registry,
                event_tx,
                apply_tx,
                audit_tx,
                config_path,
                event_enabled: config.runtime.event_server_enabled,
                service_registration_enabled: config.runtime.service_registration_enabled,
                service_registration_token: config
                    .runtime
                    .service_registration_token
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
                upload_prepare_url: config.upload.prepare_url(&config.relay),
                upload_timeout_secs: config.upload.timeout_secs,
                relay_token: config.relay.token.clone(),
                agent_id: config.relay.agent_id.clone(),
                workspace_id: config.platform.workspace_id,
                http_client: reqwest::Client::new(),
            },
        }))
    }

    pub(crate) fn bind_addr(&self) -> SocketAddr {
        self.bind
    }

    pub(crate) async fn serve(self, mut shutdown_rx: watch::Receiver<bool>) -> Result<()> {
        let state = self.state;
        let app = Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .route("/v1/local-app-events", post(emit_local_app_event))
            .route("/v1/local-app-assets", post(upload_local_app_asset))
            .route("/v1/services", get(list_services).post(register_service))
            .route(
                "/v1/services/{service}",
                put(replace_service).delete(delete_service),
            )
            .with_state(state.clone())
            .layer(middleware::from_fn_with_state(state, audit_http_request));

        axum::serve(self.listener, app)
            .with_graceful_shutdown(async move {
                while !*shutdown_rx.borrow() {
                    if shutdown_rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await
            .context("local event server stopped unexpectedly")
    }
}

async fn audit_http_request(
    State(state): State<EventServerState>,
    request: Request,
    next: Next,
) -> Response {
    let started = Instant::now();
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();
    let response = next.run(request).await;
    let status = response.status();
    let status_code = status.as_u16();
    let level = if status.is_server_error() {
        "error"
    } else if status.is_client_error() {
        "warn"
    } else {
        "info"
    };
    let outcome = if status.is_success() {
        "succeeded"
    } else {
        "failed"
    };

    emit_audit_log(
        &state,
        level,
        format!("local api {method} {path} -> {status_code}"),
        LogMetadata::category("local_api")
            .http(method, path, status_code)
            .duration_ms(started.elapsed().as_millis() as u64)
            .outcome(outcome),
    );

    response
}

async fn bind_event_listener(bind: SocketAddr) -> Result<TcpListener> {
    let first_err = match TcpListener::bind(bind).await {
        Ok(listener) => return Ok(listener),
        Err(err) => err,
    };

    if first_err.kind() != ErrorKind::AddrInUse {
        return Err(first_err.into());
    }

    let Some(reclaimed) = reclaim_occupied_event_port(bind).await? else {
        return Err(first_err.into());
    };

    for _ in 0..PORT_RECLAIM_BIND_RETRIES {
        match TcpListener::bind(bind).await {
            Ok(listener) => return Ok(listener),
            Err(err) if err.kind() == ErrorKind::AddrInUse => {
                sleep(PORT_RECLAIM_RETRY_DELAY).await;
            }
            Err(err) => return Err(err.into()),
        }
    }

    TcpListener::bind(bind).await.with_context(|| {
        format!(
            "local event server port is still occupied after stopping {} (pid {})",
            reclaimed.image_name, reclaimed.pid
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OccupiedPortOwner {
    pid: u32,
    image_name: String,
    parent_pid: Option<u32>,
    executable_path: Option<String>,
}

async fn reclaim_occupied_event_port(bind: SocketAddr) -> Result<Option<OccupiedPortOwner>> {
    if !bind.ip().is_loopback() || bind.port() == 0 {
        return Ok(None);
    }

    let Some(owner) = find_occupied_tcp_listener(bind)? else {
        return Ok(None);
    };

    if owner.pid == std::process::id() {
        return Ok(None);
    }

    if !is_bridge_agent_process_name(&owner.image_name) {
        let process_details = owner
            .executable_path
            .as_deref()
            .map(|path| format!(", path {path}"))
            .unwrap_or_default();
        anyhow::bail!(
            "local event server port {bind} is already occupied by {} (pid {}, parent pid {:?}{}), not a 百积木 process",
            owner.image_name,
            owner.pid,
            owner.parent_pid,
            process_details
        );
    }

    terminate_process(owner.pid, &owner.image_name)?;
    Ok(Some(owner))
}

#[cfg(windows)]
fn find_occupied_tcp_listener(bind: SocketAddr) -> Result<Option<OccupiedPortOwner>> {
    let Some(pid) = find_windows_tcp_listener_pid(bind)? else {
        return Ok(None);
    };
    // The listener can exit between the TCP table snapshot and the process
    // snapshot. Treat that as a normal race and let the runtime bind retry run.
    let Some(process) = inspect_windows_process(pid)? else {
        return Ok(None);
    };
    Ok(Some(OccupiedPortOwner {
        pid,
        image_name: process.image_name,
        parent_pid: process.parent_pid,
        executable_path: process.executable_path,
    }))
}

#[cfg(unix)]
fn find_occupied_tcp_listener(bind: SocketAddr) -> Result<Option<OccupiedPortOwner>> {
    let port_filter = format!("-iTCP:{}", bind.port());
    let lsof = std::process::Command::new("lsof")
        .args(["-nP", &port_filter, "-sTCP:LISTEN", "-F", "pcn"])
        .output()
        .context("failed to inspect TCP listeners with lsof")?;
    if !lsof.status.success() {
        if lsof.stdout.is_empty() && lsof.stderr.is_empty() {
            return Ok(None);
        }
        anyhow::bail!(
            "lsof failed while inspecting local event server port: {}",
            String::from_utf8_lossy(&lsof.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&lsof.stdout);
    Ok(parse_lsof_listening_owner(&stdout, bind))
}

#[cfg(not(any(windows, unix)))]
fn find_occupied_tcp_listener(_bind: SocketAddr) -> Result<Option<OccupiedPortOwner>> {
    Ok(None)
}

#[cfg(windows)]
fn terminate_process(pid: u32, image_name: &str) -> Result<()> {
    let Some(process) = inspect_windows_process(pid)? else {
        return Ok(());
    };
    if !process.image_name.eq_ignore_ascii_case(image_name) {
        anyhow::bail!(
            "pid {pid} changed owner from {image_name} to {}; refusing to terminate a reused pid",
            process.image_name
        );
    }
    terminate_windows_process(&process)
}

#[cfg(unix)]
fn terminate_process(pid: u32, image_name: &str) -> Result<()> {
    let pid_arg = pid.to_string();
    let kill = std::process::Command::new("kill")
        .args(["-TERM", &pid_arg])
        .output()
        .with_context(|| format!("failed to stop {image_name} (pid {pid}) with kill"))?;
    if !kill.status.success() {
        anyhow::bail!(
            "failed to stop {} (pid {}): {}",
            image_name,
            pid,
            String::from_utf8_lossy(&kill.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(not(any(windows, unix)))]
fn terminate_process(pid: u32, image_name: &str) -> Result<()> {
    anyhow::bail!("cannot stop {image_name} (pid {pid}) on this platform")
}

#[cfg(any(unix, test))]
fn parse_lsof_listening_owner(lsof_output: &str, bind: SocketAddr) -> Option<OccupiedPortOwner> {
    #[derive(Default)]
    struct CurrentOwner {
        pid: Option<u32>,
        image_name: Option<String>,
        matches_bind: bool,
    }

    impl CurrentOwner {
        fn into_match(self) -> Option<OccupiedPortOwner> {
            if !self.matches_bind {
                return None;
            }
            Some(OccupiedPortOwner {
                pid: self.pid?,
                image_name: self.image_name?,
                parent_pid: None,
                executable_path: None,
            })
        }
    }

    let mut current = CurrentOwner::default();
    for line in lsof_output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some(pid) = line.strip_prefix('p') {
            if let Some(owner) = current.into_match() {
                return Some(owner);
            }
            current = CurrentOwner {
                pid: pid.parse().ok(),
                ..CurrentOwner::default()
            };
            continue;
        }

        if let Some(command) = line.strip_prefix('c') {
            current.image_name = Some(command.to_string());
            continue;
        }

        if let Some(name) = line.strip_prefix('n') {
            if lsof_name_covers_bind(name, bind) {
                current.matches_bind = true;
            }
        }
    }

    current.into_match()
}

#[cfg(any(unix, test))]
fn lsof_name_covers_bind(name: &str, bind: SocketAddr) -> bool {
    name.split_whitespace()
        .map(|token| token.trim_end_matches(',').trim_end_matches(';'))
        .any(|token| local_endpoint_covers_bind(token, bind))
}

#[cfg(any(unix, test))]
fn local_endpoint_covers_bind(endpoint: &str, bind: SocketAddr) -> bool {
    let Some((host, port)) = split_endpoint(endpoint) else {
        return false;
    };
    if port != bind.port() {
        return false;
    }
    let host = host.trim_matches(['[', ']']);
    if host == "*" {
        return true;
    }
    let Ok(endpoint_ip) = host.parse::<std::net::IpAddr>() else {
        return false;
    };
    if endpoint_ip.is_unspecified() {
        return true;
    }
    endpoint_ip == bind.ip()
}

#[cfg(any(unix, test))]
fn split_endpoint(endpoint: &str) -> Option<(&str, u16)> {
    let endpoint = endpoint.trim();
    if let Some(rest) = endpoint.strip_prefix('[') {
        let close = rest.rfind(']')?;
        let host = &rest[..close];
        let port = rest.get(close + 1..)?.strip_prefix(':')?.parse().ok()?;
        return Some((host, port));
    }

    let (host, port) = endpoint.rsplit_once(':')?;
    Some((host, port.parse().ok()?))
}

async fn emit_local_app_event(
    State(state): State<EventServerState>,
    headers: HeaderMap,
    Json(request): Json<EmitLocalAppEventRequest>,
) -> Result<(StatusCode, Json<EmitLocalAppEventResponse>), EventApiError> {
    if !state.event_enabled {
        return Err(EventApiError::new(
            StatusCode::NOT_FOUND,
            "local event API is disabled",
        ));
    }
    let connector_id = request.connector_id.trim();
    let event_name = request.event.trim();
    if connector_id.is_empty() || event_name.is_empty() {
        return Err(EventApiError::new(
            StatusCode::BAD_REQUEST,
            "connectorId and event are required",
        ));
    }
    let token = bearer_token(&headers).ok_or_else(|| {
        EventApiError::new(
            StatusCode::UNAUTHORIZED,
            "connector event credential is required",
        )
    })?;
    authorize_connector_event(&state.config_path, connector_id, event_name, &token)
        .map_err(|err| EventApiError::new(StatusCode::FORBIDDEN, err.to_string()))?;
    let event_id = request
        .event_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let event = LocalAppEventEmitted {
        event_id: event_id.clone(),
        connector_id: connector_id.to_string(),
        event: event_name.to_string(),
        payload: request.payload,
        occurred_at: request
            .occurred_at
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
    };
    let (response_tx, response_rx) = oneshot::channel();
    let ack = timeout(LOCAL_APP_EVENT_FORWARD_TIMEOUT, async {
        state
            .event_tx
            .send(LocalAppEventSubmission {
                event,
                response: response_tx,
            })
            .await
            .map_err(|_| "relay runtime is not available".to_string())?;
        response_rx.await.map_err(|_| {
            "relay connection ended before Event Center acknowledged the event".to_string()
        })?
    })
    .await
    .map_err(|_| {
        EventApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "timed out waiting for Event Center acknowledgement",
        )
    })?
    .map_err(|message| EventApiError::new(StatusCode::SERVICE_UNAVAILABLE, message))?;
    let persisted = ack.matched_subscription_count > 0;

    emit_audit_log(
        &state,
        "info",
        format!(
            "local app event {connector_id}.{event_name} forwarded; matched {} subscription(s)",
            ack.matched_subscription_count
        ),
        LogMetadata::category("local_app_event")
            .event(event_name.to_string())
            .event_id(event_id.clone())
            .outcome(if persisted { "persisted" } else { "ignored" }),
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(EmitLocalAppEventResponse {
            accepted: true,
            persisted,
            matched_subscription_count: ack.matched_subscription_count,
            duplicate: ack.duplicate,
            event_id,
            connector_id: connector_id.to_string(),
            event: event_name.to_string(),
        }),
    ))
}

async fn upload_local_app_asset(
    State(state): State<EventServerState>,
    headers: HeaderMap,
    Json(request): Json<UploadLocalAppAssetRequest>,
) -> Result<Json<UploadLocalAppAssetResponse>, EventApiError> {
    let connector_id = request.connector_id.trim();
    let purpose = request.purpose.trim();
    let content_type = request.content_type.trim();
    if connector_id.is_empty() || purpose.is_empty() || request.local_path.trim().is_empty() {
        return Err(EventApiError::new(
            StatusCode::BAD_REQUEST,
            "connectorId, localPath and purpose are required",
        ));
    }
    if purpose.len() > 64
        || !purpose
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(EventApiError::new(
            StatusCode::BAD_REQUEST,
            "purpose must contain at most 64 ASCII letters, digits, dot, dash or underscore",
        ));
    }
    if !matches!(content_type, "image/png" | "image/jpeg" | "image/webp") {
        return Err(EventApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "contentType must be image/png, image/jpeg or image/webp",
        ));
    }
    let token = bearer_token(&headers).ok_or_else(|| {
        EventApiError::new(
            StatusCode::UNAUTHORIZED,
            "connector asset upload credential is required",
        )
    })?;
    let data_dir = authorize_connector_asset_upload(&state.config_path, connector_id, &token)
        .map_err(|err| EventApiError::new(StatusCode::FORBIDDEN, err.to_string()))?;

    let canonical_data_dir = fs::canonicalize(&data_dir).map_err(internal_error)?;
    let canonical_path = fs::canonicalize(request.local_path.trim()).map_err(|err| {
        EventApiError::new(
            StatusCode::BAD_REQUEST,
            format!("cannot read localPath: {err}"),
        )
    })?;
    if !canonical_path.starts_with(&canonical_data_dir) || !canonical_path.is_file() {
        return Err(EventApiError::new(
            StatusCode::FORBIDDEN,
            "localPath must be a regular file inside the connector data directory",
        ));
    }
    let bytes = fs::read(&canonical_path).map_err(internal_error)?;
    let size_bytes = bytes.len() as u64;
    if size_bytes == 0 || size_bytes > MAX_CONNECTOR_ASSET_BYTES {
        return Err(EventApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("asset must contain 1 to {MAX_CONNECTOR_ASSET_BYTES} bytes"),
        ));
    }
    if !content_type_matches_bytes(content_type, &bytes) {
        return Err(EventApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "contentType does not match the image bytes",
        ));
    }
    let prepare_url = state.upload_prepare_url.as_deref().ok_or_else(|| {
        EventApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Bridge Agent upload prepare URL is not configured",
        )
    })?;
    let requested_file_name = request
        .file_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| canonical_path.file_name().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("connector-asset.bin"));
    let file_name = requested_file_name
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .unwrap_or("connector-asset.bin")
        .to_string();
    let scoped_purpose = format!(
        "connector_{}_{}",
        connector_id
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
            .collect::<String>(),
        purpose
    );
    let prepare = state
        .http_client
        .post(prepare_url)
        .timeout(Duration::from_secs(state.upload_timeout_secs))
        .bearer_auth(&state.relay_token)
        .json(&PrepareAssetUploadRequest {
            agent_id: state.agent_id.clone(),
            workspace_id: state.workspace_id,
            purpose: scoped_purpose,
            content_type: content_type.to_string(),
            file_name,
            size_bytes,
        })
        .send()
        .await
        .map_err(|err| EventApiError::new(StatusCode::BAD_GATEWAY, err.to_string()))?;
    if !prepare.status().is_success() {
        let status = prepare.status();
        let body = prepare.text().await.unwrap_or_default();
        return Err(EventApiError::new(
            StatusCode::BAD_GATEWAY,
            format!(
                "prepare upload returned {status}: {}",
                body.chars().take(240).collect::<String>()
            ),
        ));
    }
    let slot: PrepareAssetUploadResponse = prepare.json().await.map_err(internal_error)?;
    let method = slot
        .method
        .as_deref()
        .unwrap_or("PUT")
        .parse::<Method>()
        .map_err(internal_error)?;
    let mut upload = state
        .http_client
        .request(method, &slot.upload_url)
        .timeout(Duration::from_secs(state.upload_timeout_secs))
        .body(bytes.clone());
    for (name, value) in &slot.headers {
        upload = upload.header(name, value);
    }
    if !slot
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("content-type"))
    {
        upload = upload.header(reqwest::header::CONTENT_TYPE, content_type);
    }
    let uploaded = upload
        .send()
        .await
        .map_err(|err| EventApiError::new(StatusCode::BAD_GATEWAY, err.to_string()))?;
    if !uploaded.status().is_success() {
        return Err(EventApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("asset upload returned {}", uploaded.status()),
        ));
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    emit_audit_log(
        &state,
        "info",
        format!("connector asset {connector_id}.{purpose} uploaded"),
        LogMetadata::category("local_app_asset").outcome("uploaded"),
    );
    Ok(Json(UploadLocalAppAssetResponse {
        result_type: "asset_ref",
        asset_id: slot.file_id,
        object_key: slot.object_key,
        download_url: slot.download_url,
        expires_at: slot.expires_at,
        mime_type: content_type.to_string(),
        size_bytes,
        sha256,
    }))
}

fn content_type_matches_bytes(content_type: &str, bytes: &[u8]) -> bool {
    match content_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/webp" => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

async fn list_services(
    State(state): State<EventServerState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ServiceConfig>>, EventApiError> {
    authorize_service_registration(&state, &headers)?;
    let config = load_config(&state.config_path).map_err(internal_error)?;
    Ok(Json(config.services))
}

async fn register_service(
    State(state): State<EventServerState>,
    headers: HeaderMap,
    Json(request): Json<RegisterServiceRequest>,
) -> Result<(StatusCode, Json<ServiceManagementResponse>), EventApiError> {
    authorize_service_registration(&state, &headers)?;
    let (service, replace) = service_request_parts(request)?;
    let response = upsert_service(&state, service, replace).await?;
    emit_service_audit_log(
        &state,
        "registered",
        &response.service.name,
        response.replaced,
    );
    Ok((StatusCode::CREATED, Json(response)))
}

async fn replace_service(
    State(state): State<EventServerState>,
    Path(service_name): Path<String>,
    headers: HeaderMap,
    Json(request): Json<RegisterServiceRequest>,
) -> Result<Json<ServiceManagementResponse>, EventApiError> {
    authorize_service_registration(&state, &headers)?;
    let (mut service, _) = service_request_parts(request)?;
    if service.name.trim().is_empty() {
        service.name = service_name;
    } else if service.name.trim() != service_name.trim() {
        return Err(EventApiError::new(
            StatusCode::BAD_REQUEST,
            "service name in path and body must match",
        ));
    }
    let response = upsert_service(&state, service, true).await?;
    emit_service_audit_log(
        &state,
        "replaced",
        &response.service.name,
        response.replaced,
    );
    Ok(Json(response))
}

async fn delete_service(
    State(state): State<EventServerState>,
    Path(service_name): Path<String>,
    headers: HeaderMap,
) -> Result<Json<DeleteServiceResponse>, EventApiError> {
    authorize_service_registration(&state, &headers)?;
    let service_name = service_name.trim();
    if service_name.is_empty() {
        return Err(EventApiError::new(
            StatusCode::BAD_REQUEST,
            "service name cannot be empty",
        ));
    }

    let mut config = load_config(&state.config_path).map_err(internal_error)?;
    let initial_len = config.services.len();
    config
        .services
        .retain(|service| service.name != service_name);
    let deleted = config.services.len() != initial_len;
    if !deleted {
        return Err(EventApiError::new(
            StatusCode::NOT_FOUND,
            format!("service `{service_name}` is not registered"),
        ));
    }
    save_config(&state.config_path, &config).map_err(internal_error)?;
    apply_config_update(&state, &config).await?;
    emit_service_audit_log(&state, "deleted", service_name, deleted);
    Ok(Json(DeleteServiceResponse {
        service: service_name.to_string(),
        deleted,
        runtime_applied: true,
    }))
}

fn emit_service_audit_log(
    state: &EventServerState,
    outcome: &str,
    service_name: &str,
    _replaced: bool,
) {
    let metadata = LogMetadata::category("service_registration")
        .service(service_name.to_string())
        .outcome(outcome.to_string());
    emit_audit_log(
        state,
        "info",
        format!("local service {service_name} {outcome}"),
        metadata,
    );
}

fn service_request_parts(
    request: RegisterServiceRequest,
) -> Result<(ServiceConfig, bool), EventApiError> {
    match request {
        RegisterServiceRequest::Public(registration) => {
            if !registration.local_app_events.is_empty() {
                return Err(bad_request(
                    "Service custom events are retired; declare Connector events in local_apps instead",
                ));
            }
            let replace = registration.replace;
            let service = registration.into_service_config().map_err(bad_request)?;
            Ok((service, replace))
        }
        RegisterServiceRequest::Raw { service, replace } => Ok((service, replace)),
    }
}

async fn upsert_service(
    state: &EventServerState,
    service: ServiceConfig,
    replace: bool,
) -> Result<ServiceManagementResponse, EventApiError> {
    let service_name = service.name.trim().to_string();
    if service_name.is_empty() {
        return Err(EventApiError::new(
            StatusCode::BAD_REQUEST,
            "service name cannot be empty",
        ));
    }

    let mut config = load_config(&state.config_path).map_err(internal_error)?;
    let existing_index = config
        .services
        .iter()
        .position(|candidate| candidate.name == service_name);
    let replaced = existing_index.is_some();
    match existing_index {
        Some(index) if replace => config.services[index] = service.clone(),
        Some(_) => {
            return Err(EventApiError::new(
                StatusCode::CONFLICT,
                format!("service `{service_name}` already exists; set replace=true to overwrite"),
            ))
        }
        None => config.services.push(service.clone()),
    }

    save_config(&state.config_path, &config).map_err(bad_request)?;
    apply_config_update(state, &config).await?;
    Ok(ServiceManagementResponse {
        service,
        replaced,
        runtime_applied: true,
    })
}

async fn apply_config_update(
    state: &EventServerState,
    config: &crate::config::AgentConfig,
) -> Result<(), EventApiError> {
    let config_base_dir = resolve_config_base_dir(&state.config_path);
    let registry = ServiceRegistry::from_config(config, &config_base_dir).map_err(bad_request)?;
    let relay_registry = ServiceRegistry::from_config_checked(config, &config_base_dir)
        .await
        .map_err(bad_request)?;
    let services = relay_registry.definitions();
    let local_apps = relay_registry.local_app_definitions();
    {
        let mut current = state.registry.write().await;
        *current = registry;
    }
    state
        .apply_tx
        .send(RuntimeRegistryUpdate {
            registry: relay_registry,
            services,
            local_apps,
        })
        .map_err(|_| {
            EventApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime is not accepting service updates",
            )
        })?;
    Ok(())
}

fn authorize_service_registration(
    state: &EventServerState,
    headers: &HeaderMap,
) -> Result<(), EventApiError> {
    if !state.service_registration_enabled {
        return Err(EventApiError::new(
            StatusCode::NOT_FOUND,
            "local service registration API is disabled",
        ));
    }
    authorize_token(
        &state.service_registration_token,
        headers,
        "service registration",
    )
}

fn authorize_token(
    token: &Option<String>,
    headers: &HeaderMap,
    label: &str,
) -> Result<(), EventApiError> {
    let Some(token) = token.as_deref() else {
        return Ok(());
    };

    if bearer_token(headers).as_deref() == Some(token)
        || headers
            .get("x-bridge-agent-event-token")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            == Some(token)
    {
        return Ok(());
    }

    Err(EventApiError::new(
        StatusCode::UNAUTHORIZED,
        format!("invalid {label} token"),
    ))
}

fn bad_request(err: impl std::fmt::Display) -> EventApiError {
    EventApiError::new(StatusCode::BAD_REQUEST, err.to_string())
}

fn internal_error(err: impl std::fmt::Display) -> EventApiError {
    EventApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = value.strip_prefix("Bearer ")?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn emit_audit_log(
    state: &EventServerState,
    level: impl Into<String>,
    message: impl Into<String>,
    metadata: LogMetadata,
) {
    let _ = state.audit_tx.send(RuntimeAuditLog {
        level: level.into(),
        message: message.into(),
        metadata,
    });
}

#[cfg(test)]
mod tests {
    use super::{
        content_type_matches_bytes, local_endpoint_covers_bind, parse_lsof_listening_owner,
        windows_listener_matches, LocalEventServer,
    };
    use crate::config::{save_config, AgentConfig};
    use crate::services::ServiceRegistry;
    use serde_json::json;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::{mpsc, watch, RwLock};

    #[test]
    fn connector_asset_content_type_requires_matching_magic_bytes() {
        assert!(content_type_matches_bytes(
            "image/png",
            b"\x89PNG\r\n\x1a\nrest"
        ));
        assert!(content_type_matches_bytes(
            "image/jpeg",
            &[0xff, 0xd8, 0xff, 0xe0]
        ));
        assert!(content_type_matches_bytes(
            "image/webp",
            b"RIFF\x04\x00\x00\x00WEBP"
        ));
        assert!(!content_type_matches_bytes("image/png", b"not an image"));
        assert!(!content_type_matches_bytes(
            "image/jpeg",
            b"\x89PNG\r\n\x1a\n"
        ));
    }

    #[test]
    fn windows_listener_match_accepts_exact_and_unspecified_addresses() {
        let ipv4_bind: SocketAddr = "127.0.0.1:18081".parse().unwrap();
        let ipv6_bind: SocketAddr = "[::1]:18082".parse().unwrap();

        assert!(windows_listener_matches(
            ipv4_bind,
            "127.0.0.1".parse().unwrap(),
            u16::to_be(18081).into()
        ));
        assert!(windows_listener_matches(
            ipv4_bind,
            "0.0.0.0".parse().unwrap(),
            u16::to_be(18081).into()
        ));
        assert!(windows_listener_matches(
            ipv6_bind,
            "::".parse().unwrap(),
            u16::to_be(18082).into()
        ));
        assert!(!windows_listener_matches(
            ipv4_bind,
            "127.0.0.2".parse().unwrap(),
            u16::to_be(18081).into()
        ));
        assert!(!windows_listener_matches(
            ipv4_bind,
            "127.0.0.1".parse().unwrap(),
            u16::to_be(18082).into()
        ));
    }

    #[test]
    fn local_endpoint_does_not_match_different_loopback_address() {
        let bind: SocketAddr = "127.0.0.1:18081".parse().unwrap();

        assert!(!local_endpoint_covers_bind("127.0.0.2:18081", bind));
        assert!(!local_endpoint_covers_bind("127.0.0.1:18082", bind));
    }

    #[test]
    fn parse_lsof_owner_matches_loopback_listener() {
        let output = r#"
p1234
c百积木
n127.0.0.1:18081
p5678
cnode
n127.0.0.1:18082
"#;
        let bind: SocketAddr = "127.0.0.1:18081".parse().unwrap();

        assert_eq!(
            parse_lsof_listening_owner(output, bind),
            Some(super::OccupiedPortOwner {
                pid: 1234,
                image_name: "百积木".to_string(),
                parent_pid: None,
                executable_path: None,
            })
        );
    }

    #[test]
    fn parse_lsof_owner_treats_unspecified_listener_as_occupying_bind() {
        let output = r#"
p4321
cbridge-agent
n*:18081
"#;
        let bind: SocketAddr = "127.0.0.1:18081".parse().unwrap();

        assert_eq!(
            parse_lsof_listening_owner(output, bind),
            Some(super::OccupiedPortOwner {
                pid: 4321,
                image_name: "bridge-agent".to_string(),
                parent_pid: None,
                executable_path: None,
            })
        );
    }

    #[tokio::test]
    async fn service_registration_api_writes_config_and_schedules_capability_update() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.runtime.event_server_bind = "127.0.0.1:0".to_string();
        config.runtime.service_registration_enabled = true;
        config.runtime.service_registration_token = Some("secret".to_string());
        save_config(&config_path, &config).unwrap();

        let registry = Arc::new(RwLock::new(
            ServiceRegistry::from_config(&config, dir.path()).unwrap(),
        ));
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (apply_tx, mut apply_rx) = mpsc::unbounded_channel();
        let (audit_tx, _audit_rx) = mpsc::unbounded_channel();
        let server = LocalEventServer::bind(
            &config,
            config_path.clone(),
            registry,
            event_tx,
            apply_tx,
            audit_tx,
        )
        .await
        .unwrap()
        .unwrap();
        let addr = server.bind_addr();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(server.serve(shutdown_rx));

        let client = reqwest::Client::new();
        let retired_event_response = client
            .post(format!("http://{addr}/v1/services"))
            .bearer_auth("secret")
            .json(&json!({
                "name": "retiredEventTool",
                "description": "Must not restore retired Service events.",
                "transport": {
                    "type": "http",
                    "baseUrl": "http://127.0.0.1:39127"
                },
                "methods": [{
                    "name": "generate",
                    "description": "Generate a report.",
                    "path": "/invoke/generate"
                }],
                "events": [{
                    "name": "finished",
                    "description": "Retired Service event."
                }]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(retired_event_response.status().as_u16(), 400);
        assert!(retired_event_response
            .text()
            .await
            .unwrap()
            .contains("Service custom events are retired"));

        let response = client
            .post(format!("http://{addr}/v1/services"))
            .bearer_auth("secret")
            .json(&json!({
                "name": "reportTool",
                "description": "AI generated report service.",
                "transport": {
                    "type": "http",
                    "baseUrl": "http://127.0.0.1:39127"
                },
                "methods": [
                    {
                        "name": "generate",
                        "description": "Generate a report.",
                        "path": "/invoke/generate"
                    }
                ],
                "replace": true
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 201);
        let updated = crate::config::load_config(&config_path).unwrap();
        assert!(updated
            .services
            .iter()
            .any(|service| service.name == "reportTool"));
        let update = apply_rx.recv().await.unwrap();
        assert!(update
            .services
            .iter()
            .any(|service| service.name == "reportTool"));

        shutdown_tx.send(true).unwrap();
        task.await.unwrap().unwrap();
    }
}
