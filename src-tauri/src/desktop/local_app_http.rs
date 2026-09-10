use super::*;

#[derive(Clone)]
pub(super) struct LocalAppUiHttpState {
    pub(super) ui_token: String,
    pub(super) control_token: String,
    pub(super) diagnostics: StartupDiagnostics,
    pub(super) config_path: PathBuf,
    pub(super) runtime: AgentRuntimeManager,
    pub(super) connector_lifecycles: ConnectorLifecycleManager,
    pub(super) connector_processes: ConnectorProcessManager,
    pub(super) registered_services: RegisteredServiceMonitor,
    pub(super) local_apps: LocalAppsChangeNotifier,
}

#[derive(Clone)]
pub(super) struct LocalAppUiServerDependencies {
    pub(super) diagnostics: StartupDiagnostics,
    pub(super) config_path: PathBuf,
    pub(super) runtime: AgentRuntimeManager,
    pub(super) connector_lifecycles: ConnectorLifecycleManager,
    pub(super) connector_processes: ConnectorProcessManager,
    pub(super) registered_services: RegisteredServiceMonitor,
    pub(super) local_apps: LocalAppsChangeNotifier,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LocalAppControlDiscovery {
    pub(super) schema_version: String,
    pub(super) pid: u32,
    pub(super) base_url: String,
    pub(super) token: String,
    pub(super) started_at_epoch_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LocalAppControlInstallRequest {
    pub(super) install_source: Option<local_app_contract::InstallSource>,
    pub(super) app_id: String,
    pub(super) version: String,
    pub(super) replace: bool,
    pub(super) start: bool,
    pub(super) accept_unreviewed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LocalAppControlSyncRequest {
    pub(super) accept_unreviewed: bool,
}

#[derive(Clone)]
pub(super) struct ConnectorInstallOptions {
    pub(super) install_source: Option<local_app_contract::InstallSource>,
    pub(super) identity: RegisteredAppVersionIdentity,
    pub(super) replace: bool,
    pub(super) start: bool,
    pub(super) accept_unreviewed: bool,
    pub(super) progress: Option<LocalAppInstallProgressReporter>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LocalAppControlManagementRequest {
    pub(super) payload: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct LocalAppControlUninstallQuery {
    #[serde(default)]
    pub(super) force: bool,
}

pub(super) const LOCAL_APP_UI_BRIDGE_SCRIPT: &str = r#"(() => {
  const REQUEST_TYPE = "baijimu:local-app:invoke";
  const RESPONSE_TYPE = "baijimu:local-app:response";
  const READY_TYPE = "baijimu:local-app:ready";
  const HELLO_TYPE = "baijimu:local-app:hello";
  const pending = new Map();
  let sequence = 0;

  const announceReady = () => {
    window.parent.postMessage({ type: READY_TYPE, version: 1 }, "*");
  };

  window.addEventListener("message", (event) => {
    if (event.source !== window.parent) return;
    const message = event.data;
    if (message && message.type === HELLO_TYPE && message.version === 1) {
      announceReady();
      return;
    }
    if (!message || message.type !== RESPONSE_TYPE || message.version !== 1) return;
    const request = pending.get(message.requestId);
    if (!request) return;
    pending.delete(message.requestId);
    clearTimeout(request.timeout);
    if (message.ok) request.resolve(message.data);
    else request.reject(new Error(message.error || "本地应用管理操作失败"));
  });

  const api = Object.freeze({
    version: 1,
    invoke(operation, payload = null) {
      if (typeof operation !== "string" || !/^[A-Za-z0-9._-]{1,128}$/.test(operation)) {
        return Promise.reject(new Error("management operation 名称无效"));
      }
      const requestId = `${Date.now().toString(36)}-${(++sequence).toString(36)}`;
      return new Promise((resolve, reject) => {
        const timeout = window.setTimeout(() => {
          pending.delete(requestId);
          reject(new Error("本地应用管理操作超时"));
        }, 65000);
        pending.set(requestId, { resolve, reject, timeout });
        window.parent.postMessage({
          type: REQUEST_TYPE,
          version: 1,
          requestId,
          operation,
          payload
        }, "*");
      });
    }
  });

  Object.defineProperty(window, "baijimuLocalApp", {
    value: api,
    configurable: false,
    enumerable: true,
    writable: false
  });
  announceReady();
  window.addEventListener("pageshow", announceReady);
})();
"#;

pub(super) fn start_local_app_ui_server(
    endpoint: Arc<RwLock<Option<LocalAppUiEndpoint>>>,
    startup_health: StartupHealthManager,
    dependencies: LocalAppUiServerDependencies,
) {
    startup_health.set_component("local_app_ui_server", "本地应用界面服务", "starting", None);
    tauri::async_runtime::spawn(async move {
        run_local_app_ui_server(endpoint, startup_health, dependencies).await;
    });
}

async fn run_local_app_ui_server(
    endpoint: Arc<RwLock<Option<LocalAppUiEndpoint>>>,
    startup_health: StartupHealthManager,
    dependencies: LocalAppUiServerDependencies,
) {
    let diagnostics = dependencies.diagnostics.clone();
    let (listener, port) = match bind_local_app_ui_listener().await {
        Ok(bound) => bound,
        Err(detail) => {
            mark_local_app_ui_server_failed(&startup_health, &diagnostics, detail);
            return;
        }
    };
    let (state, control_token) = match prepare_local_app_ui_state(&endpoint, dependencies, port) {
        Ok(prepared) => prepared,
        Err(detail) => {
            mark_local_app_ui_server_failed(&startup_health, &diagnostics, detail);
            return;
        }
    };
    let control_path = local_app_control_discovery_path(&state.config_path);
    if let Err(detail) = write_local_app_control_discovery(&control_path, port, &control_token) {
        mark_local_app_ui_server_failed(&startup_health, &diagnostics, detail);
        return;
    }
    startup_health.set_component(
        "local_app_ui_server",
        "本地应用界面服务",
        "ready",
        Some(format!("127.0.0.1:{port}")),
    );
    diagnostics.info(format!("local app UI server listening on 127.0.0.1:{port}"));
    let serve_result = axum::serve(listener, local_app_ui_router(state)).await;
    let _ = fs::remove_file(&control_path);
    if let Err(err) = serve_result {
        diagnostics.error(format!("local app UI server stopped: {err:#}"));
        if let Ok(mut value) = endpoint.write() {
            *value = None;
        }
        startup_health.set_component(
            "local_app_ui_server",
            "本地应用界面服务",
            "degraded",
            Some(format!("服务已停止: {err}")),
        );
    }
}

async fn bind_local_app_ui_listener() -> Result<(tokio::net::TcpListener, u16), String> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|err| format!("无法监听本机端口: {err}"))?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("无法读取监听地址: {err}"))?
        .port();
    Ok((listener, port))
}

fn prepare_local_app_ui_state(
    endpoint: &RwLock<Option<LocalAppUiEndpoint>>,
    dependencies: LocalAppUiServerDependencies,
    port: u16,
) -> Result<(LocalAppUiHttpState, String), String> {
    let ui_token = uuid::Uuid::new_v4().simple().to_string();
    let control_token = uuid::Uuid::new_v4().simple().to_string();
    *endpoint
        .write()
        .map_err(|_| "本地应用界面状态锁已损坏".to_string())? = Some(LocalAppUiEndpoint {
        port,
        token: ui_token.clone(),
    });
    Ok((
        LocalAppUiHttpState {
            ui_token,
            control_token: control_token.clone(),
            diagnostics: dependencies.diagnostics,
            config_path: dependencies.config_path,
            runtime: dependencies.runtime,
            connector_lifecycles: dependencies.connector_lifecycles,
            connector_processes: dependencies.connector_processes,
            registered_services: dependencies.registered_services,
            local_apps: dependencies.local_apps,
        },
        control_token,
    ))
}

fn local_app_ui_router(state: LocalAppUiHttpState) -> Router {
    Router::new()
        .route("/api/v1/status", get(local_app_control_status_handler))
        .route(
            "/api/v1/local-app-market",
            get(local_app_control_market_handler),
        )
        .route("/api/v1/local-apps", get(local_app_control_list_handler))
        .route(
            "/api/v1/local-apps/install",
            post(local_app_control_install_handler),
        )
        .route(
            "/api/v1/local-apps/{app_id}",
            get(local_app_control_show_handler).delete(local_app_control_uninstall_handler),
        )
        .route(
            "/api/v1/local-apps/{app_id}/start",
            post(local_app_control_start_handler),
        )
        .route(
            "/api/v1/local-apps/{app_id}/stop",
            post(local_app_control_stop_handler),
        )
        .route(
            "/api/v1/local-apps/{app_id}/sync",
            post(local_app_control_sync_handler),
        )
        .route(
            "/api/v1/local-apps/{app_id}/management/{operation}",
            post(local_app_control_management_handler),
        )
        .route("/{token}/{app_id}/", get(local_app_ui_entry_handler))
        .route(
            "/{token}/{app_id}/{*asset_path}",
            get(local_app_ui_asset_handler),
        )
        .with_state(state)
}

fn mark_local_app_ui_server_failed(
    startup_health: &StartupHealthManager,
    diagnostics: &StartupDiagnostics,
    detail: String,
) {
    diagnostics.error(format!("failed to start local app UI server: {detail}"));
    startup_health.set_component(
        "local_app_ui_server",
        "本地应用界面服务",
        "degraded",
        Some(detail),
    );
}

pub(super) fn local_app_control_discovery_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LOCAL_APP_CONTROL_FILE_NAME)
}

pub(super) fn write_local_app_control_discovery(
    path: &Path,
    port: u16,
    token: &str,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "无法确定本机应用控制文件目录".to_string())?;
    fs::create_dir_all(parent).map_err(|err| format!("创建本机应用控制目录失败: {err}"))?;
    let document = LocalAppControlDiscovery {
        schema_version: LOCAL_APP_CONTROL_SCHEMA_VERSION.to_string(),
        pid: std::process::id(),
        base_url: format!("http://127.0.0.1:{port}/api/v1"),
        token: token.to_string(),
        started_at_epoch_ms: now_ms(),
    };
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|err| format!("序列化本机应用控制地址失败: {err}"))?;
    let temporary_path = path.with_extension("json.tmp");
    fs::write(&temporary_path, bytes).map_err(|err| format!("写入本机应用控制地址失败: {err}"))?;
    #[cfg(unix)]
    fs::set_permissions(&temporary_path, fs::Permissions::from_mode(0o600))
        .map_err(|err| format!("保护本机应用控制地址失败: {err}"))?;
    if path.exists() {
        fs::remove_file(path).map_err(|err| format!("替换本机应用控制地址失败: {err}"))?;
    }
    fs::rename(&temporary_path, path).map_err(|err| format!("提交本机应用控制地址失败: {err}"))?;
    Ok(())
}

pub(super) fn local_app_control_is_authorized(
    state: &LocalAppUiHttpState,
    headers: &HeaderMap,
) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .is_some_and(|value| value == state.control_token)
}

pub(super) fn local_app_control_error(
    status: StatusCode,
    message: impl Into<String>,
) -> AxumResponse {
    (
        status,
        Json(serde_json::json!({
            "ok": false,
            "error": { "message": message.into() }
        })),
    )
        .into_response()
}

pub(super) fn local_app_control_success<T: Serialize>(value: T) -> AxumResponse {
    Json(serde_json::json!({ "ok": true, "data": value })).into_response()
}

pub(super) fn local_app_control_result<T: Serialize>(result: Result<T, String>) -> AxumResponse {
    match result {
        Ok(value) => local_app_control_success(value),
        Err(err) => local_app_control_error(StatusCode::BAD_REQUEST, err),
    }
}

include!("local_app_http/handlers.rs");
