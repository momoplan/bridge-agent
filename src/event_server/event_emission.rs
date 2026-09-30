async fn emit_local_app_event(
    State(state): State<EventServerState>,
    headers: HeaderMap,
    Json(request): Json<EmitLocalAppEventRequest>,
) -> Result<(StatusCode, Json<LocalEventAccepted>), EventApiError> {
    if !state.event_enabled {
        return Err(EventApiError::new(
            StatusCode::NOT_FOUND,
            "local event API is disabled",
        ));
    }
    let queue = state.event_queue.as_ref().ok_or_else(|| EventApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "device authorization required before event handoff",
    ))?;
    let app_id = request.app_id.trim();
    let event_name = request.event.trim();
    if app_id.is_empty() || event_name.is_empty() {
        return Err(EventApiError::new(
            StatusCode::BAD_REQUEST,
            "appId and event are required",
        ));
    }
    let token = bearer_token(&headers).ok_or_else(|| {
        EventApiError::new(
            StatusCode::UNAUTHORIZED,
            "connector event credential is required",
        )
    })?;
    authorize_connector_event(&state.config_path, app_id, event_name, &token)
        .map_err(|err| EventApiError::new(StatusCode::FORBIDDEN, err.to_string()))?;
    let event_id = request
        .event_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| EventApiError::new(StatusCode::BAD_REQUEST, "stable eventId is required"))?;
    let event = LocalAppEventEmitted {
        event_id: event_id.clone(),
        app_id: app_id.to_string(),
        event: event_name.to_string(),
        payload: AppPayload(request.payload),
        target_consumers: vec![],
        occurred_at: request
            .occurred_at
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
    };
    let policy = crate::event_delivery::read_policy(&state.config_path).await
        .map_err(|err| EventApiError::new(StatusCode::SERVICE_UNAVAILABLE, err.to_string()))?;
    let receipt = queue.admit(event, policy).await
        .map_err(|err| EventApiError::new(StatusCode::SERVICE_UNAVAILABLE, err.to_string()))?;
    Ok((StatusCode::ACCEPTED, Json(receipt)))
}
