use crate::config::{load_config, resolve_config_base_dir, AgentConfig};
use crate::event_server::{LocalEventEmitRequest, LocalEventServer};
use crate::logging::{FileLogConfig, FileLogSink};
use crate::protocol::{AgentCapabilities, AgentMessage, EventEmitted};
use crate::services::ServiceRegistry;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{interval_at, sleep, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{error, info, warn};
use url::Url;

const RELAY_KEEPALIVE_INTERVAL_SECS: u64 = 25;
const LOCAL_EVENT_QUEUE_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Stopped,
    Starting,
    Connecting,
    Online,
    Backoff,
    Stopping,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSnapshot {
    pub status: RuntimeStatus,
    pub config_path: Option<String>,
    pub agent_id: Option<String>,
    pub relay_url: Option<String>,
    pub log_file_path: Option<String>,
    pub last_error: Option<String>,
    pub last_event_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub timestamp_ms: u64,
    pub level: String,
    pub message: String,
}

#[derive(Clone, Default)]
pub struct AgentRuntimeManager {
    inner: Arc<RuntimeInner>,
}

#[derive(Default)]
struct RuntimeInner {
    state: Mutex<ManagedState>,
    logs: Mutex<VecDeque<LogEntry>>,
    file_log: Mutex<Option<FileLogSink>>,
}

struct ManagedState {
    snapshot: RuntimeSnapshot,
    task: Option<JoinHandle<()>>,
    shutdown: Option<watch::Sender<bool>>,
    apply: Option<mpsc::UnboundedSender<RuntimeRegistryUpdate>>,
}

struct RuntimeRegistryUpdate {
    registry: ServiceRegistry,
    services: Vec<crate::protocol::ServiceDefinition>,
}

impl Default for ManagedState {
    fn default() -> Self {
        Self {
            snapshot: RuntimeSnapshot {
                status: RuntimeStatus::Stopped,
                config_path: None,
                agent_id: None,
                relay_url: None,
                log_file_path: None,
                last_error: None,
                last_event_at: now_ms(),
            },
            task: None,
            shutdown: None,
            apply: None,
        }
    }
}

impl AgentRuntimeManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start_from_path(&self, path: &Path) -> Result<RuntimeSnapshot> {
        let config = load_config(path)?;
        self.start(config, path).await
    }

    pub async fn start(&self, config: AgentConfig, config_path: &Path) -> Result<RuntimeSnapshot> {
        self.stop_if_running().await?;
        let log_limit = config.runtime.log_limit;
        let config_base_dir = resolve_config_base_dir(config_path);
        let registry = Arc::new(RwLock::new(ServiceRegistry::from_config(
            &config,
            &config_base_dir,
        )?));
        let file_log = FileLogSink::from_config(
            &FileLogConfig {
                enabled: config.runtime.log_file_enabled,
                dir: config.runtime.log_file_dir.as_ref().map(PathBuf::from),
                max_bytes: config.runtime.log_file_max_bytes,
                max_files: config.runtime.log_file_max_files,
            },
            &config_base_dir,
        )?;
        let log_file_path = file_log
            .as_ref()
            .map(|sink| sink.path().display().to_string());
        let ws_url = build_agent_url(
            &config.relay.url,
            &config.relay.agent_id,
            &config.relay.token,
        )?;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (apply_tx, mut apply_rx) = mpsc::unbounded_channel();
        let (event_tx, mut event_rx) = mpsc::channel(LOCAL_EVENT_QUEUE_CAPACITY);
        let event_server =
            LocalEventServer::bind(&config.runtime, Arc::clone(&registry), event_tx).await?;
        let snapshot = RuntimeSnapshot {
            status: RuntimeStatus::Starting,
            config_path: Some(config_path.display().to_string()),
            agent_id: Some(config.relay.agent_id.clone()),
            relay_url: Some(ws_url.to_string()),
            log_file_path: log_file_path.clone(),
            last_error: None,
            last_event_at: now_ms(),
        };

        {
            let mut state = self.inner.state.lock().await;
            state.snapshot = snapshot;
        }
        {
            let mut active_file_log = self.inner.file_log.lock().await;
            *active_file_log = file_log;
        }
        self.push_log("info", "runtime starting", log_limit).await;

        let inner = Arc::clone(&self.inner);
        let config_path_string = config_path.display().to_string();
        let task = tokio::spawn(async move {
            let event_server_task = event_server.map(|server| {
                let bind_addr = server.bind_addr();
                let server_inner = Arc::clone(&inner);
                let server_shutdown_rx = shutdown_rx.clone();
                tokio::spawn(async move {
                    push_log_entry(
                        &server_inner,
                        log_limit,
                        "info",
                        &format!("local event server listening on {bind_addr}"),
                    )
                    .await;
                    if let Err(err) = server.serve(server_shutdown_rx).await {
                        push_log_entry(
                            &server_inner,
                            log_limit,
                            "error",
                            &format!("local event server stopped: {err:#}"),
                        )
                        .await;
                    }
                })
            });
            let runner = RuntimeRunner {
                inner: Arc::clone(&inner),
                log_limit,
                config,
                config_path: config_path_string,
                ws_url,
                registry,
            };
            if let Err(err) = runner.run(shutdown_rx, &mut apply_rx, &mut event_rx).await {
                runner
                    .update_snapshot(
                        RuntimeStatus::Stopped,
                        Some(err.to_string()),
                        runner.config.relay.agent_id.clone(),
                        runner.ws_url.to_string(),
                        runner.config_path.clone(),
                    )
                    .await;
                runner
                    .push_log("error", &format!("runtime stopped with error: {err:#}"))
                    .await;
            }
            if let Some(event_server_task) = event_server_task {
                if !event_server_task.is_finished() {
                    event_server_task.abort();
                }
                let _ = event_server_task.await;
            }
        });

        let mut state = self.inner.state.lock().await;
        state.shutdown = Some(shutdown_tx);
        state.apply = Some(apply_tx);
        state.task = Some(task);
        Ok(state.snapshot.clone())
    }

    pub async fn apply_capabilities_from_path(&self, path: &Path) -> Result<RuntimeSnapshot> {
        let config = load_config(path)?;
        let config_base_dir = resolve_config_base_dir(path);
        let registry = ServiceRegistry::from_config(&config, &config_base_dir)?;
        let services = registry.definitions();
        let update = RuntimeRegistryUpdate { registry, services };

        let snapshot = {
            let mut state = self.inner.state.lock().await;
            if state.snapshot.status == RuntimeStatus::Stopped {
                return Ok(state.snapshot.clone());
            }
            let apply = state
                .apply
                .as_ref()
                .context("runtime is running but cannot accept config updates")?
                .clone();
            apply
                .send(update)
                .context("failed to send runtime config update")?;
            state.snapshot.last_event_at = now_ms();
            state.snapshot.clone()
        };

        self.push_log(
            "info",
            "runtime capabilities update scheduled",
            config.runtime.log_limit,
        )
        .await;
        Ok(snapshot)
    }

    pub async fn stop(&self) -> Result<RuntimeSnapshot> {
        self.stop_if_running().await?;
        Ok(self.snapshot().await)
    }

    pub async fn snapshot(&self) -> RuntimeSnapshot {
        self.inner.state.lock().await.snapshot.clone()
    }

    pub async fn logs(&self, limit: usize) -> Vec<LogEntry> {
        let logs = self.inner.logs.lock().await;
        logs.iter()
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub async fn clear_logs(&self) {
        self.inner.logs.lock().await.clear();
        let file_log = self.inner.file_log.lock().await.clone();
        if let Some(file_log) = file_log {
            if let Err(err) = file_log.clear() {
                warn!("failed to clear file log: {err:#}");
            }
        }
    }

    async fn stop_if_running(&self) -> Result<()> {
        let (shutdown, task, apply) = {
            let mut state = self.inner.state.lock().await;
            if state.snapshot.status == RuntimeStatus::Stopped {
                return Ok(());
            }
            state.snapshot.status = RuntimeStatus::Stopping;
            state.snapshot.last_event_at = now_ms();
            (state.shutdown.take(), state.task.take(), state.apply.take())
        };

        if let Some(shutdown) = shutdown {
            let _ = shutdown.send(true);
        }
        drop(apply);
        if let Some(task) = task {
            let _ = task.await;
        }
        Ok(())
    }

    async fn push_log(&self, level: &str, message: &str, limit: usize) {
        push_log_entry(&self.inner, limit, level, message).await;
    }
}

struct RuntimeRunner {
    inner: Arc<RuntimeInner>,
    log_limit: usize,
    config: AgentConfig,
    config_path: String,
    ws_url: Url,
    registry: Arc<RwLock<ServiceRegistry>>,
}

impl RuntimeRunner {
    async fn run(
        &self,
        mut shutdown_rx: watch::Receiver<bool>,
        apply_rx: &mut mpsc::UnboundedReceiver<RuntimeRegistryUpdate>,
        event_rx: &mut mpsc::Receiver<LocalEventEmitRequest>,
    ) -> Result<()> {
        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            self.update_snapshot(
                RuntimeStatus::Connecting,
                None,
                self.config.relay.agent_id.clone(),
                self.ws_url.to_string(),
                self.config_path.clone(),
            )
            .await;
            self.push_log("info", &format!("connecting to {}", self.config.relay.url))
                .await;

            match connect_async(self.ws_url.as_str()).await {
                Ok((stream, _)) => {
                    self.update_snapshot(
                        RuntimeStatus::Online,
                        None,
                        self.config.relay.agent_id.clone(),
                        self.ws_url.to_string(),
                        self.config_path.clone(),
                    )
                    .await;
                    self.push_log("info", "connected to relay").await;

                    if let Err(err) = self
                        .handle_connection(stream, &mut shutdown_rx, apply_rx, event_rx)
                        .await
                    {
                        self.update_snapshot(
                            RuntimeStatus::Backoff,
                            Some(err.to_string()),
                            self.config.relay.agent_id.clone(),
                            self.ws_url.to_string(),
                            self.config_path.clone(),
                        )
                        .await;
                        self.push_log("warn", &format!("connection ended: {err:#}"))
                            .await;
                    }
                }
                Err(err) => {
                    self.update_snapshot(
                        RuntimeStatus::Backoff,
                        Some(err.to_string()),
                        self.config.relay.agent_id.clone(),
                        self.ws_url.to_string(),
                        self.config_path.clone(),
                    )
                    .await;
                    self.push_log("warn", &format!("connect failed: {err}"))
                        .await;
                }
            }

            tokio::select! {
                _ = shutdown_rx.changed() => {
                    break;
                }
                Some(update) = apply_rx.recv() => {
                    self.apply_registry_update(update).await;
                }
                _ = sleep(Duration::from_secs(self.config.relay.reconnect_secs)) => {}
            }
        }

        self.update_snapshot(
            RuntimeStatus::Stopped,
            None,
            self.config.relay.agent_id.clone(),
            self.ws_url.to_string(),
            self.config_path.clone(),
        )
        .await;
        self.push_log("info", "runtime stopped").await;
        Ok(())
    }

    async fn handle_connection(
        &self,
        stream: tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        shutdown_rx: &mut watch::Receiver<bool>,
        apply_rx: &mut mpsc::UnboundedReceiver<RuntimeRegistryUpdate>,
        event_rx: &mut mpsc::Receiver<LocalEventEmitRequest>,
    ) -> Result<()> {
        let (mut write, mut read) = stream.split();
        let capabilities = self.current_capabilities().await;
        write_json(&mut write, &capabilities).await?;
        let keepalive_interval = Duration::from_secs(RELAY_KEEPALIVE_INTERVAL_SECS);
        let mut keepalive = interval_at(
            tokio::time::Instant::now() + keepalive_interval,
            keepalive_interval,
        );

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    write.send(Message::Close(None)).await.ok();
                    break;
                }
                Some(update) = apply_rx.recv() => {
                    let capabilities = self.apply_registry_update(update).await;
                    write_json(&mut write, &capabilities).await?;
                    self.push_log("info", "runtime capabilities updated and sent to relay").await;
                }
                Some(event) = event_rx.recv() => {
                    let service = event.service.clone();
                    let event_name = event.event.clone();
                    let message = AgentMessage::EventEmitted(EventEmitted {
                        event_id: Some(event.event_id),
                        service: event.service,
                        event: event.event,
                        payload: event.payload,
                        occurred_at: event.occurred_at,
                    });
                    write_json(&mut write, &message).await?;
                    self.push_log("info", &format!("event {service}.{event_name} sent to relay")).await;
                }
                _ = keepalive.tick() => {
                    write.send(Message::Ping(Vec::new().into())).await?;
                }
                message = read.next() => {
                    let Some(message) = message else {
                        break;
                    };
                    match message? {
                        Message::Text(text) => {
                            let incoming: AgentMessage = serde_json::from_str(&text)
                                .with_context(|| format!("invalid relay message: {text}"))?;
                            match incoming {
                                AgentMessage::InvokeRequest(request) => {
                                    let service = request.service.clone();
                                    let method = request.method.clone();
                                    self.push_log(
                                        "info",
                                        &format!("invoke {service}.{method} started"),
                                    )
                                    .await;
                                    let result = self
                                        .registry
                                        .read()
                                        .await
                                        .invoke(
                                            request.request_id,
                                            &service,
                                            &method,
                                            request.arguments,
                                            request.timeout_secs,
                                        )
                                        .await;
                                    if result.success {
                                        self.push_log(
                                            "info",
                                            &format!(
                                                "invoke {service}.{method} succeeded in {}ms",
                                                result.duration_ms
                                            ),
                                        )
                                        .await;
                                    } else {
                                        let error = result
                                            .error
                                            .as_ref()
                                            .map(|err| {
                                                format!("{}: {}", err.code, err.message)
                                            })
                                            .unwrap_or_else(|| "unknown error".to_string());
                                        self.push_log(
                                            "warn",
                                            &format!(
                                                "invoke {service}.{method} failed in {}ms: {error}",
                                                result.duration_ms
                                            ),
                                        )
                                        .await;
                                    }
                                    let response = AgentMessage::InvokeResult(result);
                                    write_json(&mut write, &response).await?;
                                }
                                AgentMessage::Error(err) => {
                                    self.push_log("warn", &format!("relay error: {}", err.message))
                                        .await;
                                }
                                AgentMessage::Capabilities(_)
                                | AgentMessage::InvokeResult(_)
                                | AgentMessage::EventEmitted(_) => {}
                            }
                        }
                        Message::Ping(payload) => {
                            write.send(Message::Pong(payload)).await?;
                        }
                        Message::Close(_) => break,
                        Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
                    }
                }
            }
        }
        Ok(())
    }

    async fn current_capabilities(&self) -> AgentMessage {
        AgentMessage::Capabilities(AgentCapabilities {
            agent_id: self.config.relay.agent_id.clone(),
            services: self.registry.read().await.definitions(),
        })
    }

    async fn apply_registry_update(&self, update: RuntimeRegistryUpdate) -> AgentMessage {
        {
            let mut registry = self.registry.write().await;
            *registry = update.registry;
        }
        AgentMessage::Capabilities(AgentCapabilities {
            agent_id: self.config.relay.agent_id.clone(),
            services: update.services,
        })
    }

    async fn update_snapshot(
        &self,
        status: RuntimeStatus,
        last_error: Option<String>,
        agent_id: String,
        relay_url: String,
        config_path: String,
    ) {
        let mut state = self.inner.state.lock().await;
        state.snapshot = RuntimeSnapshot {
            status,
            config_path: Some(config_path),
            agent_id: Some(agent_id),
            relay_url: Some(relay_url),
            log_file_path: state.snapshot.log_file_path.clone(),
            last_error,
            last_event_at: now_ms(),
        };
    }

    async fn push_log(&self, level: &str, message: &str) {
        push_log_entry(&self.inner, self.log_limit, level, message).await;
    }
}

async fn push_log_entry(inner: &RuntimeInner, limit: usize, level: &str, message: &str) {
    emit_tracing(level, message);
    let entry = LogEntry {
        timestamp_ms: now_ms(),
        level: level.to_string(),
        message: message.to_string(),
    };
    let mut logs = inner.logs.lock().await;
    logs.push_back(entry.clone());
    while logs.len() > limit {
        logs.pop_front();
    }
    drop(logs);
    append_file_log(inner, &entry).await;
}

async fn append_file_log(inner: &RuntimeInner, entry: &LogEntry) {
    let file_log = inner.file_log.lock().await.clone();
    if let Some(file_log) = file_log {
        if let Err(err) = file_log.append(entry.timestamp_ms, &entry.level, &entry.message) {
            warn!("failed to append file log: {err:#}");
        }
    }
}

async fn write_json<S>(sink: &mut S, message: &AgentMessage) -> Result<()>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let payload = serde_json::to_string(message)?;
    sink.send(Message::Text(payload.into())).await?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::build_agent_url;

    #[test]
    fn build_url_injects_token() {
        let url = build_agent_url("wss://relay.baijimu.com/ws/agent", "devbox", "secret").unwrap();
        assert_eq!(
            url.as_str(),
            "wss://relay.baijimu.com/ws/agent/devbox?token=secret"
        );
    }
}
