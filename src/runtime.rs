use crate::config::{
    load_config, resolve_config_base_dir, AgentConfig, ServiceConfig, ServiceStartCommand,
};
use crate::connector::sync_installed_connectors;
use crate::event_server::{LocalEventEmitRequest, LocalEventServer};
use crate::logging::{FileLogConfig, FileLogSink, LogEntry, LogMetadata};
use crate::power::SystemSleepPrevention;
use crate::process_identity::is_bridge_agent_process_name;
#[cfg(windows)]
use crate::process_identity::process_file_name;
use crate::protocol::{
    AgentCapabilities, AgentMessage, EventEmitted, AGENT_PROTOCOL_FEATURE_REGISTERED_ACK,
    AGENT_PROTOCOL_VERSION,
};
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
use tokio::process::{Child, Command as AsyncCommand};
use tokio::sync::{mpsc, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{interval_at, sleep, timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{error, info, warn};
use url::Url;

const RELAY_KEEPALIVE_INTERVAL_SECS: u64 = 25;
const RELAY_HEARTBEAT_TIMEOUT_SECS: u64 = 75;
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
    pub relay_registered: bool,
    pub relay_registered_at: Option<u64>,
    pub last_relay_seen_at: Option<u64>,
    pub log_file_path: Option<String>,
    pub last_error: Option<String>,
    pub last_event_at: u64,
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
    lifecycle: Mutex<()>,
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

pub(crate) struct RuntimeAuditLog {
    pub(crate) level: String,
    pub(crate) message: String,
    pub(crate) metadata: LogMetadata,
}

impl Default for ManagedState {
    fn default() -> Self {
        Self {
            snapshot: RuntimeSnapshot {
                status: RuntimeStatus::Stopped,
                config_path: None,
                agent_id: None,
                relay_url: None,
                relay_registered: false,
                relay_registered_at: None,
                last_relay_seen_at: None,
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

    pub async fn push_desktop_log(&self, level: &str, message: &str, metadata: LogMetadata) {
        push_log_entry(&self.inner, 500, level, message, metadata).await;
    }

    pub async fn start_from_path(&self, path: &Path) -> Result<RuntimeSnapshot> {
        sync_installed_connectors(path)?;
        let config = load_config(path)?;
        self.start(config, path).await
    }

    pub async fn start(&self, config: AgentConfig, config_path: &Path) -> Result<RuntimeSnapshot> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        if let Some(snapshot) = self.active_start_snapshot(&config, config_path).await {
            return Ok(snapshot);
        }
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
        let (audit_tx, mut audit_rx) = mpsc::unbounded_channel();
        let event_server = LocalEventServer::bind(
            &config.runtime,
            config_path.to_path_buf(),
            Arc::clone(&registry),
            event_tx,
            apply_tx.clone(),
            audit_tx,
        )
        .await?;
        let snapshot = RuntimeSnapshot {
            status: RuntimeStatus::Starting,
            config_path: Some(config_path.display().to_string()),
            agent_id: Some(config.relay.agent_id.clone()),
            relay_url: Some(ws_url.to_string()),
            relay_registered: false,
            relay_registered_at: None,
            last_relay_seen_at: None,
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
            let system_sleep_prevention = match SystemSleepPrevention::acquire("百积木保持连接在线")
            {
                Ok(assertion) => {
                    if assertion.is_active() {
                        push_log_entry(
                            &inner,
                            log_limit,
                            "info",
                            "system idle sleep prevention enabled while runtime is active",
                            LogMetadata::category("power").outcome("enabled"),
                        )
                        .await;
                    }
                    Some(assertion)
                }
                Err(err) => {
                    push_log_entry(
                        &inner,
                        log_limit,
                        "warn",
                        &format!("failed to enable system idle sleep prevention: {err:#}"),
                        LogMetadata::category("power").outcome("failed"),
                    )
                    .await;
                    None
                }
            };
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
                        LogMetadata::category("event_server").outcome("listening"),
                    )
                    .await;
                    if let Err(err) = server.serve(server_shutdown_rx).await {
                        push_log_entry(
                            &server_inner,
                            log_limit,
                            "error",
                            &format!("local event server stopped: {err:#}"),
                            LogMetadata::category("event_server").outcome("stopped"),
                        )
                        .await;
                    }
                })
            });
            let mut managed_connectors =
                ManagedConnectorProcesses::start(&config.services, &inner, log_limit).await;
            let runner = RuntimeRunner {
                inner: Arc::clone(&inner),
                log_limit,
                config,
                config_path: config_path_string,
                ws_url,
                registry,
            };
            if let Err(err) = runner
                .run(shutdown_rx, &mut apply_rx, &mut event_rx, &mut audit_rx)
                .await
            {
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
            managed_connectors.stop_all(&inner, log_limit).await;
            if let Some(event_server_task) = event_server_task {
                if !event_server_task.is_finished() {
                    event_server_task.abort();
                }
                let _ = event_server_task.await;
            }
            if system_sleep_prevention
                .as_ref()
                .is_some_and(SystemSleepPrevention::is_active)
            {
                push_log_entry(
                    &inner,
                    log_limit,
                    "info",
                    "system idle sleep prevention released",
                    LogMetadata::category("power").outcome("released"),
                )
                .await;
            }
        });

        let mut state = self.inner.state.lock().await;
        state.shutdown = Some(shutdown_tx);
        state.apply = Some(apply_tx);
        state.task = Some(task);
        Ok(state.snapshot.clone())
    }

    pub async fn apply_capabilities_from_path(&self, path: &Path) -> Result<RuntimeSnapshot> {
        sync_installed_connectors(path)?;
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
        let _lifecycle = self.inner.lifecycle.lock().await;
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

    async fn active_start_snapshot(
        &self,
        config: &AgentConfig,
        config_path: &Path,
    ) -> Option<RuntimeSnapshot> {
        let state = self.inner.state.lock().await;
        let snapshot = &state.snapshot;
        if !runtime_start_is_active(snapshot.status) {
            return None;
        }
        if snapshot.agent_id.as_deref() != Some(config.relay.agent_id.as_str()) {
            return None;
        }
        if snapshot.config_path.as_deref() != Some(&config_path.display().to_string()) {
            return None;
        }
        Some(snapshot.clone())
    }

    async fn push_log(&self, level: &str, message: &str, limit: usize) {
        push_log_entry(&self.inner, limit, level, message, LogMetadata::default()).await;
    }
}

fn runtime_start_is_active(status: RuntimeStatus) -> bool {
    matches!(
        status,
        RuntimeStatus::Starting | RuntimeStatus::Connecting | RuntimeStatus::Backoff
    )
}

struct RuntimeRunner {
    inner: Arc<RuntimeInner>,
    log_limit: usize,
    config: AgentConfig,
    config_path: String,
    ws_url: Url,
    registry: Arc<RwLock<ServiceRegistry>>,
}

struct ManagedConnectorProcesses {
    processes: Vec<ManagedConnectorProcess>,
}

struct ManagedConnectorProcess {
    service: String,
    child: Child,
}

impl ManagedConnectorProcesses {
    async fn start(
        services: &[ServiceConfig],
        inner: &Arc<RuntimeInner>,
        log_limit: usize,
    ) -> Self {
        let mut processes = Vec::new();
        for service in services {
            if !service.enabled {
                continue;
            }
            let Some(command) = service.start_command.as_ref() else {
                continue;
            };
            match spawn_managed_connector_process(service, command).await {
                Ok(Some(process)) => processes.push(process),
                Ok(None) => {}
                Err(err) => {
                    push_log_entry(
                        inner,
                        log_limit,
                        "warn",
                        &format!(
                            "failed to start managed connector `{}`: {err:#}",
                            service.name
                        ),
                        LogMetadata::category("connector")
                            .service(service.name.clone())
                            .outcome("start_failed"),
                    )
                    .await;
                }
            }
        }
        Self { processes }
    }

    async fn stop_all(&mut self, inner: &Arc<RuntimeInner>, log_limit: usize) {
        while let Some(mut process) = self.processes.pop() {
            stop_managed_connector_process(&mut process, inner, log_limit).await;
        }
    }
}

async fn spawn_managed_connector_process(
    service: &ServiceConfig,
    start_command: &ServiceStartCommand,
) -> Result<Option<ManagedConnectorProcess>> {
    match start_command {
        ServiceStartCommand::ShellCommand {
            command,
            cwd,
            env,
            timeout_secs: _,
        } => {
            let command = managed_start_command(command);
            if command.is_empty() || command[0].trim().is_empty() {
                bail!("start command is empty");
            }
            let mut child = AsyncCommand::new(&command[0]);
            child.args(command.iter().skip(1));
            if let Some(cwd) = cwd.as_deref().filter(|value| !value.trim().is_empty()) {
                child.current_dir(cwd);
            }
            child.envs(env);
            child.env("BRIDGE_AGENT_MANAGED_CONNECTOR", "1");
            let child = child
                .spawn()
                .with_context(|| format!("failed to spawn `{}`", command.join(" ")))?;
            Ok(Some(ManagedConnectorProcess {
                service: service.name.clone(),
                child,
            }))
        }
    }
}

fn managed_start_command(command: &[String]) -> Vec<String> {
    let mut managed = command
        .iter()
        .filter(|part| part.as_str() != "--daemon")
        .cloned()
        .collect::<Vec<_>>();
    if is_bridge_collector_command(&managed) {
        if let Some(index) = managed.iter().position(|part| part == "start") {
            managed[index] = "run".to_string();
        }
    }
    managed
}

fn is_bridge_collector_command(command: &[String]) -> bool {
    command.iter().any(|part| {
        let normalized = part.replace('\\', "/");
        normalized.ends_with("wechat-bridge-collector")
            || normalized.ends_with("wecom-bridge-collector")
            || normalized == "wechat_bridge_collector"
            || normalized == "wecom_bridge_collector"
            || normalized.contains("/wechat_bridge_collector/")
            || normalized.contains("/wecom_bridge_collector/")
    })
}

async fn stop_managed_connector_process(
    process: &mut ManagedConnectorProcess,
    inner: &Arc<RuntimeInner>,
    log_limit: usize,
) {
    let pid = process.child.id();
    if let Some(pid) = pid {
        signal_process(pid, "TERM").await;
    }
    match timeout(Duration::from_secs(5), process.child.wait()).await {
        Ok(Ok(status)) => {
            push_log_entry(
                inner,
                log_limit,
                "info",
                &format!(
                    "managed connector `{}` stopped with status {}",
                    process.service, status
                ),
                LogMetadata::category("connector")
                    .service(process.service.clone())
                    .outcome("stopped"),
            )
            .await;
        }
        Ok(Err(err)) => {
            push_log_entry(
                inner,
                log_limit,
                "warn",
                &format!(
                    "failed to wait for managed connector `{}`: {err:#}",
                    process.service
                ),
                LogMetadata::category("connector")
                    .service(process.service.clone())
                    .outcome("wait_failed"),
            )
            .await;
        }
        Err(_) => {
            if let Some(pid) = pid {
                signal_process(pid, "KILL").await;
            } else {
                let _ = process.child.start_kill();
            }
            let _ = timeout(Duration::from_secs(2), process.child.wait()).await;
            push_log_entry(
                inner,
                log_limit,
                "warn",
                &format!(
                    "managed connector `{}` did not exit after TERM and was killed",
                    process.service
                ),
                LogMetadata::category("connector")
                    .service(process.service.clone())
                    .outcome("killed"),
            )
            .await;
        }
    }
}

async fn signal_process(pid: u32, signal: &str) {
    #[cfg(unix)]
    {
        let pid_text = pid.to_string();
        let _ = AsyncCommand::new("pkill")
            .args([format!("-{signal}"), "-P".to_string(), pid_text.clone()])
            .status()
            .await;
        let _ = AsyncCommand::new("kill")
            .args([format!("-{signal}"), pid_text])
            .status()
            .await;
    }
    #[cfg(windows)]
    {
        let _ = pid;
        let _ = signal;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        let _ = signal;
    }
}

impl RuntimeRunner {
    async fn run(
        &self,
        mut shutdown_rx: watch::Receiver<bool>,
        apply_rx: &mut mpsc::UnboundedReceiver<RuntimeRegistryUpdate>,
        event_rx: &mut mpsc::Receiver<LocalEventEmitRequest>,
        audit_rx: &mut mpsc::UnboundedReceiver<RuntimeAuditLog>,
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
                    self.push_log("info", "connected to relay, waiting for registration")
                        .await;

                    if let Err(err) = self
                        .handle_connection(stream, &mut shutdown_rx, apply_rx, event_rx, audit_rx)
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
                Some(audit) = audit_rx.recv() => {
                    self.push_audit_log(audit).await;
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
        audit_rx: &mut mpsc::UnboundedReceiver<RuntimeAuditLog>,
    ) -> Result<()> {
        let (mut write, mut read) = stream.split();
        let capabilities = self.current_capabilities().await;
        write_json(&mut write, &capabilities).await?;
        let keepalive_interval = Duration::from_secs(RELAY_KEEPALIVE_INTERVAL_SECS);
        let heartbeat_timeout = Duration::from_secs(RELAY_HEARTBEAT_TIMEOUT_SECS);
        let mut keepalive = interval_at(
            tokio::time::Instant::now() + keepalive_interval,
            keepalive_interval,
        );
        let mut last_relay_seen = tokio::time::Instant::now();

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
                    let event_id = event.event_id.clone();
                    let message = AgentMessage::EventEmitted(EventEmitted {
                        event_id: Some(event.event_id),
                        service: event.service,
                        event: event.event,
                        payload: event.payload,
                        occurred_at: event.occurred_at,
                    });
                    write_json(&mut write, &message).await?;
                    self.push_log_with_metadata(
                        "info",
                        &format!("event {service}.{event_name} sent to relay"),
                        LogMetadata::category("event")
                            .service(service)
                            .event(event_name)
                            .event_id(event_id)
                            .outcome("sent"),
                    )
                    .await;
                }
                Some(audit) = audit_rx.recv() => {
                    self.push_audit_log(audit).await;
                }
                _ = keepalive.tick() => {
                    if last_relay_seen.elapsed() > heartbeat_timeout {
                        bail!(
                            "relay heartbeat timed out after {}s without server frame",
                            RELAY_HEARTBEAT_TIMEOUT_SECS
                        );
                    }
                    write.send(Message::Ping(Vec::new().into())).await?;
                }
                message = read.next() => {
                    let Some(message) = message else {
                        bail!("relay websocket ended");
                    };
                    match message? {
                        Message::Text(text) => {
                            last_relay_seen = tokio::time::Instant::now();
                            self.update_relay_seen().await;
                            let Some(incoming) = decode_relay_message(&text)
                                .with_context(|| format!("invalid relay message: {text}"))?
                            else {
                                self.push_log("warn", "ignored unsupported relay message type")
                                    .await;
                                continue;
                            };
                            match incoming {
                                AgentMessage::RegisteredAck(ack) => {
                                    self.update_registered_snapshot(&ack).await;
                                    self.push_log(
                                        "info",
                                        &format!(
                                            "registered on relay as {} connection {}",
                                            ack.agent_id, ack.connection_id
                                        ),
                                    )
                                    .await;
                                }
                                AgentMessage::InvokeRequest(request) => {
                                    let service = request.service.clone();
                                    let method = request.method.clone();
                                    let request_id = request.request_id.clone();
                                    self.push_log_with_metadata(
                                        "info",
                                        &format!("invoke {service}.{method} started"),
                                        LogMetadata::category("invoke")
                                            .service(service.clone())
                                            .method(method.clone())
                                            .request_id(request_id.clone())
                                            .outcome("started"),
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
                                        self.push_log_with_metadata(
                                            "info",
                                            &format!(
                                                "invoke {service}.{method} succeeded in {}ms",
                                                result.duration_ms
                                            ),
                                            LogMetadata::category("invoke")
                                                .service(service.clone())
                                                .method(method.clone())
                                                .request_id(request_id.clone())
                                                .outcome("succeeded")
                                                .duration_ms(result.duration_ms),
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
                                        self.push_log_with_metadata(
                                            "warn",
                                            &format!(
                                                "invoke {service}.{method} failed in {}ms: {error}",
                                                result.duration_ms
                                            ),
                                            LogMetadata::category("invoke")
                                                .service(service.clone())
                                                .method(method.clone())
                                                .request_id(request_id.clone())
                                                .outcome("failed")
                                                .duration_ms(result.duration_ms),
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
                            last_relay_seen = tokio::time::Instant::now();
                            self.update_relay_seen().await;
                            write.send(Message::Pong(payload)).await?;
                        }
                        Message::Close(_) => {
                            bail!("relay closed websocket");
                        }
                        Message::Pong(_) => {
                            last_relay_seen = tokio::time::Instant::now();
                            self.update_relay_seen().await;
                        }
                        Message::Binary(_) | Message::Frame(_) => {}
                    }
                }
            }
        }
        Ok(())
    }

    async fn current_capabilities(&self) -> AgentMessage {
        AgentMessage::Capabilities(AgentCapabilities {
            agent_id: self.config.relay.agent_id.clone(),
            protocol_version: AGENT_PROTOCOL_VERSION,
            protocol_features: vec![AGENT_PROTOCOL_FEATURE_REGISTERED_ACK.to_string()],
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
            protocol_version: AGENT_PROTOCOL_VERSION,
            protocol_features: vec![AGENT_PROTOCOL_FEATURE_REGISTERED_ACK.to_string()],
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
            relay_registered: false,
            relay_registered_at: None,
            last_relay_seen_at: None,
            log_file_path: state.snapshot.log_file_path.clone(),
            last_error,
            last_event_at: now_ms(),
        };
    }

    async fn update_registered_snapshot(&self, ack: &crate::protocol::RegisteredAck) {
        let mut state = self.inner.state.lock().await;
        state.snapshot.status = RuntimeStatus::Online;
        state.snapshot.relay_registered = true;
        state.snapshot.relay_registered_at = Some(ack.registered_at_epoch_seconds);
        state.snapshot.last_relay_seen_at = Some(now_ms());
        state.snapshot.last_error = None;
        state.snapshot.last_event_at = now_ms();
    }

    async fn update_relay_seen(&self) {
        let mut state = self.inner.state.lock().await;
        state.snapshot.last_relay_seen_at = Some(now_ms());
    }

    async fn push_log(&self, level: &str, message: &str) {
        push_log_entry(
            &self.inner,
            self.log_limit,
            level,
            message,
            LogMetadata::default(),
        )
        .await;
    }

    async fn push_log_with_metadata(&self, level: &str, message: &str, metadata: LogMetadata) {
        push_log_entry(&self.inner, self.log_limit, level, message, metadata).await;
    }

    async fn push_audit_log(&self, audit: RuntimeAuditLog) {
        push_log_entry(
            &self.inner,
            self.log_limit,
            &audit.level,
            &audit.message,
            audit.metadata,
        )
        .await;
    }
}

async fn push_log_entry(
    inner: &RuntimeInner,
    limit: usize,
    level: &str,
    message: &str,
    metadata: LogMetadata,
) {
    emit_tracing(level, message);
    let entry = LogEntry {
        timestamp_ms: now_ms(),
        level: level.to_string(),
        message: message.to_string(),
        metadata,
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
        if let Err(err) = file_log.append(entry) {
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
            | "event_emitted"
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
    if document.pid == std::process::id() {
        return Ok(false);
    }
    let process = describe_process(document.pid);
    if runtime_lock_owner_is_active(&process) {
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

fn runtime_lock_owner_is_active(process: &RuntimeProcessInfo) -> bool {
    if !process.running {
        return false;
    }
    if process_looks_like_bridge_agent(process) {
        return true;
    }

    // If the platform cannot describe a running PID, keep the lock rather than
    // risking a second runtime. Identifiable non-Bridge-Agent PIDs are stale
    // lock reuse and can be reclaimed.
    process.name.is_none() && process.executable_path.is_none() && process.command_line.is_none()
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
    if document.pid == std::process::id() {
        bail!("runtime lock is owned by this 百积木 process; use the normal stop or restart action instead");
    }

    let process = describe_process(document.pid);
    if process.running {
        if !process_looks_like_bridge_agent(&process) {
            bail!(
                "pid {} is running but does not look like a 百积木 process",
                document.pid
            );
        }
        terminate_process(document.pid)?;
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
    if let Some(process) = describe_process_windows_api(pid) {
        return process;
    }

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

    if let Some(name) = lookup_windows_tasklist_image_name(pid) {
        return RuntimeProcessInfo {
            pid,
            parent_pid: None,
            name: Some(name),
            executable_path: None,
            command_line: None,
            running: true,
        };
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

#[cfg(windows)]
fn describe_process_windows_api(pid: u32) -> Option<RuntimeProcessInfo> {
    if let Some(entry) = find_windows_snapshot_process(pid).flatten() {
        return Some(RuntimeProcessInfo {
            pid,
            parent_pid: entry.parent_pid,
            name: entry.name,
            executable_path: query_windows_process_image_path(pid),
            command_line: None,
            running: true,
        });
    }

    query_windows_process_image_path(pid).map(|executable_path| RuntimeProcessInfo {
        pid,
        parent_pid: None,
        name: Some(process_file_name(&executable_path).to_string()),
        executable_path: Some(executable_path),
        command_line: None,
        running: true,
    })
}

#[cfg(windows)]
struct WindowsSnapshotProcessEntry {
    parent_pid: Option<u32>,
    name: Option<String>,
}

#[cfg(windows)]
fn find_windows_snapshot_process(pid: u32) -> Option<Option<WindowsSnapshotProcessEntry>> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }

    let mut found = None;
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while ok {
        if entry.th32ProcessID == pid {
            found = Some(WindowsSnapshotProcessEntry {
                parent_pid: Some(entry.th32ParentProcessID),
                name: wide_null_terminated_to_string(&entry.szExeFile),
            });
            break;
        }
        ok = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }

    unsafe {
        CloseHandle(snapshot);
    }
    Some(found)
}

#[cfg(windows)]
fn query_windows_process_image_path(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }

    let mut buffer = vec![0u16; 32768];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) } != 0;
    unsafe {
        CloseHandle(handle);
    }
    if !ok || size == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..size as usize]))
}

#[cfg(any(windows, test))]
fn wide_null_terminated_to_string(value: &[u16]) -> Option<String> {
    let end = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
    if end == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&value[..end]))
}

#[cfg(windows)]
fn lookup_windows_tasklist_image_name(pid: u32) -> Option<String> {
    let filter = format!("PID eq {pid}");
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_tasklist_image_name(&stdout)
}

#[cfg(any(windows, test))]
fn parse_tasklist_image_name(tasklist_output: &str) -> Option<String> {
    tasklist_output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with("INFO:"))
        .find_map(first_csv_field)
}

#[cfg(any(windows, test))]
fn first_csv_field(line: &str) -> Option<String> {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix('"') {
        let mut field = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                    continue;
                }
                return Some(field);
            }
            field.push(ch);
        }
        return None;
    }
    line.split(',').next().map(str::trim).map(ToOwned::to_owned)
}

#[cfg(unix)]
fn describe_process_unix(pid: u32) -> RuntimeProcessInfo {
    if let Ok(output) = std::process::Command::new("ps")
        .args([
            "-p",
            &pid.to_string(),
            "-o",
            "ppid=",
            "-o",
            "comm=",
            "-o",
            "args=",
        ])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().map(str::trim).find(|line| !line.is_empty()) {
                let mut parts = line.splitn(3, char::is_whitespace);
                let parent_pid = parts.next().and_then(|value| value.trim().parse().ok());
                let name = parts
                    .next()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let command_line = parts
                    .next()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
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
    process
        .name
        .as_deref()
        .is_some_and(is_bridge_agent_process_name)
        || process
            .executable_path
            .as_deref()
            .is_some_and(is_bridge_agent_process_name)
        || process
            .command_line
            .as_deref()
            .is_some_and(command_line_starts_with_bridge_agent)
}

fn command_line_starts_with_bridge_agent(command_line: &str) -> bool {
    let command_line = command_line.trim();
    if command_line.is_empty() {
        return false;
    }

    if let Some(rest) = command_line.strip_prefix('"') {
        if let Some((executable, _)) = rest.split_once('"') {
            return is_bridge_agent_process_name(executable);
        }
        return false;
    }

    command_line
        .split_whitespace()
        .next()
        .is_some_and(is_bridge_agent_process_name)
}

#[cfg(windows)]
fn terminate_process(pid: u32) -> Result<()> {
    let status = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .status()
        .with_context(|| format!("failed to run taskkill for pid {pid}"))?;
    if status.success() || !process_is_running(pid) {
        return Ok(());
    }
    bail!("failed to terminate runtime owner pid {pid}");
}

#[cfg(unix)]
fn terminate_process(pid: u32) -> Result<()> {
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
fn terminate_process(pid: u32) -> Result<()> {
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
    match find_windows_snapshot_process(pid) {
        Some(found) => return found.is_some(),
        None => {}
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
        build_agent_url, command_line_starts_with_bridge_agent, decode_relay_message,
        managed_start_command, parse_tasklist_image_name, process_looks_like_bridge_agent,
        read_runtime_lock, runtime_lock_owner_is_active, runtime_lock_path,
        runtime_start_is_active, terminate_runtime_lock_owner, wide_null_terminated_to_string,
        AgentRuntimeManager, RuntimeInstanceLock, RuntimeLockDocument, RuntimeProcessInfo,
        RuntimeStatus,
    };
    use crate::config::AgentConfig;
    use crate::protocol::AgentMessage;
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
    fn relay_decoder_ignores_unknown_message_type() {
        let message = r#"{"type":"server_notice","message":"hello"}"#;

        assert!(decode_relay_message(message).unwrap().is_none());
    }

    #[test]
    fn relay_decoder_accepts_registered_ack() {
        let message = r#"{"type":"registered_ack","agent_id":"dev_1","workspace_id":1327,"connection_id":"conn_1","registered_at_epoch_seconds":1783680377,"heartbeat_timeout_secs":75}"#;

        match decode_relay_message(message).unwrap().unwrap() {
            AgentMessage::RegisteredAck(ack) => {
                assert_eq!(ack.agent_id, "dev_1");
                assert_eq!(ack.workspace_id, 1327);
                assert_eq!(ack.connection_id, "conn_1");
            }
            other => panic!("expected registered_ack, got {other:?}"),
        }
    }

    #[test]
    fn relay_decoder_rejects_invalid_known_message() {
        let message = r#"{"type":"invoke_request","request_id":"req_1"}"#;

        assert!(decode_relay_message(message).is_err());
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

    #[test]
    fn runtime_lock_owner_rejects_pid_reused_by_non_bridge_process() {
        let process = RuntimeProcessInfo {
            pid: 727,
            parent_pid: Some(1),
            name: Some("ViewBridgeAuxiliary".to_string()),
            executable_path: Some(
                "/System/Library/PrivateFrameworks/ViewBridge.framework/Versions/A/XPCServices/ViewBridgeAuxiliary.xpc/Contents/MacOS/ViewBridgeAuxiliary".to_string(),
            ),
            command_line: Some(
                "/System/Library/PrivateFrameworks/ViewBridge.framework/Versions/A/XPCServices/ViewBridgeAuxiliary.xpc/Contents/MacOS/ViewBridgeAuxiliary".to_string(),
            ),
            running: true,
        };

        assert!(!runtime_lock_owner_is_active(&process));
    }

    #[test]
    fn runtime_lock_owner_keeps_unidentifiable_running_process() {
        let process = RuntimeProcessInfo {
            pid: 42,
            parent_pid: None,
            name: None,
            executable_path: None,
            command_line: None,
            running: true,
        };

        assert!(runtime_lock_owner_is_active(&process));
    }

    #[test]
    fn runtime_start_active_statuses_are_idempotent_start_targets() {
        assert!(runtime_start_is_active(RuntimeStatus::Starting));
        assert!(runtime_start_is_active(RuntimeStatus::Connecting));
        assert!(runtime_start_is_active(RuntimeStatus::Backoff));
        assert!(!runtime_start_is_active(RuntimeStatus::Online));
        assert!(!runtime_start_is_active(RuntimeStatus::Stopped));
        assert!(!runtime_start_is_active(RuntimeStatus::Stopping));
    }

    #[test]
    fn managed_start_command_removes_daemon_flag() {
        let command = vec![
            "baijimu-connector-codex".to_string(),
            "start".to_string(),
            "--daemon".to_string(),
            "--port".to_string(),
            "18110".to_string(),
        ];
        assert_eq!(
            managed_start_command(&command),
            vec![
                "baijimu-connector-codex".to_string(),
                "start".to_string(),
                "--port".to_string(),
                "18110".to_string(),
            ]
        );
    }

    #[test]
    fn managed_start_command_runs_collectors_in_foreground() {
        let command = vec![
            "wechat-bridge-collector".to_string(),
            "--config".to_string(),
            "/tmp/config.json".to_string(),
            "start".to_string(),
        ];
        assert_eq!(
            managed_start_command(&command),
            vec![
                "wechat-bridge-collector".to_string(),
                "--config".to_string(),
                "/tmp/config.json".to_string(),
                "run".to_string(),
            ]
        );
    }

    #[test]
    fn managed_start_command_runs_python_module_collectors_in_foreground() {
        let command = vec![
            "/opt/anaconda3/bin/python".to_string(),
            "-m".to_string(),
            "wechat_bridge_collector".to_string(),
            "--config".to_string(),
            "/tmp/config.json".to_string(),
            "start".to_string(),
        ];
        assert_eq!(
            managed_start_command(&command),
            vec![
                "/opt/anaconda3/bin/python".to_string(),
                "-m".to_string(),
                "wechat_bridge_collector".to_string(),
                "--config".to_string(),
                "/tmp/config.json".to_string(),
                "run".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn runtime_manager_serializes_concurrent_starts() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.relay.url = "ws://127.0.0.1:9/ws/agent".to_string();
        config.relay.agent_id = "dev_concurrent_start".to_string();
        config.runtime.event_server_enabled = false;
        config.runtime.service_registration_enabled = false;
        config.services.clear();

        let manager = AgentRuntimeManager::new();
        let first_manager = manager.clone();
        let second_manager = manager.clone();
        let first_config = config.clone();
        let second_config = config;
        let first_path = config_path.clone();
        let second_path = config_path.clone();

        let (first, second) = tokio::join!(
            async move { first_manager.start(first_config, &first_path).await },
            async move { second_manager.start(second_config, &second_path).await }
        );

        assert!(first.is_ok(), "first start failed: {first:?}");
        assert!(second.is_ok(), "second start failed: {second:?}");
        manager.stop().await.unwrap();
    }

    #[tokio::test]
    async fn runtime_manager_reuses_active_start_for_duplicate_start() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.relay.url = "ws://127.0.0.1:9/ws/agent".to_string();
        config.relay.agent_id = "dev_duplicate_start".to_string();
        config.runtime.event_server_enabled = false;
        config.runtime.service_registration_enabled = false;
        config.services.clear();

        let manager = AgentRuntimeManager::new();
        manager.start(config.clone(), &config_path).await.unwrap();
        let lock_path = runtime_lock_path(&config_path, &config.relay.agent_id);
        let before = read_runtime_lock(&lock_path).unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        manager.start(config, &config_path).await.unwrap();
        let after = read_runtime_lock(&lock_path).unwrap();

        assert_eq!(before.pid, after.pid);
        assert_eq!(before.started_at_ms, after.started_at_ms);
        manager.stop().await.unwrap();
    }

    #[test]
    fn terminating_conflicting_runtime_refuses_current_process_owner() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let lock_path = runtime_lock_path(&config_path, "dev_1");
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        let lock = RuntimeLockDocument {
            pid: std::process::id(),
            agent_id: "dev_1".to_string(),
            config_path: config_path.display().to_string(),
            started_at_ms: 1,
        };
        fs::write(&lock_path, serde_json::to_string_pretty(&lock).unwrap()).unwrap();

        let err =
            terminate_runtime_lock_owner(&lock_path, lock.pid, &lock.agent_id, &lock.config_path)
                .unwrap_err();

        assert!(err.to_string().contains("owned by this 百积木 process"));
        assert!(lock_path.exists());
    }

    #[test]
    fn tasklist_image_name_reads_product_name_with_spaces() {
        let output = r#""百积木.exe","18080","Console","1","64,000 K""#;

        assert_eq!(
            parse_tasklist_image_name(output),
            Some("百积木.exe".to_string())
        );
    }

    #[test]
    fn wide_process_name_reads_utf16_until_null() {
        let mut value = "百积木.exe".encode_utf16().collect::<Vec<_>>();
        value.push(0);
        value.extend("ignored".encode_utf16());

        assert_eq!(
            wide_null_terminated_to_string(&value),
            Some("百积木.exe".to_string())
        );
    }

    #[test]
    fn runtime_process_identity_accepts_desktop_executable_name() {
        let process = RuntimeProcessInfo {
            pid: 13304,
            parent_pid: None,
            name: Some("bridge-agent-desktop.exe".to_string()),
            executable_path: Some(
                r#"C:\Program Files\百积木\bridge-agent-desktop.exe"#.to_string(),
            ),
            command_line: None,
            running: true,
        };

        assert!(process_looks_like_bridge_agent(&process));
    }

    #[test]
    fn runtime_process_identity_accepts_quoted_install_path() {
        assert!(command_line_starts_with_bridge_agent(
            r#""C:\Program Files\百积木\百积木.exe" --config agent-config.json"#
        ));
        assert!(command_line_starts_with_bridge_agent(
            r#""C:\Program Files\百积木\bridge-agent-desktop.exe" --config agent-config.json"#
        ));
    }

    #[test]
    fn runtime_process_identity_rejects_unknown_or_helper_processes() {
        let process = RuntimeProcessInfo {
            pid: 18080,
            parent_pid: None,
            name: Some("my-bridge-agent-helper.exe".to_string()),
            executable_path: None,
            command_line: None,
            running: true,
        };

        assert!(!process_looks_like_bridge_agent(&process));
        assert!(!command_line_starts_with_bridge_agent(
            r#""C:\Program Files\nodejs\node.exe" bridge-agent.js"#
        ));
    }
}
