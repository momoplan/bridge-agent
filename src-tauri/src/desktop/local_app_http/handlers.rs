pub(super) async fn local_app_control_status_handler(
    AxumState(state): AxumState<LocalAppControlHttpState>,
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
    AxumState(state): AxumState<LocalAppControlHttpState>,
    headers: HeaderMap,
) -> AxumResponse {
    if !local_app_control_is_authorized(&state, &headers) {
        return local_app_control_error(StatusCode::UNAUTHORIZED, "本机应用控制凭证无效");
    }
    local_app_control_result(fetch_market_connector_apps(&state.config_path).await)
}

pub(super) async fn local_app_control_list_handler(
    AxumState(state): AxumState<LocalAppControlHttpState>,
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
    AxumState(state): AxumState<LocalAppControlHttpState>,
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
    AxumState(state): AxumState<LocalAppControlHttpState>,
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
                install_source: request.install_source,
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
    AxumState(state): AxumState<LocalAppControlHttpState>,
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
    AxumState(state): AxumState<LocalAppControlHttpState>,
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
    AxumState(state): AxumState<LocalAppControlHttpState>,
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
                install_source: record.install_source,
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
    AxumState(state): AxumState<LocalAppControlHttpState>,
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
    AxumState(state): AxumState<LocalAppControlHttpState>,
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
