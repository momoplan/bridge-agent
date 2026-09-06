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
    app_id: String,
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
    app_id: String,
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
    app_id: String,
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
                .filter_map(|app| connector_declares_asset_upload(&app.app_id).ok())
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

include!("event_server/port_management.rs");
include!("event_server/event_emission.rs");
include!("event_server/asset_upload.rs");
include!("event_server/service_management.rs");
include!("event_server/auth_and_audit.rs");

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

    include!("event_server/tests/content_and_ports.rs");
    include!("event_server/tests/service_management.rs");
}
