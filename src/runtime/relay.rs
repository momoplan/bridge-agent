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

const RELAY_MESSAGE_LOG_LIMIT: usize = 400;

impl RelayConnection<'_> {
    async fn handle_text_frame(&mut self, text: &str) -> Result<()> {
        self.mark_relay_seen().await;
        match decode_relay_message(text) {
            Ok(Some(incoming)) => self.handle_agent_message(incoming).await?,
            Ok(None) => {
                self.runner
                    .push_log("warn", "ignored unsupported relay message type")
                    .await;
            }
            Err(_) => {
                let preview = redact_relay_message_preview(text);
                self.runner
                    .push_log(
                        "warn",
                        &format!("ignored malformed relay message: {preview}"),
                    )
                    .await;
                if let Some(reply) = invalid_request_reply(text) {
                    write_json(&mut self.write, &reply).await?;
                }
            }
        }
        Ok(())
    }
}

fn redact_relay_message_preview(text: &str) -> String {
    let redacted = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(mut value) => {
            redact_json_value(&mut value);
            serde_json::to_string(&value).unwrap_or_else(|_| truncate_for_log(text))
        }
        Err(_) => truncate_for_log(text),
    };
    truncate_for_log(&redacted)
}

fn truncate_for_log(text: &str) -> String {
    if text.len() <= RELAY_MESSAGE_LOG_LIMIT {
        return text.to_string();
    }
    let mut end = RELAY_MESSAGE_LOG_LIMIT;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn redact_json_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if json_field_is_sensitive(key) {
                    *child = serde_json::Value::String("<redacted>".to_string());
                } else {
                    redact_json_value(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json_value(item);
            }
        }
        _ => {}
    }
}

fn json_field_is_sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "arguments"
            | "argument"
            | "args"
            | "stdin"
            | "command"
            | "token"
            | "password"
            | "secret"
            | "authorization"
    ) || key.contains("token")
        || key.contains("secret")
        || key.contains("password")
        || key.contains("authorization")
}

fn invalid_request_reply(text: &str) -> Option<AgentMessage> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let request_id = value
        .get("request_id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("requestId").and_then(serde_json::Value::as_str))
        .filter(|request_id| !request_id.is_empty())?;
    let result = InvokeResult {
        request_id: request_id.to_string(),
        success: false,
        data: None,
        error: Some(InvokeError {
            public_details: None,
            code: "INVALID_REQUEST".to_string(),
            message: "relay message could not be parsed".to_string(),
        }),
        duration_ms: 0,
    };
    if value.get("type").and_then(serde_json::Value::as_str) == Some("local_app_invoke_request") {
        Some(AgentMessage::LocalAppInvokeResult(result))
    } else {
        Some(AgentMessage::InvokeResult(result))
    }
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
            | "device_subscriptions"
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
