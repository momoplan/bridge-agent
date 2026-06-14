use crate::config::{
    load_config, resolve_config_base_dir, save_config, RuntimeConfig, ServiceConfig,
    ServiceRegistration,
};
use crate::runtime::RuntimeRegistryUpdate;
use crate::services::ServiceRegistry;
use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, watch, RwLock};
use uuid::Uuid;

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
    ) -> Result<Option<Self>> {
        if !config.event_server_enabled && !config.service_registration_enabled {
            return Ok(None);
        }

        let bind: SocketAddr = config
            .event_server_bind
            .parse()
            .with_context(|| "runtime.event_server_bind must be a socket address")?;
        let listener = TcpListener::bind(bind)
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
        let app = Router::new()
            .route("/healthz", get(|| async { "ok" }))
            .route("/v1/events", post(emit_event))
            .route("/v1/services", get(list_services).post(register_service))
            .route(
                "/v1/services/{service}",
                put(replace_service).delete(delete_service),
            )
            .with_state(self.state);

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
    Ok(Json(upsert_service(&state, service, true).await?))
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
    Ok(Json(DeleteServiceResponse {
        service: service_name.to_string(),
        deleted,
        runtime_applied: true,
    }))
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

#[cfg(test)]
mod tests {
    use super::LocalEventServer;
    use crate::config::{save_config, AgentConfig, EventConfig, ServiceConfig};
    use crate::services::ServiceRegistry;
    use serde_json::json;
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
        let config_path = current_dir.join("agent-config.json");
        let server =
            LocalEventServer::bind(&config.runtime, config_path, registry, event_tx, apply_tx)
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
        let server = LocalEventServer::bind(
            &config.runtime,
            config_path.clone(),
            registry,
            event_tx,
            apply_tx,
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
