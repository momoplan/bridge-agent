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
