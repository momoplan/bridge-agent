// Live local-app output streams. Relay attaches a consumer; Bridge pulls the
// app's own SSE stream only while the attachment exists and forwards each
// entry on the current relay connection. Nothing is stored or retried here:
// resume and gap handling belong to the app cursor carried in `after`.

const STREAM_FORWARD_BUFFER: usize = 256;
const STREAM_CONNECT_TIMEOUT_SECS: u64 = 5;
const STREAM_MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;
const APP_STREAM_OUTPUT_EVENT: &str = "notification";
const APP_STREAM_RESYNC_EVENT: &str = "resync";

/// Pumps owned by one relay connection; dropping it cancels every pump.
struct StreamPumps {
    forward: mpsc::Sender<AgentMessage>,
    frames: mpsc::Receiver<AgentMessage>,
    running: HashMap<String, JoinHandle<()>>,
    client: Option<reqwest::Client>,
}

impl StreamPumps {
    fn new() -> Self {
        let (forward, frames) = mpsc::channel(STREAM_FORWARD_BUFFER);
        Self {
            forward,
            frames,
            running: HashMap::new(),
            client: None,
        }
    }

    fn client(&mut self) -> Result<reqwest::Client> {
        if let Some(client) = &self.client {
            return Ok(client.clone());
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(STREAM_CONNECT_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("build local app stream client")?;
        self.client = Some(client.clone());
        Ok(client)
    }

    /// Starts a pump, or reports why the attachment cannot be served.
    fn attach(&mut self, attach: StreamAttach, target: Option<LocalAppStreamTarget>) -> Option<AgentMessage> {
        self.detach(&attach.attach_id);
        let Some(target) = target else {
            return Some(stream_closed(
                &attach.attach_id,
                StreamCloseCode::StreamUnsupported,
                format!("local app `{}` does not declare a notification stream", attach.app_id),
            ));
        };
        let client = match self.client() {
            Ok(client) => client,
            Err(error) => {
                return Some(stream_closed(
                    &attach.attach_id,
                    StreamCloseCode::AppUnavailable,
                    format!("{error:#}"),
                ))
            }
        };
        let forward = self.forward.clone();
        let attach_id = attach.attach_id.clone();
        let handle = tokio::spawn(async move {
            let attach_id = attach.attach_id.clone();
            let closed = pump_local_app_stream(&client, &target, &attach, &forward).await;
            let _ = forward.send(closed.into_message(&attach_id)).await;
        });
        self.running.insert(attach_id, handle);
        None
    }

    fn detach(&mut self, attach_id: &str) {
        if let Some(handle) = self.running.remove(attach_id) {
            handle.abort();
        }
    }

    /// Called for every forwarded frame; a closed attachment no longer runs.
    fn forwarded(&mut self, message: &AgentMessage) {
        if let AgentMessage::StreamClosed(closed) = message {
            self.running.remove(&closed.attach_id);
        }
    }
}

impl Drop for StreamPumps {
    fn drop(&mut self) {
        for (_, handle) in self.running.drain() {
            handle.abort();
        }
    }
}

struct PumpEnd {
    code: StreamCloseCode,
    message: String,
}

impl PumpEnd {
    fn new(code: StreamCloseCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn into_message(self, attach_id: &str) -> AgentMessage {
        stream_closed(attach_id, self.code, self.message)
    }
}

fn stream_closed(attach_id: &str, code: StreamCloseCode, message: String) -> AgentMessage {
    AgentMessage::StreamClosed(StreamClosed {
        attach_id: attach_id.to_string(),
        code,
        message,
    })
}

async fn pump_local_app_stream(
    client: &reqwest::Client,
    target: &LocalAppStreamTarget,
    attach: &StreamAttach,
    forward: &mpsc::Sender<AgentMessage>,
) -> PumpEnd {
    let mut request = client
        .get(&target.url)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .header(
            crate::services::BAIJIMU_WORKSPACE_ID_HEADER,
            attach.workspace_id.to_string(),
        );
    for (name, value) in &target.headers {
        request = request.header(name, value);
    }
    if let Some(after) = &attach.after {
        request = request.header("Last-Event-ID", after);
    }
    let mut response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return PumpEnd::new(
                StreamCloseCode::AppUnavailable,
                format!("local app stream unavailable: {error}"),
            )
        }
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let code = if status == reqwest::StatusCode::BAD_REQUEST
            && body.contains("INVALID_STREAM_CURSOR")
        {
            StreamCloseCode::InvalidStreamCursor
        } else {
            StreamCloseCode::AppUnavailable
        };
        return PumpEnd::new(code, format!("local app stream rejected with HTTP {status}"));
    }
    let mut parser = SseParser::default();
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => return PumpEnd::new(StreamCloseCode::StreamEnded, "local app ended the stream"),
            Err(error) => {
                return PumpEnd::new(
                    StreamCloseCode::AppUnavailable,
                    format!("local app stream interrupted: {error}"),
                )
            }
        };
        let events = match parser.push(&chunk) {
            Ok(events) => events,
            Err(error) => return PumpEnd::new(StreamCloseCode::AppUnavailable, error),
        };
        for event in events {
            let message = match stream_message(&attach.attach_id, event) {
                Ok(Some(message)) => message,
                Ok(None) => continue,
                Err(error) => return PumpEnd::new(StreamCloseCode::AppUnavailable, error),
            };
            if forward.send(message).await.is_err() {
                return PumpEnd::new(StreamCloseCode::StreamEnded, "relay connection ended");
            }
        }
    }
}

fn stream_message(attach_id: &str, event: SseEvent) -> std::result::Result<Option<AgentMessage>, String> {
    match event.event.as_deref() {
        Some(APP_STREAM_OUTPUT_EVENT) => {
            let id = event.id.ok_or("local app stream entry has no id")?;
            let data = serde_json::from_str(&event.data)
                .map_err(|error| format!("local app stream entry is not JSON: {error}"))?;
            Ok(Some(AgentMessage::StreamOutput(StreamOutput {
                attach_id: attach_id.to_string(),
                id,
                data,
            })))
        }
        Some(APP_STREAM_RESYNC_EVENT) => {
            let id = event.id.ok_or("local app resync has no id")?;
            Ok(Some(AgentMessage::StreamResync(StreamResync {
                attach_id: attach_id.to_string(),
                id,
            })))
        }
        _ => Ok(None),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SseEvent {
    event: Option<String>,
    id: Option<String>,
    data: String,
}

/// Minimal `text/event-stream` decoder (WHATWG field rules, no `retry`).
#[derive(Default)]
struct SseParser {
    buffer: Vec<u8>,
    event: Option<String>,
    id: Option<String>,
    data: Vec<String>,
    pending_bytes: usize,
}

impl SseParser {
    fn push(&mut self, chunk: &[u8]) -> std::result::Result<Vec<SseEvent>, String> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=end).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8(line).map_err(|_| "local app stream is not UTF-8")?;
            if let Some(event) = self.line(&line)? {
                events.push(event);
            }
        }
        if self.buffer.len() > STREAM_MAX_EVENT_BYTES {
            return Err("local app stream line is too large".into());
        }
        Ok(events)
    }

    fn line(&mut self, line: &str) -> std::result::Result<Option<SseEvent>, String> {
        if line.is_empty() {
            self.pending_bytes = 0;
            let data = std::mem::take(&mut self.data);
            let event = self.event.take();
            if data.is_empty() {
                return Ok(None);
            }
            return Ok(Some(SseEvent {
                event,
                id: self.id.clone(),
                data: data.join("\n"),
            }));
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        self.pending_bytes += value.len();
        if self.pending_bytes > STREAM_MAX_EVENT_BYTES {
            return Err("local app stream event is too large".into());
        }
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data.push(value.to_string()),
            "id" if !value.contains('\0') => self.id = Some(value.to_string()),
            _ => {}
        }
        Ok(None)
    }
}
