use crate::config::{load_config, resolve_config_base_dir, AgentConfig};
use crate::event_server::{LocalEventEmitRequest, LocalEventServer};
use crate::logging::{FileLogConfig, FileLogSink};
use crate::protocol::{AgentCapabilities, AgentMessage, EventEmitted};
use crate::services::ServiceRegistry;
use anyhow::{bail, Context, Result};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::io::{ErrorKind, Write};
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
const RUNTIME_LOCK_DIR: &str = ".bridge-agent-locks";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: Option<String>,
    pub executable_path: Option<String>,
    pub command_line: Option<String>,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLockConflict {
    pub pid: u32,
    pub agent_id: String,
    pub config_path: String,
    pub lock_path: String,
    pub process: RuntimeProcessInfo,
}

impl std::fmt::Display for RuntimeLockConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "bridge-agent runtime is already running for agent `{}` with config `{}` (pid {}, lock {})",
            self.agent_id, self.config_path, self.pid, self.lock_path
        )
    }
}

impl std::error::Error for RuntimeLockConflict {}

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

pub(crate) struct RuntimeRegistryUpdate {
    pub(crate) registry: ServiceRegistry,
    pub(crate) services: Vec<crate::protocol::ServiceDefinition>,
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
        let runtime_lock = RuntimeInstanceLock::acquire(config_path, &config.relay.agent_id)?;
        let registry = Arc::new(RwLock::new(
            ServiceRegistry::from_config_checked(&config, &config_base_dir).await?,
        ));
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
        let event_server = LocalEventServer::bind(
            &config.runtime,
            config_path.to_path_buf(),
            Arc::clone(&registry),
            event_tx,
            apply_tx.clone(),
        )
        .await?;
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
            let _runtime_lock = runtime_lock;
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
        let registry = ServiceRegistry::from_config_checked(&config, &config_base_dir).await?;
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

#[derive(Debug)]
struct RuntimeInstanceLock {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeLockDocument {
    pid: u32,
    agent_id: String,
    config_path: String,
    started_at_ms: u64,
}

impl RuntimeInstanceLock {
    fn acquire(config_path: &Path, agent_id: &str) -> Result<Self> {
        let lock_path = runtime_lock_path(config_path, agent_id);
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create runtime lock dir {}", parent.display())
            })?;
        }

        for _ in 0..3 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    let document = RuntimeLockDocument {
                        pid: std::process::id(),
                        agent_id: agent_id.to_string(),
                        config_path: config_path.display().to_string(),
                        started_at_ms: now_ms(),
                    };
                    let content = serde_json::to_vec_pretty(&document)?;
                    file.write_all(&content).with_context(|| {
                        format!("failed to write runtime lock {}", lock_path.display())
                    })?;
                    file.write_all(b"\n").with_context(|| {
                        format!("failed to write runtime lock {}", lock_path.display())
                    })?;
                    file.sync_all().with_context(|| {
                        format!("failed to flush runtime lock {}", lock_path.display())
                    })?;
                    return Ok(Self { path: lock_path });
                }
                Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                    if remove_stale_runtime_lock(&lock_path)? {
                        continue;
                    }
                    if let Ok(existing) = read_runtime_lock(&lock_path) {
                        return Err(RuntimeLockConflict {
                            pid: existing.pid,
                            agent_id: existing.agent_id,
                            config_path: existing.config_path,
                            lock_path: lock_path.display().to_string(),
                            process: describe_process(existing.pid),
                        }
                        .into());
                    }
                    anyhow::bail!(
                        "bridge-agent runtime lock already exists at {}",
                        lock_path.display()
                    );
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("failed to create runtime lock {}", lock_path.display())
                    });
                }
            }
        }

        anyhow::bail!(
            "failed to acquire runtime lock after removing stale lock {}",
            lock_path.display()
        )
    }
}

impl Drop for RuntimeInstanceLock {
    fn drop(&mut self) {
        if let Err(err) = fs::remove_file(&self.path) {
            if err.kind() != ErrorKind::NotFound {
                warn!(
                    "failed to remove runtime lock {}: {err:#}",
                    self.path.display()
                );
            }
        }
    }
}

fn runtime_lock_path(config_path: &Path, agent_id: &str) -> PathBuf {
    let config_base_dir = resolve_config_base_dir(config_path);
    let config_file = config_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("agent-config.json");
    let fingerprint =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(config_path.display().to_string());
    let fingerprint = &fingerprint[..fingerprint.len().min(16)];
    let name = format!(
        "{}-{}-{fingerprint}.lock",
        sanitize_lock_component(config_file),
        sanitize_lock_component(agent_id)
    );
    config_base_dir.join(RUNTIME_LOCK_DIR).join(name)
}

fn sanitize_lock_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim_matches('_');
    if sanitized.is_empty() {
        "runtime".to_string()
    } else {
        sanitized.chars().take(48).collect()
    }
}

fn remove_stale_runtime_lock(path: &Path) -> Result<bool> {
    let Some(document) = read_runtime_lock(path).ok() else {
        fs::remove_file(path).with_context(|| {
            format!(
                "failed to remove unreadable runtime lock {}",
                path.display()
            )
        })?;
        return Ok(true);
    };
    if process_is_running(document.pid) {
        return Ok(false);
    }
    fs::remove_file(path).with_context(|| {
        format!(
            "failed to remove stale runtime lock {} for pid {}",
            path.display(),
            document.pid
        )
    })?;
    Ok(true)
}

fn read_runtime_lock(path: &Path) -> Result<RuntimeLockDocument> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read runtime lock {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse runtime lock {}", path.display()))
}

pub fn terminate_runtime_lock_owner(
    lock_path: &Path,
    expected_pid: u32,
    expected_agent_id: &str,
    expected_config_path: &str,
) -> Result<()> {
    let document = read_runtime_lock(lock_path)?;
    if document.pid != expected_pid
        || document.agent_id != expected_agent_id
        || document.config_path != expected_config_path
    {
        bail!("runtime lock changed; please retry with the latest conflict information");
    }

    let process = describe_process(document.pid);
    if process.running {
        if !process_looks_like_bridge_agent(&process) {
            bail!(
                "pid {} is running but does not look like a Bridge Agent process",
                document.pid
            );
        }
        terminate_process_tree(document.pid)?;
        wait_for_process_exit(document.pid, Duration::from_secs(5))?;
    }

    match fs::remove_file(lock_path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| {
            format!(
                "failed to remove runtime lock after terminating owner {}",
                lock_path.display()
            )
        }),
    }
}

fn describe_process(pid: u32) -> RuntimeProcessInfo {
    #[cfg(windows)]
    {
        return describe_process_windows(pid);
    }

    #[cfg(unix)]
    {
        return describe_process_unix(pid);
    }

    #[cfg(not(any(unix, windows)))]
    {
        RuntimeProcessInfo {
            pid,
            parent_pid: None,
            name: None,
            executable_path: None,
            command_line: None,
            running: process_is_running(pid),
        }
    }
}

#[cfg(windows)]
fn describe_process_windows(pid: u32) -> RuntimeProcessInfo {
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct WindowsProcessInfo {
        process_id: u32,
        parent_process_id: Option<u32>,
        name: Option<String>,
        executable_path: Option<String>,
        command_line: Option<String>,
    }

    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"ProcessId = {pid}\" | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath,CommandLine | ConvertTo-Json -Compress"
    );
    if let Ok(output) = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Ok(info) = serde_json::from_str::<WindowsProcessInfo>(stdout.trim()) {
                return RuntimeProcessInfo {
                    pid: info.process_id,
                    parent_pid: info.parent_process_id,
                    name: info.name,
                    executable_path: info.executable_path,
                    command_line: info.command_line,
                    running: true,
                };
            }
        }
    }

    RuntimeProcessInfo {
        pid,
        parent_pid: None,
        name: None,
        executable_path: None,
        command_line: None,
        running: process_is_running(pid),
    }
}

#[cfg(unix)]
fn describe_process_unix(pid: u32) -> RuntimeProcessInfo {
    if let Ok(output) = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "ppid=", "-o", "comm=", "-o", "args="])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().map(str::trim).find(|line| !line.is_empty()) {
                let mut parts = line.splitn(3, char::is_whitespace);
                let parent_pid = parts.next().and_then(|value| value.trim().parse().ok());
                let name = parts.next().map(str::trim).filter(|value| !value.is_empty());
                let command_line = parts.next().map(str::trim).filter(|value| !value.is_empty());
                return RuntimeProcessInfo {
                    pid,
                    parent_pid,
                    name: name.map(ToOwned::to_owned),
                    executable_path: None,
                    command_line: command_line.map(ToOwned::to_owned),
                    running: true,
                };
            }
        }
    }

    RuntimeProcessInfo {
        pid,
        parent_pid: None,
        name: None,
        executable_path: None,
        command_line: None,
        running: process_is_running(pid),
    }
}

fn process_looks_like_bridge_agent(process: &RuntimeProcessInfo) -> bool {
    [
        process.name.as_deref(),
        process.executable_path.as_deref(),
        process.command_line.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_ascii_lowercase().contains("bridge-agent"))
}

#[cfg(windows)]
fn terminate_process_tree(pid: u32) -> Result<()> {
    let status = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .with_context(|| format!("failed to run taskkill for pid {pid}"))?;
    if status.success() || !process_is_running(pid) {
        return Ok(());
    }
    bail!("failed to terminate runtime owner pid {pid}");
}

#[cfg(unix)]
fn terminate_process_tree(pid: u32) -> Result<()> {
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    if wait_for_process_exit(pid, Duration::from_secs(2)).is_ok() {
        return Ok(());
    }
    let status = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status()
        .with_context(|| format!("failed to run kill for pid {pid}"))?;
    if status.success() || !process_is_running(pid) {
        return Ok(());
    }
    bail!("failed to terminate runtime owner pid {pid}");
}

#[cfg(not(any(unix, windows)))]
fn terminate_process_tree(pid: u32) -> Result<()> {
    bail!("terminating pid {pid} is not supported on this platform");
}

fn wait_for_process_exit(pid: u32, timeout: Duration) -> Result<()> {
    let started = SystemTime::now();
    while process_is_running(pid) {
        if started.elapsed().unwrap_or_default() >= timeout {
            bail!("runtime owner pid {pid} is still running");
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Ok(())
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let filter = format!("PID eq {pid}");
    let Ok(output) = std::process::Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output()
    else {
        return true;
    };
    if !output.status.success() {
        return true;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .map(str::trim)
        .any(|line| !line.is_empty() && !line.starts_with("INFO:"))
}

#[cfg(not(any(unix, windows)))]
fn process_is_running(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{
        build_agent_url, read_runtime_lock, runtime_lock_path, RuntimeInstanceLock,
        RuntimeLockDocument,
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn build_url_injects_token() {
        let url = build_agent_url("wss://relay.baijimu.com/ws/agent", "devbox", "secret").unwrap();
        assert_eq!(
            url.as_str(),
            "wss://relay.baijimu.com/ws/agent/devbox?token=secret"
        );
    }

    #[test]
    fn runtime_lock_rejects_second_owner_until_released() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");

        let first = RuntimeInstanceLock::acquire(&config_path, "dev_1").unwrap();
        let second = RuntimeInstanceLock::acquire(&config_path, "dev_1");
        assert!(second.is_err());

        drop(first);
        RuntimeInstanceLock::acquire(&config_path, "dev_1").unwrap();
    }

    #[test]
    fn runtime_lock_removes_stale_owner() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let lock_path = runtime_lock_path(&config_path, "dev_1");
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        let stale = RuntimeLockDocument {
            pid: u32::MAX,
            agent_id: "dev_1".to_string(),
            config_path: config_path.display().to_string(),
            started_at_ms: 1,
        };
        fs::write(&lock_path, serde_json::to_string_pretty(&stale).unwrap()).unwrap();

        let lock = RuntimeInstanceLock::acquire(&config_path, "dev_1").unwrap();
        let active = read_runtime_lock(&lock.path).unwrap();
        assert_eq!(active.pid, std::process::id());
    }
}
