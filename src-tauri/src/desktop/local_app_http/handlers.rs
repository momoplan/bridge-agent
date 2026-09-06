pub(super) async fn local_app_control_status_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = async {
        let config = load_agent_config(&state.config_path).map_err(|err| err.to_string())?;
        Ok::<_, String>(serde_json::json!({
            "schemaVersion": LOCAL_APP_CONTROL_SCHEMA_VERSION,
            "pid": std::process::id(),
            "version": env!("CARGO_PKG_VERSION"),
            "configPath": state.config_path.display().to_string(),
            "authorized": config_is_authorized(&config),
            "workspaceId": config.platform.workspace_id,
            "relayTokenConfigured": !config.relay.token.trim().is_empty(),
            "runtime": state.runtime.snapshot().await
        }))
    }
    .await;
    local_app_control_result(result)
}

pub(super) async fn local_app_control_market_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    local_app_control_result(fetch_market_connector_apps(&state.config_path).await)
}

pub(super) async fn local_app_control_list_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = async {
        let apps = list_connectors().map_err(|err| err.to_string())?;
        let services = state.registered_services.statuses().await?;
        Ok::<_, String>(serde_json::json!({
            "apps": apps,
            "syncFailures": [],
            "services": services,
            "lifecycles": state.connector_lifecycles.list(),
            "runtime": state.runtime.snapshot().await
        }))
    }
    .await;
    local_app_control_result(result)
}

pub(super) async fn local_app_control_show_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath(app_id): AxumPath<String>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = async {
        let record = show_connector(app_id.trim()).map_err(|err| err.to_string())?;
        let process_running = state
            .connector_processes
            .managed_running(&record.manifest.app_id)
            .await;
        let status = connector_local_app_status(
            &state.config_path,
            &record.manifest.app_id,
            process_running,
        )
        .await?;
        Ok::<_, String>(serde_json::json!({
            "app": record,
            "status": status,
            "lifecycle": state.connector_lifecycles.list().into_iter()
                .find(|snapshot| snapshot.app_id == app_id.trim()),
            "runtime": state.runtime.snapshot().await
        }))
    }
    .await;
    local_app_control_result(result)
}

pub(super) async fn local_app_control_install_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    headers: HeaderMap,
    Json(request): Json<LocalAppControlInstallRequest>,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = async {
        let identity = RegisteredAppVersionIdentity::parse(request.app_id, request.version)?;
        let document = install_connector_app_with_context(
            &state.config_path,
            &state.runtime,
            &state.connector_lifecycles,
            &state.connector_processes,
            &state.registered_services,
            ConnectorInstallOptions {
                identity,
                replace: request.replace,
                start: request.start,
                accept_unreviewed: request.accept_unreviewed,
                progress: None,
            },
        )
        .await?;
        state
            .local_apps
            .notify(LocalAppsChangeOperation::Install, &document.install.app_id);
        Ok::<_, String>(document)
    }
    .await;
    local_app_control_result(result)
}

pub(super) async fn local_app_control_start_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath(app_id): AxumPath<String>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let bundled_cli = bundled_baijimu_cli_path();
    let result = start_connector_with_lifecycle(
        &state.connector_lifecycles,
        &state.connector_processes,
        &state.config_path,
        app_id.trim(),
        "启动应用",
        bundled_cli.as_deref(),
    )
    .await;
    state.registered_services.request_refresh();
    local_app_control_result(result)
}

pub(super) async fn local_app_control_stop_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath(app_id): AxumPath<String>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = stop_connector_with_lifecycle(
        &state.connector_lifecycles,
        &state.connector_processes,
        &state.config_path,
        app_id.trim(),
        "停止应用",
    )
    .await;
    state.registered_services.request_refresh();
    local_app_control_result(result)
}

pub(super) async fn local_app_control_sync_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath(app_id): AxumPath<String>,
    headers: HeaderMap,
    Json(request): Json<LocalAppControlSyncRequest>,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = async {
        let record = show_connector(app_id.trim()).map_err(|err| err.to_string())?;
        let identity =
            RegisteredAppVersionIdentity::parse(record.manifest.app_id, record.manifest.version)?;
        let document = install_connector_app_with_context(
            &state.config_path,
            &state.runtime,
            &state.connector_lifecycles,
            &state.connector_processes,
            &state.registered_services,
            ConnectorInstallOptions {
                identity,
                replace: true,
                start: true,
                accept_unreviewed: request.accept_unreviewed,
                progress: None,
            },
        )
        .await?;
        state
            .local_apps
            .notify(LocalAppsChangeOperation::Sync, &document.install.app_id);
        Ok::<_, String>(document)
    }
    .await;
    local_app_control_result(result)
}

pub(super) async fn local_app_control_management_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath((app_id, operation)): AxumPath<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<LocalAppControlManagementRequest>,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    match invoke_connector_management_with_context(
        &state.connector_lifecycles,
        &state.connector_processes,
        app_id,
        operation,
        request.payload,
    )
    .await
    {
        Ok(value) => local_app_control_success(value),
        Err(error) => {
            let status = if error.code == "connector_not_ready" {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            (
                status,
                Json(serde_json::json!({ "ok": false, "error": error })),
            )
                .into_response()
        }
    }
}

pub(super) async fn local_app_control_uninstall_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath(app_id): AxumPath<String>,
    AxumQuery(query): AxumQuery<LocalAppControlUninstallQuery>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    let result = async {
        let app_id = app_id.trim().to_string();
        let document = uninstall_connector_app_with_context(
            &state.config_path,
            &state.runtime,
            &state.connector_lifecycles,
            &state.connector_processes,
            &state.registered_services,
            app_id.clone(),
            query.force,
        )
        .await
        .map_err(|error| error.message().to_string())?;
        state
            .local_apps
            .notify(LocalAppsChangeOperation::Uninstall, &app_id);
        Ok::<_, String>(document)
    }
    .await;
    local_app_control_result(result)
}

pub(super) async fn local_app_ui_entry_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath((token, app_id)): AxumPath<(String, String)>,
    headers: HeaderMap,
) -> AxumResponse {
    serve_local_app_ui_asset(&state, &token, &app_id, None, &headers).await
}

pub(super) async fn local_app_ui_asset_handler(
    AxumState(state): AxumState<LocalAppUiHttpState>,
    AxumPath((token, app_id, asset_path)): AxumPath<(String, String, String)>,
    headers: HeaderMap,
) -> AxumResponse {
    serve_local_app_ui_asset(&state, &token, &app_id, Some(&asset_path), &headers).await
}

pub(super) async fn serve_local_app_ui_asset(
    state: &LocalAppUiHttpState,
    token: &str,
    app_id: &str,
    asset_path: Option<&str>,
    headers: &HeaderMap,
) -> AxumResponse {
    let asset_kind = match asset_path {
        None => "entry",
        Some(LOCAL_APP_UI_BRIDGE_ASSET) => "bridge",
        Some(_) => "asset",
    };
    if token != state.ui_token || !local_app_ui_request_host_matches(headers, token, app_id) {
        state.diagnostics.warn(format!(
            "local app UI request: app_id={app_id} asset_kind={asset_kind} outcome=rejected reason=invalid_endpoint"
        ));
        return local_app_ui_error(StatusCode::NOT_FOUND, "not found");
    }
    if asset_path == Some(LOCAL_APP_UI_BRIDGE_ASSET) {
        state.diagnostics.info(format!(
            "local app UI request: app_id={app_id} asset_kind=bridge outcome=served status=200"
        ));
        return local_app_ui_response(
            StatusCode::OK,
            "application/javascript; charset=utf-8",
            LOCAL_APP_UI_BRIDGE_SCRIPT.as_bytes().to_vec(),
        );
    }

    let record = match show_connector(app_id) {
        Ok(record) => record,
        Err(_) => {
            state.diagnostics.warn(format!(
                "local app UI request: app_id={app_id} asset_kind={asset_kind} outcome=rejected reason=application_not_found"
            ));
            return local_app_ui_error(StatusCode::NOT_FOUND, "application not found");
        }
    };
    let Some(ui) = record.manifest.ui.as_ref() else {
        state.diagnostics.warn(format!(
            "local app UI request: app_id={app_id} asset_kind={asset_kind} outcome=rejected reason=ui_not_declared"
        ));
        return local_app_ui_error(StatusCode::NOT_FOUND, "application UI not found");
    };
    let package_path = Path::new(&record.package_path);
    let resolved = match resolve_connector_ui_asset(package_path, ui, asset_path) {
        Ok(path) => path,
        Err(_) => {
            state.diagnostics.warn(format!(
                "local app UI request: app_id={app_id} asset_kind={asset_kind} outcome=rejected reason=asset_not_found"
            ));
            return local_app_ui_error(StatusCode::NOT_FOUND, "asset not found");
        }
    };
    let mut body = match tokio::fs::read(&resolved).await {
        Ok(body) => body,
        Err(_) => {
            state.diagnostics.warn(format!(
                "local app UI request: app_id={app_id} asset_kind={asset_kind} outcome=rejected reason=asset_read_failed"
            ));
            return local_app_ui_error(StatusCode::NOT_FOUND, "asset not found");
        }
    };
    if asset_path.is_none() {
        body = match inject_local_app_ui_bridge(body) {
            Ok(body) => body,
            Err(message) => {
                state.diagnostics.warn(format!(
                    "local app UI request: app_id={app_id} asset_kind=entry outcome=rejected reason=bridge_injection_failed"
                ));
                return local_app_ui_error(StatusCode::UNPROCESSABLE_ENTITY, &message);
            }
        };
    }
    state.diagnostics.info(format!(
        "local app UI request: app_id={app_id} asset_kind={asset_kind} outcome=served status=200"
    ));
    local_app_ui_response(StatusCode::OK, local_app_ui_content_type(&resolved), body)
}

pub(super) fn local_app_ui_host(token: &str, app_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.update([0]);
    hasher.update(app_id.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("app-{}.localhost", &digest[..20])
}

pub(super) fn local_app_ui_request_host_matches(
    headers: &HeaderMap,
    token: &str,
    app_id: &str,
) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let host_without_port = host.split_once(':').map_or(host, |(host, _)| host);
    host_without_port.eq_ignore_ascii_case(&local_app_ui_host(token, app_id))
}

pub(super) fn inject_local_app_ui_bridge(body: Vec<u8>) -> Result<Vec<u8>, String> {
    let html = String::from_utf8(body)
        .map_err(|_| "application UI entry must be UTF-8 HTML".to_string())?;
    let script = format!(r#"<script src="./{LOCAL_APP_UI_BRIDGE_ASSET}"></script>"#);
    let lower = html.to_ascii_lowercase();
    let injected = if let Some(index) = lower.find("</head>") {
        format!("{}{}{}", &html[..index], script, &html[index..])
    } else {
        format!("{script}{html}")
    };
    Ok(injected.into_bytes())
}

pub(super) fn local_app_ui_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

pub(super) fn local_app_ui_error(status: StatusCode, message: &str) -> AxumResponse {
    local_app_ui_response(
        status,
        "text/plain; charset=utf-8",
        message.as_bytes().to_vec(),
    )
}

pub(super) fn local_app_ui_response(
    status: StatusCode,
    content_type: &'static str,
    body: Vec<u8>,
) -> AxumResponse {
    HttpResponse::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'none'; object-src 'none'; base-uri 'self'; form-action 'none'; frame-src 'none'; frame-ancestors tauri://localhost http://tauri.localhost http://localhost:1420",
        )
        .body(Body::from(body))
        .unwrap_or_else(|_| HttpResponse::new(Body::empty()))
}
