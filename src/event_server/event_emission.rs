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
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let event = LocalAppEventEmitted {
        event_id: event_id.clone(),
        app_id: app_id.to_string(),
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
            "local app event {app_id}.{event_name} forwarded; matched {} subscription(s)",
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
            app_id: app_id.to_string(),
            event: event_name.to_string(),
        }),
    ))
}
