use crate::config::RuntimeConfig;
use crate::services::ServiceRegistry;
use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
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
    token: Option<String>,
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
        registry: Arc<RwLock<ServiceRegistry>>,
        event_tx: mpsc::Sender<LocalEventEmitRequest>,
    ) -> Result<Option<Self>> {
        if !config.event_server_enabled {
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
                token,
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
    authorize(&state, &headers)?;

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

fn authorize(state: &EventServerState, headers: &HeaderMap) -> Result<(), EventApiError> {
    let Some(token) = state.token.as_deref() else {
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
        "invalid event server token",
    ))
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
    use crate::config::{AgentConfig, EventConfig, ServiceConfig};
    use crate::services::ServiceRegistry;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::{mpsc, watch, RwLock};

    #[tokio::test]
    async fn event_server_accepts_declared_event() {
        let mut config = AgentConfig::example();
        config.runtime.event_server_bind = "127.0.0.1:0".to_string();
        config.services.push(ServiceConfig {
            name: "asyncJob".to_string(),
            description: "Async job events.".to_string(),
            enabled: true,
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
        let server = LocalEventServer::bind(&config.runtime, registry, event_tx)
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
}
