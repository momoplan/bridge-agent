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
