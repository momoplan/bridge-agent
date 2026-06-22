use crate::config::{
    load_config, resolve_config_base_dir, save_config, RuntimeConfig, ServiceConfig,
    ServiceRegistration,
};
use crate::logging::LogMetadata;
use crate::process_identity::is_bridge_agent_process_name;
use crate::runtime::{RuntimeAuditLog, RuntimeRegistryUpdate};
use crate::services::ServiceRegistry;
use anyhow::{Context, Result};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, watch, RwLock};
use tokio::time::sleep;
use uuid::Uuid;

const PORT_RECLAIM_BIND_RETRIES: usize = 20;
const PORT_RECLAIM_RETRY_DELAY: Duration = Duration::from_millis(150);

#[derive(Debug, Clone)]
pub(crate) struct LocalEventEmitRequest {
    pub event_id: String,
    pub service: String,
    pub event: String,
    pub payload: Value,
    pub occurred_at: Option<String>,
}

pub(crate) struct LocalEventServer {
    bind: SocketAddr,
    listener: TcpListener,
    state: EventServerState,
}

#[derive(Clone)]
struct EventServerState {
    registry: Arc<RwLock<ServiceRegistry>>,
    event_tx: mpsc::Sender<LocalEventEmitRequest>,
    apply_tx: mpsc::UnboundedSender<RuntimeRegistryUpdate>,
    audit_tx: mpsc::UnboundedSender<RuntimeAuditLog>,
    config_path: PathBuf,
    event_enabled: bool,
    event_token: Option<String>,
    service_registration_enabled: bool,
    service_registration_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EmitEventRequest {
    service: String,
    event: String,
    #[serde(default)]
    payload: Value,
    #[serde(default, alias = "eventId")]
    event_id: Option<String>,
    #[serde(default, alias = "occurredAt")]
    occurred_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EmitEventResponse {
    accepted: bool,
    event_id: String,
    service: String,
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
        config: &RuntimeConfig,
        config_path: PathBuf,
        registry: Arc<RwLock<ServiceRegistry>>,
        event_tx: mpsc::Sender<LocalEventEmitRequest>,
        apply_tx: mpsc::UnboundedSender<RuntimeRegistryUpdate>,
        audit_tx: mpsc::UnboundedSender<RuntimeAuditLog>,
    ) -> Result<Option<Self>> {
        if !config.event_server_enabled && !config.service_registration_enabled {
            return Ok(None);
        }

        let bind: SocketAddr = config
            .event_server_bind
            .parse()
            .with_context(|| "runtime.event_server_bind must be a socket address")?;
        let listener = bind_event_listener(bind)
            .await
            .with_context(|| format!("failed to bind local event server on {bind}"))?;
        let bind = listener.local_addr()?;
        let token = config
            .event_server_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);

        Ok(Some(Self {
            bind,
            listener,
            state: EventServerState {
                registry,
                event_tx,
                apply_tx,
                audit_tx,
                config_path,
                event_enabled: config.event_server_enabled,
                event_token: token,
                service_registration_enabled: config.service_registration_enabled,
                service_registration_token: config
                    .service_registration_token
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
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
            .route("/v1/events", post(emit_event))
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
        anyhow::bail!(
            "local event server port {bind} is already occupied by {} (pid {}), not a Bridge Agent process",
            owner.image_name,
            owner.pid
        );
    }

    terminate_process(owner.pid, &owner.image_name)?;
    Ok(Some(owner))
}

#[cfg(windows)]
fn find_occupied_tcp_listener(bind: SocketAddr) -> Result<Option<OccupiedPortOwner>> {
    let netstat = std::process::Command::new("netstat")
        .args(["-ano", "-p", "TCP"])
        .output()
        .context("failed to inspect TCP listeners with netstat")?;
    if !netstat.status.success() {
        anyhow::bail!(
            "netstat failed while inspecting local event server port: {}",
            String::from_utf8_lossy(&netstat.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&netstat.stdout);
    let Some(pid) = parse_listening_pid(&stdout, bind) else {
        return Ok(None);
    };
    let image_name = lookup_process_image_name(pid)?;
    Ok(Some(OccupiedPortOwner { pid, image_name }))
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
fn lookup_process_image_name(pid: u32) -> Result<String> {
    let filter = format!("PID eq {pid}");
    let tasklist = std::process::Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output()
        .with_context(|| format!("failed to inspect process {pid} with tasklist"))?;
    if !tasklist.status.success() {
        anyhow::bail!(
            "tasklist failed while inspecting process {pid}: {}",
            String::from_utf8_lossy(&tasklist.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&tasklist.stdout);
    parse_tasklist_image_name(&stdout).with_context(|| format!("failed to identify process {pid}"))
}

#[cfg(windows)]
fn terminate_process(pid: u32, image_name: &str) -> Result<()> {
    let pid_arg = pid.to_string();
    let taskkill = std::process::Command::new("taskkill")
        .args(["/PID", &pid_arg, "/T", "/F"])
        .output()
        .with_context(|| format!("failed to stop {image_name} (pid {pid}) with taskkill"))?;
    if !taskkill.status.success() {
        anyhow::bail!(
            "failed to stop {} (pid {}): {}",
            image_name,
            pid,
            String::from_utf8_lossy(&taskkill.stderr).trim()
        );
    }
    Ok(())
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

#[cfg(any(windows, test))]
fn parse_listening_pid(netstat_output: &str, bind: SocketAddr) -> Option<u32> {
    for line in netstat_output.lines() {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.len() < 5 {
            continue;
        }
        if !columns[0].eq_ignore_ascii_case("TCP") {
            continue;
        }
        if !columns[3].eq_ignore_ascii_case("LISTENING") {
            continue;
        }
        if !local_endpoint_covers_bind(columns[1], bind) {
            continue;
        }
        if let Ok(pid) = columns[4].parse::<u32>() {
            return Some(pid);
        }
    }
    None
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

#[cfg(any(windows, test))]
fn parse_tasklist_image_name(tasklist_output: &str) -> Option<String> {
    tasklist_output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("INFO:"))
        .find_map(first_csv_field)
}

#[cfg(any(windows, test))]
fn first_csv_field(line: &str) -> Option<String> {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix('"') {
        let mut field = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                    continue;
                }
                return Some(field);
            }
            field.push(ch);
        }
        return None;
    }
    line.split(',').next().map(str::trim).map(ToOwned::to_owned)
}

#[cfg(any(windows, unix, test))]
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

#[cfg(any(windows, unix, test))]
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

async fn emit_event(
    State(state): State<EventServerState>,
    headers: HeaderMap,
    Json(request): Json<EmitEventRequest>,
) -> Result<(StatusCode, Json<EmitEventResponse>), EventApiError> {
    if !state.event_enabled {
        return Err(EventApiError::new(
            StatusCode::NOT_FOUND,
            "local event API is disabled",
        ));
    }
    authorize_token(&state.event_token, &headers, "event server")?;

    let service = request.service.trim();
    let event = request.event.trim();
    if service.is_empty() {
        return Err(EventApiError::new(
            StatusCode::BAD_REQUEST,
            "service cannot be empty",
        ));
    }
    if event.is_empty() {
        return Err(EventApiError::new(
            StatusCode::BAD_REQUEST,
            "event cannot be empty",
        ));
    }

    if !state.registry.read().await.has_event(service, event) {
        return Err(EventApiError::new(
            StatusCode::NOT_FOUND,
            format!("event `{service}.{event}` is not declared or not enabled"),
        ));
    }

    let event_id = request
        .event_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let emit_request = LocalEventEmitRequest {
        event_id: event_id.clone(),
        service: service.to_string(),
        event: event.to_string(),
        payload: request.payload,
        occurred_at: request
            .occurred_at
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
    };

    state
        .event_tx
        .try_send(emit_request)
        .map_err(|err| match err {
            TrySendError::Closed(_) => EventApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime is not accepting events",
            ),
            TrySendError::Full(_) => EventApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "event queue is full; retry later",
            ),
        })?;

    emit_audit_log(
        &state,
        "info",
        format!("local event {service}.{event} accepted"),
        LogMetadata::category("local_event")
            .service(service.to_string())
            .event(event.to_string())
            .event_id(event_id.clone())
            .outcome("accepted"),
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(EmitEventResponse {
            accepted: true,
            event_id,
            service: service.to_string(),
            event: event.to_string(),
        }),
    ))
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
    {
        let mut current = state.registry.write().await;
        *current = registry;
    }
    state
        .apply_tx
        .send(RuntimeRegistryUpdate {
            registry: relay_registry,
            services,
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
        local_endpoint_covers_bind, parse_listening_pid, parse_lsof_listening_owner,
        parse_tasklist_image_name, LocalEventServer,
    };
    use crate::config::{save_config, AgentConfig, EventConfig, ServiceConfig};
    use crate::services::ServiceRegistry;
    use serde_json::json;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::{mpsc, watch, RwLock};

    #[tokio::test]
    async fn event_server_accepts_declared_event() {
        let mut config = AgentConfig::example();
        config.runtime.event_server_bind = "127.0.0.1:0".to_string();
        config.services.push(ServiceConfig {
            name: "asyncJob".to_string(),
            description: "Async job events.".to_string(),
            enabled: true,
            health_check: None,
            start_command: None,
            stop_command: None,
            methods: Vec::new(),
            events: vec![EventConfig {
                name: "finished".to_string(),
                description: "Job finished.".to_string(),
                enabled: true,
                payload_schema: json!({"type": "object"}),
            }],
        });

        let current_dir = std::env::current_dir().unwrap();
        let registry = Arc::new(RwLock::new(
            ServiceRegistry::from_config(&config, &current_dir).unwrap(),
        ));
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let (apply_tx, _apply_rx) = mpsc::unbounded_channel();
        let (audit_tx, _audit_rx) = mpsc::unbounded_channel();
        let config_path = current_dir.join("agent-config.json");
        let server = LocalEventServer::bind(
            &config.runtime,
            config_path,
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

        let response = reqwest::Client::new()
            .post(format!("http://{addr}/v1/events"))
            .json(&json!({
                "service": "asyncJob",
                "event": "finished",
                "payload": {
                    "jobId": "job-1"
                }
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 202);
        let event = event_rx.recv().await.unwrap();
        assert_eq!(event.service, "asyncJob");
        assert_eq!(event.event, "finished");
        assert_eq!(event.payload["jobId"], "job-1");
        assert!(!event.event_id.is_empty());

        shutdown_tx.send(true).unwrap();
        task.await.unwrap().unwrap();
    }

    #[test]
    fn parse_listening_pid_matches_ipv4_loopback_listener() {
        let output = r#"
  Proto  Local Address          Foreign Address        State           PID
  TCP    127.0.0.1:18081        0.0.0.0:0              LISTENING       1234
  TCP    127.0.0.1:18082        0.0.0.0:0              LISTENING       5678
"#;
        let bind: SocketAddr = "127.0.0.1:18081".parse().unwrap();

        assert_eq!(parse_listening_pid(output, bind), Some(1234));
    }

    #[test]
    fn parse_listening_pid_treats_unspecified_listener_as_occupying_bind() {
        let output = r#"
  TCP    0.0.0.0:18081          0.0.0.0:0              LISTENING       4321
  TCP    [::]:18082             [::]:0                 LISTENING       5678
"#;
        let ipv4_bind: SocketAddr = "127.0.0.1:18081".parse().unwrap();
        let ipv6_bind: SocketAddr = "[::1]:18082".parse().unwrap();

        assert_eq!(parse_listening_pid(output, ipv4_bind), Some(4321));
        assert_eq!(parse_listening_pid(output, ipv6_bind), Some(5678));
    }

    #[test]
    fn local_endpoint_does_not_match_different_loopback_address() {
        let bind: SocketAddr = "127.0.0.1:18081".parse().unwrap();

        assert!(!local_endpoint_covers_bind("127.0.0.2:18081", bind));
        assert!(!local_endpoint_covers_bind("127.0.0.1:18082", bind));
    }

    #[test]
    fn parse_tasklist_image_name_reads_csv_first_field() {
        let output = r#""Bridge Agent.exe","1234","Console","1","64,000 K""#;

        assert_eq!(
            parse_tasklist_image_name(output),
            Some("Bridge Agent.exe".to_string())
        );
    }

    #[test]
    fn parse_lsof_owner_matches_loopback_listener() {
        let output = r#"
p1234
cBridge Agent
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
                image_name: "Bridge Agent".to_string()
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
                image_name: "bridge-agent".to_string()
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
            &config.runtime,
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

        let response = reqwest::Client::new()
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
