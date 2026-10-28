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
