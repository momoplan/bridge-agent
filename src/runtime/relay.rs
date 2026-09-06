async fn push_log_entry(
    inner: &RuntimeInner,
    limit: usize,
    level: &str,
    message: &str,
    metadata: LogMetadata,
) {
    emit_tracing(level, message);
    let mut logs = inner.logs.lock().await;
    let entry = LogEntry {
        sequence: inner.next_log_sequence.fetch_add(1, Ordering::Relaxed) + 1,
        timestamp_ms: now_ms(),
        level: level.to_string(),
        message: message.to_string(),
        metadata,
    };
    logs.push_back(entry.clone());
    while logs.len() > limit {
        logs.pop_front();
    }
    let file_log = inner.file_log.lock().await.clone();
    if let Some(file_log) = file_log {
        if let Err(err) = file_log.append(&entry) {
            warn!("failed to append file log: {err:#}");
        }
    }
    drop(logs);
    publish_runtime_event(inner, RuntimeEvent::LogAppended(entry));
}

fn publish_runtime_event(inner: &RuntimeInner, event: RuntimeEvent) {
    let _ = inner.events.send(event);
}

async fn write_json<S>(sink: &mut S, message: &AgentMessage) -> Result<()>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let payload = serde_json::to_string(message)?;
    sink.send(Message::Text(payload.into())).await?;
    Ok(())
}

fn decode_relay_message(text: &str) -> Result<Option<AgentMessage>> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    let Some(message_type) = value.get("type").and_then(serde_json::Value::as_str) else {
        let message = serde_json::from_value(value)?;
        return Ok(Some(message));
    };

    if !relay_message_type_is_supported(message_type) {
        return Ok(None);
    }

    let message = serde_json::from_value(value)?;
    Ok(Some(message))
}

fn relay_message_type_is_supported(message_type: &str) -> bool {
    matches!(
        message_type,
        "capabilities"
            | "registered_ack"
            | "invoke_request"
            | "invoke_result"
            | "local_app_invoke_request"
            | "local_app_invoke_result"
            | "local_app_event_emitted"
            | "event_ack"
            | "error"
    )
}

fn build_agent_url(base: &str, agent_id: &str, token: &str) -> Result<Url> {
    let base = base.trim_end_matches('/');
    let mut url = Url::parse(&format!("{base}/{agent_id}"))?;
    url.query_pairs_mut().append_pair("token", token);
    Ok(url)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn emit_tracing(level: &str, message: &str) {
    match level {
        "error" => error!("{message}"),
        "warn" => warn!("{message}"),
        _ => info!("{message}"),
    }
}
